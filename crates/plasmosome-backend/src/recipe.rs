use std::fmt;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// The most bytes `SessionFileRecipe::contents` may hold.
pub const MAX_SESSION_FILE_BYTES: usize = 65_536;

const MAX_DNS_NAME_BYTES: usize = 253;
const MAX_DNS_LABEL_BYTES: usize = 63;

/// The access a managed file or mount gives the guest. Encodes as `"read_only"` or
/// `"read_write"`; no other spelling decodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileAccess {
    ReadOnly,
    ReadWrite,
}

/// The transport a proxy route carries. Encodes as `"tcp"` or `"udp"`; no other spelling
/// decodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyTransport {
    Tcp,
    Udp,
}

/// The resolved recipe of a session file: its initial bytes, the guest's access, and the
/// absolute guest path it appears at.
///
/// Decoding refuses a missing or unknown field and any value `validate` refuses. A value built
/// in memory is not checked: call `validate` before acting on it. `contents` must not carry
/// credential material.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "SessionFileWire")]
pub struct SessionFileRecipe {
    pub contents: Vec<u8>,
    pub mode: FileAccess,
    pub guest_path: String,
}

impl SessionFileRecipe {
    /// Returns the first rule this recipe breaks, in field order: `contents` holds at most
    /// `MAX_SESSION_FILE_BYTES`, then `guest_path` is a canonical absolute path as
    /// `RecipeError` defines it.
    pub fn validate(&self) -> Result<(), RecipeError> {
        if self.contents.len() > MAX_SESSION_FILE_BYTES {
            return Err(RecipeError::ContentsTooLarge {
                len: self.contents.len(),
            });
        }
        canonical_path("guest_path", &self.guest_path)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionFileWire {
    contents: Vec<u8>,
    mode: FileAccess,
    guest_path: String,
}

impl TryFrom<SessionFileWire> for SessionFileRecipe {
    type Error = RecipeError;

    fn try_from(wire: SessionFileWire) -> Result<Self, RecipeError> {
        let recipe = SessionFileRecipe {
            contents: wire.contents,
            mode: wire.mode,
            guest_path: wire.guest_path,
        };
        recipe.validate().map(|()| recipe)
    }
}

/// The resolved recipe of a socket grant: the absolute host socket it relays to and the
/// absolute guest path it appears at.
///
/// Decoding refuses a missing or unknown field and any value `validate` refuses. A value built
/// in memory is not checked: call `validate` before acting on it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "UdsWire")]
pub struct UdsRecipe {
    pub upstream: String,
    pub guest_path: String,
}

impl UdsRecipe {
    /// Returns the first rule this recipe breaks, in field order: `upstream`, then
    /// `guest_path`, each a canonical absolute path as `RecipeError` defines it.
    pub fn validate(&self) -> Result<(), RecipeError> {
        canonical_path("upstream", &self.upstream)?;
        canonical_path("guest_path", &self.guest_path)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UdsWire {
    upstream: String,
    guest_path: String,
}

impl TryFrom<UdsWire> for UdsRecipe {
    type Error = RecipeError;

    fn try_from(wire: UdsWire) -> Result<Self, RecipeError> {
        let recipe = UdsRecipe {
            upstream: wire.upstream,
            guest_path: wire.guest_path,
        };
        recipe.validate().map(|()| recipe)
    }
}

/// The resolved recipe of a proxy route: its transport, the one host or address it reaches,
/// the port, and whether private addresses are allowed for this recipe alone.
///
/// Decoding refuses a missing or unknown field and any value `validate` refuses. A value built
/// in memory is not checked: call `validate` before acting on it. Passing `validate` resolves
/// no name and says nothing about the addresses a name resolves to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "ProxyWire")]
pub struct ProxyRecipe {
    pub transport: ProxyTransport,
    pub destination: String,
    pub port: u16,
    pub allow_private: bool,
}

impl ProxyRecipe {
    /// Returns the first rule this recipe breaks, in field order: `destination` is NUL-free
    /// and is either one IP literal or one ASCII DNS name, then `port` is not 0.
    ///
    /// A DNS name is 1 to 253 bytes of dot-separated labels, each 1 to 63 bytes of ASCII letters,
    /// digits and `-`, not starting or ending with `-`, with no trailing dot. Its last label must
    /// not be a number in the sense of the WHATWG URL Standard's "ends in a number" check: all
    /// ASCII digits, or `0x` or `0X` followed by hex digits. Resolvers read such names as IPv4
    /// addresses, so `0x7f000001` and `127.0.0.0x1` are refused.
    pub fn validate(&self) -> Result<(), RecipeError> {
        destination(&self.destination)?;
        if self.port == 0 {
            return Err(RecipeError::ZeroPort);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProxyWire {
    transport: ProxyTransport,
    destination: String,
    port: u16,
    allow_private: bool,
}

impl TryFrom<ProxyWire> for ProxyRecipe {
    type Error = RecipeError;

    fn try_from(wire: ProxyWire) -> Result<Self, RecipeError> {
        let recipe = ProxyRecipe {
            transport: wire.transport,
            destination: wire.destination,
            port: wire.port,
            allow_private: wire.allow_private,
        };
        recipe.validate().map(|()| recipe)
    }
}

/// The resolved recipe of a mount: the access the guest gets.
///
/// Decoding refuses a missing or unknown field.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "MountWire")]
pub struct MountRecipe {
    pub access: FileAccess,
}

impl MountRecipe {
    /// Accepts every mount recipe; it exists so every recipe is checked the same way.
    pub fn validate(&self) -> Result<(), RecipeError> {
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MountWire {
    access: FileAccess,
}

impl TryFrom<MountWire> for MountRecipe {
    type Error = RecipeError;

    fn try_from(wire: MountWire) -> Result<Self, RecipeError> {
        let recipe = MountRecipe {
            access: wire.access,
        };
        recipe.validate().map(|()| recipe)
    }
}

/// How to launch a broker: the exact argument vector, whose first word is an absolute program
/// path, and two distinct absolute host-private endpoints.
///
/// Every word is data. Nothing here runs a shell, searches `PATH` or expands `$HOME` or `~`.
/// Decoding refuses a missing or unknown field and any value `validate` refuses. A value built
/// in memory is not checked: call `validate` before acting on it. Passing `validate` does not
/// mean the program exists or the endpoints can be bound.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "BrokerLaunchWire")]
pub struct BrokerLaunch {
    pub command: Vec<String>,
    pub control_socket: String,
    pub data_socket: String,
}

impl BrokerLaunch {
    /// Returns the first rule this launch breaks, in field order: `command` is not empty,
    /// every word is NUL-free and the first is a canonical absolute path; then
    /// `control_socket` and `data_socket` are each canonical absolute paths, as `RecipeError`
    /// defines them; then they differ.
    ///
    /// Endpoints differ when their canonical spellings differ. That is all this check can see:
    /// preflight must still establish that they are different files, because a symlink or a
    /// mount can give one file two canonical names.
    pub fn validate(&self) -> Result<(), RecipeError> {
        let Some(program) = self.command.first() else {
            return Err(RecipeError::EmptyCommand);
        };
        for word in &self.command {
            nul_free("command", word)?;
        }
        canonical_path("command", program)?;
        canonical_path("control_socket", &self.control_socket)?;
        canonical_path("data_socket", &self.data_socket)?;
        if self.control_socket == self.data_socket {
            return Err(RecipeError::SharedEndpoint {
                path: self.control_socket.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrokerLaunchWire {
    command: Vec<String>,
    control_socket: String,
    data_socket: String,
}

impl TryFrom<BrokerLaunchWire> for BrokerLaunch {
    type Error = RecipeError;

    fn try_from(wire: BrokerLaunchWire) -> Result<Self, RecipeError> {
        let launch = BrokerLaunch {
            command: wire.command,
            control_socket: wire.control_socket,
            data_socket: wire.data_socket,
        };
        launch.validate().map(|()| launch)
    }
}

/// The first structural rule a recipe breaks. `field` is the serde field name; every word of
/// `command` reports as `"command"`.
///
/// A path field must be a canonical absolute path: NUL-free, starting with `/`, with no empty,
/// `.` or `..` component and no trailing `/`. `/` alone is refused, because every path field
/// names a file, a socket or a program. A path that breaks this is refused, never rewritten,
/// so an accepted path has one spelling and round-trips unchanged. Later `command` words are
/// arguments, not paths, and only need to be NUL-free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipeError {
    ContainsNul { field: &'static str },
    NotAbsolute { field: &'static str, value: String },
    NotCanonical { field: &'static str, value: String },
    ContentsTooLarge { len: usize },
    ZeroPort,
    InvalidDestination { value: String },
    EmptyCommand,
    SharedEndpoint { path: String },
}

impl fmt::Display for RecipeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecipeError::ContainsNul { field } => write!(f, "`{field}` contains a NUL byte"),
            RecipeError::NotAbsolute { field, value } => {
                write!(f, "`{field}` must be an absolute path, not {value:?}")
            }
            RecipeError::NotCanonical { field, value } => write!(
                f,
                "`{field}` has an empty, `.` or `..` component or a trailing `/`: {value:?}"
            ),
            RecipeError::ContentsTooLarge { len } => write!(
                f,
                "session file contents are {len} bytes, over the {MAX_SESSION_FILE_BYTES}-byte cap"
            ),
            RecipeError::ZeroPort => write!(f, "`port` must not be 0"),
            RecipeError::InvalidDestination { value } => write!(
                f,
                "`destination` must be one DNS name or IP address, not {value:?}"
            ),
            RecipeError::EmptyCommand => write!(f, "`command` must name a program"),
            RecipeError::SharedEndpoint { path } => write!(
                f,
                "`control_socket` and `data_socket` must differ, but both are {path:?}"
            ),
        }
    }
}

impl std::error::Error for RecipeError {}

fn nul_free(field: &'static str, value: &str) -> Result<(), RecipeError> {
    if value.contains('\0') {
        return Err(RecipeError::ContainsNul { field });
    }
    Ok(())
}

fn canonical_path(field: &'static str, value: &str) -> Result<(), RecipeError> {
    nul_free(field, value)?;
    let Some(rest) = value.strip_prefix('/') else {
        return Err(RecipeError::NotAbsolute {
            field,
            value: value.to_string(),
        });
    };
    if rest
        .split('/')
        .any(|component| matches!(component, "" | "." | ".."))
    {
        return Err(RecipeError::NotCanonical {
            field,
            value: value.to_string(),
        });
    }
    Ok(())
}

fn destination(value: &str) -> Result<(), RecipeError> {
    nul_free("destination", value)?;
    if value.parse::<IpAddr>().is_err() && !is_dns_name(value) {
        return Err(RecipeError::InvalidDestination {
            value: value.to_string(),
        });
    }
    Ok(())
}

fn is_dns_name(value: &str) -> bool {
    let ends_in_a_number = value.rsplit('.').next().is_some_and(is_number);
    value.len() <= MAX_DNS_NAME_BYTES && value.split('.').all(is_dns_label) && !ends_in_a_number
}

fn is_number(label: &str) -> bool {
    match label
        .strip_prefix("0x")
        .or_else(|| label.strip_prefix("0X"))
    {
        Some(hex) => hex.bytes().all(|byte| byte.is_ascii_hexdigit()),
        None => label.bytes().all(|byte| byte.is_ascii_digit()),
    }
}

fn is_dns_label(label: &str) -> bool {
    (1..=MAX_DNS_LABEL_BYTES).contains(&label.len())
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};
    use std::fmt::Debug;
    use std::hash::Hash;

    use serde::Serialize;
    use serde::de::DeserializeOwned;
    use serde_json::{Value, json};

    use crate::{
        BrokerLaunch, FileAccess, MAX_SESSION_FILE_BYTES, MountRecipe, ProxyRecipe, ProxyTransport,
        RecipeError, SessionFileRecipe, UdsRecipe,
    };

    const SEED: &str = "/etc/plasmosome/seed";
    const UPSTREAM: &str = "/run/plasmosome/upstream.sock";
    const GUEST_SOCKET: &str = "/run/agent.sock";
    const PROGRAM: &str = "/usr/libexec/egressd";
    const CONTROL: &str = "/run/plasmosome/egressd.control";
    const DATA: &str = "/run/plasmosome/egressd.data";

    trait Recipe: Serialize + DeserializeOwned + Debug + Clone + Ord + Hash {
        fn check(&self) -> Result<(), RecipeError>;
    }

    macro_rules! recipes {
        ($($record:ty),*) => {
            $(impl Recipe for $record {
                fn check(&self) -> Result<(), RecipeError> {
                    self.validate()
                }
            })*
        };
    }

    recipes!(
        SessionFileRecipe,
        UdsRecipe,
        ProxyRecipe,
        MountRecipe,
        BrokerLaunch
    );

    fn session_file(contents: Vec<u8>, guest_path: &str) -> SessionFileRecipe {
        SessionFileRecipe {
            contents,
            mode: FileAccess::ReadOnly,
            guest_path: guest_path.to_string(),
        }
    }

    fn uds(upstream: &str, guest_path: &str) -> UdsRecipe {
        UdsRecipe {
            upstream: upstream.to_string(),
            guest_path: guest_path.to_string(),
        }
    }

    fn proxy(destination: &str, port: u16) -> ProxyRecipe {
        ProxyRecipe {
            transport: ProxyTransport::Tcp,
            destination: destination.to_string(),
            port,
            allow_private: false,
        }
    }

    fn launch(command: &[&str], control_socket: &str, data_socket: &str) -> BrokerLaunch {
        BrokerLaunch {
            command: command.iter().map(|word| word.to_string()).collect(),
            control_socket: control_socket.to_string(),
            data_socket: data_socket.to_string(),
        }
    }

    fn valid_session_file() -> SessionFileRecipe {
        session_file(b"seed\n".to_vec(), SEED)
    }

    fn valid_uds() -> UdsRecipe {
        uds(UPSTREAM, GUEST_SOCKET)
    }

    fn valid_proxy() -> ProxyRecipe {
        proxy("api.github.com", 443)
    }

    fn valid_mount() -> MountRecipe {
        MountRecipe {
            access: FileAccess::ReadWrite,
        }
    }

    fn valid_launch() -> BrokerLaunch {
        launch(&[PROGRAM, "--serve"], CONTROL, DATA)
    }

    fn decode<R: Recipe>(json: Value) -> Result<R, String> {
        serde_json::from_value(json).map_err(|error| error.to_string())
    }

    fn assert_accepted<R: Recipe>(recipe: &R) {
        assert_eq!(recipe.check(), Ok(()), "validate() refused {recipe:?}");
        let json = serde_json::to_value(recipe).unwrap();
        assert_eq!(decode(json).as_ref(), Ok(recipe), "decode of {recipe:?}");
    }

    fn assert_refused<R: Recipe>(recipe: &R, expected: RecipeError) {
        assert_eq!(
            recipe.check(),
            Err(expected.clone()),
            "validate() of {recipe:?}"
        );
        let json = serde_json::to_value(recipe).unwrap();
        assert_eq!(
            decode::<R>(json),
            Err(expected.to_string()),
            "decode of {recipe:?}"
        );
    }

    fn assert_round_trip<R: Recipe>(recipe: R, expected: Value) {
        assert_eq!(serde_json::to_value(&recipe).unwrap(), expected);
        assert_eq!(decode(expected), Ok(recipe));
    }

    fn assert_every_field_required<R: Recipe>(recipe: R, fields: &[&str]) {
        let full = serde_json::to_value(&recipe).unwrap();
        let mut present: Vec<&str> = full
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let mut expected = fields.to_vec();
        present.sort_unstable();
        expected.sort_unstable();
        assert_eq!(present, expected, "{recipe:?} encodes other fields");
        for field in fields {
            let mut partial = full.clone();
            partial.as_object_mut().unwrap().remove(*field);
            let missing = format!("missing field `{field}`");
            assert_eq!(decode::<R>(partial), Err(missing), "{recipe:?}");
        }
    }

    fn assert_unknown_field_refused<R: Recipe>(recipe: R, field: &str) {
        let mut json = serde_json::to_value(&recipe).unwrap();
        json.as_object_mut()
            .unwrap()
            .insert(field.to_string(), json!(1));
        let error = decode::<R>(json).unwrap_err();
        let unknown = format!("unknown field `{field}`");
        assert!(
            error.starts_with(&unknown),
            "{recipe:?} with `{field}`: {error}"
        );
    }

    fn assert_duplicate_field_refused<R: Recipe>(recipe: R, field: &str) {
        let json = serde_json::to_string(&recipe).unwrap();
        let value = serde_json::to_value(&recipe).unwrap()[field].to_string();
        let doubled = format!("{{\"{field}\":{value},{}", &json[1..]);
        let error = serde_json::from_str::<R>(&doubled).unwrap_err().to_string();
        let duplicate = format!("duplicate field `{field}`");
        assert!(error.starts_with(&duplicate), "{doubled} gave: {error}");
    }

    fn assert_distinct<R: Recipe>(recipes: Vec<R>) {
        let count = recipes.len();
        let ordered: BTreeSet<R> = recipes.iter().cloned().collect();
        let hashed: HashSet<R> = recipes.into_iter().collect();
        assert_eq!((ordered.len(), hashed.len()), (count, count));
    }

    fn nul(field: &'static str) -> RecipeError {
        RecipeError::ContainsNul { field }
    }

    fn relative(field: &'static str, value: &str) -> RecipeError {
        let value = value.to_string();
        RecipeError::NotAbsolute { field, value }
    }

    fn uncanonical(field: &'static str, value: &str) -> RecipeError {
        let value = value.to_string();
        RecipeError::NotCanonical { field, value }
    }

    fn invalid_destination(value: &str) -> RecipeError {
        let value = value.to_string();
        RecipeError::InvalidDestination { value }
    }

    fn too_large() -> RecipeError {
        RecipeError::ContentsTooLarge { len: 65_537 }
    }

    #[test]
    fn each_record_round_trips_with_the_spec_field_names() {
        assert_round_trip(
            valid_session_file(),
            json!({
                "contents": [115, 101, 101, 100, 10],
                "mode": "read_only",
                "guest_path": SEED,
            }),
        );
        assert_round_trip(
            valid_uds(),
            json!({ "upstream": UPSTREAM, "guest_path": GUEST_SOCKET }),
        );
        assert_round_trip(
            valid_proxy(),
            json!({
                "transport": "tcp",
                "destination": "api.github.com",
                "port": 443,
                "allow_private": false,
            }),
        );
        assert_round_trip(valid_mount(), json!({ "access": "read_write" }));
        assert_round_trip(
            valid_launch(),
            json!({
                "command": [PROGRAM, "--serve"],
                "control_socket": CONTROL,
                "data_socket": DATA,
            }),
        );
    }

    #[test]
    fn access_and_transport_encode_only_their_snake_case_names() {
        for (access, name) in [
            (FileAccess::ReadOnly, "read_only"),
            (FileAccess::ReadWrite, "read_write"),
        ] {
            assert_eq!(serde_json::to_value(access).unwrap(), json!(name));
            assert_eq!(
                serde_json::from_value::<FileAccess>(json!(name)).unwrap(),
                access
            );
        }
        for (transport, name) in [(ProxyTransport::Tcp, "tcp"), (ProxyTransport::Udp, "udp")] {
            assert_eq!(serde_json::to_value(transport).unwrap(), json!(name));
            let decoded = serde_json::from_value::<ProxyTransport>(json!(name)).unwrap();
            assert_eq!(decoded, transport);
        }
        for drifted in ["ReadOnly", "READ_ONLY", "read-only", "readonly", ""] {
            assert!(
                serde_json::from_value::<FileAccess>(json!(drifted)).is_err(),
                "{drifted}"
            );
        }
        for drifted in ["Tcp", "TCP", "sctp", ""] {
            assert!(
                serde_json::from_value::<ProxyTransport>(json!(drifted)).is_err(),
                "{drifted}"
            );
        }
    }

    #[test]
    fn removing_any_required_field_refuses_decode() {
        assert_every_field_required(valid_session_file(), &["contents", "mode", "guest_path"]);
        assert_every_field_required(valid_uds(), &["upstream", "guest_path"]);
        let proxy_fields = ["transport", "destination", "port", "allow_private"];
        assert_every_field_required(valid_proxy(), &proxy_fields);
        assert_every_field_required(valid_mount(), &["access"]);
        assert_every_field_required(
            valid_launch(),
            &["command", "control_socket", "data_socket"],
        );
    }

    #[test]
    fn an_unknown_field_refuses_decode() {
        assert_unknown_field_refused(valid_session_file(), "path");
        assert_unknown_field_refused(valid_uds(), "path");
        assert_unknown_field_refused(valid_proxy(), "host");
        assert_unknown_field_refused(valid_mount(), "source");
        assert_unknown_field_refused(valid_launch(), "pid");
    }

    #[test]
    fn a_repeated_field_refuses_decode_from_text() {
        assert_duplicate_field_refused(valid_session_file(), "guest_path");
        assert_duplicate_field_refused(valid_uds(), "upstream");
        assert_duplicate_field_refused(valid_proxy(), "port");
        assert_duplicate_field_refused(valid_mount(), "access");
        assert_duplicate_field_refused(valid_launch(), "data_socket");
    }

    #[test]
    fn a_nul_in_any_string_field_is_refused() {
        let seed = b"seed\n".to_vec();
        assert_refused(
            &session_file(seed, "/etc/plasmosome/se\0ed"),
            nul("guest_path"),
        );
        assert_refused(&uds("/run/up\0stream.sock", GUEST_SOCKET), nul("upstream"));
        assert_refused(&uds(UPSTREAM, "/run/agent\0.sock"), nul("guest_path"));
        assert_refused(&proxy("api.github.com\0", 443), nul("destination"));
        let program = format!("{PROGRAM}\0");
        assert_refused(
            &launch(&[&program, "--serve", "-q"], CONTROL, DATA),
            nul("command"),
        );
        assert_refused(
            &launch(&[PROGRAM, "--se\0rve", "-q"], CONTROL, DATA),
            nul("command"),
        );
        assert_refused(
            &launch(&[PROGRAM, "--serve", "-q\0"], CONTROL, DATA),
            nul("command"),
        );
        assert_refused(
            &launch(&[PROGRAM], "/run/con\0trol", DATA),
            nul("control_socket"),
        );
        assert_refused(
            &launch(&[PROGRAM], CONTROL, "/run/da\0ta"),
            nul("data_socket"),
        );
    }

    #[test]
    fn a_relative_value_in_any_absolute_field_is_refused() {
        for value in ["etc/plasmosome/seed", "./seed", "", " /etc/plasmosome/seed"] {
            let seed = b"seed\n".to_vec();
            assert_refused(&session_file(seed, value), relative("guest_path", value));
            assert_refused(&uds(value, GUEST_SOCKET), relative("upstream", value));
            assert_refused(&uds(UPSTREAM, value), relative("guest_path", value));
            let control = relative("control_socket", value);
            assert_refused(&launch(&[PROGRAM], value, DATA), control);
            assert_refused(
                &launch(&[PROGRAM], CONTROL, value),
                relative("data_socket", value),
            );
        }
        for program in [
            "egressd",
            "bin/egressd",
            "~/bin/egressd",
            "$HOME/bin/egressd",
            "",
        ] {
            let refused = relative("command", program);
            assert_refused(&launch(&[program, "--serve"], CONTROL, DATA), refused);
        }
    }

    #[test]
    fn a_path_with_an_empty_dot_or_dot_dot_component_is_refused() {
        for value in [
            "/",
            "//run/p/e.sock",
            "/run/p//e.sock",
            "/run/p/./e.sock",
            "/run/q/../p/e.sock",
            "/run/../etc/shadow",
            "/run/p/",
            "/run/p/.",
            "/run/p/..",
        ] {
            let seed = b"seed\n".to_vec();
            assert_refused(&session_file(seed, value), uncanonical("guest_path", value));
            assert_refused(&uds(value, GUEST_SOCKET), uncanonical("upstream", value));
            assert_refused(&uds(UPSTREAM, value), uncanonical("guest_path", value));
            assert_refused(
                &launch(&[value], CONTROL, DATA),
                uncanonical("command", value),
            );
            let control = uncanonical("control_socket", value);
            assert_refused(&launch(&[PROGRAM], value, DATA), control);
            let data = uncanonical("data_socket", value);
            assert_refused(&launch(&[PROGRAM], CONTROL, value), data);
        }
    }

    #[test]
    fn names_made_of_dots_are_ordinary_components_kept_exactly() {
        for value in [
            "/run/.hidden",
            "/run/..x",
            "/run/x.",
            "/run/x..y",
            "/run/...",
        ] {
            assert_accepted(&session_file(b"seed\n".to_vec(), value));
            assert_accepted(&uds(value, GUEST_SOCKET));
            assert_accepted(&uds(UPSTREAM, value));
            assert_accepted(&launch(&[value], CONTROL, DATA));
            assert_accepted(&launch(&[PROGRAM], value, DATA));
            assert_accepted(&launch(&[PROGRAM], CONTROL, value));
        }
    }

    #[test]
    fn only_the_program_word_must_be_absolute() {
        assert_accepted(&launch(&[PROGRAM], CONTROL, DATA));
        assert_accepted(&launch(
            &[
                PROGRAM,
                "relative/config",
                "",
                "-",
                "../up",
                "//twice",
                "/trailing/",
            ],
            CONTROL,
            DATA,
        ));
    }

    #[test]
    fn session_file_contents_may_hold_any_bytes_up_to_the_cap() {
        assert_eq!(MAX_SESSION_FILE_BYTES, 65_536);
        assert_accepted(&session_file(Vec::new(), SEED));
        assert_accepted(&session_file(vec![0, 255, 0], SEED));
        assert_accepted(&session_file(vec![b'x'; MAX_SESSION_FILE_BYTES], SEED));
        assert_refused(
            &session_file(vec![b'x'; MAX_SESSION_FILE_BYTES + 1], SEED),
            too_large(),
        );
        assert_refused(
            &session_file(vec![b'x'; 70_000], SEED),
            RecipeError::ContentsTooLarge { len: 70_000 },
        );
    }

    #[test]
    fn every_access_and_transport_is_accepted() {
        for mode in [FileAccess::ReadOnly, FileAccess::ReadWrite] {
            assert_accepted(&SessionFileRecipe {
                mode,
                ..valid_session_file()
            });
            assert_accepted(&MountRecipe { access: mode });
        }
        for transport in [ProxyTransport::Tcp, ProxyTransport::Udp] {
            for allow_private in [false, true] {
                let proxy = proxy("api.github.com", 443);
                assert_accepted(&ProxyRecipe {
                    transport,
                    allow_private,
                    ..proxy
                });
            }
        }
    }

    #[test]
    fn proxy_port_zero_is_refused_and_both_ends_of_the_range_accepted() {
        assert_refused(&proxy("api.github.com", 0), RecipeError::ZeroPort);
        assert_accepted(&proxy("api.github.com", 1));
        assert_accepted(&proxy("api.github.com", 65_535));
    }

    #[test]
    fn destination_accepts_one_dns_name_or_one_ip_literal() {
        let longest_label = "a".repeat(63);
        let longest_name = format!("{0}.{0}.{0}.{1}", "b".repeat(63), "c".repeat(61));
        assert_eq!(longest_name.len(), 253);
        for destination in [
            "api.github.com",
            "localhost",
            "a-b.example",
            "API.GitHub.com",
            "1password.com",
            "xn--bcher-kva.example",
            "example.c0m",
            "0.pool.ntp.org",
            "1.example",
            "123.example.com",
            "0x7f.example",
            "example.0xg",
            longest_label.as_str(),
            longest_name.as_str(),
            "10.0.0.1",
            "::1",
            "2001:db8::1",
            "::ffff:10.0.0.1",
        ] {
            assert_accepted(&proxy(destination, 443));
        }
    }

    #[test]
    fn destination_refuses_anything_but_one_name_or_address() {
        let long_label = format!("{}.example", "a".repeat(64));
        let long_name = format!("{0}.{0}.{0}.{1}", "b".repeat(63), "c".repeat(62));
        assert_eq!(long_name.len(), 254);
        for destination in [
            "https://api.github.com",
            "api.github.com/repos",
            "*.github.com",
            "user@api.github.com",
            "api.github.com:443",
            "10.0.0.1:443",
            "[::1]",
            "fe80::1%en0",
            "a..b",
            ".example",
            long_label.as_str(),
            long_name.as_str(),
            "-api.github.com",
            "api-.github.com",
            "api.github.com-",
            "api.github.com.",
            "999.1.1.1",
            "example.123",
            "a.1.1.1",
            "api.github.443",
            "api.-github.com",
            "0x7f000001",
            "127.0.0.0x1",
            "0x7f.0x0.0x0.0x1",
            "example.0x",
            "example.0X1F",
            "10.0.0",
            "010.0.0.1",
            "bücher.example",
            "api_github.com",
            "api github.com",
            " api.github.com",
            "",
        ] {
            assert_refused(&proxy(destination, 443), invalid_destination(destination));
        }
    }

    #[test]
    fn broker_launch_needs_a_program_and_two_distinct_endpoints() {
        assert_refused(&launch(&[], CONTROL, DATA), RecipeError::EmptyCommand);
        let path = "/run/plasmosome/egressd.sock".to_string();
        let shared = RecipeError::SharedEndpoint { path: path.clone() };
        assert_refused(&launch(&[PROGRAM], &path, &path), shared);
    }

    #[test]
    fn launch_words_are_plain_data_kept_exactly() {
        let command = [
            "/opt/egress broker/bin/egressd",
            "--name=two words",
            "'single'",
            "\"double\"",
            "$HOME",
            "~",
            "a;b && c",
            "*",
            "",
        ];
        let launch = launch(&command, CONTROL, DATA);
        assert_accepted(&launch);
        assert_eq!(
            serde_json::to_value(&launch).unwrap()["command"],
            json!(command)
        );
    }

    #[test]
    fn a_value_breaking_several_rules_reports_the_first_in_field_order() {
        let oversized = vec![0; MAX_SESSION_FILE_BYTES + 1];
        assert_refused(&session_file(oversized, "rel\0ative"), too_large());
        assert_refused(&session_file(Vec::new(), "rel\0ative"), nul("guest_path"));
        let upstream = relative("upstream", "relative.sock");
        assert_refused(&uds("relative.sock", "rel\0ative"), upstream);
        assert_refused(&uds(UPSTREAM, "rel\0ative"), nul("guest_path"));
        assert_refused(&proxy("https://x\0", 0), nul("destination"));
        assert_refused(&proxy("https://x", 0), invalid_destination("https://x"));
        let same = "relative.sock";
        assert_refused(&launch(&[], same, same), RecipeError::EmptyCommand);
        assert_refused(
            &launch(&["egressd", "--serve\0"], same, same),
            nul("command"),
        );
        let program = relative("command", "egressd");
        assert_refused(&launch(&["egressd"], "rel\0ative", "relative"), program);
        let control = nul("control_socket");
        assert_refused(&launch(&[PROGRAM], "rel\0ative", "relative"), control);
        assert_refused(
            &launch(&[PROGRAM], same, same),
            relative("control_socket", same),
        );
        assert_refused(
            &launch(&[PROGRAM], CONTROL, same),
            relative("data_socket", same),
        );
        let seed = b"seed\n".to_vec();
        assert_refused(
            &session_file(seed.clone(), "run//x"),
            relative("guest_path", "run//x"),
        );
        assert_refused(&session_file(seed, "/run//x\0"), nul("guest_path"));
        let spelled_twice = uncanonical("data_socket", "/run/p//e.sock");
        assert_refused(
            &launch(&[PROGRAM], "/run/p/e.sock", "/run/p//e.sock"),
            spelled_twice,
        );
        let both = uncanonical("control_socket", "/run//e.sock");
        assert_refused(&launch(&[PROGRAM], "/run//e.sock", "/run//e.sock"), both);
    }

    #[test]
    fn each_error_names_its_field_and_offending_value() {
        let shared = "/run/plasmosome/egressd.sock".to_string();
        let errors = [
            (nul("guest_path"), vec!["guest_path", "NUL"]),
            (
                relative("upstream", "relative.sock"),
                vec!["upstream", "relative.sock"],
            ),
            (
                uncanonical("data_socket", "/run/p//e.sock"),
                vec!["data_socket", "/run/p//e.sock"],
            ),
            (too_large(), vec!["contents", "65537", "65536"]),
            (RecipeError::ZeroPort, vec!["port"]),
            (invalid_destination("x:443"), vec!["destination", "x:443"]),
            (RecipeError::EmptyCommand, vec!["command"]),
            (
                RecipeError::SharedEndpoint {
                    path: shared.clone(),
                },
                vec!["control_socket", "data_socket", shared.as_str()],
            ),
        ];
        let mut texts = BTreeSet::new();
        for (error, named) in &errors {
            let _: &dyn std::error::Error = error;
            let text = error.to_string();
            for name in named {
                assert!(text.contains(name), "{text:?} does not name {name:?}");
            }
            texts.insert(text);
        }
        assert_eq!(texts.len(), errors.len(), "two variants share one message");
    }

    #[test]
    fn records_order_and_hash_over_every_field_like_capability() {
        assert_distinct(vec![
            valid_session_file(),
            session_file(b"other\n".to_vec(), SEED),
            SessionFileRecipe {
                mode: FileAccess::ReadWrite,
                ..valid_session_file()
            },
            session_file(b"seed\n".to_vec(), "/etc/plasmosome/other"),
        ]);
        assert_distinct(vec![
            valid_uds(),
            uds("/run/plasmosome/other.sock", GUEST_SOCKET),
            uds(UPSTREAM, "/run/other.sock"),
        ]);
        assert_distinct(vec![
            valid_proxy(),
            ProxyRecipe {
                transport: ProxyTransport::Udp,
                ..valid_proxy()
            },
            proxy("10.0.0.1", 443),
            proxy("api.github.com", 8443),
            ProxyRecipe {
                allow_private: true,
                ..valid_proxy()
            },
        ]);
        assert_distinct(vec![
            valid_mount(),
            MountRecipe {
                access: FileAccess::ReadOnly,
            },
        ]);
        assert_distinct(vec![
            valid_launch(),
            launch(&[PROGRAM, "--other"], CONTROL, DATA),
            launch(&[PROGRAM, "--serve"], "/run/other.control", DATA),
            launch(&[PROGRAM, "--serve"], CONTROL, "/run/other.data"),
        ]);
    }
}
