mod support;

use plasmosome_backend::{CellId, GrantId, Handle, PluginId, UniverseClass};
use plasmosome_ledger::cell_journal::{
    CellHistory, CellJournalFaultKind as Kind, CellJournalRecord, Change, Decision,
    JournalEvent as Event, RecordedEffect,
};
use plasmosome_ledger::{Effect, InverseVia};
use support::*;

fn empty() -> CellHistory {
    CellHistory::new(CellId::from("c"))
}
fn start(g: u64) -> CellJournalRecord {
    prepare(g, vec![attach("p", vec![])])
}
fn remove() -> Change {
    Change {
        plugin: PluginId::from("p"),
        replacement: None,
    }
}
fn settle(history: &mut CellHistory, prepare: CellJournalRecord, decision: Event) {
    let g = prepare.generation;
    for r in [prepare, record(g, decision), record(g, Event::Finish)] {
        history.admit(&r).unwrap();
    }
}
fn backend() -> RecordedEffect {
    RecordedEffect {
        effect: Effect::exact(
            "opaque",
            InverseVia::Backend(Handle {
                class: UniverseClass::Mount,
                id: GrantId::new(),
            }),
        ),
        operation: None,
    }
}

#[test]
fn committed_and_aborted_generations_are_legal() {
    let mut h = empty();
    assert_eq!(h.cell(), &CellId::from("c"));
    assert_eq!(h.last_generation(), 0);
    assert_eq!(h.next_generation(), Some(1));
    for (g, decision, expected) in [
        (1, Event::Commit, Decision::Commit),
        (2, Event::Abort, Decision::Abort),
    ] {
        h.admit(&start(g)).unwrap();
        let open = h.open_transaction().unwrap();
        assert_eq!(
            (open.generation, &open.prepare, open.decision),
            (
                g,
                match &start(g).event {
                    Event::Prepare(p) => p,
                    _ => unreachable!(),
                },
                None
            )
        );
        h.admit(&record(g, decision)).unwrap();
        assert_eq!(h.open_transaction().unwrap().decision, Some(expected));
        h.admit(&record(g, Event::Finish)).unwrap();
        assert!(h.open_transaction().is_none());
    }
    assert_eq!(h.last_generation(), 2);
    assert_eq!(h.next_generation(), Some(3));
}

#[test]
fn empty_attachment_and_removal_consume_generations() {
    let mut h = empty();
    settle(&mut h, start(1), Event::Commit);
    assert_eq!(h.committed().len(), 1);
    settle(&mut h, prepare(2, vec![remove()]), Event::Commit);
    assert!(h.committed().is_empty());
    assert_eq!(h.last_generation(), 2);
}

#[test]
fn removing_one_plugin_preserves_other_attachment() {
    let mut h = empty();
    settle(
        &mut h,
        prepare(1, vec![attach("p", vec![]), attach("q", vec![])]),
        Event::Commit,
    );
    let unchanged = h.committed()[&PluginId::from("q")].clone();
    assert_eq!(h.committed().len(), 2);
    settle(&mut h, prepare(2, vec![remove()]), Event::Commit);
    assert_eq!(h.committed().len(), 1);
    assert_eq!(h.committed()[&PluginId::from("q")], unchanged);
    assert_eq!(h.last_generation(), 2);
}

