#[allow(dead_code, reason = "this file uses part of the shared support")]
mod support;

use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use plasmosome_cli::json::Json;
use plasmosome_cli::{
    ArgSource, Build, COMMANDS, ClientCode, ClientError, Clock, Command, CommandArgs, Context, Env,
    Exit, Flag, FlagKind, InstanceParam, Param, SystemClock, VerbCall, VerbDeadline, handle,
};
use plasmosome_core::private_socket::effective_uid;
use serde_json::{Value, json};
use support::{Answer, FakeDaemon, PATIENCE, Vars, base, private_root};
use tempfile::TempDir;

const BOUND: usize = 1_048_576;

const EPIPE: i32 = 32;

const CELL: Flag = Flag {
    name: "--cell",
    key: "cell",
    kind: FlagKind::Text,
};

const NOW: Flag = Flag {
    name: "--now",
    key: "now",
    kind: FlagKind::Bare,
};

const MOCK: Flag = Flag {
    name: "--mock",
    key: "mock",
    kind: FlagKind::Mock,
};

fn call(
    method: &'static str,
    args: &CommandArgs,
    flags: &[Flag],
    max_ms: Option<Option<u64>>,
) -> VerbCall {
    VerbCall {
        method,
        instance: InstanceParam::Kernel,
        params: flags.iter().filter_map(|flag| args.flag(flag)).collect(),
        deadline: max_ms.map(|max_ms| VerbDeadline { max_ms }),
    }
}

fn kill(args: &CommandArgs) -> Result<VerbCall, ClientError> {
    Ok(call("cell.kill", args, &[CELL], None))
}

fn look(args: &CommandArgs) -> Result<VerbCall, ClientError> {
    Ok(call("cell.status", args, &[CELL], None))
}

fn wait(args: &CommandArgs) -> Result<VerbCall, ClientError> {
    Ok(call("exec.status", args, &[CELL], Some(None)))
}

fn exec(args: &CommandArgs) -> Result<VerbCall, ClientError> {
    let mut call = call("cell.exec", args, &[CELL], Some(Some(600_000)));
    call.params.extend(args.argv("argv"));
    Ok(call)
}

fn germinate(args: &CommandArgs) -> Result<VerbCall, ClientError> {
    let mut call = call("cell.new", args, &[CELL, NOW, MOCK], None);
    if let Some(genome) = args.positional("GENOME", "genome") {
        call.params.insert(0, genome);
    }
    Ok(call)
}

fn refuses(_: &CommandArgs) -> Result<VerbCall, ClientError> {
    let argument = vec![("argument".to_string(), Json::String("--x".to_string()))];
    let error = ClientError::new(ClientCode::InvalidArgument, "refused".to_string(), argument);
    Err(error.expect("a client error"))
}

fn fails(_: &CommandArgs) -> Result<VerbCall, ClientError> {
    Err(ClientError::new(ClientCode::Internal, "failed".to_string(), Vec::new()).expect("an error"))
}

fn repeats(args: &CommandArgs) -> Result<VerbCall, ClientError> {
    let text = |value: &str| Json::String(value.to_string());
    let param = |key, value| Param {
        key,
        value,
        source: ArgSource::Flag("--cell"),
    };
    let case = args.flag(&CELL).map(|cell| match &cell.value {
        Json::String(case) => case.clone(),
        _ => String::new(),
    });
    let params = match case.as_deref() {
        Some("name") => vec![param("name", text("x"))],
        Some("deadline") => vec![param("deadline_ms", text("x"))],
        Some("twice") => vec![param("a", text("x")), param("a", text("y"))],
        _ => {
            let inner = vec![("b".to_string(), text("1")), ("b".to_string(), text("2"))];
            vec![param("a", Json::Object(inner))]
        }
    };
    let mut call = call("plasmosome.recovery", args, &[], Some(None));
    call.instance = InstanceParam::Name;
    call.params = params;
    Ok(call)
}

