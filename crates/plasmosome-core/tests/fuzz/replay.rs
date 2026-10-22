use std::any::Any;
use std::io::Read;
use std::path::Path;

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

/// Passes every saved input in `dir`, as [`read_corpus`] reads them, to `check`, in name
/// order, and panics once at the end naming every input whose check panicked.
///
/// Each check runs on a thread named `input <name>`, so a check that overflows its stack, which
/// ends the process, still names its input.
pub fn replay_dir(dir: &Path, check: fn(&[u8])) {
    let inputs = read_corpus(dir);
    let count = inputs.len();
    let mut panicked = Vec::new();
    for (name, bytes) in inputs {
        let run = std::thread::Builder::new()
            .name(format!("input {name}"))
            .spawn(move || check(&bytes))
            .unwrap_or_else(|error| panic!("{name} gets no thread: {error}"));
        if let Err(payload) = run.join() {
            panicked.push(format!("{name}: {}", panic_message(payload.as_ref())));
        }
    }
    assert!(
        panicked.is_empty(),
        "{} of the {count} inputs in {} panicked:\n{}",
        panicked.len(),
        dir.display(),
        panicked.join("\n")
    );
}

/// Lists `dir` once and reads every entry whose name does not start with `.`, in name order,
/// as `(name, bytes)`.
///
/// Before it opens any input, it panics naming every such entry that is not a regular file, a
/// directory or a symbolic link among them. It panics naming `dir` when `dir` cannot be listed,
/// holds no input, or holds more than [`MAX_CORPUS_BYTES`] in all, and as [`read_input`] does.
pub fn read_corpus(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let listing = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("the corpus {} cannot be listed: {error}", dir.display()));
    let mut paths = Vec::new();
    let mut refused = Vec::new();
    for entry in listing {
        let entry = entry.unwrap_or_else(|error| {
            panic!("the corpus {} cannot be listed: {error}", dir.display())
        });
        if entry.file_name().as_encoded_bytes().starts_with(b".") {
            continue;
        }
        let kind = entry
            .file_type()
            .unwrap_or_else(|error| panic!("{} cannot be read: {error}", entry.path().display()));
        if kind.is_file() {
            paths.push(entry.path());
        } else {
            refused.push(entry.path().display().to_string());
        }
    }
    assert!(
        refused.is_empty(),
        "the corpus {} holds entries that are not regular files: {}",
        dir.display(),
        refused.join(", ")
    );
    paths.sort();
    assert!(
        !paths.is_empty(),
        "the corpus {} holds no input",
        dir.display()
    );
    let mut total = 0;
    let mut inputs = Vec::new();
    for path in paths {
        let file = std::fs::File::open(&path)
            .unwrap_or_else(|error| panic!("{} cannot be read: {error}", path.display()));
        let bytes = read_input(&path, file);
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

/// Reads at most [`MAX_INPUT_BYTES`] plus one byte from `source`, and panics naming `path` when
/// `source` holds more than [`MAX_INPUT_BYTES`] or cannot be read.
pub fn read_input(path: &Path, source: impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    source
        .take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .unwrap_or_else(|error| panic!("{} cannot be read: {error}", path.display()));
    assert!(
        bytes.len() <= MAX_INPUT_BYTES,
        "{} holds more than {MAX_INPUT_BYTES} bytes",
        path.display()
    );
    bytes
}

/// The text a panic carried, or a fixed phrase when it carried none.
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
        .unwrap_or_else(|| "a panic that carries no text".to_string())
}
