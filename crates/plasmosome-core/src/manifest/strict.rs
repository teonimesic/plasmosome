use super::*;
use toml_edit::{ImDocument, Item, TableLike};

#[derive(Clone, Copy)]
enum Shape {
    Root,
    Impl,
    Network,
    Requires,
    Bindings,
    Binding,
    Tools,
    Secrets,
    Ref,
    CommandRef,
    Scope,
    Commands,
    CommandMap,
    Command,
    Workspace,
    Mount,
    Mock,
    Backend,
    Model,
    Lifecycle,
}

impl Shape {
    fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Root => &[
                "id",
                "description",
                "version",
                "impl",
                "network",
                "requires",
                "provides",
                "secrets",
                "commands",
                "workspace",
                "mock",
                "model",
                "lifecycle",
            ],
            Self::Impl => &["wasm"],
            Self::Network => &["hosts", "ports", "pin_cidrs"],
            Self::Requires => &["capabilities"],
            Self::Binding => &["tools"],
            Self::Secrets => &["refs"],
            Self::Ref => &["id", "consumer", "delivery", "scope", "ttl"],
            Self::CommandRef => &["id", "consumer", "delivery", "scope", "ttl", "subject"],
            Self::Scope => &["path_scope", "repos", "permissions"],
            Self::Commands => &["address_plan", "commands"],
            Self::Command => &["exec", "subject", "network", "secrets"],
            Self::Workspace => &["mount"],
            Self::Mount => &["backend", "dst"],
            Self::Mock => &["hosts", "api", "backend"],
            Self::Backend => &["source", "kind"],
            Self::Model => &["endpoint", "credential"],
            Self::Lifecycle => &["drain_ms", "lease_ms", "idle_ms"],
            Self::Bindings | Self::Tools | Self::CommandMap => &[],
        }
    }

    fn child(self, key: &str) -> Option<Self> {
        match (self, key) {
            (Self::Root, "impl") => Some(Self::Impl),
            (Self::Root | Self::Command, "network") => Some(Self::Network),
            (Self::Root, "requires") => Some(Self::Requires),
            (Self::Root, "provides") => Some(Self::Bindings),
            (Self::Root | Self::Command, "secrets") => Some(Self::Secrets),
            (Self::Root, "commands") => Some(Self::Commands),
            (Self::Root, "workspace") => Some(Self::Workspace),
            (Self::Root, "mock") => Some(Self::Mock),
            (Self::Root, "model") => Some(Self::Model),
            (Self::Root, "lifecycle") => Some(Self::Lifecycle),
            (Self::Bindings, _) => Some(Self::Binding),
            (Self::Binding, "tools") => Some(Self::Tools),
            (Self::Ref | Self::CommandRef, "scope") => Some(Self::Scope),
            (Self::Commands, "commands") => Some(Self::CommandMap),
            (Self::CommandMap, _) => Some(Self::Command),
            (Self::Workspace, "mount") => Some(Self::Mount),
            (Self::Mock, "backend") => Some(Self::Backend),
            _ => None,
        }
    }

    fn string_default(self, key: &str) -> Option<&'static str> {
        match (self, key) {
            (Self::Root, "version") => Some("0.0.0"),
            (Self::Impl, "wasm") => Some("component.wasm"),
            (Self::Mount, "dst") => None,
            (Self::Mock, "api") => Some("github"),
            (Self::Backend, "source") => Some(""),
            (Self::Backend, "kind") => Some("recorded"),
            (Self::Model, "endpoint") => Some("https://api.openai.com/v1/chat/completions"),
            (Self::Model, "credential") => Some("model-provider/key"),
            (Self::Commands, "address_plan") => Some("10.29.0.0/24"),
            (Self::Command, "subject") => Some("command-name"),
            _ => None,
        }
    }

    fn table_fix(self, key: &str) -> Option<String> {
        let body = match self {
            Self::Impl => "wasm = \"component.wasm\"",
            Self::Requires => "capabilities = []",
            Self::Secrets => "refs = []",
            Self::Workspace => "mount = { dst = \"/workspace\" }",
            Self::Mount => "dst = \"/workspace\"",
            Self::Mock | Self::Model | Self::Lifecycle => "",
            Self::Backend => "kind = \"recorded\", source = \"\"",
            _ => return None,
        };
        Some(
            if matches!(self, Self::Mount | Self::Backend) || key == "secrets" {
                format!("{key} = {{ {body} }}")
            } else {
                format!("[{key}]\n{body}")
            },
        )
    }
}

pub(super) fn check(id: &str, text: &str, raw: &toml::Value) -> Result<(), ManifestError> {
    blank_keys(id, raw)?;
    let document = ImDocument::parse(text).expect("the declaration already parsed as TOML");
    walk(id, text, document.as_table(), "", Shape::Root)
}

