use plasmosome_backend::{
    BrokerLaunch, CellId, CellOwner, FileAccess, GrantId, Handle, MockMode, PluginId,
    SessionFileRecipe, UniverseClass, UniverseOp,
};
use plasmosome_ledger::artifact::{
    ArtifactKind, Digest, Population, RegistryId, ReleaseKey, ReleaseRef,
};
use plasmosome_ledger::cell_journal::{
    AttachmentSource, CellJournalFault, CellJournalFaultKind, CellJournalRecord, Change,
    ForceAssertion, JournalEvent, Prepare, RecordedEffect, Replacement, encode_record,
    parse_record,
};
use plasmosome_ledger::{Effect, InverseVia, Ledger};
use serde_json::{Value, json};

const PLUGIN: &str = "github-pr";
const PREPARE: &str = "/event/prepare";
const FORCE: &str = "/event/prepare/force";
const CHANGES: &str = "/event/prepare/changes";
const CHANGE: &str = "/event/prepare/changes/0";
const REPLACEMENT: &str = "/event/prepare/changes/0/replacement";
const MOCK: &str = "/event/prepare/changes/0/replacement/mock";
const SOURCE: &str = "/event/prepare/changes/0/replacement/source";
const MEMBER: &str = "/event/prepare/changes/0/replacement/source/member";
const SPEC_008: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/008-cell-recovery-contract.md"
);

fn cell() -> CellId {
    CellId::from("cell-1")
}

fn owner() -> CellOwner {
    CellOwner {
        cell: cell(),
        plugin: PluginId::from(PLUGIN),
    }
}

fn file_op(path: &str) -> UniverseOp {
    UniverseOp::WriteSessionFile {
        id: GrantId::new(),
        path: path.to_string(),
        recipe: SessionFileRecipe {
            contents: b"skill".to_vec(),
            mode: FileAccess::ReadOnly,
            guest_path: "/run/plasmosome/session".to_string(),
        },
        owner: owner(),
    }
}

fn exact(op: UniverseOp) -> RecordedEffect {
    RecordedEffect {
        effect: Effect::exact("skill file", InverseVia::Universe(op.removal())),
        operation: Some(op),
    }
}

fn forensic(effect: Effect) -> RecordedEffect {
    RecordedEffect {
        effect,
        operation: None,
    }
}

fn release(kind: ArtifactKind, name: &str) -> ReleaseRef {
    let key = ReleaseKey::new(kind, Population::Curated, "plasmosome", name, "1.0.0").unwrap();
    ReleaseRef::new(key, Digest::from_sha256([7; 32]))
}

fn registry() -> AttachmentSource {
    AttachmentSource::Registry {
        registry_id: RegistryId::parse("0f8fad5b-d9cb-469f-a165-70867728950e").unwrap(),
        root: release(ArtifactKind::Genome, "review-kit"),
        member: release(ArtifactKind::Plasmid, PLUGIN),
    }
}

fn record(generation: u64, event: JournalEvent) -> CellJournalRecord {
    CellJournalRecord { generation, event }
}

fn prepare(changes: Vec<Change>, force: Option<ForceAssertion>) -> CellJournalRecord {
    record(1, JournalEvent::Prepare(Prepare { changes, force }))
}

fn attach(plugin: &str, source: AttachmentSource, effects: Vec<RecordedEffect>) -> Change {
    Change {
        plugin: PluginId::from(plugin),
        replacement: Some(Replacement {
            mock: MockMode::Simulate,
            source,
            effects,
        }),
    }
}

fn every_effect() -> Vec<RecordedEffect> {
    let witness = file_op("/skills/b.md");
    let handle = Handle {
        class: UniverseClass::UdsPath,
        id: GrantId::new(),
    };
    vec![
        exact(file_op("/skills/a.md")),
        RecordedEffect {
            effect: Effect::compensating("retract", witness.removal()),
            operation: Some(witness),
        },
        forensic(Effect::exact("opaque", InverseVia::Backend(handle))),
        forensic(Effect::external("posted")),
        forensic(Effect::delayed_unpublished("outbox", "payload")),
        forensic(Effect::delayed_published("outbox", "payload")),
    ]
}

fn base() -> CellJournalRecord {
    prepare(vec![attach(PLUGIN, registry(), every_effect())], None)
}

