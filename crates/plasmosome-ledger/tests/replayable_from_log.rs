use std::io::Write;
use std::time::Duration;

use plasmosome_backend::{
    Capability, Diff, DrainSpec, EnforcementBackend, FakeBackend, Grant, GrantKind, PluginId,
    UniverseOp, UniverseRemoval,
};
use plasmosome_ledger::{Closure, Effect, InverseVia, Ledger};

fn populate(backend: &mut FakeBackend) -> (PluginId, Vec<Effect>) {
    backend
        .apply(UniverseOp::WriteSessionFile {
            path: "skills/pr.md".to_string(),
            owner: PluginId::from("github-pr"),
        })
        .unwrap();
    let entry = backend.grant(Grant {
        plugin: PluginId::from("network"),
        capability: Capability::UdsSocket {
            path: "/run/plasmosome/egressd.uds".to_string(),
        },
        kind: GrantKind::Hot,
    });
    let effects = vec![
        Effect::exact(
            "injected skill file",
            InverseVia::Universe(UniverseRemoval::RemoveSessionFile {
                path: "skills/pr.md".to_string(),
            }),
        ),
        Effect::exact("bound egress socket", InverseVia::Backend(entry.handle)),
        Effect::delayed_unpublished("outbox/github", "pr-comment payload"),
    ];
    (PluginId::from("github-pr"), effects)
}

#[test]
fn a_ledger_rebuilt_from_its_log_replays_to_an_empty_universe() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let mut backend = FakeBackend::new();
    let before = backend.snapshot_os_state();
    let (plugin, effects) = populate(&mut backend);

    {
        let mut ledger = Ledger::new(plugin.clone());
        for effect in &effects {
            ledger.push(effect.clone());
        }
        let written = ledger
            .append_to_file(&log)
            .expect("the ledger appends to its log");
        assert_eq!(written, effects.len());
    }

    let rebuilt = Ledger::open_file(&log).expect("the log alone must rebuild the ledger");
    assert_eq!(rebuilt.plugin(), &plugin);
    assert_eq!(rebuilt.len(), effects.len());
    assert_eq!(rebuilt.effects(), &effects);

    let Closure::ExternalFree(mut sealed) = rebuilt.close() else {
        panic!("a log of exact and unpublished-delayed entries must close external-free");
    };
    let report = sealed
        .detach(&mut backend, DrainSpec::graceful(Duration::from_millis(1)))
        .unwrap();
    assert_eq!(
        report.replayed,
        vec![
            "bound egress socket".to_string(),
            "injected skill file".to_string()
        ],
        "the rebuilt ledger replays LIFO exactly as the in-memory one"
    );
    assert_eq!(report.delayed_discarded, 1);
    let after = backend.snapshot_os_state();
    assert!(
        Diff::between(&before, &after).is_empty(),
        "replay of the rebuilt ledger must restore the pre-attach universe"
    );
}

#[test]
fn a_log_rebuilt_in_a_fresh_process_round_trips_through_serde_only() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let mut backend = FakeBackend::new();
    let (plugin, effects) = populate(&mut backend);
    let mut ledger = Ledger::new(plugin.clone());
    for effect in &effects {
        ledger.push(effect.clone());
    }
    ledger.append_to_file(&log).unwrap();

    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        text.lines().count(),
        effects.len(),
        "one ndjson line per effect"
    );
    for line in text.lines() {
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(record["plugin"], "github-pr");
        assert!(record["effect"]["description"].is_string());
        assert!(record["effect"]["reversibility"].is_object());
    }
    drop(text);

    let reopened = Ledger::open_file(&log).unwrap();
    assert_eq!(reopened.plugin(), &plugin);
    assert_eq!(reopened.effects(), &effects);
}

#[test]
fn a_crash_truncated_final_line_costs_only_its_own_entry() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let mut ledger = Ledger::new("github-pr");
    ledger.push(Effect::exact(
        "entry one",
        InverseVia::Universe(UniverseRemoval::RemoveSessionFile {
            path: "skills/a.md".to_string(),
        }),
    ));
    ledger.push(Effect::exact(
        "entry two",
        InverseVia::Universe(UniverseRemoval::RemoveSessionFile {
            path: "skills/b.md".to_string(),
        }),
    ));
    ledger.append_to_file(&log).unwrap();

    let mut torn = std::fs::read_to_string(&log).unwrap();
    torn.truncate(torn.len() - 12);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&log)
        .unwrap();
    file.write_all(torn.as_bytes()).unwrap();
    drop(file);

    let reopened = Ledger::open_file(&log).expect("a torn final line must not lose the log");
    assert_eq!(reopened.len(), 1, "only the torn entry is lost");
    assert_eq!(reopened.effects()[0].description, "entry one");
}

#[test]
fn a_log_whose_lines_disagree_on_the_plugin_is_a_named_error() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let mut ledger = Ledger::new("github-pr");
    ledger.push(Effect::exact(
        "entry one",
        InverseVia::Universe(UniverseRemoval::RemoveSessionFile {
            path: "skills/a.md".to_string(),
        }),
    ));
    ledger.append_to_file(&log).unwrap();
    let mut other = Ledger::new("model-provider");
    other.push(Effect::exact(
        "entry two",
        InverseVia::Universe(UniverseRemoval::RemoveSessionFile {
            path: "skills/b.md".to_string(),
        }),
    ));
    let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
    other.write_to(&mut file).unwrap();
    drop(file);

    let err = Ledger::open_file(&log).unwrap_err();
    assert!(
        err.to_string().contains("model-provider"),
        "the error must name the disagreeing plugin: {err}"
    );
}
