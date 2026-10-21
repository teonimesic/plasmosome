use std::ffi::OsStr;
use std::io::{self, ErrorKind};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::Path;

use plasmosome_core::private_socket::{
    EntryKind, ListedEntry, PrivateSocketError, check_peer_uid, check_private_path,
    list_trusted_dir,
};

use crate::cli::{is_instance_name, is_normal_absolute};
use crate::codes::ClientCode;
use crate::env::Env;
use crate::envelope::{ClientError, Instance};
use crate::json::Json;
use crate::refusal::{PathOrigin, at, map_refusal, number, refusal};

#[cfg(target_os = "macos")]
const MAX_SOCKET_PATH: usize = 103;
#[cfg(not(target_os = "macos"))]
const MAX_SOCKET_PATH: usize = 107;

pub(crate) const VARIABLE: &str = "PLASMOSOME_KERNEL";

/// Where a command's socket is, found from its flags and the environment alone.
pub(crate) enum Target {
    /// One socket: `--socket`'s, or the one an instance name gives under `$HOME`.
    Named(Named),
    /// No socket and no name was given: default selection over this `instances/` directory.
    Default(String),
}

pub(crate) struct Named {
    pub(crate) instance: Instance,
    pub(crate) origin: PathOrigin,
    pub(crate) from_variable: bool,
}

/// The one instance default selection found, on the connection it made.
pub(crate) struct Selected {
    pub(crate) stream: UnixStream,
    pub(crate) instance: Instance,
}

/// Why default selection gave no connection, and the instance when it had chosen one.
pub(crate) struct Unselected {
    pub(crate) error: ClientError,
    pub(crate) instance: Option<Instance>,
}

impl Unselected {
    fn unchosen(error: ClientError) -> Unselected {
        Unselected {
            error,
            instance: None,
        }
    }
}

pub(crate) fn target(
    socket: Option<String>,
    kernel: Option<String>,
    env: &dyn Env,
) -> Result<Target, ClientError> {
    if let Some(socket) = socket {
        return Ok(Target::Named(Named {
            instance: Instance {
                name: kernel,
                socket,
            },
            origin: PathOrigin::SocketFlag,
            from_variable: false,
        }));
    }
    let (name, from_variable) = match kernel {
        Some(name) => (Some(name), false),
        None => (variable(env)?, true),
    };
    let instances = format!("{}/.plasmosome/instances", home(env)?);
    let Some(name) = name else {
        return Ok(Target::Default(instances));
    };
    Ok(Target::Named(Named {
        instance: Instance {
            socket: socket_of(&instances, &name),
            name: Some(name),
        },
        origin: PathOrigin::Home,
        from_variable,
    }))
}

fn variable(env: &dyn Env) -> Result<Option<String>, ClientError> {
    let Some(value) = env.kernel() else {
        return Ok(None);
    };
    let text = value.to_string_lossy();
    let message = if value.is_empty() {
        format!("{VARIABLE} is set to the empty string, which is a usage error, not unset")
    } else if value.to_str().is_none() {
        format!("`{text}` is refused for {VARIABLE}: it is not valid UTF-8")
    } else if !is_instance_name(&text) {
        format!(
            "`{text}` is refused for {VARIABLE}: an instance name is not empty, `.` or `..`, and \
             holds no `/`, backslash, NUL or terminal control"
        )
    } else {
        return Ok(Some(text.into_owned()));
    };
    Err(refusal(
        ClientCode::InvalidArgument,
        message,
        vec![("argument", Json::String(VARIABLE.to_string()))],
    ))
}

fn home(env: &dyn Env) -> Result<String, ClientError> {
    let invalid = |message: String| refusal(ClientCode::InvalidHome, message, Vec::new());
    let home = env
        .home()
        .ok_or_else(|| invalid("HOME is unset".to_string()))?
        .into_string()
        .map_err(|home| {
            invalid(format!(
                "HOME `{}` is not valid UTF-8",
                home.to_string_lossy()
            ))
        })?;
    if home == "/" {
        return Ok(String::new());
    }
    let trimmed = home.strip_suffix('/').unwrap_or(&home);
    if !is_normal_absolute(trimmed) {
        return Err(invalid(format!(
            "HOME `{home}` is not an absolute path with no empty, `.` or `..` component"
        )));
    }
    Ok(trimmed.to_string())
}