fn commit() -> CellJournalRecord {
    record(1, JournalEvent::Commit)
}

fn value(record: &CellJournalRecord) -> Value {
    serde_json::from_slice(&encode_record(record).unwrap()).unwrap()
}

fn at(record: &CellJournalRecord, pointer: &str) -> Value {
    value(record).pointer(pointer).unwrap().clone()
}

fn effect(index: usize, rest: &str) -> String {
    format!("{REPLACEMENT}/effects/{index}{rest}")
}

fn set(record: &CellJournalRecord, pointer: &str, new: Value) -> Vec<u8> {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    let mut value = value(record);
    match value.pointer_mut(parent).unwrap() {
        Value::Object(fields) => {
            fields.insert(key.to_string(), new);
        }
        Value::Array(items) => items[key.parse::<usize>().unwrap()] = new,
        other => panic!("{parent} holds no field: {other}"),
    }
    serde_json::to_vec(&value).unwrap()
}

fn remove(record: &CellJournalRecord, pointer: &str) -> Vec<u8> {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    let mut value = value(record);
    let fields = value.pointer_mut(parent).unwrap().as_object_mut().unwrap();
    fields.remove(key).unwrap();
    serde_json::to_vec(&value).unwrap()
}

fn replaced(record: &CellJournalRecord, from: &str, to: &str) -> Vec<u8> {
    let text = String::from_utf8(encode_record(record).unwrap()).unwrap();
    assert!(text.contains(from), "{from} is in {text}");
    text.replacen(from, to, 1).into_bytes()
}

fn assert_refused(kind: CellJournalFaultKind, rows: Vec<(&str, Vec<u8>)>) {
    for (name, bytes) in rows {
        let refused = parse_record(&cell(), &bytes)
            .err()
            .map(|refusal| refusal.kind);
        assert_eq!(refused, Some(kind), "{name}");
    }
}

