use std::io::BufRead;

use crate::codes::Exit;
use crate::envelope::{ClientError, KernelError};
use crate::json::Json;

/// Why [`read_reply`] returned no line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadFailure {
    /// The input ended before a newline, whatever came before it.
    Closed,
    /// The line passed [`crate::json::MAX_REPLY_BYTES`] before its newline.
    TooLarge,
    /// A read reported `WouldBlock` or `TimedOut`: the reader's timeout passed.
    TimedOut,
    /// A read failed for another reason. `errno` is the operating system's error number, when the
    /// error has one.
    Transport { errno: Option<i32> },
}

/// Reads one reply line from `reader` and returns it without its newline.
pub fn read_reply(reader: &mut impl BufRead) -> Result<Vec<u8>, ReadFailure> {
    let _ = reader;
    todo!()
}

/// A reply that passed spec 025's rules.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    /// The daemon's `result` object, as it arrived.
    Result(Json),
    /// The daemon's `error` object, to relay with `domain: "kernel"` added.
    Kernel(KernelError),
}

/// Judges one reply line under spec 025 section 4.
pub fn judge_reply(line: &[u8]) -> Result<Reply, ClientError> {
    let _ = line;
    todo!()
}

/// Returns the exit status spec 025 section 4 gives a kernel error, by its code.
pub fn kernel_exit(error: &KernelError) -> Exit {
    let _ = error;
    todo!()
}

/// Returns the client error for a request whose reply line never arrived.
pub fn unanswered(failure: ReadFailure, changes_state: bool) -> ClientError {
    let _ = (failure, changes_state);
    todo!()
}

/// Builds the request line for `method` with `params`.
pub fn request_line(
    method: &str,
    params: Vec<(String, Json)>,
) -> Result<String, Vec<(String, Json)>> {
    let _ = (method, params);
    todo!()
}
