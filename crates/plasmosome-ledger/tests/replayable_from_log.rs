use std::io::Write;
use std::time::Duration;

use plasmosome_backend::{
    BrokerLaunch, Capability, CellId, CellOwner, Diff, DrainSpec, EnforcementBackend, FakeBackend,
    FileAccess, GrantId, GrantKind, LedgerEntry, MountRecipe, PluginId, ProxyRecipe,
    ProxyTransport, SessionFileRecipe, UdsRecipe, UniverseClass, UniverseOp, UniverseRemoval,
};
use plasmosome_ledger::{Closure, Effect, Inverse, InverseVia, Ledger, LogRecord, Reversibility};

fn cell() -> CellId {
    CellId::from("cell-1")
}

fn cell_owner(plugin: &str) -> CellOwner {
    CellOwner {
        cell: cell(),
        plugin: PluginId::from(plugin),
    }
}

fn session_file(path: &str) -> Capability {
    Capability::SessionFile {
        path: path.to_string(),
        recipe: SessionFileRecipe {
            contents: Vec::new(),
            mode: FileAccess::ReadOnly,
            guest_path: "/run/plasmosome/session".to_string(),
        },
    }
}

fn uds_socket(path: &str) -> Capability {
    Capability::UdsSocket {
        path: path.to_string(),
        recipe: UdsRecipe {
            upstream: "/run/plasmosome/upstream.sock".to_string(),
            guest_path: "/run/plasmosome/guest.sock".to_string(),
        },
    }
}

fn proxy_map(host: &str, route: &str) -> Capability {
    Capability::ProxyMap {
        host: host.to_string(),
        route: route.to_string(),
        recipe: ProxyRecipe {
            transport: ProxyTransport::Tcp,
            destination: "upstream.plasmosome.test".to_string(),
            port: 443,
            allow_private: false,
        },
    }
}

fn broker(name: &str) -> Capability {
    Capability::Broker {
        name: name.to_string(),
        launch: BrokerLaunch {
            command: vec!["/usr/libexec/plasmosome-broker".to_string()],
            control_socket: "/run/plasmosome/broker.control".to_string(),
            data_socket: "/run/plasmosome/broker.data".to_string(),
        },
    }
}

fn mount(source: &str, target: &str) -> Capability {
    Capability::Mount {
        source: source.to_string(),
        target: target.to_string(),
        recipe: MountRecipe {
            access: FileAccess::ReadOnly,
        },
    }
}

fn op(id: GrantId, owner: CellOwner, capability: Capability) -> UniverseOp {
    match capability {
        Capability::SessionFile { path, recipe } => UniverseOp::WriteSessionFile {
            id,
            path,
            recipe,
            owner,
        },
        Capability::UdsSocket { path, recipe } => UniverseOp::BindUds {
            id,
            path,
            recipe,
            owner,
        },
        Capability::ProxyMap {
            host,
            route,
            recipe,
        } => UniverseOp::SetProxyMap {
            id,
            host,
            route,
            recipe,
            owner,
        },
        Capability::Broker { name, launch } => UniverseOp::SpawnBroker {
            id,
            name,
            launch,
            owner,
        },
        Capability::Mount {
            source,
            target,
            recipe,
        } => UniverseOp::AddMount {
            id,
            source,
            target,
            recipe,
            owner,
        },
    }
}

fn grant(backend: &mut FakeBackend, plugin: &str, capability: Capability) -> LedgerEntry {
    backend
        .grant(
            op(GrantId::new(), cell_owner(plugin), capability),
            GrantKind::Hot,
        )
        .unwrap()
}

fn file_removal(path: &str) -> UniverseRemoval {
    UniverseRemoval {
        id: GrantId::new(),
        capability: session_file(path),
    }
}

fn wire_capabilities() -> Vec<Capability> {
    vec![
        session_file("/skills/pr.md"),
        uds_socket("/run/plasmosome/egressd.uds"),
        proxy_map("api.github.com", "splice"),
        broker("egressd"),
        mount("/secrets", "/workspace"),
    ]
}