#[test]
fn every_record_shape_round_trips_on_one_line() {
    let force = ForceAssertion {
        operator: "ops".to_string(),
        reason: "the emission was approved".to_string(),
    };
    let removal = Change {
        plugin: PluginId::from("workspace"),
        replacement: None,
    };
    let empty = attach("network", AttachmentSource::Local, Vec::new());
    let full = prepare(
        vec![attach(PLUGIN, registry(), every_effect()), empty, removal],
        Some(force),
    );
    let records = [
        full,
        base(),
        commit(),
        record(2, JournalEvent::Abort),
        record(3, JournalEvent::Finish),
    ];
    for record in &records {
        let line = encode_record(record).unwrap();
        assert_eq!(line.iter().filter(|byte| **byte == b'\n').count(), 1);
        assert_eq!(line.last(), Some(&b'\n'));
        assert_eq!(parse_record(&cell(), &line).as_ref(), Ok(record));
        let unterminated = &line[..line.len() - 1];
        assert_eq!(parse_record(&cell(), unterminated).as_ref(), Ok(record));
    }
    let text = |index: usize| String::from_utf8(encode_record(&records[index]).unwrap()).unwrap();
    let full = text(0);
    #[rustfmt::skip]
    let written = [r#""replacement":null"#, r#""operation":null"#, r#""kind":"local""#];
    for written in written {
        assert!(full.contains(written), "{written} in {full}");
    }
    assert!(text(1).starts_with(r#"{"format":1,"generation":1,"event":{"prepare":{"changes":"#));
    assert!(text(1).ends_with("\"force\":null}}}\n"));
    let abort = concat!(r#"{"format":1,"generation":2,"event":{"abort":{}}}"#, "\n");
    assert_eq!(text(3), abort);
}

#[test]
fn fault_kinds_are_spec_008s_closed_list() {
    use CellJournalFaultKind::*;
    #[rustfmt::skip]
    let kinds = [Io, Unparseable, UnsupportedFormat, GenerationZero, GenerationOrder, EventOrder,
        InvalidEffect, InvalidSource, InvalidClosure, InvalidLifetime];
    let every = |kind: CellJournalFaultKind| match kind {
        Io | Unparseable | UnsupportedFormat | GenerationZero | GenerationOrder | EventOrder
        | InvalidEffect | InvalidSource | InvalidClosure | InvalidLifetime => kind,
    };
    let spec = std::fs::read_to_string(SPEC_008).expect("spec 008 is readable beside the crate");
    let words = spec.split_whitespace().collect::<Vec<_>>().join(" ");
    let (_, list) = words.split_once("`lines_parsed: u64`, and kind `").unwrap();
    let (list, _) = list.split_once('`').unwrap();
    let named: Vec<String> = kinds.map(|kind| format!("{:?}", every(kind))).to_vec();
    assert_eq!(named, list.split(" | ").collect::<Vec<_>>());
}

#[test]
fn unparseable_records_refuse() {
    let r = &base();
    let c = &commit();
    let change = at(r, CHANGE);
    let external = at(r, &effect(3, "/effect"));
    let mut invalid_utf8 = encode_record(r).unwrap();
    let start = invalid_utf8.windows(5).position(|w| w == b"skill").unwrap();
    invalid_utf8[start] = 0xff;
    let twice = "\"generation\":1,\"generation\":1";
    #[rustfmt::skip]
    let rows = vec![
        ("not JSON", b"{\"format\":1,".to_vec()),
        ("a positional envelope", b"[1,1,{\"commit\":{}}]".to_vec()),
        ("an empty line", Vec::new()),
        ("a lone CR", b"\r".to_vec()),
        ("invalid UTF-8", invalid_utf8),
        ("an unknown envelope field", set(r, "/obsolete", json!(1))),
        ("no generation", remove(r, "/generation")),
        ("a repeated generation", replaced(r, "\"generation\":1", twice)),
        ("generation as text", set(r, "/generation", json!("1"))),
        ("a negative generation", set(r, "/generation", json!(-1))),
        ("a fractional generation", set(r, "/generation", json!(1.5))),
        ("an unknown event", set(c, "/event", json!({"rollback": {}}))),
        ("a terminal event as text", set(c, "/event", json!("commit"))),
        ("a terminal event with a field", set(c, "/event/commit", json!({"x": 1}))),
        ("a positional terminal event", set(c, "/event/commit", json!([]))),
        ("two events", set(c, "/event", json!({"commit": {}, "abort": {}}))),
        ("a prepare without force", remove(r, FORCE)),
        ("an unknown prepare field", set(r, &format!("{PREPARE}/obsolete"), json!(1))),
        ("a positional prepare", set(r, PREPARE, json!([[change], null]))),
        ("no changes", set(r, CHANGES, json!([]))),
        ("one plugin twice", set(r, CHANGES, json!([change, change]))),
        ("a blank operator", set(r, FORCE, json!({"operator": " ", "reason": "why"}))),
        ("a blank reason", set(r, FORCE, json!({"operator": "ops", "reason": ""}))),
        ("an unknown force field", set(r, FORCE, json!({"operator": "o", "reason": "r", "x": 1}))),
        ("a positional force", set(r, FORCE, json!(["ops", "why"]))),
        ("a change without replacement", remove(r, &format!("{CHANGE}/replacement"))),
        ("an unknown change field", set(r, &format!("{CHANGE}/obsolete"), json!(1))),
        ("a positional change", set(r, CHANGE, json!([PLUGIN, null]))),
        ("a replacement without source", remove(r, SOURCE)),
        ("lifetime, until dlyi", set(r, &format!("{REPLACEMENT}/lifetime"), json!({}))),
        ("named, until i4z2", set(r, &format!("{REPLACEMENT}/named"), json!(true))),
        ("a positional replacement", set(r, REPLACEMENT, json!(["simulate", {}, []]))),
        ("an unknown mock mode", set(r, MOCK, json!("live"))),
        ("a capitalized mock mode", set(r, MOCK, json!("Simulate"))),
        ("an empty mock mode", set(r, MOCK, json!(""))),
        ("a mock mode as an object", set(r, MOCK, json!({"simulate": null}))),
        ("an effect without operation", remove(r, &effect(0, "/operation"))),
        ("an unknown recorded-effect field", set(r, &effect(0, "/obsolete"), json!(1))),
        ("a positional recorded effect", set(r, &effect(3, ""), json!([external, null]))),
    ];
    assert_refused(CellJournalFaultKind::Unparseable, rows);
}

#[test]
fn unsupported_formats_and_generation_zero_refuse() {
    let mut single_plugin = Ledger::new(PLUGIN);
    single_plugin.push(Effect::external("posted"));
    let mut format_three = Vec::new();
    single_plugin.write_to(&mut format_three).unwrap();
    let r = &base();
    let draft = br#"{"grant":{"plugin":"x","capability":"y"},"plugin":"x"}"#.to_vec();
    #[rustfmt::skip]
    let rows = vec![
        ("format 2", set(r, "/format", json!(2))),
        ("format 3", set(r, "/format", json!(3))),
        ("format 0", set(&commit(), "/format", json!(0))),
        ("format as text", set(r, "/format", json!("1"))),
        ("format as a float", set(r, "/format", json!(1.0))),
        ("format null", set(r, "/format", json!(null))),
        ("no format", remove(r, "/format")),
        ("format 2 with an unknown field", replaced(r, "\"format\":1", "\"format\":2,\"x\":1")),
        ("an old draft grant line", draft),
        ("a single-plugin format 3 line", format_three),
    ];
    assert_refused(CellJournalFaultKind::UnsupportedFormat, rows);
    #[rustfmt::skip]
    let rows = vec![
        ("a prepare", set(r, "/generation", json!(0))),
        ("a commit", set(&commit(), "/generation", json!(0))),
    ];
    assert_refused(CellJournalFaultKind::GenerationZero, rows);
}

#[test]
fn invalid_sources_refuse() {
    let r = &base();
    let root = at(r, &format!("{SOURCE}/root"));
    let member = |field: &str, text: Value| set(r, &format!("{MEMBER}/{field}"), text);
    let digest = |text: String| member("digest", json!(text));
    let registry_id = |text: &str| set(r, &format!("{SOURCE}/registry_id"), json!(text));
    let hex = "7".repeat(64);
    let repeated = "\"kind\":\"registry\",\"kind\":\"registry\"";
    #[rustfmt::skip]
    let rows = vec![
        ("an unknown kind", set(r, SOURCE, json!({"kind": "git"}))),
        ("no kind", set(r, SOURCE, json!({}))),
        ("a local source with a root", set(r, SOURCE, json!({"kind": "local", "root": root}))),
        ("a source as text", set(r, SOURCE, json!("local"))),
        ("a positional source", set(r, SOURCE, json!(["local"]))),
        ("a repeated kind", replaced(r, "\"kind\":\"registry\"", repeated)),
        ("an unknown registry field", set(r, &format!("{SOURCE}/obsolete"), json!(1))),
        ("no member", remove(r, MEMBER)),
        ("a null member", set(r, MEMBER, json!(null))),
        ("no root", remove(r, &format!("{SOURCE}/root"))),
        ("no registry_id", remove(r, &format!("{SOURCE}/registry_id"))),
        ("a nil registry_id", registry_id("00000000-0000-0000-0000-000000000000")),
        ("an upper-case registry_id", registry_id("0F8FAD5B-D9CB-469F-A165-70867728950E")),
        ("a 63-digit digest", digest(format!("sha256:{}", &hex[1..]))),
        ("an upper-case digest", digest(format!("sha256:{}", "A".repeat(64)))),
        ("a sha512 digest", digest(format!("sha512:{hex}"))),
        ("an upper-case publisher", member("publisher", json!("Plasmosome"))),
        ("a 65-byte root name", set(r, &format!("{SOURCE}/root/name"), json!("a".repeat(65)))),
        ("a bad version", member("version", json!("-1"))),
        ("a genome member", member("kind", json!("genome"))),
        ("a member named for another plugin", member("name", json!("network"))),
    ];
    assert_refused(CellJournalFaultKind::InvalidSource, rows);
    let genome_root = parse_record(&cell(), &encode_record(r).unwrap());
    assert_eq!(genome_root.as_ref(), Ok(r));
    let mut genome_member = at(r, SOURCE);
    genome_member["member"]["kind"] = json!("genome");
    let refused = serde_json::from_value::<AttachmentSource>(genome_member).unwrap_err();
    let refused = refused.to_string();
    assert!(refused.contains("must be a plasmid"), "{refused}");
}

#[test]
fn invalid_effects_refuse() {
    let r = &base();
    let operation = at(r, &effect(0, "/operation"));
    let file = |rest: &str| effect(0, &format!("/operation/WriteSessionFile{rest}"));
    let broker_op = UniverseOp::SpawnBroker {
        id: GrantId::new(),
        name: "github".to_string(),
        launch: BrokerLaunch {
            command: vec!["/usr/libexec/plasmosome-broker".to_string()],
            control_socket: "/run/plasmosome/broker.control".to_string(),
            data_socket: "/run/plasmosome/broker.data".to_string(),
        },
        owner: owner(),
    };
    let mut broker = serde_json::to_value(broker_op).unwrap();
    broker["SpawnBroker"]["launch"]["pid"] = json!(31337);
    let fresh = json!(GrantId::new().to_string());
    let with_op = |index: usize, new: Value| set(r, &effect(index, "/operation"), new);
    let inverse =
        |index: usize, rest: &str| effect(index, &format!("/effect/reversibility/{rest}"));
    let version_one = json!("c232ab00-9414-11ec-b3c8-9f6bdeced846");
    #[rustfmt::skip]
    let rows = vec![
        ("an exact effect with a null operation", with_op(0, json!(null))),
        ("a compensation with a null operation", with_op(1, json!(null))),
        ("an operation ID other than its inverse's", set(r, &file("/id"), fresh.clone())),
        ("another capability", set(r, &file("/recipe/guest_path"), json!("/run/plasmosome/b"))),
        ("an operation owned in another cell", set(r, &file("/owner/cell"), json!("cell-2"))),
        ("an operation owned by another plugin", set(r, &file("/owner/plugin"), json!("network"))),
        ("a witness unlike its operation", set(r, &inverse(1, "Compensating/witness/id"), fresh)),
        ("a backend-handle effect with an operation", with_op(2, operation.clone())),
        ("an external effect with an operation", with_op(3, operation.clone())),
        ("a delayed effect with an operation", with_op(4, operation)),
        ("a version 1 grant ID", set(r, &file("/id"), version_one)),
        ("an unknown recipe field", set(r, &file("/recipe/obsolete"), json!(1))),
        ("a broker carrying a pid", with_op(0, broker)),
        ("an unknown outbox field", set(r, &inverse(4, "Delayed/x"), json!(1))),
        ("an unknown policy field", set(r, &inverse(3, "External/x"), json!(1))),
        ("an effect that is not one", set(r, &effect(3, "/effect"), json!(1))),
    ];
    assert_refused(CellJournalFaultKind::InvalidEffect, rows);
}

#[test]
fn equal_capabilities_with_two_ids_and_a_crlf_line_parse() {
    let first = file_op("/skills/a.md");
    let mut second = first.clone();
    if let UniverseOp::WriteSessionFile { id, .. } = &mut second {
        *id = GrantId::new();
    }
    let effects = vec![exact(first), exact(second)];
    let record = prepare(vec![attach(PLUGIN, AttachmentSource::Local, effects)], None);
    let mut line = encode_record(&record).unwrap();
    line.insert(line.len() - 1, b'\r');
    assert_eq!(parse_record(&cell(), &line), Ok(record));
}

#[test]
fn each_effect_names_its_address() {
    let effects = every_effect();
    let addresses: Vec<Option<Handle>> = effects.iter().map(RecordedEffect::address).collect();
    let handle = |index: usize| effects[index].operation.as_ref().map(UniverseOp::handle);
    assert_eq!(&addresses[..2], &[handle(0), handle(1)]);
    let class = addresses[2].map(|handle| handle.class);
    assert_eq!(class, Some(UniverseClass::UdsPath));
    assert_eq!(&addresses[3..], &[None, None, None]);
}

#[test]
fn refusals_and_faults_display_their_kind_line_and_detail() {
    let refusal = parse_record(&cell(), b"").unwrap_err();
    assert!(refusal.to_string().starts_with("Unparseable: "));
    let mut fault = CellJournalFault {
        path: "cells/c/ledger.ndjson".into(),
        line: Some(3),
        lines_parsed: 2,
        kind: CellJournalFaultKind::EventOrder,
        detail: "finish twice".to_string(),
    };
    let shown = "cell journal cells/c/ledger.ndjson line 3: EventOrder after 2 accepted records";
    assert_eq!(fault.to_string(), format!("{shown}: finish twice"));
    fault.line = None;
    let shown = "cell journal cells/c/ledger.ndjson: EventOrder after 2 accepted records";
    assert_eq!(fault.to_string(), format!("{shown}: finish twice"));
}
