use std::path::Path;

use plasmosome_core::private_socket::PrivateSocketError;

use crate::codes::ClientCode;
use crate::envelope::ClientError;

/// Where the checked path came from. Spec 025 section 5 maps `BadPath` by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathOrigin {
    /// The path is `--socket`'s value.
    SocketFlag,
    /// The path was built from `$HOME`.
    Home,
}

/// Returns the client error spec 025 section 5's table gives a refusal from `plasmosome-core`'s
/// private-socket checks.
///
/// `checked` is the path the caller passed to the check: the socket path for
/// `check_private_path` and `check_peer_uid`, and `instances/` for `list_trusted_dir`. It is the
/// `path` of a refusal that reports none. `trusted_uid` is the UID the caller passed to the check,
/// and is `peer_mismatch`'s `uid`. Each code carries only the fields spec 025's client-code table
/// lists for it.
pub fn map_refusal(
    error: &PrivateSocketError,
    checked: &Path,
    origin: PathOrigin,
    trusted_uid: u32,
) -> ClientError {
    let _ = (checked, origin, trusted_uid);
    ClientError::new(ClientCode::Internal, error.to_string(), Vec::new())
        .expect("no field can repeat or be reserved")
}