fn populate(backend: &mut FakeBackend) -> (PluginId, Vec<Effect>) {
    let op = op(
        GrantId::new(),
        cell_owner("github-pr"),
        session_file("/skills/pr.md"),
    );
    let removal = op.removal();
    backend.apply(op).unwrap();
    let entry = grant(
        backend,
        "network",
        uds_socket("/run/plasmosome/egressd.uds"),
    );
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
        .detach(
            &mut backend,
            &cell(),
            DrainSpec::graceful(Duration::from_millis(1)),
        )
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
        assert_eq!(record["format"], 3);
    }
    drop(text);

    let reopened = Ledger::open_file(&log).unwrap();
    assert_eq!(reopened.plugin(), &plugin);
    assert_eq!(reopened.effects(), &effects);
}

#[test]
fn exact_state_wire_preserves_identity_and_refuses_ambiguous_rows() {
    let mut backend = FakeBackend::new();
    grant(
        &mut backend,
        "network",
        proxy_map("api.github.com", "splice"),
    );
    let state = backend.snapshot_os_state();
    let encoded = serde_json::to_string(&state).unwrap();
    let round_trip: plasmosome_backend::OsState = serde_json::from_str(&encoded).unwrap();
    assert_eq!(round_trip, state);

    let object = serde_json::to_value(state.objects().next().unwrap()).unwrap();
    let duplicate = serde_json::json!({"objects": [object.clone(), object.clone()]});
    assert!(serde_json::from_value::<plasmosome_backend::OsState>(duplicate).is_err());
    let mut conflict = object.clone();
    conflict["owner"] = serde_json::json!({"cell": "cell-1", "plugin": "audit"});
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
                "owner": {"cell": "cell-1", "plugin": "deploy"},
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
    let entry = grant(&mut backend, "network", session_file("/skills/pr.md"));
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
        format: 3,
        plugin: PluginId::from("network"),
        effect: Effect::exact("wire", InverseVia::Universe(file_removal("/skills/pr.md"))),
    };
    let encoded = serde_json::to_string(&record).unwrap();
    assert!(encoded.starts_with(r#"{"format":3,"#), "{encoded}");
    assert_eq!(serde_json::from_str::<LogRecord>(&encoded).unwrap(), record);

    for format in [1, 2, 4] {
        let expected = format!("unsupported ledger format {format}; expected 3");
        let invalid = LogRecord {
            format,
            ..record.clone()
        };
        assert_eq!(
            serde_json::to_string(&invalid).unwrap_err().to_string(),
            expected
        );
        let decoded = encoded.replacen(r#""format":3"#, &format!(r#""format":{format}"#), 1);
        let error = serde_json::from_str::<LogRecord>(&decoded).unwrap_err();
        assert!(error.to_string().contains(&expected), "{error}");
    }
}

#[test]
fn format_three_refuses_unknown_fields_in_every_nested_capability() {
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
        InverseVia::Universe(file_removal("/skills/a.md")),
    ));
    ledger.push(Effect::exact(
        "entry two",
        InverseVia::Universe(file_removal("/skills/b.md")),
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
        InverseVia::Universe(file_removal("/skills/a.md")),
    );
    let mut ledger = Ledger::new("github-pr");
    ledger.push(first.clone());
    ledger.push(Effect::exact(
        "entry €",
        InverseVia::Universe(file_removal("/skills/b.md")),
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
        InverseVia::Universe(file_removal("/skills/a.md")),
    ));
    let mut valid = Vec::new();
    ledger.write_to(&mut valid).unwrap();
    let prefix = br#"{"format":3,"plugin":"github-pr","effect":{"description":""#;
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
        InverseVia::Universe(file_removal("/skills/a.md")),
    ));
    ledger.append_to_file(&log).unwrap();
    let mut other = Ledger::new("model-provider");
    other.push(Effect::exact(
        "entry two",
        InverseVia::Universe(file_removal("/skills/b.md")),
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
    let capability = uds_socket("/run/plasmosome/egressd.uds");
    let mut backend = FakeBackend::new();
    let first = grant(&mut backend, "network", capability.clone());
    let second = grant(&mut backend, "network", capability);
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
            .detach(
                &mut backend,
                &cell(),
                DrainSpec::graceful(Duration::from_millis(1))
            )
            .is_err()
    );
    assert_eq!(
        backend.snapshot_os_state().objects().collect::<Vec<_>>(),
        vec![&first.object()]
    );
    let resumed = sealed
        .detach(&mut backend, &cell(), DrainSpec::forcing())
        .expect("force resumes at the exact timed-out holding");
    assert_eq!(resumed.replayed, vec!["first equal grant"]);
    assert!(backend.snapshot_os_state().is_empty());
}

