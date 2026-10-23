use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;
use std::hash::Hash;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

pub use plasmosome_backend::is_proxy_host;
use plasmosome_backend::{canonical_ip_literal, canonical_path};

use crate::toml_depth;

mod strict;

/// The most bytes a declaration may hold: spec 020's limit for one
/// declaration. `load` and `parse` refuse a longer one with
/// `ManifestError::TooLarge`.
pub const MAX_MANIFEST_BYTES: usize = 1 << 20;

pub(crate) const MAX_REFUSAL_BYTES: usize = 4096;
pub(crate) const MAX_SHOWN_FIX_BYTES: usize = 1024;

/// The deepest a declaration may nest. The root table is level 1. Each array
/// and each inline table adds a level, and so does each dot in a key, for the
/// table it implies. Each part of a table header's name adds two levels,
/// enough for an array of tables and its element. Nothing inside a string or
/// a comment counts. `parse` refuses a deeper declaration with
/// `ManifestError::TooDeep` before any TOML is parsed.
pub const MAX_MANIFEST_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct PlasmidManifest {
    pub id: String,
    pub description: String,
    pub version: String,
    pub wasm: Option<PathBuf>,
    pub network: Option<NetworkSpec>,
    pub requires: Vec<String>,
    pub provides_tools: Vec<ToolDeclaration>,
    /// The `[provides]` binding keys, such as `github:tools`, as written, in the order the file
    /// gives them: a binding with its own `[provides."<key>"]` header where that header is, and
    /// any other binding where its key first appears.
    pub provides: Vec<String>,
    pub secrets: Vec<SecretRef>,
    pub commands: Option<CommandsSpec>,
    pub workspace: Option<WorkspaceMount>,
    pub mock: Option<MockSpec>,
    pub model: Option<ModelSpec>,
    pub drain_ms: Option<u64>,
}

/// A tool name and the author-written description a registry consumer reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDeclaration {
    pub name: String,
    pub description: String,
}

/// The top-level `[network]`, with spec 022 §6's exact values: at least one host, every host a
/// lower-case DNS name that `is_proxy_host` accepts, at least one port, and no host or port
/// written twice. Each pin is a `Cidr` with no host bits set.
#[derive(Debug, Clone, PartialEq)]
pub struct NetworkSpec {
    pub hosts: Vec<String>,
    pub ports: Vec<u16>,
    pub pin_cidrs: Vec<Cidr>,
}

/// A command's own `[network]`, with its values as written: `hosts` and `pin_cidrs` are lists
/// of strings and every port is from 1 to 65535. Nothing else is checked, so a host need not be
/// a DNS name, a pin need not be a CIDR, and either list may be empty or repeat an entry.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandNetwork {
    pub hosts: Vec<String>,
    pub ports: Vec<u16>,
    pub pin_cidrs: Vec<String>,
}

/// An IP network written as `<address>/<prefix>`, such as `192.0.2.0/24`, with no bit set past
/// the prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cidr {
    network: IpAddr,
    prefix: u8,
}

/// Why `Cidr::parse` refused a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CidrError {
    /// The text is not `<address>/<prefix>` in its one accepted spelling.
    Malformed,
    /// The address has a bit set past the prefix. `network` is the same address and prefix with
    /// those bits cleared.
    HostBitsSet { network: Cidr },
}

impl Cidr {
    /// Parses `<address>/<prefix>`. The address must pass `plasmosome_backend`'s
    /// `canonical_ip_literal`, the rule `ProxyRecipe::validate` applies to a destination: spelled
    /// as Rust's `IpAddr` displays it, with no IPv4 address in IPv4-mapped or IPv4-compatible
    /// form. So `192.000.002.000/24`, `2001:DB8::/32`, `::ffff:192.0.2.0/120` and `::2/128` are
    /// `Malformed`, while `::/0` and `::1/128` parse, since `::` and `::1` are not
    /// IPv4-compatible. The prefix is 0 to 32 for IPv4 and 0 to 128 for IPv6, in decimal digits
    /// with no sign and no leading zero. An address with a bit set past the prefix is
    /// `HostBitsSet`, never rewritten.
    pub fn parse(text: &str) -> Result<Cidr, CidrError> {
        let (written, prefix) = text.split_once('/').ok_or(CidrError::Malformed)?;
        let address = canonical_ip_literal(written).ok_or(CidrError::Malformed)?;
        let width = if address.is_ipv4() { 32 } else { 128 };
        let prefix = prefix_length(prefix, width).ok_or(CidrError::Malformed)?;
        let network = Cidr {
            network: masked(address, prefix),
            prefix,
        };
        if network.network != address {
            return Err(CidrError::HostBitsSet { network });
        }
        Ok(network)
    }

    /// True when `address` is of this network's family and its first `prefix` bits are the
    /// network's. An IPv4-mapped IPv6 address is IPv6, so `192.0.2.0/24` does not contain
    /// `::ffff:192.0.2.1`.
    pub fn contains(&self, address: IpAddr) -> bool {
        address.is_ipv4() == self.network.is_ipv4() && masked(address, self.prefix) == self.network
    }
}

impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix)
    }
}

fn prefix_length(text: &str, width: u8) -> Option<u8> {
    let digits = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    if !digits || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse().ok().filter(|prefix| *prefix <= width)
}

fn masked(address: IpAddr, prefix: u8) -> IpAddr {
    match address {
        IpAddr::V4(v4) => {
            let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            IpAddr::V4(Ipv4Addr::from(u32::from(v4) & mask))
        }
        IpAddr::V6(v6) => {
            let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            IpAddr::V6(Ipv6Addr::from(u128::from(v6) & mask))
        }
    }
}

/// The `[workspace].mount` a declaration writes. `dst` is the guest path, `/workspace` when the
/// declaration leaves it out (spec 022 §6). A written `backend` is not kept: it selects nothing
/// (spec 011), so nothing here can choose how the Mount is realized.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceMount {
    /// The guest path the Mount appears at.
    pub dst: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MockSpec {
    pub hosts: Vec<String>,
    pub kind: String,
    pub api: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelSpec {
    pub endpoint: String,
    pub credential: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryMode {
    Handle,
    Helper,
    Inject,
    Mint,
}

impl DeliveryMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeliveryMode::Handle => "handle",
            DeliveryMode::Helper => "helper",
            DeliveryMode::Inject => "inject",
            DeliveryMode::Mint => "mint",
        }
    }

    pub fn parse(text: &str) -> Option<DeliveryMode> {
        match text {
            "handle" => Some(DeliveryMode::Handle),
            "helper" => Some(DeliveryMode::Helper),
            "inject" => Some(DeliveryMode::Inject),
            "mint" => Some(DeliveryMode::Mint),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecretConsumer {
    Wasm,
    Git,
    Http,
    Process,
}

impl SecretConsumer {
    pub fn as_str(&self) -> &'static str {
        match self {
            SecretConsumer::Wasm => "wasm",
            SecretConsumer::Git => "git",
            SecretConsumer::Http => "http",
            SecretConsumer::Process => "process",
        }
    }

    pub fn parse(text: &str) -> Option<SecretConsumer> {
        match text {
            "wasm" => Some(SecretConsumer::Wasm),
            "git" => Some(SecretConsumer::Git),
            "http" => Some(SecretConsumer::Http),
            "process" => Some(SecretConsumer::Process),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct SecretScope {
    #[serde(default)]
    pub path_scope: Vec<String>,
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SecretRef {
    pub id: String,
    pub consumer: SecretConsumer,
    #[serde(rename = "delivery")]
    pub delivery: Vec<DeliveryMode>,
    #[serde(default)]
    pub scope: Option<SecretScope>,
    #[serde(default)]
    pub ttl: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandDecl {
    pub id: String,
    pub exec: Vec<String>,
    pub subject: Option<String>,
    pub network: Option<CommandNetwork>,
    pub secrets: Vec<SecretRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandsSpec {
    pub address_plan: String,
    pub commands: Vec<CommandDecl>,
}

/// A declaration refusal. Its text is at most 4,096 bytes, keeping the first 2,046 and last
/// 2,047 around U+2026, cut at character boundaries. A fix over 1,024 bytes is given by its
/// length in that text; the fields themselves remain whole. Formatting first builds the full
/// text, using time and memory linear in the declaration (about 10 MB for a 1 MiB declaration).
/// `Debug` is unbounded and can include the complete declaration in a TOML parser error.
#[derive(Debug)]
pub enum ManifestError {
    Io(std::io::Error),
    Parse(toml::de::Error),
    Field {
        plasmid: Option<String>,
        field: String,
        fix: String,
        detail: String,
    },
    TooLarge {
        limit: usize,
    },
    TooDeep {
        limit: usize,
    },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut text = String::new();
        use std::fmt::Write;
        match self {
            ManifestError::Io(e) => write!(text, "manifest io error: {e}"),
            ManifestError::Parse(e) => {
                write!(
                    text,
                    "manifest toml error: {}",
                    visible_lines(&e.to_string())
                )
            }
            ManifestError::Field {
                plasmid,
                field,
                fix,
                detail,
            } => {
                if let Some(id) = plasmid {
                    write!(text, "plasmid {}: ", diagnostic_key(id))?;
                }
                if fix == REMOVE_ENTRY {
                    write!(text, "{field}: {detail}; {fix}")
                } else if fix.len() > MAX_SHOWN_FIX_BYTES {
                    write!(
                        text,
                        "{field}: {detail}; its fix is {} bytes, too long to show here",
                        fix.len()
                    )
                } else {
                    write!(text, "{field}: {detail}; write {fix}")
                }
            }
            ManifestError::TooLarge { limit } => {
                write!(text, "manifest exceeds the {limit}-byte limit")
            }
            ManifestError::TooDeep { limit } => {
                write!(
                    text,
                    "manifest nesting counts more than the {limit}-level limit"
                )
            }
        }?;
        f.write_str(&bounded(&text))
    }
}

fn bounded(text: &str) -> Cow<'_, str> {
    if text.len() <= MAX_REFUSAL_BYTES {
        return Cow::Borrowed(text);
    }
    let head = text.floor_char_boundary(2046);
    let tail = text.ceil_char_boundary(text.len() - 2047);
    Cow::Owned(format!("{}\u{2026}{}", &text[..head], &text[tail..]))
}

impl std::error::Error for ManifestError {}

impl PlasmidManifest {
    /// Reads the declaration at `path` and parses it. At most
    /// `MAX_MANIFEST_BYTES` plus one byte is read, so a file, device or FIFO
    /// that delivers more than that is refused with `TooLarge` at the bound,
    /// and nothing past it is parsed. Time is not bounded: a FIFO with no
    /// writer blocks this call while it opens the FIFO, and a FIFO whose
    /// writer delivers at most `MAX_MANIFEST_BYTES` and never closes blocks it while it
    /// reads. A file that is not UTF-8 is an `Io` error of kind `InvalidData`.
    /// What was read goes to `parse`, which refuses a declaration nested
    /// deeper than `MAX_MANIFEST_DEPTH` with `TooDeep`.
    pub fn load(path: &Path) -> Result<PlasmidManifest, ManifestError> {
        let file = std::fs::File::open(path).map_err(ManifestError::Io)?;
        let mut bytes = Vec::new();
        file.take(MAX_MANIFEST_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(ManifestError::Io)?;
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge {
                limit: MAX_MANIFEST_BYTES,
            });
        }
        let text = String::from_utf8(bytes).map_err(|error| {
            ManifestError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                error.utf8_error(),
            ))
        })?;
        Self::parse(&text)
    }

    /// Parses a declaration. Text longer than `MAX_MANIFEST_BYTES` is refused
    /// with `TooLarge`, and then text nested deeper than `MAX_MANIFEST_DEPTH`
    /// with `TooDeep`, both before any of it is parsed as TOML.
    pub fn parse(text: &str) -> Result<PlasmidManifest, ManifestError> {
        if text.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge {
                limit: MAX_MANIFEST_BYTES,
            });
        }
        if toml_depth::deeper_than(text, MAX_MANIFEST_DEPTH) {
            return Err(ManifestError::TooDeep {
                limit: MAX_MANIFEST_DEPTH,
            });
        }
        let raw: toml::Value = toml::from_str(text).map_err(ManifestError::Parse)?;
        let id = raw
            .get("id")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| id_error("a string id is required"))?
            .to_string();
        if id.trim().is_empty() {
            return Err(id_error("the id must not be empty or only whitespace"));
        }
        let description = raw
            .get("description")
            .and_then(toml::Value::as_str)
            .filter(|description| !description.trim().is_empty())
            .ok_or_else(|| {
                field_error(
                    &id,
                    "description".into(),
                    "description = \"Describe what this plasmid is for.\"".into(),
                    "a nonblank description string is required",
                )
            })?
            .to_string();
        strict::check(&id, text, &raw)?;
        let version = raw
            .get("version")
            .and_then(toml::Value::as_str)
            .unwrap_or("0.0.0")
            .to_string();
        let wasm = raw
            .get("impl")
            .and_then(|impl_table| impl_table.get("wasm"))
            .and_then(toml::Value::as_str)
            .map(PathBuf::from);
        let requires = raw
            .get("requires")
            .map(|r| {
                string_list(
                    &id,
                    "requires.capabilities",
                    r.get("capabilities"),
                    "capabilities = []",
                )
            })
            .transpose()?
            .unwrap_or_default();
        let provides_tools = raw
            .get("provides")
            .map(|provides| parse_tools(&id, provides))
            .transpose()?
            .unwrap_or_default();
        let provides = raw
            .get("provides")
            .and_then(toml::Value::as_table)
            .map(|bindings| binding_keys(text, bindings))
            .transpose()?
            .unwrap_or_default();
        let secrets = raw
            .get("secrets")
            .map(|secrets| parse_secret_refs(&id, "secrets", secrets))
            .transpose()?
            .unwrap_or_default();
        let commands = raw
            .get("commands")
            .map(|c| parse_commands(&id, c))
            .transpose()?;
        let workspace = raw
            .get("workspace")
            .map(|workspace| parse_workspace(&id, workspace))
            .transpose()?
            .flatten();
        let mock = raw
            .get("mock")
            .map(|m| {
                let backend = m.get("backend");
                let source = backend
                    .and_then(|b| b.get("source"))
                    .and_then(toml::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let kind = backend
                    .and_then(|b| b.get("kind"))
                    .and_then(toml::Value::as_str)
                    .unwrap_or("recorded")
                    .to_string();
                let api = m
                    .get("api")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("github")
                    .to_string();
                Ok(MockSpec {
                    hosts: declared_string_list(
                        &id,
                        "mock",
                        m.get("hosts"),
                        ListRepair::MockHosts,
                    )?,
                    kind,
                    api,
                    source,
                })
            })
            .transpose()?;
        let mock_hosts = mock
            .as_ref()
            .map(|mock| mock.hosts.as_slice())
            .unwrap_or_default();
        let network = raw
            .get("network")
            .map(|n| parse_network(&id, "network", n, mock_hosts))
            .transpose()?;
        let model = raw.get("model").map(|m| ModelSpec {
            endpoint: m
                .get("endpoint")
                .and_then(toml::Value::as_str)
                .unwrap_or("https://api.openai.com/v1/chat/completions")
                .to_string(),
            credential: m
                .get("credential")
                .and_then(toml::Value::as_str)
                .unwrap_or("model-provider/key")
                .to_string(),
        });
        let drain_ms = raw
            .get("lifecycle")
            .and_then(|l| l.get("drain_ms"))
            .and_then(toml::Value::as_integer)
            .and_then(|v| u64::try_from(v).ok());
        if wasm.is_none()
            && network.is_none()
            && workspace.is_none()
            && mock.is_none()
            && model.is_none()
            && commands
                .as_ref()
                .is_none_or(|commands| commands.commands.is_empty())
        {
            if commands.is_some() {
                return Err(field_error(
                    &id,
                    "commands.commands".into(),
                    command_sections(&[EXAMPLE_COMMAND.into()]),
                    "the commands table declares no command",
                ));
            }
            return Err(field_error(
                &id,
                "impl".into(),
                "[impl]\nwasm = \"component.wasm\"".into(),
                "the declaration names no capability and no implementation",
            ));
        }
        if let Some(spec) = &mock {
            validate_mock(&id, spec, network.as_ref())?;
        }
        let network = network
            .map(|written| exact_network(&id, written))
            .transpose()?;
        if let Some(commands) = &commands {
            validate_commands(&id, commands)?;
        }
        Ok(PlasmidManifest {
            id,
            description,
            version,
            wasm,
            network,
            requires,
            provides_tools,
            provides,
            secrets,
            commands,
            workspace,
            mock,
            model,
            drain_ms,
        })
    }

    pub fn declares_any_host(&self, host: &str) -> bool {
        self.network
            .as_ref()
            .is_some_and(|n| n.hosts.iter().any(|h| h == host))
    }
}

fn id_error(detail: &str) -> ManifestError {
    ManifestError::Field {
        plasmid: None,
        field: "id".into(),
        fix: "id = \"choose-a-stable-id\"".into(),
        detail: detail.to_string(),
    }
}

fn field_error(id: &str, field: String, fix: String, detail: &str) -> ManifestError {
    ManifestError::Field {
        plasmid: Some(id.to_string()),
        field,
        fix,
        detail: detail.to_string(),
    }
}

pub(crate) fn diagnostic_key(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if bare {
        name.to_string()
    } else {
        toml_string(name)
    }
}

pub(crate) fn toml_string(text: &str) -> String {
    let mut quoted = String::from('"');
    for character in text.chars() {
        if matches!(character, '"' | '\\') {
            quoted.push('\\');
            quoted.push(character);
        } else {
            push_visible(&mut quoted, character);
        }
    }
    quoted.push('"');
    quoted
}

fn without_controls(text: &str) -> String {
    let mut visible = String::with_capacity(text.len());
    for character in text.chars() {
        push_visible(&mut visible, character);
    }
    visible
}

fn visible_lines(text: &str) -> String {
    let lines: Vec<String> = text.split('\n').map(without_controls).collect();
    lines.join("\n")
}

pub(crate) fn acted_on(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061C}' | '\u{2028}' | '\u{2029}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        )
}

fn push_visible(text: &mut String, character: char) {
    let escape = match character {
        '\x08' => 'b',
        '\t' => 't',
        '\n' => 'n',
        '\x0C' => 'f',
        '\r' => 'r',
        control if acted_on(control) => {
            text.push('\\');
            text.push_str(&format!("u{:04X}", u32::from(control)));
            return;
        }
        visible => {
            text.push(visible);
            return;
        }
    };
    text.push('\\');
    text.push(escape);
}

fn parse_tools(id: &str, provides: &toml::Value) -> Result<Vec<ToolDeclaration>, ManifestError> {
    let bindings = provides.as_table().ok_or_else(|| {
        field_error(
            id,
            "provides".into(),
            "[provides]".into(),
            "expected a table",
        )
    })?;
    let mut tools = Vec::new();
    for (name, binding) in bindings {
        let binding_path = || format!("provides.{}", diagnostic_key(name));
        if name.trim().is_empty() {
            return Err(field_error(
                id,
                binding_path(),
                REMOVE_ENTRY.into(),
                "a capability key must not be empty or only whitespace",
            ));
        }
        let binding = binding.as_table().ok_or_else(|| {
            let field = binding_path();
            let fix = format!("[{field}]");
            field_error(id, field, fix, "expected a capability binding table")
        })?;
        let declarations = binding.get("tools");
        let declarations = declarations.and_then(toml::Value::as_table).ok_or_else(|| {
            let field = format!("{}.tools", binding_path());
            let first_tool = declarations
                .and_then(toml::Value::as_array)
                .and_then(|names| names.first())
                .and_then(toml::Value::as_str);
            let (fix, detail) = match first_tool {
                Some(tool) if tool.trim().is_empty() => {
                    return field_error(id, format!("{field}[0]"), REMOVE_ENTRY.into(), BLANK_TOOL);
                }
                Some(tool) => (
                    format!(
                        "[{field}]\n{} = \"Describe what this tool does.\"",
                        diagnostic_key(tool)
                    ),
                    format!(
                        "tool {} needs a description; replace the names list with a table",
                        toml_string(tool)
                    ),
                ),
                None => (
                    format!("[{field}]"),
                    "expected a tools table; an empty table declares no tools until entries are written".into(),
                ),
            };
            field_error(id, field, fix, &detail)
        })?;
        for (name, description) in declarations {
            if name.trim().is_empty() {
                let field = format!("{}.tools.{}", binding_path(), diagnostic_key(name));
                return Err(field_error(id, field, REMOVE_ENTRY.into(), BLANK_TOOL));
            }
            let description = description
                .as_str()
                .filter(|description| !description.trim().is_empty())
                .ok_or_else(|| {
                    let key = diagnostic_key(name);
                    field_error(
                        id,
                        format!("{}.tools.{key}", binding_path()),
                        format!("{key} = \"Describe what this tool does.\""),
                        "a nonblank tool description string is required",
                    )
                })?;
            tools.push(ToolDeclaration {
                name: name.clone(),
                description: description.to_string(),
            });
        }
    }
    Ok(tools)
}

fn parse_network(
    id: &str,
    field_path: &str,
    n: &toml::Value,
    mock_hosts: &[String],
) -> Result<CommandNetwork, ManifestError> {
    if !n.is_table() {
        let hosts = evident_or(n, mock_hosts, EXAMPLE_HOST);
        return Err(field_error(
            id,
            field_path.to_string(),
            format!("[{field_path}]\n{}", strings_line("hosts", &hosts)),
            &format!("{} is not a table", shown(n)),
        ));
    }
    let hosts = declared_string_list(
        id,
        field_path,
        n.get("hosts"),
        ListRepair::Hosts(mock_hosts),
    )?;
    let ports = declared_ports(id, &format!("{field_path}.ports"), n.get("ports"))?;
    let pin_cidrs = declared_string_list(id, field_path, n.get("pin_cidrs"), ListRepair::Pins)?;
    Ok(CommandNetwork {
        hosts,
        ports,
        pin_cidrs,
    })
}

