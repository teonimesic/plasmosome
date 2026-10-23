use plasmosome_core::{ManifestError, PlasmidManifest, WireError};
use toml_edit::{DocumentMut, Key, TableLike};

const BASE: &str =
    "id = 'strict'\ndescription = 'Test strict declarations.'\nimpl.wasm = 'c.wasm'\n";

fn refusal(text: &str, expected: &str) -> String {
    let error = PlasmidManifest::parse(text).expect_err(text);
    let ManifestError::Field {
        plasmid,
        field,
        fix,
        ..
    } = &error
    else {
        panic!("{error:?}");
    };
    assert_eq!(field, expected, "{text}");
    assert_eq!(plasmid.as_deref(), Some("strict"));
    let wire = serde_json::to_value(WireError::manifest_refusal("p.toml".into(), &error)).unwrap();
    assert_eq!(wire["code"], 108);
    assert_eq!(wire["field"], *field);
    assert_eq!(wire["fix"], *fix);
    assert_eq!(wire["plasmid"], "strict");
    assert!(error.to_string().len() <= 4096);
    fix.clone()
}

fn apply(text: &str, field: &str, fix: &str) -> String {
    let mut document: DocumentMut = text.parse().unwrap();
    let keys = Key::parse(field).unwrap();
    let (last, parents) = keys.split_last().unwrap();
    let mut parent: &mut dyn TableLike = document.as_table_mut();
    for key in parents {
        parent = parent
            .get_mut(key.get())
            .unwrap()
            .as_table_like_mut()
            .unwrap();
    }
    if fix == "remove this entry" || fix.starts_with('#') {
        parent.remove(last.get()).unwrap();
    } else if fix.starts_with('[') {
        assert!(parents.is_empty());
        parent.remove(last.get());
        return format!("{document}\n{fix}\n");
    } else {
        let line: DocumentMut = fix.parse().unwrap();
        assert_eq!(
            line.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            [last.get()]
        );
        parent.insert(last.get(), line[last.get()].clone());
    }
    format!(
        "{document}\n{}",
        if fix.starts_with('#') { fix } else { "" }
    )
}

fn unknowns() -> Vec<(&'static str, &'static str)> {
    vec![
        ("typo = true", "typo"),
        ("[impl]\nwasm = 'c.wasm'\ntypo = true", "impl.typo"),
        (
            "[network]\nhosts = ['api.example.com']\nports = [443]\ntypo = true",
            "network.typo",
        ),
        (
            "[requires]\ncapabilities = []\ntypo = true",
            "requires.typo",
        ),
        ("[provides.x]\ntools = {}\ntypo = true", "provides.x.typo"),
        ("[secrets]\nrefs = []\ntypo = true", "secrets.typo"),
        (
            "[secrets]\nrefs = [{id = 'r', consumer = 'wasm', typo = true}]",
            "secrets.refs[0].typo",
        ),
        (
            "[[secrets.refs]]\nid = 'r'\nconsumer = 'wasm'\n[secrets.refs.scope]\ntypo = true",
            "secrets.refs[0].scope.typo",
        ),
        (
            "[commands]\ncommands = {git = {exec = ['git']}}\ntypo = true",
            "commands.typo",
        ),
        (
            "[commands.commands.git]\nexec = ['git']\ntypo = true",
            "commands.commands.git.typo",
        ),
        (
            "[commands.commands.git]\nexec = ['git']\n[commands.commands.git.network]\ntypo = true",
            "commands.commands.git.network.typo",
        ),
        (
            "[commands.commands.git]\nexec = ['git']\n[commands.commands.git.secrets]\nrefs = []\ntypo = true",
            "commands.commands.git.secrets.typo",
        ),
        (
            "[commands.commands.git]\nexec = ['git']\n[[commands.commands.git.secrets.refs]]\nid = 'r'\nconsumer = 'wasm'\nsubject = 'git'\ntypo = true",
            "commands.commands.git.secrets.refs[0].typo",
        ),
        (
            "[commands.commands.git]\nexec = ['git']\n[[commands.commands.git.secrets.refs]]\nid = 'r'\nconsumer = 'wasm'\nsubject = 'git'\nscope = {typo = true}",
            "commands.commands.git.secrets.refs[0].scope.typo",
        ),
        ("[workspace]\nmount = {}\ntypo = true", "workspace.typo"),
        ("[workspace]\nmount = {typo = true}", "workspace.mount.typo"),
        (
            "[network]\nhosts = ['api.example.com']\nports = [443]\n[mock]\ntypo = true",
            "mock.typo",
        ),
        (
            "[network]\nhosts = ['api.example.com']\nports = [443]\n[mock.backend]\ntypo = true",
            "mock.backend.typo",
        ),
        ("[model]\ntypo = true", "model.typo"),
        ("[lifecycle]\ntypo = true", "lifecycle.typo"),
    ]
}

