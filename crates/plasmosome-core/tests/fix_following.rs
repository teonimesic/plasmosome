use plasmosome_core::{ManifestError, PlasmidManifest, WireError};
use serde_json::Value as Json;
use toml_edit::{DocumentMut, Item, Key, TableLike, Value};

const DECLARED: &str = "id = \"github-pr\"\ndescription = \"Reach the declared API.\"\n";

const REMOVE: &str = "remove this entry";

const EXAMPLE_HOST: &str = "api.example.com";

const EXAMPLE_PIN: &str = "192.0.2.0/24";

fn split_index(field: &str) -> (&str, Option<usize>) {
    match field
        .strip_suffix(']')
        .and_then(|rest| rest.rsplit_once('['))
    {
        Some((path, index)) => (path, Some(index.parse().unwrap())),
        None => (field, None),
    }
}

fn table_at<'a>(document: &'a mut DocumentMut, keys: &[Key]) -> Option<&'a mut dyn TableLike> {
    let mut item = document.as_item_mut();
    for key in keys {
        item = item.as_table_like_mut()?.get_mut(key.get())?;
    }
    item.as_table_like_mut()
}

fn list<'a>(table: &'a mut dyn TableLike, key: &Key, field: &str) -> &'a mut toml_edit::Array {
    table
        .get_mut(key.get())
        .and_then(Item::as_array_mut)
        .unwrap_or_else(|| panic!("{field} names no list"))
}

fn entry(fix: &str) -> Value {
    fix.parse()
        .unwrap_or_else(|error| panic!("{fix:?} is neither a line nor an entry: {error}"))
}

fn applied(text: &str, field: &str, fix: &str) -> String {
    let mut document: DocumentMut = text.parse().unwrap();
    let (path, index) = split_index(field);
    let keys =
        Key::parse(path).unwrap_or_else(|error| panic!("{field} is not a TOML path: {error}"));
    let (last, parents) = keys.split_last().unwrap();
    if fix.starts_with('[') {
        assert_eq!(index, None, "a section fix for the entry {field}");
        if let Some(parent) = table_at(&mut document, parents) {
            parent.remove(last.get());
        }
        return format!("{}\n{fix}\n", document.to_string().trim_end());
    }
    let parent = table_at(&mut document, parents)
        .unwrap_or_else(|| panic!("{field} has no table to write into"));
    match (index, fix) {
        (Some(index), REMOVE) => {
            list(parent, last, field).remove(index);
        }
        (None, REMOVE) => {
            parent
                .remove(last.get())
                .unwrap_or_else(|| panic!("{field} names nothing to remove"));
        }
        (Some(index), fix) => {
            list(parent, last, field).replace(index, entry(fix));
        }
        (None, fix) => match fix.parse::<DocumentMut>() {
            Ok(line) => {
                let written: Vec<(&str, &Item)> = line.iter().collect();
                assert_eq!(written.len(), 1, "{fix:?} writes one line");
                let (key, value) = written[0];
                assert_eq!(key, last.get(), "{fix:?} must write {field}");
                parent.insert(key, value.clone());
            }
            Err(_) => list(parent, last, field).push(entry(fix)),
        },
    }
    document.to_string()
}

fn followed(start: &str) -> PlasmidManifest {
    let mut text = start.to_string();
    let mut steps: Vec<(String, String)> = Vec::new();
    let mut written: Vec<String> = Vec::new();
    while steps.len() < 8 {
        match PlasmidManifest::parse(&text) {
            Ok(manifest) => return manifest,
            Err(ManifestError::Field { field, fix, .. }) => {
                let step = (field, fix);
                text = applied(&text, &step.0, &step.1);
                assert!(
                    !written.contains(&text),
                    "{start}\n{step:?} after {steps:?} writes a declaration already refused"
                );
                written.push(text.clone());
                steps.push(step);
            }
            Err(error) => {
                panic!("{start}\nafter {steps:?} is refused with no field: {error}\n{text}")
            }
        }
    }
    panic!("{start}\nhas no end after {steps:?}");
}

