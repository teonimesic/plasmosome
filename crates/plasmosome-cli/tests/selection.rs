#[allow(dead_code, reason = "this file uses part of the shared support")]
mod support;

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use plasmosome_cli::{COMMANDS, Context, Exit, SystemClock, handle};
use plasmosome_core::private_socket::effective_uid;
use serde_json::{Value, json};
use support::{Answer, FakeDaemon, PATIENCE, Vars, base};
use tempfile::TempDir;

#[cfg(target_os = "macos")]
const MAX_PATH: usize = 103;
#[cfg(not(target_os = "macos"))]
const MAX_PATH: usize = 107;

const VARIABLE: &str = "PLASMOSOME_KERNEL";

struct Ran {
    exit: Exit,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run(line: &[&str], vars: Vars) -> Ran {
    let line: Vec<OsString> = line.iter().map(OsString::from).collect();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let context = Context {
            version: "0.0.0",
            commands: COMMANDS,
            trusted_uid: effective_uid(),
            clock: &SystemClock,
            env: &vars,
        };
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let exit = handle(&line, &context, &mut stdout, &mut stderr);
        let _ = sender.send(Ran {
            exit,
            stdout,
            stderr,
        });
    });
    receiver
        .recv_timeout(PATIENCE)
        .unwrap_or_else(|_| panic!("plasmosome did not return within {PATIENCE:?}"))
}

fn one_line(bytes: &[u8]) -> Value {
    let text = std::str::from_utf8(bytes).expect("the output is UTF-8");
    assert!(
        text.ends_with('\n') && text.matches('\n').count() == 1,
        "the output is not one line: {text:?}"
    );
    serde_json::from_str(text).expect("the output is one JSON line")
}

fn success(ran: &Ran) -> Value {
    assert_eq!(
        ran.exit,
        Exit::Done,
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    assert!(ran.stderr.is_empty(), "a success wrote to stderr");
    one_line(&ran.stdout)
}

fn failure(ran: &Ran, exit: Exit) -> Value {
    assert_eq!(ran.exit, exit, "{}", String::from_utf8_lossy(&ran.stderr));
    assert!(ran.stdout.is_empty(), "a failure wrote to stdout");
    one_line(&ran.stderr)
}

fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a UTF-8 path")
}

fn escape() -> char {
    char::from_u32(0x1b).expect("ESC is a code point")
}

fn backslash() -> char {
    char::from(0x5c_u8)
}

fn reply_as(name: &str) -> Answer {
    Answer::Reply(format!("{}\n", json!({"id": 1, "result": {"name": name}})).into_bytes())
}

fn requests(fake: &FakeDaemon, count: usize) -> Vec<Value> {
    let sent = fake.ended(count);
    assert_eq!(sent.len(), count, "the fake saw {} connections", sent.len());
    sent.iter()
        .map(|bytes| serde_json::from_slice(bytes).expect("each request is JSON"))
        .collect()
}

fn assert_never_reached(listener: &UnixListener, what: &str) {
    match listener.accept() {
        Err(error) if error.kind() == ErrorKind::WouldBlock => {}
        other => panic!("{what} reached a socket it must not: {other:?}"),
    }
}

/// A test HOME: a private root with a short name, so that each instance's socket path fits
/// `sun_path`.
struct Home {
    root: TempDir,
}