#[test]
fn serialized_observations_restore_exact_inverses_but_not_backend_handles() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let capability = broker("egressd");
    let mut source = FakeBackend::new();
    let first = grant(&mut source, "network", capability.clone());
    let second = grant(&mut source, "network", capability);
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
        .detach(&mut fresh, &cell(), DrainSpec::forcing())
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
        InverseVia::Universe(file_removal("/skills/a.md")),
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
        serde_json::json!("session//skills/a.md");
    cases.push(serde_json::to_string(&old_lossy_field).unwrap() + "\n");
    let mut positional = value.clone();
    let universe = &mut positional["effect"]["reversibility"]["Exact"]["via"]["Universe"];
    *universe = serde_json::json!([universe["id"].clone(), universe["capability"].clone()]);
    cases.push(serde_json::to_string(&positional).unwrap() + "\n");
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
        InverseVia::Universe(file_removal("/skills/a.md")),
    ));
    let mut encoded = Vec::new();
    ledger.write_to(&mut encoded).unwrap();
    let valid = String::from_utf8(encoded).unwrap();

    let middle = format!("{valid}{{\"format\":3}}\n{valid}");
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

#[test]
fn a_record_that_cannot_be_encoded_leaves_every_target_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let mut ledger = Ledger::new("github-pr");
    ledger.push(Effect::exact(
        "entry one",
        InverseVia::Universe(file_removal("/skills/a.md")),
    ));
    ledger.append_to_file(&log).unwrap();
    let before = std::fs::read(&log).unwrap();

    let mut unencodable = Ledger::new("github-pr");
    unencodable.push(Effect::exact(
        "a valid entry written first",
        InverseVia::Universe(file_removal("/skills/b.md")),
    ));
    unencodable.push(Effect::compensating(
        "a relative path",
        file_removal("skills/c.md"),
    ));
    let error = unencodable.append_to_file(&log).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        error.to_string().contains(
            "effect 2 of 2 cannot be encoded: `path` must be an absolute path, not \"skills/c.md\""
        ),
        "{error}"
    );
    let after = std::fs::read(&log).unwrap();
    assert!(after == before, "{}", String::from_utf8_lossy(&after));

    let missing = dir.path().join("cell").join("ledger.ndjson");
    assert_eq!(
        unencodable.append_to_file(&missing).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
    assert!(!dir.path().join("cell").exists());

    let mut buffer = Vec::new();
    assert!(unencodable.write_to(&mut buffer).is_err());
    assert!(buffer.is_empty(), "{}", String::from_utf8_lossy(&buffer));
}

#[test]
fn a_record_with_a_repeated_key_at_any_depth_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let mut ledger = Ledger::new("github-pr");
    ledger.push(Effect {
        description: "effect".to_string(),
        reversibility: Reversibility::Exact(Inverse {
            description: "inverse".to_string(),
            via: InverseVia::Universe(file_removal("/skills/a.md")),
        }),
    });
    let mut encoded = Vec::new();
    ledger.write_to(&mut encoded).unwrap();
    let valid = String::from_utf8(encoded).unwrap();
    for (once, twice, field) in [
        (
            r#""plugin":"github-pr""#,
            r#""plugin":"github-pr","plugin":"audit""#,
            "plugin",
        ),
        (
            r#""description":"effect""#,
            r#""description":"effect","description":"other""#,
            "description",
        ),
        (
            r#""description":"inverse""#,
            r#""description":"inverse","description":"other""#,
            "description",
        ),
        (
            r#""path":"/skills/a.md""#,
            r#""path":"/skills/a.md","path":"/skills/b.md""#,
            "path",
        ),
    ] {
        assert_eq!(valid.matches(once).count(), 1, "{once} in {valid}");
        let doubled = valid.replacen(once, twice, 1);
        std::fs::write(&log, &doubled).unwrap();
        let error = Ledger::open_file(&log).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(
            error.to_string().contains("line 1")
                && error
                    .to_string()
                    .contains(&format!("duplicate field `{field}`")),
            "{error}"
        );
        assert_eq!(std::fs::read_to_string(&log).unwrap(), doubled);
    }
}

