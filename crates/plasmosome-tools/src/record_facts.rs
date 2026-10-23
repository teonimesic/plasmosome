use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use serde_json::Value;

use crate::bounded;

/// The most bytes read from one `cargo metadata` file. The files are a few MiB; the bound only
/// stops a runaway.
pub const METADATA_LIMIT: u64 = 256 * 1024 * 1024;

/// The most bytes read from one `.rs` file while looking for `link(name = "...")`.
pub const SOURCE_LIMIT: u64 = 16 * 1024 * 1024;

const USAGE: &str = "expected ALL.json HOSTS.json CRATE [RANGE]: the cargo metadata for all \
                     platforms, the one filtered to the two hosts, and the crate";

/// Writes to `out` the derived fields of the dependency record of one crate, `name`, `range`,
/// `brings` and `build_time_code`, then two comment lines: what leaves Cargo.lock with the crate,
/// and the native-code hints to read. `docs/dependencies/README.md` says how each is made.
///
/// `args` are the arguments after the program name: the `cargo metadata` file for all platforms,
/// the one filtered to the two supported hosts, the crate's name, and its range when the
/// lockfile holds two. The files are read through [`METADATA_LIMIT`], and each `.rs` file of a
/// package that builds on a host through [`SOURCE_LIMIT`].
///
/// Returns 0 when the lines are written. Returns 1 when not exactly one locked third-party
/// package matches the name and range, when a file cannot be read or is not what cargo writes,
/// or when writing or flushing `out` fails. Input failures write nothing to `out`; an output
/// failure can leave a prefix there. Returns 2 unless `args` holds three or four arguments.
/// Failures are reported on `err`.
pub fn run(args: &[OsString], out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let (all, hosts, name, wanted) = match args {
        [all, hosts, name] => (all, hosts, name, None),
        [all, hosts, name, range] => (all, hosts, name, Some(range.to_string_lossy())),
        _ => return fail(err, 2, USAGE),
    };
    let name = name.to_string_lossy();
    let lines = match facts(Path::new(all), Path::new(hosts), &name, wanted.as_deref()) {
        Ok(lines) => lines,
        Err(Refusal::Matches(count)) => {
            let _ = writeln!(
                err,
                "{name}: {count} locked third-party packages match; pass the range too"
            );
            return 1;
        }
        Err(Refusal::Failed(reason)) => return fail(err, 1, &reason),
    };
    match out.write_all(lines.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => 0,
        Err(error) => fail(err, 1, &format!("cannot write to stdout: {error}")),
    }
}

enum Refusal {
    Matches(usize),
    Failed(String),
}

impl From<String> for Refusal {
    fn from(reason: String) -> Self {
        Self::Failed(reason)
    }
}

fn facts(all: &Path, hosts: &Path, name: &str, wanted: Option<&str>) -> Result<String, Refusal> {
    let (all_value, host_value) = (load(all)?, load(hosts)?);
    let (all, hosts) = (
        Resolve::of(&all_value, all)?,
        Resolve::of(&host_value, hosts)?,
    );
    let root = all.find(name, wanted)?;
    let range = ranged(text(all.package(root)?, "version")?)?;
    let mut brings = closure(&all.nodes, root)?
        .into_iter()
        .map(|id| all.label(id))
        .collect::<Result<Vec<_>, _>>()?;
    brings.sort();
    let (built, hints) = host_facts(&hosts, root)?;
    let kept = kept_without(&all, root)?;
    let mut gone = Vec::new();
    for (id, package) in &all.packages {
        if truthy(get(package, "source")?) && !kept.contains(id) {
            gone.push(all.label(id)?);
        }
    }
    gone.sort();
    let count = gone.len();
    let leaves = or_else(gone.join(", "), "none, it stays");
    let hints = or_else(hints.join("; "), "none");
    Ok(format!(
        "name = \"{name}\"\nrange = \"{range}\"\nbrings = {}\nbuild_time_code = {}\n\
         # leaves Cargo.lock with it: {count}: {leaves}\n\
         # native-code hints to read, not a field: {hints}\n",
        quoted(&brings),
        quoted(&built),
    ))
}

fn load(path: &Path) -> Result<Value, String> {
    let shown = path.display();
    let real =
        std::fs::canonicalize(path).map_err(|error| format!("cannot read {shown}: {error}"))?;
    let bytes = bounded::read_file(&real, METADATA_LIMIT)?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{shown} is not JSON: {error}"))
}

struct Resolve<'a> {
    packages: HashMap<&'a str, &'a Value>,
    nodes: HashMap<&'a str, &'a Value>,
    members: HashSet<&'a str>,
}