impl Home {
    fn new() -> Home {
        let base = base();
        let root = tempfile::Builder::new()
            .prefix("h")
            .rand_bytes(4)
            .tempdir_in(&base)
            .unwrap_or_else(|error| panic!("no test HOME under {}: {error}", base.display()));
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))
            .expect("the test HOME is 0700");
        let home = Home { root };
        let longest = home.socket("a");
        assert!(
            longest.as_os_str().len() <= MAX_PATH,
            "{} does not fit sun_path",
            longest.display()
        );
        home
    }

    fn path(&self) -> &Path {
        self.root.path()
    }

    fn instances(&self) -> PathBuf {
        self.path().join(".plasmosome/instances")
    }

    fn socket(&self, name: &str) -> PathBuf {
        self.instances().join(name).join("control.uds")
    }

    fn dir(&self, name: impl AsRef<OsStr>) -> PathBuf {
        let dir = self.instances().join(name.as_ref());
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .unwrap_or_else(|error| panic!("no directory {}: {error}", dir.display()));
        dir
    }

    fn empty_instances(&self) -> PathBuf {
        let instances = self.instances();
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&instances)
            .expect("instances/ is made");
        instances
    }

    fn ds_store(&self) {
        fs::write(self.empty_instances().join(".DS_Store"), b"Bud1").expect("a .DS_Store file");
    }

    fn live(&self, name: &str, answers: usize) -> FakeDaemon {
        self.dir(name);
        FakeDaemon::start(
            &self.socket(name),
            (0..answers).map(|_| reply_as(name)).collect(),
        )
    }

    fn listening(&self, name: &str) -> UnixListener {
        self.dir(name);
        let socket = self.socket(name);
        let listener = UnixListener::bind(&socket).expect("a listening socket");
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).expect("0600");
        listener
            .set_nonblocking(true)
            .expect("the listener does not block");
        listener
    }

    fn stale(&self, name: &str) -> u64 {
        self.dir(name);
        let socket = self.socket(name);
        drop(UnixListener::bind(&socket).expect("a socket"));
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).expect("0600");
        inode(&socket)
    }

    fn vars(&self) -> Vars {
        Vars {
            home: Some(self.path().into()),
            kernel: None,
        }
    }

    fn with_kernel(&self, kernel: &str) -> Vars {
        Vars {
            kernel: Some(kernel.into()),
            ..self.vars()
        }
    }
}

fn inode(path: &Path) -> u64 {
    fs::symlink_metadata(path)
        .unwrap_or_else(|error| panic!("{} is gone: {error}", path.display()))
        .ino()
}

#[test]
fn a_down_instance_exits_1_and_no_other_is_contacted() {
    let home = Home::new();
    let other = home.listening("b");

    let envelope = failure(
        &run(&["status", "--kernel", "z"], home.vars()),
        Exit::Unavailable,
    );
    assert_eq!(envelope["command"], "plasmosome.status");
    let error = &envelope["error"];
    assert_eq!(error["code"], "no_instance");
    assert_eq!(error["path"], text(&home.instances().join("z")));
    assert!(error.get("seen").is_none(), "{error}");
    assert_eq!(
        envelope["instance"],
        json!({"name": "z", "socket": text(&home.socket("z"))})
    );

    home.dir("y");
    let envelope = failure(
        &run(&["status", "--kernel", "y"], home.vars()),
        Exit::Unavailable,
    );
    let error = &envelope["error"];
    assert_eq!(error["code"], "not_running");
    assert_eq!(error["path"], text(&home.socket("y")));
    assert_eq!(
        envelope["instance"],
        json!({"name": "y", "socket": text(&home.socket("y"))})
    );

    let before = home.stale("x");
    for vars in [home.vars(), home.with_kernel("x")] {
        let line: &[&str] = if vars.kernel.is_some() {
            &["status"]
        } else {
            &["status", "--kernel", "x"]
        };
        let envelope = failure(&run(line, vars), Exit::Unavailable);
        let error = &envelope["error"];
        assert_eq!(error["code"], "unreachable");
        assert_eq!(error["path"], text(&home.socket("x")));
        assert!(error.get("fix").is_none(), "{error}");
        assert_eq!(
            envelope["instance"],
            json!({"name": "x", "socket": text(&home.socket("x"))})
        );
        assert_eq!(inode(&home.socket("x")), before);
    }

    assert_never_reached(&other, "a down instance");
}

