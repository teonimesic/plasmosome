use std::any::Any;
use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

/// The most bytes one saved input may hold: libFuzzer's `-max_len` in the weekly job.
pub const MAX_INPUT_BYTES: usize = 4096;

/// The most bytes one target's saved inputs may hold together.
pub const MAX_CORPUS_BYTES: usize = 65_536;

/// Replays `target`'s committed corpus, `tests/fuzz/corpus/<target>/` in the crate that
/// includes this file, through `check`, as [`replay_dir`] does.
pub fn replay(target: &str, check: fn(&[u8])) {
    let corpus = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fuzz/corpus/"));
    replay_dir(&corpus.join(target), check);
}

/// Passes every saved input in `dir` to `check`, in name order, and panics once at the end
/// naming every input whose check panicked.
///
/// It reads each regular file whose name does not start with `.`. It panics, naming the
/// directory, when `dir` cannot be listed, holds no input, or holds more than
/// [`MAX_CORPUS_BYTES`] in all, and naming the file when one input holds more than
/// [`MAX_INPUT_BYTES`].
pub fn replay_dir(dir: &Path, check: fn(&[u8])) {
    let inputs = read_corpus(dir);
    let mut panicked = Vec::new();
    for (name, bytes) in &inputs {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| check(bytes))) {
            panicked.push(format!("{name}: {}", panic_message(payload.as_ref())));
        }
    }
    assert!(
        panicked.is_empty(),
        "{} of the {} inputs in {} panicked:\n{}",
        panicked.len(),
        inputs.len(),
        dir.display(),
        panicked.join("\n")
    );
}

fn read_corpus(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let listing = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("the corpus {} cannot be listed: {error}", dir.display()));
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in listing {
        let entry = entry.unwrap_or_else(|error| {
            panic!("the corpus {} cannot be listed: {error}", dir.display())
        });
        let kind = entry
            .file_type()
            .unwrap_or_else(|error| panic!("{} cannot be read: {error}", entry.path().display()));
        if kind.is_file() && !entry.file_name().as_encoded_bytes().starts_with(b".") {
            paths.push(entry.path());
        }
    }
    paths.sort();
    assert!(
        !paths.is_empty(),
        "the corpus {} holds no input",
        dir.display()
    );
    let mut total = 0;
    let mut inputs = Vec::new();
    for path in paths {
        let bytes = read_input(&path);
        total += bytes.len();
        assert!(
            total <= MAX_CORPUS_BYTES,
            "the corpus {} holds more than {MAX_CORPUS_BYTES} bytes",
            dir.display()
        );
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        inputs.push((name, bytes));
    }
    inputs
}

fn read_input(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| {
            file.take(MAX_INPUT_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
        })
        .unwrap_or_else(|error| panic!("{} cannot be read: {error}", path.display()));
    assert!(
        bytes.len() <= MAX_INPUT_BYTES,
        "{} holds more than {MAX_INPUT_BYTES} bytes",
        path.display()
    );
    bytes
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
        .unwrap_or_else(|| "a panic that carries no text".to_string())
}