impl<'a> Resolve<'a> {
    fn of(metadata: &'a Value, path: &Path) -> Result<Self, String> {
        let index = |key: &str| -> Result<HashMap<&'a str, &'a Value>, String> {
            let list = match key {
                "packages" => list(metadata, key)?,
                _ => list(get(metadata, "resolve")?, key)?,
            };
            list.iter()
                .map(|item| Ok((text(item, "id")?, item)))
                .collect()
        };
        let read = || -> Result<Self, String> {
            Ok(Self {
                packages: index("packages")?,
                nodes: index("nodes")?,
                members: list(metadata, "workspace_members")?
                    .iter()
                    .map(|member| member.as_str().ok_or_else(|| not("a member", "a string")))
                    .collect::<Result<_, _>>()?,
            })
        };
        read().map_err(|reason| format!("{} is not cargo metadata: {reason}", path.display()))
    }

    fn package(&self, id: &str) -> Result<&'a Value, String> {
        self.packages
            .get(id)
            .copied()
            .ok_or_else(|| format!("no package has the id {id}"))
    }

    fn label(&self, id: &str) -> Result<String, String> {
        let package = self.package(id)?;
        Ok(format!(
            "{}@{}",
            text(package, "name")?,
            ranged(text(package, "version")?)?
        ))
    }

    fn find(&self, name: &str, wanted: Option<&str>) -> Result<&'a str, Refusal> {
        let mut found = Vec::new();
        for (&id, package) in &self.packages {
            if truthy(get(package, "source")?)
                && get(package, "name")?.as_str() == Some(name)
                && match wanted {
                    None => true,
                    Some(wanted) => ranged(text(package, "version")?)? == wanted,
                }
            {
                found.push(id);
            }
        }
        match found[..] {
            [id] => Ok(id),
            _ => Err(Refusal::Matches(found.len())),
        }
    }
}

/// The semver-compatible part of a version, as spec 026 defines a range: `0.8` for 0.8.23, `1`
/// for 1.0.229 and `0.0.3` for 0.0.3. Pre-release and build metadata are dropped.
fn ranged(version: &str) -> Result<String, String> {
    let core = version.split('-').next().unwrap_or_default();
    let numbers: Vec<_> = core
        .split('+')
        .next()
        .unwrap_or_default()
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect();
    let [Some(major), Some(minor), Some(patch)] = numbers[..] else {
        return Err(format!("the version {version} is not three numbers"));
    };
    Ok(match (major, minor) {
        (0, 0) => format!("0.0.{patch}"),
        (0, minor) => format!("0.{minor}"),
        (major, _) => major.to_string(),
    })
}

fn closure<'a>(
    nodes: &HashMap<&'a str, &'a Value>,
    root: &'a str,
) -> Result<HashSet<&'a str>, String> {
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    while let Some(here) = stack.pop() {
        for dep in list(node(nodes, here)?, "deps")? {
            if follows(dep)? {
                let pkg = text(dep, "pkg")?;
                if seen.insert(pkg) {
                    stack.push(pkg);
                }
            }
        }
    }
    seen.remove(root);
    Ok(seen)
}

/// Whether an edge is a normal or a build edge. A kind that is `null`, or false as Python tests
/// it, is a normal edge.
fn follows(dep: &Value) -> Result<bool, String> {
    let mut follows = false;
    for kind in list(dep, "dep_kinds")? {
        let kind = get(kind, "kind")?;
        follows |= !truthy(kind) || kind == "normal" || kind == "build";
    }
    Ok(follows)
}

fn kept_without<'a>(resolve: &Resolve<'a>, root: &str) -> Result<HashSet<&'a str>, String> {
    let mut seen = resolve.members.clone();
    let mut stack: Vec<_> = resolve.members.iter().copied().collect();
    while let Some(here) = stack.pop() {
        for dep in list(node(&resolve.nodes, here)?, "deps")? {
            let pkg = text(dep, "pkg")?;
            if (resolve.members.contains(here) && pkg == root) || !seen.insert(pkg) {
                continue;
            }
            stack.push(pkg);
        }
    }
    Ok(seen)
}