fn exact_network(id: &str, written: CommandNetwork) -> Result<NetworkSpec, ManifestError> {
    let CommandNetwork {
        hosts,
        ports,
        pin_cidrs,
    } = written;
    let refused =
        |field: String, fix: String, detail: &str| Err(field_error(id, field, fix, detail));
    match (hosts.is_empty(), ports.is_empty()) {
        (true, true) => {
            return refused(
                "network".into(),
                REMOVE_ENTRY.into(),
                "a network section with no host and no port declares nothing",
            );
        }
        (true, false) => {
            return refused(
                "network.hosts".into(),
                strings_line("hosts", &[EXAMPLE_HOST.into()]),
                "a network section must name at least one host",
            );
        }
        (false, true) => {
            return refused(
                "network.hosts[0]".into(),
                REMOVE_ENTRY.into(),
                "a network section that names hosts must declare a port",
            );
        }
        (false, false) => {}
    }
    if let Some(index) = first_repeat(&ports) {
        return refused(
            format!("network.ports[{index}]"),
            REMOVE_ENTRY.into(),
            &format!("{} is already declared", ports[index]),
        );
    }
    let declared: HashSet<&str> = hosts.iter().map(String::as_str).collect();
    if let Some((index, host)) = hosts
        .iter()
        .enumerate()
        .find(|(_, host)| !is_proxy_host(host))
    {
        let lower = host.to_ascii_lowercase();
        let fix = if is_proxy_host(&lower) && !declared.contains(lower.as_str()) {
            toml_string(&lower)
        } else {
            REMOVE_ENTRY.into()
        };
        return refused(
            format!("network.hosts[{index}]"),
            fix,
            &format!("{} is not a lower-case DNS name", toml_string(host)),
        );
    }
    if let Some(index) = first_repeat(&hosts) {
        return refused(
            format!("network.hosts[{index}]"),
            REMOVE_ENTRY.into(),
            &format!("{} is already declared", toml_string(&hosts[index])),
        );
    }
    let pin_cidrs = exact_pins(id, &pin_cidrs)?;
    Ok(NetworkSpec {
        hosts,
        ports,
        pin_cidrs,
    })
}

fn first_repeat<T: Eq + Hash>(items: &[T]) -> Option<usize> {
    let mut seen = HashSet::with_capacity(items.len());
    items.iter().position(|item| !seen.insert(item))
}

fn exact_pins(id: &str, written: &[String]) -> Result<Vec<Cidr>, ManifestError> {
    let parsed: Vec<Result<Cidr, CidrError>> = written.iter().map(|pin| Cidr::parse(pin)).collect();
    let refused = |index: usize, fix: String, detail: String| {
        Err(field_error(
            id,
            format!("network.pin_cidrs[{index}]"),
            fix,
            &detail,
        ))
    };
    let masked = parsed
        .iter()
        .enumerate()
        .find_map(|(index, pin)| match pin {
            Err(CidrError::HostBitsSet { network }) => Some((index, network)),
            _ => None,
        });
    if let Some((index, network)) = masked {
        return refused(
            index,
            toml_string(&network.to_string()),
            format!(
                "{} has host bits set past its prefix",
                toml_string(&written[index])
            ),
        );
    }
    if let Some(index) = parsed.iter().position(Result::is_err) {
        let fix = if parsed.iter().any(Result::is_ok) {
            REMOVE_ENTRY.into()
        } else {
            toml_string(EXAMPLE_PIN)
        };
        return refused(
            index,
            fix,
            format!(
                "{} is not a CIDR such as {}",
                toml_string(&written[index]),
                toml_string(EXAMPLE_PIN)
            ),
        );
    }
    Ok(parsed.into_iter().flatten().collect())
}

const DEFAULT_DST: &str = "/workspace";

fn parse_workspace(
    id: &str,
    workspace: &toml::Value,
) -> Result<Option<WorkspaceMount>, ManifestError> {
    let Some(table) = workspace.as_table() else {
        return Ok(None);
    };
    let Some(mount) = table.get("mount") else {
        return Err(field_error(
            id,
            "workspace.mount".into(),
            format!("mount = {{ dst = {} }}", toml_string(DEFAULT_DST)),
            "a workspace section must declare its mount",
        ));
    };
    let refused = |fix: &str, detail: String| {
        Err(field_error(
            id,
            "workspace.mount.dst".into(),
            format!("dst = {}", toml_string(fix)),
            &detail,
        ))
    };
    let dst = match mount.get("dst") {
        None => DEFAULT_DST,
        Some(toml::Value::String(dst)) if canonical_path("dst", dst).is_ok() => dst,
        Some(toml::Value::String(dst)) => {
            let rooted = if dst.starts_with('/') {
                dst.clone()
            } else {
                format!("/{dst}")
            };
            let trimmed = rooted.trim_end_matches('/');
            let fix = if canonical_path("dst", trimmed).is_ok() {
                trimmed
            } else {
                DEFAULT_DST
            };
            return refused(
                fix,
                format!("{} is not a canonical absolute path", toml_string(dst)),
            );
        }
        Some(other) => {
            return refused(
                DEFAULT_DST,
                format!("{} is not a path string", shown(other)),
            );
        }
    };
    Ok(Some(WorkspaceMount {
        dst: dst.to_string(),
    }))
}

fn binding_keys(text: &str, bindings: &toml::Table) -> Result<Vec<String>, ManifestError> {
    if bindings.len() < 2 {
        return Ok(bindings.keys().cloned().collect());
    }
    let in_parse_order: ProvidesInParseOrder =
        toml::from_str(text).map_err(ManifestError::Parse)?;
    Ok(in_parse_order.provides.0)
}

#[derive(serde::Deserialize)]
struct ProvidesInParseOrder {
    provides: BindingKeys,
}

struct BindingKeys(Vec<String>);

impl<'de> serde::Deserialize<'de> for BindingKeys {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(BindingKeysVisitor)
    }
}

struct BindingKeysVisitor;

impl<'de> serde::de::Visitor<'de> for BindingKeysVisitor {
    type Value = BindingKeys;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a [provides] table")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<BindingKeys, A::Error> {
        let mut keys = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            map.next_value::<serde::de::IgnoredAny>()?;
            keys.push(key);
        }
        Ok(BindingKeys(keys))
    }
}

fn parse_secret_refs(
    id: &str,
    section: &str,
    secrets: &toml::Value,
) -> Result<Vec<SecretRef>, ManifestError> {
    let Some(refs) = secrets.get("refs") else {
        return Ok(Vec::new());
    };
    let items = refs.as_array().ok_or_else(|| {
        field_error(
            id,
            format!("{section}.refs"),
            "refs = [{ id = \"credential-id\", consumer = \"wasm\" }]".into(),
            "credential refs must be a list",
        )
    })?;
    let mut parsed = Vec::with_capacity(items.len());
    for (index, original) in items.iter().enumerate() {
        let field = |name: &str| format!("{section}.refs[{index}].{name}");
        let mut item = original.clone();
        normalize_scope(&mut item);
        let consumer = item
            .get("consumer")
            .and_then(toml::Value::as_str)
            .and_then(SecretConsumer::parse)
            .ok_or_else(|| {
                field_error(
                    id,
                    field("consumer"),
                    "consumer = \"wasm\"".into(),
                    "consumer must be wasm, git, http or process",
                )
            })?;
        let paths = item.get("scope").and_then(|scope| scope.get("path_scope"));
        let derived = narrowest_delivery(consumer, paths.is_some());
        let delivery_omitted = item.get("delivery").is_none();
        if delivery_omitted {
            item.as_table_mut()
                .expect("a named consumer belongs to a table")
                .insert(
                    "delivery".into(),
                    toml::Value::Array(vec![toml::Value::String(derived.as_str().into())]),
                );
        }
        let secret: SecretRef = item.try_into().map_err(|error| {
            let (name, fix) = if original.get("id").and_then(toml::Value::as_str).is_none() {
                ("id", "id = \"credential-id\"".into())
            } else if original.get("delivery").is_some_and(|delivery| {
                delivery.as_array().is_none_or(|modes| {
                    modes
                        .iter()
                        .any(|mode| mode.as_str().and_then(DeliveryMode::parse).is_none())
                })
            }) {
                ("delivery", format!("delivery = [\"{}\"]", derived.as_str()))
            } else if original.get("ttl").is_some_and(|ttl| !ttl.is_str()) {
                ("ttl", "ttl = \"1h\"".into())
            } else if original
                .get("subject")
                .is_some_and(|subject| !subject.is_str())
            {
                ("subject", "subject = \"command-name\"".into())
            } else {
                (
                    "scope",
                    "scope = { path_scope = [\"/required/path/\"] }".into(),
                )
            };
            field_error(
                id,
                field(name),
                fix,
                &format!(
                    "invalid credential reference: {}",
                    without_controls(error.message())
                ),
            )
        })?;
        validate_secret_ref(id, section, index, &secret, delivery_omitted)?;
        parsed.push(secret);
    }
    Ok(parsed)
}

fn normalize_scope(item: &mut toml::Value) {
    if let Some(table) = item.as_table_mut()
        && table.get("scope").is_some_and(toml::Value::is_array)
    {
        let scope = table
            .remove("scope")
            .expect("the scope array was just found");
        table.insert(
            "scope".into(),
            toml::Value::Table(toml::map::Map::from_iter([("path_scope".into(), scope)])),
        );
    }
}

fn parse_commands(plasmid_id: &str, raw: &toml::Value) -> Result<CommandsSpec, ManifestError> {
    if !raw.is_table() {
        return Err(field_error(
            plasmid_id,
            "commands".into(),
            command_sections(&evident_or(raw, &[], EXAMPLE_COMMAND)),
            &format!("{} is not a table", shown(raw)),
        ));
    }
    let address_plan = raw
        .get("address_plan")
        .and_then(toml::Value::as_str)
        .unwrap_or("10.29.0.0/24")
        .to_string();
    let table = match raw.get("commands") {
        Some(toml::Value::Table(table)) => table,
        Some(other) => {
            return Err(field_error(
                plasmid_id,
                "commands.commands".into(),
                command_sections(&evident_or(other, &[], EXAMPLE_COMMAND)),
                &format!("{} is not a table", shown(other)),
            ));
        }
        None => {
            return Err(field_error(
                plasmid_id,
                "commands.commands".into(),
                command_sections(&[EXAMPLE_COMMAND.into()]),
                "[commands] holds no commands table",
            ));
        }
    };
    let mut commands = Vec::new();
    for (id, decl) in table {
        let path = format!("commands.commands.{}", diagnostic_key(id));
        if !decl.is_table() {
            return Err(field_error(
                plasmid_id,
                path.clone(),
                format!(
                    "[{path}]\nexec = [{}]",
                    toml_string(&program(Some(decl), id))
                ),
                &format!("{} is not a table", shown(decl)),
            ));
        }
        let declared = decl.get("exec");
        let exec = string_list(
            plasmid_id,
            &format!("{path}.exec"),
            declared,
            &format!("exec = [{}]", toml_string(&program(declared, id))),
        )?;
        if exec.is_empty() {
            let detail = match declared {
                None => "the command declares no exec".to_string(),
                Some(value) if value.is_array() => {
                    format!("{} names no program to run", shown(value))
                }
                Some(value) => format!("{} is not a list of strings", shown(value)),
            };
            return Err(field_error(
                plasmid_id,
                format!("{path}.exec"),
                format!("exec = [{}]", toml_string(&program(declared, id))),
                &detail,
            ));
        }
        commands.push(CommandDecl {
            id: id.clone(),
            exec,
            subject: decl
                .get("subject")
                .and_then(toml::Value::as_str)
                .map(String::from),
            network: decl
                .get("network")
                .map(|n| parse_network(plasmid_id, &format!("{path}.network"), n, &[]))
                .transpose()?,
            secrets: decl
                .get("secrets")
                .map(|secrets| parse_secret_refs(plasmid_id, &format!("{path}.secrets"), secrets))
                .transpose()?
                .unwrap_or_default(),
        });
    }
    Ok(CommandsSpec {
        address_plan,
        commands,
    })
}

fn narrowest_delivery(consumer: SecretConsumer, path_scoped: bool) -> DeliveryMode {
    match consumer {
        SecretConsumer::Wasm => DeliveryMode::Handle,
        SecretConsumer::Git => DeliveryMode::Helper,
        SecretConsumer::Http | SecretConsumer::Process if path_scoped => DeliveryMode::Inject,
        SecretConsumer::Http | SecretConsumer::Process => DeliveryMode::Mint,
    }
}

fn validate_secret_ref(
    id: &str,
    section: &str,
    index: usize,
    secret: &SecretRef,
    delivery_omitted: bool,
) -> Result<(), ManifestError> {
    let field = |name: &str| format!("{section}.refs[{index}].{name}");
    let narrowest = narrowest_delivery(
        secret.consumer,
        secret
            .scope
            .as_ref()
            .is_some_and(|scope| !scope.path_scope.is_empty()),
    )
    .as_str();
    if secret.delivery.is_empty() {
        return Err(field_error(
            id,
            field("delivery"),
            format!("delivery = [\"{narrowest}\"]"),
            "an explicit delivery list must not be empty",
        ));
    }
    let paths = secret
        .scope
        .as_ref()
        .map(|scope| &scope.path_scope[..])
        .unwrap_or_default();
    let relative = paths.iter().any(|path| !path.starts_with('/'));
    for mode in &secret.delivery {
        let consumer_ok = matches!(
            (secret.consumer, mode),
            (SecretConsumer::Wasm, DeliveryMode::Handle)
                | (SecretConsumer::Git, DeliveryMode::Helper)
                | (SecretConsumer::Git, DeliveryMode::Mint)
                | (
                    SecretConsumer::Http | SecretConsumer::Process,
                    DeliveryMode::Inject | DeliveryMode::Mint
                )
        );
        if !consumer_ok {
            return Err(field_error(
                id,
                field("delivery"),
                format!("delivery = [\"{narrowest}\"]"),
                &format!(
                    "delivery `{}` is not valid for consumer `{}`",
                    mode.as_str(),
                    secret.consumer.as_str()
                ),
            ));
        }
        if (*mode == DeliveryMode::Inject || delivery_omitted)
            && ((*mode == DeliveryMode::Inject && paths.is_empty()) || relative)
        {
            return Err(field_error(
                id,
                field("scope.path_scope"),
                "scope = { path_scope = [\"/required/path/\"] }".into(),
                "path-scope entries must be absolute; inject also requires a nonempty scope",
            ));
        }
    }
    Ok(())
}

fn validate_mock(
    id: &str,
    mock: &MockSpec,
    network: Option<&CommandNetwork>,
) -> Result<(), ManifestError> {
    let Some(network) = network else {
        let detail = if mock.hosts.is_empty() {
            "[mock] names no host, and there is no [network]".to_string()
        } else {
            format!(
                "[mock] stands in for {}, and there is no [network]",
                hosts_named(&distinct(&mock.hosts))
            )
        };
        return Err(field_error(id, "mock".into(), REMOVE_ENTRY.into(), &detail));
    };
    let declared: HashSet<&str> = network.hosts.iter().map(String::as_str).collect();
    let Some((index, host)) = mock
        .hosts
        .iter()
        .enumerate()
        .find(|(_, host)| !declared.contains(host.as_str()))
    else {
        return Ok(());
    };
    if network.ports.is_empty() {
        let undeclared: Vec<String> = distinct(&mock.hosts)
            .into_iter()
            .filter(|host| !declared.contains(host.as_str()))
            .collect();
        return Err(field_error(
            id,
            "mock".into(),
            REMOVE_ENTRY.into(),
            &format!(
                "[mock] stands in for {}, which [network] does not declare, \
                 and [network] declares no port",
                hosts_named(&undeclared)
            ),
        ));
    }
    if !is_proxy_host(host) {
        let lower = host.to_ascii_lowercase();
        let fix = if is_proxy_host(&lower) && !mock.hosts.contains(&lower) {
            toml_string(&lower)
        } else {
            REMOVE_ENTRY.into()
        };
        return Err(field_error(
            id,
            format!("mock.hosts[{index}]"),
            fix,
            &format!(
                "[mock] stands in for {}, which is not a lower-case DNS name",
                toml_string(host)
            ),
        ));
    }
    let (fix, written) = if network.hosts.is_empty() {
        let hosts = distinct(&mock.hosts);
        (strings_line("hosts", &hosts), hosts)
    } else {
        (toml_string(host), vec![host.clone()])
    };
    Err(field_error(
        id,
        "network.hosts".into(),
        fix,
        &format!(
            "[mock] stands in for {}, which [network] does not declare",
            hosts_named(&written)
        ),
    ))
}

fn hosts_named(hosts: &[String]) -> String {
    let quoted: Vec<String> = hosts.iter().map(|host| toml_string(host)).collect();
    match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => quoted.concat(),
    }
}

fn validate_commands(id: &str, commands: &CommandsSpec) -> Result<(), ManifestError> {
    for decl in &commands.commands {
        let command_subject_blank = decl
            .subject
            .as_deref()
            .is_none_or(|subject| subject.trim().is_empty());
        for (index, secret) in decl.secrets.iter().enumerate() {
            if secret
                .subject
                .as_deref()
                .is_none_or(|subject| subject.trim().is_empty())
                && command_subject_blank
            {
                return Err(field_error(
                    id,
                    format!(
                        "commands.commands.{}.secrets.refs[{index}].subject",
                        diagnostic_key(&decl.id)
                    ),
                    format!("subject = {}", toml_string(&program(None, &decl.id))),
                    "a command credential requires a nonblank ref or command subject",
                ));
            }
        }
    }
    Ok(())
}

const EXAMPLE_HOST: &str = "api.example.com";

const EXAMPLE_PIN: &str = "192.0.2.0/24";

const EXAMPLE_COMMAND: &str = "git";

fn strings_line(field: &str, entries: &[String]) -> String {
    let entries: Vec<String> = entries.iter().map(|entry| toml_string(entry)).collect();
    format!("{field} = [{}]", entries.join(", "))
}

fn evident_strings(value: &toml::Value) -> Vec<String> {
    let nonblank = |item: &toml::Value| {
        item.as_str()
            .filter(|text| !text.trim().is_empty())
            .map(String::from)
    };
    match value {
        toml::Value::Array(items) => items
            .iter()
            .map(nonblank)
            .collect::<Option<Vec<String>>>()
            .map(|names| distinct(&names))
            .unwrap_or_default(),
        value => nonblank(value).into_iter().collect(),
    }
}

fn distinct(names: &[String]) -> Vec<String> {
    let mut seen = HashSet::with_capacity(names.len());
    names
        .iter()
        .filter(|name| seen.insert(name.as_str()))
        .cloned()
        .collect()
}

fn evident_or(value: &toml::Value, fallback: &[String], example: &str) -> Vec<String> {
    let evident = evident_strings(value);
    if !evident.is_empty() {
        evident
    } else if !fallback.is_empty() {
        distinct(fallback)
    } else {
        vec![example.to_string()]
    }
}

fn command_sections(names: &[String]) -> String {
    let sections: Vec<String> = names
        .iter()
        .map(|name| {
            format!(
                "[commands.commands.{}]\nexec = [{}]",
                diagnostic_key(name),
                toml_string(name)
            )
        })
        .collect();
    sections.join("\n")
}

fn program(evident: Option<&toml::Value>, key: &str) -> String {
    evident
        .and_then(toml::Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .or(Some(key).filter(|key| !key.trim().is_empty()))
        .unwrap_or("command-name")
        .to_string()
}

#[derive(Clone, Copy)]
enum ListRepair<'a> {
    Hosts(&'a [String]),
    Pins,
    MockHosts,
}

impl ListRepair<'_> {
    fn field(self) -> &'static str {
        match self {
            ListRepair::Hosts(_) | ListRepair::MockHosts => "hosts",
            ListRepair::Pins => "pin_cidrs",
        }
    }

    fn whole(self, value: &toml::Value) -> String {
        match self {
            ListRepair::Hosts(fallback) => {
                strings_line("hosts", &evident_or(value, fallback, EXAMPLE_HOST))
            }
            ListRepair::Pins => strings_line("pin_cidrs", &evident_or(value, &[], EXAMPLE_PIN)),
            ListRepair::MockHosts => match evident_strings(value) {
                hosts if hosts.is_empty() => REMOVE_ENTRY.into(),
                hosts => strings_line("hosts", &hosts),
            },
        }
    }

    fn entry(self, items: &[toml::Value]) -> String {
        match self {
            ListRepair::Pins if !items.iter().any(toml::Value::is_str) => toml_string(EXAMPLE_PIN),
            _ => REMOVE_ENTRY.into(),
        }
    }
}

fn declared_string_list(
    id: &str,
    section: &str,
    value: Option<&toml::Value>,
    repair: ListRepair,
) -> Result<Vec<String>, ManifestError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let path = format!("{section}.{}", repair.field());
    let Some(items) = value.as_array() else {
        return Err(field_error(
            id,
            path,
            repair.whole(value),
            &format!("{} is not a list of strings", shown(value)),
        ));
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_str().map(String::from).ok_or_else(|| {
                field_error(
                    id,
                    format!("{path}[{index}]"),
                    repair.entry(items),
                    &format!("{} is not a string", shown(item)),
                )
            })
        })
        .collect()
}

const REMOVE_ENTRY: &str = "remove this entry";

const BLANK_TOOL: &str = "a tool name must not be empty or only whitespace";

fn declared_ports(
    id: &str,
    field: &str,
    value: Option<&toml::Value>,
) -> Result<Vec<u16>, ManifestError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        let fix = evident_port(value).map_or_else(
            || REMOVE_ENTRY.to_string(),
            |port| format!("ports = [{port}]"),
        );
        return Err(field_error(
            id,
            field.to_string(),
            fix,
            &format!("{} is not a list of integers from 1 to 65535", shown(value)),
        ));
    };
    let ports: Vec<Option<u16>> = items
        .iter()
        .map(|item| item.as_integer().and_then(nonzero_port))
        .collect();
    let Some(index) = ports.iter().position(Option::is_none) else {
        return Ok(ports.into_iter().flatten().collect());
    };
    let item = &items[index];
    let fix = evident_port(item)
        .filter(|port| !ports.contains(&Some(*port)))
        .map_or_else(|| REMOVE_ENTRY.to_string(), |port| port.to_string());
    Err(field_error(
        id,
        format!("{field}[{index}]"),
        fix,
        &format!("{} is not an integer from 1 to 65535", shown(item)),
    ))
}

fn shown(value: &toml::Value) -> String {
    match value {
        toml::Value::String(text) => toml_string(text),
        toml::Value::Float(number) if number.is_nan() => "nan".into(),
        toml::Value::Float(number) => format!("{number:?}"),
        toml::Value::Datetime(datetime) => datetime.to_string(),
        toml::Value::Array(items) => {
            let items: Vec<String> = items.iter().map(shown).collect();
            format!("[{}]", items.join(", "))
        }
        toml::Value::Table(table) if table.is_empty() => "{}".into(),
        toml::Value::Table(table) => {
            let entries: Vec<String> = table
                .iter()
                .map(|(key, value)| format!("{} = {}", diagnostic_key(key), shown(value)))
                .collect();
            format!("{{ {} }}", entries.join(", "))
        }
        toml::Value::Integer(_) | toml::Value::Boolean(_) => value.to_string(),
    }
}

fn evident_port(value: &toml::Value) -> Option<u16> {
    let number = match value {
        toml::Value::Integer(number) => *number,
        toml::Value::String(text) => text.trim().parse().ok()?,
        toml::Value::Float(number) if number.fract() == 0.0 => *number as i64,
        _ => return None,
    };
    nonzero_port(number)
}

fn nonzero_port(number: i64) -> Option<u16> {
    u16::try_from(number).ok().filter(|port| *port != 0)
}

