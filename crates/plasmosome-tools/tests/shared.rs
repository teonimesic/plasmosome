#![deny(clippy::exit)]

use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use plasmosome_tools::bounded::read_file;

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().expect("a scratch directory is created")
}

#[test]
fn a_file_at_the_limit_is_read_and_one_byte_over_is_refused() {
    let dir = scratch();
    let (at, over) = (dir.path().join("at"), dir.path().join("over"));
    std::fs::write(&at, "abcd").unwrap();
    std::fs::write(&over, "abcde").unwrap();

    assert_eq!(read_file(&at, 4).unwrap(), b"abcd");
    assert_eq!(read_file(&at, u64::MAX).unwrap(), b"abcd");
    assert_eq!(
        read_file(&over, 4).unwrap_err(),
        format!("{} is over 4 bytes", over.display())
    );
}

#[test]
fn a_link_is_refused_not_followed() {
    let dir = scratch();
    let (target, link) = (dir.path().join("target"), dir.path().join("link"));
    std::fs::write(&target, "kept").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let refused = read_file(&link, 1024).unwrap_err();

    assert!(
        refused.starts_with(&format!("cannot read {}: ", link.display())),
        "{refused}"
    );
}

#[test]
fn a_directory_a_device_or_a_missing_file_is_refused() {
    let dir = scratch();
    let missing = dir.path().join("missing");

    for (path, refusal) in [
        (dir.path(), "is not a regular file"),
        (Path::new("/dev/zero"), "is not a regular file"),
        (missing.as_path(), "cannot read"),
    ] {
        let refused = read_file(path, 1024).unwrap_err();

        assert!(refused.contains(refusal), "{}: {refused}", path.display());
        assert!(refused.contains(&path.display().to_string()), "{refused}");
    }
}

#[test]
fn a_fifo_is_refused_without_waiting_for_a_writer() {
    let dir = scratch();
    let fifo = dir.path().join("fifo");
    let made = Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo runs");
    assert!(made.success(), "mkfifo failed: {made}");
    let (sent, received) = mpsc::channel();

    std::thread::spawn(move || sent.send(read_file(&fifo, 1024)));
    let refused = received
        .recv_timeout(Duration::from_secs(10))
        .expect("read_file returns within 10 s without a writer");

    assert!(
        refused.unwrap_err().ends_with(" is not a regular file"),
        "a FIFO was read"
    );
}