fn socket_of(instances: &str, name: &str) -> String {
    format!("{instances}/{name}/control.uds")
}

/// Checks the path's length and the path boundary, then connects. Nothing is written.
pub(crate) fn connect(
    socket: &Path,
    trusted_uid: u32,
    origin: PathOrigin,
) -> Result<UnixStream, ClientError> {
    fits(socket)?;
    check_private_path(socket, trusted_uid)
        .map_err(|error| map_refusal(&error, socket, origin, trusted_uid))?;
    UnixStream::connect(socket).map_err(|error| unconnected(socket, &error))
}

fn fits(socket: &Path) -> Result<(), ClientError> {
    let length = socket.as_os_str().len();
    if length <= MAX_SOCKET_PATH {
        return Ok(());
    }
    Err(refusal(
        ClientCode::SocketPathTooLong,
        format!(
            "{} is {length} bytes, longer than the {MAX_SOCKET_PATH} a socket address holds",
            socket.display()
        ),
        vec![at(socket), ("max_bytes", number(MAX_SOCKET_PATH))],
    ))
}

fn unconnected(socket: &Path, error: &io::Error) -> ClientError {
    let (code, what) = match error.kind() {
        ErrorKind::ConnectionRefused => (
            ClientCode::Unreachable,
            "refused the connection: its daemon may be stopped, or alive with a full backlog"
                .to_string(),
        ),
        ErrorKind::NotFound => (
            ClientCode::NotRunning,
            "does not exist: no socket is bound at that name".to_string(),
        ),
        _ => (
            ClientCode::Transport,
            format!("could not be reached: {error}"),
        ),
    };
    let mut fields = vec![at(socket)];
    if code == ClientCode::Transport {
        fields.extend(error.raw_os_error().map(|errno| ("errno", number(errno))));
    }
    refusal(code, format!("{} {what}", socket.display()), fields)
}

/// Spec 025 section 5's default selection over `instances`. Every entry is judged before any
/// connection is made, so a refused selection contacts no instance. With one candidate, the
/// connection is returned unwritten, and the caller's exchange checks its peer. With several,
/// each is connected and its peer checked, and none is written to.
pub(crate) fn select_default(instances: &str, trusted_uid: u32) -> Result<Selected, Unselected> {
    let found = judge(instances, trusted_uid).map_err(Unselected::unchosen)?;
    match found.candidates.as_slice() {
        [] => Err(Unselected::unchosen(no_candidate(instances, &found.seen))),
        [name] => {
            let instance = Instance {
                name: Some(name.clone()),
                socket: socket_of(instances, name),
            };
            match UnixStream::connect(&instance.socket) {
                Ok(stream) => Ok(Selected { stream, instance }),
                Err(error) => Err(Unselected {
                    error: unconnected(Path::new(&instance.socket), &error),
                    instance: Some(instance),
                }),
            }
        }
        names => Err(Unselected::unchosen(ambiguous(
            instances,
            names,
            trusted_uid,
        ))),
    }
}

#[derive(Default)]
struct Found {
    candidates: Vec<String>,
    seen: Vec<String>,
}

