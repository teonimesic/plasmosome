use std::io::Write;
use std::time::Duration;

use plasmosome_backend::{
    Capability, Diff, DrainSpec, EnforcementBackend, FakeBackend, Grant, GrantId, GrantKind,
    LedgerEntry, PluginId, UniverseClass, UniverseOp, UniverseRemoval,
};
use plasmosome_ledger::{Closure, Effect, InverseVia, Ledger, LogRecord};

fn file_removal(path: &str) -> UniverseRemoval {
    UniverseRemoval {
        id: GrantId::new(),
        capability: Capability::SessionFile {
            path: path.to_string(),
        },
    }
}

fn wire_capabilities() -> Vec<Capability> {
    vec![
        Capability::SessionFile {
            path: "skills/pr.md".to_string(),
        },
        Capability::UdsSocket {
            path: "/run/plasmosome/egressd.uds".to_string(),
        },
        Capability::ProxyMap {
            host: "api.github.com".to_string(),
            route: "splice".to_string(),
        },
        Capability::Broker {
            pid: 31337,
            name: "egressd".to_string(),
        },
        Capability::Mount {
            source: "/secrets".to_string(),
            target: "/workspace".to_string(),
        },
    ]
}

fn populate(backend: &mut FakeBackend) -> (PluginId, Vec<Effect>) {
    let op = UniverseOp::WriteSessionFile {
        id: GrantId::new(),
        path: "skills/pr.md".to_string(),
        owner: PluginId::from("github-pr"),
    };
    let removal = op.removal();
    backend.apply(op).unwrap();
    let entry = backend.grant(Grant {
        plugin: PluginId::from("network"),
        capability: Capability::UdsSocket {
            path: "/run/plasmosome/egressd.uds".to_string(),
        },
        kind: GrantKind::Hot,
    });
    let effects = vec![
        Effect::exact("injected skill file", InverseVia::Universe(removal)),
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
        assert_eq!(record["format"], 2);
    }
    drop(text);

    let reopened = Ledger::open_file(&log).unwrap();
    assert_eq!(reopened.plugin(), &plugin);
    assert_eq!(reopened.effects(), &effects);
}

