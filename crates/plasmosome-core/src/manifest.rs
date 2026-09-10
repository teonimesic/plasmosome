use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct PlasmidManifest {
    pub id: String,
    pub description: String,
    pub version: String,
    pub wasm: Option<PathBuf>,
    pub network: Option<NetworkSpec>,
    pub requires: Vec<String>,
    pub provides_tools: Vec<ToolDeclaration>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct NetworkSpec {
    pub hosts: Vec<String>,
    pub ports: Vec<u16>,
    pub pin_cidrs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceMount {
    pub backend: String,
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
    pub network: Option<NetworkSpec>,
    pub secrets: Vec<SecretRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandsSpec {
    pub address_plan: String,
    pub commands: Vec<CommandDecl>,
}

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
    Invalid(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Io(e) => write!(f, "manifest io error: {e}"),
            ManifestError::Parse(e) => write!(f, "manifest toml error: {e}"),
            ManifestError::Field {
                plasmid,
                field,
                fix,
                detail,
            } => {
                if let Some(id) = plasmid {
                    write!(f, "plasmid {id}: ")?;
                }
                write!(f, "{field}: {detail}; write {fix}")
            }
            ManifestError::Invalid(d) => write!(f, "invalid manifest: {d}"),
        }
    }
}

impl std::error::Error for ManifestError {}

impl PlasmidManifest {
    pub fn load(path: &Path) -> Result<PlasmidManifest, ManifestError> {
        let text = std::fs::read_to_string(path).map_err(ManifestError::Io)?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<PlasmidManifest, ManifestError> {
        let raw: toml::Value = toml::from_str(text).map_err(ManifestError::Parse)?;
        let id = raw
            .get("id")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| ManifestError::Field {
                plasmid: None,
                field: "id".into(),
                fix: "id = \"choose-a-stable-id\"".into(),
                detail: "a string id is required".into(),
            })?
            .to_string();
        if id.is_empty() {
            return Err(ManifestError::Invalid("id must not be empty".into()));
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
        let network = raw
            .get("network")
            .map(|n| parse_network(&id, "network", n))
            .transpose()?;
        let requires = raw
            .get("requires")
            .map(|r| string_list(r.get("capabilities")))
            .unwrap_or_default();
        let provides_tools = raw
            .get("provides")
            .map(|provides| parse_tools(&id, provides))
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
            .and_then(|w| w.get("mount"))
            .map(|m| WorkspaceMount {
                backend: m
                    .get("backend")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("virtiofs")
                    .to_string(),
                dst: m
                    .get("dst")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("/workspace")
                    .to_string(),
            });
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
                    hosts: declared_string_list(&id, "mock", "hosts", m.get("hosts"))?,
                    kind,
                    api,
                    source,
                })
            })
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
            .map(|v| v as u64);
        if wasm.is_none()
            && network.is_none()
            && workspace.is_none()
            && mock.is_none()
            && model.is_none()
            && commands.is_none()
        {
            return Err(ManifestError::Invalid(format!(
                "plasmid {id} declares no capability and no implementation"
            )));
        }
        if let Some(spec) = &network
            && spec.hosts.is_empty()
        {
            return Err(ManifestError::Invalid(format!(
                "plasmid {id} declares [network] without hosts"
            )));
        }
        if let Some(spec) = &mock {
            validate_mock(&id, spec, network.as_ref())?;
        }
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

fn field_error(id: &str, field: String, fix: String, detail: &str) -> ManifestError {
    ManifestError::Field {
        plasmid: Some(id.to_string()),
        field,
        fix,
        detail: detail.to_string(),
    }
}

