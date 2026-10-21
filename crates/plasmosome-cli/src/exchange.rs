use std::os::unix::net::UnixStream;
use std::path::Path;

use crate::clock::Deadline;
use crate::codes::ClientCode;
use crate::envelope::ClientError;
use crate::reply::ReadFailure;

/// Why [`exchange`] returned no reply line.
#[derive(Debug)]
pub enum Unreplied {
    /// No byte of the request was written. The error says why.
    NotSent(ClientError),
    /// The request was written, at least in part, and no whole reply line came back.
    Unanswered(ReadFailure),
}

/// Sends `request` on `stream` and reads one reply line, within `deadline`.
pub fn exchange(
    _stream: UnixStream,
    _socket: &Path,
    _trusted_uid: u32,
    _request: &str,
    _deadline: &Deadline<'_>,
) -> Result<Vec<u8>, Unreplied> {
    Err(Unreplied::NotSent(
        ClientError::new(
            ClientCode::Internal,
            "not written yet".to_string(),
            Vec::new(),
        )
        .expect("no field"),
    ))
}