const REFUSED_TARGET_NAMES: [&str; 27] = [
    "an unversioned record",
    "the format before the current one",
    "the format after the current one",
    "a format 2 record whose only inverse is a session file removal",
    "a format 2 record whose only inverse is a broker removal by pid",
    "a format number too large for a byte",
    "a format number that reads as 3 in a byte",
    "the largest format number",
    "a format 1 record",
    "a numeric backend handle",
    "a universe inverse without its id",
    "an unknown field nested in a capability",
    "an unknown field in a delayed outbox",
    "an unknown field in an external policy",
    "a broker carrying a pid",
    "a capability without its recipe",
    "a broker launch without data_socket",
    "an unknown field inside a recipe",
    "a key repeated with its own value",
    "invalid UTF-8 in a complete line",
    "a malformed line between two valid records",
    "an empty line between two valid records",
    "a complete final format 2 broker record by pid without LF",
    "two valid records naming different plugins",
    "a torn final record that ends with LF",
    "a valid record followed by a torn final fragment",
    "one complete valid record without LF",
];

struct RefusedTarget {
    name: &'static str,
    bytes: Vec<u8>,
    open_refuses: Option<(usize, &'static str)>,
}

fn one_effect_ledger(plugin: &str, description: &str, path: &str) -> Ledger {
    let mut ledger = Ledger::new(plugin);
    ledger.push(Effect::exact(
        description,
        InverseVia::Universe(file_removal(path)),
    ));
    ledger
}

fn encoded_line(plugin: &str, description: &str, path: &str) -> String {
    let mut encoded = Vec::new();
    one_effect_ledger(plugin, description, path)
        .write_to(&mut encoded)
        .unwrap();
    String::from_utf8(encoded).unwrap()
}

fn effect_line(effect: Effect) -> String {
    let mut ledger = Ledger::new("github-pr");
    ledger.push(effect);
    let mut encoded = Vec::new();
    ledger.write_to(&mut encoded).unwrap();
    String::from_utf8(encoded).unwrap()
}

fn with_unknown_field(line: &str, pointer: &str) -> String {
    changed(line, |record| {
        record
            .pointer_mut(pointer)
            .and_then(serde_json::Value::as_object_mut)
            .unwrap()
            .insert("obsolete".to_string(), serde_json::json!("discarded"));
    })
}

fn changed(line: &str, change: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut value: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
    change(&mut value);
    serde_json::to_string(&value).unwrap() + "\n"
}

fn current_format(line: &str) -> u64 {
    let value: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
    value["format"].as_u64().expect("a record names its format")
}

fn refused_targets() -> Vec<RefusedTarget> {
    let first = encoded_line("github-pr", "entry one", "/skills/a.md");
    let second = encoded_line("github-pr", "entry two", "/skills/b.md");
    let format = current_format(&first);
    let universe = "/effect/reversibility/Exact/via/Universe";
    let session_file = format!("{universe}/capability/SessionFile");
    let broker_line = effect_line(Effect::exact(
        "broker",
        InverseVia::Universe(UniverseRemoval {
            id: GrantId::new(),
            capability: broker("egressd"),
        }),
    ));
    let broker_at = format!("{universe}/capability/Broker");
    let without = |line: &str, pointer: &str, field: &str| {
        changed(line, |record| {
            record
                .pointer_mut(pointer)
                .and_then(serde_json::Value::as_object_mut)
                .unwrap()
                .remove(field);
        })
    };
    let unsupported_later = changed(&first, |record| {
        record["format"] = serde_json::json!(format + 1)
    });
    let format_2_broker = changed(&without(&broker_line, &broker_at, "launch"), |record| {
        record
            .pointer_mut(&broker_at)
            .and_then(serde_json::Value::as_object_mut)
            .unwrap()
            .insert("pid".to_string(), serde_json::json!(31337));
        record["format"] = serde_json::json!(2);
    });
    let path_once = r#""path":"/skills/a.md""#;
    assert_eq!(first.matches(path_once).count(), 1, "{first}");
    let (before_description, after_description) = first.split_once("entry one").unwrap();
    let mut invalid_utf8 = first.as_bytes().to_vec();
    invalid_utf8.extend_from_slice(before_description.as_bytes());
    invalid_utf8.extend_from_slice(b"entry \xff");
    invalid_utf8.extend_from_slice(after_description.as_bytes());
    let row = |name, text: String, refusal| RefusedTarget {
        name,
        bytes: text.into_bytes(),
        open_refuses: refusal,
    };
    vec![
        row(
            "an unversioned record",
            changed(&first, |record| {
                record.as_object_mut().unwrap().remove("format");
            }),
            Some((1, "missing field `format`")),
        ),
        row(
            "the format before the current one",
            changed(&first, |record| {
                record["format"] = serde_json::json!(format - 1)
            }),
            Some((1, "unsupported ledger format 2; expected 3")),
        ),
        row(
            "the format after the current one",
            unsupported_later,
            Some((1, "unsupported ledger format 4; expected 3")),
        ),
        row(
            "a format 2 record whose only inverse is a session file removal",
            changed(&without(&first, &session_file, "recipe"), |record| {
                record["format"] = serde_json::json!(2)
            }),
            Some((1, "unsupported ledger format 2; expected 3")),
        ),
        row(
            "a format 2 record whose only inverse is a broker removal by pid",
            format_2_broker.clone(),
            Some((1, "unsupported ledger format 2; expected 3")),
        ),
        row(
            "a format number too large for a byte",
            changed(&first, |record| record["format"] = serde_json::json!(256)),
            Some((1, "unsupported ledger format 256; expected 3")),
        ),
        row(
            "a format number that reads as 3 in a byte",
            changed(&first, |record| record["format"] = serde_json::json!(259)),
            Some((1, "unsupported ledger format 259; expected 3")),
        ),
        row(
            "the largest format number",
            changed(&first, |record| {
                record["format"] = serde_json::json!(u64::MAX)
            }),
            Some((
                1,
                "unsupported ledger format 18446744073709551615; expected 3",
            )),
        ),
        row(
            "a format 1 record",
            changed(&first, |record| record["format"] = serde_json::json!(1)),
            Some((1, "unsupported ledger format 1; expected 3")),
        ),
        row(
            "a numeric backend handle",
            changed(&first, |record| {
                record["effect"]["reversibility"]["Exact"]["via"] =
                    serde_json::json!({"Backend": 7})
            }),
            Some((1, "invalid type: integer `7`")),
        ),
        row(
            "a universe inverse without its id",
            changed(&first, |record| {
                record
                    .pointer_mut(universe)
                    .and_then(serde_json::Value::as_object_mut)
                    .unwrap()
                    .remove("id");
            }),
            Some((1, "missing field `id`")),
        ),
        row(
            "an unknown field nested in a capability",
            changed(&first, |record| {
                record
                    .pointer_mut(&format!("{universe}/capability"))
                    .and_then(serde_json::Value::as_object_mut)
                    .and_then(|variants| variants.values_mut().next())
                    .and_then(serde_json::Value::as_object_mut)
                    .unwrap()
                    .insert("obsolete".to_string(), serde_json::json!("discarded"));
            }),
            Some((1, "unknown field `obsolete`")),
        ),
        row(
            "an unknown field in a delayed outbox",
            with_unknown_field(
                &effect_line(Effect::delayed_unpublished("outbox/github", "payload")),
                "/effect/reversibility/Delayed",
            ),
            Some((1, "unknown field `obsolete`")),
        ),
        row(
            "an unknown field in an external policy",
            with_unknown_field(
                &effect_line(Effect::external("the comment left the host")),
                "/effect/reversibility/External",
            ),
            Some((1, "unknown field `obsolete`")),
        ),
        row(
            "a broker carrying a pid",
            changed(&broker_line, |record| {
                record
                    .pointer_mut(&broker_at)
                    .and_then(serde_json::Value::as_object_mut)
                    .unwrap()
                    .insert("pid".to_string(), serde_json::json!(31337));
            }),
            Some((1, "unknown field `pid`")),
        ),
        row(
            "a capability without its recipe",
            without(&first, &session_file, "recipe"),
            Some((1, "missing field `recipe`")),
        ),
        row(
            "a broker launch without data_socket",
            without(&broker_line, &format!("{broker_at}/launch"), "data_socket"),
            Some((1, "missing field `data_socket`")),
        ),
        row(
            "an unknown field inside a recipe",
            with_unknown_field(&first, &format!("{session_file}/recipe")),
            Some((1, "unknown field `obsolete`")),
        ),
        row(
            "a key repeated with its own value",
            first.replacen(path_once, &format!("{path_once},{path_once}"), 1),
            Some((1, "duplicate field `path`")),
        ),
        RefusedTarget {
            name: "invalid UTF-8 in a complete line",
            bytes: invalid_utf8,
            open_refuses: Some((2, "invalid utf-8 sequence")),
        },
        row(
            "a malformed line between two valid records",
            format!("{first}{}\n{second}", &first[..first.len() / 2]),
            Some((2, "EOF while parsing")),
        ),
        row(
            "an empty line between two valid records",
            format!("{first}\n{second}"),
            Some((2, "EOF while parsing a value")),
        ),
        row(
            "a complete final format 2 broker record by pid without LF",
            format!("{first}{}", format_2_broker.trim_end()),
            Some((2, "unsupported ledger format 2; expected 3")),
        ),
        row(
            "two valid records naming different plugins",
            format!(
                "{first}{}",
                encoded_line("audit", "entry two", "/skills/b.md")
            ),
            Some((2, "`github-pr` and `audit`")),
        ),
        row(
            "a torn final record that ends with LF",
            format!("{first}{}\n", &second[..second.len() - 12]),
            Some((2, "EOF while parsing")),
        ),
        row(
            "a valid record followed by a torn final fragment",
            format!("{first}{}", &second[..second.len() - 12]),
            None,
        ),
        row(
            "one complete valid record without LF",
            first.trim_end().to_string(),
            None,
        ),
    ]
}

#[test]
fn open_file_refuses_each_refused_target_on_its_line_and_keeps_its_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let mut problems = Vec::new();
    let mut checked = Vec::new();
    for target in refused_targets() {
        checked.push(target.name);
        std::fs::write(&log, &target.bytes).unwrap();
        match (Ledger::open_file(&log), target.open_refuses) {
            (Err(error), Some((line, reason)))
                if error.kind() == std::io::ErrorKind::InvalidData
                    && error
                        .to_string()
                        .contains(&format!("on line {line}: {reason}")) => {}
            (Ok(ledger), None)
                if ledger
                    .effects()
                    .iter()
                    .map(|effect| effect.description.as_str())
                    .eq(["entry one"]) => {}
            (outcome, refusal) => problems.push(format!(
                "{}: {outcome:?}, expected the refusal {refusal:?}",
                target.name
            )),
        }
        if std::fs::read(&log).unwrap() != target.bytes {
            problems.push(format!("{}: open_file changed the bytes", target.name));
        }
    }
    assert_eq!(problems, Vec::<String>::new());
    assert_eq!(checked, REFUSED_TARGET_NAMES);
}