#[test]
fn exact_state_wire_preserves_identity_and_refuses_ambiguous_rows() {
    let mut backend = FakeBackend::new();
    backend.grant(Grant {
        plugin: PluginId::from("network"),
        capability: Capability::ProxyMap {
            host: "api.github.com".to_string(),
            route: "splice".to_string(),
        },
        kind: GrantKind::Hot,
    });
    let state = backend.snapshot_os_state();
    let encoded = serde_json::to_string(&state).unwrap();
    let round_trip: plasmosome_backend::OsState = serde_json::from_str(&encoded).unwrap();
    assert_eq!(round_trip, state);

    let object = serde_json::to_value(state.objects().next().unwrap()).unwrap();
    let duplicate = serde_json::json!({"objects": [object.clone(), object.clone()]});
    assert!(serde_json::from_value::<plasmosome_backend::OsState>(duplicate).is_err());
    let mut conflict = object.clone();
    conflict["owner"] = serde_json::json!("audit");
    assert!(
        serde_json::from_value::<plasmosome_backend::OsState>(
            serde_json::json!({"objects": [object.clone(), conflict]})
        )
        .is_err()
    );
    let mut unknown_field = object;
    unknown_field["class"] = serde_json::json!("ProxyMap");
    assert!(
        serde_json::from_value::<plasmosome_backend::OsState>(
            serde_json::json!({"objects": [unknown_field]})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<plasmosome_backend::OsState>(serde_json::json!({
            "objects": [{
                "owner": "deploy",
                "capability": {"ProxyMap": {"host": "api.github.com", "route": "splice"}}
            }]
        }))
        .is_err()
    );
    for invalid in [
        "00000000-0000-0000-0000-000000000000",
        "00000000-0000-1000-8000-000000000001",
        "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
        "00000000-0000-4000-0000-000000000001",
        "00000000-0000-4000-c000-000000000001",
        "00000000-0000-4000-e000-000000000001",
    ] {
        assert!(
            serde_json::from_value::<GrantId>(serde_json::json!(invalid)).is_err(),
            "{invalid} must not decode as a grant identity"
        );
    }
}

#[test]
fn ledger_entry_refuses_serializing_a_class_capability_mismatch() {
    let mut backend = FakeBackend::new();
    let entry = backend.grant(Grant {
        plugin: PluginId::from("network"),
        capability: Capability::SessionFile {
            path: "skills/pr.md".to_string(),
        },
        kind: GrantKind::Hot,
    });
    let encoded = serde_json::to_string(&entry).unwrap();
    assert_eq!(
        serde_json::from_str::<LedgerEntry>(&encoded).unwrap(),
        entry
    );

    let mut invalid = entry;
    invalid.handle.class = UniverseClass::Mount;
    assert!(serde_json::to_string(&invalid).is_err());
}

#[test]
fn log_record_refuses_serializing_an_unsupported_format() {
    let record = LogRecord {
        format: 2,
        plugin: PluginId::from("network"),
        effect: Effect::exact("wire", InverseVia::Universe(file_removal("skills/pr.md"))),
    };
    let encoded = serde_json::to_string(&record).unwrap();
    assert_eq!(serde_json::from_str::<LogRecord>(&encoded).unwrap(), record);

    let mut invalid = record;
    invalid.format = 1;
    assert!(serde_json::to_string(&invalid).is_err());
}

#[test]
fn format_two_refuses_unknown_fields_in_every_nested_capability() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");

    for capability in wire_capabilities() {
        let removal = UniverseRemoval {
            id: GrantId::new(),
            capability,
        };
        for (effect, pointer) in [
            (
                Effect::exact("wire", InverseVia::Universe(removal.clone())),
                "/effect/reversibility/Exact/via/Universe/capability",
            ),
            (
                Effect::compensating("wire", removal.clone()),
                "/effect/reversibility/Compensating/witness/capability",
            ),
        ] {
            let mut ledger = Ledger::new("owner");
            ledger.push(effect.clone());
            let mut valid = Vec::new();
            ledger.write_to(&mut valid).unwrap();
            std::fs::write(&log, &valid).unwrap();
            assert_eq!(
                Ledger::open_file(&log).unwrap().effects(),
                std::slice::from_ref(&effect)
            );

            let mut malformed: serde_json::Value = serde_json::from_slice(&valid).unwrap();
            malformed
                .pointer_mut(pointer)
                .and_then(serde_json::Value::as_object_mut)
                .and_then(|variants| variants.values_mut().next())
                .and_then(serde_json::Value::as_object_mut)
                .expect("a structured capability payload")
                .insert("obsolete".to_string(), serde_json::json!("discarded"));
            let mut encoded = serde_json::to_vec(&malformed).unwrap();
            encoded.push(b'\n');
            std::fs::write(&log, &encoded).unwrap();
            let error = Ledger::open_file(&log).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("line 1"));
            assert_eq!(std::fs::read(&log).unwrap(), encoded);
        }
    }
}

#[test]
fn a_crash_truncated_final_line_costs_only_its_own_entry() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let mut ledger = Ledger::new("github-pr");
    ledger.push(Effect::exact(
        "entry one",
        InverseVia::Universe(file_removal("skills/a.md")),
    ));
    ledger.push(Effect::exact(
        "entry two",
        InverseVia::Universe(file_removal("skills/b.md")),
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
fn a_torn_utf8_tail_preserves_complete_entries_without_rewriting() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let first = Effect::exact(
        "entry one",
        InverseVia::Universe(file_removal("skills/a.md")),
    );
    let mut ledger = Ledger::new("github-pr");
    ledger.push(first.clone());
    ledger.push(Effect::exact(
        "entry €",
        InverseVia::Universe(file_removal("skills/b.md")),
    ));
    let mut encoded = Vec::new();
    ledger.write_to(&mut encoded).unwrap();
    let split = encoded
        .windows("€".len())
        .position(|bytes| bytes == "€".as_bytes())
        .unwrap()
        + 1;
    let torn = &encoded[..split];
    std::fs::write(&log, torn).unwrap();

    let reopened = Ledger::open_file(&log).expect("only the incomplete final record may be lost");
    assert_eq!(reopened.effects(), &[first]);
    assert_eq!(std::fs::read(&log).unwrap(), torn);
}

#[test]
fn invalid_utf8_and_complete_final_records_cannot_be_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let mut ledger = Ledger::new("github-pr");
    ledger.push(Effect::exact(
        "entry",
        InverseVia::Universe(file_removal("skills/a.md")),
    ));
    let mut valid = Vec::new();
    ledger.write_to(&mut valid).unwrap();
    let prefix = br#"{"format":2,"plugin":"github-pr","effect":{"description":""#;
    let mut invalid_byte = prefix.to_vec();
    invalid_byte.push(0xff);
    let mut terminated_sequence = prefix.to_vec();
    terminated_sequence.extend_from_slice(&[0xe2, b'\n']);
    let mut complete_with_junk = valid.strip_suffix(b"\n").unwrap().to_vec();
    complete_with_junk.push(0xe2);

    for tail in [invalid_byte, terminated_sequence, complete_with_junk] {
        let mut input = valid.clone();
        input.extend_from_slice(&tail);
        std::fs::write(&log, &input).unwrap();
        let error = Ledger::open_file(&log).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("line 2"));
        assert_eq!(std::fs::read(&log).unwrap(), input);
    }
}