#[test]
fn generation_and_event_order_faults() {
    let p = start;
    let r = record;
    let cases = [
        (vec![], p(2), Kind::GenerationOrder),
        (
            vec![p(1), r(1, Event::Abort), r(1, Event::Finish)],
            p(3),
            Kind::GenerationOrder,
        ),
        (
            vec![
                p(1),
                r(1, Event::Abort),
                r(1, Event::Finish),
                p(2),
                r(2, Event::Commit),
                r(2, Event::Finish),
            ],
            p(1),
            Kind::GenerationOrder,
        ),
        (vec![p(1)], r(2, Event::Commit), Kind::GenerationOrder),
        (
            vec![p(1), r(1, Event::Commit)],
            r(2, Event::Finish),
            Kind::GenerationOrder,
        ),
        (vec![], r(1, Event::Commit), Kind::EventOrder),
        (vec![], r(1, Event::Abort), Kind::EventOrder),
        (vec![], r(1, Event::Finish), Kind::EventOrder),
        (vec![p(1)], r(1, Event::Finish), Kind::EventOrder),
        (vec![p(1)], p(1), Kind::EventOrder),
        (vec![p(1)], p(2), Kind::EventOrder),
        (
            vec![p(1), r(1, Event::Commit)],
            r(1, Event::Commit),
            Kind::EventOrder,
        ),
        (
            vec![p(1), r(1, Event::Commit)],
            r(1, Event::Abort),
            Kind::EventOrder,
        ),
        (
            vec![p(1), r(1, Event::Abort)],
            r(1, Event::Commit),
            Kind::EventOrder,
        ),
        (
            vec![p(1), r(1, Event::Abort), r(1, Event::Finish)],
            r(1, Event::Finish),
            Kind::EventOrder,
        ),
        (
            vec![p(1), r(1, Event::Abort), r(1, Event::Finish)],
            p(1),
            Kind::GenerationOrder,
        ),
        (
            vec![p(1), r(1, Event::Abort)],
            r(1, Event::Abort),
            Kind::EventOrder,
        ),
        (
            vec![p(1), r(1, Event::Commit), r(1, Event::Finish), p(2)],
            r(1, Event::Commit),
            Kind::GenerationOrder,
        ),
        (
            vec![p(1), r(1, Event::Commit)],
            r(0, Event::Finish),
            Kind::GenerationZero,
        ),
        (vec![], p(0), Kind::GenerationZero),
        (vec![p(1)], r(0, Event::Commit), Kind::GenerationZero),
    ];
    for (prefix, bad, kind) in cases {
        let mut h = empty();
        for r in prefix {
            h.admit(&r).unwrap();
        }
        let before = h.clone();
        assert_eq!(h.check(&bad).unwrap_err().kind, kind, "{bad:?}");
        assert_eq!(h.admit(&bad).unwrap_err().kind, kind, "{bad:?}");
        assert_eq!(h, before);
    }
}