fn string_list(
    id: &str,
    path: &str,
    value: Option<&toml::Value>,
    fix: &str,
) -> Result<Vec<String>, ManifestError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value.as_array().ok_or_else(|| {
        field_error(
            id,
            path.into(),
            fix.into(),
            &format!("{} is not a list of strings", shown(value)),
        )
    })?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_str()
                .filter(|text| !text.trim().is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    field_error(
                        id,
                        format!("{path}[{index}]"),
                        REMOVE_ENTRY.into(),
                        "expected a nonblank string",
                    )
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::*;

    const PROVIDER: &str =
        "id = \"github-pr\"\ndescription = \"Read pull requests.\"\nimpl.wasm = \"pr.wasm\"\n";

    fn field_fix(source: &str, expected_field: &str) -> String {
        let ManifestError::Field {
            plasmid,
            field,
            fix,
            ..
        } = PlasmidManifest::parse(source).unwrap_err()
        else {
            panic!("expected a structured declaration refusal");
        };
        assert_eq!(plasmid.as_deref(), Some("github-pr"));
        let actual_path: toml::Value = format!("{field} = true").parse().unwrap();
        let expected_path: toml::Value = format!("{expected_field} = true").parse().unwrap();
        assert_eq!(actual_path, expected_path);
        fix
    }

    #[test]
    fn missing_or_invalid_purpose_has_an_actionable_field_refusal() {
        for purpose in ["", "description = 42", "description = \" \\t \""] {
            let source = format!("id = \"github-pr\"\n{purpose}\nimpl.wasm = \"pr.wasm\"");
            let fix = field_fix(&source, "description");
            let repaired: toml::Value = fix.parse().unwrap();
            assert!(!repaired["description"].as_str().unwrap().trim().is_empty());
        }
        let error = PlasmidManifest::parse("id = 42").unwrap_err();
        assert!(matches!(
            error,
            ManifestError::Field { plasmid: None, field, .. } if field == "id"
        ));
    }

    #[test]
    fn malformed_tool_declarations_are_refused_instead_of_dropped() {
        for (declaration, field) in [
            ("provides = false", "provides"),
            (
                "[provides]\n\"github:tools\" = false",
                "provides.\"github:tools\"",
            ),
            (
                "[provides.\"github:tools\"]",
                "provides.\"github:tools\".tools",
            ),
            (
                "[provides.\"github:tools\"]\ntools = false",
                "provides.\"github:tools\".tools",
            ),
            (
                "[provides.\"github:tools\"]\ntools = []",
                "provides.\"github:tools\".tools",
            ),
        ] {
            let fix = field_fix(&format!("{PROVIDER}{declaration}"), field);
            let repair: toml::Value = fix.parse().unwrap();
            assert!(repair["provides"].is_table());
        }
        let fix = field_fix(
            &format!("{PROVIDER}[provides.\"github:tools\"]\ntools = [\"pr.read\"]"),
            "provides.\"github:tools\".tools",
        );
        let repair: toml::Value = fix.parse().unwrap();
        assert!(repair["provides"]["github:tools"]["tools"]["pr.read"].is_str());
    }

    #[test]
    fn tool_description_refusals_quote_the_exact_key_and_repair_entry() {
        for description in ["42", "\"\"", "\" \\t \""] {
            let source = format!(
                "{PROVIDER}[provides.\"git\\\"hub:tools\".tools]\n\"pr.\\\"read\" = {description}"
            );
            let fix = field_fix(
                &source,
                "provides.\"git\\\"hub:tools\".tools.\"pr.\\\"read\"",
            );
            let repair: toml::Value = fix.parse().unwrap();
            assert_eq!(repair.as_table().unwrap().len(), 1);
            assert!(!repair["pr.\"read"].as_str().unwrap().trim().is_empty());
        }
    }

    #[test]
    fn diagnostic_paths_and_repairs_preserve_special_binding_and_tool_keys() {
        for (name, quoted) in [
            ("line\nname", "\"line\\nname\""),
            ("both'\"quotes", "\"both'\\\"quotes\""),
            ("tab\tname", "\"tab\\tname\""),
            ("back\\slash", "\"back\\\\slash\""),
        ] {
            for (declaration, expected_path, replacement) in [
                (
                    format!("[provides]\n{quoted} = false"),
                    format!("provides.{quoted}"),
                    "{ tools = {} }",
                ),
                (
                    format!("[provides.{quoted}]\ntools = [{quoted}]"),
                    format!("provides.{quoted}.tools"),
                    "{}",
                ),
                (
                    format!("[provides.{quoted}.tools]\n{quoted} = 42"),
                    format!("provides.{quoted}.tools.{quoted}"),
                    "\"Read the title.\"",
                ),
            ] {
                let source = format!("{PROVIDER}{declaration}");
                let ManifestError::Field {
                    plasmid,
                    field,
                    fix,
                    ..
                } = PlasmidManifest::parse(&source).unwrap_err()
                else {
                    panic!("expected a structured declaration refusal for {name:?}");
                };
                assert_eq!(plasmid.as_deref(), Some("github-pr"));
                let actual_path: toml::Value = format!("{field} = true").parse().unwrap();
                let expected_path: toml::Value = format!("{expected_path} = true").parse().unwrap();
                assert_eq!(actual_path, expected_path);
                let repaired_path =
                    PlasmidManifest::parse(&format!("{PROVIDER}{field} = {replacement}")).unwrap();
                let repair: toml::Value = fix.parse().unwrap();
                if declaration.ends_with("false") {
                    let bindings = repair["provides"].as_table().unwrap();
                    assert_eq!(bindings.len(), 1);
                    assert!(bindings[name].as_table().unwrap().is_empty());
                    assert!(repaired_path.provides_tools.is_empty());
                } else {
                    let repaired_source = if declaration.ends_with("42") {
                        assert_eq!(repair.as_table().unwrap().len(), 1);
                        assert!(!repair[name].as_str().unwrap().trim().is_empty());
                        assert_eq!(
                            repaired_path.provides_tools,
                            vec![ToolDeclaration {
                                name: name.into(),
                                description: "Read the title.".into(),
                            }]
                        );
                        format!("{PROVIDER}[provides.{quoted}.tools]\n{fix}")
                    } else {
                        assert_eq!(repair["provides"].as_table().unwrap().len(), 1);
                        assert_eq!(
                            repair["provides"][name]["tools"].as_table().unwrap().len(),
                            1
                        );
                        assert!(
                            !repair["provides"][name]["tools"][name]
                                .as_str()
                                .unwrap()
                                .trim()
                                .is_empty()
                        );
                        assert!(repaired_path.provides_tools.is_empty());
                        format!("{PROVIDER}{fix}")
                    };
                    let repaired = PlasmidManifest::parse(&repaired_source).unwrap();
                    assert_eq!(repaired.provides_tools.len(), 1);
                    assert_eq!(repaired.provides_tools[0].name, name);
                    assert!(!repaired.provides_tools[0].description.trim().is_empty());
                }
            }
        }
    }

    #[test]
    fn an_empty_tool_key_is_quoted_in_its_path() {
        let source = format!("{PROVIDER}[provides.\"a:tools\".tools]\n\"\" = 42");
        assert_eq!(
            field_fix(&source, "provides.\"a:tools\".tools.\"\""),
            REMOVAL
        );
    }

    #[test]
    fn blank_binding_keys_are_refused_one_at_a_time() {
        let mut declaration = format!(
            "{PROVIDER}[provides.\"a:tools\".tools]\n\"a.read\" = \"Read a.\"\n\
             [provides.\"\".tools]\n\"b.read\" = \"Read b.\"\n\
             [provides.\" \".tools]\n\"c.read\" = \"Read c.\"\n\
             [provides.\"\\t\".tools]\n\"d.read\" = \"Read d.\"\n\
             [provides.\"\u{3000}\".tools]\n\"e.read\" = \"Read e.\"\n"
        );
        let blank = [
            String::new(),
            " ".to_string(),
            raw(0x09).to_string(),
            "\u{3000}".to_string(),
        ];
        let fixture: toml::Table = toml::from_str(&declaration).unwrap();
        let bindings: std::collections::BTreeSet<String> = fixture["provides"]
            .as_table()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let expected = blank.iter().cloned().chain(["a:tools".to_string()]);
        assert_eq!(bindings, expected.collect());
        let mut refused: Vec<String> = Vec::new();
        for _ in 0..5 {
            if PlasmidManifest::parse(&declaration).is_ok() {
                break;
            }
            let (plasmid, field, fix) = field_refusal(&declaration);
            assert_eq!(plasmid.as_deref(), Some("github-pr"), "{field}");
            assert_eq!(fix, REMOVAL, "{field}");
            let (keys, index) = field_keys(&field);
            assert_eq!(index, None, "{field}");
            let [section, key] = keys.as_slice() else {
                panic!("{field} does not name a binding");
            };
            assert_eq!(section, "provides", "{field}");
            assert!(
                blank.contains(key) && !refused.contains(key),
                "{field} is not a blank binding refused for the first time"
            );
            refused.push(key.clone());
            declaration = with_fix(&declaration, &field, &fix);
        }
        assert_eq!(refused.len(), 4, "{refused:?}");
        assert_eq!(
            PlasmidManifest::parse(&declaration).unwrap().provides_tools,
            vec![ToolDeclaration {
                name: "a.read".into(),
                description: "Read a.".into(),
            }]
        );
    }

    #[test]
    fn a_blank_binding_key_is_refused_before_anything_inside_it() {
        let ideographic = format!("\"{}\"", concat!("\\", "u3000"));
        let mixed = format!(
            "\"{}{}{}\"",
            concat!("\\", "u00A0"),
            concat!("\\", "u0085"),
            concat!("\\", "u2028")
        );
        for (key, decoded) in [
            ("\"\"".to_string(), String::new()),
            ("\" \"".to_string(), " ".to_string()),
            ("\"\\t\"".to_string(), raw(0x09).to_string()),
            (ideographic, raw(0x3000).to_string()),
            (mixed, "\u{a0}\u{85}\u{2028}".to_string()),
        ] {
            for tail in [
                format!("[provides]\n{key} = false"),
                format!("[provides]\n{key} = {{}}"),
                format!("[provides.{key}]\ntool = {{ \"x.read\" = \"Read x.\" }}"),
                format!("[provides]\n{key} = {{ tools = [\"x.read\"] }}"),
                format!("[provides]\n{key} = {{ tools = {{}} }}"),
                format!("[provides]\n{key} = {{ tools = {{ \"x.read\" = 42 }} }}"),
            ] {
                let source = format!("{PROVIDER}{tail}");
                let fixture: toml::Table = toml::from_str(&source).unwrap();
                let bindings: Vec<&String> =
                    fixture["provides"].as_table().unwrap().keys().collect();
                assert_eq!(bindings, [&decoded], "{tail}");
                assert_eq!(
                    field_fix(&source, &format!("provides.{key}")),
                    REMOVAL,
                    "{tail}"
                );
            }
        }
    }

    #[test]
    fn a_key_that_is_not_only_whitespace_parses_with_its_tools() {
        let keys = [
            " a",
            "a ",
            "\u{3000}a",
            "\ta",
            "a b",
            "\u{feff}",
            "\u{200b}",
            "\u{180e}",
        ];
        let mut declaration = PROVIDER.to_string();
        for (index, key) in keys.iter().enumerate() {
            declaration.push_str(&format!(
                "[provides.\"{key}\".tools]\n\"t{index}.read\" = \"Read {index}.\"\n"
            ));
        }
        let fixture: toml::Table = toml::from_str(&declaration).unwrap();
        let bindings: std::collections::BTreeSet<&str> = fixture["provides"]
            .as_table()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(bindings, keys.into_iter().collect());
        let mut tools = PlasmidManifest::parse(&declaration).unwrap().provides_tools;
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        let expected: Vec<ToolDeclaration> = (0..keys.len())
            .map(|index| ToolDeclaration {
                name: format!("t{index}.read"),
                description: format!("Read {index}."),
            })
            .collect();
        assert_eq!(tools, expected);
    }

    const WHITE_SPACE: [u32; 25] = [
        0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20, 0x85, 0xA0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003,
        0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200A, 0x2028, 0x2029, 0x202F, 0x205F,
        0x3000,
    ];

    fn blank_tool_keys() -> [(String, String); 7] {
        [
            ("\"\"".to_string(), String::new()),
            ("\" \"".to_string(), " ".to_string()),
            ("\"\\t\"".to_string(), raw(0x09).to_string()),
            (
                format!("\"{}\"", concat!("\\", "u3000")),
                raw(0x3000).to_string(),
            ),
            (
                format!(
                    "\"{}{}{LINE_SEPARATOR}\"",
                    concat!("\\", "u00A0"),
                    concat!("\\", "u0085")
                ),
                [0xA0, 0x85, 0x2028].map(raw).iter().collect(),
            ),
            (
                format!(
                    "\"{}{}{}\"",
                    concat!("\\", "u1680"),
                    concat!("\\", "u2029"),
                    concat!("\\", "u205F")
                ),
                [0x1680, 0x2029, 0x205F].map(raw).iter().collect(),
            ),
            (
                format!(
                    "\"{}\"",
                    WHITE_SPACE.map(|code| format!("\\u{code:04X}")).concat()
                ),
                WHITE_SPACE.map(raw).iter().collect(),
            ),
        ]
    }

    fn tool_names(source: &str) -> Vec<String> {
        let fixture: toml::Table = toml::from_str(source).unwrap();
        let tools = fixture["provides"]["a:tools"]["tools"].as_table().unwrap();
        let mut names: Vec<String> = tools.keys().cloned().collect();
        names.sort();
        names
    }

    #[test]
    fn blank_tool_names_are_refused_one_at_a_time() {
        let keys = blank_tool_keys();
        let mut declaration =
            format!("{PROVIDER}[provides.\"a:tools\".tools]\n\"a.read\" = \"Read a.\"\n");
        for (index, (quoted, _)) in keys.iter().enumerate() {
            declaration.push_str(&format!("{quoted} = \"Read {index}.\"\n"));
        }
        let blank: Vec<String> = keys.into_iter().map(|(_, decoded)| decoded).collect();
        let mut expected = blank.clone();
        expected.push("a.read".into());
        expected.sort();
        assert_eq!(tool_names(&declaration), expected);
        let mut refused: Vec<String> = Vec::new();
        while PlasmidManifest::parse(&declaration).is_err() {
            let (plasmid, field, fix) = field_refusal(&declaration);
            assert_eq!(plasmid.as_deref(), Some("github-pr"), "{field}");
            assert_eq!(fix, REMOVAL, "{field}");
            let (keys, index) = field_keys(&field);
            assert_eq!(index, None, "{field}");
            let [provides, binding, tools, name] = keys.as_slice() else {
                panic!("{field} does not name a tool");
            };
            assert_eq!(
                [provides, binding, tools],
                ["provides", "a:tools", "tools"],
                "{field}"
            );
            assert!(
                blank.contains(name) && !refused.contains(name),
                "{field} is not a blank tool refused for the first time"
            );
            refused.push(name.clone());
            declaration = with_fix(&declaration, &field, &fix);
        }
        assert_eq!(refused.len(), blank.len(), "{refused:?}");
        assert_eq!(
            PlasmidManifest::parse(&declaration).unwrap().provides_tools,
            vec![ToolDeclaration {
                name: "a.read".into(),
                description: "Read a.".into(),
            }]
        );
    }

    #[test]
    fn a_blank_tool_name_is_refused_before_its_description() {
        for (quoted, decoded) in blank_tool_keys() {
            for value in ["42", "false", "{}", "\"\"", "\" \""] {
                let source = format!("{PROVIDER}[provides.\"a:tools\".tools]\n{quoted} = {value}");
                assert_eq!(
                    tool_names(&source),
                    std::slice::from_ref(&decoded),
                    "{source}"
                );
                let path = format!("provides.\"a:tools\".tools.{quoted}");
                assert_eq!(field_fix(&source, &path), REMOVAL, "{source}");
            }
        }
    }

    #[test]
    fn a_blank_binding_is_refused_before_its_blank_tool() {
        for (binding, _) in blank_tool_keys() {
            for (tool, _) in blank_tool_keys() {
                let source = format!("{PROVIDER}[provides.{binding}.tools]\n{tool} = \"Read.\"");
                let path = format!("provides.{binding}");
                assert_eq!(field_fix(&source, &path), REMOVAL, "{source}");
            }
        }
    }

    #[test]
    fn a_blank_name_in_a_tools_list_is_refused_at_its_entry() {
        for (quoted, _) in blank_tool_keys() {
            for (list, expected) in [
                (format!("[{quoted}]"), vec![]),
                (
                    format!("[{quoted}, \"a.read\"]"),
                    vec![ToolDeclaration {
                        name: "a.read".into(),
                        description: "Describe what this tool does.".into(),
                    }],
                ),
            ] {
                let declaration = format!("{PROVIDER}[provides.\"a:tools\"]\ntools = {list}\n");
                let (plasmid, field, fix) = field_refusal(&declaration);
                assert_eq!(plasmid.as_deref(), Some("github-pr"), "{list}");
                assert_eq!(fix, REMOVAL, "{list}");
                let (keys, index) = field_keys(&field);
                assert_eq!(keys, ["provides", "a:tools", "tools"], "{list}");
                assert_eq!(index, Some(0), "{list}");
                let declaration = with_fix(&declaration, &field, &fix);
                let (_, field, fix) = field_refusal(&declaration);
                let repaired = PlasmidManifest::parse(&with_fix(&declaration, &field, &fix));
                assert_eq!(repaired.unwrap().provides_tools, expected, "{list}");
            }
        }
    }

    #[test]
    fn a_blank_name_after_the_first_in_a_tools_list_goes_with_the_lists_fix() {
        for (quoted, _) in blank_tool_keys() {
            for list in [
                format!("[\"a.read\", {quoted}]"),
                format!("[\"a.read\", {quoted}, \"b.read\"]"),
            ] {
                let declaration = format!("{PROVIDER}[provides.\"a:tools\"]\ntools = {list}\n");
                let (plasmid, field, fix) = field_refusal(&declaration);
                assert_eq!(plasmid.as_deref(), Some("github-pr"), "{list}");
                let (keys, index) = field_keys(&field);
                assert_eq!(keys, ["provides", "a:tools", "tools"], "{list}");
                assert_eq!(index, None, "{list}");
                let repair: toml::Table = fix.parse().unwrap();
                let tools = repair["provides"]["a:tools"]["tools"].as_table().unwrap();
                assert_eq!(tools.keys().collect::<Vec<_>>(), ["a.read"], "{list}");
            }
        }
    }

    #[test]
    fn a_tool_name_that_is_not_only_whitespace_parses() {
        let names = [
            " a".to_string(),
            "a ".to_string(),
            format!("{}a", raw(0x3000)),
            raw(0xFEFF).to_string(),
            raw(0x200B).to_string(),
            raw(0x180E).to_string(),
        ];
        let mut declaration = format!("{PROVIDER}[provides.\"a:tools\".tools]\n");
        for name in &names {
            declaration.push_str(&format!("\"{name}\" = \"Read.\"\n"));
        }
        let mut expected = names.to_vec();
        expected.sort();
        assert_eq!(tool_names(&declaration), expected);
        let tools = PlasmidManifest::parse(&declaration).unwrap().provides_tools;
        let mut parsed: Vec<String> = tools.into_iter().map(|tool| tool.name).collect();
        parsed.sort();
        assert_eq!(parsed, expected);
    }

    #[test]
    fn equivalent_tables_and_provenance_comments_preserve_author_descriptions() {
        let base = "id = \"github-pr\"\ndescription = \" Read pull requests. \"\nimpl.wasm = \"pr.wasm\"\n";
        let nested =
            format!("{base}[provides.\"github:tools\".tools]\n\"pr.read\" = \" Read the title. \"");
        let inline = format!(
            "{base}# Generated from an author's request.\n[provides]\n\"github:tools\" = {{ tools = {{ \"pr.read\" = \" Read the title. \" }} }}"
        );
        let manifest = PlasmidManifest::parse(&nested).unwrap();
        assert_eq!(manifest, PlasmidManifest::parse(&inline).unwrap());
        assert_eq!(manifest.description, " Read pull requests. ");
        assert_eq!(
            manifest.provides_tools,
            vec![ToolDeclaration {
                name: "pr.read".into(),
                description: " Read the title. ".into(),
            }]
        );
    }

    #[test]
    fn absent_or_empty_tools_do_not_invent_a_tool() {
        let base = "id = \"github-pr\"\ndescription = \"Reach GitHub.\"\n[network]\nhosts = [\"api.github.com\"]\nports = [443]\n";
        for provides in ["", "[provides.\"github:tools\".tools]"] {
            let manifest = PlasmidManifest::parse(&format!("{base}{provides}")).unwrap();
            assert!(manifest.provides_tools.is_empty());
            assert!(manifest.declares_any_host("api.github.com"));
        }
    }

    const GITHUB_PR: &str = r#"
id = "github-pr"
description = "Read and comment on pull requests."
version = "0.1.0"
impl.wasm = "components/github-pr.wasm"

[requires]
capabilities = ["network:hosts=api.github.com"]

[provides]
"github:tools" = { tools = { "pr.read" = "Read a pull request.", "pr.comment" = "Post a comment on a pull request." } }

[network]
hosts = ["api.github.com"]
ports = [443]
pin_cidrs = ["140.82.112.0/20"]

[lifecycle]
drain_ms = 750
"#;

    const GITHUB_PR_LEGACY_STRING_REFS: &str = r#"
id = "github-pr"
description = "Read and comment on pull requests."
version = "0.1.0"
impl.wasm = "components/github-pr.wasm"

[network]
hosts = ["api.github.com"]
ports = [443]

[secrets]
refs = ["github-pr/token"]
"#;

    const GITHUB_PR_FROZEN: &str = r#"
id = "github-pr"
description = "Read and comment on pull requests."
version = "0.2.0"
impl.wasm = "components/github-pr.wasm"

[network]
hosts = ["api.github.com", "github.com"]
ports = [443]

[[secrets.refs]]
id = "github:token:pr"
consumer = "git"
delivery = ["helper", "mint"]
scope = { repos = ["acme/widgets"], permissions = ["contents:write", "pull_requests:write"] }
ttl = "1h"

[[secrets.refs]]
id = "github:token:api"
consumer = "process"
delivery = ["inject"]
scope = ["/repos/", "/repos/*/pulls/"]
"#;

    const MODEL_PROVIDER: &str = r#"
id = "model-provider"
description = "Complete prompts through the model endpoint."
version = "0.1.0"

[network]
hosts = ["api.openai.com"]
ports = [443]

[secrets]
refs = [
  { id = "model-provider:key", consumer = "wasm", delivery = ["handle"] },
]

[model]
endpoint = "https://api.openai.com/v1/chat/completions"
credential = "model-provider/key"
"#;

    const WORKSPACE: &str = r#"
id = "workspace-bind"
description = "Make the workspace available to the cell."
version = "0.1.0"

[workspace]
mount = { dst = "/workspace" }
"#;

    const GITHUB_PR_WITH_MOCK: &str = r#"
id = "github-pr"
description = "Read and comment on pull requests."
version = "0.1.0"
impl.wasm = "components/github-pr.wasm"

[network]
hosts = ["api.github.com"]
ports = [443]

[mock]
hosts = ["api.github.com"]
api = "github"
backend = { kind = "recorded", source = "fixtures/github-pr" }
"#;

    const MOCK_WITHOUT_NETWORK: &str = r#"
id = "mock-github"
description = "Replay recorded GitHub responses."
version = "0.1.0"

[mock]
hosts = ["api.github.com"]
api = "github"
backend = { kind = "recorded", source = "fixtures/github-pr" }
"#;

    const MOCK_HOSTS_DRIFTED_FROM_NETWORK: &str = r#"
id = "github-pr"
description = "Read and comment on pull requests."
version = "0.1.0"

[network]
hosts = ["api.github.com"]
ports = [443]

[mock]
hosts = ["api.github.example"]
api = "github"
backend = { kind = "recorded", source = "fixtures/github-pr" }
"#;

    const MOCK_HOSTS_AS_SCALAR: &str = r#"
id = "github-pr"
description = "Read and comment on pull requests."
version = "0.1.0"

[network]
hosts = ["api.github.com"]
ports = [443]

[mock]
hosts = "api.github.com"
api = "github"
backend = { kind = "recorded", source = "fixtures/github-pr" }
"#;

    const MOCK_HOSTS_MIXED_TYPES: &str = r#"
id = "github-pr"
description = "Read and comment on pull requests."
version = "0.1.0"

[network]
hosts = ["api.github.com"]
ports = [443]

[mock]
hosts = ["api.github.com", 443]
api = "github"
backend = { kind = "recorded", source = "fixtures/github-pr" }
"#;

    const NETWORK_PIN_CIDRS_AS_SCALAR: &str = r#"
id = "github-pr"
description = "Read and comment on pull requests."
version = "0.1.0"

[network]
hosts = ["api.github.com"]
ports = [443]
pin_cidrs = "140.82.112.0/20"
"#;

    const NETWORK_PIN_CIDRS_MIXED_TYPES: &str = r#"
id = "github-pr"
description = "Read and comment on pull requests."
version = "0.1.0"

[network]
hosts = ["api.github.com"]
ports = [443]
pin_cidrs = ["140.82.112.0/20", 20]
"#;

    const NETWORK_HOSTS_AS_SCALAR: &str = r#"
id = "github-pr"
description = "Read and comment on pull requests."
version = "0.1.0"

[network]
hosts = "api.github.com"
ports = [443]
"#;

    const NETWORK_PIN_CIDRS_EMPTY: &str = r#"
id = "github-pr"
description = "Read and comment on pull requests."
version = "0.1.0"

[network]
hosts = ["api.github.com"]
ports = [443]
pin_cidrs = []
"#;

    const COMMAND_NETWORK_HOSTS_AS_SCALAR: &str = r#"
id = "e13-commands-fixture"
description = "Run git with declared network access."
version = "0.1.0"

[network]
hosts = ["alpha.ak.local"]
ports = [443]

[commands]
address_plan = "10.29.0.0/24"

[commands.commands.git]
exec = ["git"]
subject = "git"

[commands.commands.git.network]
hosts = "alpha.ak.local"
ports = [443]
"#;

    const NETWORK_SECTION_AS_SCALAR: &str = r#"
id = "probe"
description = "Reach the declared network endpoint."
version = "0.1.0"
network = "api.github.com"
"#;

    const COMMAND_NETWORK_SECTION_AS_SCALAR: &str = r#"
id = "e13-commands-fixture"
description = "Run git with declared network access."
version = "0.1.0"

[network]
hosts = ["alpha.ak.local"]
ports = [443]

[commands]
address_plan = "10.29.0.0/24"

[commands.commands.git]
exec = ["git"]
network = "alpha.ak.local"
"#;

    const COMMAND_NETWORK_PIN_CIDRS_AS_SCALAR: &str = r#"
id = "e13-commands-fixture"
description = "Run git with declared network access."
version = "0.1.0"

[network]
hosts = ["alpha.ak.local"]
ports = [443]

[commands]
address_plan = "10.29.0.0/24"

[commands.commands.git]
exec = ["git"]
subject = "git"

[commands.commands.git.network]
hosts = ["alpha.ak.local"]
ports = [443]
pin_cidrs = "10.29.0.0/24"
"#;

    const COMMANDS_E13: &str = r#"
id = "e13-commands-fixture"
description = "Run git with declared network access."
version = "0.1.0"

[network]
hosts = ["alpha.ak.local"]
ports = [443]

[commands]
address_plan = "10.29.0.0/24"

[commands.commands.git]
exec = ["git"]
subject = "git"

[commands.commands.git.network]
hosts = ["alpha.ak.local", "github.ak.local"]
ports = [443]

[[commands.commands.git.secrets.refs]]
id = "github:token:git"
consumer = "http"
delivery = ["inject"]
scope = { path_scope = ["/repos/"] }
subject = "git"
"#;

    #[test]
    fn github_pr_manifest_carries_tools_network_and_drain() {
        let manifest = PlasmidManifest::parse(GITHUB_PR).unwrap();
        assert_eq!(manifest.id, "github-pr");
        assert_eq!(manifest.description, "Read and comment on pull requests.");
        assert_eq!(
            manifest.provides_tools,
            vec![
                ToolDeclaration {
                    name: "pr.comment".into(),
                    description: "Post a comment on a pull request.".into(),
                },
                ToolDeclaration {
                    name: "pr.read".into(),
                    description: "Read a pull request.".into(),
                },
            ]
        );
        assert_eq!(
            manifest.network.as_ref().unwrap().hosts,
            vec!["api.github.com".to_string()]
        );
        assert_eq!(manifest.network.as_ref().unwrap().ports, vec![443]);
        assert_eq!(
            pins(manifest.network.as_ref().unwrap()),
            vec!["140.82.112.0/20".to_string()]
        );
        assert_eq!(manifest.secrets, Vec::new());
        assert_eq!(manifest.drain_ms, Some(750));
        assert!(manifest.wasm.is_some());
    }

    #[test]
    fn the_frozen_secret_grammar_parses_delivery_consumer_scope_and_ttl() {
        let manifest = PlasmidManifest::parse(GITHUB_PR_FROZEN).unwrap();
        assert_eq!(manifest.secrets.len(), 2);
        let git_ref = &manifest.secrets[0];
        assert_eq!(git_ref.id, "github:token:pr");
        assert_eq!(git_ref.consumer, SecretConsumer::Git);
        assert_eq!(
            git_ref.delivery,
            vec![DeliveryMode::Helper, DeliveryMode::Mint]
        );
        assert_eq!(git_ref.ttl.as_deref(), Some("1h"));
        let scope = git_ref.scope.as_ref().unwrap();
        assert_eq!(scope.repos, vec!["acme/widgets".to_string()]);
        assert_eq!(
            scope.permissions,
            vec![
                "contents:write".to_string(),
                "pull_requests:write".to_string()
            ]
        );
        let api_ref = &manifest.secrets[1];
        assert_eq!(api_ref.consumer, SecretConsumer::Process);
        assert_eq!(api_ref.delivery, vec![DeliveryMode::Inject]);
        assert_eq!(
            api_ref.scope.as_ref().unwrap().path_scope,
            vec!["/repos/".to_string(), "/repos/*/pulls/".to_string()]
        );
    }

    #[test]
    fn secret_refs_survive_a_serde_round_trip_in_the_frozen_shape() {
        let manifest = PlasmidManifest::parse(GITHUB_PR_FROZEN).unwrap();
        for secret in &manifest.secrets {
            let json = serde_json::to_string(secret).unwrap();
            let back: SecretRef = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, secret);
        }
        let json = serde_json::to_string(&manifest.secrets[1]).unwrap();
        assert!(json.contains("\"delivery\":[\"inject\"]"), "{json}");
    }

    #[test]
    fn model_provider_manifest_needs_no_wasm() {
        let manifest = PlasmidManifest::parse(MODEL_PROVIDER).unwrap();
        assert!(manifest.wasm.is_none());
        assert_eq!(
            manifest.model.as_ref().unwrap().endpoint,
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            manifest.model.as_ref().unwrap().credential,
            "model-provider/key"
        );
        assert_eq!(manifest.secrets[0].delivery, vec![DeliveryMode::Handle]);
    }

    #[test]
    fn workspace_manifest_parses_the_mount() {
        let manifest = PlasmidManifest::parse(WORKSPACE).unwrap();
        assert_eq!(manifest.workspace.as_ref().unwrap().dst, "/workspace");
    }

    fn workspace_declaration(mount: &str) -> String {
        format!(
            "id = \"workspace-bind\"\n\
             description = \"Make the workspace available to the cell.\"\n\
             version = \"0.1.0\"\n\
             \n\
             [workspace]\n\
             mount = {mount}\n"
        )
    }

    #[test]
    fn a_written_backend_changes_nothing_in_the_parsed_declaration() {
        for (dst, parsed) in [("dst = \"/srv/work\"", "/srv/work"), ("", "/workspace")] {
            let without =
                PlasmidManifest::parse(&workspace_declaration(&format!("{{ {dst} }}"))).unwrap();
            let only_dst = WorkspaceMount {
                dst: parsed.to_string(),
            };
            assert_eq!(without.workspace, Some(only_dst));
            for backend in ["virtiofs", "fuse"] {
                let mount = if dst.is_empty() {
                    format!("{{ backend = \"{backend}\" }}")
                } else {
                    format!("{{ backend = \"{backend}\", {dst} }}")
                };
                let written = PlasmidManifest::parse(&workspace_declaration(&mount))
                    .unwrap_or_else(|error| panic!("{mount} was refused: {error:?}"));
                assert_eq!(written, without, "{mount}");
            }
        }
    }

    #[test]
    fn a_mount_with_no_dst_gets_the_workspace_path() {
        let manifest = PlasmidManifest::parse(&workspace_declaration("{}")).unwrap();
        assert_eq!(manifest.workspace.as_ref().unwrap().dst, "/workspace");
    }

    #[test]
    fn a_plasmid_carries_its_own_mock_alongside_the_hosts_it_stands_in_for() {
        let manifest = PlasmidManifest::parse(GITHUB_PR_WITH_MOCK).unwrap();
        assert_eq!(manifest.id, "github-pr");
        let mock = manifest.mock.as_ref().unwrap();
        assert_eq!(mock.hosts, vec!["api.github.com".to_string()]);
        assert_eq!(mock.kind, "recorded");
        assert_eq!(mock.api, "github");
        assert_eq!(mock.source, "fixtures/github-pr");
        assert_eq!(
            mock.hosts,
            manifest.network.as_ref().unwrap().hosts,
            "a mock names the hosts its own manifest declares, so the two lists cannot drift"
        );
    }

    fn refused_at(declaration: &str) -> (Option<String>, String) {
        let (plasmid, field, _) = field_refusal(declaration);
        (plasmid, field)
    }

    #[test]
    fn a_manifest_whose_whole_content_is_a_mock_is_refused() {
        assert_eq!(
            refused_at(MOCK_WITHOUT_NETWORK),
            (Some("mock-github".into()), "mock".into()),
            "a mock stands in for hosts a plasmid declares, so it is never a plasmid of its own"
        );
    }

    #[test]
    fn a_mock_naming_a_host_its_own_manifest_does_not_declare_is_refused() {
        let (plasmid, field, fix) = field_refusal(MOCK_HOSTS_DRIFTED_FROM_NETWORK);
        assert_eq!(
            (plasmid.as_deref(), field.as_str(), fix.as_str()),
            (Some("github-pr"), "network.hosts", "\"api.github.example\""),
            "the refusal names the host that drifted"
        );
    }

    #[test]
    fn a_mock_whose_hosts_is_a_bare_string_is_refused() {
        assert_eq!(
            refused_at(MOCK_HOSTS_AS_SCALAR),
            (Some("github-pr".into()), "mock.hosts".into()),
            "a scalar `hosts` declares no host at all, and silently standing in for nothing is \
             the failure a mock exists to prevent"
        );
    }

    #[test]
    fn a_mock_whose_hosts_holds_a_non_string_is_refused() {
        assert_eq!(
            refused_at(MOCK_HOSTS_MIXED_TYPES),
            (Some("github-pr".into()), "mock.hosts[1]".into()),
            "dropping the entry that is not a string would narrow the mock without saying so"
        );
    }

    #[test]
    fn a_pin_declared_as_a_bare_string_must_not_parse_to_no_pins_at_all() {
        assert_eq!(
            refused_at(NETWORK_PIN_CIDRS_AS_SCALAR),
            (Some("github-pr".into()), "network.pin_cidrs".into()),
            "a scalar pin_cidrs read as an empty list is an egress restriction that fails open \
             — the author declared a pin and nothing was pinned"
        );
    }

    #[test]
    fn a_pin_cidrs_holding_a_non_string_is_refused() {
        assert_eq!(
            refused_at(NETWORK_PIN_CIDRS_MIXED_TYPES),
            (Some("github-pr".into()), "network.pin_cidrs[1]".into()),
            "dropping the entry that is not a string would widen egress without saying so"
        );
    }

    #[test]
    fn a_network_hosts_declared_as_a_bare_string_is_refused_as_a_type_error_naming_the_field() {
        assert_eq!(
            refused_at(NETWORK_HOSTS_AS_SCALAR),
            (Some("github-pr".into()), "network.hosts".into()),
            "a hosts-shaped typo is a type error, not an absence, and reporting it as absence \
             sends the author looking for a line that is already there"
        );
    }

    #[test]
    fn a_command_network_hosts_declared_as_a_bare_string_is_refused() {
        assert_eq!(
            refused_at(COMMAND_NETWORK_HOSTS_AS_SCALAR),
            (
                Some("e13-commands-fixture".into()),
                "commands.commands.git.network.hosts".into()
            ),
            "a command whose network hosts read as an empty list carries no host restriction \
             at all, and nothing behind the parser notices"
        );
    }

    #[test]
    fn a_command_network_section_declared_as_a_scalar_is_refused() {
        assert_eq!(
            refused_at(COMMAND_NETWORK_SECTION_AS_SCALAR),
            (
                Some("e13-commands-fixture".into()),
                "commands.commands.git.network".into()
            ),
            "a network section that is not a table reads every field as absent, so the command \
             carries a network declaration that restricts nothing"
        );
    }

    #[test]
    fn a_network_section_declared_as_a_scalar_is_refused_as_a_type_error_not_an_absence() {
        assert_eq!(
            refused_at(NETWORK_SECTION_AS_SCALAR),
            (Some("probe".into()), "network".into()),
            "reporting a section-shaped typo as missing hosts sends the author looking for a \
             line that is already there"
        );
    }

    #[test]
    fn a_command_pin_cidrs_declared_as_a_bare_string_is_refused() {
        assert_eq!(
            refused_at(COMMAND_NETWORK_PIN_CIDRS_AS_SCALAR),
            (
                Some("e13-commands-fixture".into()),
                "commands.commands.git.network.pin_cidrs".into()
            ),
            "the fourth cell the criterion claims: a command-level pin that parses to no pins \
             at all must be refused by a test, not only by a shared helper"
        );
    }

    #[test]
    fn an_explicitly_empty_pin_cidrs_list_parses_and_pins_nothing() {
        let manifest = PlasmidManifest::parse(NETWORK_PIN_CIDRS_EMPTY).unwrap();
        assert!(manifest.network.as_ref().unwrap().pin_cidrs.is_empty());
    }

    const PORT_SECTIONS: [(&str, &str); 4] = [
        ("[network]\nhosts = [\"api.example.com\"]\n", "network"),
        (
            "[commands.commands.\"git ops\"]\nexec = [\"git\"]\n\
             [commands.commands.\"git ops\".network]\nhosts = [\"api.example.com\"]\n",
            "commands.commands.\"git ops\".network",
        ),
        (
            "[commands.commands.'say \"hi\"']\nexec = [\"say\"]\n\
             [commands.commands.'say \"hi\"'.network]\nhosts = [\"api.example.com\"]\n",
            "commands.commands.\"say \\\"hi\\\"\".network",
        ),
        (
            "[commands.commands.git]\nexec = [\"git\"]\n\
             [commands.commands.git.network]\nhosts = [\"api.example.com\"]\n",
            "commands.commands.git.network",
        ),
    ];

    const REMOVAL: &str = "remove this entry";

    fn ports_declaration(section: &str, ports: &str) -> String {
        format!("id = \"github-pr\"\ndescription = \"Reach the declared API.\"\n{section}{ports}\n")
    }

    fn ports_of(manifest: PlasmidManifest) -> Vec<u16> {
        match manifest.commands {
            Some(commands) => commands.commands[0].network.clone().map(|n| n.ports),
            None => manifest.network.map(|n| n.ports),
        }
        .expect("the declaration has a network section")
    }

    fn parsed_ports(section: &str, ports: &str) -> Result<Vec<u16>, ManifestError> {
        PlasmidManifest::parse(&ports_declaration(section, ports)).map(ports_of)
    }

    fn port_refusal(section: &str, ports: &str) -> (String, String, String) {
        refusal_of(&ports_declaration(section, ports))
    }

    fn refusal_of(declaration: &str) -> (String, String, String) {
        let error = PlasmidManifest::parse(declaration)
            .expect_err(&format!("{declaration} must be refused"));
        let ManifestError::Field {
            plasmid,
            field,
            fix,
            detail,
        } = error
        else {
            panic!("{declaration} was not refused with its field: {error:?}");
        };
        assert_eq!(plasmid.as_deref(), Some("github-pr"), "{declaration}");
        (field, fix, detail)
    }

    fn repaired_ports(section: &str, ports: &str, entry: Option<usize>, fix: &str) -> Vec<u16> {
        let repaired = with_port_fix(&ports_declaration(section, ports), entry, fix);
        match PlasmidManifest::parse(&repaired) {
            Ok(manifest) => ports_of(manifest),
            Err(error) => {
                assert!(
                    section.starts_with("[network]"),
                    "{ports} with {fix:?} applied is refused: {error}"
                );
                let (field, fix, _) = refusal_of(&repaired);
                assert_eq!(
                    (field.as_str(), fix.as_str()),
                    ("network.hosts[0]", REMOVAL),
                    "{ports}: only an emptied top-level list is refused, as hosts with no ports"
                );
                Vec::new()
            }
        }
    }

    fn with_port_fix(declaration: &str, entry: Option<usize>, fix: &str) -> String {
        let mut declaration: toml::Value = toml::from_str(declaration).unwrap();
        let network = match declaration.get_mut("commands") {
            Some(commands) => {
                let (_, command) = commands["commands"]
                    .as_table_mut()
                    .unwrap()
                    .iter_mut()
                    .next()
                    .unwrap();
                &mut command["network"]
            }
            None => &mut declaration["network"],
        };
        let network = network.as_table_mut().unwrap();
        match (entry, fix) {
            (Some(index), REMOVAL) => {
                network["ports"].as_array_mut().unwrap().remove(index);
            }
            (Some(index), replacement) => {
                let entry: toml::Table = toml::from_str(&format!("entry = {replacement}")).unwrap();
                network["ports"][index] = entry["entry"].clone();
            }
            (None, REMOVAL) => {
                network.remove("ports");
            }
            (None, line) => {
                let line: toml::Table = toml::from_str(line).unwrap();
                network.insert("ports".into(), line["ports"].clone());
            }
        }
        toml::to_string(&declaration).unwrap()
    }

    #[test]
    fn ports_from_1_to_65535_parse_to_exactly_the_declared_list() {
        for (section, _) in PORT_SECTIONS {
            for (ports, expected) in [
                ("ports = [443]", vec![443]),
                ("ports = [1]", vec![1]),
                ("ports = [65535]", vec![65535]),
                ("ports = [65535, 1, 443]", vec![65535, 1, 443]),
                ("ports = []", vec![]),
                ("", vec![]),
            ] {
                if expected.is_empty() && section.starts_with("[network]") {
                    let (field, fix, _) = port_refusal(section, ports);
                    assert_eq!(
                        (field.as_str(), fix.as_str()),
                        ("network.hosts[0]", REMOVAL),
                        "{section}{ports}: a top-level host with no port is refused"
                    );
                    continue;
                }
                assert_eq!(
                    parsed_ports(section, ports).unwrap(),
                    expected,
                    "{section}{ports}"
                );
            }
        }
    }

    #[test]
    fn a_port_that_is_not_an_integer_from_1_to_65535_is_refused_at_its_own_entry() {
        for (section, path) in PORT_SECTIONS {
            for (ports, entry, value, fix, repaired) in [
                ("ports = [65536]", Some(0), "65536", REMOVAL, vec![]),
                ("ports = [70000]", Some(0), "70000", REMOVAL, vec![]),
                ("ports = [0]", Some(0), "0", REMOVAL, vec![]),
                ("ports = [-1]", Some(0), "-1", REMOVAL, vec![]),
                ("ports = [\"443\"]", Some(0), "\"443\"", "443", vec![443]),
                ("ports = [443.0]", Some(0), "443.0", "443", vec![443]),
                ("ports = [true]", Some(0), "true", REMOVAL, vec![]),
                (
                    "ports = [443, \"x\", 8080]",
                    Some(1),
                    "\"x\"",
                    REMOVAL,
                    vec![443, 8080],
                ),
                ("ports = [1, 65536]", Some(1), "65536", REMOVAL, vec![1]),
                ("ports = [443, 70000]", Some(1), "70000", REMOVAL, vec![443]),
                (
                    "ports = [8080, 70000]",
                    Some(1),
                    "70000",
                    REMOVAL,
                    vec![8080],
                ),
                (
                    "ports = [8080, \"8080\"]",
                    Some(1),
                    "\"8080\"",
                    REMOVAL,
                    vec![8080],
                ),
                (
                    "ports = [443, \"8080\", 22]",
                    Some(1),
                    "\"8080\"",
                    "8080",
                    vec![443, 8080, 22],
                ),
                ("ports = 443", None, "443", "ports = [443]", vec![443]),
                ("ports = \"http\"", None, "\"http\"", REMOVAL, vec![]),
            ] {
                let row = format!("{section}{ports}");
                let (field, actual_fix, detail) = port_refusal(section, ports);
                let expected_field = match entry {
                    Some(index) => format!("{path}.ports[{index}]"),
                    None => format!("{path}.ports"),
                };
                assert_eq!(field, expected_field, "{row}");
                assert!(
                    detail.starts_with(&format!("{value} is not ")),
                    "{row}: {detail}"
                );
                assert_eq!(actual_fix, fix, "{row}");
                assert_eq!(
                    repaired_ports(section, ports, entry, &actual_fix),
                    repaired,
                    "{row}"
                );
            }
        }
    }

    #[test]
    fn following_each_port_fix_from_the_first_refused_entry_keeps_every_declared_port() {
        for (section, path) in PORT_SECTIONS {
            let mut declaration =
                ports_declaration(section, "ports = [70000, \"8080\", 443, \"x\", \"443\"]");
            for (index, fix) in [(0, REMOVAL), (0, "8080"), (2, REMOVAL), (2, REMOVAL)] {
                let (field, actual_fix, _) = refusal_of(&declaration);
                assert_eq!(field, format!("{path}.ports[{index}]"), "{declaration}");
                assert_eq!(actual_fix, fix, "{declaration}");
                declaration = with_port_fix(&declaration, Some(index), &actual_fix);
            }
            assert_eq!(
                PlasmidManifest::parse(&declaration).map(ports_of).unwrap(),
                vec![8080, 443],
                "{declaration}"
            );
        }
    }

    #[test]
    fn a_port_refusal_fix_is_the_port_the_author_evidently_meant_or_the_entry_removed() {
        let (section, path) = PORT_SECTIONS[0];
        for (ports, entry, fix, repaired) in [
            ("ports = 8080", None, "ports = [8080]", vec![8080]),
            ("ports = \"8080\"", None, "ports = [8080]", vec![8080]),
            ("ports = \" 8080 \"", None, "ports = [8080]", vec![8080]),
            ("ports = 8080.0", None, "ports = [8080]", vec![8080]),
            ("ports = 70000", None, REMOVAL, vec![]),
            ("ports = 0", None, REMOVAL, vec![]),
            ("ports = -8080", None, REMOVAL, vec![]),
            ("ports = { port = 8080 }", None, REMOVAL, vec![]),
            ("ports = [\"8080\"]", Some(0), "8080", vec![8080]),
            ("ports = [\"8080 \"]", Some(0), "8080", vec![8080]),
            ("ports = [\" 8080\"]", Some(0), "8080", vec![8080]),
            ("ports = [\"+8080\"]", Some(0), "8080", vec![8080]),
            ("ports = [\"08080\"]", Some(0), "8080", vec![8080]),
            ("ports = [8080.0]", Some(0), "8080", vec![8080]),
            ("ports = [22, \"8080\"]", Some(1), "8080", vec![22, 8080]),
            ("ports = [\"1\"]", Some(0), "1", vec![1]),
            ("ports = [\"65535\"]", Some(0), "65535", vec![65535]),
            ("ports = [1.0]", Some(0), "1", vec![1]),
            ("ports = [65535.0]", Some(0), "65535", vec![65535]),
            ("ports = [\"8080\", 8080]", Some(0), REMOVAL, vec![8080]),
            ("ports = [8080, 8080.0]", Some(1), REMOVAL, vec![8080]),
            ("ports = [\"0\"]", Some(0), REMOVAL, vec![]),
            ("ports = [\"65536\"]", Some(0), REMOVAL, vec![]),
            ("ports = [\"http\"]", Some(0), REMOVAL, vec![]),
            ("ports = [8080.5]", Some(0), REMOVAL, vec![]),
            ("ports = [0.0]", Some(0), REMOVAL, vec![]),
            ("ports = [65536.0]", Some(0), REMOVAL, vec![]),
            ("ports = [-8080.0]", Some(0), REMOVAL, vec![]),
            ("ports = [nan]", Some(0), REMOVAL, vec![]),
            ("ports = [inf]", Some(0), REMOVAL, vec![]),
            ("ports = [[8080]]", Some(0), REMOVAL, vec![]),
        ] {
            let (field, actual_fix, _) = port_refusal(section, ports);
            let expected_field = match entry {
                Some(index) => format!("{path}.ports[{index}]"),
                None => format!("{path}.ports"),
            };
            assert_eq!(field, expected_field, "{ports}");
            assert_eq!(actual_fix, fix, "{ports}");
            assert_eq!(
                repaired_ports(section, ports, entry, &actual_fix),
                repaired,
                "{ports}"
            );
        }
    }

    #[test]
    fn a_removal_fix_reads_as_a_removal_and_a_line_fix_as_a_line_to_write() {
        let (section, _) = PORT_SECTIONS[0];
        for (ports, rendered) in [
            (
                "ports = [443, \"x\"]",
                "plasmid github-pr: network.ports[1]: \"x\" is not an integer from 1 to 65535; \
                 remove this entry",
            ),
            (
                "ports = 8080",
                "plasmid github-pr: network.ports: 8080 is not a list of integers from 1 to 65535; \
                 write ports = [8080]",
            ),
        ] {
            let error = PlasmidManifest::parse(&ports_declaration(section, ports))
                .expect_err(&format!("{ports} must be refused"));
            assert_eq!(error.to_string(), rendered, "{ports}");
        }
    }

    #[test]
    fn the_reserved_commands_section_parses_to_child_domain_decls() {
        let manifest = PlasmidManifest::parse(COMMANDS_E13).unwrap();
        let commands = manifest.commands.as_ref().unwrap();
        assert_eq!(commands.address_plan, "10.29.0.0/24");
        assert_eq!(commands.commands.len(), 1);
        let git = &commands.commands[0];
        assert_eq!(git.id, "git");
        assert_eq!(git.exec, vec!["git".to_string()]);
        assert_eq!(git.subject.as_deref(), Some("git"));
        assert_eq!(
            git.network.as_ref().unwrap().hosts,
            vec!["alpha.ak.local".to_string(), "github.ak.local".to_string()]
        );
        assert_eq!(git.secrets[0].subject.as_deref(), Some("git"));
        assert_eq!(git.secrets[0].delivery, vec![DeliveryMode::Inject]);
    }

    #[test]
    fn mint_is_a_legal_fallback_for_a_git_consumer() {
        let text = r#"
id = "mint-fallback"
description = "Read GitHub with a minted fallback credential."
[network]
hosts = ["api.github.com"]
ports = [443]
[secrets]
refs = [{ id = "t", consumer = "git", delivery = ["helper", "mint"], ttl = "1h" }]
"#;
        let manifest = PlasmidManifest::parse(text).unwrap();
        assert_eq!(
            manifest.secrets[0].delivery,
            vec![DeliveryMode::Helper, DeliveryMode::Mint]
        );
    }

    #[test]
    fn invalid_credential_modes_and_scopes_are_refused_at_both_locations() {
        assert!(PlasmidManifest::parse(GITHUB_PR_LEGACY_STRING_REFS).is_err());
        let cases = [
            (
                r#"consumer = "wasm", scope = { path_scope = ["relative"] }"#,
                "scope.path_scope",
                DeliveryMode::Handle,
            ),
            (
                r#"consumer = "git", scope = { path_scope = ["relative"] }"#,
                "scope.path_scope",
                DeliveryMode::Helper,
            ),
            (
                r#"consumer = "git", delivery = []"#,
                "delivery",
                DeliveryMode::Helper,
            ),
            (
                r#"consumer = "wasm", delivery = ["inject"], scope = ["/v1/"]"#,
                "delivery",
                DeliveryMode::Handle,
            ),
            (
                r#"consumer = "git", delivery = ["handle"]"#,
                "delivery",
                DeliveryMode::Helper,
            ),
            (
                r#"consumer = "http", delivery = ["inject"]"#,
                "scope.path_scope",
                DeliveryMode::Inject,
            ),
            (
                r#"consumer = "http", delivery = ["inject"], scope = []"#,
                "scope.path_scope",
                DeliveryMode::Inject,
            ),
            (
                r#"consumer = "http", scope = { path_scope = [] }"#,
                "scope.path_scope",
                DeliveryMode::Inject,
            ),
            (
                r#"consumer = "process", scope = { path_scope = ["relative"] }"#,
                "scope.path_scope",
                DeliveryMode::Inject,
            ),
            (
                r#"consumer = "http", scope = ["/v1/", 7]"#,
                "scope",
                DeliveryMode::Inject,
            ),
            (
                r#"consumer = "http", delivery = ["teleport"]"#,
                "delivery",
                DeliveryMode::Mint,
            ),
        ];
        for (header, section) in [
            ("[secrets]", "secrets"),
            (
                "[commands.commands.\"git ops\"]\nexec = [\"git\"]\nsubject = \"git\"\n[commands.commands.\"git ops\".secrets]",
                "commands.commands.\"git ops\".secrets",
            ),
        ] {
            for (entry, suffix, expected) in cases {
                let text = format!(
                    "id = \"credential-example\"\ndescription = \"Use only a scoped credential.\"\nimpl.wasm = \"example.wasm\"\n{header}\nrefs = [{{ id = \"token\", {entry} }}]"
                );
                let error = PlasmidManifest::parse(&text).unwrap_err();
                let ManifestError::Field {
                    plasmid,
                    field,
                    fix,
                    ..
                } = error
                else {
                    panic!("credential refusal lost its repair context: {error:?}");
                };
                assert_eq!(plasmid.as_deref(), Some("credential-example"));
                assert_eq!(field, format!("{section}.refs[0].{suffix}"), "{entry}");
                let repair: toml::Value = toml::from_str(&fix).unwrap();
                let mut repaired: toml::Value = toml::from_str(&text).unwrap();
                let reference = if section == "secrets" {
                    &mut repaired["secrets"]["refs"][0]
                } else {
                    &mut repaired["commands"]["commands"]["git ops"]["secrets"]["refs"][0]
                };
                for (key, value) in repair.as_table().unwrap() {
                    reference
                        .as_table_mut()
                        .unwrap()
                        .insert(key.clone(), value.clone());
                }
                let accepted =
                    PlasmidManifest::parse(&toml::to_string(&repaired).unwrap()).unwrap();
                let secret = if section == "secrets" {
                    &accepted.secrets[0]
                } else {
                    &accepted.commands.as_ref().unwrap().commands[0].secrets[0]
                };
                assert_eq!(secret.delivery, vec![expected]);
            }
        }
    }

    #[test]
    fn command_ref_diagnostics_repair_missing_or_blank_subjects() {
        for (command_subject, ref_subject, command_key) in [
            ("", "", "git ops"),
            ("", "subject = \"\"", "git ops"),
            ("subject = \"\"", "", "git ops"),
            ("", "subject = \" \\t\"", "git ops"),
            ("subject = \" \\t\"", "", "git ops"),
            ("", "", " "),
        ] {
            let quoted_key = toml::Value::String(command_key.into()).to_string();
            let text = format!(
                "id = \"command-credential\"\ndescription = \"Run Git with its own credential.\"\n\
                 [commands.commands.{quoted_key}]\nexec = [\"git\"]\n{command_subject}\n\
                 [[commands.commands.{quoted_key}.secrets.refs]]\nid = \"token\"\n\
                 consumer = \"git\"\n{ref_subject}\n"
            );
            let error = PlasmidManifest::parse(&text).unwrap_err();
            let ManifestError::Field { field, fix, .. } = error else {
                panic!("missing command subject has no repair context: {error:?}");
            };
            assert_eq!(
                field,
                format!("commands.commands.{quoted_key}.secrets.refs[0].subject")
            );
            let mut declaration: toml::Value = toml::from_str(&text).unwrap();
            let repair: toml::Value = toml::from_str(&fix).unwrap();
            declaration["commands"]["commands"][command_key]["secrets"]["refs"][0]
                .as_table_mut()
                .unwrap()
                .insert("subject".into(), repair["subject"].clone());
            let repaired = PlasmidManifest::parse(&toml::to_string(&declaration).unwrap()).unwrap();
            let command = &repaired.commands.as_ref().unwrap().commands[0];
            assert!(
                !command.secrets[0]
                    .subject
                    .as_deref()
                    .unwrap()
                    .trim()
                    .is_empty()
            );
            assert_eq!(command.secrets[0].delivery, vec![DeliveryMode::Helper]);
        }
    }

    #[test]
    fn a_declaration_with_no_capability_and_no_implementation_is_refused_at_impl() {
        let (plasmid, field, fix) =
            field_refusal("id = \"empty\"\ndescription = \"A declaration with no capability.\"");
        assert_eq!(plasmid.as_deref(), Some("empty"));
        assert_eq!(field, "impl");
        assert_eq!(fix, "[impl]\nwasm = \"component.wasm\"");
    }

    #[test]
    fn a_network_section_with_no_hosts_is_refused_at_network_hosts() {
        for hosts in ["", "hosts = []"] {
            let (plasmid, field, fix) = field_refusal(&format!(
                "id = \"netless\"\ndescription = \"Reach a network host.\"\n\n[network]\n\
                 ports = [443]\n{hosts}"
            ));
            assert_eq!(plasmid.as_deref(), Some("netless"), "{hosts}");
            assert_eq!(field, "network.hosts", "{hosts}");
            assert_eq!(fix, "hosts = [\"api.example.com\"]", "{hosts}");
        }
    }

    #[test]
    fn an_empty_id_is_refused_naming_id_and_no_plasmid() {
        for declaration in [
            "id = \"\"\ndescription = \"Read pull requests.\"\nimpl.wasm = \"pr.wasm\"",
            "id = \"\"\nimpl.wasm = \"pr.wasm\"",
        ] {
            let (plasmid, field, fix) = field_refusal(declaration);
            assert_eq!(plasmid, None, "{declaration}");
            assert_eq!(field, "id", "{declaration}");
            assert_eq!(fix, "id = \"choose-a-stable-id\"", "{declaration}");
        }
    }

    #[test]
    fn a_blank_id_is_refused_naming_id_and_no_plasmid() {
        let every: Vec<char> = (0..=0x10FFFF)
            .filter_map(char::from_u32)
            .filter(|character| character.is_whitespace())
            .collect();
        assert_eq!(WHITE_SPACE.map(raw).to_vec(), every);
        let mut ids = Vec::from(blank_tool_keys());
        ids.push(("\"\u{3000}\"".to_string(), raw(0x3000).to_string()));
        for (written, decoded) in ids {
            for body in [
                "description = \"Read pull requests.\"\nimpl.wasm = \"pr.wasm\"",
                "impl.wasm = \"pr.wasm\"",
            ] {
                let declaration = format!("id = {written}\n{body}");
                let fixture: toml::Table = toml::from_str(&declaration).unwrap();
                assert_eq!(
                    fixture["id"].as_str(),
                    Some(decoded.as_str()),
                    "{declaration}"
                );
                assert!(decoded.trim().is_empty(), "{declaration}");
                assert_eq!(
                    field_refusal(&declaration),
                    (None, "id".into(), "id = \"choose-a-stable-id\"".into()),
                    "{declaration}"
                );
                let text = PlasmidManifest::parse(&declaration)
                    .unwrap_err()
                    .to_string();
                assert!(!text.starts_with("plasmid "), "{text:?}");
                assert!(!text.contains([0x85, 0x2028].map(raw)), "{text:?}");
            }
        }
    }

    #[test]
    fn an_id_that_is_not_only_whitespace_is_kept_as_written() {
        for (written, decoded) in [
            (" a", " a".to_string()),
            ("a ", "a ".to_string()),
            ("\u{3000}a", format!("{}a", raw(0x3000))),
            ("\\ta", format!("{}a", raw(0x09))),
            ("\u{feff}", raw(0xFEFF).to_string()),
            ("\u{200b}", raw(0x200B).to_string()),
            ("\u{180e}", raw(0x180E).to_string()),
        ] {
            let declaration = format!(
                "id = \"{written}\"\ndescription = \"Read pull requests.\"\nimpl.wasm = \"pr.wasm\""
            );
            let fixture: toml::Table = toml::from_str(&declaration).unwrap();
            assert_eq!(
                fixture["id"].as_str(),
                Some(decoded.as_str()),
                "{declaration}"
            );
            assert_eq!(
                PlasmidManifest::parse(&declaration).unwrap().id,
                decoded,
                "{declaration}"
            );
        }
    }

    #[test]
    fn missing_id_is_rejected() {
        let err = PlasmidManifest::parse("version = \"1\"").unwrap_err();
        assert!(matches!(
            err,
            ManifestError::Field { plasmid: None, field, fix, .. }
                if field == "id" && fix.parse::<toml::Value>().unwrap()["id"].is_str()
        ));
    }

    #[test]
    fn malformed_toml_is_a_parse_error() {
        let err = PlasmidManifest::parse("id = ").unwrap_err();
        assert!(matches!(err, ManifestError::Parse(_)));
    }

    #[test]
    fn declares_any_host_matches_only_declared_hosts() {
        let manifest = PlasmidManifest::parse(GITHUB_PR).unwrap();
        assert!(manifest.declares_any_host("api.github.com"));
        assert!(!manifest.declares_any_host("api.openai.com"));
    }

    const DECLARED: &str = "id = \"github-pr\"\ndescription = \"Reach the declared API.\"\n";

    const COMMAND_KEYS: [(&str, &str, &str); 3] = [
        ("git", "git", "\"git\""),
        ("git ops", "\"git ops\"", "\"git ops\""),
        ("say \"hi\"", "\"say \\\"hi\\\"\"", "\"say \\\"hi\\\"\""),
    ];

    const ESCAPE: &str = concat!("\\", "u001B");

    const CSI: &str = concat!("\\", "u009B");

    const DELETE: &str = concat!("\\", "u007F");

    const BELL: &str = concat!("\\", "u0007");

    const LINE_SEPARATOR: &str = concat!("\\", "u2028");

    const RIGHT_TO_LEFT: &str = concat!("\\", "u202E");

    const ACTED_ON: [u32; 14] = [
        0x061C, 0x2028, 0x2029, 0x200E, 0x200F, 0x202A, 0x202B, 0x202C, 0x202D, 0x202E, 0x2066,
        0x2067, 0x2068, 0x2069,
    ];

    fn raw(code: u32) -> char {
        char::from_u32(code).unwrap()
    }

    fn acted_on(character: char) -> bool {
        character.is_control() || ACTED_ON.contains(&u32::from(character))
    }

    fn line_and_bidi_characters() -> String {
        ACTED_ON.iter().map(|code| raw(*code)).collect()
    }

    fn declared(tail: &str) -> String {
        format!("{DECLARED}{tail}\n")
    }

    fn field_refusal(declaration: &str) -> (Option<String>, String, String) {
        let error = PlasmidManifest::parse(declaration)
            .expect_err(&format!("{declaration} must be refused"));
        let ManifestError::Field {
            plasmid,
            field,
            fix,
            detail,
        } = error
        else {
            panic!("{declaration} was not refused with its field: {error:?}");
        };
        assert!(!fix.trim().is_empty(), "{declaration} has no fix");
        assert!(!detail.trim().is_empty(), "{declaration} has no detail");
        (plasmid, field, fix)
    }

    fn field_keys(field: &str) -> (Vec<String>, Option<usize>) {
        let (path, index) = match field
            .strip_suffix(']')
            .and_then(|rest| rest.rsplit_once('['))
        {
            Some((path, index)) => (path, Some(index.parse().unwrap())),
            None => (field, None),
        };
        let mut value: toml::Value = format!("{path} = true").parse().unwrap();
        let mut keys = Vec::new();
        while let toml::Value::Table(table) = value {
            let (key, inner) = table.into_iter().next().unwrap();
            keys.push(key);
            value = inner;
        }
        (keys, index)
    }

    fn merged_section(document: &mut toml::Table, section: toml::Table, field: &[String]) {
        for (key, value) in section {
            let inside = field.first() == Some(&key);
            if inside && field.len() == 1 {
                document.insert(key, value);
                continue;
            }
            let below = if inside { &field[1..] } else { &[] };
            match (document.get_mut(&key), value) {
                (None, value) => {
                    document.insert(key, value);
                }
                (Some(toml::Value::Table(existing)), toml::Value::Table(value)) => {
                    merged_section(existing, value, below)
                }
                (Some(existing), _) => {
                    panic!("the section would overwrite {key} = {existing}, which is not the field")
                }
            }
        }
    }

    fn parsed_entry(entry: &str) -> toml::Value {
        let table: toml::Table = toml::from_str(&format!("entry = {entry}"))
            .unwrap_or_else(|error| panic!("{entry:?} is not a TOML value: {error}"));
        table["entry"].clone()
    }

    fn with_fix(declaration: &str, field: &str, fix: &str) -> String {
        let mut document: toml::Value = toml::from_str(declaration).unwrap();
        let line = fix.parse::<toml::Table>();
        let (keys, index) = field_keys(field);
        if fix.starts_with('[') {
            let section =
                line.unwrap_or_else(|error| panic!("{fix:?} is not a TOML section: {error}"));
            assert_eq!(index, None, "a section fix for the entry {field}");
            merged_section(document.as_table_mut().unwrap(), section, &keys);
            return toml::to_string(&document).unwrap();
        }
        let (last, parents) = keys.split_last().unwrap();
        let mut parent = &mut document;
        for key in parents {
            parent = &mut parent[key.as_str()];
        }
        let parent = parent.as_table_mut().unwrap();
        match (index, fix, line) {
            (Some(index), REMOVAL, _) => {
                parent[last.as_str()].as_array_mut().unwrap().remove(index);
            }
            (Some(index), entry, _) => {
                parent[last.as_str()][index] = parsed_entry(entry);
            }
            (None, REMOVAL, _) => {
                parent.remove(last.as_str());
            }
            (None, _, Ok(line)) => {
                assert_eq!(
                    line.keys().collect::<Vec<_>>(),
                    vec![last],
                    "{fix:?} must write exactly {field}"
                );
                parent.insert(last.clone(), line[last.as_str()].clone());
            }
            (None, entry, Err(_)) => {
                parent[last.as_str()]
                    .as_array_mut()
                    .unwrap()
                    .push(parsed_entry(entry));
            }
        }
        toml::to_string(&document).unwrap()
    }

    fn author_refusals() -> Vec<(String, Option<&'static str>, String, String)> {
        let github = Some("github-pr");
        let mut rows: Vec<(String, Option<&'static str>, String, String)> = [
            (
                "id = \"\"\ndescription = \"Reach the declared API.\"\nimpl.wasm = \"pr.wasm\"\n".to_string(),
                None,
                "id",
                "id = \"choose-a-stable-id\"",
            ),
            (
                "id = \"\"\nimpl.wasm = \"pr.wasm\"\n".to_string(),
                None,
                "id",
                "id = \"choose-a-stable-id\"",
            ),
            (
                "id = \" \"\ndescription = \"Reach the declared API.\"\nimpl.wasm = \"pr.wasm\"\n".to_string(),
                None,
                "id",
                "id = \"choose-a-stable-id\"",
            ),
            (
                "id = \"\u{3000}\"\ndescription = \"Reach the declared API.\"\nimpl.wasm = \"pr.wasm\"\n".to_string(),
                None,
                "id",
                "id = \"choose-a-stable-id\"",
            ),
            (declared(""), github, "impl", "[impl]\nwasm = \"component.wasm\""),
            (
                declared("[network]\nports = [443]"),
                github,
                "network.hosts",
                "hosts = [\"api.example.com\"]",
            ),
            (declared("[network]\nhosts = []"), github, "network", REMOVAL),
            (
                declared("network = \"api.github.com\""),
                github,
                "network",
                "[network]\nhosts = [\"api.github.com\"]",
            ),
            (
                declared("network = 443"),
                github,
                "network",
                "[network]\nhosts = [\"api.example.com\"]",
            ),
            (
                declared("[network]\nhosts = \"api.github.com\""),
                github,
                "network.hosts",
                "hosts = [\"api.github.com\"]",
            ),
            (
                declared("[network]\nhosts = 443"),
                github,
                "network.hosts",
                "hosts = [\"api.example.com\"]",
            ),
            (
                declared("[network]\nhosts = \" \""),
                github,
                "network.hosts",
                "hosts = [\"api.example.com\"]",
            ),
            (
                declared("[network]\nhosts = [\"api.github.com\", 443]\nports = [443]"),
                github,
                "network.hosts[1]",
                REMOVAL,
            ),
            (
                declared("[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = \"140.82.112.0/20\""),
                github,
                "network.pin_cidrs",
                "pin_cidrs = [\"140.82.112.0/20\"]",
            ),
            (
                declared("[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = 20"),
                github,
                "network.pin_cidrs",
                "pin_cidrs = [\"192.0.2.0/24\"]",
            ),
            (
                declared("[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = [\"140.82.112.0/20\", 20]"),
                github,
                "network.pin_cidrs[1]",
                REMOVAL,
            ),
            (
                declared("[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = [20]"),
                github,
                "network.pin_cidrs[0]",
                "\"192.0.2.0/24\"",
            ),
            (
                declared("[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = [20, 24]"),
                github,
                "network.pin_cidrs[0]",
                "\"192.0.2.0/24\"",
            ),
            (
                declared("[network]\nhosts = [\"api.github.com\"]\nports = [443]\n[mock]\nhosts = 5"),
                github,
                "mock.hosts",
                REMOVAL,
            ),
            (
                declared("[network]\nhosts = [\"api.github.com\"]\nports = [443]\n[mock]\nhosts = \" \""),
                github,
                "mock.hosts",
                REMOVAL,
            ),
            (
                declared("impl.wasm = \"pr.wasm\"\nnetwork = 443\n[mock]\nhosts = [\"api.github.com\"]"),
                github,
                "network",
                "[network]\nhosts = [\"api.github.com\"]",
            ),
            (
                declared(
                    "impl.wasm = \"pr.wasm\"\nnetwork = [\"api.github.com\", \"uploads.github.com\"]",
                ),
                github,
                "network",
                "[network]\nhosts = [\"api.github.com\", \"uploads.github.com\"]",
            ),
            (
                declared("[network]\nhosts = 443\nports = [443]\n[mock]\nhosts = [\"api.github.com\"]"),
                github,
                "network.hosts",
                "hosts = [\"api.github.com\"]",
            ),
            (
                declared("[network]\nhosts = []\nports = [443]\n[mock]\nhosts = [\"api.github.com\"]"),
                github,
                "network.hosts",
                "hosts = [\"api.github.com\"]",
            ),
            (
                declared(
                    "[network]\nports = [443]\n[mock]\nhosts = [\"api.github.com\", \"uploads.github.com\"]",
                ),
                github,
                "network.hosts",
                "hosts = [\"api.github.com\", \"uploads.github.com\"]",
            ),
            (
                declared("commands = [\"gh\", \"gh\"]"),
                github,
                "commands",
                "[commands.commands.gh]\nexec = [\"gh\"]",
            ),
            (
                declared("[commands]\ncommands = [\"gh\", \"gh\"]"),
                github,
                "commands.commands",
                "[commands.commands.gh]\nexec = [\"gh\"]",
            ),
            (
                declared("commands = [\"gh\", \"git\"]"),
                github,
                "commands",
                "[commands.commands.gh]\nexec = [\"gh\"]\n[commands.commands.git]\nexec = [\"git\"]",
            ),
            (
                declared("network = [\"a.example\", \"a.example\"]"),
                github,
                "network",
                "[network]\nhosts = [\"a.example\"]",
            ),
            (
                declared("[network]\nhosts = 443\nports = [443]\n[mock]\nhosts = [\"a.example\", \"a.example\"]"),
                github,
                "network.hosts",
                "hosts = [\"a.example\"]",
            ),
            (
                declared("[mock]\nhosts = [\"a.example\", \"a.example\"]"),
                github,
                "mock",
                REMOVAL,
            ),
            (
                declared("[network]\nhosts = []\nports = [443]\n[mock]\nhosts = [\"a.example\", \"a.example\"]"),
                github,
                "network.hosts",
                "hosts = [\"a.example\"]",
            ),
            (
                declared("[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = [20, \"10.0.0.0/8\"]"),
                github,
                "network.pin_cidrs[0]",
                REMOVAL,
            ),
            (
                declared("[commands.commands]\ngit = \" \""),
                github,
                "commands.commands.git",
                "[commands.commands.git]\nexec = [\"git\"]",
            ),
            (
                declared("[commands.commands.git]\nexec = \" \""),
                github,
                "commands.commands.git.exec",
                "exec = [\"git\"]",
            ),
            (
                declared("[commands.commands.\"café\"]\nsubject = \"git\""),
                github,
                "commands.commands.\"café\".exec",
                "exec = [\"café\"]",
            ),
            (
                declared("[mock]\nhosts = [\"a.example\", \"b.example\"]"),
                github,
                "mock",
                REMOVAL,
            ),
            (
                declared("[network]\nhosts = \"a\\\"b\\\\c\""),
                github,
                "network.hosts",
                "hosts = [\"a\\\"b\\\\c\"]",
            ),
            (
                declared("[network]\nhosts = \"x\\ny\""),
                github,
                "network.hosts",
                "hosts = [\"x\\ny\"]",
            ),
            (
                declared("commands = 5"),
                github,
                "commands",
                "[commands.commands.git]\nexec = [\"git\"]",
            ),
            (
                declared("commands = \"gh\""),
                github,
                "commands",
                "[commands.commands.gh]\nexec = [\"gh\"]",
            ),
            (
                declared("[commands]\ncommands = 5"),
                github,
                "commands.commands",
                "[commands.commands.git]\nexec = [\"git\"]",
            ),
            (
                declared("[commands]\ncommands = \"gh\""),
                github,
                "commands.commands",
                "[commands.commands.gh]\nexec = [\"gh\"]",
            ),
            (
                declared("[commands.commands]\ngit = \"/usr/bin/git\""),
                github,
                "commands.commands.git",
                "[commands.commands.git]\nexec = [\"/usr/bin/git\"]",
            ),
            (
                declared("[commands.commands]\ngit = 5"),
                github,
                "commands.commands.git",
                "[commands.commands.git]\nexec = [\"git\"]",
            ),
            (
                declared("[commands]\n[commands.commands]"),
                github,
                "commands.commands",
                "[commands.commands.git]\nexec = [\"git\"]",
            ),
            (
                declared("[commands.commands.git]\nexec = []"),
                github,
                "commands.commands.git.exec",
                "exec = [\"git\"]",
            ),
            (
                declared("[commands.commands.git]\nexec = [5]"),
                github,
                "commands.commands.git.exec[0]",
                REMOVAL,
            ),
            (
                declared("[commands.commands.gh]\nexec = \"/opt/bin/gh\""),
                github,
                "commands.commands.gh.exec",
                "exec = [\"/opt/bin/gh\"]",
            ),
            (
                declared("[commands.commands.\"\"]\nsubject = \"git\""),
                github,
                "commands.commands.\"\".exec",
                "exec = [\"command-name\"]",
            ),
            (
                declared("[commands.commands.\" \"]\nsubject = \"git\""),
                github,
                "commands.commands.\" \".exec",
                "exec = [\"command-name\"]",
            ),
            (
                declared("[network]\nhosts = [\"api.github.com\"]\nports = [443]\n[mock]\nhosts = \"api.github.com\""),
                github,
                "mock.hosts",
                "hosts = [\"api.github.com\"]",
            ),
            (
                declared("[network]\nhosts = [\"api.github.com\"]\nports = [443]\n[mock]\nhosts = [\"api.github.com\", 443]"),
                github,
                "mock.hosts[1]",
                REMOVAL,
            ),
            (
                declared("[commands]\naddress_plan = \"10.29.0.0/24\""),
                github,
                "commands.commands",
                "[commands.commands.git]\nexec = [\"git\"]",
            ),
            (
                declared("[mock]\nhosts = [\"api.github.com\"]"),
                github,
                "mock",
                REMOVAL,
            ),
            (declared("[mock]\nhosts = []"), github, "mock", REMOVAL),
            (declared("mock = 5"), github, "mock", REMOVAL),
            (
                declared("[network]\nhosts = [\"api.github.com\"]\nports = [443]\n[mock]\nhosts = [\"b\\nc\"]"),
                github,
                "mock.hosts[0]",
                REMOVAL,
            ),
            (
                declared("[commands.commands.\"a\\nb\"]\nsubject = \"git\""),
                github,
                "commands.commands.\"a\\nb\".exec",
                "exec = [\"a\\nb\"]",
            ),
            (
                declared("[network]\nhosts = [\"api.github.com\"]\nports = [443]\n[mock]\nhosts = [\"api.github.example\"]"),
                github,
                "network.hosts",
                "\"api.github.example\"",
            ),
            (
                declared(
                    "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n\
                     [mock]\nhosts = [\"api.github.com\", \"uploads.github.example\"]",
                ),
                github,
                "network.hosts",
                "\"uploads.github.example\"",
            ),
            (
                declared("impl.wasm = \"pr.wasm\"\n[provides.\"\".tools]\n\"b.read\" = \"Read b.\""),
                github,
                "provides.\"\"",
                REMOVAL,
            ),
            (
                declared("impl.wasm = \"pr.wasm\"\n[provides.\" \".tools]\n\"b.read\" = \"Read b.\""),
                github,
                "provides.\" \"",
                REMOVAL,
            ),
            (
                declared("impl.wasm = \"pr.wasm\"\n[provides.\"\\t\".tools]\n\"b.read\" = \"Read b.\""),
                github,
                "provides.\"\\t\"",
                REMOVAL,
            ),
        ]
        .into_iter()
        .map(|(declaration, plasmid, field, fix)| {
            (declaration, plasmid, field.to_string(), fix.to_string())
        })
        .collect();
        rows.push((
            declared(&format!(
                "[commands.commands.\"a{ESCAPE}b\"]\nsubject = \"git\""
            )),
            github,
            format!("commands.commands.\"a{ESCAPE}b\".exec"),
            format!("exec = [\"a{ESCAPE}b\"]"),
        ));
        rows.push((
            declared(&format!(
                "[commands.commands.\"a{CSI}b\"]\nsubject = \"git\""
            )),
            github,
            format!("commands.commands.\"a{CSI}b\".exec"),
            format!("exec = [\"a{CSI}b\"]"),
        ));
        rows.push((
            declared(&format!("[network]\nhosts = \"x{}y\"", raw(0x2028))),
            github,
            "network.hosts".to_string(),
            format!("hosts = [\"x{LINE_SEPARATOR}y\"]"),
        ));
        rows.push((
            declared(&format!(
                "[commands.commands.\"a{}b\"]\nsubject = \"git\"",
                raw(0x202E)
            )),
            github,
            format!("commands.commands.\"a{RIGHT_TO_LEFT}b\".exec"),
            format!("exec = [\"a{RIGHT_TO_LEFT}b\"]"),
        ));
        rows.push((
            declared(&format!("[network]\nhosts = \"x{ESCAPE}y\"")),
            github,
            "network.hosts".to_string(),
            format!("hosts = [\"x{ESCAPE}y\"]"),
        ));
        rows.push((
            declared(&format!(
                "impl.wasm = \"pr.wasm\"\n[provides.\"{}{}{LINE_SEPARATOR}\".tools]\n\"b.read\" = \"Read b.\"",
                concat!("\\", "u00A0"),
                concat!("\\", "u0085")
            )),
            github,
            format!(
                "provides.\"{}{}{LINE_SEPARATOR}\"",
                raw(0xA0),
                concat!("\\", "u0085")
            ),
            REMOVAL.to_string(),
        ));
        for name in [concat!("\\", "u0085"), LINE_SEPARATOR] {
            rows.push((
                declared(&format!(
                    "impl.wasm = \"pr.wasm\"\n[provides.\"a:tools\".tools]\n\"{name}\" = \"Read b.\""
                )),
                github,
                format!("provides.\"a:tools\".tools.\"{name}\""),
                REMOVAL.to_string(),
            ));
        }
        for (_, quoted, program) in COMMAND_KEYS {
            let command = format!("[commands.commands.{quoted}]\nexec = [\"git\"]\n");
            let path = format!("commands.commands.{quoted}");
            let network = format!("{command}[{path}.network]\n");
            for (tail, field, fix) in [
                (
                    format!("[{path}]\nsubject = \"git\""),
                    format!("{path}.exec"),
                    format!("exec = [{program}]"),
                ),
                (
                    format!("{command}network = \"alpha.ak.local\""),
                    format!("{path}.network"),
                    format!("[{path}.network]\nhosts = [\"alpha.ak.local\"]"),
                ),
                (
                    format!("{command}network = 443"),
                    format!("{path}.network"),
                    format!("[{path}.network]\nhosts = [\"api.example.com\"]"),
                ),
                (
                    format!("{network}hosts = \"alpha.ak.local\""),
                    format!("{path}.network.hosts"),
                    "hosts = [\"alpha.ak.local\"]".to_string(),
                ),
                (
                    format!("{network}hosts = [\"alpha.ak.local\", 443]"),
                    format!("{path}.network.hosts[1]"),
                    REMOVAL.to_string(),
                ),
                (
                    format!("{network}hosts = [\"alpha.ak.local\"]\npin_cidrs = \"10.29.0.0/24\""),
                    format!("{path}.network.pin_cidrs"),
                    "pin_cidrs = [\"10.29.0.0/24\"]".to_string(),
                ),
                (
                    format!(
                        "{network}hosts = [\"alpha.ak.local\"]\npin_cidrs = [\"10.29.0.0/24\", 24]"
                    ),
                    format!("{path}.network.pin_cidrs[1]"),
                    REMOVAL.to_string(),
                ),
                (
                    format!("{network}hosts = [\"alpha.ak.local\"]\npin_cidrs = [24]"),
                    format!("{path}.network.pin_cidrs[0]"),
                    "\"192.0.2.0/24\"".to_string(),
                ),
                (
                    format!("[commands.commands]\n{quoted} = 5"),
                    path.clone(),
                    format!("[{path}]\nexec = [{program}]"),
                ),
            ] {
                rows.push((declared(&tail), github, field, fix));
            }
        }
        for (tail, field, fix) in exact_value_refusals() {
            rows.push((declared(tail), github, field.to_string(), fix.to_string()));
        }
        rows
    }

    const WITH_HOST: &str = "[network]\nhosts = [\"api.github.com\"]\n";

    fn exact_value_refusals() -> Vec<(&'static str, &'static str, &'static str)> {
        vec![
            ("[network]", "network", REMOVAL),
            ("[network]\nhosts = []\nports = []", "network", REMOVAL),
            (
                "[network]\nhosts = []\nports = []\npin_cidrs = [\"192.0.2.0/24\"]",
                "network",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]",
                "network.hosts[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = []",
                "network.hosts[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"a.example\", \"b.example\"]\nports = []",
                "network.hosts[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"API.github.com\"]\nports = []",
                "network.hosts[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [70000]",
                "network.ports[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [-1]",
                "network.ports[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [0]",
                "network.ports[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [65536]",
                "network.ports[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [\"443\"]",
                "network.ports[0]",
                "443",
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [443.5]",
                "network.ports[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [443, 443]",
                "network.ports[1]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [443, 8080, 22, 8080, 443]",
                "network.ports[3]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"API.github.com\"]\nports = [443]",
                "network.hosts[0]",
                "\"api.github.com\"",
            ),
            (
                "[network]\nhosts = [\"api.github.com\", \"Api.GitHub.com\"]\nports = [443]",
                "network.hosts[1]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"API.github.com\", \"api.github.com\"]\nports = [443]",
                "network.hosts[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"API.GITHUB.COM.\"]\nports = [443]",
                "network.hosts[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"192.0.2.1\"]\nports = [443]",
                "network.hosts[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\", \"2001:db8::1\"]\nports = [443]",
                "network.hosts[1]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"0x7f000001\"]\nports = [443]",
                "network.hosts[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"a.example\", \"b.example\", \"a.example\"]\nports = [443]",
                "network.hosts[2]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = [\"nonsense\"]",
                "network.pin_cidrs[0]",
                "\"192.0.2.0/24\"",
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n\
                 pin_cidrs = [\"nonsense\", \"192.0.2.0/24\"]",
                "network.pin_cidrs[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = [\"192.0.2.1/24\"]",
                "network.pin_cidrs[0]",
                "\"192.0.2.0/24\"",
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n\
                 pin_cidrs = [\"192.0.2.0/24\", \"2001:db8::1/32\"]",
                "network.pin_cidrs[1]",
                "\"2001:db8::/32\"",
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n\
                 pin_cidrs = [\"192.0.2.0/24\", \"192.000.002.000/24\"]",
                "network.pin_cidrs[1]",
                REMOVAL,
            ),
            (
                "[workspace]",
                "workspace.mount",
                "mount = { dst = \"/workspace\" }",
            ),
            (
                "workspace = {}",
                "workspace.mount",
                "mount = { dst = \"/workspace\" }",
            ),
            (
                "impl.wasm = \"pr.wasm\"\n[workspace]",
                "workspace.mount",
                "mount = { dst = \"/workspace\" }",
            ),
            (
                "[workspace]\nmount = { dst = \"workspace\" }",
                "workspace.mount.dst",
                "dst = \"/workspace\"",
            ),
            (
                "[workspace]\nmount = { dst = \"/workspace/\" }",
                "workspace.mount.dst",
                "dst = \"/workspace\"",
            ),
            (
                "[workspace]\nmount = { dst = \"srv/work//\" }",
                "workspace.mount.dst",
                "dst = \"/srv/work\"",
            ),
            (
                "[workspace]\nmount = { dst = \"/\" }",
                "workspace.mount.dst",
                "dst = \"/workspace\"",
            ),
            (
                "[workspace]\nmount = { dst = \"\" }",
                "workspace.mount.dst",
                "dst = \"/workspace\"",
            ),
            (
                "[workspace]\nmount = { dst = \"/a/../b\" }",
                "workspace.mount.dst",
                "dst = \"/workspace\"",
            ),
            (
                "[workspace]\nmount = { dst = \"/a/./b\" }",
                "workspace.mount.dst",
                "dst = \"/workspace\"",
            ),
            (
                "[workspace]\nmount = { dst = \"/a//b\" }",
                "workspace.mount.dst",
                "dst = \"/workspace\"",
            ),
            (
                "[workspace]\nmount = { dst = \"a//b/\" }",
                "workspace.mount.dst",
                "dst = \"/workspace\"",
            ),
            (
                "[workspace.mount]\ndst = 5",
                "workspace.mount.dst",
                "dst = \"/workspace\"",
            ),
            (
                "[workspace.mount]\ndst = [\"/srv/work\"]",
                "workspace.mount.dst",
                "dst = \"/workspace\"",
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\n[mock]\nhosts = [\"uploads.github.com\"]",
                "mock",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\n[mock]\nhosts = [\"api.github.com\"]",
                "network.hosts[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = []\n[mock]\nhosts = [\"api.github.com\"]",
                "mock",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n\
                 [mock]\nhosts = [\"API.github.com\"]",
                "mock.hosts[0]",
                "\"api.github.com\"",
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n\
                 [mock]\nhosts = [\"api.github.com\", \"API.github.com\"]",
                "mock.hosts[1]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n\
                 [mock]\nhosts = [\"API.github.com\", \"api.github.com\"]",
                "mock.hosts[0]",
                REMOVAL,
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n\
                 [mock]\nhosts = [\"api.github.com\", \"192.0.2.1\"]",
                "mock.hosts[1]",
                REMOVAL,
            ),
        ]
    }

    #[test]
    fn every_author_refusal_names_its_exact_field_fix_and_plasmid() {
        let mut wrong = Vec::new();
        for (declaration, plasmid, field, fix) in author_refusals() {
            let expected = (plasmid.map(String::from), field, fix);
            let actual = match PlasmidManifest::parse(&declaration) {
                Err(ManifestError::Field { .. }) => Ok(field_refusal(&declaration)),
                Err(error) => Err(error.to_string()),
                Ok(_) => Err("parses".to_string()),
            };
            if actual.as_ref() != Ok(&expected) {
                wrong.push(format!(
                    "{declaration:?}
  expected {expected:?}
  got {actual:?}"
                ));
            }
        }
        assert!(
            wrong.is_empty(),
            "{} rows differ:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }

    #[test]
    fn command_refusals_name_the_command_by_its_whole_toml_path() {
        for (key, quoted, _) in COMMAND_KEYS {
            for tail in [
                format!(
                    "[commands.commands.{quoted}]\nexec = [\"git\"]\nnetwork = \"alpha.ak.local\""
                ),
                format!("[commands.commands.{quoted}]\nsubject = \"git\""),
                format!(
                    "[commands.commands.{quoted}]\nexec = [\"git\"]\n\
                     [commands.commands.{quoted}.network]\nhosts = [\"alpha.ak.local\", 443]"
                ),
            ] {
                let (_, field, _) = field_refusal(&declared(&tail));
                assert!(
                    field.starts_with(&format!("commands.commands.{quoted}.")),
                    "{key}: {field}"
                );
            }
        }
    }

    #[test]
    fn following_the_fixes_from_each_author_refusal_ends_in_a_declaration_that_parses() {
        for (start, ..) in author_refusals() {
            let mut declaration = start.clone();
            let mut followed: Vec<(String, String)> = Vec::new();
            let mut written: Vec<String> = Vec::new();
            while let Err(error) = PlasmidManifest::parse(&declaration) {
                let (_, field, fix) = field_refusal(&declaration);
                assert!(
                    followed.len() < 4,
                    "{start}\nafter {followed:?} is refused again: {error}"
                );
                declaration = with_fix(&declaration, &field, &fix);
                assert!(
                    !written.contains(&declaration),
                    "{start}\n{field} with {fix:?} returns to a declaration already refused, \
                     after {followed:?}"
                );
                written.push(declaration.clone());
                followed.push((field, fix));
            }
        }
    }

    #[test]
    fn a_mock_host_fix_adds_only_the_host_its_network_does_not_declare() {
        let declaration = declared(
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n\
             [mock]\nhosts = [\"api.github.com\", \"uploads.github.example\"]",
        );
        let (_, field, fix) = field_refusal(&declaration);
        let repaired = PlasmidManifest::parse(&with_fix(&declaration, &field, &fix)).unwrap();
        assert_eq!(
            repaired.network.unwrap().hosts,
            vec![
                "api.github.com".to_string(),
                "uploads.github.example".to_string()
            ]
        );
    }

    fn within(seconds: u64, what: &'static str, work: impl FnOnce() + Send + 'static) {
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            work();
            send.send(()).unwrap();
        });
        match receive.recv_timeout(std::time::Duration::from_secs(seconds)) {
            Ok(()) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("{what} did not finish within {seconds} s: the check is not linear")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("{what} panicked")
            }
        }
    }

    #[test]
    fn distinct_keeps_the_first_of_each_name_in_the_authors_order() {
        for (names, expected) in [
            (vec!["b", "a", "b", "c", "a"], vec!["b", "a", "c"]),
            (vec![], vec![]),
            (vec!["x", "x", "x"], vec!["x"]),
        ] {
            let names: Vec<String> = names.into_iter().map(String::from).collect();
            assert_eq!(distinct(&names), expected);
        }
    }

    #[test]
    fn distinct_of_half_a_million_names_finishes_within_30_seconds() {
        within(30, "distinct", || {
            let expected: Vec<String> = (0..250_000).map(|index| format!("h{index}")).collect();
            let names = expected
                .iter()
                .chain(&expected)
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(distinct(&names), expected);
        });
    }

    fn commands_with_subjects(subject: Option<&str>, subjects: &[Option<&str>]) -> CommandsSpec {
        CommandsSpec {
            address_plan: "explicit".into(),
            commands: vec![CommandDecl {
                id: "git".into(),
                exec: vec!["git".into()],
                subject: subject.map(String::from),
                network: None,
                secrets: subjects
                    .iter()
                    .map(|subject| SecretRef {
                        id: "example:token".into(),
                        consumer: SecretConsumer::Http,
                        delivery: vec![DeliveryMode::Mint],
                        scope: None,
                        ttl: None,
                        subject: subject.map(String::from),
                    })
                    .collect(),
            }],
        }
    }

    #[test]
    fn a_command_subject_inherited_by_6000_credentials_is_checked_within_30_seconds() {
        within(30, "command subject inherited by 6000 credentials", || {
            let subject = format!("{}x", " ".repeat(524_288));
            let commands = commands_with_subjects(Some(&subject), &vec![None; 6_000]);
            validate_commands("github-pr", &commands).unwrap();
        });
    }

    #[test]
    fn a_blank_command_subject_accepts_credentials_with_their_own_subjects() {
        let commands = commands_with_subjects(Some("   "), &[Some("first"), Some("second")]);
        validate_commands("github-pr", &commands).unwrap();
    }

    #[test]
    fn a_blank_command_subject_refuses_the_third_credential_without_a_subject() {
        let commands = commands_with_subjects(Some("   "), &[Some("first"), Some("second"), None]);
        let ManifestError::Field {
            field, fix, detail, ..
        } = validate_commands("github-pr", &commands).unwrap_err()
        else {
            panic!("expected a field refusal");
        };
        assert_eq!(field, "commands.commands.git.secrets.refs[2].subject");
        assert_eq!(fix, "subject = \"git\"");
        assert_eq!(
            detail,
            "a command credential requires a nonblank ref or command subject"
        );
    }

    #[test]
    fn an_absent_command_subject_refuses_the_first_credential_with_a_blank_subject() {
        let commands = commands_with_subjects(None, &[Some("   "), Some("second")]);
        let ManifestError::Field {
            field, fix, detail, ..
        } = validate_commands("github-pr", &commands).unwrap_err()
        else {
            panic!("expected a field refusal");
        };
        assert_eq!(field, "commands.commands.git.secrets.refs[0].subject");
        assert_eq!(fix, "subject = \"git\"");
        assert_eq!(
            detail,
            "a command credential requires a nonblank ref or command subject"
        );
    }

    #[test]
    fn mock_refusal_details_keep_the_first_of_each_host_in_the_authors_order() {
        let mut mock = MockSpec {
            hosts: ["c.example", "a.example", "b.example", "c.example"]
                .map(String::from)
                .into(),
            kind: "recorded".into(),
            api: "example".into(),
            source: "fixtures/example".into(),
        };
        let ManifestError::Field { detail, .. } =
            validate_mock("github-pr", &mock, None).unwrap_err()
        else {
            panic!("expected a field refusal");
        };
        assert_eq!(
            detail,
            "[mock] stands in for \"c.example\", \"a.example\" and \"b.example\", and there is no [network]"
        );
        mock.hosts = ["b.example", "c.example", "b.example", "a.example"]
            .map(String::from)
            .into();
        let network = CommandNetwork {
            hosts: vec!["a.example".into()],
            ports: vec![],
            pin_cidrs: vec![],
        };
        let ManifestError::Field { detail, .. } =
            validate_mock("github-pr", &mock, Some(&network)).unwrap_err()
        else {
            panic!("expected a field refusal");
        };
        assert_eq!(
            detail,
            "[mock] stands in for \"b.example\" and \"c.example\", which [network] does not declare, and [network] declares no port"
        );
    }

    fn large_mock() -> MockSpec {
        MockSpec {
            hosts: (0..300_000)
                .map(|index| format!("h{index}.example"))
                .collect(),
            kind: "recorded".into(),
            api: "example".into(),
            source: "fixtures/example".into(),
        }
    }

    #[test]
    fn a_mock_check_of_300000_hosts_with_a_port_finishes_within_30_seconds() {
        within(30, "mock with a port", || {
            let mock = large_mock();
            let mut network = CommandNetwork {
                hosts: mock.hosts.clone(),
                ports: vec![443],
                pin_cidrs: vec![],
            };
            let last = network.hosts.pop().unwrap();
            let error = validate_mock("github-pr", &mock, Some(&network)).unwrap_err();
            let ManifestError::Field {
                plasmid,
                field,
                fix,
                detail,
            } = error
            else {
                panic!("expected a field refusal");
            };
            assert_eq!(plasmid.as_deref(), Some("github-pr"));
            assert_eq!(field, "network.hosts");
            assert_eq!(fix, toml_string(&last));
            assert_eq!(
                detail,
                format!(
                    "[mock] stands in for {}, which [network] does not declare",
                    toml_string(&last)
                )
            );
        });
    }

    #[test]
    fn a_mock_check_of_300000_hosts_without_a_port_finishes_within_30_seconds() {
        within(30, "mock without a port", || {
            let mock = large_mock();
            let mut network = CommandNetwork {
                hosts: mock.hosts.clone(),
                ports: vec![],
                pin_cidrs: vec![],
            };
            let last = network.hosts.pop().unwrap();
            let error = validate_mock("github-pr", &mock, Some(&network)).unwrap_err();
            let ManifestError::Field {
                field, fix, detail, ..
            } = error
            else {
                panic!("expected a field refusal");
            };
            assert_eq!(field, "mock");
            assert_eq!(fix, REMOVE_ENTRY);
            assert_eq!(
                detail,
                format!(
                    "[mock] stands in for {}, which [network] does not declare, and [network] declares no port",
                    toml_string(&last)
                )
            );
        });
    }

    #[test]
    fn a_mock_check_of_300000_hosts_without_a_network_finishes_within_30_seconds() {
        within(30, "mock without a network", || {
            let mock = large_mock();
            let error = validate_mock("github-pr", &mock, None).unwrap_err();
            let ManifestError::Field {
                field, fix, detail, ..
            } = error
            else {
                panic!("expected a field refusal");
            };
            assert_eq!(field, "mock");
            assert_eq!(fix, REMOVE_ENTRY);
            assert_eq!(
                detail,
                format!(
                    "[mock] stands in for {}, and there is no [network]",
                    hosts_named(&mock.hosts)
                )
            );
        });
    }

    #[test]
    fn a_credential_with_300000_modes_and_paths_is_checked_within_30_seconds() {
        within(30, "credential modes and paths", || {
            let mut secret = SecretRef {
                id: "example:token".into(),
                consumer: SecretConsumer::Http,
                delivery: vec![DeliveryMode::Inject; 300_000],
                scope: Some(SecretScope {
                    path_scope: vec!["/a".into(); 300_000],
                    ..SecretScope::default()
                }),
                ttl: None,
                subject: None,
            };
            validate_secret_ref("github-pr", "secrets", 0, &secret, false).unwrap();
            *secret
                .scope
                .as_mut()
                .unwrap()
                .path_scope
                .last_mut()
                .unwrap() = "a".into();
            let error = validate_secret_ref("github-pr", "secrets", 0, &secret, false).unwrap_err();
            let ManifestError::Field {
                field, fix, detail, ..
            } = error
            else {
                panic!("expected a field refusal");
            };
            assert_eq!(field, "secrets.refs[0].scope.path_scope");
            assert_eq!(fix, "scope = { path_scope = [\"/required/path/\"] }");
            assert_eq!(
                detail,
                "path-scope entries must be absolute; inject also requires a nonempty scope"
            );
        });
    }

    #[test]
    fn a_declaration_listing_130000_hosts_is_refused_naming_each_host_once_within_120_seconds() {
        within(120, "130000-host declaration", || {
            let hosts: Vec<String> = (0..130_000)
                .map(|mut index| {
                    (0..4)
                        .map(|_| {
                            let letter = char::from(b'a' + (index % 26) as u8);
                            index /= 26;
                            letter
                        })
                        .collect()
                })
                .collect();
            let entries = hosts
                .iter()
                .chain(hosts.iter().take(1_000))
                .map(|host| toml_string(host))
                .collect::<Vec<_>>()
                .join(",");
            let declaration = format!("{DECLARED}network = [{entries}]");
            assert!(declaration.len() <= MAX_MANIFEST_BYTES);
            let error = PlasmidManifest::parse(&declaration).unwrap_err();
            let ManifestError::Field {
                plasmid,
                field,
                fix,
                detail,
            } = &error
            else {
                panic!("expected a field refusal");
            };
            assert_eq!(plasmid.as_deref(), Some("github-pr"));
            assert_eq!(field, "network");
            assert_eq!(
                fix,
                &format!("[network]\n{}", strings_line("hosts", &hosts))
            );
            eprintln!(
                "declaration={} detail={} fix={} display={}",
                declaration.len(),
                detail.len(),
                fix.len(),
                error.to_string().len()
            );
        });
    }

    #[test]
    fn a_list_entry_that_is_not_a_string_is_removed_by_its_index_and_no_other_entry_is() {
        let declaration = declared(
            "[network]\nhosts = [\"api.github.com\", 443, \"uploads.github.com\"]\nports = [443]\n\
             pin_cidrs = [\"140.82.112.0/20\", true, \"192.30.252.0/22\"]",
        );
        let (_, field, fix) = field_refusal(&declaration);
        assert_eq!(
            (field.as_str(), fix.as_str()),
            ("network.hosts[1]", REMOVAL)
        );
        let declaration = with_fix(&declaration, &field, &fix);
        let (_, field, fix) = field_refusal(&declaration);
        assert_eq!(
            (field.as_str(), fix.as_str()),
            ("network.pin_cidrs[1]", REMOVAL)
        );
        let network = PlasmidManifest::parse(&with_fix(&declaration, &field, &fix))
            .unwrap()
            .network
            .unwrap();
        assert_eq!(
            network.hosts,
            vec![
                "api.github.com".to_string(),
                "uploads.github.com".to_string()
            ]
        );
        assert_eq!(
            pins(&network),
            vec!["140.82.112.0/20".to_string(), "192.30.252.0/22".to_string()]
        );
    }

    fn pins(network: &NetworkSpec) -> Vec<String> {
        network.pin_cidrs.iter().map(ToString::to_string).collect()
    }

    fn written_ports(declaration: &str) -> Vec<toml::Value> {
        let document: toml::Value = toml::from_str(declaration).unwrap();
        document
            .get("network")
            .and_then(|network| network.get("ports"))
            .and_then(toml::Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    fn fixes_followed(start: &str) -> (Vec<(String, String)>, PlasmidManifest) {
        let mut declaration = start.to_string();
        let mut followed: Vec<(String, String)> = Vec::new();
        loop {
            let error = match PlasmidManifest::parse(&declaration) {
                Ok(manifest) => return (followed, manifest),
                Err(error) => error,
            };
            assert!(
                followed.len() < 10,
                "{start}\nhas no end after {followed:?}: {error}"
            );
            let (_, field, fix) = field_refusal(&declaration);
            let before = written_ports(&declaration);
            declaration = with_fix(&declaration, &field, &fix);
            let after = written_ports(&declaration);
            assert!(
                after.iter().all(|port| before.contains(port)),
                "{field} with {fix:?} wrote a port: {before:?} became {after:?}"
            );
            followed.push((field, fix));
        }
    }

    #[test]
    fn following_the_fixes_from_one_host_and_port_65536_ends_with_no_network_and_no_added_port() {
        let start = "id = \"github-pr\"\ndescription = \"Reach the declared API.\"\n\
                     [impl]\nwasm = \"pr.wasm\"\n\
                     [network]\nhosts = [\"api.github.com\"]\nports = [65536]\n";
        let (followed, manifest) = fixes_followed(start);
        let removed = |field: &str| (field.to_string(), REMOVAL.to_string());
        assert_eq!(
            followed,
            vec![
                removed("network.ports[0]"),
                removed("network.hosts[0]"),
                removed("network"),
            ]
        );
        assert_eq!(manifest.network, None);
        assert_eq!(manifest.wasm, Some(PathBuf::from("pr.wasm")));
    }

    #[test]
    fn hosts_with_no_ports_are_removed_one_entry_at_a_time_and_no_port_is_written() {
        for ports in ["", "ports = []"] {
            let start = format!(
                "id = \"github-pr\"\ndescription = \"Reach the declared API.\"\n\
                 impl.wasm = \"pr.wasm\"\n\
                 [network]\nhosts = [\"a.example\", \"b.example\", \"c.example\"]\n{ports}\n"
            );
            let (followed, manifest) = fixes_followed(&start);
            let fields: Vec<&str> = followed.iter().map(|(field, _)| field.as_str()).collect();
            assert_eq!(
                fields,
                [
                    "network.hosts[0]",
                    "network.hosts[0]",
                    "network.hosts[0]",
                    "network"
                ],
                "{start}"
            );
            assert!(followed.iter().all(|(_, fix)| fix == REMOVAL), "{start}");
            assert_eq!(manifest.network, None, "{start}");
        }
    }

    #[test]
    fn a_pin_fix_never_unpins_the_network() {
        for (pins_written, field, fix, kept) in [
            (
                "[\"nonsense\"]",
                "network.pin_cidrs[0]",
                "\"192.0.2.0/24\"",
                vec!["192.0.2.0/24"],
            ),
            (
                "[\"\"]",
                "network.pin_cidrs[0]",
                "\"192.0.2.0/24\"",
                vec!["192.0.2.0/24"],
            ),
            (
                "[\"nonsense\", \"192.0.2.0/24\"]",
                "network.pin_cidrs[0]",
                REMOVAL,
                vec!["192.0.2.0/24"],
            ),
            (
                "[\"192.0.2.0/24\", \"nonsense\", \"no\"]",
                "network.pin_cidrs[1]",
                REMOVAL,
                vec!["192.0.2.0/24"],
            ),
            (
                "[\"nonsense\", \"198.51.100.7/24\"]",
                "network.pin_cidrs[1]",
                "\"198.51.100.0/24\"",
                vec!["198.51.100.0/24"],
            ),
            (
                "[\"192.0.2.1/24\"]",
                "network.pin_cidrs[0]",
                "\"192.0.2.0/24\"",
                vec!["192.0.2.0/24"],
            ),
            (
                "[\"10.1.2.3/8\", \"2001:db8::1/32\"]",
                "network.pin_cidrs[0]",
                "\"10.0.0.0/8\"",
                vec!["10.0.0.0/8", "2001:db8::/32"],
            ),
        ] {
            let start = declared(&format!(
                "{WITH_HOST}ports = [443]\npin_cidrs = {pins_written}"
            ));
            let (_, actual_field, actual_fix) = field_refusal(&start);
            assert_eq!(
                (actual_field.as_str(), actual_fix.as_str()),
                (field, fix),
                "{pins_written}"
            );
            let (_, manifest) = fixes_followed(&start);
            assert_eq!(pins(&manifest.network.unwrap()), kept, "{pins_written}");
        }
    }

    #[test]
    fn a_cidr_parses_only_its_one_display_spelling_with_no_host_bits() {
        for text in [
            "192.0.2.0/24",
            "2001:db8::/32",
            "0.0.0.0/0",
            "::/0",
            "192.0.2.7/32",
            "255.255.255.255/32",
            "::1/128",
            "2001:db8::1/128",
            "2001:db8::8000/113",
            "10.0.0.0/8",
            "::ffff:0:c000:200/120",
            "64:ff9b::c000:200/120",
        ] {
            let cidr = Cidr::parse(text).unwrap_or_else(|error| panic!("{text}: {error:?}"));
            assert_eq!(cidr.to_string(), text);
        }
        for (text, network) in [
            ("192.0.2.1/24", "192.0.2.0/24"),
            ("192.0.2.129/25", "192.0.2.128/25"),
            ("10.1.2.3/8", "10.0.0.0/8"),
            ("1.2.3.4/0", "0.0.0.0/0"),
            ("2001:db8::1/32", "2001:db8::/32"),
            ("2001:db8::4000/113", "2001:db8::/113"),
        ] {
            assert_eq!(
                Cidr::parse(text),
                Err(CidrError::HostBitsSet {
                    network: Cidr::parse(network).unwrap()
                }),
                "{text}"
            );
        }
        for text in [
            "",
            "nonsense",
            "192.0.2.0",
            "192.0.2.0/",
            "/24",
            "192.0.2.0/33",
            "2001:db8::/129",
            "192.0.2.0/024",
            "192.0.2.0/00",
            "192.0.2.0/+24",
            "192.0.2.0/-1",
            "192.0.2.0/2a",
            "192.0.2.0/ 24",
            "192.0.2.0/24 ",
            " 192.0.2.0/24",
            "192.0.2.0//24",
            "192.0.2.0/24/24",
            "192.0.2.0/99999999999999999999",
            "192.000.002.000/24",
            "::ffff:192.0.2.0/120",
            "::c000:200/120",
            "::2/128",
            "2001:DB8::/32",
            "2001:0db8::/32",
            "[2001:db8::]/32",
            "api.github.com/24",
        ] {
            assert_eq!(Cidr::parse(text), Err(CidrError::Malformed), "{text:?}");
        }
    }

    #[test]
    fn a_cidr_contains_only_addresses_of_its_own_family_inside_its_prefix() {
        for (network, inside, outside) in [
            (
                "192.0.2.0/25",
                vec!["192.0.2.0", "192.0.2.10", "192.0.2.127"],
                vec![
                    "192.0.2.128",
                    "192.0.2.200",
                    "192.0.3.10",
                    "::ffff:192.0.2.10",
                    "::192.0.2.10",
                    "2001:db8::1",
                ],
            ),
            (
                "10.0.0.0/8",
                vec!["10.0.0.0", "10.255.255.255"],
                vec!["100.0.0.1", "11.0.0.0", "9.255.255.255"],
            ),
            (
                "192.0.2.7/32",
                vec!["192.0.2.7"],
                vec!["192.0.2.6", "192.0.2.70"],
            ),
            (
                "0.0.0.0/0",
                vec!["0.0.0.0", "255.255.255.255"],
                vec!["::", "::ffff:1.2.3.4"],
            ),
            (
                "::/0",
                vec!["::", "2001:db8::1", "::ffff:192.0.2.10"],
                vec!["0.0.0.0", "192.0.2.10"],
            ),
            (
                "2001:db8::/32",
                vec!["2001:db8::", "2001:db8:ffff:ffff::1"],
                vec!["2001:db9::1", "2001:db7:ffff::", "192.0.2.1"],
            ),
            (
                "2001:db8::/48",
                vec!["2001:db8::", "2001:db8:0:ffff::1"],
                vec!["2001:db8:1::", "192.0.2.1", "0.0.0.0"],
            ),
            (
                "128.0.0.0/1",
                vec!["128.0.0.0", "255.255.255.255"],
                vec!["127.255.255.255"],
            ),
            (
                "192.0.2.6/31",
                vec!["192.0.2.6", "192.0.2.7"],
                vec!["192.0.2.5", "192.0.2.8"],
            ),
            (
                "8000::/1",
                vec!["8000::", "ffff::1"],
                vec!["7fff:ffff::", "::", "128.0.0.1"],
            ),
            (
                "2001:db8::/31",
                vec!["2001:db9:ffff::"],
                vec!["2001:dba::", "2001:db7::"],
            ),
            (
                "2001:db8:0:1::/64",
                vec!["2001:db8:0:1:ffff:ffff:ffff:ffff"],
                vec!["2001:db8:0:2::", "2001:db8::ffff:ffff:ffff:ffff"],
            ),
            (
                "2001:db8::/127",
                vec!["2001:db8::", "2001:db8::1"],
                vec!["2001:db8::2"],
            ),
            (
                "2001:db8::1/128",
                vec!["2001:db8::1"],
                vec!["2001:db8::", "0.0.0.1"],
            ),
        ] {
            let cidr = Cidr::parse(network).unwrap();
            for address in inside {
                let parsed: IpAddr = address.parse().unwrap();
                assert!(cidr.contains(parsed), "{network} contains {address}");
            }
            for address in outside {
                let parsed: IpAddr = address.parse().unwrap();
                assert!(
                    !cidr.contains(parsed),
                    "{network} does not contain {address}"
                );
            }
        }
    }

    #[test]
    fn a_proxy_host_is_a_lower_case_dns_name_and_never_an_ip_literal() {
        let label = |length: usize| "a".repeat(length);
        let longest_label = label(63);
        let too_long_label = label(64);
        let longest = [label(63), label(63), label(63), label(61)].join(".");
        let too_long = [label(63), label(63), label(63), label(62)].join(".");
        for host in [
            "api.github.com",
            "a",
            "a-b.example",
            "1a.example",
            "xn--bcher-kva.example",
            longest_label.as_str(),
            longest.as_str(),
        ] {
            assert!(is_proxy_host(host), "{host}");
        }
        for host in [
            "API.github.com",
            "api.GitHub.com",
            "192.0.2.1",
            "0x7f000001",
            "1.2.3",
            "2001:db8::1",
            "-a.example",
            "a-.example",
            "a..example",
            ".a.example",
            "a.example.",
            "",
            ".",
            "a_b.example",
            "a b.example",
            "caf\u{e9}.example",
            "api.github.com\n",
            too_long_label.as_str(),
            too_long.as_str(),
        ] {
            assert!(!is_proxy_host(host), "{host:?}");
        }
    }

    #[test]
    fn provides_lists_each_binding_key_as_written_where_the_file_places_it() {
        let tools = |key: &str| format!("[provides.{key}.tools]\n\"t\" = \"Do t.\"\n");
        for (bindings, keys) in [
            (String::new(), vec![]),
            (tools("\"github:tools\""), vec!["github:tools"]),
            (
                format!(
                    "{}{}{}",
                    tools("\"z:tools\""),
                    tools("\"a:tools\""),
                    tools("\"m:tools\"")
                ),
                vec!["z:tools", "a:tools", "m:tools"],
            ),
            (
                "[provides]\n\"b:k\" = { tools = { t = \"Do t.\" } }\n\
                 \"a:k\" = { tools = { t = \"Do t.\" } }\n"
                    .to_string(),
                vec!["b:k", "a:k"],
            ),
            (
                "provides.\"b:k\".tools.t = \"Do t.\"\nprovides.\"a\\tk\".tools.t = \"Do t.\"\n"
                    .to_string(),
                vec!["b:k", "a\tk"],
            ),
            (
                format!(
                    "{}{}[provides.\"z:k\"]\n",
                    tools("\"z:k\""),
                    tools("\"a:k\"")
                ),
                vec!["a:k", "z:k"],
            ),
        ] {
            let manifest = PlasmidManifest::parse(&format!("{PROVIDER}{bindings}"))
                .unwrap_or_else(|error| panic!("{bindings}: {error}"));
            assert_eq!(manifest.provides, keys, "{bindings}");
        }
    }

    #[test]
    fn the_top_level_pins_are_cidrs() {
        let manifest = PlasmidManifest::parse(GITHUB_PR).unwrap();
        assert_eq!(
            manifest.network.unwrap().pin_cidrs,
            vec![Cidr::parse("140.82.112.0/20").unwrap()]
        );
    }

    #[test]
    fn a_canonical_dst_is_kept_exactly_as_written() {
        for dst in [
            "/srv/work.d",
            "/srv/work",
            "/workspace",
            "/a/.hidden/b..c/...",
            "/w",
        ] {
            let manifest =
                PlasmidManifest::parse(&workspace_declaration(&format!("{{ dst = \"{dst}\" }}")))
                    .unwrap_or_else(|error| panic!("{dst} was refused: {error}"));
            assert_eq!(manifest.workspace.unwrap().dst, dst);
        }
    }

    #[test]
    fn a_workspace_with_no_mount_is_refused_even_beside_another_capability() {
        let declaration = declared(&format!("{WITH_HOST}ports = [443]\n[workspace]"));
        let (_, field, fix) = field_refusal(&declaration);
        assert_eq!(
            (field.as_str(), fix.as_str()),
            ("workspace.mount", "mount = { dst = \"/workspace\" }")
        );
        let repaired = PlasmidManifest::parse(&with_fix(&declaration, &field, &fix)).unwrap();
        assert_eq!(repaired.workspace.unwrap().dst, "/workspace");
    }

    #[test]
    fn a_command_network_keeps_the_values_it_parses_today() {
        let declaration = declared(
            "[commands.commands.git]\nexec = [\"git\"]\n\
             [commands.commands.git.network]\n\
             hosts = [\"API.example\", \"192.0.2.1\", \"API.example\"]\n\
             ports = [443, 443]\npin_cidrs = [\"nonsense\", \"192.0.2.1/24\"]\n",
        );
        let manifest = PlasmidManifest::parse(&declaration).unwrap();
        let network = manifest.commands.unwrap().commands[0]
            .network
            .clone()
            .unwrap();
        assert_eq!(network.hosts, ["API.example", "192.0.2.1", "API.example"]);
        assert_eq!(network.ports, [443, 443]);
        assert_eq!(network.pin_cidrs, ["nonsense", "192.0.2.1/24"]);
        for ports in ["", "ports = []"] {
            let declaration = declared(&format!(
                "[commands.commands.git]\nexec = [\"git\"]\n\
                 [commands.commands.git.network]\nhosts = [\"alpha.ak.local\"]\n{ports}"
            ));
            let manifest = PlasmidManifest::parse(&declaration)
                .unwrap_or_else(|error| panic!("{ports}: {error}"));
            let network = manifest.commands.unwrap().commands[0]
                .network
                .clone()
                .unwrap();
            assert!(network.ports.is_empty(), "{ports}");
        }
    }

    fn refusal_detail(declaration: &str) -> String {
        let error = PlasmidManifest::parse(declaration)
            .expect_err(&format!("{declaration} must be refused"));
        let ManifestError::Field { detail, .. } = error else {
            panic!("{declaration} was not refused with its field: {error:?}");
        };
        detail
    }

    #[test]
    fn every_mock_refusal_names_each_host_it_concerns() {
        let portless = "[network]\nhosts = [\"a.example\"]\n";
        let ported = "[network]\nhosts = [\"a.example\"]\nports = [443]\n";
        let mocked = |hosts: &str| format!("[mock]\nhosts = [{hosts}]");
        for (tail, field, named, unnamed) in [
            (mocked("\"b.example\""), "mock", vec!["b.example"], vec![]),
            (
                mocked("\"b.example\", \"c.example\", \"b.example\""),
                "mock",
                vec!["b.example", "c.example"],
                vec![],
            ),
            (
                mocked("\"b.example\", \"c.example\", \"d.example\""),
                "mock",
                vec!["b.example", "c.example", "d.example"],
                vec![],
            ),
            (
                format!(
                    "{portless}{}",
                    mocked("\"a.example\", \"b.example\", \"c.example\"")
                ),
                "mock",
                vec!["b.example", "c.example"],
                vec!["a.example"],
            ),
            (
                format!(
                    "{portless}{}",
                    mocked("\"b.example\", \"a.example\", \"c.example\", \"d.example\"")
                ),
                "mock",
                vec!["b.example", "c.example", "d.example"],
                vec!["a.example"],
            ),
            (
                format!("[network]\nhosts = []\n{}", mocked("\"b.example\"")),
                "mock",
                vec!["b.example"],
                vec![],
            ),
            (
                format!("{ported}{}", mocked("\"a.example\", \"B.example\"")),
                "mock.hosts[1]",
                vec!["B.example"],
                vec!["a.example"],
            ),
            (
                format!("{ported}{}", mocked("\"192.0.2.1\"")),
                "mock.hosts[0]",
                vec!["192.0.2.1"],
                vec!["a.example"],
            ),
            (
                format!("{ported}{}", mocked("\"b.example\", \"c.example\"")),
                "network.hosts",
                vec!["b.example"],
                vec!["a.example"],
            ),
            (
                format!(
                    "[network]\nhosts = []\nports = [443]\n{}",
                    mocked("\"b.example\", \"c.example\"")
                ),
                "network.hosts",
                vec!["b.example", "c.example"],
                vec![],
            ),
        ] {
            let declaration = declared(&tail);
            assert_eq!(field_refusal(&declaration).1, field, "{tail}");
            let detail = refusal_detail(&declaration);
            for host in named {
                assert!(
                    detail.contains(&toml_string(host)),
                    "{tail}\n{detail} does not name {host}"
                );
            }
            for host in unnamed {
                assert!(
                    !detail.contains(&toml_string(host)),
                    "{tail}\n{detail} names {host}, which [network] declares"
                );
            }
        }
    }

    #[test]
    fn a_refused_value_reads_on_one_line_as_the_author_wrote_it() {
        let network = "[network]\nhosts = [\"api.github.com\"]\n";
        let rows = [
            (
                format!("{network}ports = [1979-05-27]"),
                "1979-05-27 is not an integer from 1 to 65535",
            ),
            (
                format!("{network}ports = [1e300]"),
                "1e300 is not an integer from 1 to 65535",
            ),
            (
                format!("{network}ports = 1e300"),
                "1e300 is not a list of integers from 1 to 65535",
            ),
            (
                format!("{network}ports = [nan]"),
                "nan is not an integer from 1 to 65535",
            ),
            (
                format!("{network}ports = [-inf]"),
                "-inf is not an integer from 1 to 65535",
            ),
            (
                format!("{network}ports = [\"\\t8080\\n\"]"),
                "\"\\t8080\\n\" is not an integer from 1 to 65535",
            ),
            (
                format!("{network}ports = [[443, 8080]]"),
                "[443, 8080] is not an integer from 1 to 65535",
            ),
            (
                format!("{network}ports = [{{ port = 443, scheme = \"https\" }}]"),
                "{ port = 443, scheme = \"https\" } is not an integer from 1 to 65535",
            ),
            (
                format!("{network}ports = [{{}}]"),
                "{} is not an integer from 1 to 65535",
            ),
            (
                "[network]\nhosts = [\"api.github.com\", 1979-05-27T07:32:00Z]".to_string(),
                "1979-05-27T07:32:00Z is not a string",
            ),
            (
                "[network]\nhosts = \"\\tapi.github.com\\n\"".to_string(),
                "\"\\tapi.github.com\\n\" is not a list of strings",
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\npin_cidrs = [{ \"two words\" = 1.5 }]"
                    .to_string(),
                "{ \"two words\" = 1.5 } is not a string",
            ),
            ("network = 07:32:00".to_string(), "07:32:00 is not a table"),
            (
                format!("{network}ports = [\"{ESCAPE}\"]"),
                &format!("\"{ESCAPE}\" is not an integer from 1 to 65535"),
            ),
            (
                format!("{network}ports = [\"\\b\\f\\r{DELETE}\"]"),
                &format!("\"\\b\\f\\r{DELETE}\" is not an integer from 1 to 65535"),
            ),
            (
                format!("[network]\nhosts = \"x{ESCAPE}y\""),
                &format!("\"x{ESCAPE}y\" is not a list of strings"),
            ),
            ("commands = 5".to_string(), "5 is not a table"),
            (
                "[commands.commands]\ngit = \"x\"".to_string(),
                "\"x\" is not a table",
            ),
            (
                format!("[provides.x]\ntools = [\"a{ESCAPE}b\"]"),
                &format!(
                    "tool \"a{ESCAPE}b\" needs a description; replace the names list with a table"
                ),
            ),
            (
                format!(
                    "[network]\nhosts = [\"api.github.com\"]\n[mock]\nhosts = [\"x{ESCAPE}y\"]"
                ),
                &format!(
                    "[mock] stands in for \"x{ESCAPE}y\", which [network] does not declare, \
                     and [network] declares no port"
                ),
            ),
            (
                format!(
                    "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n\
                     [mock]\nhosts = [\"x{ESCAPE}y\"]"
                ),
                &format!("[mock] stands in for \"x{ESCAPE}y\", which is not a lower-case DNS name"),
            ),
            (
                "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n\
                 [mock]\nhosts = [\"uploads.github.com\"]"
                    .to_string(),
                "[mock] stands in for \"uploads.github.com\", which [network] does not declare",
            ),
            (
                "[mock]\nhosts = [\"api.github.com\"]".to_string(),
                "[mock] stands in for \"api.github.com\", and there is no [network]",
            ),
            (
                format!("[network]\nhosts = [\"x{ESCAPE}y\"]\nports = [443]"),
                &format!("\"x{ESCAPE}y\" is not a lower-case DNS name"),
            ),
            (
                "[network]\nhosts = [\"a.example\", \"a.example\"]\nports = [443]".to_string(),
                "\"a.example\" is already declared",
            ),
            (
                "[network]\nhosts = [\"a.example\"]\nports = [443, 443]".to_string(),
                "443 is already declared",
            ),
            (
                "[network]\nhosts = [\"a.example\"]".to_string(),
                "a network section that names hosts must declare a port",
            ),
            (
                "[network]".to_string(),
                "a network section with no host and no port declares nothing",
            ),
            (
                "[network]\nhosts = [\"a.example\"]\nports = [443]\npin_cidrs = [\"192.0.2.1/24\"]"
                    .to_string(),
                "\"192.0.2.1/24\" has host bits set past its prefix",
            ),
            (
                format!(
                    "[network]\nhosts = [\"a.example\"]\nports = [443]\npin_cidrs = [\"x{ESCAPE}\"]"
                ),
                &format!("\"x{ESCAPE}\" is not a CIDR such as \"192.0.2.0/24\""),
            ),
            (
                "[workspace]".to_string(),
                "a workspace section must declare its mount",
            ),
            (
                format!("[workspace]\nmount = {{ dst = \"a{ESCAPE}\" }}"),
                &format!("\"a{ESCAPE}\" is not a canonical absolute path"),
            ),
            (
                "[workspace]\nmount = { dst = 5 }".to_string(),
                "5 is not a path string",
            ),
            (
                "[commands.commands.git]\nexec = []".to_string(),
                "[] names no program to run",
            ),
            (
                "[commands.commands.gh]\nexec = \"/opt/bin/gh\"".to_string(),
                "\"/opt/bin/gh\" is not a list of strings",
            ),
            (
                "[commands.commands.git]\nsubject = \"git\"".to_string(),
                "the command declares no exec",
            ),
            (
                "[commands]\n[commands.commands]".to_string(),
                "the commands table declares no command",
            ),
            (
                "[mock]\nhosts = []".to_string(),
                "[mock] names no host, and there is no [network]",
            ),
            (
                format!(
                    "[secrets]\nrefs = [{{ id = \"c\", consumer = \"wasm\", delivery = [\"x{ESCAPE}\"] }}]"
                ),
                &format!(
                    "invalid credential reference: unknown variant `x{ESCAPE}`, \
                     expected one of `handle`, `helper`, `inject`, `mint`"
                ),
            ),
        ];
        let details: Vec<String> = rows
            .iter()
            .map(|(tail, _)| refusal_detail(&declared(tail)))
            .collect();
        let expected: Vec<&str> = rows.iter().map(|(_, detail)| *detail).collect();
        assert_eq!(details, expected);
    }

    fn control_character_declarations() -> Vec<String> {
        vec![
            format!(
                "id = \"a\\nb{ESCAPE}c{CSI}\"\ndescription = \"Reach the declared API.\"\n\
                 [network]\nports = [443]\n"
            ),
            declared(&format!(
                "[provides.\"x{CSI}\\ny\"]\ntools = [\"a{ESCAPE}b\"]"
            )),
            declared(&format!(
                "[secrets]\nrefs = [{{ id = \"c\", consumer = \"wasm\", delivery = [\"x{ESCAPE}\\ny\"] }}]"
            )),
            declared(&format!(
                "[commands.commands.\"a\\nb{CSI}\"]\nexec = [\"git\"]\n\
                 secrets.refs = [{{ id = \"c\", consumer = \"git\" }}]"
            )),
            declared(&format!(
                "[commands.commands.\"a\\nb\"]\nexec = [\"git\"]\n\
                 [commands.commands.\"a\\nb\".network]\nhosts = [\"h{ESCAPE}\", 5]"
            )),
            format!(
                "id = \"a{}b\"\ndescription = \"Reach the declared API.\"\n[network]\nports = [443]\n",
                line_and_bidi_characters()
            ),
            declared(&format!(
                "[network]\nhosts = \"x{}y\"",
                line_and_bidi_characters()
            )),
            declared(&format!(
                "[network]\nhosts = [\"api.github.com\"]\n[mock]\nhosts = [\"m{}\"]",
                line_and_bidi_characters()
            )),
            declared(&format!(
                "[commands.commands.\"k{}\"]\nexec = [\"git\"]\nnetwork = 443",
                line_and_bidi_characters()
            )),
        ]
    }

    #[test]
    fn a_refusal_writes_no_raw_control_line_or_bidi_character_but_a_section_fix_line_break() {
        let declarations = author_refusals()
            .into_iter()
            .map(|(declaration, ..)| declaration)
            .chain(control_character_declarations());
        for declaration in declarations {
            let error = PlasmidManifest::parse(&declaration).unwrap_err();
            let ManifestError::Field { fix, .. } = &error else {
                panic!("{declaration} was not refused with its field: {error:?}");
            };
            let breaks = if fix.starts_with('[') {
                fix.matches('\n').count()
            } else {
                0
            };
            let controls: String = error
                .to_string()
                .chars()
                .filter(|character| acted_on(*character))
                .collect();
            assert_eq!(controls, "\n".repeat(breaks), "{declaration}\n{error}");
        }
    }

    #[test]
    fn a_toml_syntax_error_writes_no_raw_control_line_or_bidi_character_but_its_line_breaks() {
        for (code, escaped) in [
            (0x1B, ESCAPE),
            (0x9B, CSI),
            (0x07, BELL),
            (0x2028, LINE_SEPARATOR),
            (0x202E, RIGHT_TO_LEFT),
        ] {
            let declaration = format!("id = \"github-pr\"\nx = {}\n", raw(code));
            let error = PlasmidManifest::parse(&declaration).unwrap_err();
            assert!(matches!(error, ManifestError::Parse(_)), "{error:?}");
            let text = error.to_string();
            assert!(
                !text
                    .chars()
                    .any(|character| character != '\n' && acted_on(character)),
                "{code:#x}: {text:?}"
            );
            assert!(text.contains(escaped), "{code:#x}: {text:?}");
            assert!(text.contains('\n'), "{code:#x} keeps its layout: {text:?}");
        }
    }

    #[test]
    fn bounded_text_is_unchanged_up_to_4096_bytes_and_cut_past_them() {
        assert_eq!(bounded(""), "");
        let exact = "x".repeat(4096);
        assert_eq!(bounded(&exact), exact);
        let over = format!("{}{}", "h".repeat(2048), "t".repeat(2049));
        let cut = bounded(&over);
        assert_eq!(cut.len(), 4096);
        assert_eq!(
            cut,
            format!("{}\u{2026}{}", "h".repeat(2046), "t".repeat(2047))
        );
        for text in [
            "\u{e9}".repeat(3000),
            "\u{20ac}".repeat(2000),
            "\u{1f600}".repeat(1500),
        ] {
            let cut = bounded(&text);
            assert!(cut.len() <= 4096);
            let (head, tail) = cut.split_once('\u{2026}').unwrap();
            assert!(text.starts_with(head));
            assert!(text.ends_with(tail));
        }
        let text = format!("a{}z", "\u{1f600}".repeat(1500));
        let cut = bounded(&text);
        let (head, tail) = cut.split_once('\u{2026}').unwrap();
        assert!(text.starts_with(head));
        assert!(text.ends_with(tail));
        assert!(cut.len() <= 4096);
    }

    #[test]
    fn a_refusal_propagates_an_io_errors_formatting_failure() {
        #[derive(Debug)]
        struct Unformattable;

        impl fmt::Display for Unformattable {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                Err(fmt::Error)
            }
        }

        impl std::error::Error for Unformattable {}

        let error = ManifestError::Io(std::io::Error::other(Unformattable));
        let mut text = String::new();
        assert!(fmt::write(&mut text, format_args!("{error}")).is_err());
    }

    #[test]
    fn a_parse_refusal_of_a_mebibyte_of_nul_writes_at_most_4096_bytes() {
        let error = PlasmidManifest::parse(&"\0".repeat(MAX_MANIFEST_BYTES)).unwrap_err();
        let ManifestError::Parse(parser) = &error else {
            panic!("a syntax refusal");
        };
        let parser_text = visible_lines(&parser.to_string());
        let text = error.to_string();
        assert!(text.len() <= 4096, "{} bytes", text.len());
        assert_eq!(text.matches('\u{2026}').count(), 1);
        assert!(text.starts_with(&format!(
            "manifest toml error: {}",
            parser_text.lines().next().unwrap()
        )));
        let last_line = parser_text
            .lines()
            .rev()
            .find(|line| !line.is_empty())
            .unwrap();
        let ending = &parser_text[parser_text.rfind(last_line).unwrap()..];
        assert!(text.ends_with(ending));
    }

    #[test]
    fn a_cut_refusal_writes_no_raw_control_character() {
        let commands: Vec<String> = (0..2000)
            .map(|n| {
                let name: String = (0..4)
                    .map(|k| char::from_u32(0x80 + ((n >> (k * 5)) & 31)).unwrap())
                    .collect();
                format!("\"{name}\"")
            })
            .collect();
        for declaration in [
            "\0".repeat(MAX_MANIFEST_BYTES),
            declared(&format!("commands = [{}]", commands.join(","))),
        ] {
            let text = PlasmidManifest::parse(&declaration)
                .unwrap_err()
                .to_string();
            assert!(text.len() <= 4096);
            assert!(text.contains('\u{2026}'));
            assert!(!text.chars().any(|c| c != '\n' && acted_on(c)));
        }
    }

    #[test]
    fn a_fix_of_1024_bytes_is_shown_and_a_longer_one_is_given_by_its_length() {
        for (fix, ending) in [
            ("x".repeat(1024), format!("write {}", "x".repeat(1024))),
            (
                "x".repeat(1025),
                "its fix is 1025 bytes, too long to show here".to_string(),
            ),
            (REMOVE_ENTRY.to_string(), REMOVE_ENTRY.to_string()),
        ] {
            let error = ManifestError::Field {
                plasmid: Some("p".into()),
                field: "network".into(),
                detail: "d".into(),
                fix,
            };
            assert_eq!(
                error.to_string(),
                format!("plasmid p: network: d; {ending}")
            );
        }
    }

    #[test]
    fn a_refusal_of_130000_hosts_names_its_field_and_gives_its_fix_by_length() {
        let hosts: Vec<String> = (0..130_000)
            .map(|mut index| {
                (0..4)
                    .map(|_| {
                        let letter = char::from(b'a' + (index % 26) as u8);
                        index /= 26;
                        letter
                    })
                    .collect()
            })
            .collect();
        let entries = hosts
            .iter()
            .map(|host| toml_string(host))
            .collect::<Vec<_>>()
            .join(",");
        let error =
            PlasmidManifest::parse(&declared(&format!("network = [{entries}]"))).unwrap_err();
        let ManifestError::Field { fix, .. } = &error else {
            panic!("a field refusal");
        };
        assert_eq!(
            fix,
            &format!("[network]\n{}", strings_line("hosts", &hosts))
        );
        let text = error.to_string();
        assert!(text.len() <= 4096);
        assert!(text.starts_with("plasmid github-pr: network: ["));
        assert!(text.ends_with(&format!(
            "; its fix is {} bytes, too long to show here",
            fix.len()
        )));
    }

    #[test]
    fn every_author_refusal_is_written_whole() {
        for (declaration, ..) in author_refusals() {
            let error = PlasmidManifest::parse(&declaration).unwrap_err();
            let ManifestError::Field { fix, .. } = &error else {
                panic!("a field refusal");
            };
            let text = error.to_string();
            assert!(text.len() <= 4096, "{declaration}");
            assert!(!text.contains('\u{2026}'), "{declaration}");
            assert!(text.ends_with(&if fix == REMOVE_ENTRY {
                fix.clone()
            } else {
                format!("write {fix}")
            }));
        }
    }

    #[test]
    fn a_refusal_writes_the_plasmid_id_as_a_toml_key() {
        for (id, written) in [
            ("github-pr".to_string(), "github-pr".to_string()),
            ("two words".to_string(), "\"two words\"".to_string()),
            (format!("a{}b", raw(0x061C)), "\"a\\u061Cb\"".to_string()),
            (format!("a{}b", raw(0x200E)), "\"a\\u200Eb\"".to_string()),
            (
                format!("a\\nb{ESCAPE}c{CSI}"),
                format!("\"a\\nb{ESCAPE}c{CSI}\""),
            ),
        ] {
            let declaration = format!(
                "id = \"{id}\"\ndescription = \"Reach the declared API.\"\n[network]\nports = [443]\n"
            );
            assert_eq!(
                PlasmidManifest::parse(&declaration)
                    .unwrap_err()
                    .to_string(),
                format!(
                    "plasmid {written}: network.hosts: a network section must name at least one \
                     host; write hosts = [\"api.example.com\"]"
                )
            );
        }
    }
}
