use std::ffi::OsString;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::process::{Command, ExitStatus};

use crate::bounded::read_file;

/// The most captured log bytes inspected after a run. Longer logs fail privately.
pub const LOG_LIMIT: u64 = 64 * 1024 * 1024;
/// The exact compiler used by the weekly job and its child commands.
pub const TOOLCHAIN: &str = "+nightly-2026-10-14";
const TRIPLE: &str = "x86_64-unknown-linux-gnu";
const TARGETS: [(&str, &str); 7] = [
    ("control_line", "core"),
    ("manifest", "core"),
    ("readiness_reply", "membrane"),
    ("hello_reply", "membrane"),
    ("cli_reply", "cli"),
    ("guest_control_reply", "backend"),
    ("operator_file", "core"),
];

/// Runs one target with private child streams, returning 0 only after 600 seconds of fuzzing.
/// Arguments are the absolute repository root, target name and a new absolute private directory.
/// `execute` must run the configured command synchronously and reap it without changing its
/// streams. Production passes `Command::status`. Only validated numeric summaries and crash
/// hashes reach `out`; failures print fixed text to `err`. Returns 2 for invalid arguments,
/// otherwise 1 on a child, file, hash or output failure. The private directory is never uploaded.
pub fn run(
    args: &[OsString],
    execute: &mut dyn FnMut(&mut Command) -> io::Result<ExitStatus>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let [root, target, private] = args else {
        return fail(err, 2, "expected root, target and new private directory");
    };
    let (root, private) = (Path::new(root), Path::new(private));
    let Some((index, &(target, owner))) = TARGETS
        .iter()
        .enumerate()
        .find(|(_, (name, _))| Some(*name) == target.to_str())
    else {
        return fail(err, 2, "unknown target");
    };
    if !root.is_absolute() || !private.is_absolute() {
        return fail(err, 2, "paths must be absolute");
    }
    match search(root, private, target, owner, index, execute, out) {
        Ok(code) => code,
        Err(reason) => fail(err, 1, reason),
    }
}

fn search(
    root: &Path,
    private: &Path,
    target: &str,
    owner: &str,
    index: usize,
    execute: &mut dyn FnMut(&mut Command) -> io::Result<ExitStatus>,
    out: &mut dyn Write,
) -> Result<u8, &'static str> {
    DirBuilder::new()
        .mode(0o700)
        .create(private)
        .map_err(|_| "cannot create private directory")?;
    let found = private.join("found");
    let crashes = private.join("crashes");
    fs::create_dir(&found)
        .and_then(|()| fs::create_dir(&crashes))
        .map_err(|_| "cannot create input directories")?;
    let seed = 20261023 + index;
    writeln!(out, "target={target} seed={seed}")
        .and_then(|()| out.flush())
        .map_err(|_| "cannot write seed")?;
    let build = private.join("build");
    let corpus = root.join(format!(
        "crates/plasmosome-{owner}/tests/fuzz/corpus/{target}"
    ));
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .env("CARGO_NET_OFFLINE", "true")
        .args([
            TOOLCHAIN,
            "fuzz",
            "run",
            "--fuzz-dir",
            "fuzz",
            "--debug-assertions",
            "--target",
            TRIPLE,
            "--target-dir",
        ])
        .arg(&build)
        .arg(target)
        .arg(&found)
        .arg(&corpus)
        .arg("--")
        .args(["-max_total_time=600", "-max_len=4096", "-timeout=10"])
        .arg(format!("-seed={seed}"))
        .arg(format!("-artifact_prefix={}/", crashes.display()));
    capture(&mut command, private, "fuzz")?;
    let status = execute(&mut command).map_err(|_| "cannot run fuzz child")?;
    let mut artifacts = Vec::new();
    for entry in fs::read_dir(&crashes).map_err(|_| "cannot list crash inputs")? {
        let entry = entry.map_err(|_| "cannot list crash inputs")?;
        let name = entry.file_name();
        let name = name.to_str().ok_or("invalid crash entry")?;
        if !["crash-", "timeout-", "oom-"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
            || !entry
                .file_type()
                .map_err(|_| "invalid crash entry")?
                .is_file()
        {
            return Err("invalid crash entry");
        }
        artifacts.push(entry.path());
    }
    artifacts.sort();
    if status.success() {
        if !artifacts.is_empty() {
            return Err("successful child left crash inputs");
        }
        let bytes = read_file(&private.join("fuzz.stderr"), LOG_LIMIT)
            .map_err(|_| "cannot inspect private log")?;
        let text = std::str::from_utf8(&bytes).map_err(|_| "invalid private summary")?;
        let (files, runs, seconds) = summary(text).ok_or("missing completed fuzz summary")?;
        writeln!(out, "corpus_files={files} runs={runs} seconds={seconds}")
            .and_then(|()| out.flush())
            .map_err(|_| "cannot write summary")?;
        return Ok(0);
    }
    if artifacts.is_empty() {
        return Err("failed child left no crash input");
    }
    let binary = build.join(TRIPLE).join("release").join(target);
    let binary_hash = hash(root, private, &binary, execute)?;
    for artifact in artifacts {
        let length = fs::metadata(&artifact)
            .map_err(|_| "cannot inspect crash input")?
            .len();
        let digest = hash(root, private, &artifact, execute)?;
        writeln!(out, "target={target} seed={seed} input_sha256={digest} length={length} binary_sha256={binary_hash}")
            .map_err(|_| "cannot write crash metadata")?;
    }
    out.flush().map_err(|_| "cannot flush crash metadata")?;
    Ok(1)
}

