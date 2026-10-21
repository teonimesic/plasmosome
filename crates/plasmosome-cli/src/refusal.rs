use std::path::Path;

use plasmosome_core::private_socket::PrivateSocketError;

use crate::codes::ClientCode;
use crate::envelope::ClientError;
use crate::json::{Json, Number};

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
///
/// Every path is written by lossy UTF-8 conversion, each invalid sequence as U+FFFD. So a path
/// that is not UTF-8 is shown as another path, and `NotPrivate`'s `fix` would change that other
/// path. Spec 025 section 5 has the caller refuse a socket path that is not UTF-8 first.
pub fn map_refusal(
    error: &PrivateSocketError,
    checked: &Path,
    origin: PathOrigin,
    trusted_uid: u32,
) -> ClientError {
    #[deny(clippy::wildcard_enum_match_arm)]
    let (code, fields) = match error {
        PrivateSocketError::BadPath { .. } => match origin {
            PathOrigin::SocketFlag => (
                ClientCode::InvalidArgument,
                vec![("argument", text(&checked.to_string_lossy()))],
            ),
            PathOrigin::Home => (ClientCode::InvalidHome, Vec::new()),
        },
        PrivateSocketError::SymlinkInPath { path } => unsafe_socket(path, "symlink"),
        PrivateSocketError::NoDirectory { path } => (ClientCode::NoInstance, vec![at(path)]),
        PrivateSocketError::NoSocket { path } => (ClientCode::NotRunning, vec![at(path)]),
        PrivateSocketError::NotDirectory { path } => unsafe_socket(path, "not_directory"),
        PrivateSocketError::ForeignOwner { path, .. } => unsafe_socket(path, "owner"),
        PrivateSocketError::Replaceable { path, .. } => unsafe_socket(path, "mode"),
        PrivateSocketError::ReplaceableByAcl { path } => unsafe_socket(path, "acl"),
        PrivateSocketError::OwnershipIgnored { path } => unsafe_socket(path, "owner"),
        PrivateSocketError::NotPrivate { path, .. } => {
            let (code, mut fields) = unsafe_socket(path, "mode");
            fields.push(("fix", text(&chmod_700(path))));
            (code, fields)
        }
        PrivateSocketError::AclPresent { path } => unsafe_socket(path, "acl"),
        PrivateSocketError::Replaced { .. } => (ClientCode::Internal, Vec::new()),
        PrivateSocketError::BadName { .. } => unsafe_socket(checked, "name"),
        PrivateSocketError::PathTooLong { path, max } => (
            ClientCode::SocketPathTooLong,
            vec![at(path), ("max_bytes", number(max))],
        ),
        PrivateSocketError::AddressInUse { .. } => (ClientCode::Internal, Vec::new()),
        PrivateSocketError::NotASocket { path } => unsafe_socket(path, "not_socket"),
        PrivateSocketError::SocketOwner { path, .. } => unsafe_socket(path, "owner"),
        PrivateSocketError::SocketMode { path, .. } => unsafe_socket(path, "mode"),
        PrivateSocketError::BindEscaped { .. } => (ClientCode::Internal, Vec::new()),
        PrivateSocketError::PeerCredentials { .. } => (
            ClientCode::PeerMismatch,
            vec![at(checked), ("uid", number(trusted_uid))],
        ),
        PrivateSocketError::PeerMismatch { found, .. } => (
            ClientCode::PeerMismatch,
            vec![
                at(checked),
                ("uid", number(trusted_uid)),
                ("peer_uid", number(found)),
            ],
        ),
        PrivateSocketError::Io { path, errno, .. } => (
            ClientCode::LocalIo,
            vec![at(path), ("errno", number(errno))],
        ),
    };
    let message = match code {
        ClientCode::InvalidArgument => format!("--socket: {error}"),
        ClientCode::InvalidHome => format!("the path built from $HOME is refused: {error}"),
        ClientCode::Internal => format!("the client failed: {error}"),
        _ => error.to_string(),
    };
    refusal(code, message, fields)
}

pub(crate) fn refusal(
    code: ClientCode,
    message: String,
    fields: Vec<(&'static str, Json)>,
) -> ClientError {
    let fields = fields
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    ClientError::new(code, message, fields)
        .expect("each caller names distinct fields, none of them domain, code or message")
}

fn unsafe_socket(path: &Path, rule: &str) -> (ClientCode, Vec<(&'static str, Json)>) {
    (
        ClientCode::UnsafeSocket,
        vec![at(path), ("rule", text(rule))],
    )
}

pub(crate) fn at(path: &Path) -> (&'static str, Json) {
    ("path", text(&path.to_string_lossy()))
}

fn text(value: &str) -> Json {
    Json::String(value.to_string())
}

pub(crate) fn number(value: impl ToString) -> Json {
    Json::Number(Number::new(value.to_string()).expect("a decimal integer is a JSON number"))
}

fn chmod_700(directory: &Path) -> String {
    let quoted = directory.to_string_lossy().replace('\'', r#"'"'"'"#);
    format!("chmod 700 '{quoted}'")
}