fn followed_from_the_wire(start: &str) -> PlasmidManifest {
    let mut text = start.to_string();
    let mut steps: Vec<(String, String)> = Vec::new();
    let mut written: Vec<String> = Vec::new();
    while steps.len() < 8 {
        let error = match PlasmidManifest::parse(&text) {
            Ok(manifest) => return manifest,
            Err(error) => error,
        };
        let ManifestError::Field { plasmid, .. } = &error else {
            panic!("{start}\nafter {steps:?} is refused with no field: {error}\n{text}");
        };
        let wire = WireError::manifest_refusal("plasmids/github-pr.toml".to_string(), &error);
        let line = serde_json::to_string(&wire).unwrap();
        let read: Json = serde_json::from_str(&line).unwrap();
        let member = |name: &str| {
            read.get(name)
                .and_then(Json::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| panic!("{start}\nafter {steps:?} the 108 has no {name}: {line}"))
        };
        let step = (member("field"), member("fix"));
        assert_eq!(
            read.get("plasmid"),
            plasmid.as_deref().map(Json::from).as_ref(),
            "{start}\nthe 108's plasmid is the refusal's own: {line}"
        );
        text = applied(&text, &step.0, &step.1);
        assert!(
            !written.contains(&text),
            "{start}\n{step:?} after {steps:?} writes a declaration already refused"
        );
        written.push(text.clone());
        steps.push(step);
    }
    panic!("{start}\nhas no end after {steps:?}");
}

fn granted(manifest: &PlasmidManifest) -> (Vec<String>, Vec<String>) {
    let commands = manifest
        .commands
        .iter()
        .flat_map(|commands| &commands.commands);
    let networks = manifest
        .network
        .iter()
        .chain(commands.filter_map(|command| command.network.as_ref()));
    let mut hosts = Vec::new();
    let mut pins = Vec::new();
    for network in networks {
        hosts.extend(network.hosts.iter().cloned());
        pins.extend(network.pin_cidrs.iter().map(ToString::to_string));
    }
    (hosts, pins)
}

