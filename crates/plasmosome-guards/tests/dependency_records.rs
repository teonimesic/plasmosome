use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use plasmosome_guards::check_workspace;
use tempfile::TempDir;

const HARM: &str = "A build script or proc macro nobody reviewed runs at the first cargo command on every machine that builds main, including ones that hold publish credentials (spec 026 section 1).";

const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

const MAX_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;

const RECORDS: &str = "docs/dependencies/";

const FIELDS: [&str; 10] = [
    "name",
    "range",
    "reason",
    "write_it_instead",
    "exception",
    "brings",
    "build_time_code",
    "expands_unsafe",
    "native_code",
    "verdict",
];

const VERDICTS: [&str; 3] = ["admitted", "kept-pending-owner", "replace"];

#[derive(Clone)]
struct Package {
    name: String,
    version: String,
    source: Option<String>,
    lockfile: String,
}

impl Package {
    fn new(name: String, version: String, source: Option<String>, lockfile: &str) -> Package {
        let lockfile = lockfile.to_string();
        Package {
            name,
            version,
            source,
            lockfile,
        }
    }
}

struct Record {
    file: String,
    text: String,
}

fn is_number(part: &str) -> bool {
    !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
}

fn ranged(version: &str) -> Result<String, String> {
    let core = version.split('+').next().unwrap_or_default();
    let parts: Vec<&str> = core
        .split('-')
        .next()
        .unwrap_or_default()
        .split('.')
        .collect();
    let numbers: Vec<u64> = parts
        .iter()
        .filter(|part| is_number(part))
        .filter_map(|part| part.parse().ok())
        .collect();
    match (parts.len(), &numbers[..]) {
        (3, [0, 0, patch]) => Ok(format!("0.0.{patch}")),
        (3, [0, minor, _]) => Ok(format!("0.{minor}")),
        (3, [major, _, _]) => Ok(major.to_string()),
        _ => Err(format!("{version:?} is not a version of the form x.y.z")),
    }
}

fn is_range(text: &str) -> bool {
    let counted = |part: &str| is_number(part) && !part.starts_with('0');
    match text.split('.').collect::<Vec<_>>()[..] {
        ["0", "0", patch] => is_number(patch),
        ["0", minor] => counted(minor),
        [major] => counted(major),
        _ => false,
    }
}

fn is_label(text: &str) -> bool {
    text.split_once('@').is_some_and(|(name, range)| {
        let named = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_';
        !name.is_empty() && name.bytes().all(named) && is_range(range)
    })
}

fn is_lockfile(path: &str) -> bool {
    path == "Cargo.lock" || path.ends_with("/Cargo.lock")
}

fn is_record(path: &str) -> bool {
    path.strip_prefix(RECORDS)
        .is_some_and(|file| !file.contains('/') && file.len() > 5 && file.ends_with(".toml"))
}

fn shape(field: &str) -> &'static str {
    match field {
        "range" => "must be the semver-compatible part of a version, such as \"0.8\" or \"1\"",
        "reason" => "must be a non-empty string",
        "write_it_instead" => "must be a table of `lines` and `how`",
        "exception" => "must be a string, empty when there is no exception",
        "brings" => "must be an array of \"<name>@<range>\" strings",
        "build_time_code" => "must be an array of strings",
        "verdict" => "must be one of admitted, kept-pending-owner and replace",
        _ => "must be \"none\" or a non-empty array of non-empty strings",
    }
}

fn valid(field: &str, value: &toml::Value) -> bool {
    let text = |value: &toml::Value| value.as_str().is_some_and(|text| !text.trim().is_empty());
    let all = |value: &toml::Value, each: &dyn Fn(&toml::Value) -> bool| {
        value.as_array().is_some_and(|items| items.iter().all(each))
    };
    match field {
        "range" => value.as_str().is_some_and(is_range),
        "reason" => text(value),
        "write_it_instead" => value.is_table(),
        "exception" => value.is_str(),
        "brings" | "build_time_code" => all(value, &toml::Value::is_str),
        "verdict" => value.as_str().is_some_and(|text| VERDICTS.contains(&text)),
        _ => {
            value.as_str() == Some("none")
                || (value.as_array().is_some_and(|items| !items.is_empty()) && all(value, &text))
        }
    }
}