#[test]
fn append_to_file_refuses_each_refused_target_and_keeps_its_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let appended = one_effect_ledger("github-pr", "appended", "/skills/c.md");
    let mut problems = Vec::new();
    let mut checked = Vec::new();
    for target in refused_targets() {
        checked.push(target.name);
        std::fs::write(&log, &target.bytes).unwrap();
        let expected = if target.bytes.ends_with(b"\n") {
            Ledger::open_file(&log).expect_err(target.name).to_string()
        } else {
            "ledger log does not end with a newline; refusing to append".to_string()
        };
        match appended.append_to_file(&log) {
            Err(error)
                if error.kind() == std::io::ErrorKind::InvalidData
                    && error.to_string() == expected => {}
            outcome => problems.push(format!(
                "{}: {outcome:?}, expected InvalidData: {expected}",
                target.name
            )),
        }
        if std::fs::read(&log).unwrap() != target.bytes {
            problems.push(format!("{}: append_to_file changed the bytes", target.name));
        }
    }
    assert_eq!(problems, Vec::<String>::new());
    assert_eq!(checked, REFUSED_TARGET_NAMES);
}

#[test]
fn append_to_file_refuses_another_plugins_log_and_names_both_plugins() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let existing = encoded_line("audit", "entry one", "/skills/a.md");
    std::fs::write(&log, &existing).unwrap();
    assert_eq!(
        Ledger::open_file(&log).unwrap().plugin(),
        &PluginId::from("audit")
    );

    let error = one_effect_ledger("github-pr", "appended", "/skills/c.md")
        .append_to_file(&log)
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "ledger log belongs to plugin `audit`; refusing to append plugin `github-pr`"
    );
    assert_eq!(std::fs::read_to_string(&log).unwrap(), existing);
}