#[test]
fn a_home_with_no_plasmosome_directory_has_no_instance() {
    let home = Home::new();
    let missing = text(&home.path().join(".plasmosome")).to_string();

    let envelope = failure(
        &run(&["status", "--kernel", "z"], home.vars()),
        Exit::Unavailable,
    );
    assert_eq!(envelope["error"]["code"], "no_instance");
    assert_eq!(envelope["error"]["path"], missing);

    let envelope = failure(&run(&["status"], home.vars()), Exit::Unavailable);
    assert_eq!(envelope["command"], "plasmosome.status");
    let error = &envelope["error"];
    assert_eq!(error["code"], "no_instance");
    assert_eq!(error["path"], missing);
    assert_eq!(error["seen"], json!([]));
    assert!(envelope.get("instance").is_none(), "{envelope}");
}

#[test]
fn the_flag_beats_the_variable() {
    let home = Home::new();
    let a = home.live("a", 2);
    let b = home.live("b", 1);

    let envelope = success(&run(&["status", "--kernel", "a"], home.vars()));
    assert_eq!(
        envelope["instance"],
        json!({"name": "a", "socket": text(&home.socket("a"))})
    );
    assert_eq!(envelope["result"], json!({"name": "a"}));

    let envelope = success(&run(&["status"], home.with_kernel("b")));
    assert_eq!(
        envelope["instance"],
        json!({"name": "b", "socket": text(&home.socket("b"))})
    );

    let envelope = success(&run(&["status", "--kernel", "a"], home.with_kernel("b")));
    assert_eq!(envelope["instance"]["name"], "a");

    let named =
        |name: &str| json!({"id": 1, "method": "plasmosome.status", "params": {"name": name}});
    assert_eq!(requests(&a, 2), [named("a"), named("a")]);
    assert_eq!(requests(&b, 1), [named("b")]);
}

#[test]
fn the_socket_flag_ignores_the_variable() {
    let home = Home::new();
    let a = home.listening("a");
    let b = home.live("b", 3);
    let socket = text(&home.socket("b")).to_string();

    let envelope = success(&run(
        &["status", "--socket", &socket],
        home.with_kernel("a"),
    ));
    assert_eq!(envelope["instance"], json!({"socket": socket}));

    let envelope = success(&run(
        &["status", "--socket", &socket, "--kernel", "a"],
        home.with_kernel("b"),
    ));
    assert_eq!(envelope["instance"], json!({"name": "a", "socket": socket}));

    let envelope = success(&run(
        &["status", "--socket", &socket],
        home.with_kernel("../x"),
    ));
    assert_eq!(envelope["instance"], json!({"socket": socket}));

    let params: Vec<Value> = requests(&b, 3)
        .into_iter()
        .map(|request| request["params"].clone())
        .collect();
    assert_eq!(params, [json!({}), json!({"name": "a"}), json!({})]);
    assert_never_reached(&a, "--socket at another instance");
}

#[test]
fn the_flag_ignores_a_bad_variable() {
    let home = Home::new();
    let a = home.live("a", 1);
    let envelope = success(&run(&["status", "--kernel", "a"], home.with_kernel("../x")));
    assert_eq!(envelope["instance"]["name"], "a");
    assert_eq!(requests(&a, 1)[0]["params"], json!({"name": "a"}));
}

#[test]
fn two_instances_are_ambiguous_and_neither_gets_a_byte() {
    for ds_store in [false, true] {
        let home = Home::new();
        if ds_store {
            home.ds_store();
        }
        let a = home.live("a", 1);
        let b = home.live("b", 1);
        let envelope = failure(&run(&["status"], home.vars()), Exit::Refused);
        assert_eq!(envelope["command"], "plasmosome.status");
        let error = &envelope["error"];
        assert_eq!(error["code"], "ambiguous_instance", "{ds_store}");
        assert_eq!(error["candidates"], json!(["a", "b"]));
        assert_eq!(
            keys(error),
            ["candidates", "code", "domain", "message"],
            "{error}"
        );
        assert!(envelope.get("instance").is_none(), "{envelope}");
        assert_eq!(a.ended(1), [Vec::<u8>::new()]);
        assert_eq!(b.ended(1), [Vec::<u8>::new()]);
    }
}