fn read_record(
    record: &Record,
    named: &mut BTreeSet<String>,
    own: &mut BTreeSet<String>,
) -> Vec<String> {
    let file = &record.file;
    let table: toml::Table = match record.text.parse() {
        Ok(table) => table,
        Err(error) => return vec![format!("{file} is not TOML: {}", error.message())],
    };
    let stem = file
        .strip_prefix(RECORDS)
        .and_then(|name| name.strip_suffix(".toml"));
    let mut problems = Vec::new();
    let mut problem = |field: &str, what: &str| problems.push(format!("{file}: `{field}` {what}"));
    for key in table.keys().filter(|key| !FIELDS.contains(&key.as_str())) {
        problem(key, "is not one of the ten fields");
    }
    for field in FIELDS {
        match table.get(field) {
            None => problem(field, "is missing"),
            Some(value) if field == "name" && value.as_str() != stem => problem(
                field,
                &format!(
                    "must be the string {:?}, the file's own name",
                    stem.unwrap_or_default()
                ),
            ),
            Some(value) if field != "name" && !valid(field, value) => problem(field, shape(field)),
            Some(_) => {}
        }
    }
    if let Some(estimate) = table
        .get("write_it_instead")
        .and_then(toml::Value::as_table)
    {
        for key in estimate
            .keys()
            .filter(|key| !["lines", "how"].contains(&key.as_str()))
        {
            problem(&format!("write_it_instead.{key}"), "is not a field");
        }
        match estimate.get("lines").map(toml::Value::as_integer) {
            None => problem("write_it_instead.lines", "is missing"),
            Some(Some(lines)) if lines >= 1 => {}
            Some(_) => problem("write_it_instead.lines", "must be an integer of at least 1"),
        }
        match estimate.get("how").map(toml::Value::as_str) {
            None => problem("write_it_instead.how", "is missing"),
            Some(Some(how)) if !how.trim().is_empty() => {}
            Some(_) => problem("write_it_instead.how", "must be a non-empty string"),
        }
    }
    let brings = table.get("brings").and_then(toml::Value::as_array);
    for label in brings.into_iter().flatten().filter_map(toml::Value::as_str) {
        if is_label(label) {
            named.insert(label.to_string());
        } else {
            problem(
                "brings",
                &format!("entry {label:?} must be \"<name>@<range>\""),
            );
        }
    }
    let name = table.get("name").and_then(toml::Value::as_str);
    let range = table
        .get("range")
        .and_then(toml::Value::as_str)
        .filter(|range| is_range(range));
    if let (Some(name), Some(range)) = (name, range) {
        named.insert(format!("{name}@{range}"));
        own.insert(format!("{name}@{range}"));
    }
    problems
}

fn findings(locked: &[Package], direct: &[Package], records: &[Record]) -> Vec<String> {
    let mut named = BTreeSet::new();
    let mut own = BTreeSet::new();
    let fields: Vec<String> = records
        .iter()
        .flat_map(|record| read_record(record, &mut named, &mut own))
        .collect();
    let mut versions = BTreeSet::new();
    let mut unnamed = BTreeSet::new();
    let mut orphans = BTreeSet::new();
    let candidates = locked.iter().map(|package| (package, false));
    let candidates = candidates.chain(direct.iter().map(|package| (package, true)));
    for (package, is_direct) in candidates.filter(|(package, _)| package.source.is_some()) {
        let (name, version, lockfile) = (&package.name, &package.version, &package.lockfile);
        let label = match ranged(version) {
            Ok(range) => format!("{name}@{range}"),
            Err(error) => {
                versions.insert(format!("{name} in {lockfile}: {error}"));
                continue;
            }
        };
        if !is_direct && !named.contains(&label) {
            unnamed.insert(format!(
                "{label} (locked {version} in {lockfile}) is named by no record in docs/dependencies/"
            ));
        }
        if is_direct && !own.contains(&label) {
            orphans.insert(format!(
                "direct dependency {label} has no record of its own; write docs/dependencies/{name}.toml"
            ));
        }
    }
    versions
        .into_iter()
        .chain(unnamed)
        .chain(orphans)
        .chain(fields)
        .collect()
}

