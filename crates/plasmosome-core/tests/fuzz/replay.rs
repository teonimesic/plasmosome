use std::path::Path;

/// The most bytes one saved input may hold: libFuzzer's `-max_len` in the weekly job.
pub const MAX_INPUT_BYTES: usize = 4096;

/// The most bytes one target's saved inputs may hold together.
pub const MAX_CORPUS_BYTES: usize = 65_536;

/// Replays `target`'s committed corpus, `tests/fuzz/corpus/<target>/` in the crate that
/// includes this file, through `check`, as [`replay_dir`] does.
pub fn replay(target: &str, check: fn(&[u8])) {
    let _ = (target, check);
    todo!()
}

/// Passes every saved input in `dir` to `check`, in name order, and panics once at the end
/// naming every input whose check panicked.
///
/// It reads each regular file whose name does not start with `.`. It panics, naming the
/// directory, when `dir` cannot be listed, holds no input, or holds more than
/// [`MAX_CORPUS_BYTES`] in all, and naming the file when one input holds more than
/// [`MAX_INPUT_BYTES`].
pub fn replay_dir(dir: &Path, check: fn(&[u8])) {
    let _ = (dir, check);
    todo!()
}