fn diagnostic_key(name: &str) -> String {
    toml_edit::Key::new(name).display_repr().into_owned()
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
            let (fix, detail) = if let Some(tool) = first_tool {
                (
                    format!(
                        "[{field}]\n{} = \"Describe what this tool does.\"",
                        diagnostic_key(tool)
                    ),
                    format!("tool {tool:?} needs a description; replace the names list with a table"),
                )
            } else {
                (
                    format!("[{field}]"),
                    "expected a tools table; an empty table declares no tools until entries are written".into(),
                )
            };
            field_error(id, field, fix, &detail)
        })?;
        for (name, description) in declarations {
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

fn parse_network(id: &str, section: &str, n: &toml::Value) -> Result<NetworkSpec, ManifestError> {
    if !n.is_table() {
        return Err(ManifestError::Invalid(format!(
            "plasmid {id}: [{section}] must be a table"
        )));
    }
    let hosts = declared_string_list(id, section, "hosts", n.get("hosts"))?;
    let ports = n
        .get("ports")
        .and_then(toml::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(toml::Value::as_integer)
                .map(|p| p as u16)
                .collect()
        })
        .unwrap_or_default();
    let pin_cidrs = declared_string_list(id, section, "pin_cidrs", n.get("pin_cidrs"))?;
    Ok(NetworkSpec {
        hosts,
        ports,
        pin_cidrs,
    })
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
                &format!("invalid credential reference: {error}"),
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
    let address_plan = raw
        .get("address_plan")
        .and_then(toml::Value::as_str)
        .unwrap_or("10.29.0.0/24")
        .to_string();
    let table = raw
        .get("commands")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| ManifestError::Invalid("[commands] must hold a commands table".into()))?;
    let mut commands = Vec::new();
    for (id, decl) in table {
        let exec = string_list(decl.get("exec"));
        if exec.is_empty() {
            return Err(ManifestError::Invalid(format!(
                "[commands.{id}] declares no exec"
            )));
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
                .map(|n| parse_network(plasmid_id, &format!("commands.{id}.network"), n))
                .transpose()?,
            secrets: decl
                .get("secrets")
                .map(|secrets| {
                    parse_secret_refs(
                        plasmid_id,
                        &format!("commands.commands.{}.secrets", diagnostic_key(id)),
                        secrets,
                    )
                })
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
        if *mode == DeliveryMode::Inject || delivery_omitted {
            let paths = secret
                .scope
                .as_ref()
                .map(|scope| scope.path_scope.as_slice())
                .unwrap_or_default();
            if (*mode == DeliveryMode::Inject && paths.is_empty())
                || paths.iter().any(|path| !path.starts_with('/'))
            {
                return Err(field_error(
                    id,
                    field("scope.path_scope"),
                    "scope = { path_scope = [\"/required/path/\"] }".into(),
                    "path-scope entries must be absolute; inject also requires a nonempty scope",
                ));
            }
        }
    }
    Ok(())
}

fn validate_mock(
    id: &str,
    mock: &MockSpec,
    network: Option<&NetworkSpec>,
) -> Result<(), ManifestError> {
    let Some(network) = network else {
        return Err(ManifestError::Invalid(format!(
            "plasmid {id} declares [mock] without the [network] hosts it stands in for"
        )));
    };
    for host in &mock.hosts {
        if !network.hosts.iter().any(|declared| declared == host) {
            return Err(ManifestError::Invalid(format!(
                "plasmid {id}: [mock] names host `{host}`, which its [network] does not declare"
            )));
        }
    }
    Ok(())
}

fn validate_commands(id: &str, commands: &CommandsSpec) -> Result<(), ManifestError> {
    for decl in &commands.commands {
        for (index, secret) in decl.secrets.iter().enumerate() {
            if secret.subject.is_none() && decl.subject.is_none() {
                return Err(field_error(
                    id,
                    format!(
                        "commands.commands.{}.secrets.refs[{index}].subject",
                        diagnostic_key(&decl.id)
                    ),
                    format!("subject = {}", toml::Value::String(decl.id.clone())),
                    "a command credential requires a ref or command subject",
                ));
            }
        }
    }
    Ok(())
}

fn declared_string_list(
    id: &str,
    section: &str,
    field: &str,
    value: Option<&toml::Value>,
) -> Result<Vec<String>, ManifestError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(ManifestError::Invalid(format!(
            "plasmid {id}: [{section}] `{field}` must be an array of strings"
        )));
    };
    items
        .iter()
        .map(|item| {
            item.as_str().map(String::from).ok_or_else(|| {
                ManifestError::Invalid(format!(
                    "plasmid {id}: [{section}] `{field}` holds `{item}`, which is not a string"
                ))
            })
        })
        .collect()
}