fn table() -> Vec<Command> {
    COMMANDS
        .iter()
        .map(|command| {
            let (flags, takes_argv, default_deadline_ms, build): (&'static [Flag], _, _, Build) =
                match command.words {
                    ["cell", "kill"] => (&[CELL], false, 10_000, kill),
                    ["cell", "status"] => (&[CELL], false, 10_000, look),
                    ["exec", "status"] => (&[CELL], false, 10_000, wait),
                    ["cell", "exec"] => (&[CELL], true, 30_000, exec),
                    ["germinate"] => (&[CELL, NOW, MOCK], false, 10_000, germinate),
                    ["cell", "new"] => (&[], false, 10_000, refuses),
                    ["list"] => (&[], false, 10_000, fails),
                    ["recovery"] => (&[CELL], false, 10_000, repeats),
                    _ => return *command,
                };
            Command {
                served: true,
                flags,
                takes_argv,
                default_deadline_ms,
                build: Some(build),
                ..*command
            }
        })
        .collect()
}

struct Ran {
    exit: Exit,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    took: Duration,
}

struct Closed;

impl Write for Closed {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::from_raw_os_error(EPIPE))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn run_with(
    line: &[&str],
    env: impl Env + Send + 'static,
    clock: &'static dyn Clock,
    closed_stdout: bool,
) -> Ran {
    let shown: String = line
        .iter()
        .take(6)
        .flat_map(|arg| arg.chars().take(20).chain([' ']))
        .collect();
    let line: Vec<OsString> = line.iter().map(OsString::from).collect();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let commands = table();
        let context = Context {
            version: "0.0.0",
            commands: &commands,
            trusted_uid: effective_uid(),
            clock,
            env: &env,
        };
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let started = Instant::now();
        let exit = if closed_stdout {
            handle(&line, &context, &mut Closed, &mut stderr)
        } else {
            handle(&line, &context, &mut stdout, &mut stderr)
        };
        let took = started.elapsed();
        let _ = sender.send(Ran {
            exit,
            stdout,
            stderr,
            took,
        });
    });
    receiver
        .recv_timeout(PATIENCE)
        .unwrap_or_else(|_| panic!("{shown} did not return within {PATIENCE:?}"))
}

fn run(line: &[&str]) -> Ran {
    run_with(line, Vars::default(), &SystemClock, false)
}

fn one_line(bytes: &[u8]) -> Value {
    let text = std::str::from_utf8(bytes).expect("the output is UTF-8");
    assert!(
        text.ends_with('\n') && text.matches('\n').count() == 1,
        "the output is not one line: {}",
        &text[..text.len().min(300)]
    );
    serde_json::from_str(text).expect("the output is one JSON line")
}

fn outcome(ran: &Ran, exit: Exit) -> Value {
    assert_eq!(
        ran.exit,
        exit,
        "{}",
        String::from_utf8_lossy(&ran.stderr[..ran.stderr.len().min(300)])
    );
    if exit == Exit::Done {
        assert!(ran.stderr.is_empty(), "a success wrote to stderr");
        one_line(&ran.stdout)
    } else {
        assert!(ran.stdout.is_empty(), "a failure wrote to stdout");
        one_line(&ran.stderr)
    }
}

fn usage(ran: &Ran) -> Value {
    let envelope = outcome(ran, Exit::Usage);
    assert_eq!(envelope["command"], "invalid");
    assert_eq!(envelope["error"]["code"], "invalid_argument");
    envelope
}

fn ok() -> Answer {
    Answer::Reply(b"{\"id\":1,\"result\":{}}\n".to_vec())
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a UTF-8 path")
}

fn the_one_request(fake: &FakeDaemon) -> Vec<u8> {
    let mut sent = fake.ended(1);
    assert_eq!(
        fake.connections(),
        1,
        "the request went on a second connection"
    );
    sent.remove(0)
}

fn listening(socket: &Path) -> UnixListener {
    let listener = UnixListener::bind(socket).expect("a socket");
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600)).expect("the socket is 0600");
    listener
        .set_nonblocking(true)
        .expect("the listener does not block");
    listener
}

fn assert_never_reached(listener: &UnixListener) {
    let accepted = listener.accept().map(|_| ());
    assert!(
        matches!(&accepted, Err(error) if error.kind() == io::ErrorKind::WouldBlock),
        "the listener saw {accepted:?}"
    );
}

struct Home(TempDir);

impl Home {
    fn new() -> Home {
        let root = tempfile::Builder::new()
            .prefix("h")
            .rand_bytes(4)
            .tempdir_in(base())
            .expect("a test HOME");
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).expect("0700");
        Home(root)
    }

    fn socket(&self, name: &str) -> PathBuf {
        let dir = self.0.path().join(".plasmosome/instances").join(name);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .expect("the instance directory");
        dir.join("control.uds")
    }

    fn vars(&self, kernel: Option<&str>) -> Vars {
        Vars {
            home: Some(self.0.path().into()),
            kernel: kernel.map(OsString::from),
        }
    }
}

