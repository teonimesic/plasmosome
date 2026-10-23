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
    file_size_signal_ignored();
    let path = dir.join("plasmid.toml");
    fs::write(&path, bytes).expect("the declaration is written");
    path
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn file_size_signal_ignored() {
    // SAFETY: signal takes integer constants and SIG_IGN. Ignoring SIGXFSZ in this binary makes
    // writes past ulimit -f return EFBIG in every test instead of killing the binary.
    let previous = unsafe { libc::signal(libc::SIGXFSZ, libc::SIG_IGN) };
    assert_ne!(previous, libc::SIG_ERR, "SIGXFSZ can be ignored");
}

fn refused_for_size(outcome: Result<PlasmidManifest, ManifestError>, what: &str) {
    match outcome {
        Err(error @ ManifestError::TooLarge { limit }) => {
            assert_eq!(limit, BOUND, "{what}: {error:?}");
            assert!(error.to_string().contains("1048576"), "{what}: {error}");
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
    let (delivered, delivery) = mpsc::channel::<io::Result<()>>();
    let (_release, released) = mpsc::channel::<()>();
    {
        let fifo = fifo.clone();
        thread::spawn(move || {
            let mut end = match fs::OpenOptions::new().write(true).open(&fifo) {
                Ok(end) => end,
                Err(error) => {
                    let _ = delivered.send(Err(error));
                    return;
                }
            };
            let _ = delivered.send(end.write_all(padded(BOUND + 1).as_bytes()));
            let _ = released.recv();
        });
    }
    refused_for_size(
        load_within_patience(fifo),
        "a FIFO that writes 1 MiB plus one byte and stays open",
    );
    match delivery.recv_timeout(PATIENCE) {
        Ok(wrote) => wrote.expect("the writer delivers every byte before load returns"),
        Err(_) => panic!("load returned without reading the FIFO"),
    }
}

#[cfg(unix)]
#[test]
fn load_refuses_a_symlink_to_dev_zero() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let link = dir.path().join("plasmid.toml");
    std::os::unix::fs::symlink("/dev/zero", &link).expect("the symlink is made");
    refused_for_size(load_within_patience(link), "a symlink to /dev/zero");
}

#[cfg(unix)]
#[test]
fn load_refuses_a_symlink_to_dev_urandom() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let link = dir.path().join("plasmid.toml");
    std::os::unix::fs::symlink("/dev/urandom", &link).expect("the symlink is made");
    refused_for_size(load_within_patience(link), "a symlink to /dev/urandom");
}

#[cfg(unix)]
#[test]
fn load_refuses_a_sparse_file_far_over_the_bound_without_reserving_its_length() {
    use std::os::fd::AsRawFd;

    file_size_signal_ignored();
    let file = tempfile::tempfile().expect("an unnamed declaration file");
    let path = PathBuf::from(format!("/dev/fd/{}", file.as_raw_fd()));
    let mut cause = None;
    let length = [1_u64 << 48, 1 << 43]
        .into_iter()
        .find(|length| match file.set_len(*length) {
            Ok(()) => true,
            Err(error) => {
                cause = Some(error);
                false
            }
        })
        .unwrap_or_else(|| {
            panic!(
                "ulimit -f or the filesystem refuses a sparse file of 8 TiB or more: {}",
                cause.unwrap()
            )
        });
    refused_for_size(
        load_within_patience(path),
        &format!("a sparse file of {length} bytes"),
    );
}

#[test]
fn load_refuses_a_file_over_the_bound_that_is_not_utf8_for_its_size() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    refused_for_size(
        PlasmidManifest::load(&written(dir.path(), &vec![0xff; BOUND + 1])),
        "load of 1 MiB plus one byte of 0xff",
    );
}

#[test]
fn the_bound_counts_bytes_not_characters() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut ending = padded(BOUND - 1);
    ending.push('é');
    assert_eq!(ending.len(), BOUND + 1);
    assert_eq!(ending.chars().count(), BOUND);
    let mut wide = String::from(DECLARATION);
    wide.push('#');
    wide.push_str(&"é".repeat(3 * BOUND / 4));
    assert!(wide.len() > BOUND && wide.chars().count() < BOUND);
    for (text, what) in [
        (ending, "1 MiB plus one byte ending in é"),
        (wide, "1.5 MiB of é in a comment"),
    ] {
        refused_for_size(PlasmidManifest::parse(&text), &format!("parse of {what}"));
        refused_for_size(
            PlasmidManifest::load(&written(dir.path(), text.as_bytes())),
            &format!("load of {what}"),
        );
    }
}

#[test]
fn parse_decides_the_size_before_it_reads_any_toml() {
    refused_for_size(
        PlasmidManifest::parse(&"x".repeat(BOUND + 1)),
        "parse of 1 MiB plus one byte that is not TOML",
    );
}

#[test]
fn load_of_a_directory_is_an_io_error() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    match PlasmidManifest::load(dir.path()) {
        Err(ManifestError::Io(error)) => {
            assert_eq!(error.kind(), ErrorKind::IsADirectory, "{error}");
        }
        other => panic!("a directory is an io error, not {other:?}"),
    }
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
fn parse_and_load_refuse_a_full_mebibyte_followed_by_whitespace() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    for suffix in [" ", "\t", "\n", "\r\n"] {
        let text = padded(BOUND) + suffix;
        refused_for_size(
            PlasmidManifest::parse(&text),
            &format!("parse with suffix {suffix:?}"),
        );
        refused_for_size(
            PlasmidManifest::load(&written(dir.path(), text.as_bytes())),
            &format!("load with suffix {suffix:?}"),
        );
    }
}

#[cfg(unix)]
#[test]
fn load_waits_on_a_fifo_whose_writer_delivers_exactly_the_bound_and_holds_it_open() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let fifo = dir.path().join("plasmid.toml");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let (delivered, delivery) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let writer_path = fifo.clone();
    let writer = thread::spawn(move || {
        let mut end = fs::OpenOptions::new()
            .write(true)
            .open(writer_path)
            .unwrap();
        delivered
            .send(end.write_all(padded(BOUND).as_bytes()))
            .unwrap();
        let _ = released.recv();
    });
    let (result, loaded) = mpsc::channel();
    let reader = thread::spawn(move || {
        let _ = result.send(PlasmidManifest::load(&fifo));
    });
    delivery
        .recv_timeout(PATIENCE)
        .expect("the writer delivers within patience")
        .unwrap();
    let before_close = loaded.recv_timeout(Duration::from_secs(1));
    release.send(()).expect("release the writer");
    writer.join().unwrap();
    assert!(
        matches!(before_close, Err(RecvTimeoutError::Timeout)),
        "load waits for an extra byte or EOF: {before_close:?}"
    );
    let manifest = loaded
        .recv_timeout(PATIENCE)
        .expect("load returns after EOF")
        .unwrap();
    reader.join().unwrap();
    assert_eq!(manifest.id, "github-pr");
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

#[test]
fn a_non_utf8_refusal_does_not_hold_the_declaration_bytes() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut bytes = padded(BOUND - 1).into_bytes();
    bytes.push(0xff);
    match PlasmidManifest::load(&written(dir.path(), &bytes)) {
        Err(ManifestError::Io(error)) => {
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
            let shown = format!("{error:?}").len();
            assert!(shown < 1024, "the refusal's Debug text is {shown} bytes");
        }
        other => panic!("a non-UTF-8 declaration is an InvalidData io error, not {other:?}"),
    }
}