#[test]
fn append_to_file_creates_a_missing_target_and_extends_an_empty_or_valid_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut new = one_effect_ledger("github-pr", "new one", "/skills/c.md");
    new.push(Effect::exact(
        "new two",
        InverseVia::Universe(file_removal("/skills/d.md")),
    ));
    let mut old = one_effect_ledger("github-pr", "old one", "/skills/a.md");
    old.push(Effect::exact(
        "old two",
        InverseVia::Universe(file_removal("/skills/b.md")),
    ));
    let mut old_bytes = Vec::new();
    old.write_to(&mut old_bytes).unwrap();

    let missing = dir.path().join("cell").join("plugin").join("ledger.ndjson");
    let empty = dir.path().join("empty.ndjson");
    std::fs::write(&empty, b"").unwrap();
    let valid = dir.path().join("valid.ndjson");
    std::fs::write(&valid, &old_bytes).unwrap();

    for (target, before) in [
        (&missing, &[][..]),
        (&empty, &[][..]),
        (&valid, old.effects()),
    ] {
        assert_eq!(
            new.append_to_file(target).unwrap(),
            2,
            "{}",
            target.display()
        );
        let reopened = Ledger::open_file(target).unwrap();
        let expected: Vec<Effect> = before.iter().chain(new.effects()).cloned().collect();
        assert_eq!(reopened.effects(), expected, "{}", target.display());
        assert_eq!(reopened.plugin(), &PluginId::from("github-pr"));
    }
}