#[derive(Clone)]
struct Watched {
    home: Option<OsString>,
    kernel: Option<OsString>,
    moves: Option<&'static Moving>,
    read: Arc<AtomicBool>,
}

impl Watched {
    fn home(home: Option<&Path>) -> Watched {
        Watched {
            home: home.map(OsString::from),
            kernel: None,
            moves: None,
            read: Arc::new(AtomicBool::new(false)),
        }
    }

    fn naming(self, kernel: &str) -> Watched {
        Watched {
            kernel: Some(kernel.into()),
            ..self
        }
    }

    fn was_read(&self) -> bool {
        self.read.load(Ordering::SeqCst)
    }
}

impl Env for Watched {
    fn home(&self) -> Option<OsString> {
        self.read.store(true, Ordering::SeqCst);
        if let Some(clock) = self.moves {
            clock.moved.store(true, Ordering::SeqCst);
        }
        self.home.clone()
    }

    fn kernel(&self) -> Option<OsString> {
        self.kernel.clone()
    }
}

struct Moving {
    start: Instant,
    by_ms: u64,
    moved: AtomicBool,
    daemon: Option<(UnixListener, mpsc::Sender<Vec<u8>>)>,
}

impl Clock for Moving {
    fn now(&self) -> Instant {
        if let Some((listener, requests)) = &self.daemon
            && let Ok((stream, _)) = listener.accept()
        {
            self.moved.store(true, Ordering::SeqCst);
            let requests = requests.clone();
            thread::spawn(move || answer_one(&stream, &requests));
        }
        let offset = if self.moved.load(Ordering::SeqCst) {
            self.by_ms
        } else {
            0
        };
        self.start + Duration::from_millis(offset)
    }
}

fn moving(by_ms: u64, daemon: Option<(UnixListener, mpsc::Sender<Vec<u8>>)>) -> &'static Moving {
    Box::leak(Box::new(Moving {
        start: Instant::now(),
        by_ms,
        moved: AtomicBool::new(false),
        daemon,
    }))
}

fn answer_one(stream: &UnixStream, requests: &mpsc::Sender<Vec<u8>>) {
    stream.set_nonblocking(false).expect("the stream blocks");
    stream.set_read_timeout(Some(PATIENCE)).expect("a timeout");
    let mut request = Vec::new();
    BufReader::new(stream)
        .read_until(b'\n', &mut request)
        .expect("the request arrives");
    let mut writer = stream;
    writer
        .write_all(b"{\"id\":1,\"result\":{}}\n")
        .expect("the answer is written");
    let _ = requests.send(request);
}

struct Scripted {
    start: Instant,
    offsets_ms: &'static [u64],
    reads: AtomicUsize,
}

impl Clock for Scripted {
    fn now(&self) -> Instant {
        let reading = self.reads.fetch_add(1, Ordering::Relaxed);
        let offset = self.offsets_ms[reading.min(self.offsets_ms.len() - 1)];
        self.start + Duration::from_millis(offset)
    }
}