#[test]
fn each_table_refuses_unknown_fields_and_the_wire_keeps_the_repair() {
    for (tail, field) in unknowns() {
        let source = if tail.starts_with("[impl]") {
            format!("id = 'strict'\ndescription = 'Test.'\n{tail}")
        } else {
            format!("{BASE}{tail}\n")
        };
        let fix = refusal(&source, field);
        assert_eq!(fix, "# typo = true", "{field}");
        if !field.contains("[0]") {
            PlasmidManifest::parse(&apply(&source, field, &fix)).unwrap();
        } else {
            let repaired = source
                .replace(", typo = true", "")
                .replace("typo = true", "");
            PlasmidManifest::parse(&repaired).unwrap();
            assert!(fix.starts_with("# "));
        }
    }
}

#[test]
fn provenance_is_a_comment_and_unknown_fixes_keep_the_original_text() {
    let plain = PlasmidManifest::parse(BASE).unwrap();
    assert_eq!(
        PlasmidManifest::parse(&format!("{BASE}# provenance: generated\n")).unwrap(),
        plain
    );
    for line in [
        "provenance  = 'generated'",
        "'odd.key' = [ 1, 2 ]",
        "extra = { inner = 1 }",
        "extra = [\n 1,\n 2,\n]",
    ] {
        let source = format!("{BASE}{line}\n");
        let field = if line.starts_with("provenance") {
            "provenance"
        } else if line.starts_with("'") {
            "\"odd.key\""
        } else {
            "extra"
        };
        let fix = refusal(&source, field);
        assert_eq!(
            fix,
            line.lines()
                .map(|l| format!("# {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert_eq!(
            PlasmidManifest::parse(&apply(&source, field, &fix)).unwrap(),
            plain
        );
    }
    let source = format!("{BASE}[extra]\nvalue = 'note'\n");
    let fix = refusal(&source, "extra");
    assert_eq!(fix, "# [extra]\n# value = 'note'");
    assert_eq!(
        PlasmidManifest::parse(&apply(&source, "extra", &fix)).unwrap(),
        plain
    );
    let source = format!("{BASE}note = '{}'", "é".repeat(3000));
    let fix = refusal(&source, "note");
    assert_eq!(fix, format!("# note = '{}'", "é".repeat(3000)));
}

#[test]
fn top_level_subject_is_removed_and_command_subject_is_accepted() {
    for refs in [
        "refs = [{id = 'r', consumer = 'wasm', subject = 'git'}]",
        "[[secrets.refs]]\nid = 'r'\nconsumer = 'wasm'\nsubject = 'git'",
    ] {
        let top = if refs.starts_with("refs") {
            format!("{BASE}[secrets]\n{refs}")
        } else {
            format!("{BASE}{refs}")
        };
        let fix = refusal(&top, "secrets.refs[0].subject");
        assert_eq!(fix, "remove this entry");
        let error = PlasmidManifest::parse(&top).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("commands.commands.<name>.secrets")
        );
        let mut raw: toml::Value = top.parse().unwrap();
        raw["secrets"]["refs"][0]
            .as_table_mut()
            .unwrap()
            .remove("subject");
        PlasmidManifest::parse(&toml::to_string(&raw).unwrap()).unwrap();
    }
    let source = format!(
        "{BASE}[commands.commands.git]\nexec = ['git']\n[commands.commands.git.secrets]\nrefs = [{{id = 'r', consumer = 'wasm', subject = 'git'}}]"
    );
    let parsed = PlasmidManifest::parse(&source).unwrap();
    assert_eq!(
        parsed.commands.unwrap().commands[0].secrets[0]
            .subject
            .as_deref(),
        Some("git")
    );
}

#[test]
fn every_new_type_refusal_has_a_field_local_fix() {
    for (tail, field) in [
        ("version = 5", "version"),
        ("impl = 5", "impl"),
        ("impl = {wasm = 5}", "impl.wasm"),
        ("requires = 5", "requires"),
        ("requires = {capabilities = 5}", "requires.capabilities"),
        ("workspace = 5", "workspace"),
        ("workspace = {mount = 5}", "workspace.mount"),
        ("mock = 5", "mock"),
        ("mock = {api = 5}", "mock.api"),
        ("mock = {backend = 5}", "mock.backend"),
        ("mock = {backend = {source = 5}}", "mock.backend.source"),
        ("mock = {backend = {kind = 5}}", "mock.backend.kind"),
        ("model = 5", "model"),
        ("model = {endpoint = 5}", "model.endpoint"),
        ("model = {credential = 5}", "model.credential"),
        ("lifecycle = 5", "lifecycle"),
        ("secrets = 5", "secrets"),
        (
            "commands = {address_plan = 5, commands = {git = {exec = ['git']}}}",
            "commands.address_plan",
        ),
        (
            "commands = {commands = {git = {exec = ['git'], subject = 5}}}",
            "commands.commands.git.subject",
        ),
        (
            "commands = {commands = {git = {exec = ['git'], secrets = 5}}}",
            "commands.commands.git.secrets",
        ),
    ] {
        let base = if field.starts_with("impl") {
            "id = 'strict'\ndescription = 'Test.'\n"
        } else {
            BASE
        };
        let source = format!("{base}{tail}\n");
        let fix = refusal(&source, field);
        let written = apply(&source, field, &fix);
        assert!(
            !matches!(
                PlasmidManifest::parse(&written),
                Err(ManifestError::Parse(_))
            ),
            "{written}"
        );
    }
    for value in ["5", "'wrong'", "[]"] {
        let source = format!("{BASE}[workspace]\nmount = {value}");
        let fix = refusal(&source, "workspace.mount");
        assert_eq!(fix, "mount = { dst = \"/workspace\" }");
        PlasmidManifest::parse(&apply(&source, "workspace.mount", &fix)).unwrap();
    }
    for value in ["5", "[]", "{}", "true"] {
        let source = format!("{BASE}[workspace]\nmount = {{backend = {value}}}");
        assert_eq!(
            refusal(&source, "workspace.mount.backend"),
            "remove this entry"
        );
        PlasmidManifest::parse(&apply(
            &source,
            "workspace.mount.backend",
            "remove this entry",
        ))
        .unwrap();
    }
}

#[test]
fn lifecycle_uses_checked_unsigned_values_and_the_drain_range() {
    for key in ["drain_ms", "lease_ms", "idle_ms"] {
        for value in ["-5", "'5s'", "1.5", "[]", "{}"] {
            refusal(
                &format!("{BASE}[lifecycle]\n{key} = {value}"),
                &format!("lifecycle.{key}"),
            );
        }
    }
    for value in [0, 30001] {
        refusal(
            &format!("{BASE}[lifecycle]\ndrain_ms = {value}"),
            "lifecycle.drain_ms",
        );
    }
    for value in [1, 30000] {
        assert_eq!(
            PlasmidManifest::parse(&format!("{BASE}[lifecycle]\ndrain_ms = {value}"))
                .unwrap()
                .drain_ms,
            Some(value)
        );
    }
    for value in [0, 1, i64::MAX] {
        let parsed = PlasmidManifest::parse(&format!(
            "{BASE}[lifecycle]\nlease_ms = {value}\nidle_ms = {value}"
        ))
        .unwrap();
        assert_eq!(parsed, PlasmidManifest::parse(BASE).unwrap());
    }
}

#[test]
fn strict_lists_keep_original_indexes_and_use_trim_blankness() {
    for (prefix, field, key) in [
        ("[requires]", "requires.capabilities", "capabilities"),
        (
            "[commands.commands.git]",
            "commands.commands.git.exec",
            "exec",
        ),
    ] {
        let mut raw: toml::Value = format!("{BASE}{prefix}\n{key} = ['a', 5, 'b', '', ' ']")
            .parse()
            .unwrap();
        for index in [1, 2, 2] {
            let source = toml::to_string(&raw).unwrap();
            assert_eq!(
                refusal(&source, &format!("{field}[{index}]")),
                "remove this entry"
            );
            let array = if key == "exec" {
                &mut raw["commands"]["commands"]["git"]["exec"]
            } else {
                &mut raw["requires"]["capabilities"]
            };
            array.as_array_mut().unwrap().remove(index);
        }
        let parsed = PlasmidManifest::parse(&toml::to_string(&raw).unwrap()).unwrap();
        let values = if key == "exec" {
            parsed.commands.unwrap().commands[0].exec.clone()
        } else {
            parsed.requires
        };
        assert_eq!(values, ["a", "b"]);
        for blank in ["", " ", "\t", "\u{3000}", "\u{00a0}\u{0085}\u{2028}"] {
            let entry = toml::Value::String(blank.into());
            assert_eq!(
                refusal(
                    &format!("{BASE}{prefix}\n{key} = [{entry}]"),
                    &format!("{field}[0]")
                ),
                "remove this entry"
            );
        }
        PlasmidManifest::parse(&format!(
            "{BASE}{prefix}\n{key} = [' a', '\u{feff}', '\u{200b}', '\u{180e}']"
        ))
        .unwrap();
    }
}

#[test]
fn existing_priority_and_defaults_survive_strictness() {
    assert_eq!(PlasmidManifest::parse(BASE).unwrap().version, "0.0.0");
    let model = PlasmidManifest::parse(&format!("{BASE}[model]"))
        .unwrap()
        .model
        .unwrap();
    assert_eq!(model.endpoint, "https://api.openai.com/v1/chat/completions");
    assert_eq!(model.credential, "model-provider/key");
    let mock = PlasmidManifest::parse(&format!(
        "{BASE}[network]\nhosts = ['api.example.com']\nports = [443]\n[mock]"
    ))
    .unwrap()
    .mock
    .unwrap();
    assert_eq!(
        (mock.kind.as_str(), mock.api.as_str(), mock.source.as_str()),
        ("recorded", "github", "")
    );
    let commands =
        PlasmidManifest::parse(&format!("{BASE}[commands.commands.git]\nexec = ['git']"))
            .unwrap()
            .commands
            .unwrap();
    assert_eq!(commands.address_plan, "10.29.0.0/24");
    for backend in ["", "virtiofs", "fuse"] {
        let workspace = PlasmidManifest::parse(&format!(
            "{BASE}[workspace]\nmount = {{backend = '{backend}'}}"
        ))
        .unwrap()
        .workspace
        .unwrap();
        assert_eq!(
            workspace,
            plasmosome_core::manifest::WorkspaceMount {
                dst: "/workspace".into()
            }
        );
    }
    for binding in ["", " ", "\u{3000}"] {
        let key = toml::Value::String(binding.into()).to_string();
        assert_eq!(
            refusal(
                &format!("{BASE}typo = true\n[provides.{key}]\ntool = 5"),
                &format!("provides.{key}")
            ),
            "remove this entry"
        );
        assert_eq!(
            refusal(
                &format!("{BASE}typo = true\n[provides.x.tools]\n{key} = 5"),
                &format!("provides.x.tools.{key}")
            ),
            "remove this entry"
        );
    }
    let error = PlasmidManifest::parse("id = ' '\ntypo = true").unwrap_err();
    let ManifestError::Field { plasmid, field, .. } = error else {
        panic!();
    };
    assert_eq!(field, "id");
    assert!(plasmid.is_none());
}

#[test]
fn the_benchmark_input_and_two_binding_variant_still_parse() {
    let source = include_str!("../benches/manifest_parse.rs");
    let manifest = source
        .split("const MANIFEST: &str = r#\"")
        .nth(1)
        .unwrap()
        .split("\"#;")
        .next()
        .unwrap();
    PlasmidManifest::parse(manifest).unwrap();
    let two = manifest.replace(
        "\n[network]",
        "\"github:issues\" = {tools = {\"issue.read\" = \"Read an issue.\"}}\n[network]",
    );
    assert_eq!(
        PlasmidManifest::parse(&two).unwrap().provides,
        ["github:tools", "github:issues"]
    );
}
