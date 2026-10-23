use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use plasmosome_backend::{
    FileAccess, MockMode, ProxyRecipe, ProxyTransport, RecipeError, canonical_path, is_proxy_host,
};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::manifest::acted_on;
use crate::runtime::strict_json::{StrictJsonError, parse_value};

#[cfg(test)]
#[path = "../../tests/fuzz/operator_file.rs"]
mod operator_file;
#[cfg(test)]
#[path = "../../tests/fuzz/replay.rs"]
mod replay;

const MAX_SHOWN_CHARS: usize = 64;

static ABSENT: Value = Value::Null;

const PROXY_MAP_KEYS: &[&str] = &[
    "host",
    "port",
    "modes",
    "route",
    "transport",
    "destination",
    "allow_private",
];

/// Spec 022 §4's operator declarations file, read strictly: each plasmid ID with its entry, in
/// key order. A value from [`OperatorFile::parse`] has passed every content rule of §3 rule 6
/// and §4. Whether a listed declaration's `id` equals its key is not checked here: §5 checks it
/// when the declaration is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OperatorFile {
    plasmids: BTreeMap<String, OperatorEntry>,
}

/// One plasmid's entry: the canonical path of its declaration, when the file lists one for local
/// selection, and its ProxyMap and Mount entries in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OperatorEntry {
    pub(crate) declaration: Option<PathBuf>,
    pub(crate) proxy_maps: Vec<ProxyEntry>,
    pub(crate) mounts: Vec<MountEntry>,
}

/// One ProxyMap entry: a lower-case DNS host, the modes it serves with no repeat, a nonempty
/// NUL-free route label, and a recipe `ProxyRecipe::validate` accepts. No other ProxyMap entry of
/// the same plasmid has this host, port and transport with one of these modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProxyEntry {
    pub(crate) host: String,
    pub(crate) modes: Vec<MockMode>,
    pub(crate) route: String,
    pub(crate) recipe: ProxyRecipe,
}

/// One Mount entry: a canonical guest `target`, a canonical host `source` and the guest's
/// access. No other Mount entry of the same plasmid has this `target`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MountEntry {
    pub(crate) target: String,
    pub(crate) source: PathBuf,
    pub(crate) access: FileAccess,
}

/// Why the operator file's content is refused, which spec 022 §3 makes a 108 with `detail`
/// `content`. `at` is the dotted location of the first fault, such as
/// `plasmids.github-pr.proxy_maps[1].modes`, and is empty when the whole file is at fault: not
/// UTF-8, not one JSON value, or not an object. `reason` reads after `at`, so show the refusal
/// through `Display`, which joins the two and writes each control, line-break or bidi character
/// as a `\uXXXX` escape. `at` and `reason` keep the raw text; a caller that prints `at` alone
/// passes it through spec 025's output formatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContentRefusal {
    pub(crate) at: String,
    pub(crate) reason: String,
}

impl fmt::Display for ContentRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, text) in [&self.at, &self.reason].into_iter().enumerate() {
            if index == 1 && !self.at.is_empty() {
                f.write_str(": ")?;
            }
            for character in text.chars() {
                if acted_on(character) {
                    write!(f, "\\u{:04X}", u32::from(character))?;
                } else {
                    write!(f, "{character}")?;
                }
            }
        }
        Ok(())
    }
}

impl std::error::Error for ContentRefusal {}

impl OperatorFile {
    /// The file the controller acts as if it read when `operator_declarations` is unset (spec
    /// 022 §1): no plasmid is listed and nothing is covered.
    pub(crate) fn empty() -> OperatorFile {
        OperatorFile {
            plasmids: BTreeMap::new(),
        }
    }