fn scripted(offsets_ms: &'static [u64]) -> &'static dyn Clock {
    Box::leak(Box::new(Scripted {
        start: Instant::now(),
        offsets_ms,
        reads: AtomicUsize::new(0),
    }))
}

#[test]
fn every_target_sends_the_one_request_line() {
    let root = private_root();
    let socket = root.path().join("control.uds");
    let fake = FakeDaemon::start(&socket, vec![ok()]);
    let ran = run(&["cell", "kill", "--cell", "c-1", "--socket", text(&socket)]);
    let envelope = outcome(&ran, Exit::Done);
    assert_eq!(envelope["command"], "cell.kill");
    assert_eq!(envelope["instance"], json!({"socket": text(&socket)}));
    assert_eq!(
        the_one_request(&fake),
        b"{\"id\":1,\"method\":\"cell.kill\",\"params\":{\"cell\":\"c-1\"}}\n"
    );

    let home = Home::new();
    let socket = home.socket("work");
    let work = FakeDaemon::start(&socket, vec![ok(), ok(), ok()]);
    for (line, vars) in [
        (
            &["cell", "kill", "--kernel", "work", "--cell", "c-1"][..],
            home.vars(None),
        ),
        (&["cell", "kill", "--cell", "c-1"], home.vars(Some("work"))),
        (&["cell", "kill", "--cell", "c-1"], home.vars(None)),
    ] {
        let envelope = outcome(&run_with(line, vars, &SystemClock, false), Exit::Done);
        assert_eq!(
            envelope["instance"],
            json!({"name": "work", "socket": text(&socket)})
        );
    }
    let line =
        b"{\"id\":1,\"method\":\"cell.kill\",\"params\":{\"kernel\":\"work\",\"cell\":\"c-1\"}}\n";
    assert_eq!(work.ended(3), [line, line, line]);
    assert_eq!(work.connections(), 3);
}

#[test]
fn a_state_change_that_loses_its_answer_exits_3() {
    let lost = b"{\"id\":1,\"error\":{\"code\":105,\"message\":\"m\",\"from\":\"running\",\"to\":\"unknown\"}}\n";
    for (command, changes_state) in [("kill", true), ("status", false)] {
        let cases = [
            (Answer::Close, &[][..], false, "closed"),
            (Answer::Silent, &["--deadline-ms", "300"], false, "deadline"),
            (Answer::Endless, &[], false, "reply_too_large"),
            (ok(), &[], true, "local_io"),
            (Answer::Reply(lost.to_vec()), &[], false, "kernel"),
        ];
        for (answer, extra, closed_stdout, cause) in cases {
            let root = private_root();
            let socket = root.path().join("control.uds");
            let fake = FakeDaemon::start(&socket, vec![answer]);
            let mut line = vec!["cell", command, "--cell", "c", "--socket", text(&socket)];
            line.extend(extra);
            let clock: &'static dyn Clock = if cause == "deadline" {
                scripted(&[0])
            } else {
                &SystemClock
            };
            let ran = run_with(&line, Vars::default(), clock, closed_stdout);
            let expected = match (changes_state, cause) {
                (_, "kernel") | (true, _) => Exit::OutcomeUnknown,
                (false, "reply_too_large") => Exit::Refused,
                (false, _) => Exit::Unavailable,
            };
            let error = &outcome(&ran, expected)["error"];
            let case = format!("cell {command} against {cause}: {error}");
            if cause == "kernel" {
                let mut relayed: Value = serde_json::from_slice(lost).expect("JSON");
                relayed["error"]["domain"] = json!("kernel");
                assert_eq!(error, &relayed["error"], "{case}");
            } else if changes_state {
                assert_eq!(error["code"], "outcome_unknown", "{case}");
                assert_eq!(error["cause"], cause, "{case}");
                if cause == "local_io" {
                    let message = error["message"].as_str().unwrap_or_default();
                    assert!(message.starts_with("the daemon answered, but"), "{case}");
                }
            } else if cause == "closed" || cause == "deadline" {
                assert_eq!(error["code"], "no_reply", "{case}");
                assert_eq!(error["cause"], cause, "{case}");
            } else {
                assert_eq!(error["code"], cause, "{case}");
            }
            if cause == "deadline" {
                let took = ran.took;
                assert!(
                    took >= Duration::from_millis(300) && took < Duration::from_secs(2),
                    "{took:?}"
                );
            }
            let request = format!(
                "{{\"id\":1,\"method\":\"cell.{command}\",\"params\":{{\"cell\":\"c\"}}}}\n"
            );
            assert_eq!(the_one_request(&fake), request.as_bytes(), "{case}");
        }
    }
    let root = private_root();
    let socket = root.path().join("control.uds");
    let fake = FakeDaemon::start(&socket, vec![Answer::Close]);
    let error = &outcome(
        &run(&["germinate", "g", "--socket", text(&socket)]),
        Exit::OutcomeUnknown,
    )["error"];
    assert_eq!(error["code"], "outcome_unknown", "{error}");
    assert_eq!(error["cause"], "closed", "{error}");
    assert_eq!(
        the_one_request(&fake),
        b"{\"id\":1,\"method\":\"cell.new\",\"params\":{\"genome\":\"g\"}}\n"
    );
}