#[test]
fn a_log_whose_lines_disagree_on_the_plugin_is_a_named_error() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let mut ledger = Ledger::new("github-pr");
    ledger.push(Effect::exact(
        "entry one",
        InverseVia::Universe(file_removal("skills/a.md")),
    ));
    ledger.append_to_file(&log).unwrap();
    let mut other = Ledger::new("model-provider");
    other.push(Effect::exact(
        "entry two",
        InverseVia::Universe(file_removal("skills/b.md")),
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

#[test]
fn exact_neighbours_survive_interrupted_logged_replay_and_resume() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let capability = Capability::UdsSocket {
        path: "/run/plasmosome/egressd.uds".to_string(),
    };
    let mut backend = FakeBackend::new();
    let first = backend.grant(Grant {
        plugin: PluginId::from("network"),
        capability: capability.clone(),
        kind: GrantKind::Hot,
    });
    let second = backend.grant(Grant {
        plugin: PluginId::from("network"),
        capability,
        kind: GrantKind::Hot,
    });
    backend.mark_stuck(first.handle);
    let mut ledger = Ledger::new("network");
    ledger.push(Effect::exact(
        "first equal grant",
        InverseVia::Backend(first.handle),
    ));
    ledger.push(Effect::exact(
        "second equal grant",
        InverseVia::Backend(second.handle),
    ));
    ledger.append_to_file(&log).unwrap();
    let rebuilt = Ledger::open_file(&log).unwrap();
    let Closure::ExternalFree(mut sealed) = rebuilt.close() else {
        panic!("exact effects close external-free");
    };
    assert!(
        sealed
            .detach(&mut backend, DrainSpec::graceful(Duration::from_millis(1)))
            .is_err()
    );
    assert_eq!(
        backend.snapshot_os_state().objects().collect::<Vec<_>>(),
        vec![&first.object()]
    );
    let resumed = sealed
        .detach(&mut backend, DrainSpec::forcing())
        .expect("force resumes at the exact timed-out holding");
    assert_eq!(resumed.replayed, vec!["first equal grant"]);
    assert!(backend.snapshot_os_state().is_empty());
}

#[test]
fn serialized_observations_restore_exact_inverses_but_not_backend_handles() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let capability = Capability::Broker {
        pid: 31337,
        name: "egressd".to_string(),
    };
    let mut source = FakeBackend::new();
    let first = source.grant(Grant {
        plugin: PluginId::from("network"),
        capability: capability.clone(),
        kind: GrantKind::Hot,
    });
    let second = source.grant(Grant {
        plugin: PluginId::from("network"),
        capability,
        kind: GrantKind::Hot,
    });
    let encoded = serde_json::to_string(&source.snapshot_os_state()).unwrap();
    let restored: plasmosome_backend::OsState = serde_json::from_str(&encoded).unwrap();
    let mut fresh = FakeBackend::new();
    for object in restored.objects() {
        fresh.plant(object.clone()).unwrap();
    }
    let before_refusal = fresh.snapshot_os_state();
    assert_eq!(
        fresh
            .revoke(first.handle, DrainSpec::forcing())
            .unwrap_err(),
        plasmosome_backend::BackendError::UnknownHandle {
            handle: first.handle
        }
    );
    assert_eq!(fresh.snapshot_os_state(), before_refusal);

    let mut ledger = Ledger::new("network");
    ledger.push(Effect::exact(
        "first observed grant",
        InverseVia::Universe(first.removal()),
    ));
    ledger.push(Effect::exact(
        "second observed grant",
        InverseVia::Universe(second.removal()),
    ));
    ledger.append_to_file(&log).unwrap();
    let Closure::ExternalFree(mut sealed) = Ledger::open_file(&log).unwrap().close() else {
        panic!("exact observed inverses close external-free");
    };
    sealed
        .detach(&mut fresh, DrainSpec::forcing())
        .expect("serialized exact inverses select both restored observations");
    assert!(fresh.snapshot_os_state().is_empty());
}