fn string_list(value: Option<&toml::Value>) -> Vec<String> {
    value
        .and_then(toml::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(toml::Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let base =
            "id = \"github-pr\"\ndescription = \"Read pull requests.\"\nimpl.wasm = \"pr.wasm\"\n";
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
            let fix = field_fix(&format!("{base}{declaration}"), field);
            let repair: toml::Value = fix.parse().unwrap();
            assert!(repair["provides"].is_table());
        }
        let fix = field_fix(
            &format!("{base}[provides.\"github:tools\"]\ntools = [\"pr.read\"]"),
            "provides.\"github:tools\".tools",
        );
        let repair: toml::Value = fix.parse().unwrap();
        assert!(repair["provides"]["github:tools"]["tools"]["pr.read"].is_str());
    }

    #[test]
    fn tool_description_refusals_quote_the_exact_key_and_repair_entry() {
        let base =
            "id = \"github-pr\"\ndescription = \"Read pull requests.\"\nimpl.wasm = \"pr.wasm\"\n";
        for description in ["42", "\"\"", "\" \\t \""] {
            let source = format!(
                "{base}[provides.\"git\\\"hub:tools\".tools]\n\"pr.\\\"read\" = {description}"
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
        let base =
            "id = \"github-pr\"\ndescription = \"Read pull requests.\"\nimpl.wasm = \"pr.wasm\"\n";
        for (name, quoted) in [
            ("line\nname", "\"line\\nname\""),
            ("both'\"quotes", "\"both'\\\"quotes\""),
            ("tab\tname", "\"tab\\tname\""),
            ("back\\slash", "\"back\\\\slash\""),
            ("", "\"\""),
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
                let source = format!("{base}{declaration}");
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
                    PlasmidManifest::parse(&format!("{base}{field} = {replacement}")).unwrap();
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
                        format!("{base}[provides.{quoted}.tools]\n{fix}")
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
                        format!("{base}{fix}")
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
        let base = "id = \"github-pr\"\ndescription = \"Reach GitHub.\"\n[network]\nhosts = [\"api.github.com\"]\n";
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
mount = { backend = "virtiofs", dst = "/workspace" }
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
            manifest.network.as_ref().unwrap().pin_cidrs,
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
        assert_eq!(manifest.workspace.as_ref().unwrap().backend, "virtiofs");
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

    #[test]
    fn a_manifest_whose_whole_content_is_a_mock_is_refused() {
        let err = PlasmidManifest::parse(MOCK_WITHOUT_NETWORK).unwrap_err();
        assert!(
            matches!(&err, ManifestError::Invalid(m) if m.contains("[mock]") && m.contains("[network]")),
            "a mock stands in for hosts a plasmid declares, so it is never a plasmid of its own: {err:?}"
        );
    }

    #[test]
    fn a_mock_naming_a_host_its_own_manifest_does_not_declare_is_refused() {
        let err = PlasmidManifest::parse(MOCK_HOSTS_DRIFTED_FROM_NETWORK).unwrap_err();
        assert!(
            matches!(&err, ManifestError::Invalid(m) if m.contains("api.github.example")),
            "the refusal names the host that drifted: {err:?}"
        );
    }

    #[test]
    fn a_mock_whose_hosts_is_a_bare_string_is_refused() {
        let err = PlasmidManifest::parse(MOCK_HOSTS_AS_SCALAR).unwrap_err();
        assert!(
            matches!(&err, ManifestError::Invalid(m) if m.contains("[mock]") && m.contains("hosts")),
            "a scalar `hosts` declares no host at all, and silently standing in for nothing is \
             the failure a mock exists to prevent: {err:?}"
        );
    }

    #[test]
    fn a_mock_whose_hosts_holds_a_non_string_is_refused() {
        let err = PlasmidManifest::parse(MOCK_HOSTS_MIXED_TYPES).unwrap_err();
        assert!(
            matches!(&err, ManifestError::Invalid(m) if m.contains("[mock]") && m.contains("hosts")),
            "dropping the entry that is not a string would narrow the mock without saying so: {err:?}"
        );
    }

    #[test]
    fn a_pin_declared_as_a_bare_string_must_not_parse_to_no_pins_at_all() {
        let err = PlasmidManifest::parse(NETWORK_PIN_CIDRS_AS_SCALAR).unwrap_err();
        assert!(
            matches!(&err, ManifestError::Invalid(m) if m.contains("[network]") && m.contains("pin_cidrs")),
            "a scalar pin_cidrs read as an empty list is an egress restriction that fails open \
             — the author declared a pin and nothing was pinned: {err:?}"
        );
    }

    #[test]
    fn a_pin_cidrs_holding_a_non_string_is_refused() {
        let err = PlasmidManifest::parse(NETWORK_PIN_CIDRS_MIXED_TYPES).unwrap_err();
        assert!(
            matches!(&err, ManifestError::Invalid(m) if m.contains("pin_cidrs") && m.contains("not a string")),
            "dropping the entry that is not a string would widen egress without saying so: {err:?}"
        );
    }

    #[test]
    fn a_network_hosts_declared_as_a_bare_string_is_refused_as_a_type_error_naming_the_field() {
        let err = PlasmidManifest::parse(NETWORK_HOSTS_AS_SCALAR).unwrap_err();
        assert!(
            matches!(&err, ManifestError::Invalid(m) if m.contains("hosts") && m.contains("must be an array of strings")),
            "a hosts-shaped typo is a type error, not an absence, and reporting it as absence \
             sends the author looking for a line that is already there: {err:?}"
        );
    }

    #[test]
    fn a_command_network_hosts_declared_as_a_bare_string_is_refused() {
        let err = PlasmidManifest::parse(COMMAND_NETWORK_HOSTS_AS_SCALAR).unwrap_err();
        assert!(
            matches!(&err, ManifestError::Invalid(m) if m.contains("[commands.git.network]") && m.contains("hosts")),
            "a command whose network hosts read as an empty list carries no host restriction \
             at all, and nothing behind the parser notices: {err:?}"
        );
    }

    #[test]
    fn a_command_network_section_declared_as_a_scalar_is_refused() {
        let err = PlasmidManifest::parse(COMMAND_NETWORK_SECTION_AS_SCALAR).unwrap_err();
        assert!(
            matches!(&err, ManifestError::Invalid(m) if m.contains("[commands.git.network]") && m.contains("must be a table")),
            "a network section that is not a table reads every field as absent, so the command \
             carries a network declaration that restricts nothing: {err:?}"
        );
    }

    #[test]
    fn a_network_section_declared_as_a_scalar_is_refused_as_a_type_error_not_an_absence() {
        let err = PlasmidManifest::parse(NETWORK_SECTION_AS_SCALAR).unwrap_err();
        assert!(
            matches!(&err, ManifestError::Invalid(m) if m.contains("[network]") && m.contains("must be a table")),
            "reporting a section-shaped typo as missing hosts sends the author looking for a \
             line that is already there: {err:?}"
        );
    }

    #[test]
    fn a_command_pin_cidrs_declared_as_a_bare_string_is_refused() {
        let err = PlasmidManifest::parse(COMMAND_NETWORK_PIN_CIDRS_AS_SCALAR).unwrap_err();
        assert!(
            matches!(&err, ManifestError::Invalid(m) if m.contains("[commands.git.network]") && m.contains("pin_cidrs")),
            "the fourth cell the criterion claims: a command-level pin that parses to no pins \
             at all must be refused by a test, not only by a shared helper: {err:?}"
        );
    }

    #[test]
    fn an_explicitly_empty_pin_cidrs_list_parses_and_pins_nothing() {
        let manifest = PlasmidManifest::parse(NETWORK_PIN_CIDRS_EMPTY).unwrap();
        assert!(manifest.network.as_ref().unwrap().pin_cidrs.is_empty());
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
    fn command_ref_diagnostics_repair_the_missing_subject_in_its_own_declaration() {
        let text = r#"
id = "command-credential"
description = "Run Git with its own credential."
[commands.commands."git ops"]
exec = ["git"]
[[commands.commands."git ops".secrets.refs]]
id = "token"
consumer = "git"
"#;
        let error = PlasmidManifest::parse(text).unwrap_err();
        let ManifestError::Field { field, fix, .. } = error else {
            panic!("missing command subject has no repair context: {error:?}");
        };
        assert_eq!(
            field,
            "commands.commands.\"git ops\".secrets.refs[0].subject"
        );
        let repaired = PlasmidManifest::parse(&format!("{text}\n{fix}")).unwrap();
        let command = &repaired.commands.as_ref().unwrap().commands[0];
        assert_eq!(
            command.secrets[0].subject.as_deref(),
            Some(command.id.as_str())
        );
        assert_eq!(command.secrets[0].delivery, vec![DeliveryMode::Helper]);
    }

    #[test]
    fn manifest_with_no_capability_section_is_rejected() {
        let err = PlasmidManifest::parse(
            "id = \"empty\"\ndescription = \"A declaration with no capability.\"",
        )
        .unwrap_err();
        assert!(matches!(err, ManifestError::Invalid(_)));
    }

    #[test]
    fn network_section_without_hosts_is_rejected() {
        let err = PlasmidManifest::parse(
            "id = \"netless\"\ndescription = \"Reach a network host.\"\n\n[network]\nports = [443]",
        )
        .unwrap_err();
        assert!(matches!(err, ManifestError::Invalid(_)));
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
}