fn terminal_controls() -> Vec<char> {
    [
        (0x00, 0x1f),
        (0x7f, 0x7f),
        (0x80, 0x9f),
        (0x061c, 0x061c),
        (0x200e, 0x200f),
        (0x202a, 0x202e),
        (0x2066, 0x2069),
    ]
    .into_iter()
    .flat_map(|(first, last)| first..=last)
    .map(|code| char::from_u32(code).expect("a code point"))
    .collect()
}

#[test]
fn a_sent_line_escapes_every_control_and_is_one_line() {
    let controls = terminal_controls();
    assert_eq!(
        controls.len(),
        77,
        "spec 025 section 3 names 77 code points"
    );
    let [escape, newline, backslash] =
        [0x1b, 0x0a, 0x5c].map(|code| char::from_u32(code).expect("a character"));
    let mut cases: Vec<(String, String)> = controls
        .iter()
        .map(|&control| {
            let written = format!("\"a{backslash}u{:04x}b\"", u32::from(control));
            (format!("a{control}b"), written)
        })
        .collect();
    cases.push((
        format!("a{escape}{newline}b\"c{backslash}"),
        format!("\"a{backslash}u001b{backslash}u000ab{backslash}\"c{backslash}{backslash}\""),
    ));
    let root = private_root();
    let socket = root.path().join("control.uds");
    let fake = FakeDaemon::start(&socket, cases.iter().map(|_| ok()).collect());
    for (cell, _) in &cases {
        let line = ["cell", "kill", "--cell", cell, "--socket", text(&socket)];
        outcome(&run(&line), Exit::Done);
    }
    let sent = fake.ended(cases.len());
    assert_eq!(fake.connections(), cases.len());
    for ((cell, written), sent) in cases.iter().zip(sent) {
        let expected = format!(
            "{{\"id\":1,\"method\":\"cell.kill\",\"params\":{{\"cell\":{written}}}}}{newline}"
        );
        assert_eq!(String::from_utf8(sent), Ok(expected), "{cell:?}");
    }
}

