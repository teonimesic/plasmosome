use std::io::{self, BufReader, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use plasmosome_core::private_socket::check_peer_uid;

use crate::clock::Deadline;
use crate::codes::ClientCode;
use crate::envelope::ClientError;
use crate::refusal::{PathOrigin, at, map_refusal, number, refusal};
use crate::reply::{ReadFailure, failure_of, read_reply};

const PIECE: usize = 64 * 1024;

/// Why [`exchange`] returned no reply line.
#[derive(Debug)]
pub enum Unreplied {
    /// No byte of the request was written. The error says why.
    NotSent(ClientError),
    /// The request was written, at least in part, and no whole reply line came back.
    Unanswered(ReadFailure),
}

/// Sends `request` on `stream` and reads one reply line with [`read_reply`], within `deadline`.
///
/// It first checks that the peer runs as `trusted_uid`. A peer that does not, or whose UID cannot
/// be read, is `peer_mismatch`, and the stream is closed unread with nothing written. With no
/// time left it is `deadline`, and nothing is written. Each write and each read then waits only
/// the time left. A write that fails before any byte is `transport`. `socket` is the path the
/// stream connected to, which these errors name.
///
/// On macOS a socket whose peer has closed refuses a new timeout with `EINVAL` (measured on
/// Darwin 25.6). Such a socket can receive nothing more, so the read or write is then made
/// without blocking.
pub fn exchange(
    stream: UnixStream,
    socket: &Path,
    trusted_uid: u32,
    request: &str,
    deadline: &Deadline<'_>,
) -> Result<Vec<u8>, Unreplied> {
    if let Err(refusal) = check_peer_uid(&stream, trusted_uid) {
        let error = map_refusal(&refusal, socket, PathOrigin::SocketFlag, trusted_uid);
        return Err(Unreplied::NotSent(error));
    }
    let mut writer = Bounded {
        stream: &stream,
        deadline,
        set: UnixStream::set_write_timeout,
    };
    let mut written = 0;
    while written < request.len() {
        let error = match writer.write(&request.as_bytes()[written..]) {
            Ok(0) => ErrorKind::WriteZero.into(),
            Ok(count) => {
                written += count;
                continue;
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => error,
        };
        return Err(unsent(written, &error, socket, deadline));
    }
    let reader = Bounded {
        stream: &stream,
        deadline,
        set: UnixStream::set_read_timeout,
    };
    read_reply(&mut BufReader::with_capacity(PIECE, reader)).map_err(Unreplied::Unanswered)
}

fn unsent(written: usize, error: &io::Error, socket: &Path, deadline: &Deadline<'_>) -> Unreplied {
    let failure = failure_of(error);
    if written > 0 {
        return Unreplied::Unanswered(failure);
    }
    if failure == ReadFailure::TimedOut {
        return Unreplied::NotSent(refusal(
            ClientCode::Deadline,
            format!(
                "the deadline of {} ms passed before the request was written",
                deadline.ms()
            ),
            vec![("deadline_ms", number(deadline.ms()))],
        ));
    }
    let mut fields = vec![at(socket)];
    fields.extend(error.raw_os_error().map(|errno| ("errno", number(errno))));
    Unreplied::NotSent(refusal(
        ClientCode::Transport,
        format!(
            "writing the request to {} failed: {error}",
            socket.display()
        ),
        fields,
    ))
}

struct Bounded<'a> {
    stream: &'a UnixStream,
    deadline: &'a Deadline<'a>,
    set: fn(&UnixStream, Option<Duration>) -> io::Result<()>,
}

impl Bounded<'_> {
    fn ready(&self) -> io::Result<&UnixStream> {
        let left = self.deadline.left();
        if left.is_zero() {
            return Err(ErrorKind::TimedOut.into());
        }
        if (self.set)(self.stream, Some(left)).is_err() {
            self.stream.set_nonblocking(true)?;
        }
        Ok(self.stream)
    }
}

impl Read for Bounded<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.ready()?.read(buffer)
    }
}

impl Write for Bounded<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.ready()?.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