fn capture(command: &mut Command, private: &Path, prefix: &str) -> Result<(), &'static str> {
    let file = |suffix| -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(private.join(format!("{prefix}.{suffix}")))
    };
    command
        .stdout(file("stdout").map_err(|_| "cannot capture stdout")?)
        .stderr(file("stderr").map_err(|_| "cannot capture stderr")?);
    Ok(())
}

fn hash(
    root: &Path,
    private: &Path,
    path: &Path,
    execute: &mut dyn FnMut(&mut Command) -> io::Result<ExitStatus>,
) -> Result<String, &'static str> {
    let mut command = Command::new("sha256sum");
    command.current_dir(root).arg("--").arg(path);
    capture(&mut command, private, "hash")?;
    if !execute(&mut command)
        .map_err(|_| "cannot run hash child")?
        .success()
    {
        return Err("hash child failed");
    }
    let bytes = read_file(&private.join("hash.stdout"), 4096).map_err(|_| "cannot read hash")?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "invalid hash")?;
    let (digest, name) = text
        .strip_suffix('\n')
        .and_then(|s| s.split_once("  "))
        .ok_or("invalid hash")?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || name.is_empty()
        || name.contains('\n')
    {
        return Err("invalid hash");
    }
    Ok(digest.to_owned())
}

fn summary(text: &str) -> Option<(u64, u64, u64)> {
    let mut files = None;
    let mut done = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("INFO: seed corpus: files: ") {
            files = rest.split_ascii_whitespace().next()?.parse::<u64>().ok();
        }
        let words: Vec<_> = line.split_ascii_whitespace().collect();
        if let ["Done", runs, "runs", "in", seconds, "second(s)"] = words.as_slice() {
            done = Some((runs.parse::<u64>().ok()?, seconds.parse::<u64>().ok()?));
        }
    }
    let (runs, seconds) = done?;
    (files? > 0 && runs > 0 && seconds >= 600 && text.contains("INITED"))
        .then_some((files?, runs, seconds))
}

fn fail(err: &mut dyn Write, code: u8, reason: &str) -> u8 {
    let _ = writeln!(err, "fuzz-run: {reason}");
    code
}