    /// Reads `bytes`, the operator file as `TrustedFile` read it, under spec 022 §3 rule 6 and
    /// §4, and refuses the whole file at its first fault, so no caller acts on part of it.
    ///
    /// The order: UTF-8; one JSON value with no key repeated at any depth; an object with exactly
    /// `version`, the JSON integer 1, and `plasmids`, an object; then each plasmid ID in key
    /// order: nonempty and NUL-free, with an entry holding exactly `proxy_maps` and `mounts` and
    /// an optional canonical `declaration`; each ProxyMap entry with exactly its seven keys, a
    /// host `is_proxy_host` accepts, an integer port up to 65535, nonempty modes with no repeat,
    /// a nonempty NUL-free route, `tcp` or `udp`, a string destination and a boolean
    /// `allow_private`, and then `ProxyRecipe::validate`, which also refuses port 0; each Mount
    /// entry with exactly a canonical `target`, a canonical `source` and `access`; and last the
    /// entry's overlap rules. At each object an unknown key, the smallest named, refuses before
    /// a missing one. A path that is not canonical is refused, never rewritten. `lifetime` (spec
    /// 028) is not read yet, so an entry naming it is refused as an unknown key. The size is not
    /// checked here: `TrustedFile` refuses an oversized file as `size`.
    pub(crate) fn parse(bytes: &[u8]) -> Result<OperatorFile, ContentRefusal> {
        let text = std::str::from_utf8(bytes).map_err(|error| {
            refuse(
                "",
                format!("the file is not UTF-8 from byte {}", error.valid_up_to()),
            )
        })?;
        let value = parse_value(text.as_bytes()).map_err(not_json)?;
        let file = Fields::of(
            &value,
            "",
            "the operator file",
            &["version", "plasmids"],
            &[],
        )?;
        if file.value("version").as_u64() != Some(1) {
            return Err(file.wrong("version", "must be the integer 1"));
        }
        let Value::Object(listed) = file.value("plasmids") else {
            return Err(file.wrong("plasmids", "must be an object from plasmid IDs to entries"));
        };
        let mut plasmids = BTreeMap::new();
        for (id, entry) in listed {
            plasmids.insert(id.clone(), plasmid_entry(id, entry)?);
        }
        Ok(OperatorFile { plasmids })
    }

    /// The entry for plasmid `id`, or `None` when the file lists no such plasmid.
    pub(crate) fn entry(&self, id: &str) -> Option<&OperatorEntry> {
        self.plasmids.get(id)
    }

    /// Each plasmid ID whose entry names a declaration, with that declaration's path, in key
    /// order. An entry with no `declaration` is left out.
    pub(crate) fn listed(&self) -> impl Iterator<Item = (&str, &Path)> {
        self.plasmids.iter().filter_map(|(id, entry)| {
            let declaration = entry.declaration.as_deref()?;
            Some((id.as_str(), declaration))
        })
    }

    /// Every Mount entry's plasmid ID and source, in key order and then file order: the sources
    /// the file declares, for spec 022 §3 rule 5.
    pub(crate) fn mount_sources(&self) -> impl Iterator<Item = (&str, &Path)> {
        self.plasmids.iter().flat_map(|(id, entry)| {
            let sources = entry.mounts.iter().map(|mount| mount.source.as_path());
            sources.map(move |source| (id.as_str(), source))
        })
    }
}

fn plasmid_entry(id: &str, value: &Value) -> Result<OperatorEntry, ContentRefusal> {
    let at = format!("plasmids.{id}");
    if id.is_empty() || id.contains('\0') {
        return Err(refuse(&at, "a plasmid ID must be nonempty and NUL-free"));
    }
    let entry = Fields::of(
        value,
        &at,
        "a plasmid entry",
        &["proxy_maps", "mounts"],
        &["declaration"],
    )?;
    let declaration = entry.fields.contains_key("declaration");
    let declaration = declaration.then(|| entry.path("declaration")).transpose()?;
    let proxy_maps = entry.each("proxy_maps", proxy_entry)?;
    let mounts = entry.each("mounts", mount_entry)?;
    no_shared_mode(&proxy_maps, &at)?;
    no_shared_target(&mounts, &at)?;
    Ok(OperatorEntry {
        declaration: declaration.map(PathBuf::from),
        proxy_maps,
        mounts,
    })
}

fn proxy_entry(value: &Value, at: &str) -> Result<ProxyEntry, ContentRefusal> {
    let proxy = Fields::of(value, at, "a ProxyMap entry", PROXY_MAP_KEYS, &[])?;
    let host = proxy.string("host")?;
    if !is_proxy_host(host) {
        return Err(proxy.wrong("host", "must be a lower-case DNS name"));
    }
    let port = proxy
        .value("port")
        .as_u64()
        .and_then(|port| u16::try_from(port).ok());
    let port = port.ok_or_else(|| proxy.wrong("port", "must be an integer from 1 to 65535"))?;
    let modes = modes(proxy.value("modes"), &proxy.at("modes"))?;
    let route = proxy.string("route")?;
    if route.is_empty() || route.contains('\0') {
        return Err(proxy.wrong("route", "must be a nonempty label with no NUL"));
    }
    let transport = ProxyTransport::deserialize(proxy.value("transport"))
        .map_err(|_| proxy.wrong("transport", "must be \"tcp\" or \"udp\""))?;
    let destination = proxy.string("destination")?.to_owned();
    let allow_private = proxy.value("allow_private").as_bool();
    let allow_private =
        allow_private.ok_or_else(|| proxy.wrong("allow_private", "must be true or false"))?;
    let recipe = ProxyRecipe {
        transport,
        destination,
        port,
        allow_private,
    };
    recipe.validate().map_err(|error| {
        let field = match error {
            RecipeError::ZeroPort => "port",
            _ => "destination",
        };
        refuse(&proxy.at(field), error.to_string())
    })?;
    Ok(ProxyEntry {
        host: host.to_owned(),
        modes,
        route: route.to_owned(),
        recipe,
    })
}