fn refusal(findings: &[String]) -> Option<String> {
    (!findings.is_empty()).then(|| format!("{}\n{HARM}", findings.join("\n")))
}

fn read_bounded(path: &Path) -> Result<String, String> {
    let unreadable = |error: std::io::Error| format!("cannot be read: {error}");
    if !std::fs::metadata(path).map_err(unreadable)?.is_file() {
        return Err("is not a regular file".to_string());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(unreadable)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("is larger than 8 MiB".to_string());
    }
    String::from_utf8(bytes).map_err(|_| "is not UTF-8".to_string())
}

fn run_bounded(command: &mut Command) -> Result<Vec<u8>, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start: {error}"))?;
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let errors = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = (&mut stderr).take(1024 * 1024).read_to_end(&mut text);
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
        text
    });
    let mut output = Vec::new();
    let stdout = child.stdout.take().expect("stdout is piped");
    let read = stdout.take(MAX_OUTPUT_BYTES + 1).read_to_end(&mut output);
    let over = output.len() as u64 > MAX_OUTPUT_BYTES;
    if read.is_err() || over {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|error| format!("cannot be waited for: {error}"))?;
    let errors = errors.join().unwrap_or_default();
    match (read, over) {
        (Err(error), _) => Err(format!("its output cannot be read: {error}")),
        (_, true) => Err("its output is larger than 64 MiB".to_string()),
        _ if !status.success() => Err(format!(
            "{status}: {}",
            String::from_utf8_lossy(&errors).trim()
        )),
        _ => Ok(output),
    }
}

fn locked_in(text: &str, lockfile: &str) -> Result<Vec<Package>, String> {
    let table: toml::Table = text
        .parse()
        .map_err(|error: toml::de::Error| format!("{lockfile} is not TOML: {}", error.message()))?;
    let Some(entries) = table.get("package") else {
        return Ok(Vec::new());
    };
    let entries = entries
        .as_array()
        .ok_or(format!("{lockfile}: `package` is not an array"))?;
    entries
        .iter()
        .map(|entry| {
            let text = |key: &str| {
                entry
                    .get(key)
                    .and_then(toml::Value::as_str)
                    .map(str::to_string)
            };
            let (Some(name), Some(version)) = (text("name"), text("version")) else {
                return Err(format!(
                    "{lockfile} holds a [[package]] without a name and a version"
                ));
            };
            let source = entry.get("source").map(ToString::to_string);
            Ok(Package::new(name, version, source, lockfile))
        })
        .collect()
}

fn resolve(
    root: &Path,
    cargo: &OsStr,
    lockfile: &str,
) -> Result<(Vec<Package>, Vec<Package>), String> {
    let manifest = Path::new(lockfile).with_file_name("Cargo.toml");
    let output = run_bounded(
        Command::new(cargo)
            .current_dir(root)
            .args(["metadata", "--locked", "--all-features"])
            .args(["--format-version", "1", "--manifest-path"])
            .arg(&manifest),
    )?;
    let metadata: serde_json::Value = serde_json::from_slice(&output)
        .map_err(|error| format!("its JSON does not parse: {error}"))?;
    let text = |value: &serde_json::Value| value.as_str().map(str::to_string);
    let mut packages = BTreeMap::new();
    for package in metadata["packages"]
        .as_array()
        .ok_or("it lists no packages")?
    {
        let name = text(&package["name"]).unwrap_or_default();
        let version = text(&package["version"]).unwrap_or_default();
        let package_id = text(&package["id"]);
        let source = text(&package["source"]);
        packages.insert(package_id, Package::new(name, version, source, lockfile));
    }
    let members = metadata["workspace_members"]
        .as_array()
        .ok_or("it lists no members")?;
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .ok_or("it holds no resolve")?;
    let mut direct = Vec::new();
    for node in nodes.iter().filter(|node| members.contains(&node["id"])) {
        for dependency in node["deps"].as_array().into_iter().flatten() {
            let package = packages.get(&text(&dependency["pkg"]));
            direct.push(package.ok_or("a dependency names no package")?.clone());
        }
    }
    let sourced = packages
        .into_values()
        .filter(|package| package.source.is_some())
        .collect();
    Ok((direct, sourced))
}