fn chains() -> Vec<(String, Vec<&'static str>, Vec<&'static str>)> {
    let github = vec!["api.github.com"];
    let example = vec![EXAMPLE_HOST];
    let none: Vec<&str> = Vec::new();
    let tails: Vec<(&str, Vec<&str>, Vec<&str>)> = vec![
        ("", none.clone(), none.clone()),
        ("[network]", none.clone(), none.clone()),
        ("[network]\nports = [443]", example.clone(), none.clone()),
        (
            "[network]\nhosts = []\nports = [443]",
            example.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = [443]\nports = [443]",
            example.clone(),
            none.clone(),
        ),
        ("network = 443", none.clone(), none.clone()),
        ("network = \"api.github.com\"", none.clone(), none.clone()),
        ("network = [\"api.github.com\"]", none.clone(), none.clone()),
        (
            "[network]\nhosts = \"api.github.com\"\nports = [443]",
            github.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = 443\nports = [443]",
            example.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = \" \"\nports = [443]",
            example.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"api.github.com\", 443]\nports = [443]",
            github.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = \"a\\\"b\\\\c\"\nports = [443]",
            example.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = \"x\\ny\"\nports = [443]",
            example.clone(),
            none.clone(),
        ),
        ("[network]\nhosts = \"x\\ny\"", none.clone(), none.clone()),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = \"140.82.112.0/20\"",
            github.clone(),
            vec!["140.82.112.0/20"],
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = 20",
            github.clone(),
            vec![EXAMPLE_PIN],
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = [\"140.82.112.0/20\", 20]",
            github.clone(),
            vec!["140.82.112.0/20"],
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = [20]",
            github.clone(),
            vec![EXAMPLE_PIN],
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = [20, 24]",
            github.clone(),
            vec![EXAMPLE_PIN],
        ),
        (
            "[mock]\nhosts = [\"api.github.com\"]",
            none.clone(),
            none.clone(),
        ),
        (
            "[mock]\nhosts = \"api.github.com\"",
            none.clone(),
            none.clone(),
        ),
        (
            "[mock]\nhosts = [\"a.example\", \"b.example\"]",
            none.clone(),
            none.clone(),
        ),
        ("[mock]\nhosts = []", none.clone(), none.clone()),
        ("[mock]\nhosts = 5", none.clone(), none.clone()),
        ("mock = 5", none.clone(), none.clone()),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n[mock]\nhosts = \"api.github.com\"",
            github.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n[mock]\nhosts = [\"api.github.com\", 443]",
            github.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n[mock]\nhosts = 5",
            github.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n[mock]\nhosts = [[\"api.github.com\"]]",
            github.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n[mock]\nhosts = [\"api.github.example\"]",
            vec!["api.github.com", "api.github.example"],
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"a.example\"]\nports = [443]\n\
             [mock]\nhosts = [\"a.example\", \"b.example\", \"c.example\"]",
            vec!["a.example", "b.example", "c.example"],
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n[mock]\nhosts = [\"b\\nc\"]",
            github.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\n[mock]\nhosts = [\"b\\nc\"]",
            none.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = []\nports = [443]\n[mock]\nhosts = [\"api.github.com\"]",
            github.clone(),
            none.clone(),
        ),
        (
            "[network]\nports = [443]\n[mock]\nhosts = [\"api.github.com\", \"uploads.github.com\"]",
            vec!["api.github.com", "uploads.github.com"],
            none.clone(),
        ),
        (
            "[network]\nhosts = 443\nports = [443]\n[mock]\nhosts = [\"api.github.com\"]",
            github.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = [443]\nports = [443]\n[mock]\nhosts = [\"api.github.com\"]",
            github.clone(),
            none.clone(),
        ),
        (
            "network = 443\n[mock]\nhosts = [\"api.github.com\"]",
            none.clone(),
            none.clone(),
        ),
        (
            "network = \"api.github.com\"\n[mock]\nhosts = [\"api.github.com\"]",
            none.clone(),
            none.clone(),
        ),
        (
            "network = [\"a.example\", \"a.example\"]",
            none.clone(),
            none.clone(),
        ),
        (
            "[mock]\nhosts = [\"a.example\", \"a.example\"]",
            none.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = []\nports = [443]\n[mock]\nhosts = [\"a.example\", \"a.example\"]",
            vec!["a.example"],
            none.clone(),
        ),
        (
            "[network]\nhosts = 443\nports = [443]\n[mock]\nhosts = [\"a.example\", \"a.example\"]",
            vec!["a.example"],
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\npin_cidrs = [20, \"10.0.0.0/8\"]",
            github.clone(),
            vec!["10.0.0.0/8"],
        ),
        (
            "impl.wasm = \"pr.wasm\"\n[network]\nhosts = [\"api.github.com\"]\nports = [65536]",
            none.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"a.example\", \"b.example\"]\nports = []",
            none.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443, 8080, 443]",
            github.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"API.github.com\", \"a.example\", \"A.example\"]\nports = [443]",
            vec!["api.github.com", "a.example"],
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"192.0.2.1\", \"api.github.com\", \"api.github.com\"]\n\
             ports = [443]",
            github.clone(),
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n\
             pin_cidrs = [\"nonsense\", \"192.0.2.1/24\", \"also nonsense\"]",
            github.clone(),
            vec![EXAMPLE_PIN],
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\nports = [443]\n\
             [mock]\nhosts = [\"API.github.com\", \"uploads.github.com\"]",
            vec!["api.github.com", "uploads.github.com"],
            none.clone(),
        ),
        (
            "[network]\nhosts = [\"api.github.com\"]\n\
             [mock]\nhosts = [\"api.github.com\", \"uploads.github.com\"]",
            none.clone(),
            none.clone(),
        ),
        ("[workspace]", none.clone(), none.clone()),
        (
            "[workspace]\nmount = { dst = \"/a//b\" }",
            none.clone(),
            none.clone(),
        ),
        ("[impl]\nwasm = 5", none.clone(), none.clone()),
        ("impl = \"x.wasm\"", none.clone(), none.clone()),
        ("commands = 5", none.clone(), none.clone()),
        ("commands = \"gh\"", none.clone(), none.clone()),
        (
            "[commands]\naddress_plan = \"10.29.0.0/24\"",
            none.clone(),
            none.clone(),
        ),
        ("[commands]\ncommands = 5", none.clone(), none.clone()),
        (
            "[commands]\ncommands = [\"gh\"]",
            none.clone(),
            none.clone(),
        ),
        (
            "[commands]\n[commands.commands]",
            none.clone(),
            none.clone(),
        ),
        (
            "[commands.commands]\ngit = \"x\"",
            none.clone(),
            none.clone(),
        ),
        (
            "[commands.commands.git]\nsubject = \"git\"",
            none.clone(),
            none.clone(),
        ),
        (
            "[commands.commands.git]\nexec = []",
            none.clone(),
            none.clone(),
        ),
        (
            "[commands.commands.gh]\nexec = \"/opt/bin/gh\"",
            none.clone(),
            none.clone(),
        ),
        (
            "[commands.commands.\"\"]\nsubject = \"git\"",
            none.clone(),
            none.clone(),
        ),
        (
            "[commands.commands.\"café\"]\nsubject = \"git\"",
            none.clone(),
            none.clone(),
        ),
        ("commands = [\"gh\", \"gh\"]", none.clone(), none.clone()),
        (
            "[commands]\ncommands = [\"gh\", \"gh\"]",
            none.clone(),
            none.clone(),
        ),
        ("commands = [\"gh\", \"git\"]", none.clone(), none.clone()),
        (
            "[commands.commands.\"a\\nb\"]\nsubject = \"git\"",
            none.clone(),
            none.clone(),
        ),
        (
            "[commands.commands.\"git ops\"]\nexec = [\"git\"]\nnetwork = \"alpha.ak.local\"",
            vec!["alpha.ak.local"],
            none.clone(),
        ),
        (
            "[commands.commands.'say \"hi\"']\nexec = [\"git\"]\nnetwork = 443",
            example.clone(),
            none.clone(),
        ),
        (
            "[commands.commands.git]\nexec = [\"git\"]\n\
             [commands.commands.git.network]\nhosts = \"alpha.ak.local\"",
            vec!["alpha.ak.local"],
            none.clone(),
        ),
        (
            "[commands.commands.git]\nexec = [\"git\"]\n\
             [commands.commands.git.network]\nhosts = [\"alpha.ak.local\", 443]",
            vec!["alpha.ak.local"],
            none.clone(),
        ),
        (
            "[commands.commands.git]\nexec = [\"git\"]\n\
             [commands.commands.git.network]\nhosts = [\"alpha.ak.local\"]\npin_cidrs = [24]",
            vec!["alpha.ak.local"],
            vec![EXAMPLE_PIN],
        ),
        (
            "[commands.commands.git]\nexec = [\"git\"]\n\
             [commands.commands.git.network]\nhosts = [\"alpha.ak.local\"]\n\
             pin_cidrs = [\"10.29.0.0/24\", 24]",
            vec!["alpha.ak.local"],
            vec!["10.29.0.0/24"],
        ),
    ];
    let mut chains: Vec<(String, Vec<&str>, Vec<&str>)> = tails
        .into_iter()
        .map(|(tail, hosts, pins)| (format!("{DECLARED}{tail}\n"), hosts, pins))
        .collect();
    for start in [
        "id = \"\"\ndescription = \"Reach the declared API.\"\nimpl.wasm = \"pr.wasm\"\n",
        "id = \"\"\nimpl.wasm = \"pr.wasm\"\n",
        "id = 5\ndescription = \"Reach the declared API.\"\nimpl.wasm = \"pr.wasm\"\n",
    ] {
        chains.push((start.to_string(), none.clone(), none.clone()));
    }
    chains
}

#[test]
fn following_each_fix_as_written_ends_granting_only_the_hosts_and_pins_the_author_named() {
    for (start, hosts, pins) in chains() {
        let manifest = followed(&start);
        assert_eq!(
            granted(&manifest),
            (strings(&hosts), strings(&pins)),
            "{start}"
        );
    }
}

#[test]
fn a_client_following_each_fix_from_the_wire_alone_ends_where_the_parser_does() {
    let mut starts: Vec<String> = chains().into_iter().map(|(start, _, _)| start).collect();
    starts.push(
        "id = \"x: network.hosts\"\ndescription = \"Run git.\"\n\
         [commands.commands.\"a: b\"]\nexec = [\"git\"]\n\
         [commands.commands.\"a: b\".network]\nhosts = [\"api.github.com\", 5]\n"
            .to_string(),
    );
    starts.push(
        "id = \"x: network.hosts\"\ndescription = \"Reach the declared API.\"\n\
         [network]\nports = [443]\n"
            .to_string(),
    );
    for start in starts {
        assert_eq!(followed_from_the_wire(&start), followed(&start), "{start}");
    }
}

#[test]
fn a_command_whose_name_is_blank_is_given_the_command_name_program() {
    for key in ["\"\"", "\" \""] {
        let manifest = followed(&format!(
            "{DECLARED}[commands.commands.{key}]\nsubject = \"git\"\n"
        ));
        let commands = manifest.commands.unwrap().commands;
        assert_eq!(commands[0].exec, vec!["command-name".to_string()], "{key}");
    }
}

#[test]
fn a_command_fix_runs_the_program_the_author_wrote() {
    for (tail, exec) in [
        ("commands = \"gh\"", "gh"),
        ("[commands]\ncommands = [\"gh\"]", "gh"),
        (
            "[commands.commands]\ngit = \"/usr/bin/git\"",
            "/usr/bin/git",
        ),
        (
            "[commands.commands.gh]\nexec = \"/opt/bin/gh\"",
            "/opt/bin/gh",
        ),
        ("[commands.commands.git]\nexec = []", "git"),
        ("commands = 5", "git"),
        ("[commands]\n[commands.commands]", "git"),
        ("commands = [\"gh\", \"gh\"]", "gh"),
        ("[commands]\ncommands = [\"gh\", \"gh\"]", "gh"),
        ("[commands.commands]\ngit = \" \"", "git"),
        ("[commands.commands.git]\nexec = \" \"", "git"),
        ("[commands.commands.\"café\"]\nsubject = \"git\"", "café"),
    ] {
        let manifest = followed(&format!("{DECLARED}{tail}\n"));
        let commands = manifest.commands.unwrap().commands;
        assert_eq!(commands.len(), 1, "{tail}");
        assert_eq!(commands[0].exec, vec![exec.to_string()], "{tail}");
    }
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

#[test]
fn a_commands_fix_writes_each_name_the_author_wrote_once() {
    for tail in [
        "commands = [\"gh\", \"git\", \"gh\"]",
        "[commands]\ncommands = [\"gh\", \"git\", \"gh\"]",
    ] {
        let manifest = followed(&format!("{DECLARED}{tail}\n"));
        let commands = manifest.commands.unwrap().commands;
        let written: Vec<(String, Vec<String>)> = commands
            .into_iter()
            .map(|command| (command.id, command.exec))
            .collect();
        assert_eq!(
            written,
            vec![
                ("gh".to_string(), strings(&["gh"])),
                ("git".to_string(), strings(&["git"]))
            ],
            "{tail}"
        );
    }
}

#[test]
fn a_section_fix_inside_an_inline_table_does_not_apply_today() {
    let start =
        format!("{DECLARED}[commands.commands]\ngit = {{ exec = [\"git\"], network = 443 }}\n");
    let Err(ManifestError::Field { field, fix, .. }) = PlasmidManifest::parse(&start) else {
        panic!("{start} is refused at the command's network");
    };
    assert_eq!(
        (field.as_str(), fix.as_str()),
        (
            "commands.commands.git.network",
            "[commands.commands.git.network]\nhosts = [\"api.example.com\"]"
        )
    );
    let written = applied(&start, &field, &fix);
    assert!(
        matches!(
            PlasmidManifest::parse(&written),
            Err(ManifestError::Parse(_))
        ),
        "a [table] header cannot add to an inline table, so this fix is not TOML once \
         written:\n{written}"
    );
}