#[test]
fn open_file_refuses_an_empty_log_or_one_holding_only_a_torn_record() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("ledger.ndjson");
    let line = encoded_line("github-pr", "entry one", "/skills/a.md");
    for bytes in [&[][..], &line.as_bytes()[..line.len() - 12]] {
        std::fs::write(&log, bytes).unwrap();
        let error = Ledger::open_file(&log).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "ledger log holds no record");
        assert_eq!(std::fs::read(&log).unwrap(), bytes);
    }
}

#[test]
fn append_to_file_reads_and_checks_the_whole_of_a_long_target() {
    let dir = tempfile::tempdir().unwrap();
    let mut long = Ledger::new("github-pr");
    for index in 0..1000 {
        long.push(Effect::exact(
            format!("entry {index}"),
            InverseVia::Universe(file_removal(&format!("/skills/{index}.md"))),
        ));
    }
    let mut valid = Vec::new();
    long.write_to(&mut valid).unwrap();
    assert!(valid.len() > 64 * 1024, "{}", valid.len());
    let appended = one_effect_ledger("github-pr", "appended", "/skills/c.md");

    let extended = dir.path().join("extended.ndjson");
    std::fs::write(&extended, &valid).unwrap();
    assert_eq!(appended.append_to_file(&extended).unwrap(), 1);
    let expected: Vec<Effect> = long
        .effects()
        .iter()
        .chain(appended.effects())
        .cloned()
        .collect();
    assert_eq!(Ledger::open_file(&extended).unwrap().effects(), expected);

    let refused = dir.path().join("refused.ndjson");
    let unversioned = changed(
        &encoded_line("github-pr", "entry one", "/skills/a.md"),
        |record| {
            record.as_object_mut().unwrap().remove("format");
        },
    );
    let bytes = [unversioned.into_bytes(), valid].concat();
    std::fs::write(&refused, &bytes).unwrap();
    let error = appended.append_to_file(&refused).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("on line 1:"), "{error}");
    assert_eq!(std::fs::read(&refused).unwrap(), bytes);
}