fn guard(root: &Path, cargo: &OsStr) -> Vec<String> {
    let listing = run_bounded(
        Command::new("git")
            .current_dir(root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .args(["--no-optional-locks", "ls-files", "-z"]),
    );
    let listing = match listing {
        Ok(listing) => String::from_utf8_lossy(&listing).into_owned(),
        Err(error) => return vec![format!("git ls-files failed, so nothing was read: {error}")],
    };
    let tracked: BTreeSet<&str> = listing
        .split('\0')
        .filter(|path| !path.is_empty())
        .collect();
    let mut problems = Vec::new();
    let mut locked = Vec::new();
    let mut direct = Vec::new();
    for lockfile in tracked.iter().copied().filter(|path| is_lockfile(path)) {
        let held = read_bounded(&root.join(lockfile))
            .map_err(|error| format!("{lockfile} {error}"))
            .and_then(|text| locked_in(&text, lockfile));
        let held = match held {
            Ok(held) => held,
            Err(error) => {
                problems.push(error);
                continue;
            }
        };
        match resolve(root, cargo, lockfile) {
            Ok((members, sourced)) => {
                for package in sourced {
                    let same = |lock: &Package| {
                        lock.source.is_some()
                            && (&lock.name, &lock.version) == (&package.name, &package.version)
                    };
                    if !held.iter().any(same) {
                        let (name, version) = (&package.name, &package.version);
                        problems.push(format!(
                            "the resolve names {name} {version}, which {lockfile} does not lock"
                        ));
                    }
                }
                direct.extend(members);
            }
            Err(error) => problems.push(format!("cannot read the resolve of {lockfile}: {error}")),
        }
        locked.extend(held);
    }
    let mut records = Vec::new();
    for file in tracked.iter().copied().filter(|path| is_record(path)) {
        match read_bounded(&root.join(file)) {
            Ok(text) => records.push(Record {
                file: file.to_string(),
                text,
            }),
            Err(error) => problems.push(format!("{file} {error}")),
        }
    }
    problems.extend(findings(&locked, &direct, &records));
    problems
}

fn cargo() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

#[test]
fn every_dependency_has_a_review_record() {
    check_workspace(|root| {
        if let Some(refusal) = refusal(&guard(root, &cargo())) {
            panic!("{refusal}");
        }
    });
}

const NONE: [&str; 0] = [];

fn locked(name: &str, version: &str) -> Package {
    let source = Some("registry+https://github.com/rust-lang/crates.io-index".to_string());
    Package::new(name.to_string(), version.to_string(), source, "Cargo.lock")
}

fn fields(name: &str, range: &str, brings: &[&str]) -> Vec<(String, String)> {
    let quoted = |text: &str| format!("{text:?}");
    [
        ("name", quoted(name)),
        ("range", quoted(range)),
        ("reason", quoted("Used here.")),
        ("write_it_instead.lines", "100".to_string()),
        ("write_it_instead.how", quoted("Counted.")),
        ("exception", quoted("")),
        ("brings", format!("{brings:?}")),
        ("build_time_code", "[]".to_string()),
        ("expands_unsafe", quoted("none")),
        ("native_code", quoted("none")),
        ("verdict", quoted("kept-pending-owner")),
    ]
    .map(|(key, value)| (key.to_string(), value))
    .to_vec()
}

fn record_of(name: &str, fields: &[(String, String)]) -> Record {
    let text = fields
        .iter()
        .map(|(key, value)| format!("{key} = {value}\n"))
        .collect();
    Record {
        file: format!("{RECORDS}{name}.toml"),
        text,
    }
}

fn record(name: &str, range: &str, brings: &[&str]) -> Record {
    record_of(name, &fields(name, range, brings))
}

fn edited(remove: &[&str], set: &[(&str, &str)]) -> Record {
    let mut fields = fields("demo", "1", &[]);
    fields.retain(|(key, _)| {
        !remove.contains(&key.as_str()) && !set.iter().any(|(set, _)| set == key)
    });
    fields.extend(
        set.iter()
            .map(|(key, value)| (key.to_string(), value.to_string())),
    );
    record_of("demo", &fields)
}

fn unnamed(label: &str, version: &str, lockfile: &str) -> String {
    format!("{label} (locked {version} in {lockfile}) is named by no record in docs/dependencies/")
}

fn orphan(label: &str) -> String {
    let name = label.split('@').next().unwrap_or_default();
    format!(
        "direct dependency {label} has no record of its own; write docs/dependencies/{name}.toml"
    )
}

#[test]
fn ranges_are_the_semver_compatible_part() {
    for (version, range) in [
        ("0.8.23", "0.8"),
        ("1.0.229", "1"),
        ("0.0.3", "0.0.3"),
        ("2.0.0-rc.1", "2"),
        ("0.10.0+meta", "0.10"),
        ("1.0.4+wasi-0.2.12", "1"),
    ] {
        assert_eq!(ranged(version).as_deref(), Ok(range), "{version}");
        assert!(is_range(range), "{range} is what ranged writes");
    }
    for version in ["1.2", "x.y.z", "", "1.2.3.4", "1..3", "+1.2.3"] {
        assert!(ranged(version).is_err(), "{version:?} must be refused");
    }
    for range in ["0.8.23", "01", "0.08", "0", "0.0", "1.0", "", "0.x"] {
        assert!(!is_range(range), "{range:?} is not a range");
    }
}

#[test]
fn a_transitive_package_no_record_brings_is_refused() {
    let serde = locked("serde", "1.0.229");
    let locks = [serde.clone(), locked("serde_derive", "1.0.229")];
    let found = findings(&locks, &[serde], &[record("serde", "1", &[])]);
    assert_eq!(found, [unnamed("serde_derive@1", "1.0.229", "Cargo.lock")]);
    let message = refusal(&found).expect("a finding refuses");
    assert!(message.starts_with(&found[0]), "{message}");
    assert_eq!(message.matches(HARM).count(), 1, "{message}");
    assert_eq!(refusal(&[]), None);
}

#[test]
fn a_direct_package_with_no_record_is_refused() {
    let smallvec = [locked("smallvec", "1.15.1")];
    let expected = [
        unnamed("smallvec@1", "1.15.1", "Cargo.lock"),
        orphan("smallvec@1"),
    ];
    assert_eq!(findings(&smallvec, &smallvec, &[]), expected);
}

#[test]
fn a_new_range_without_a_record_change_is_refused() {
    let toml = [locked("toml", "0.9.0")];
    let expected = [
        unnamed("toml@0.9", "0.9.0", "Cargo.lock"),
        orphan("toml@0.9"),
    ];
    assert_eq!(
        findings(&toml, &toml, &[record("toml", "0.8", &[])]),
        expected
    );
    let serde = locked("serde", "1.0.229");
    let locks = [serde.clone(), locked("syn", "3.0.4")];
    let found = findings(&locks, &[serde], &[record("serde", "1", &["syn@2"])]);
    assert_eq!(found, [unnamed("syn@3", "3.0.4", "Cargo.lock")]);
}

#[test]
fn a_compatible_bump_passes() {
    let toml = [locked("toml", "0.8.24")];
    assert_eq!(findings(&toml, &toml, &[record("toml", "0.8", &[])]), NONE);
}

#[test]
fn a_compatible_bump_that_adds_a_package_is_refused() {
    let records = [record("serde_json", "1", &["itoa@1", "memchr@2"])];
    let before = vec![
        locked("serde_json", "1.0.151"),
        locked("itoa", "1.0.18"),
        locked("memchr", "2.8.3"),
    ];
    assert_eq!(findings(&before, &before[..1], &records), NONE);
    let mut after = before.clone();
    after[0] = locked("serde_json", "1.0.154");
    after.push(locked("ryu", "1.0.20"));
    assert_eq!(
        findings(&after, &after[..1], &records),
        [unnamed("ryu@1", "1.0.20", "Cargo.lock")]
    );
}

#[test]
fn a_direct_dependency_covered_only_by_brings_is_refused() {
    let both = [locked("serde", "1.0.229"), locked("serde_core", "1.0.229")];
    let records = [record("serde", "1", &["serde_core@1"])];
    assert_eq!(findings(&both, &both, &records), [orphan("serde_core@1")]);
}

#[test]
fn each_missing_or_malformed_field_is_named() {
    let file = format!("{RECORDS}demo.toml");
    let said = |field: &str, what: &str| format!("{file}: `{field}` {what}");
    let estimate = ["write_it_instead.lines", "write_it_instead.how"];
    let mut cases: Vec<(Record, String)> = FIELDS
        .iter()
        .filter(|field| **field != "write_it_instead")
        .chain(&estimate)
        .map(|field| (edited(&[field], &[]), said(field, "is missing")))
        .collect();
    cases.push((
        edited(&estimate, &[]),
        said("write_it_instead", "is missing"),
    ));
    cases.push((
        edited(&estimate, &[("write_it_instead", "\"300\"")]),
        said("write_it_instead", shape("write_it_instead")),
    ));
    let stem = "must be the string \"demo\", the file's own name";
    let lines = "must be an integer of at least 1";
    let entry = |label: &str| format!("entry {label:?} must be \"<name>@<range>\"");
    for (field, value, what) in [
        ("name", "\"other\"", stem),
        ("name", "1", stem),
        ("range", "\"0.8.23\"", shape("range")),
        ("range", "1", shape("range")),
        ("reason", "\"\"", shape("reason")),
        ("reason", "1", shape("reason")),
        ("write_it_instead.lines", "0", lines),
        ("write_it_instead.lines", "\"300\"", lines),
        ("write_it_instead.lines", "1.5", lines),
        (
            "write_it_instead.how",
            "\" \"",
            "must be a non-empty string",
        ),
        ("write_it_instead.extra", "1", "is not a field"),
        ("exception", "1", shape("exception")),
        ("brings", "\"serde@1\"", shape("brings")),
        ("brings", "[1]", shape("brings")),
        ("brings", "[\"serde\"]", &entry("serde")),
        ("brings", "[\"serde@1.0\"]", &entry("serde@1.0")),
        ("build_time_code", "\"none\"", shape("build_time_code")),
        ("build_time_code", "[1]", shape("build_time_code")),
        ("expands_unsafe", "[]", shape("expands_unsafe")),
        ("expands_unsafe", "1", shape("expands_unsafe")),
        ("native_code", "[\"\"]", shape("native_code")),
        ("native_code", "true", shape("native_code")),
        ("verdict", "\"ok\"", shape("verdict")),
        ("verdict", "1", shape("verdict")),
        ("features", "[]", "is not one of the ten fields"),
    ] {
        cases.push((edited(&[], &[(field, value)]), said(field, what)));
    }
    for (record, expected) in cases {
        let found = findings(&[], &[], std::slice::from_ref(&record));
        assert_eq!(found, [expected], "{}", record.text);
    }
    let broken = Record {
        file: file.clone(),
        text: "name = \n".to_string(),
    };
    let found = findings(&[], &[], &[broken]);
    assert!(
        found.len() == 1 && found[0].starts_with(&format!("{file} is not TOML: ")),
        "{found:?}"
    );
    let first = edited(
        &["reason"],
        &[("verdict", "\"ok\""), ("write_it_instead.lines", "0")],
    );
    let second = record("other", "1", &["serde"]);
    let expected = [
        said("reason", "is missing"),
        said("verdict", shape("verdict")),
        said("write_it_instead.lines", lines),
        format!("{RECORDS}other.toml: `brings` {}", entry("serde")),
    ];
    assert_eq!(findings(&[], &[], &[first, second]), expected);
}

#[test]
fn a_complete_record_passes_in_either_table_form() {
    let estimate = ["write_it_instead.lines", "write_it_instead.how"];
    let inline = ("write_it_instead", "{ lines = 1, how = \"Counted.\" }");
    for record in [
        record("demo", "1", &["a@0.0.1"]),
        edited(&estimate, &[inline]),
    ] {
        assert_eq!(findings(&[], &[], &[record]), NONE);
    }
}

#[test]
fn path_packages_never_count() {
    let path = [Package {
        source: None,
        ..locked("plasmosome-core", "0.1.0")
    }];
    assert_eq!(findings(&path, &path, &[]), NONE);
}

#[test]
fn only_tracked_lockfiles_and_top_level_records_are_read() {
    assert!(is_lockfile("Cargo.lock") && is_lockfile("fuzz/Cargo.lock"));
    for path in ["xCargo.lock", "Cargo.lock.orig", "fuzz/xCargo.lock"] {
        assert!(!is_lockfile(path), "{path}");
    }
    assert!(is_record("docs/dependencies/toml.toml"));
    for path in [
        "docs/dependencies/native/libkrun.toml",
        "docs/dependencies/README.md",
        "docs/dependencies/.toml",
        "docs/dependencies.toml",
        "x/docs/dependencies/toml.toml",
    ] {
        assert!(!is_record(path), "{path}");
    }
}

#[test]
fn files_are_read_through_a_bound_of_8_mib() {
    let directory = tempfile::tempdir().expect("a scratch directory is created");
    let file = |name: &str, bytes: &[u8]| {
        let path = directory.path().join(name);
        std::fs::write(&path, bytes).expect("the file is written");
        path
    };
    let bound = usize::try_from(MAX_FILE_BYTES).expect("8 MiB fits");
    let at = read_bounded(&file("at.toml", &vec![b'#'; bound]));
    assert_eq!(at.map(|text| text.len()), Ok(bound));
    let past = read_bounded(&file("past.toml", &vec![b'#'; bound + 1]));
    assert_eq!(past, Err("is larger than 8 MiB".to_string()));
    let zero = directory.path().join("zero.toml");
    std::os::unix::fs::symlink("/dev/zero", &zero).expect("the link is made");
    assert_eq!(
        read_bounded(&zero),
        Err("is not a regular file".to_string())
    );
    let binary = read_bounded(&file("binary.toml", &[0xff]));
    assert_eq!(binary, Err("is not UTF-8".to_string()));
}

struct Fixture {
    directory: TempDir,
}

impl Fixture {
    fn new(vendored: &[(&str, &str, &str)]) -> Fixture {
        let directory = tempfile::tempdir().expect("a scratch directory is created");
        let fixture = Fixture { directory };
        fixture.write(
            ".cargo/config.toml",
            "[source.crates-io]\nreplace-with = \"vendored\"\n\n[source.vendored]\ndirectory = \"vendor\"\n\n[net]\noffline = true\n",
        );
        for (name, version, dependencies) in vendored {
            let manifest = format!(
                "[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2021\"\n\n[dependencies]\n{dependencies}"
            );
            fixture.write(&format!("vendor/{name}/Cargo.toml"), &manifest);
            fixture.write(&format!("vendor/{name}/src/lib.rs"), "");
            let checksum = "{\"files\":{},\"package\":null}";
            fixture.write(&format!("vendor/{name}/.cargo-checksum.json"), checksum);
        }
        fixture.write(
            "Cargo.toml",
            "[workspace]\nresolver = \"2\"\nmembers = [\"crates/app\"]\n",
        );
        fixture.run(Command::new("git").args(["init", "--quiet"]));
        fixture
    }

    fn package(&self, directory: &str, name: &str, manifest: &str) {
        let package =
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
        self.write(
            &format!("{directory}/Cargo.toml"),
            &format!("{package}{manifest}"),
        );
        self.write(&format!("{directory}/src/lib.rs"), "");
    }

    fn record(&self, name: &str, range: &str, brings: &[&str]) {
        self.write(
            &format!("{RECORDS}{name}.toml"),
            &record(name, range, brings).text,
        );
    }

    fn write(&self, path: &str, content: &str) {
        let path = self.directory.path().join(path);
        std::fs::create_dir_all(path.parent().expect("a fixture path has a parent"))
            .expect("the fixture directory is made");
        std::fs::write(path, content).expect("the fixture file is written");
    }

    fn lock(&self, manifest: &str) {
        self.run(Command::new(cargo()).args(["generate-lockfile", "--manifest-path", manifest]));
    }

    fn track(&self) {
        self.run(Command::new("git").args(["add", "-A"]));
    }

    fn run(&self, command: &mut Command) {
        let output = command
            .current_dir(self.directory.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("the fixture command runs");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{command:?} failed: {stderr}");
    }

    fn guard(&self) -> Vec<String> {
        guard(self.directory.path(), &cargo())
    }
}

#[test]
fn every_tracked_lockfile_is_read_and_path_packages_do_not_count() {
    let fixture = Fixture::new(&[
        ("alpha", "1.2.3", "beta = \"0.4\"\n"),
        ("beta", "0.4.1", ""),
        ("gamma", "2.0.0", ""),
    ]);
    fixture.package("crates/app", "app", "\n[dependencies]\nalpha = \"1\"\n");
    let fuzz =
        "\n[workspace]\n\n[dependencies]\napp = { path = \"../crates/app\" }\ngamma = \"2\"\n";
    fixture.package("fuzz", "fuzzer", fuzz);
    fixture.lock("Cargo.toml");
    fixture.lock("fuzz/Cargo.toml");
    fixture.record("alpha", "1", &["beta@0.4"]);
    fixture.record("gamma", "2", &[]);
    let native = "name = \"libkrun\"\npurpose = \"Boots cells.\"\nversion = \"1.17.4\"\n";
    fixture.write("docs/dependencies/native/libkrun.toml", native);
    fixture.write("docs/dependencies/README.md", "# Dependency records\n");
    fixture.track();
    assert_eq!(fixture.guard(), NONE);
    let gamma = fixture
        .directory
        .path()
        .join("docs/dependencies/gamma.toml");
    std::fs::remove_file(gamma).expect("the record is removed");
    fixture.track();
    let without = [
        unnamed("gamma@2", "2.0.0", "fuzz/Cargo.lock"),
        orphan("gamma@2"),
    ];
    assert_eq!(fixture.guard(), without);
    fixture.record("gamma", "2", &[]);
    assert_eq!(
        fixture.guard(),
        without,
        "an untracked record does not count"
    );
}

#[test]
fn an_optional_dependency_still_counts() {
    let fixture = Fixture::new(&[("opt", "0.3.1", "")]);
    let optional = "\n[dependencies]\nopt = { version = \"0.3\", optional = true }\n\n[features]\nextra = [\"dep:opt\"]\n";
    fixture.package("crates/app", "app", optional);
    fixture.lock("Cargo.toml");
    fixture.track();
    let expected = [unnamed("opt@0.3", "0.3.1", "Cargo.lock"), orphan("opt@0.3")];
    assert_eq!(fixture.guard(), expected);
}

#[test]
fn a_stale_lockfile_is_a_finding_not_a_pass() {
    let fixture = Fixture::new(&[("alpha", "1.2.3", ""), ("delta", "0.1.0", "")]);
    fixture.package("crates/app", "app", "\n[dependencies]\nalpha = \"1\"\n");
    fixture.lock("Cargo.toml");
    fixture.package(
        "crates/app",
        "app",
        "\n[dependencies]\nalpha = \"1\"\ndelta = \"0.1\"\n",
    );
    fixture.record("alpha", "1", &[]);
    fixture.record("delta", "0.1", &[]);
    fixture.track();
    let found = fixture.guard();
    let stale = |finding: &String| {
        finding.starts_with("cannot read the resolve of Cargo.lock: ")
            && finding.contains("--locked")
    };
    assert!(found.len() == 1 && stale(&found[0]), "{found:?}");
    assert!(refusal(&found).is_some());
}