#[test]
fn an_unreachable_candidate_is_still_a_candidate() {
    for ds_store in [false, true] {
        let home = Home::new();
        if ds_store {
            home.ds_store();
        }
        let a = home.live("a", 1);
        let before = home.stale("b");
        let envelope = failure(&run(&["status"], home.vars()), Exit::Refused);
        let error = &envelope["error"];
        assert_eq!(error["code"], "ambiguous_instance", "{ds_store}");
        assert_eq!(error["candidates"], json!(["a", "b"]));
        assert_eq!(error["unreachable"], json!(["b"]));
        assert!(envelope.get("instance").is_none(), "{envelope}");
        assert_eq!(a.ended(1), [Vec::<u8>::new()]);
        assert_eq!(inode(&home.socket("b")), before);
    }
}

#[test]
fn the_one_instance_with_a_socket_is_the_default() {
    for ds_store in [false, true] {
        let home = Home::new();
        if ds_store {
            home.ds_store();
        }
        let a = home.live("a", 2);
        home.dir("b");
        let envelope = success(&run(&["status"], home.vars()));
        assert_eq!(
            envelope["instance"],
            json!({"name": "a", "socket": text(&home.socket("a"))})
        );
        assert_eq!(
            requests(&a, 1),
            [json!({"id": 1, "method": "plasmosome.status", "params": {"name": "a"}})]
        );
        assert_eq!(
            a.connections(),
            1,
            "the request went on a second connection"
        );
    }
}

#[test]
fn a_lone_unreachable_instance_exits_1() {
    for ds_store in [false, true] {
        let home = Home::new();
        if ds_store {
            home.ds_store();
        }
        let before = home.stale("b");
        let envelope = failure(&run(&["status"], home.vars()), Exit::Unavailable);
        let error = &envelope["error"];
        assert_eq!(error["code"], "unreachable", "{ds_store}");
        assert_eq!(error["path"], text(&home.socket("b")));
        assert!(error.get("fix").is_none(), "{error}");
        assert_eq!(
            envelope["instance"],
            json!({"name": "b", "socket": text(&home.socket("b"))})
        );
        assert_eq!(inode(&home.socket("b")), before);
    }
}

#[test]
fn no_candidate_is_no_instance_with_what_was_seen() {
    for ds_store in [false, true] {
        let home = Home::new();
        home.empty_instances();
        if ds_store {
            home.ds_store();
        }
        let envelope = failure(&run(&["status"], home.vars()), Exit::Unavailable);
        let error = &envelope["error"];
        assert_eq!(error["code"], "no_instance", "{ds_store}");
        assert_eq!(error["path"], text(&home.instances()));
        assert_eq!(error["seen"], json!([]));
        assert!(envelope.get("instance").is_none(), "{envelope}");

        home.dir("b");
        home.dir("a");
        let envelope = failure(&run(&["status"], home.vars()), Exit::Unavailable);
        assert_eq!(envelope["error"]["code"], "no_instance");
        assert_eq!(envelope["error"]["seen"], json!(["a", "b"]));
    }
}

fn assert_unsafe(home: &Home, rule: &str, path: &Path) -> Value {
    let ran = run(&["status"], home.vars());
    let raw = String::from_utf8_lossy(&ran.stderr);
    assert!(!raw.contains(escape()), "a raw ESC reached stderr: {raw:?}");
    let envelope = failure(&ran, Exit::Refused);
    assert_eq!(envelope["command"], "plasmosome.status");
    let error = &envelope["error"];
    assert_eq!(error["code"], "unsafe_socket", "{error}");
    assert_eq!(error["rule"], rule, "{error}");
    assert_eq!(error["path"], text(path), "{error}");
    assert!(envelope.get("instance").is_none(), "{envelope}");
    error.clone()
}