fn blank_keys(id: &str, raw: &toml::Value) -> Result<(), ManifestError> {
    if let Some(bindings) = raw.get("provides").and_then(toml::Value::as_table) {
        for (binding, value) in bindings {
            let path = format!("provides.{}", diagnostic_key(binding));
            if binding.trim().is_empty() {
                return Err(field_error(
                    id,
                    path,
                    REMOVE_ENTRY.into(),
                    "a capability key must not be empty or only whitespace",
                ));
            }
            if let Some(tools) = value.get("tools").and_then(toml::Value::as_table) {
                for name in tools.keys() {
                    if name.trim().is_empty() {
                        return Err(field_error(
                            id,
                            format!("{path}.tools.{}", diagnostic_key(name)),
                            REMOVE_ENTRY.into(),
                            BLANK_TOOL,
                        ));
                    }
                }
            }
            if value
                .get("tools")
                .and_then(toml::Value::as_array)
                .and_then(|items| items.first())
                .and_then(toml::Value::as_str)
                .is_some_and(|name| name.trim().is_empty())
            {
                return Err(field_error(
                    id,
                    format!("{path}.tools[0]"),
                    REMOVE_ENTRY.into(),
                    BLANK_TOOL,
                ));
            }
        }
    }
    Ok(())
}

fn walk(
    id: &str,
    text: &str,
    table: &dyn TableLike,
    path: &str,
    shape: Shape,
) -> Result<(), ManifestError> {
    let allowed: HashSet<&str> = shape.keys().iter().copied().collect();
    let dynamic = matches!(shape, Shape::Bindings | Shape::Tools | Shape::CommandMap);
    for (key, item) in table.iter() {
        let field = if path.is_empty() {
            diagnostic_key(key)
        } else {
            format!("{path}.{}", diagnostic_key(key))
        };
        if !dynamic && !allowed.contains(key) {
            let subject = matches!(shape, Shape::Ref) && key == "subject";
            let fix = if subject {
                REMOVE_ENTRY.into()
            } else {
                commented(text, table, key, item)
            };
            let detail = if subject {
                "spec 001 gives subject to command refs only; a command credential belongs under commands.commands.<name>.secrets".to_string()
            } else {
                format!(
                    "unknown field; known fields are {}",
                    shape.keys().join(", ")
                )
            };
            return Err(field_error(id, field, fix, &detail));
        }
        if let Some(default) = shape.string_default(key)
            && item.as_str().is_none()
        {
            return Err(field_error(
                id,
                field,
                format!("{} = {}", diagnostic_key(key), toml_string(default)),
                "expected a string",
            ));
        }
        if matches!(shape, Shape::Mount) && key == "backend" && item.as_str().is_none() {
            return Err(field_error(
                id,
                field,
                REMOVE_ENTRY.into(),
                "backend must be a string and selects nothing",
            ));
        }
        if matches!(shape, Shape::Lifecycle) {
            let value = item.as_integer().and_then(|n| u64::try_from(n).ok());
            let valid = value.is_some_and(|n| key != "drain_ms" || (1..=30_000).contains(&n));
            if !valid {
                let example = if key == "drain_ms" { 750 } else { 1000 };
                return Err(field_error(
                    id,
                    field,
                    format!("{key} = {example}"),
                    "expected an integer fitting u64; drain_ms must be from 1 to 30000",
                ));
            }
        }
        if matches!(shape, Shape::Secrets) && key == "refs" {
            let ref_shape = if path == "secrets" {
                Shape::Ref
            } else {
                Shape::CommandRef
            };
            if let Some(items) = item.as_array() {
                for (index, value) in items.iter().enumerate() {
                    if let Some(reference) = value.as_inline_table() {
                        walk(id, text, reference, &format!("{field}[{index}]"), ref_shape)?;
                    }
                }
            } else if let Some(items) = item.as_array_of_tables() {
                for (index, reference) in items.iter().enumerate() {
                    walk(id, text, reference, &format!("{field}[{index}]"), ref_shape)?;
                }
            }
        }
        if let Some(child) = shape.child(key) {
            if let Some(table) = item.as_table_like() {
                walk(id, text, table, &field, child)?;
            } else if (!matches!(child, Shape::Scope) || item.as_array().is_none())
                && let Some(mut fix) = child.table_fix(key)
            {
                if matches!(child, Shape::Mock) && table.get("network").is_none() {
                    fix = REMOVE_ENTRY.into();
                }
                return Err(field_error(id, field, fix, "expected a table"));
            }
        }
    }
    Ok(())
}

fn commented(text: &str, table: &dyn TableLike, key: &str, item: &Item) -> String {
    let key_span = table.get_key_value(key).and_then(|(key, _)| key.span());
    let Some(span) = key_span else {
        return format!("# {} = {}", diagnostic_key(key), item);
    };
    let mut start = span.start;
    let end = item.span().map_or(span.end, |span| span.end);
    if item.is_table() || item.is_array_of_tables() {
        start = text[..start].rfind('\n').map_or(0, |n| n + 1);
    }
    text[start..end]
        .lines()
        .map(|line| format!("# {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