#[test]
fn the_verb_deadline_is_the_time_left_before_the_first_byte() {
    let cases: [(&[&str], &'static [u64], Option<u64>); 8] = [
        (
            &["exec", "status", "--deadline-ms", "10000"],
            &[0, 0],
            Some(9_000),
        ),
        (
            &["exec", "status", "--deadline-ms", "10000"],
            &[0, 9_700],
            Some(270),
        ),
        (
            &["exec", "status", "--deadline-ms", "10000"],
            &[0, 9_999],
            Some(1),
        ),
        (
            &["exec", "status", "--deadline-ms", "3600000"],
            &[0, 0],
            Some(3_599_000),
        ),
        (
            &["cell", "exec", "--deadline-ms", "3600000"],
            &[0, 0],
            Some(600_000),
        ),
        (&["exec", "status"], &[0, 0], Some(9_000)),
        (&["cell", "exec"], &[0, 0], Some(29_000)),
        (&["cell", "kill"], &[0, 0], None),
    ];
    for (words, offsets_ms, expected) in cases {
        let root = private_root();
        let socket = root.path().join("control.uds");
        let fake = FakeDaemon::start(&socket, vec![ok()]);
        let mut line = words.to_vec();
        line.extend(["--socket", text(&socket)]);
        run_with(&line, Vars::default(), scripted(offsets_ms), false);
        let request: Value =
            serde_json::from_slice(&the_one_request(&fake)).expect("the request is JSON");
        let sent = request["params"].get("deadline_ms").map(Value::as_u64);
        assert_eq!(
            sent,
            expected.map(Some),
            "{words:?} at {offsets_ms:?}: {request}"
        );
    }
}

#[test]
fn the_verb_deadline_is_read_after_home_and_after_the_connect() {
    let home = Home::new();
    let work = FakeDaemon::start(&home.socket("work"), vec![ok()]);
    let clock = moving(5_000, None);
    let env = Watched {
        moves: Some(clock),
        ..Watched::home(Some(home.0.path()))
    };
    let line = ["exec", "status", "--kernel", "work"];
    outcome(&run_with(&line, env.clone(), clock, false), Exit::Done);
    assert!(env.was_read(), "HOME was not read");
    let request: Value = serde_json::from_slice(&the_one_request(&work)).expect("JSON");
    assert_eq!(request["params"]["deadline_ms"], 4_500, "{request}");

    let root = private_root();
    let socket = root.path().join("control.uds");
    let (requests, sent) = mpsc::channel();
    let clock = moving(5_000, Some((listening(&socket), requests)));
    let line = ["exec", "status", "--socket", text(&socket)];
    outcome(&run_with(&line, Vars::default(), clock, false), Exit::Done);
    let request = sent.recv_timeout(PATIENCE).expect("one request");
    let request: Value = serde_json::from_slice(&request).expect("JSON");
    assert_eq!(request["params"]["deadline_ms"], 4_500, "{request}");
}

fn filler(empty_line: &str, line_bytes: usize) -> String {
    "a".repeat(line_bytes - empty_line.len())
}

#[test]
fn an_oversized_request_is_refused_before_home_is_read() {
    let control = char::from_u32(1).expect("U+0001 is a code point");
    let genome: String = std::iter::repeat_n(control, 180_000).collect();
    let base = base();
    for (extra, variable) in [
        (&[][..], None),
        (&["--kernel", "work"], None),
        (&[], Some("work")),
    ] {
        let watched = Watched::home(Some(&base));
        let watched = variable.map_or(watched.clone(), |name| watched.naming(name));
        let mut line = vec!["germinate", genome.as_str()];
        line.extend(extra);
        let envelope = usage(&run_with(&line, watched.clone(), &SystemClock, false));
        assert_eq!(envelope["error"]["argument"], genome.as_str());
        assert!(envelope.get("instance").is_none(), "{extra:?} {variable:?}");
        assert!(
            !watched.was_read(),
            "HOME was read for {extra:?} {variable:?}"
        );
    }
    let root = private_root();
    let socket = root.path().join("control.uds");
    let listener = listening(&socket);
    let envelope = usage(&run(&["germinate", &genome, "--socket", text(&socket)]));
    assert_eq!(envelope["error"]["argument"], genome.as_str());
    assert_eq!(envelope["instance"], json!({"socket": text(&socket)}));
    assert_never_reached(&listener);
}

#[test]
fn a_line_of_exactly_the_bound_is_sent() {
    let root = private_root();
    let socket = root.path().join("control.uds");
    let fake = FakeDaemon::start(&socket, vec![ok()]);
    let genome = filler(
        "{\"id\":1,\"method\":\"cell.new\",\"params\":{\"genome\":\"\"}}",
        BOUND,
    );
    let variable = Vars {
        home: None,
        kernel: Some("work".into()),
    };
    let line = ["germinate", &genome, "--socket", text(&socket)];
    outcome(&run_with(&line, variable, &SystemClock, false), Exit::Done);
    assert_eq!(the_one_request(&fake).len(), BOUND + 1);
    drop(fake);
    let listener = listening(&socket);
    let longer = format!("{genome}a");
    let envelope = usage(&run(&["germinate", &longer, "--socket", text(&socket)]));
    assert_eq!(envelope["error"]["argument"], longer.as_str());
    assert_never_reached(&listener);

    let home = Home::new();
    let named = FakeDaemon::start(&home.socket("w"), vec![ok(), ok()]);
    let genome = filler(&genome_line("w"), BOUND);
    for line in [
        &["germinate", &genome, "--kernel", "w"][..],
        &["germinate", &genome],
    ] {
        let ran = run_with(line, home.vars(None), &SystemClock, false);
        outcome(&ran, Exit::Done);
    }
    assert!(named.ended(2).iter().all(|sent| sent.len() == BOUND + 1));
    let watched = Watched::home(Some(home.0.path()));
    let longer = format!("{genome}a");
    let ran = run_with(
        &["germinate", &longer],
        watched.clone(),
        &SystemClock,
        false,
    );
    let envelope = usage(&ran);
    assert_eq!(envelope["error"]["argument"], longer.as_str());
    assert!(envelope.get("instance").is_none());
    assert!(!watched.was_read(), "HOME was read");
    assert_eq!(named.connections(), 2);

    let home = Home::new();
    let work = FakeDaemon::start(&home.socket("work"), vec![ok()]);
    let genome = filler(&genome_line("work"), BOUND);
    let ran = run_with(
        &["germinate", &genome],
        home.vars(Some("work")),
        &SystemClock,
        false,
    );
    outcome(&ran, Exit::Done);
    assert_eq!(the_one_request(&work).len(), BOUND + 1);
    let watched = Watched::home(Some(home.0.path())).naming("work");
    let longer = format!("{genome}a");
    let ran = run_with(
        &["germinate", &longer],
        watched.clone(),
        &SystemClock,
        false,
    );
    assert_eq!(usage(&ran)["error"]["argument"], longer.as_str());
    assert!(!watched.was_read(), "HOME was read");
    assert_eq!(work.connections(), 1);
}

fn genome_line(kernel: &str) -> String {
    format!(
        "{{\"id\":1,\"method\":\"cell.new\",\"params\":{{\"kernel\":\"{kernel}\",\"genome\":\"\"}}}}"
    )
}

#[test]
fn default_selection_measures_the_found_name_before_connecting() {
    let home = Home::new();
    let socket = home.socket("work");
    let work = FakeDaemon::start(&socket, vec![ok()]);
    let genome = filler(&genome_line("work"), BOUND);
    let ran = run_with(
        &["germinate", &genome],
        home.vars(None),
        &SystemClock,
        false,
    );
    outcome(&ran, Exit::Done);
    assert_eq!(the_one_request(&work).len(), BOUND + 1);
    let longer = format!("{genome}a");
    let ran = run_with(
        &["germinate", &longer],
        home.vars(None),
        &SystemClock,
        false,
    );
    let envelope = usage(&ran);
    assert_eq!(envelope["error"]["argument"], longer.as_str());
    assert_eq!(
        envelope["instance"],
        json!({"name": "work", "socket": text(&socket)})
    );
    assert_eq!(work.connections(), 1);
}

#[test]
fn the_bound_counts_the_deadline_the_call_can_send() {
    for (words, extra, sent_ms) in [
        (["exec", "status"], &[][..], "9000"),
        (["cell", "exec"], &["--deadline-ms", "3600000"], "600000"),
    ] {
        let root = private_root();
        let socket = root.path().join("control.uds");
        let fake = FakeDaemon::start(&socket, vec![ok()]);
        let method = words.join(".");
        let empty = format!(
            "{{\"id\":1,\"method\":\"{method}\",\"params\":{{\"cell\":\"\",\"deadline_ms\":{sent_ms}}}}}"
        );
        let cell = filler(&empty, BOUND);
        let tail = [&["--socket", text(&socket)][..], extra].concat();
        let exact = [&[words[0], words[1], "--cell", &cell][..], &tail].concat();
        let ran = run_with(&exact, Vars::default(), scripted(&[0, 0]), false);
        outcome(&ran, Exit::Done);
        let sent = the_one_request(&fake);
        assert_eq!(sent.len(), BOUND + 1, "{words:?}");
        let ending = format!(",\"deadline_ms\":{sent_ms}}}}}\n");
        assert!(sent.ends_with(ending.as_bytes()), "{words:?}");
        drop(fake);
        let listener = listening(&socket);
        let longer = format!("{cell}a");
        let over = [&[words[0], words[1], "--cell", &longer][..], &tail].concat();
        let envelope = usage(&run(&over));
        assert_eq!(envelope["error"]["argument"], longer.as_str(), "{words:?}");
        assert_never_reached(&listener);
    }

    let home = Home::new();
    let socket = home.socket("work");
    let work = FakeDaemon::start(&socket, vec![ok()]);
    let empty = "{\"id\":1,\"method\":\"cell.exec\",\"params\":{\"kernel\":\"work\",\"cell\":\"\",\"deadline_ms\":600000}}";
    let cell = filler(empty, BOUND);
    let longer = format!("{cell}a");
    let deadline = ["--deadline-ms", "3600000"];
    let over = [&["cell", "exec", "--cell", &longer][..], &deadline].concat();
    let envelope = usage(&run_with(&over, home.vars(None), scripted(&[0, 0]), false));
    assert_eq!(envelope["error"]["argument"], longer.as_str());
    assert_eq!(
        envelope["instance"],
        json!({"name": "work", "socket": text(&socket)})
    );
    assert_eq!(work.connections(), 0);
    let exact = [&["cell", "exec", "--cell", &cell][..], &deadline].concat();
    let ran = run_with(&exact, home.vars(None), scripted(&[0, 0]), false);
    outcome(&ran, Exit::Done);
    let sent = the_one_request(&work);
    assert_eq!(sent.len(), BOUND + 1);
    assert!(sent.ends_with(b",\"deadline_ms\":600000}}\n"));
}

#[test]
fn the_argument_that_contributes_most_is_named_and_the_first_of_equals() {
    let root = private_root();
    let socket = root.path().join("control.uds");
    let listener = listening(&socket);
    let (a, b) = ("a".repeat(600_000), "b".repeat(600_000));
    let mock = format!("--mock={}", "m".repeat(1_100_000));
    let control = char::from_u32(1).expect("U+0001 is a code point");
    let genome: String = std::iter::repeat_n(control, 180_000).collect();
    let cell = "a".repeat(500_000);
    for (case, (line, named)) in [
        (vec!["germinate", &a, "--cell", &b], a.as_str()),
        (vec!["germinate", &genome, "--cell", &cell], genome.as_str()),
        (vec!["germinate", "--cell", &b, &a], b.as_str()),
        (vec!["germinate", "--now", "g", &mock], mock.as_str()),
        (
            vec!["cell", "exec", "--cell", "c", "--", "run", &a, &b],
            "--",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let line = [&["--socket", text(&socket)][..], &line].concat();
        let envelope = usage(&run(&line));
        assert_eq!(envelope["error"]["argument"], named, "case {case}");
    }
    assert_never_reached(&listener);

    let home = Home::new();
    let watched = Watched::home(Some(home.0.path()));
    let envelope = usage(&run_with(
        &["germinate", &b, "--kernel", &a],
        watched.clone(),
        &SystemClock,
        false,
    ));
    assert_eq!(envelope["error"]["argument"], b.as_str());
    assert!(!watched.was_read(), "HOME was read");
    let variable = Watched::home(Some(home.0.path())).naming(&a);
    let ran = run_with(&["germinate", &b], variable.clone(), &SystemClock, false);
    assert_eq!(usage(&ran)["error"]["argument"], b.as_str());
    assert!(!variable.was_read(), "HOME was read");
    let ran = run_with(
        &["germinate", "--kernel", &a, &b],
        home.vars(None),
        &SystemClock,
        false,
    );
    let envelope = usage(&ran);
    assert_eq!(envelope["error"]["argument"], a.as_str());
    assert_eq!(envelope["instance"]["name"], a.as_str());
}

#[test]
fn two_commands_are_equal_only_with_the_same_build() {
    let entry = *table()
        .iter()
        .find(|command| command.words == ["cell", "kill"])
        .expect("cell kill is in the table");
    let with = |build: Option<Build>| Command { build, ..entry };
    assert_eq!(with(Some(kill)), entry);
    assert_ne!(with(Some(look)), entry);
    assert_ne!(with(None), entry);
    assert_eq!(with(None), with(None));
    let others = [
        Command {
            words: &["cell", "status"],
            ..entry
        },
        Command {
            operands: &["CELL"],
            ..entry
        },
        Command {
            command: "cell.status",
            ..entry
        },
        Command {
            is_method: false,
            ..entry
        },
        Command {
            changes_state: false,
            ..entry
        },
        Command {
            served: false,
            ..entry
        },
        Command {
            flags: &[],
            ..entry
        },
        Command {
            takes_argv: true,
            ..entry
        },
        Command {
            default_deadline_ms: 10_001,
            ..entry
        },
    ];
    for (field, other) in others.iter().enumerate() {
        assert_ne!(*other, entry, "field {field}");
    }
}

#[test]
fn a_build_refusal_and_a_repeated_key_reach_nothing() {
    let unread = Watched::home(None);
    let envelope = usage(&run_with(
        &["cell", "new"],
        unread.clone(),
        &SystemClock,
        false,
    ));
    assert_eq!(envelope["error"]["argument"], "--x");
    let envelope = outcome(
        &run_with(&["list"], unread.clone(), &SystemClock, false),
        Exit::Internal,
    );
    assert_eq!(envelope["command"], "plasmosome.list");
    for case in ["name", "deadline", "twice", "nested"] {
        let ran = run_with(
            &["recovery", "--cell", case],
            unread.clone(),
            &SystemClock,
            false,
        );
        let envelope = outcome(&ran, Exit::Internal);
        assert_eq!(envelope["command"], "plasmosome.recovery", "{case}");
        assert_eq!(envelope["error"]["code"], "internal", "{case}");
    }
    assert!(!unread.was_read(), "HOME was read");
}
