use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use plasmosome_core::{MAX_MANIFEST_BYTES, ManifestError, PlasmidManifest};

const BOUND: usize = 1 << 20;

const PATIENCE: Duration = Duration::from_secs(5);

const DECLARATION: &str =
    "id = \"github-pr\"\ndescription = \"Read pull requests.\"\nimpl.wasm = \"pr.wasm\"\n";

fn padded(length: usize) -> String {
    let mut text = String::with_capacity(length);
    text.push_str(DECLARATION);
    text.push_str(&"#".repeat(length - DECLARATION.len()));
    text
}

fn written(dir: &Path, bytes: &[u8]) -> PathBuf {
    let path = dir.join("plasmid.toml");
    fs::write(&path, bytes).expect("the declaration is written");
    path
}

fn refused_for_size(outcome: Result<PlasmidManifest, ManifestError>, what: &str) {
    match outcome {
        Err(error @ ManifestError::TooLarge { limit }) => {
            assert_eq!(limit, BOUND, "{what}: {error:?}");
            assert_eq!(
                error.to_string(),
                "manifest exceeds the 1048576-byte limit",
                "{what}"
            );
        }
        Err(error) => panic!("{what} is refused for its size, not as {error:?}"),
        Ok(manifest) => panic!("{what} was accepted as {manifest:?}"),
    }
}

fn load_within_patience(path: PathBuf) -> Result<PlasmidManifest, ManifestError> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(PlasmidManifest::load(&path));
    });
    match receiver.recv_timeout(PATIENCE) {
        Ok(outcome) => outcome,
        Err(RecvTimeoutError::Timeout) => {
            let _ = writeln!(
                io::stderr(),
                "PlasmidManifest::load did not return within 5 s: it reads without a bound"
            );
            std::process::abort();
        }
        Err(RecvTimeoutError::Disconnected) => panic!("PlasmidManifest::load panicked"),
    }
}

#[cfg(unix)]
#[test]
fn load_refuses_a_fifo_that_writes_past_the_bound_and_never_closes() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let fifo = dir.path().join("plasmid.toml");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo runs");
    assert!(made.success(), "mkfifo {}: {made}", fifo.display());
    let (release, released) = mpsc::channel::<()>();
    let writer = {
        let fifo = fifo.clone();
        thread::spawn(move || -> io::Result<()> {
            let mut end = fs::OpenOptions::new().write(true).open(&fifo)?;
            let wrote = end.write_all(padded(BOUND + 1).as_bytes());
            let _ = released.recv();
            wrote
        })
    };
    refused_for_size(
        load_within_patience(fifo),
        "a FIFO that writes 1 MiB plus one byte and stays open",
    );
    drop(release);
    writer
        .join()
        .expect("the writer thread does not panic")
        .expect("the writer delivers every byte before load returns");
}

#[cfg(unix)]
#[test]
fn load_refuses_a_symlink_to_dev_zero() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let link = dir.path().join("plasmid.toml");
    std::os::unix::fs::symlink("/dev/zero", &link).expect("the symlink is made");
    refused_for_size(load_within_patience(link), "a symlink to /dev/zero");
}

#[test]
fn a_declaration_of_exactly_one_mebibyte_parses_and_loads() {
    assert_eq!(MAX_MANIFEST_BYTES, BOUND);
    let text = padded(BOUND);
    assert_eq!(text.len(), BOUND);
    let parsed = PlasmidManifest::parse(&text).expect("parse accepts exactly 1 MiB");
    assert_eq!(parsed.id, "github-pr");
    let dir = tempfile::tempdir().expect("a temporary directory");
    let loaded = PlasmidManifest::load(&written(dir.path(), text.as_bytes()))
        .expect("load accepts a file of exactly 1 MiB");
    assert_eq!(loaded, parsed);
}

#[test]
fn a_declaration_one_byte_over_the_bound_is_refused_not_parsed_from_its_first_mebibyte() {
    let text = padded(BOUND + 1);
    assert_eq!(text.len(), BOUND + 1);
    PlasmidManifest::parse(&text[..BOUND]).expect("its first 1 MiB is a valid declaration");
    refused_for_size(
        PlasmidManifest::parse(&text),
        "parse of a string of 1 MiB plus one byte",
    );
    let dir = tempfile::tempdir().expect("a temporary directory");
    refused_for_size(
        PlasmidManifest::load(&written(dir.path(), text.as_bytes())),
        "load of a file of 1 MiB plus one byte",
    );
}

#[test]
fn a_declaration_under_the_bound_that_is_not_utf8_is_an_invalid_data_io_error() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    match PlasmidManifest::load(&written(dir.path(), &[0xff, 0xff, 0xff])) {
        Err(ManifestError::Io(error)) => {
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        }
        other => panic!("a non-UTF-8 declaration is an InvalidData io error, not {other:?}"),
    }
}