#[test]
fn identity_faults_and_atomic_admission() {
    let shared = backend();
    let bad = prepare(
        1,
        vec![attach("p", vec![shared.clone()]), attach("q", vec![shared])],
    );
    let mut h = empty();
    let before = h.clone();
    assert_eq!(h.check(&bad).unwrap_err().kind, Kind::InvalidEffect);
    assert_eq!(h.admit(&bad).unwrap_err().kind, Kind::InvalidEffect);
    assert_eq!(h, before);
    let bytes = plasmosome_ledger::cell_journal::encode_record(&bad).unwrap();
    let fault = plasmosome_ledger::cell_journal::read_cell_journal_from(
        &bytes[..],
        std::path::Path::new("diagnostic-only"),
        &CellId::from("c"),
    )
    .unwrap_err();
    assert_eq!(
        (fault.kind, fault.line, fault.lines_parsed),
        (Kind::InvalidEffect, Some(1), 0)
    );

    let effect = exact_uds("c", "p");
    let mut changed = effect.clone();
    changed.effect.description.push_str(" changed");
    let mut h = empty();
    settle(
        &mut h,
        prepare(1, vec![attach("p", vec![effect.clone()])]),
        Event::Commit,
    );
    for changes in [
        vec![attach("p", vec![changed])],
        vec![attach("p", vec![effect.clone(), effect.clone()])],
        vec![
            attach("p", vec![effect.clone()]),
            attach("q", vec![effect.clone()]),
        ],
    ] {
        let before = h.clone();
        assert_eq!(
            h.admit(&prepare(2, changes)).unwrap_err().kind,
            Kind::InvalidEffect
        );
        assert_eq!(h, before);
    }
    let fresh = exact_uds("c", "p");
    let before = h.clone();
    let mut changed = effect.clone();
    changed.effect.description.push('!');
    assert_eq!(
        h.admit(&prepare(2, vec![attach("p", vec![fresh.clone(), changed])]))
            .unwrap_err()
            .kind,
        Kind::InvalidEffect
    );
    assert_eq!(h, before);
    settle(
        &mut h,
        prepare(2, vec![attach("p", vec![fresh.clone()])]),
        Event::Abort,
    );
    assert_eq!(
        h.admit(&prepare(3, vec![attach("p", vec![fresh])]))
            .unwrap_err()
            .kind,
        Kind::InvalidEffect
    );
    settle(&mut h, prepare(3, vec![remove()]), Event::Commit);
    assert_eq!(
        h.admit(&prepare(4, vec![attach("p", vec![effect.clone()])]))
            .unwrap_err()
            .kind,
        Kind::InvalidEffect
    );
    assert_eq!(h.addresses().count(), 2);
    assert!(
        h.addresses()
            .all(|(_, plugin)| plugin == &PluginId::from("p"))
    );
    let mut h = empty();
    assert_eq!(
        h.admit(&prepare(1, vec![remove()])).unwrap_err().kind,
        Kind::InvalidEffect
    );
    let opaque = backend();
    settle(
        &mut h,
        prepare(1, vec![attach("p", vec![opaque.clone()])]),
        Event::Commit,
    );
    let bad = prepare(
        2,
        vec![Change {
            plugin: PluginId::from("q"),
            replacement: None,
        }],
    );
    let before = h.clone();
    assert_eq!(h.check(&bad).unwrap_err().kind, Kind::InvalidEffect);
    assert_eq!(h.admit(&bad).unwrap_err().kind, Kind::InvalidEffect);
    assert_eq!(h, before);
    assert_eq!(
        h.admit(&prepare(2, vec![attach("q", vec![opaque])]))
            .unwrap_err()
            .kind,
        Kind::InvalidEffect
    );
}

#[test]
fn changed_recipe_refuses_the_whole_prepare() {
    let effect = exact_uds("c", "p");
    let mut changed = effect.clone();
    let op = changed.operation.as_mut().unwrap();
    let plasmosome_backend::UniverseOp::BindUds { recipe, .. } = op else {
        unreachable!()
    };
    recipe.upstream = "/run/other-upstream.sock".into();
    changed.effect = Effect::exact(
        effect.effect.description.clone(),
        InverseVia::Universe(op.removal()),
    );
    let prefix = [
        prepare(1, vec![attach("p", vec![effect])]),
        record(1, Event::Commit),
        record(1, Event::Finish),
    ];
    let mut h = empty();
    for r in &prefix {
        h.admit(r).unwrap();
    }
    let bad = prepare(2, vec![attach("p", vec![exact_uds("c", "p"), changed])]);
    let before = h.clone();
    assert_eq!(h.check(&bad).unwrap_err().kind, Kind::InvalidEffect);
    assert_eq!(h.admit(&bad).unwrap_err().kind, Kind::InvalidEffect);
    assert_eq!(h, before);
    let bytes: Vec<_> = prefix
        .iter()
        .chain([&bad])
        .flat_map(|r| plasmosome_ledger::cell_journal::encode_record(r).unwrap())
        .collect();
    let fault = plasmosome_ledger::cell_journal::read_cell_journal_from(
        &bytes[..],
        std::path::Path::new("diagnostic-only"),
        &CellId::from("c"),
    )
    .unwrap_err();
    assert_eq!(
        (fault.kind, fault.line, fault.lines_parsed),
        (Kind::InvalidEffect, Some(4), 3)
    );
}