fn host_facts(hosts: &Resolve<'_>, root: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let (mut built, mut hints) = (Vec::new(), Vec::new());
    if !hosts.nodes.contains_key(root) {
        return Ok((built, hints));
    }
    let mut on_hosts = closure(&hosts.nodes, root)?;
    on_hosts.insert(root);
    let mut labelled = on_hosts
        .into_iter()
        .map(|id| Ok((hosts.label(id)?, id)))
        .collect::<Result<Vec<_>, String>>()?;
    labelled.sort();
    for (label, id) in labelled {
        let package = hosts.package(id)?;
        let mut kinds = Vec::new();
        for target in list(package, "targets")? {
            kinds.extend(list(target, "kind")?);
        }
        for (kind, what) in [
            ("custom-build", "build script"),
            ("proc-macro", "proc macro"),
        ] {
            if kinds.iter().any(|&found| found == kind) {
                built.push(format!("{label} {what}"));
            }
        }
        if let Some(links) = package.get("links").filter(|links| truthy(links)) {
            let links = links.as_str().ok_or_else(|| not("links", "a string"))?;
            hints.push(format!("{label} links {links}"));
        }
        let mut builders = Vec::new();
        for dependency in list(package, "dependencies")? {
            if get(dependency, "kind")? == "build" {
                builders.push(text(dependency, "name")?);
            }
        }
        builders.sort_unstable();
        if builders
            .iter()
            .any(|name| ["cc", "cmake", "pkg-config"].contains(name))
        {
            hints.push(format!("{label} builds with {}", builders.join(", ")));
        }
        let libraries = linked(Path::new(text(package, "manifest_path")?))?;
        if !libraries.is_empty() {
            let libraries: Vec<_> = libraries.into_iter().collect();
            hints.push(format!(
                "{label} names in link(name): {}",
                libraries.join(", ")
            ));
        }
    }
    Ok((built, hints))
}

/// Every library a `link(name = "...")` names in a `.rs` file under the manifest's directory.
/// The walk does not enter a directory that is a symbolic link, as Python's `rglob` does not.
fn linked(manifest: &Path) -> Result<BTreeSet<String>, String> {
    let root = manifest
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| format!("the manifest {} has no directory", manifest.display()))?;
    let mut names = BTreeSet::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let cannot = |error| format!("cannot read {}: {error}", directory.display());
        let mut entries = std::fs::read_dir(&directory)
            .and_then(Iterator::collect::<Result<Vec<_>, _>>)
            .map_err(cannot)?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            if entry.file_name().as_bytes().ends_with(b".rs") {
                let bytes = bounded::read_file(&entry.path(), SOURCE_LIMIT)?;
                scan(&String::from_utf8_lossy(&bytes), &mut names);
            }
            if entry.file_type().map_err(cannot)?.is_dir() {
                directories.push(entry.path());
            }
        }
    }
    Ok(names)
}

/// Adds to `names` each name the pattern `link\(\s*name\s*=\s*"([^"]+)"` finds in `text`, as
/// Python's `re.findall` finds them in a file read in text mode, where `\r\n` and `\r` read as
/// `\n`.
fn scan(text: &str, names: &mut BTreeSet<String>) {
    let mut rest = text;
    while let Some(at) = rest.find("link(") {
        match link_name(&rest[at + "link(".len()..]) {
            Some((name, after)) => {
                names.insert(name.replace("\r\n", "\n").replace('\r', "\n"));
                rest = after;
            }
            None => rest = &rest[at + 1..],
        }
    }
}

fn link_name(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start_matches(python_space).strip_prefix("name")?;
    let text = text.trim_start_matches(python_space).strip_prefix('=')?;
    let text = text.trim_start_matches(python_space).strip_prefix('"')?;
    let end = text.find('"').filter(|&end| end > 0)?;
    Some((&text[..end], &text[end + 1..]))
}

/// Python's `\s` in a `str` pattern: Rust's whitespace, and also U+001C to U+001F.
fn python_space(character: char) -> bool {
    character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
}

/// Whether Python reads the JSON value as true.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(members) => !members.is_empty(),
    }
}

fn node<'a>(nodes: &HashMap<&'a str, &'a Value>, id: &str) -> Result<&'a Value, String> {
    nodes
        .get(id)
        .copied()
        .ok_or_else(|| format!("the resolve has no node {id}"))
}

fn get<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    value
        .get(key)
        .ok_or_else(|| format!("an object has no key {key:?}"))
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    get(value, key)?
        .as_str()
        .ok_or_else(|| not(key, "a string"))
}

fn list<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], String> {
    get(value, key)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| not(key, "a list"))
}

fn not(what: &str, kind: &str) -> String {
    format!("{what:?} is not {kind}")
}

fn quoted(items: &[String]) -> String {
    let items: Vec<_> = items.iter().map(|item| format!("\"{item}\"")).collect();
    format!("[{}]", items.join(", "))
}

fn or_else(joined: String, empty: &str) -> String {
    if joined.is_empty() {
        empty.to_owned()
    } else {
        joined
    }
}

fn fail(err: &mut dyn Write, code: u8, reason: &str) -> u8 {
    let _ = writeln!(err, "record-facts: {reason}");
    code
}
