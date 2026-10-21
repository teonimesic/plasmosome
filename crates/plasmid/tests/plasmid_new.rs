use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use plasmosome_core::{ManifestError, PlasmidManifest};

mod support;

use support::{entries, text, usage_error};

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

fn working_directory() -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let work = scratch.path().join("work");
    std::fs::create_dir(&work).expect("the working directory is made");
    (scratch, work)
}

fn printed(stdout: &str) -> Vec<(String, String)> {
    stdout
        .lines()
        .map(|line| {
            let (section, shown) = line
                .strip_prefix("add ")
                .and_then(|rest| rest.split_once(": "))
                .unwrap_or_else(|| {
                    panic!("each stdout line names one section as `add <section>: ...`: {line:?}")
                });
            (section.to_string(), shown.to_string())
        })
        .collect()
}

fn printed_sections(stdout: &str) -> Vec<String> {
    printed(stdout)
        .into_iter()
        .map(|(section, _)| section)
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

fn refusals(declaration: &str) -> Vec<(String, String)> {
    let mut declaration = declaration.to_string();
    let mut refused: Vec<(String, String)> = Vec::new();
    loop {
        match PlasmidManifest::parse(&declaration) {
            Ok(manifest) => {
                assert_eq!(manifest.id, "demo");
                return refused;
            }
            Err(ManifestError::Field {
                plasmid,
                field,
                fix,
                ..
            }) => {
                assert_eq!(plasmid.as_deref(), Some("demo"), "{field}");
                assert!(
                    refused.iter().all(|(seen, _)| *seen != field),
                    "{field} is refused again after its fix was applied:\n{declaration}"
                );
                declaration = applied(&declaration, &field, &fix);
                refused.push((field, fix));
            }
            Err(other) => panic!("the scaffold is refused for no missing section: {other}"),
        }
    }
}

#[test]
fn plasmid_new_writes_one_declaration_and_nothing_else() {
    let (scratch, work) = working_directory();
    let output = plasmid_in(&work, &["new", "demo"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(entries(&work), ["demo.toml"]);
    assert_eq!(entries(scratch.path()), ["work"]);
    let written = std::fs::symlink_metadata(work.join("demo.toml")).expect("demo.toml exists");
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
    let refused: Vec<String> = refusals(&scaffold.declaration)
        .into_iter()
        .map(|(field, _)| field)
        .collect();
    assert_eq!(printed_sections(&scaffold.stdout), refused);
}

#[test]
fn each_printed_section_shows_the_fix_the_parser_gives_for_it() {
    let scaffold = scaffold("demo");
    let shown = printed(&scaffold.stdout);
    let refused = refusals(&scaffold.declaration);
    assert_eq!(shown.len(), refused.len(), "{}", scaffold.stdout);
    for ((section, line), (field, fix)) in shown.iter().zip(&refused) {
        assert_eq!(section, field);
        let mut rest = line.as_str();
        for fix_line in fix.lines() {
            let at = rest.find(fix_line).unwrap_or_else(|| {
                panic!(
                    "the line for {field} shows the parser's fix {fix:?} line by line, in order, \
                     and {fix_line:?} is missing from what follows in {line:?}"
                )
            });
            rest = &rest[at + fix_line.len()..];
        }
    }
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

#[cfg(unix)]
#[test]
fn a_declaration_it_cannot_finish_writing_exits_3() {
    let (scratch, work) = working_directory();
    let output = Command::new("/bin/sh")
        .current_dir(&work)
        .args([
            "-c",
            "trap '' XFSZ; ulimit -f 0; exec \"$0\" new demo",
            env!("CARGO_BIN_EXE_plasmid"),
        ])
        .output()
        .expect("sh runs plasmid under a file size limit of zero");
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(entries(&work), Vec::<String>::new(), "{output:?}");
    assert_eq!(entries(scratch.path()), ["work"]);
    let stderr = text(&output.stderr);
    for named in [
        "cannot finish writing demo.toml",
        "File too large",
        "nothing was written",
    ] {
        assert!(stderr.contains(named), "stderr names {named:?}: {stderr}");
    }

    let second = plasmid_in(&work, &["new", "demo"]);
    assert_eq!(second.status.code(), Some(2), "{second:?}");
    assert_eq!(entries(&work), ["demo.toml"]);
}

#[cfg(unix)]
#[test]
fn a_directory_it_cannot_write_into_is_refused_and_nothing_is_written() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let locked = scratch.path().join("locked");
    std::fs::create_dir(&locked).expect("the locked directory is made");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555))
        .expect("the directory is made read-only");
    let bypassed = std::fs::write(locked.join("probe"), "").is_ok();
    let output = (!bypassed).then(|| plasmid_in(&locked, &["new", "demo"]));
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
        .expect("the directory is made writable again");
    let Some(output) = output else {
        eprintln!(
            "skipped a_directory_it_cannot_write_into_is_refused_and_nothing_is_written: this \
             process writes into a read-only directory, as root does"
        );
        return;
    };
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = text(&output.stderr);
    assert!(stderr.contains("cannot create demo.toml"), "{stderr}");
    assert_eq!(entries(&locked), Vec::<String>::new());
}

fn assert_name_refused<S: AsRef<OsStr> + std::fmt::Debug>(name: S) {
    let (scratch, work) = working_directory();
    let name = name.as_ref();
    let output = plasmid_in(&work, &[OsStr::new("new"), name]);
    let error = usage_error(&output);
    assert_eq!(
        error["argument"],
        name.to_string_lossy().as_ref(),
        "{name:?}: {output:?}"
    );
    if name.to_str().is_some_and(|name| !name.starts_with('-')) {
        let message = error["message"].as_str().expect("the message is a string");
        assert!(message.contains(NOT_A_NAME), "{name:?}: {message}");
    }
    assert_eq!(entries(&work), Vec::<String>::new(), "{name:?}");
    assert_eq!(entries(scratch.path()), ["work"], "{name:?}");
}

#[test]
fn a_name_outside_the_registry_grammar_is_refused_and_writes_nothing() {
    let long = "a".repeat(65);
    let long_with_hyphen = format!("{}-{}", "a".repeat(32), "b".repeat(32));
    assert_eq!(long_with_hyphen.len(), 65);
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
        long_with_hyphen.as_str(),
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
    for (arguments, argument) in [
        (vec!["new"], None),
        (vec!["new", "demo", "other"], Some("other")),
    ] {
        let scratch = tempfile::tempdir().expect("a scratch directory");
        let output = plasmid_in(scratch.path(), &arguments);
        let error = usage_error(&output);
        match argument {
            Some(argument) => assert_eq!(error["argument"], argument, "{arguments:?}"),
            None => assert!(error.get("argument").is_none(), "{arguments:?}: {error:?}"),
        }
        let message = error["message"].as_str().expect("the message is a string");
        assert!(
            message.contains("exactly one name"),
            "{arguments:?}: {message}"
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
fn plasmid_new_refuses_what_does_not_apply_to_it() {
    for (arguments, argument) in [
        (&["new", "demo", "--kernel", "a"][..], "--kernel"),
        (&["--kernel", "a", "new", "demo"], "--kernel"),
        (&["new", "demo", "--socket", "/x"], "--socket"),
        (&["new", "demo", "--deadline-ms", "5"], "--deadline-ms"),
        (&["new", "demo", "--json"], "--json"),
        (&["new", "demo", "--frob"], "--frob"),
        (&["new", "demo", "--"], "--"),
        (&["new", "--json", "demo"], "--json"),
        (&["new", "demo", "--kernel", "a", "--json"], "--kernel"),
    ] {
        let (scratch, work) = working_directory();
        let error = usage_error(&plasmid_in(&work, arguments));
        assert_eq!(error["argument"], argument, "{arguments:?}");
        assert_eq!(entries(&work), Vec::<String>::new(), "{arguments:?}");
        assert_eq!(entries(scratch.path()), ["work"], "{arguments:?}");
    }
    let (_scratch, work) = working_directory();
    let output = plasmid_in(&work, &["new", "demo", "--human"]);
    assert_eq!(output.status.code(), Some(64), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = text(&output.stderr);
    assert!(
        stderr.ends_with("(client invalid_argument)\n"),
        "{stderr:?}"
    );
    assert_eq!(entries(&work), Vec::<String>::new());

    let output = plasmid_in(&work, &["new", "demo", "--kernel", "a", "--human"]);
    assert_eq!(output.status.code(), Some(64), "{output:?}");
    let stderr = text(&output.stderr);
    let (message, _) = stderr.split_once('\n').expect("two lines");
    assert!(
        message.contains("--kernel") && !message.contains("--human"),
        "{stderr:?}"
    );
    assert_eq!(entries(&work), Vec::<String>::new());
}

#[test]
fn plasmid_new_names_the_first_fault_on_its_line() {
    for (arguments, argument) in [
        (&["new", "Demo", "--json"][..], "Demo"),
        (&["new", "--json", "Demo"], "--json"),
        (&["new", "Demo", "--frob"], "Demo"),
        (&["new", "--frob", "Demo"], "--frob"),
        (&["new", "Demo", "--"], "Demo"),
        (&["new", "--", "Demo"], "--"),
        (&["new", "Demo", "--kernel", "a"], "Demo"),
        (&["new", "--kernel", "a", "Demo"], "--kernel"),
        (&["--kernel", "a", "new", "Demo"], "--kernel"),
        (&["--kernel", "../x", "new", "demo"], "--kernel"),
        (&["--kernel", "a", "--frob", "new", "demo"], "--frob"),
        (&["--kernel", "a", "--frob=x", "new", "demo"], "--kernel"),
        (&["--kernel", "a", "-x=1", "new", "demo"], "-x=1"),
        (&["--frob=x", "new", "demo"], "--frob=x"),
        (&["--json", "--human", "new", "demo"], "--json"),
        (&["new", "a", "b", "--json"], "b"),
        (&["new", "a", "--json", "b"], "--json"),
        (&["new", "a", "b", "--"], "b"),
        (&["new", "a", "--", "b"], "--"),
        (&["new", "Demo", "b"], "Demo"),
        (&["new", "a", "B"], "B"),
    ] {
        let (scratch, work) = working_directory();
        let error = usage_error(&plasmid_in(&work, arguments));
        assert_eq!(error["argument"], argument, "{arguments:?}");
        assert_eq!(entries(&work), Vec::<String>::new(), "{arguments:?}");
        assert_eq!(entries(scratch.path()), ["work"], "{arguments:?}");
    }
}