#[test]
fn identity_rules_allow_and_commit_publishes_while_abort_preserves() {
    let effect = exact_uds("c", "p");
    let external = RecordedEffect {
        effect: Effect::external("sent"),
        operation: None,
    };
    let mut h = empty();
    let initial = attach("p", vec![effect.clone(), external.clone()]);
    settle(
        &mut h,
        prepare(1, vec![initial.clone(), attach("q", vec![])]),
        Event::Commit,
    );
    let original = h.committed().clone();
    let reload = attach(
        "p",
        vec![effect.clone(), exact_uds("c", "p"), external.clone()],
    );
    h.admit(&prepare(2, vec![reload])).unwrap();
    assert_eq!(h.committed(), &original);
    h.admit(&record(2, Event::Abort)).unwrap();
    assert_eq!(h.committed(), &original);
    h.admit(&record(2, Event::Finish)).unwrap();
    let reload = attach("p", vec![effect.clone(), exact_uds("c", "p"), external]);
    h.admit(&prepare(3, vec![reload.clone()])).unwrap();
    h.admit(&record(3, Event::Commit)).unwrap();
    assert_eq!(
        h.committed()[&PluginId::from("p")].replacement,
        reload.replacement.unwrap()
    );
    assert_eq!(h.committed()[&PluginId::from("p")].generation, 3);
    assert_eq!(h.committed()[&PluginId::from("q")].generation, 1);
    h.admit(&record(3, Event::Finish)).unwrap();
    settle(&mut h, prepare(4, vec![initial]), Event::Commit);
    assert_eq!(
        h.addresses()
            .filter(|(address, _)| **address == effect.address().unwrap())
            .count(),
        1
    );
}

fn assert_identity_change_is_refused(
    label: &str,
    original: RecordedEffect,
    changed: RecordedEffect,
) {
    use plasmosome_ledger::cell_journal::{encode_record, parse_record, read_cell_journal_from};
    assert_ne!(original, changed, "{label}");
    assert_eq!(original.address(), changed.address(), "{label}");
    let prefix = [
        prepare(1, vec![attach("p", vec![original])]),
        record(1, Event::Commit),
        record(1, Event::Finish),
    ];
    let bad = prepare(2, vec![attach("p", vec![exact_uds("c", "p"), changed])]);
    for r in prefix.iter().chain([&bad]) {
        parse_record(&CellId::from("c"), &encode_record(r).unwrap()).unwrap();
    }
    let mut history = empty();
    for r in &prefix {
        history.admit(r).unwrap();
    }
    let before = history.clone();
    assert_eq!(
        history.check(&bad).unwrap_err().kind,
        Kind::InvalidEffect,
        "{label}"
    );
    assert_eq!(
        history.admit(&bad).unwrap_err().kind,
        Kind::InvalidEffect,
        "{label}"
    );
    assert_eq!(history, before, "{label}");
    let bytes: Vec<_> = prefix
        .iter()
        .chain([&bad])
        .flat_map(|r| encode_record(r).unwrap())
        .collect();
    let fault = read_cell_journal_from(
        &bytes[..],
        std::path::Path::new("diagnostic-only"),
        &CellId::from("c"),
    )
    .unwrap_err();
    assert_eq!(
        (fault.kind, fault.line, fault.lines_parsed),
        (Kind::InvalidEffect, Some(4), 3),
        "{label}"
    );
}

#[test]
fn retained_identity_compares_inverse_description_and_kind() {
    use plasmosome_ledger::Reversibility;
    let original = exact_uds("c", "p");
    let mut changed = original.clone();
    let Reversibility::Exact(inverse) = &mut changed.effect.reversibility else {
        unreachable!()
    };
    inverse.description.push_str(" changed");
    assert_identity_change_is_refused("inverse description", original.clone(), changed);
    let changed = RecordedEffect {
        effect: Effect::compensating(
            original.effect.description.clone(),
            original.operation.as_ref().unwrap().removal(),
        ),
        operation: original.operation.clone(),
    };
    assert_identity_change_is_refused(
        "exact becomes compensating",
        original.clone(),
        changed.clone(),
    );
    assert_identity_change_is_refused("compensating becomes exact", changed, original);
}

