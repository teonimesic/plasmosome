use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct PlasmidManifest {
    pub id: String,
    pub version: String,
    pub wasm: Option<PathBuf>,
    pub network: Option<NetworkSpec>,
    pub requires: Vec<String>,
    pub provides_tools: Vec<String>,
    pub secrets: Vec<SecretRef>,
    pub commands: Option<CommandsSpec>,
    pub workspace: Option<WorkspaceMount>,
    pub mock: Option<MockSpec>,
    pub model: Option<ModelSpec>,
    pub drain_ms: Option<u64>,
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
    MissingField(String),
    Invalid(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Io(e) => write!(f, "manifest io error: {e}"),
            ManifestError::Parse(e) => write!(f, "manifest toml error: {e}"),
            ManifestError::MissingField(name) => write!(f, "manifest is missing `{name}`"),
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
            .ok_or_else(|| ManifestError::MissingField("id".into()))?
            .to_string();
        if id.is_empty() {
            return Err(ManifestError::Invalid("id must not be empty".into()));
        }
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
            .and_then(toml::Value::as_table)
            .map(|table| {
                table
                    .values()
                    .filter_map(|binding| binding.get("tools"))
                    .flat_map(|tools| string_list(Some(tools)))
                    .collect()
            })
            .unwrap_or_default();
        let secrets = raw
            .get("secrets")
            .map(parse_secret_refs)
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
        validate_secret_refs(&id, &secrets)?;
        if let Some(commands) = &commands {
            validate_commands(&id, commands)?;
        }
        Ok(PlasmidManifest {
            id,
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

fn parse_secret_refs(secrets: &toml::Value) -> Result<Vec<SecretRef>, ManifestError> {
    let Some(refs) = secrets.get("refs") else {
        return Ok(Vec::new());
    };
    let items = refs
        .as_array()
        .ok_or_else(|| ManifestError::Invalid("[secrets] refs must be a list".into()))?;
    let mut parsed = Vec::new();
    for item in items {
        let mut item = item.clone();
        normalize_scope(&mut item);
        let secret: SecretRef = item.try_into().map_err(|e| {
            ManifestError::Invalid(format!("secret ref is not the frozen shape: {e}"))
        })?;
        parsed.push(secret);
    }
    Ok(parsed)
}

fn normalize_scope(item: &mut toml::Value) {
    let Some(scope) = item.get("scope") else {
        return;
    };
    if !scope.is_array() {
        return;
    }
    let paths = string_list(Some(scope));
    let mut table = toml::map::Map::new();
    table.insert(
        "path_scope".to_string(),
        toml::Value::Array(paths.into_iter().map(toml::Value::String).collect()),
    );
    if let Some(entry) = item.as_table_mut() {
        entry.insert("scope".to_string(), toml::Value::Table(table));
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
                .map(parse_secret_refs)
                .transpose()?
                .unwrap_or_default(),
        });
    }
    Ok(CommandsSpec {
        address_plan,
        commands,
    })
}

fn validate_secret_refs(id: &str, refs: &[SecretRef]) -> Result<(), ManifestError> {
    for secret in refs {
        if secret.delivery.is_empty() {
            return Err(ManifestError::Invalid(format!(
                "plasmid {id}: secret ref `{}` has an empty delivery list",
                secret.id
            )));
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
                return Err(ManifestError::Invalid(format!(
                    "plasmid {id}: secret ref `{}` pairs delivery `{}` with consumer `{}`",
                    secret.id,
                    mode.as_str(),
                    secret.consumer.as_str()
                )));
            }
            if *mode == DeliveryMode::Inject {
                let Some(scope) = &secret.scope else {
                    return Err(ManifestError::Invalid(format!(
                        "plasmid {id}: secret ref `{}` uses `inject` without a path_scope",
                        secret.id
                    )));
                };
                for prefix in &scope.path_scope {
                    if !prefix.starts_with('/') {
                        return Err(ManifestError::Invalid(format!(
                            "plasmid {id}: secret ref `{}` path_scope entry `{prefix}` is not an absolute path",
                            secret.id
                        )));
                    }
                }
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
        for secret in &decl.secrets {
            if secret.subject.is_none() && decl.subject.is_none() {
                return Err(ManifestError::Invalid(format!(
                    "plasmid {id}: [commands.{}] secret ref `{}` names no subject",
                    decl.id, secret.id
                )));
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

    const GITHUB_PR: &str = r#"
id = "github-pr"
version = "0.1.0"
impl.wasm = "components/github-pr.wasm"

[requires]
capabilities = ["network:hosts=api.github.com"]

[provides]
"github:tools" = { tools = ["pr.read", "pr.comment"] }

[network]
hosts = ["api.github.com"]
ports = [443]
pin_cidrs = ["140.82.112.0/20"]

[lifecycle]
drain_ms = 750
"#;

    const GITHUB_PR_LEGACY_STRING_REFS: &str = r#"
id = "github-pr"
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
version = "0.1.0"

[workspace]
mount = { backend = "virtiofs", dst = "/workspace" }
"#;

    const GITHUB_PR_WITH_MOCK: &str = r#"
id = "github-pr"
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
version = "0.1.0"

[mock]
hosts = ["api.github.com"]
api = "github"
backend = { kind = "recorded", source = "fixtures/github-pr" }
"#;

    const MOCK_HOSTS_DRIFTED_FROM_NETWORK: &str = r#"
id = "github-pr"
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
version = "0.1.0"

[network]
hosts = ["api.github.com"]
ports = [443]
pin_cidrs = "140.82.112.0/20"
"#;

    const NETWORK_PIN_CIDRS_MIXED_TYPES: &str = r#"
id = "github-pr"
version = "0.1.0"

[network]
hosts = ["api.github.com"]
ports = [443]
pin_cidrs = ["140.82.112.0/20", 20]
"#;

    const NETWORK_HOSTS_AS_SCALAR: &str = r#"
id = "github-pr"
version = "0.1.0"

[network]
hosts = "api.github.com"
ports = [443]
"#;

    const NETWORK_PIN_CIDRS_EMPTY: &str = r#"
id = "github-pr"
version = "0.1.0"

[network]
hosts = ["api.github.com"]
ports = [443]
pin_cidrs = []
"#;

    const COMMAND_NETWORK_HOSTS_AS_SCALAR: &str = r#"
id = "e13-commands-fixture"
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
version = "0.1.0"
network = "api.github.com"
"#;

    const COMMAND_NETWORK_SECTION_AS_SCALAR: &str = r#"
id = "e13-commands-fixture"
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
        assert_eq!(manifest.provides_tools, vec!["pr.read", "pr.comment"]);
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
    fn the_pre_freeze_string_refs_form_is_rejected_as_not_the_frozen_shape() {
        let err = PlasmidManifest::parse(GITHUB_PR_LEGACY_STRING_REFS).unwrap_err();
        assert!(matches!(err, ManifestError::Invalid(m) if m.contains("frozen shape")));
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
    fn a_command_secret_without_any_subject_is_a_named_error() {
        let text = r#"
id = "bad-commands"
[network]
hosts = ["alpha.ak.local"]
[commands.commands.gh]
exec = ["gh"]
[[commands.commands.gh.secrets.refs]]
id = "github:token:gh"
consumer = "http"
delivery = ["inject"]
scope = { path_scope = ["/api/"] }
"#;
        let err = PlasmidManifest::parse(text).unwrap_err();
        assert!(matches!(err, ManifestError::Invalid(m) if m.contains("names no subject")));
    }

    #[test]
    fn a_delivery_consumer_mismatch_is_a_named_error() {
        let text = r#"
id = "mismatched"
[network]
hosts = ["api.github.com"]
[secrets]
refs = [{ id = "t", consumer = "git", delivery = ["handle"] }]
"#;
        let err = PlasmidManifest::parse(text).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::Invalid(m) if m.contains("`handle`") && m.contains("`git`")
        ));
    }

    #[test]
    fn mint_is_a_legal_fallback_for_a_git_consumer() {
        let text = r#"
id = "mint-fallback"
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
    fn a_wasm_consumer_cannot_take_inject() {
        let text = r#"
id = "wasm-inject"
[network]
hosts = ["api.github.com"]
[secrets]
refs = [{ id = "t", consumer = "wasm", delivery = ["inject"], scope = { path_scope = ["/v1/"] } }]
"#;
        let err = PlasmidManifest::parse(text).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::Invalid(m) if m.contains("`inject`") && m.contains("`wasm`")
        ));
    }

    #[test]
    fn an_empty_delivery_list_is_a_named_error() {
        let text = r#"
id = "empty-delivery"
[network]
hosts = ["api.github.com"]
[secrets]
refs = [{ id = "t", consumer = "http", delivery = [] }]
"#;
        let err = PlasmidManifest::parse(text).unwrap_err();
        assert!(matches!(err, ManifestError::Invalid(m) if m.contains("empty delivery")));
    }

    #[test]
    fn inject_without_a_path_scope_is_a_named_error() {
        let text = r#"
id = "scopeless-inject"
[network]
hosts = ["api.github.com"]
[secrets]
refs = [{ id = "t", consumer = "http", delivery = ["inject"] }]
"#;
        let err = PlasmidManifest::parse(text).unwrap_err();
        assert!(matches!(err, ManifestError::Invalid(m) if m.contains("without a path_scope")));
    }

    #[test]
    fn a_relative_path_scope_entry_is_a_named_error() {
        let text = r#"
id = "relative-scope"
[network]
hosts = ["api.github.com"]
[secrets]
refs = [{ id = "t", consumer = "http", delivery = ["inject"], scope = { path_scope = ["repos/"] } }]
"#;
        let err = PlasmidManifest::parse(text).unwrap_err();
        assert!(matches!(err, ManifestError::Invalid(m) if m.contains("not an absolute path")));
    }

    #[test]
    fn an_unknown_delivery_mode_is_a_parse_error() {
        let text = r#"
id = "future-mode"
[network]
hosts = ["api.github.com"]
[secrets]
refs = [{ id = "t", consumer = "http", delivery = ["teleport"] }]
"#;
        let err = PlasmidManifest::parse(text).unwrap_err();
        assert!(matches!(err, ManifestError::Invalid(m) if m.contains("frozen shape")));
    }

    #[test]
    fn manifest_with_no_capability_section_is_rejected() {
        let err = PlasmidManifest::parse("id = \"empty\"").unwrap_err();
        assert!(matches!(err, ManifestError::Invalid(_)));
    }

    #[test]
    fn network_section_without_hosts_is_rejected() {
        let err =
            PlasmidManifest::parse("id = \"netless\"\n\n[network]\nports = [443]").unwrap_err();
        assert!(matches!(err, ManifestError::Invalid(_)));
    }

    #[test]
    fn missing_id_is_rejected() {
        let err = PlasmidManifest::parse("version = \"1\"").unwrap_err();
        assert!(matches!(err, ManifestError::MissingField(_)));
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