#[test]
fn complete_invalid_records_are_refused_without_rewriting_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let mut ledger = Ledger::new("github-pr");
    ledger.push(Effect::exact(
        "entry",
        InverseVia::Universe(file_removal("skills/a.md")),
    ));
    let mut encoded = Vec::new();
    ledger.write_to(&mut encoded).unwrap();
    let valid = String::from_utf8(encoded).unwrap();
    let value: serde_json::Value = serde_json::from_str(valid.trim_end()).unwrap();

    let mut cases = Vec::new();
    let mut unversioned = value.clone();
    unversioned.as_object_mut().unwrap().remove("format");
    cases.push(serde_json::to_string(&unversioned).unwrap() + "\n");
    let mut unsupported = value.clone();
    unsupported["format"] = serde_json::json!(1);
    cases.push(serde_json::to_string(&unsupported).unwrap() + "\n");
    let mut missing_id = value.clone();
    missing_id["effect"]["reversibility"]["Exact"]["via"]["Universe"]
        .as_object_mut()
        .unwrap()
        .remove("id");
    cases.push(serde_json::to_string(&missing_id).unwrap() + "\n");
    let mut invalid_variant = value.clone();
    invalid_variant["effect"]["reversibility"]["Exact"]["via"]["Universe"]["id"] =
        serde_json::json!("00000000-0000-4000-c000-000000000001");
    cases.push(serde_json::to_string(&invalid_variant).unwrap() + "\n");
    let mut old_lossy_field = value.clone();
    old_lossy_field["effect"]["reversibility"]["Exact"]["via"]["Universe"]["key"] =
        serde_json::json!("session/skills/a.md");
    cases.push(serde_json::to_string(&old_lossy_field).unwrap() + "\n");
    let mut numeric_handle = value;
    numeric_handle["effect"]["reversibility"]["Exact"]["via"] = serde_json::json!({"Backend": 7});
    cases.push(serde_json::to_string(&numeric_handle).unwrap() + "\n");

    for text in cases {
        std::fs::write(&log, &text).unwrap();
        let before = std::fs::read(&log).unwrap();
        let error = Ledger::open_file(&log).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("line 1"));
        assert_eq!(std::fs::read(&log).unwrap(), before);
    }
}

#[test]
fn malformed_middle_and_complete_final_records_never_disappear() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let mut ledger = Ledger::new("github-pr");
    ledger.push(Effect::exact(
        "entry",
        InverseVia::Universe(file_removal("skills/a.md")),
    ));
    let mut encoded = Vec::new();
    ledger.write_to(&mut encoded).unwrap();
    let valid = String::from_utf8(encoded).unwrap();

    let middle = format!("{valid}{{\"format\":2}}\n{valid}");
    std::fs::write(&log, &middle).unwrap();
    let error = Ledger::open_file(&log).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("line 2"));

    let complete_final = format!("{valid}{{\"format\":1}}");
    std::fs::write(&log, &complete_final).unwrap();
    let error = Ledger::open_file(&log).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("line 2"));

    std::fs::write(&log, valid.trim_end()).unwrap();
    assert_eq!(Ledger::open_file(&log).unwrap().len(), 1);
}