#[test]
fn a_symbolic_link_in_instances_refuses_the_selection() {
    let elsewhere = Home::new();
    let target = elsewhere.listening("a");
    let file = elsewhere.path().join("file");
    fs::write(&file, b"not an instance").expect("a regular file");
    let pointees = [
        ("a directory", elsewhere.instances().join("a")),
        ("a socket", elsewhere.socket("a")),
        ("a file", file),
        ("nothing", elsewhere.path().join("missing")),
    ];
    for (what, pointee) in pointees {
        let home = Home::new();
        let a = home.listening("a");
        let link = home.instances().join("l");
        std::os::unix::fs::symlink(&pointee, &link).expect("a symlink");
        let ran = run(&["status"], home.vars());
        assert_eq!(
            ran.exit,
            Exit::Refused,
            "a link to {what}: {}",
            String::from_utf8_lossy(&ran.stderr)
        );
        assert_unsafe(&home, "symlink", &link);
        assert_never_reached(&a, &format!("a selection beside a link to {what}"));
    }
    assert_never_reached(&target, "a selection with a symbolic link");
}

#[test]
fn instances_itself_a_symbolic_link_refuses_the_selection() {
    let elsewhere = Home::new();
    let target = elsewhere.listening("a");
    let home = Home::new();
    fs::DirBuilder::new()
        .mode(0o700)
        .create(home.path().join(".plasmosome"))
        .expect(".plasmosome is made");
    std::os::unix::fs::symlink(elsewhere.instances(), home.instances()).expect("a symlink");
    assert_unsafe(&home, "symlink", &home.instances());
    assert_never_reached(&target, "a selection through a linked instances/");
}

