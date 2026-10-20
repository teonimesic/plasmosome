use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};

use plasmosome_core::{ManifestError, PlasmidManifest};

const NO_COMPONENT: &str = "no component";

const UNTIL_FROZEN: &str = "until the plasmid interface is frozen";

const NOT_A_NAME: &str = "is not a plasmid name";

const UNTOUCHED: &str = "left untouched";

fn plasmid_in<S: AsRef<OsStr>>(directory: &Path, arguments: &[S]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_plasmid"))
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("the plasmid binary runs")
}

fn entries(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .expect("the directory is readable")
        .map(|entry| {
            entry
                .expect("a directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn text(stream: &[u8]) -> String {
    String::from_utf8(stream.to_vec()).expect("the stream is UTF-8")
}

fn printed_sections(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(|line| {
            let (section, _) = line
                .strip_prefix("add ")
                .and_then(|rest| rest.split_once(": "))
                .unwrap_or_else(|| {
                    panic!("each stdout line names one section as `add <section>: ...`: {line:?}")
                });
            section.to_string()
        })
        .collect()
}

struct Scaffold {
    declaration: String,
    stdout: String,
}

fn scaffold(name: &str) -> Scaffold {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let output = plasmid_in(scratch.path(), &["new", name]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let declaration = std::fs::read_to_string(scratch.path().join(format!("{name}.toml")))
        .expect("plasmid new wrote the declaration");
    Scaffold {
        declaration,
        stdout: text(&output.stdout),
    }
}

fn applied(declaration: &str, field: &str, fix: &str) -> String {
    let mut table: toml::Table = toml::from_str(declaration).expect("the declaration is TOML");
    let written: toml::Table = toml::from_str(fix)
        .unwrap_or_else(|error| panic!("the fix for {field} is not TOML: {fix:?}: {error}"));
    let keys: Vec<&str> = written.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [field],
        "the fix for {field} writes that field and nothing else: {fix:?}"
    );
    table.extend(written);
    toml::to_string(&table).expect("the declaration serializes")
}

#[test]
fn plasmid_new_writes_one_declaration_and_nothing_else() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let output = plasmid_in(scratch.path(), &["new", "demo"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(entries(scratch.path()), ["demo.toml"]);
    let written =
        std::fs::symlink_metadata(scratch.path().join("demo.toml")).expect("demo.toml exists");
    assert!(written.is_file(), "{written:?}");
    let stdout = text(&output.stdout);
    let sections: BTreeSet<String> = printed_sections(&stdout).into_iter().collect();
    assert_eq!(
        sections,
        BTreeSet::from(["description".to_string(), "impl".to_string()]),
        "{stdout}"
    );
    let stderr = text(&output.stderr);
    for named in ["demo.toml", NO_COMPONENT, UNTIL_FROZEN] {
        assert!(stderr.contains(named), "stderr names {named:?}: {stderr}");
    }
}

#[test]
fn the_sections_printed_are_the_parser_refusals_in_the_order_it_gives_them() {
    let scaffold = scaffold("demo");
    let mut declaration = scaffold.declaration.clone();
    let mut refused: Vec<String> = Vec::new();
    loop {
        match PlasmidManifest::parse(&declaration) {
            Ok(manifest) => {
                assert_eq!(manifest.id, "demo");
                break;
            }
            Err(ManifestError::Field {
                plasmid,
                field,
                fix,
                ..
            }) => {
                assert_eq!(plasmid.as_deref(), Some("demo"), "{field}");
                assert!(
                    !refused.contains(&field),
                    "{field} is refused again after its fix was applied:\n{declaration}"
                );
                declaration = applied(&declaration, &field, &fix);
                refused.push(field);
            }
            Err(other) => panic!("the scaffold is refused for no missing section: {other}"),
        }
    }
    assert_eq!(printed_sections(&scaffold.stdout), refused);
}

#[test]
fn the_filled_scaffold_parses_equal_to_the_same_declaration_typed_by_hand() {
    let scaffold = scaffold("demo");
    let written: toml::Table = toml::from_str(&scaffold.declaration).expect("the scaffold is TOML");
    let keys: BTreeSet<&str> = written.keys().map(String::as_str).collect();
    assert_eq!(keys, BTreeSet::from(["description", "id", "version"]));
    assert_eq!(written["id"].as_str(), Some("demo"));
    assert_eq!(written["version"].as_str(), Some("0.1.0"));
    assert_eq!(written["description"].as_str(), Some(""));

    let empty = "description = \"\"";
    assert_eq!(scaffold.declaration.matches(empty).count(), 1);
    let filled = format!(
        "{}\n[impl]\nwasm = \"component.wasm\"\n",
        scaffold
            .declaration
            .replace(empty, "description = \"Read pull requests.\"")
    );
    let typed = "id = \"demo\"\n\
                 version = \"0.1.0\"\n\
                 description = \"Read pull requests.\"\n\
                 \n\
                 [impl]\n\
                 wasm = \"component.wasm\"\n";
    assert_eq!(
        PlasmidManifest::parse(&filled).expect("the filled scaffold parses"),
        PlasmidManifest::parse(typed).expect("the typed declaration parses")
    );
}

fn assert_refused_untouched(scratch: &Path, before: &[String]) {
    let output = plasmid_in(scratch, &["new", "demo"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = text(&output.stderr);
    for named in ["demo.toml", UNTOUCHED] {
        assert!(stderr.contains(named), "stderr names {named:?}: {stderr}");
    }
    assert_eq!(entries(scratch), before);
}

#[test]
fn an_existing_file_at_the_declaration_path_is_refused_and_left_untouched() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let existing = scratch.path().join("demo.toml");
    std::fs::write(&existing, "id = \"mine\"\n").expect("the existing file is written");
    assert_refused_untouched(scratch.path(), &["demo.toml".to_string()]);
    assert_eq!(
        std::fs::read_to_string(&existing).expect("the existing file is readable"),
        "id = \"mine\"\n"
    );
}

#[test]
fn an_existing_directory_at_the_declaration_path_is_refused_and_left_untouched() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let existing = scratch.path().join("demo.toml");
    std::fs::create_dir(&existing).expect("the existing directory is made");
    std::fs::write(existing.join("kept"), "kept").expect("a file inside it is written");
    assert_refused_untouched(scratch.path(), &["demo.toml".to_string()]);
    assert!(existing.is_dir());
    assert_eq!(entries(&existing), ["kept"]);
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_at_the_declaration_path_is_refused_and_not_followed() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let link = scratch.path().join("demo.toml");
    std::os::unix::fs::symlink("elsewhere.toml", &link).expect("the symlink is made");
    assert_refused_untouched(scratch.path(), &["demo.toml".to_string()]);
    assert_eq!(
        std::fs::read_link(&link).expect("demo.toml is still a symlink"),
        Path::new("elsewhere.toml")
    );
    assert!(!scratch.path().join("elsewhere.toml").exists());
}

#[cfg(unix)]
#[test]
fn a_symlink_to_a_file_at_the_declaration_path_is_refused_and_its_target_left_untouched() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let target = scratch.path().join("target.toml");
    std::fs::write(&target, "id = \"mine\"\n").expect("the target is written");
    std::os::unix::fs::symlink("target.toml", scratch.path().join("demo.toml"))
        .expect("the symlink is made");
    assert_refused_untouched(
        scratch.path(),
        &["demo.toml".to_string(), "target.toml".to_string()],
    );
    assert_eq!(
        std::fs::read_to_string(&target).expect("the target is readable"),
        "id = \"mine\"\n"
    );
}

fn assert_name_refused<S: AsRef<OsStr> + std::fmt::Debug>(name: S) {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let work = scratch.path().join("work");
    std::fs::create_dir(&work).expect("the working directory is made");
    let output = plasmid_in(&work, &[OsStr::new("new"), name.as_ref()]);
    assert_eq!(output.status.code(), Some(1), "{name:?}: {output:?}");
    assert!(output.stdout.is_empty(), "{name:?}: {output:?}");
    let stderr = text(&output.stderr);
    assert!(stderr.contains(NOT_A_NAME), "{name:?}: {stderr}");
    assert_eq!(entries(&work), Vec::<String>::new(), "{name:?}");
    assert_eq!(entries(scratch.path()), ["work"], "{name:?}");
}

#[test]
fn a_name_outside_the_registry_grammar_is_refused_and_writes_nothing() {
    let long = "a".repeat(65);
    for name in [
        "",
        "Demo",
        "../x",
        "a/b",
        "-x",
        "x-",
        "a--b",
        "a_b",
        "demo.toml",
        " demo",
        "demo\n",
        "d\u{e9}mo",
        long.as_str(),
    ] {
        assert_name_refused(name);
    }
}

#[cfg(unix)]
#[test]
fn a_name_that_is_not_utf8_is_refused_and_writes_nothing() {
    use std::os::unix::ffi::OsStrExt;
    assert_name_refused(OsStr::from_bytes(b"demo\xff"));
}

#[test]
fn a_name_of_sixty_four_bytes_is_accepted() {
    let name = format!("{}-{}", "a".repeat(31), "b".repeat(32));
    assert_eq!(name.len(), 64);
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let output = plasmid_in(scratch.path(), &["new", name.as_str()]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(entries(scratch.path()), [format!("{name}.toml")]);
}

#[test]
fn plasmid_new_without_exactly_one_name_is_a_usage_error_that_writes_nothing() {
    for arguments in [vec!["new"], vec!["new", "demo", "other"]] {
        let scratch = tempfile::tempdir().expect("a scratch directory");
        let output = plasmid_in(scratch.path(), &arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}: {output:?}");
        let stderr = text(&output.stderr);
        assert!(
            stderr.contains("exactly one name"),
            "{arguments:?}: {stderr}"
        );
        assert_eq!(
            entries(scratch.path()),
            Vec::<String>::new(),
            "{arguments:?}"
        );
    }
}

#[test]
fn plasmid_help_describes_writing_a_declaration() {
    for flag in ["--help", "-h"] {
        let output = Command::new(env!("CARGO_BIN_EXE_plasmid"))
            .arg(flag)
            .output()
            .expect("the plasmid binary runs");
        assert_eq!(output.status.code(), Some(0), "{flag}");
        assert!(output.stderr.is_empty(), "{flag}: {output:?}");
        let stdout = text(&output.stdout);
        assert!(stdout.contains("plasmid new <name>"), "{flag}: {stdout}");
        assert!(stdout.contains("declaration"), "{flag}: {stdout}");
        for stale in ["P1 freeze groundwork", "scaffold a plasmid crate"] {
            assert!(
                !stdout.contains(stale),
                "{flag} still says {stale:?}: {stdout}"
            );
        }
    }
}

#[test]
fn an_unknown_verb_is_a_named_refusal() {
    let output = Command::new(env!("CARGO_BIN_EXE_plasmid"))
        .arg("attach")
        .output()
        .expect("the plasmid binary runs");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown verb `attach`"), "{stderr}");
}
