mod support;

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use support::{Daemon, base, private_dir};
use tempfile::TempDir;

const BINARY: &str = env!("CARGO_BIN_EXE_plasmosome");

#[cfg(target_os = "macos")]
const MAX_PATH: usize = 103;
#[cfg(not(target_os = "macos"))]
const MAX_PATH: usize = 107;

fn short_home() -> TempDir {
    let base = base();
    let home = tempfile::Builder::new()
        .prefix("h")
        .rand_bytes(4)
        .tempdir_in(&base)
        .unwrap_or_else(|error| panic!("no test HOME under {}: {error}", base.display()));
    fs::set_permissions(home.path(), fs::Permissions::from_mode(0o700)).expect("0700");
    let longest = home.path().join(".plasmosome/instances/a/control.uds");
    assert!(
        longest.as_os_str().len() <= MAX_PATH,
        "{} does not fit sun_path",
        longest.display()
    );
    home
}

fn instance(home: &Path, name: &str) -> PathBuf {
    let dir = home.join(".plasmosome/instances").join(name);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .unwrap_or_else(|error| panic!("no directory {}: {error}", dir.display()));
    dir.join("control.uds")
}

fn plasmosome(args: &[&str], vars: &[(&str, &OsStr)], cwd: Option<&Path>) -> Output {
    let mut command = Command::new(BINARY);
    command.args(args).env_clear().stdin(Stdio::null());
    for (key, value) in vars {
        command.env(key, value);
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.output().expect("the plasmosome binary runs")
}

fn one_line(bytes: &[u8]) -> Value {
    let text = std::str::from_utf8(bytes).expect("the output is UTF-8");
    assert!(
        text.ends_with('\n') && text.matches('\n').count() == 1,
        "the output is not one line: {text:?}"
    );
    serde_json::from_str(text).expect("the output is one JSON line")
}

fn success(output: &Output) -> Value {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    one_line(&output.stdout)
}

fn failure(output: &Output, code: i32) -> Value {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    one_line(&output.stderr)
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a UTF-8 path")
}

fn tree(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("{} cannot be read: {error}", dir.display()));
        for entry in entries {
            let entry = entry.expect("an entry");
            let path = entry.path();
            let relative = path.strip_prefix(root).expect("an entry below the root");
            found.push(relative.to_string_lossy().into_owned());
            if entry.file_type().expect("a file type").is_dir() {
                pending.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn no_run_keeps_state_between_runs() {
    let home = short_home();
    let [tmp, cache, state, cwd, roots] = [(); 5].map(|()| private_dir(&base()));
    let vars = [
        ("HOME", home.path().as_os_str()),
        ("TMPDIR", tmp.path().as_os_str()),
        ("XDG_CACHE_HOME", cache.path().as_os_str()),
        ("XDG_STATE_HOME", state.path().as_os_str()),
    ];

    let a = instance(home.path(), "a");
    let daemon = Daemon::start("a", roots.path(), &a);
    let first = success(&plasmosome(&["status"], &vars, Some(cwd.path())));
    assert_eq!(first["instance"], json!({"name": "a", "socket": text(&a)}));
    drop(daemon);
    fs::remove_dir_all(home.path().join(".plasmosome/instances/a")).expect("a is removed");

    let b = instance(home.path(), "b");
    let _daemon = Daemon::start("b", roots.path(), &b);
    let second = success(&plasmosome(&["status"], &vars, Some(cwd.path())));
    assert_eq!(second["instance"], json!({"name": "b", "socket": text(&b)}));

    assert_eq!(
        tree(home.path()),
        [
            ".plasmosome",
            ".plasmosome/instances",
            ".plasmosome/instances/b",
            ".plasmosome/instances/b/control.uds",
        ]
    );
    for (name, dir) in [
        ("TMPDIR", &tmp),
        ("XDG_CACHE_HOME", &cache),
        ("XDG_STATE_HOME", &state),
        ("the working directory", &cwd),
    ] {
        assert_eq!(tree(dir.path()), Vec::<String>::new(), "{name} was written");
    }
}

#[test]
fn the_binary_reads_home_and_plasmosome_kernel() {
    let home = short_home();
    let roots = private_dir(&base());
    let a = instance(home.path(), "a");
    let b = instance(home.path(), "b");
    let _a = Daemon::start("a", roots.path(), &a);
    let _b = Daemon::start("b", roots.path(), &b);
    let with = |kernel: &'static str| {
        [
            ("HOME", home.path().as_os_str()),
            ("PLASMOSOME_KERNEL", OsStr::new(kernel)),
        ]
    };

    let envelope = success(&plasmosome(&["status"], &with("b"), None));
    assert_eq!(
        envelope["instance"],
        json!({"name": "b", "socket": text(&b)})
    );

    let envelope = success(&plasmosome(&["status", "--kernel", "a"], &with("b"), None));
    assert_eq!(
        envelope["instance"],
        json!({"name": "a", "socket": text(&a)})
    );

    let envelope = failure(
        &plasmosome(&["status"], &[("HOME", home.path().as_os_str())], None),
        2,
    );
    assert_eq!(envelope["error"]["code"], "ambiguous_instance");
    assert_eq!(envelope["error"]["candidates"], json!(["a", "b"]));

    for kernel in ["", "../x"] {
        let envelope = failure(&plasmosome(&["status"], &with(kernel), None), 64);
        assert_eq!(envelope["error"]["code"], "invalid_argument");
        assert_eq!(envelope["error"]["argument"], "PLASMOSOME_KERNEL");
    }
}

#[test]
fn an_unset_home_is_invalid_home() {
    let kernel = [("PLASMOSOME_KERNEL", OsStr::new("x"))];
    for (line, vars) in [
        (&["status", "--kernel", "x"][..], &[][..]),
        (&["status"], &kernel[..]),
        (&["status"], &[]),
    ] {
        let envelope = failure(&plasmosome(line, vars, None), 2);
        assert_eq!(envelope["command"], "plasmosome.status");
        assert_eq!(envelope["error"]["code"], "invalid_home", "{line:?}");
        assert!(envelope.get("instance").is_none(), "{envelope}");
    }
}