fn modes(value: &Value, at: &str) -> Result<Vec<MockMode>, ContentRefusal> {
    let names = array(value, at)?;
    if names.is_empty() {
        return Err(refuse(at, "must name at least one mode"));
    }
    let mut modes = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let at = format!("{at}[{index}]");
        let mode = name.as_str().and_then(MockMode::parse).ok_or_else(|| {
            let rule = "must be \"simulate\", \"capture\" or \"passthrough\"";
            refuse(&at, format!("{rule}, not {}", shown(name)))
        })?;
        if modes.contains(&mode) {
            return Err(refuse(&at, format!("repeats the mode {}", mode.as_str())));
        }
        modes.push(mode);
    }
    Ok(modes)
}

fn mount_entry(value: &Value, at: &str) -> Result<MountEntry, ContentRefusal> {
    let mount = Fields::of(
        value,
        at,
        "a Mount entry",
        &["target", "source", "access"],
        &[],
    )?;
    Ok(MountEntry {
        target: mount.path("target")?.to_owned(),
        source: PathBuf::from(mount.path("source")?),
        access: FileAccess::deserialize(mount.value("access"))
            .map_err(|_| mount.wrong("access", "must be \"read_only\" or \"read_write\""))?,
    })
}

fn no_shared_mode(entries: &[ProxyEntry], at: &str) -> Result<(), ContentRefusal> {
    let mut served = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        for mode in &entry.modes {
            let recipe = &entry.recipe;
            let slot = (
                entry.host.as_str(),
                recipe.port,
                recipe.transport,
                mode.as_str(),
            );
            if let Some(earlier) = served.insert(slot, index) {
                return Err(refuse(
                    &format!("{at}.proxy_maps[{index}].modes"),
                    format!(
                        "shares the mode {} with proxy_maps[{earlier}], which has the same host, port and transport",
                        mode.as_str()
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn no_shared_target(mounts: &[MountEntry], at: &str) -> Result<(), ContentRefusal> {
    let mut targets = BTreeMap::new();
    for (index, mount) in mounts.iter().enumerate() {
        if let Some(earlier) = targets.insert(mount.target.as_str(), index) {
            return Err(refuse(
                &format!("{at}.mounts[{index}].target"),
                format!("is also the target of mounts[{earlier}]"),
            ));
        }
    }
    Ok(())
}

struct Fields<'a> {
    fields: &'a Map<String, Value>,
    at: &'a str,
}

impl<'a> Fields<'a> {
    fn of(
        value: &'a Value,
        at: &'a str,
        name: &str,
        required: &[&str],
        optional: &[&str],
    ) -> Result<Fields<'a>, ContentRefusal> {
        let Value::Object(fields) = value else {
            return Err(refuse(at, format!("{name} must be a JSON object")));
        };
        let object = Fields { fields, at };
        let known = |key: &str| required.contains(&key) || optional.contains(&key);
        if let Some(unknown) = fields.keys().filter(|key| !known(key.as_str())).min() {
            return Err(refuse(
                &object.at(unknown),
                format!("is not a field of {name}"),
            ));
        }
        if let Some(missing) = required.iter().find(|key| !fields.contains_key(**key)) {
            return Err(refuse(
                &object.at(missing),
                format!("is required in {name}"),
            ));
        }
        Ok(object)
    }

    fn at(&self, key: &str) -> String {
        located(self.at, key)
    }

    fn value(&self, key: &str) -> &'a Value {
        self.fields.get(key).unwrap_or(&ABSENT)
    }

    fn wrong(&self, key: &str, rule: &str) -> ContentRefusal {
        refuse(
            &self.at(key),
            format!("{rule}, not {}", shown(self.value(key))),
        )
    }

    fn string(&self, key: &str) -> Result<&'a str, ContentRefusal> {
        let text = self.value(key).as_str();
        text.ok_or_else(|| self.wrong(key, "must be a string"))
    }

    fn path(&self, key: &'static str) -> Result<&'a str, ContentRefusal> {
        let text = self.string(key)?;
        canonical_path(key, text).map_err(|error| refuse(&self.at(key), error.to_string()))?;
        Ok(text)
    }

    fn each<T>(
        &self,
        key: &str,
        read: fn(&Value, &str) -> Result<T, ContentRefusal>,
    ) -> Result<Vec<T>, ContentRefusal> {
        let at = self.at(key);
        let items = array(self.value(key), &at)?.iter().enumerate();
        items
            .map(|(index, item)| read(item, &format!("{at}[{index}]")))
            .collect()
    }
}

fn located(at: &str, key: &str) -> String {
    match at {
        "" => key.to_owned(),
        at => format!("{at}.{key}"),
    }
}

fn array<'a>(value: &'a Value, at: &str) -> Result<&'a [Value], ContentRefusal> {
    match value {
        Value::Array(items) => Ok(items),
        other => Err(refuse(
            at,
            format!("must be an array, not {}", shown(other)),
        )),
    }
}

fn shown(value: &Value) -> String {
    match value {
        Value::String(text) if text.chars().count() > MAX_SHOWN_CHARS => {
            format!("a string of {} bytes", text.len())
        }
        Value::Array(_) => "an array".to_owned(),
        Value::Object(_) => "an object".to_owned(),
        scalar => scalar.to_string(),
    }
}

fn not_json(error: StrictJsonError) -> ContentRefusal {
    match error {
        StrictJsonError::NotJson {
            line,
            column,
            reason,
        } => refuse(
            "",
            format!("the file is not one JSON value at line {line}, column {column}: {reason}"),
        ),
        StrictJsonError::DuplicateKey { field, .. } => {
            refuse(&field, "appears twice in one object")
        }
    }
}

fn refuse(at: &str, reason: impl Into<String>) -> ContentRefusal {
    ContentRefusal {
        at: at.to_owned(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::*;

    const SPEC_022: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/specs/022-trusted-operator-declarations.md"
    );
    const PROXY: &str = "/plasmids/github-pr/proxy_maps/0";
    const MOUNT: &str = "/plasmids/workspace/mounts/0";
    const GITHUB_DECLARATION: &str = "/Users/operator/plasmids/github-pr/plasmid.toml";
    const WORKSPACE_DECLARATION: &str = "/Users/operator/plasmids/workspace/plasmid.toml";

    const REPEATS: &str = r#"
        "version": 1 => version
        "mounts": [] => plasmids.github-pr.mounts
        "route": "github-api" => plasmids.github-pr.proxy_maps[0].route
        "access": "read_write" => plasmids.workspace.mounts[0].access
    "#;

    const MISSING: &str = "
        /version => version
        /plasmids => plasmids
        /plasmids/github-pr/proxy_maps => plasmids.github-pr.proxy_maps
        /plasmids/workspace/mounts => plasmids.workspace.mounts
        /plasmids/workspace/mounts/0/target => plasmids.workspace.mounts[0].target
        /plasmids/workspace/mounts/0/source => plasmids.workspace.mounts[0].source
        /plasmids/workspace/mounts/0/access => plasmids.workspace.mounts[0].access
    ";

    const TOP: &str = r#"
        version 2 => must be the integer 1, not 2
        version 1.0 => must be the integer 1, not 1.0
        version "1" => must be the integer 1, not "1"
        plasmids [] => must be an object from plasmid IDs to entries, not an array
        plasmids.github-pr [] => a plasmid entry must be a JSON object
        plasmids.workspace.mounts[0] [1] => a Mount entry must be a JSON object
    "#;

    const ENTRY: &str = r#"
        declaration 1 => must be a string, not 1
        declaration "plasmids/p.toml" => `declaration` must be an absolute path
        declaration "/" => `declaration` has an empty, `.` or `..` component or a trailing `/`
        declaration "/Users/operator/./p.toml" => `declaration` has an empty, `.` or `..` component
        proxy_maps {} => must be an array, not an object
        proxy_maps[0] ["x"] => a ProxyMap entry must be a JSON object
    "#;

    const PROXY_FIELDS: &str = r#"
        host "API.github.com" => must be a lower-case DNS name, not "API.github.com"
        host "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" => not a string of 75 bytes
        port 0 => `port` must not be 0
        port 65536 => must be an integer from 1 to 65535, not 65536
        port "443" => must be an integer from 1 to 65535, not "443"
        modes [] => must name at least one mode
        modes[0] ["Simulate"] => "capture" or "passthrough", not "Simulate"
        modes[0] [["simulate"]] => "capture" or "passthrough", not an array
        modes[1] ["capture", "capture"] => repeats the mode capture
        route "" => must be a nonempty label with no NUL
        transport "quic" => must be "tcp" or "udp", not "quic"
        destination null => must be a string, not null
        allow_private "false" => must be true or false, not "false"
        destination "::192.0.2.10" => must spell the IPv4 address 192.0.2.10 as IPv4
        destination "::ffff:192.0.2.10" => must spell the IPv4 address 192.0.2.10 as IPv4
        destination "192.000.002.010" => must be one DNS name or IP address
    "#;

    const MOUNT_FIELDS: &str = r#"
        target "workspace" => `target` must be an absolute path
        target "/workspace/../etc" => `target` has an empty, `.` or `..` component
        target "/work//space" => `target` has an empty, `.` or `..` component
        source "/Users/operator/src/../project" => `source` has an empty, `.` or `..` component
        source "/Users/operator/src/project/" => `source` has an empty, `.` or `..` component
        source "/" => `source` has an empty, `.` or `..` component or a trailing `/`
        access "rw" => must be "read_only" or "read_write", not "rw"
    "#;

    fn spec_example() -> String {
        let spec = fs::read_to_string(SPEC_022).expect("spec 022 is readable beside the crate");
        let (_, shape) = spec
            .split_once("### 4. The file's shape")
            .expect("section 4");
        let (_, block) = shape.split_once("```json\n").expect("a JSON block");
        let (example, _) = block.split_once("```").expect("the JSON block ends");
        example.to_owned()
    }

    fn example() -> Value {
        serde_json::from_str(&spec_example()).expect("spec 022's example is JSON")
    }

    fn encoded(file: &Value) -> Vec<u8> {
        serde_json::to_vec(file).expect("a JSON value encodes")
    }

    fn edited(edit: impl FnOnce(&mut Value)) -> Vec<u8> {
        let mut file = example();
        edit(&mut file);
        encoded(&file)
    }

    fn set(pointer: &str, value: Value) -> Vec<u8> {
        edited(|file| *file.pointer_mut(pointer).expect("a pointer") = value)
    }

    fn object<'a>(file: &'a mut Value, pointer: &str) -> &'a mut Map<String, Value> {
        let object = file.pointer_mut(pointer).and_then(Value::as_object_mut);
        object.expect("a pointer to an object in the example")
    }

    fn with(pointer: &str, key: &str, value: Value) -> Vec<u8> {
        edited(|file| drop(object(file, pointer).insert(key.to_owned(), value)))
    }

    fn without(pointer: &str) -> Vec<u8> {
        let (parent, key) = pointer.rsplit_once('/').expect("a pointer below the root");
        edited(|file| drop(object(file, parent).remove(key).expect("a key")))
    }

    fn plasmids(entries: Value) -> Vec<u8> {
        encoded(&json!({"version": 1, "plasmids": entries}))
    }

    fn entry(proxy_maps: Value, mounts: Value) -> Value {
        json!({"proxy_maps": proxy_maps, "mounts": mounts})
    }

    fn proxy_map(host: &str, port: u16, modes: &[&str], transport: &str) -> Value {
        json!({"host": host, "port": port, "modes": modes, "route": "r", "transport": transport,
               "destination": "192.0.2.10", "allow_private": false})
    }

    fn mount(target: &str, source: &str) -> Value {
        json!({"target": target, "source": source, "access": "read_only"})
    }

    fn parsed(bytes: &[u8]) -> OperatorFile {
        OperatorFile::parse(bytes).unwrap_or_else(|refusal| panic!("refused: {refusal}"))
    }

    fn refused(bytes: &[u8]) -> ContentRefusal {
        OperatorFile::parse(bytes).expect_err("the file is refused")
    }

    fn nul() -> char {
        char::from(0)
    }

    struct Row {
        case: String,
        bytes: Vec<u8>,
        at: String,
        reason: String,
    }

    fn row(case: &str, bytes: Vec<u8>, at: &str, reason: &str) -> Row {
        let (case, at, reason) = (case.to_owned(), at.to_owned(), reason.to_owned());
        Row {
            case,
            bytes,
            at,
            reason,
        }
    }

    fn lines(table: &str) -> impl Iterator<Item = (&str, &str)> {
        let rows = table.lines().map(str::trim).filter(|line| !line.is_empty());
        rows.map(|line| line.split_once(" => ").expect("a row holds =>"))
    }

    fn edits(pointer: &str, at: &str, table: &str) -> Vec<Row> {
        let edit = |(change, reason): (&str, &str)| {
            let (key, value) = change.split_once(' ').expect("a key, then a JSON value");
            let value = serde_json::from_str(value).expect("a JSON value");
            let field = key.split('[').next().expect("a field name");
            let pointer = format!("{pointer}/{}", field.replace('.', "/"));
            row(change, set(&pointer, value), &located(at, key), reason)
        };
        lines(table).map(edit).collect()
    }

    fn content_rows() -> Vec<Row> {
        let not_utf8 = b"{\"version\": 1, \"plasmids\": {\"\xff\": {}}}".to_vec();
        let bad = not_utf8.iter().position(|byte| *byte == 0xff);
        let not_utf8_from = format!("not UTF-8 from byte {}", bad.expect("a bad byte"));
        let nul_id = format!("a{}b", nul());
        let empty = entry(json!([]), json!([]));
        let id_rule = "a plasmid ID must be nonempty and NUL-free";
        let mut rows = vec![
            row("not UTF-8", not_utf8, "", &not_utf8_from),
            row(
                "not JSON",
                b"{\"version\": 1,".to_vec(),
                "",
                "at line 1, column 14",
            ),
            row("an array", b"[]".to_vec(), "", "the operator file must be"),
            row(
                "an empty ID",
                with("/plasmids", "", empty.clone()),
                "plasmids.",
                id_rule,
            ),
            row(
                "NUL ID",
                with("/plasmids", &nul_id, empty),
                &located("plasmids", &nul_id),
                id_rule,
            ),
            row(
                "a route with NUL",
                set(&format!("{PROXY}/route"), json!(format!("r{}", nul()))),
                "plasmids.github-pr.proxy_maps[0].route",
                "must be a nonempty label with no NUL",
            ),
            row(
                "a source with NUL",
                set(
                    &format!("{MOUNT}/source"),
                    json!(format!("/src/a{}b", nul())),
                ),
                "plasmids.workspace.mounts[0].source",
                "`source` contains a NUL byte",
            ),
        ];
        let example = spec_example();
        for (text, at) in lines(REPEATS) {
            let bytes = example
                .replacen(text, &format!("{text}, {text}"), 1)
                .into_bytes();
            rows.push(row(text, bytes, at, "appears twice in one object"));
        }
        for (pointer, at) in lines(MISSING) {
            rows.push(row(pointer, without(pointer), at, "is required in"));
        }
        for key in PROXY_MAP_KEYS {
            let at = format!("plasmids.github-pr.proxy_maps[0].{key}");
            let bytes = without(&format!("{PROXY}/{key}"));
            rows.push(row(&at, bytes, &at, "is required in a ProxyMap entry"));
        }
        for (pointer, at, name) in [
            ("", "", "the operator file"),
            (
                "/plasmids/github-pr",
                "plasmids.github-pr",
                "a plasmid entry",
            ),
            (
                PROXY,
                "plasmids.github-pr.proxy_maps[0]",
                "a ProxyMap entry",
            ),
            (MOUNT, "plasmids.workspace.mounts[0]", "a Mount entry"),
        ] {
            let bytes = with(pointer, "x", json!(1));
            let reason = format!("is not a field of {name}");
            rows.push(row(&reason, bytes, &located(at, "x"), &reason));
        }
        rows.extend(edits("", "", TOP));
        rows.extend(edits("/plasmids/github-pr", "plasmids.github-pr", ENTRY));
        rows.extend(edits(
            PROXY,
            "plasmids.github-pr.proxy_maps[0]",
            PROXY_FIELDS,
        ));
        rows.extend(edits(MOUNT, "plasmids.workspace.mounts[0]", MOUNT_FIELDS));
        rows
    }

    #[test]
    fn content_refusals_name_the_location_and_the_reason() {
        let mut wrong = Vec::new();
        for row in content_rows() {
            match OperatorFile::parse(&row.bytes) {
                Err(refusal) if refusal.at == row.at && refusal.reason.contains(&row.reason) => {}
                other => wrong.push(format!(
                    "{}: wanted {:?} with a reason holding {:?}, got {other:?}",
                    row.case, row.at, row.reason
                )),
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    fn proxy(mode: MockMode, route: &str, destination: &str, allow_private: bool) -> ProxyEntry {
        let recipe = ProxyRecipe {
            transport: ProxyTransport::Tcp,
            destination: destination.to_owned(),
            port: 443,
            allow_private,
        };
        let (host, modes, route) = ("api.github.com".to_owned(), vec![mode], route.to_owned());
        ProxyEntry {
            host,
            modes,
            route,
            recipe,
        }
    }

    #[test]
    fn parses_the_spec_example() {
        let file = parsed(spec_example().as_bytes());
        let github = OperatorEntry {
            declaration: Some(GITHUB_DECLARATION.into()),
            proxy_maps: vec![
                proxy(MockMode::Passthrough, "github-api", "140.82.112.6", false),
                proxy(MockMode::Simulate, "github-mock", "127.0.0.1", true),
            ],
            mounts: vec![],
        };
        let workspace = OperatorEntry {
            declaration: Some(WORKSPACE_DECLARATION.into()),
            proxy_maps: vec![],
            mounts: vec![MountEntry {
                target: "/workspace".to_owned(),
                source: "/Users/operator/src/project".into(),
                access: FileAccess::ReadWrite,
            }],
        };
        assert_eq!(file.entry("github-pr"), Some(&github));
        assert_eq!(file.entry("workspace"), Some(&workspace));
        assert_eq!(file.entry("unlisted"), None);
        let listed: Vec<(&str, &Path)> = file.listed().collect();
        let declarations = [GITHUB_DECLARATION, WORKSPACE_DECLARATION].map(Path::new);
        assert_eq!(
            listed,
            [
                ("github-pr", declarations[0]),
                ("workspace", declarations[1])
            ]
        );
        let sources: Vec<(&str, &Path)> = file.mount_sources().collect();
        assert_eq!(
            sources,
            [("workspace", Path::new("/Users/operator/src/project"))]
        );
    }

    #[test]
    fn accepted_plasmid_ids_keep_their_exact_spelling() {
        let empty = entry(json!([]), json!([]));
        let file = parsed(&plasmids(json!({
            "p": empty.clone(),
            " p ": empty.clone(),
            "P": empty,
        })));
        for id in ["p", " p ", "P"] {
            assert!(file.entry(id).is_some(), "{id:?} is kept");
        }
        assert_eq!(file.plasmids.len(), 3);
    }

    #[test]
    fn accepted_field_strings_keep_their_exact_spelling() {
        let mut proxy = proxy_map("api.example", 443, &["capture"], "tcp");
        proxy["destination"] = json!("API.Example");
        proxy["route"] = json!(" Route ");
        let mut declared = entry(json!([proxy]), json!([mount("/Work Space", "/Src Path")]));
        declared["declaration"] = json!("/Decl Path/P.toml");
        let file = parsed(&plasmids(json!({"p": declared})));
        let declared = file.entry("p").expect("the entry is kept");
        assert_eq!(
            declared.declaration.as_deref(),
            Some(Path::new("/Decl Path/P.toml"))
        );
        assert_eq!(declared.proxy_maps[0].host, "api.example");
        assert_eq!(declared.proxy_maps[0].route, " Route ");
        assert_eq!(declared.proxy_maps[0].recipe.destination, "API.Example");
        assert_eq!(declared.mounts[0].target, "/Work Space");
        assert_eq!(declared.mounts[0].source, Path::new("/Src Path"));
    }

    #[test]
    fn empty_is_no_entries() {
        let empty = OperatorFile::empty();
        assert_eq!(empty.entry("github-pr"), None);
        assert_eq!(empty.listed().count(), 0);
        assert_eq!(empty.mount_sources().count(), 0);
        assert_eq!(parsed(&plasmids(json!({}))), empty);
    }

    #[test]
    fn an_entry_without_a_declaration_is_kept_but_not_listed() {
        let file = parsed(&without("/plasmids/github-pr/declaration"));
        let github = file.entry("github-pr").expect("the entry is kept");
        assert_eq!(
            (github.declaration.as_deref(), github.proxy_maps.len()),
            (None, 2)
        );
        let listed: Vec<&str> = file.listed().map(|(id, _)| id).collect();
        assert_eq!(listed, ["workspace"]);
    }

    #[test]
    fn access_reads_both_names() {
        let file = parsed(&set(&format!("{MOUNT}/access"), json!("read_only")));
        let workspace = file.entry("workspace").expect("the entry is kept");
        assert_eq!(workspace.mounts[0].access, FileAccess::ReadOnly);
    }

    #[test]
    fn mount_sources_name_every_mount_in_key_then_file_order() {
        let file = parsed(&plasmids(json!({
            "b": entry(json!([]), json!([mount("/b", "/src/b")])),
            "a": entry(json!([]), json!([mount("/two", "/src/a2"), mount("/one", "/src/a1")])),
        })));
        let sources: Vec<(&str, &Path)> = file.mount_sources().collect();
        let expected = [("a", "/src/a2"), ("a", "/src/a1"), ("b", "/src/b")];
        assert_eq!(
            sources,
            expected.map(|(id, source)| (id, Path::new(source)))
        );
    }

    #[test]
    fn tcp_and_udp_on_one_host_port_and_mode_are_two_entries() {
        let dns = |transport| proxy_map("dns.example", 53, &["passthrough"], transport);
        let file = parsed(&plasmids(
            json!({"p": entry(json!([dns("tcp"), dns("udp")]), json!([]))}),
        ));
        let proxy_maps = &file.entry("p").expect("the entry is kept").proxy_maps;
        let transports: Vec<ProxyTransport> =
            proxy_maps.iter().map(|p| p.recipe.transport).collect();
        assert_eq!(transports, [ProxyTransport::Tcp, ProxyTransport::Udp]);
    }

    #[test]
    fn overlapping_modes_on_one_host_port_and_transport_refuse() {
        for (first, second, shared) in [
            (&["simulate", "capture"][..], &["capture"][..], "capture"),
            (&["capture"][..], &["simulate", "capture"][..], "capture"),
            (&["passthrough"][..], &["passthrough"][..], "passthrough"),
        ] {
            let api = |modes| proxy_map("api.example", 443, modes, "tcp");
            let pair = json!([api(first), api(second)]);
            let refusal = refused(&plasmids(json!({"p": entry(pair, json!([]))})));
            assert_eq!(refusal.at, "plasmids.p.proxy_maps[1].modes");
            let tail = "which has the same host, port and transport";
            let reason = format!("shares the mode {shared} with proxy_maps[0], {tail}");
            assert_eq!(refusal.reason, reason);
        }
    }

    #[test]
    fn one_mode_on_another_host_port_transport_or_plasmid_is_no_overlap() {
        let capture = |host, port, transport| proxy_map(host, port, &["capture"], transport);
        let pair =
            |other: Value| entry(json!([capture("a.example", 443, "tcp"), other]), json!([]));
        let file = parsed(&plasmids(json!({
            "host": pair(capture("b.example", 443, "tcp")),
            "port": pair(capture("a.example", 8443, "tcp")),
            "transport": pair(capture("a.example", 443, "udp")),
            "plasmid": entry(json!([capture("a.example", 443, "tcp")]), json!([])),
        })));
        for id in ["host", "port", "transport", "plasmid"] {
            assert!(file.entry(id).is_some(), "{id} is kept");
        }
    }

    #[test]
    fn two_mounts_of_one_plasmid_with_one_target_refuse() {
        let mounts = json!([mount("/workspace", "/src/a"), mount("/workspace", "/src/b")]);
        let refusal = refused(&plasmids(json!({"p": entry(json!([]), mounts)})));
        assert_eq!(refusal.at, "plasmids.p.mounts[1].target");
        assert_eq!(refusal.reason, "is also the target of mounts[0]");
    }

    #[test]
    fn one_target_in_two_plasmids_or_one_source_twice_is_no_overlap() {
        let shared = |target| mount(target, "/src/shared");
        let file = parsed(&plasmids(json!({
            "a": entry(json!([]), json!([shared("/workspace")])),
            "b": entry(json!([]), json!([shared("/workspace"), shared("/other")])),
        })));
        assert_eq!(file.mount_sources().count(), 3);
    }

    #[test]
    fn the_first_fault_in_key_order_wins() {
        let bad = |host| {
            entry(
                json!([proxy_map(host, 443, &["capture"], "tcp")]),
                json!([]),
            )
        };
        let refusal = refused(&plasmids(
            json!({"b": bad("B.example"), "a": bad("A.example")}),
        ));
        assert_eq!(refusal.at, "plasmids.a.proxy_maps[0].host");
        let refusal = refused(&encoded(&json!({"zeta": 1, "alpha": 2, "plasmids": {}})));
        assert_eq!(
            refusal.at, "alpha",
            "the smallest unknown key, before the missing version"
        );
    }

    #[test]
    fn a_refusal_displays_its_location_then_its_reason() {
        let whole = refused(b"[]").to_string();
        assert_eq!(whole, "the operator file must be a JSON object");
        let version = refused(&set("/version", json!(2))).to_string();
        assert_eq!(version, "version: must be the integer 1, not 2");
        let unknown = refused(&with("", "api-key", json!(1))).to_string();
        assert_eq!(unknown, "api-key: is not a field of the operator file");
    }

    #[test]
    fn refusals_escape_controls_in_ids_keys_and_reasons_but_keep_raw_fields() {
        for character in ['\n', '\u{1B}', '\u{202E}', '\u{85}'] {
            let raw = format!("a{character}b");
            let escape = format!("a\\u{:04X}b", u32::from(character));
            let id = refused(&plasmids(json!({raw.clone(): {}})));
            let key = refused(&with("", &raw, json!(1)));
            for (refusal, at, visible) in [
                (
                    id,
                    format!("plasmids.{raw}.proxy_maps"),
                    format!("plasmids.{escape}.proxy_maps"),
                ),
                (key, raw.clone(), escape.clone()),
            ] {
                assert_eq!(refusal.at, at);
                let display = refusal.to_string();
                assert!(!display.chars().any(acted_on), "{display:?}");
                assert_eq!(display, format!("{visible}: {}", refusal.reason));
            }
            for at in ["", "api-key"] {
                let refusal = ContentRefusal {
                    at: at.to_owned(),
                    reason: raw.clone(),
                };
                let prefix = if at.is_empty() { "" } else { "api-key: " };
                assert_eq!(refusal.to_string(), format!("{prefix}{escape}"));
                assert_eq!(refusal.reason, raw);
            }
        }
    }

    #[test]
    fn the_operator_file_corpus_replays() {
        replay::replay("operator_file", operator_file::check);
    }
}