fn judge(instances: &str, trusted_uid: u32) -> Result<Found, ClientError> {
    let listed = Path::new(instances);
    let entries = list_trusted_dir(listed).map_err(|error| match error {
        PrivateSocketError::NoDirectory { path } => refusal(
            ClientCode::NoInstance,
            format!(
                "{} does not exist, so no instance is under it",
                path.display()
            ),
            vec![at(&path), ("seen", names(&[]))],
        ),
        error => map_refusal(&error, listed, PathOrigin::Home, trusted_uid),
    })?;
    let mut found = Found::default();
    for ListedEntry { name, kind } in entries {
        let name = match kind {
            EntryKind::Other => continue,
            EntryKind::Symlink => return Err(linked(&listed.join(OsStr::from_bytes(&name)))),
            EntryKind::Directory => String::from_utf8(name)
                .ok()
                .filter(|name| is_instance_name(name))
                .ok_or_else(|| misnamed(listed))?,
        };
        let socket = socket_of(instances, &name);
        let socket = Path::new(&socket);
        match check_private_path(socket, trusted_uid) {
            Ok(_) => {
                fits(socket)?;
                found.candidates.push(name);
            }
            Err(PrivateSocketError::NoSocket { .. }) => found.seen.push(name),
            Err(error) => {
                return Err(map_refusal(&error, socket, PathOrigin::Home, trusted_uid));
            }
        }
    }
    Ok(found)
}

fn linked(entry: &Path) -> ClientError {
    refusal(
        ClientCode::UnsafeSocket,
        format!(
            "{} is a symbolic link, and default selection follows none",
            entry.display()
        ),
        vec![at(entry), ("rule", Json::String("symlink".to_string()))],
    )
}

fn misnamed(instances: &Path) -> ClientError {
    refusal(
        ClientCode::UnsafeSocket,
        format!(
            "a directory in {} is not named as an instance: its name is not UTF-8, is `.` or \
             `..`, or holds `/`, a backslash, NUL or a terminal control",
            instances.display()
        ),
        vec![at(instances), ("rule", Json::String("name".to_string()))],
    )
}

fn no_candidate(instances: &str, seen: &[String]) -> ClientError {
    let message = if seen.is_empty() {
        format!("{instances} holds no instance")
    } else {
        format!(
            "no instance under {instances} is running: {} hold no socket",
            seen.join(", ")
        )
    };
    refusal(
        ClientCode::NoInstance,
        message,
        vec![at(Path::new(instances)), ("seen", names(seen))],
    )
}

fn ambiguous(instances: &str, candidates: &[String], trusted_uid: u32) -> ClientError {
    let mut unreachable = Vec::new();
    for name in candidates {
        let socket = socket_of(instances, name);
        let socket = Path::new(&socket);
        match UnixStream::connect(socket) {
            Ok(stream) => {
                if let Err(error) = check_peer_uid(&stream, trusted_uid) {
                    return map_refusal(&error, socket, PathOrigin::Home, trusted_uid);
                }
            }
            Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
                unreachable.push(name.clone());
            }
            Err(error) => return unconnected(socket, &error),
        }
    }
    let mut fields = vec![("candidates", names(candidates))];
    if !unreachable.is_empty() {
        fields.push(("unreachable", names(&unreachable)));
    }
    refusal(
        ClientCode::AmbiguousInstance,
        format!(
            "{} instances are under {instances}: pass --kernel NAME to choose one",
            candidates.len()
        ),
        fields,
    )
}

fn names(names: &[String]) -> Json {
    Json::Array(names.iter().cloned().map(Json::String).collect())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    struct Home(Option<OsString>);

    impl Env for Home {
        fn home(&self) -> Option<OsString> {
            self.0.clone()
        }

        fn kernel(&self) -> Option<OsString> {
            None
        }
    }

    fn judged(home: &str) -> Result<String, ClientCode> {
        super::home(&Home(Some(home.into()))).map_err(|error| error.code())
    }

    #[test]
    fn one_trailing_slash_is_dropped_and_root_is_a_home() {
        assert_eq!(judged("/Users/me"), Ok("/Users/me".to_string()));
        assert_eq!(judged("/Users/me/"), Ok("/Users/me".to_string()));
        assert_eq!(judged("/"), Ok(String::new()));
        assert_eq!(judged("//"), Ok(String::new()));
        for refused in [
            "",
            "///",
            "/Users/me//",
            "/Users//me",
            "Users/me",
            "./me",
            "/Users/./me",
            "/Users/me/.",
            "/Users/../me",
            "/Users/me/..",
        ] {
            assert_eq!(judged(refused), Err(ClientCode::InvalidHome), "{refused:?}");
        }
    }
}