#[test]
fn a_bad_name_in_instances_refuses_the_selection() {
    for bad in [format!("e{}", escape()), format!("a{}b", backslash())] {
        let home = Home::new();
        let a = home.listening("a");
        home.dir(&bad);
        assert_unsafe(&home, "name", &home.instances());
        assert_never_reached(&a, "a selection with a bad name");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn a_name_that_is_not_utf8_refuses_the_selection() {
    let home = Home::new();
    let a = home.listening("a");
    home.dir(OsString::from_vec(vec![b'n', 0xff]));
    assert_unsafe(&home, "name", &home.instances());
    assert_never_reached(&a, "a selection with a name that is not UTF-8");
}

#[test]
fn a_failing_candidate_refuses_the_selection() {
    let home = Home::new();
    let a = home.listening("a");
    let c = home.dir("c");
    let _c = UnixListener::bind(c.join("control.uds")).expect("a socket in c");
    fs::set_permissions(c.join("control.uds"), fs::Permissions::from_mode(0o600)).expect("0600");
    fs::set_permissions(&c, fs::Permissions::from_mode(0o750)).expect("c is 0750");
    let error = assert_unsafe(&home, "mode", &c);
    assert!(
        error["fix"]
            .as_str()
            .is_some_and(|fix| fix.starts_with("chmod 700 "))
    );
    assert_never_reached(&a, "a selection with a failing candidate");
}

#[test]
fn a_bad_variable_is_a_usage_error_naming_it() {
    let home = Home::new();
    let a = home.listening("a");
    let values = [
        OsString::new(),
        "../x".into(),
        ".".into(),
        "..".into(),
        "a/b".into(),
        format!("a{}b", backslash()).into(),
        format!("a{}[31m", escape()).into(),
        OsString::from_vec(vec![b'a', 0xff]),
        OsString::from_vec(vec![b'a', 0, b'b']),
    ];
    for value in values {
        for home_value in [Some(home.path().as_os_str().to_owned()), None] {
            let vars = Vars {
                home: home_value,
                kernel: Some(value.clone()),
            };
            for form in [None, Some("--human")] {
                let mut line = vec!["status"];
                line.extend(form);
                let ran = run(&line, vars.clone());
                assert_eq!(ran.exit, Exit::Usage, "{value:?}");
                assert!(ran.stdout.is_empty());
                let raw = String::from_utf8(ran.stderr.clone()).expect("stderr is UTF-8");
                assert!(!raw.contains(escape()), "a raw ESC reached stderr: {raw:?}");
                if form.is_some() {
                    assert!(raw.ends_with("\n(client invalid_argument)\n"), "{raw:?}");
                    continue;
                }
                let envelope = one_line(&ran.stderr);
                assert_eq!(envelope["command"], "invalid");
                assert_eq!(envelope["error"]["code"], "invalid_argument");
                assert_eq!(envelope["error"]["argument"], VARIABLE, "{value:?}");
                assert!(envelope.get("instance").is_none(), "{envelope}");
            }
        }
    }
    assert_never_reached(&a, "a bad PLASMOSOME_KERNEL");
}

#[test]
fn a_home_that_is_not_absolute_and_normal_is_invalid_home() {
    let home = Home::new();
    let x = home.live("x", 2);
    let spelled = text(home.path()).to_string();
    let (parent, name) = spelled.rsplit_once('/').expect("an absolute HOME");
    let homes: Vec<Option<OsString>> = vec![
        None,
        Some(OsString::new()),
        Some("relative/home".into()),
        Some(format!("{parent}//{name}").into()),
        Some(format!("{spelled}//").into()),
        Some(format!("{parent}/./{name}").into()),
        Some(format!("{spelled}/../{name}").into()),
        Some(format!("{spelled}/.").into()),
        Some(OsString::from_vec([spelled.as_bytes(), &[0xff]].concat())),
    ];
    for home_value in homes {
        for (line, kernel) in [
            (&["status", "--kernel", "x"][..], None),
            (&["status"], Some(OsString::from("x"))),
            (&["status"], None),
        ] {
            let vars = Vars {
                home: home_value.clone(),
                kernel,
            };
            let envelope = failure(&run(line, vars), Exit::Refused);
            assert_eq!(envelope["command"], "plasmosome.status");
            let error = &envelope["error"];
            assert_eq!(error["code"], "invalid_home", "{home_value:?} {line:?}");
            assert_eq!(keys(error), ["code", "domain", "message"], "{error}");
            assert!(envelope.get("instance").is_none(), "{envelope}");
        }
    }
    assert_eq!(x.connections(), 0, "an invalid HOME reached an instance");

    let slashed = Vars {
        home: Some(format!("{spelled}/").into()),
        kernel: None,
    };
    for line in [&["status", "--kernel", "x"][..], &["status"]] {
        let envelope = success(&run(line, slashed.clone()));
        assert_eq!(
            envelope["instance"],
            json!({"name": "x", "socket": format!("{spelled}/.plasmosome/instances/x/control.uds")})
        );
    }
    assert_eq!(requests(&x, 2).len(), 2);
}

#[test]
fn instance_says_whether_a_socket_was_chosen() {
    let home = Home::new();
    home.stale("x");
    let cases: [(&[&str], Vars, bool); 7] = [
        (&["status", "--kernel", "z"], home.vars(), true),
        (&["status", "--kernel", "x"], home.vars(), true),
        (&["status"], home.vars(), true),
        (&["status"], home.with_kernel("../x"), false),
        (&["status", "--kernel", "../x"], home.vars(), false),
        (&["cell", "list"], home.with_kernel("../x"), false),
        (&["status", "--kernel", "x"], Vars::default(), false),
    ];
    for (line, vars, present) in cases {
        let ran = run(line, vars);
        assert_ne!(ran.exit, Exit::Done, "{line:?}");
        let envelope = one_line(&ran.stderr);
        assert_eq!(
            envelope.get("instance").is_some(),
            present,
            "{line:?}: {envelope}"
        );
    }
}

#[test]
fn an_oversized_name_from_the_variable_names_the_variable() {
    let home = Home::new();
    let a = home.listening("a");
    let name = "a".repeat(1_048_576);
    let envelope = failure(&run(&["status"], home.with_kernel(&name)), Exit::Usage);
    assert_eq!(envelope["command"], "invalid");
    assert_eq!(envelope["error"]["code"], "invalid_argument");
    assert_eq!(envelope["error"]["argument"], VARIABLE);
    assert_eq!(envelope["instance"]["name"], name.as_str());
    assert_never_reached(&a, "an oversized request");
}

fn datagram(home: &Home, name: &str) -> std::os::unix::net::UnixDatagram {
    home.dir(name);
    let socket = home.socket(name);
    let datagram = std::os::unix::net::UnixDatagram::bind(&socket).expect("a datagram socket");
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).expect("0600");
    datagram
}

#[cfg(target_os = "macos")]
#[test]
fn a_lone_candidate_that_cannot_be_connected_to_is_transport() {
    let eprototype = 41;
    let home = Home::new();
    let _datagram = datagram(&home, "b");
    let envelope = failure(&run(&["status"], home.vars()), Exit::Unavailable);
    let error = &envelope["error"];
    assert_eq!(error["code"], "transport", "{error}");
    assert_eq!(error["path"], text(&home.socket("b")));
    assert_eq!(error["errno"], eprototype);
    assert_eq!(
        envelope["instance"],
        json!({"name": "b", "socket": text(&home.socket("b"))})
    );
}

#[test]
fn several_candidates_are_ambiguous_whatever_each_connect_gives() {
    let home = Home::new();
    let a = home.live("a", 1);
    let _datagram = datagram(&home, "b");
    let before = home.stale("c");
    let envelope = failure(&run(&["status"], home.vars()), Exit::Refused);
    let error = &envelope["error"];
    assert_eq!(error["code"], "ambiguous_instance", "{error}");
    assert_eq!(error["candidates"], json!(["a", "b", "c"]));
    assert_eq!(error["unreachable"], json!(["b", "c"]));
    assert!(envelope.get("instance").is_none(), "{envelope}");
    assert_eq!(a.ended(1), [Vec::<u8>::new()]);
    assert_eq!(inode(&home.socket("c")), before);
}

fn too_long(home: &Home) -> String {
    let around = text(&home.instances()).len() + "/".len() + "/control.uds".len();
    let name = "s".repeat(MAX_PATH + 1 - around);
    assert_eq!(home.socket(&name).as_os_str().len(), MAX_PATH + 1);
    name
}

#[test]
fn a_stopped_instance_with_a_long_name_blocks_nothing() {
    let home = Home::new();
    let long = too_long(&home);
    home.dir(&long);
    let a = home.live("a", 1);

    let envelope = success(&run(&["status"], home.vars()));
    assert_eq!(
        envelope["instance"],
        json!({"name": "a", "socket": text(&home.socket("a"))})
    );
    assert_eq!(requests(&a, 1)[0]["params"], json!({"name": "a"}));
    drop(a);
    fs::remove_dir_all(home.instances().join("a")).expect("a is removed");

    let envelope = failure(&run(&["status"], home.vars()), Exit::Unavailable);
    let error = &envelope["error"];
    assert_eq!(error["code"], "no_instance", "{error}");
    assert_eq!(error["seen"], json!([long]));
}

#[test]
fn a_candidate_whose_path_is_too_long_refuses_the_selection() {
    let home = Home::new();
    let a = home.listening("a");
    let long = too_long(&home);
    home.dir(&long);
    let elsewhere = Home::new();
    let bound = elsewhere.path().join("s.uds");
    let listener = UnixListener::bind(&bound).expect("a socket at a short path");
    listener
        .set_nonblocking(true)
        .expect("the listener does not block");
    let socket = home.socket(&long);
    fs::rename(&bound, &socket).expect("the socket moves to the long path");
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).expect("0600");

    let envelope = failure(&run(&["status"], home.vars()), Exit::Refused);
    let error = &envelope["error"];
    assert_eq!(error["code"], "socket_path_too_long", "{error}");
    assert_eq!(error["path"], text(&socket));
    assert_eq!(error["max_bytes"], MAX_PATH);
    assert!(envelope.get("instance").is_none(), "{envelope}");
    assert_never_reached(&a, "a selection with a candidate too long to connect to");
    assert_never_reached(&listener, "a selection with a candidate too long to connect to");
}