#[test]
fn retained_identity_compares_every_capability_field() {
    use plasmosome_backend::UniverseOp;
    use serde_json::{Value, json};
    type FieldChange = (&'static str, Value);
    type CapabilityCase = (&'static str, Value, Vec<FieldChange>);
    let owner = json!({"cell":"c", "plugin":"p"});
    let cases: Vec<CapabilityCase> = vec![
        (
            "WriteSessionFile",
            json!({"path":"/run/file","recipe":{"contents":[1],"mode":"read_only","guest_path":"/run/file"},"owner":owner}),
            vec![
                ("/path", json!("/run/other-file")),
                ("/recipe/contents", json!([2])),
                ("/recipe/mode", json!("read_write")),
                ("/recipe/guest_path", json!("/run/other-guest-file")),
            ],
        ),
        (
            "BindUds",
            json!({"path":"/run/socket","recipe":{"upstream":"/run/upstream","guest_path":"/run/socket"},"owner":owner}),
            vec![
                ("/path", json!("/run/other-socket")),
                ("/recipe/upstream", json!("/run/other-upstream")),
                ("/recipe/guest_path", json!("/run/other-guest-socket")),
            ],
        ),
        (
            "SetProxyMap",
            json!({"host":"example.test","route":"route","recipe":{"transport":"tcp","destination":"example.test","port":443,"allow_private":false},"owner":owner}),
            vec![
                ("/host", json!("other.test")),
                ("/route", json!("other-route")),
                ("/recipe/transport", json!("udp")),
                ("/recipe/destination", json!("other.test")),
                ("/recipe/port", json!(8443)),
                ("/recipe/allow_private", json!(true)),
            ],
        ),
        (
            "SpawnBroker",
            json!({"name":"broker","launch":{"command":["/bin/cat"],"control_socket":"/run/control","data_socket":"/run/data"},"owner":owner}),
            vec![
                ("/name", json!("other-broker")),
                ("/launch/command", json!(["/bin/cat", "argument"])),
                ("/launch/control_socket", json!("/run/other-control")),
                ("/launch/data_socket", json!("/run/other-data")),
            ],
        ),
        (
            "AddMount",
            json!({"source":"/source","target":"/target","recipe":{"access":"read_only"},"owner":owner}),
            vec![
                ("/source", json!("/other-source")),
                ("/target", json!("/other-target")),
                ("/recipe/access", json!("read_write")),
            ],
        ),
    ];
    for (variant, mut payload, fields) in cases {
        payload["id"] = serde_json::to_value(GrantId::new()).unwrap();
        let original_op: UniverseOp = serde_json::from_value(json!({variant: payload})).unwrap();
        let original = RecordedEffect {
            effect: Effect::exact("holding", InverseVia::Universe(original_op.removal())),
            operation: Some(original_op),
        };
        for (field, value) in fields {
            let mut changed_payload = payload.clone();
            *changed_payload.pointer_mut(field).unwrap() = value;
            let op: UniverseOp = serde_json::from_value(json!({variant: changed_payload})).unwrap();
            let changed = RecordedEffect {
                effect: Effect::exact("holding", InverseVia::Universe(op.removal())),
                operation: Some(op),
            };
            assert_identity_change_is_refused(
                &format!("{variant}{field}"),
                original.clone(),
                changed,
            );
        }
    }
}

#[test]
fn backend_and_universe_forms_cannot_share_a_standing_address() {
    let original = exact_uds("c", "p");
    let changed = RecordedEffect {
        effect: Effect::exact(
            original.effect.description.clone(),
            InverseVia::Backend(original.address().unwrap()),
        ),
        operation: None,
    };
    assert_identity_change_is_refused(
        "universe becomes backend",
        original.clone(),
        changed.clone(),
    );
    assert_identity_change_is_refused("backend becomes universe", changed, original);
}
