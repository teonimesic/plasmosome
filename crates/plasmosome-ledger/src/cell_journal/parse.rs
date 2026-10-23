use std::collections::BTreeSet;

use plasmosome_backend::{CellId, CellOwner, MockMode, ObjectOnly, PluginId};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::value::{RawValue, to_raw_value};

use super::{
    AttachmentSource, CELL_JOURNAL_FORMAT, CellJournalFaultKind, CellJournalRecord, Change,
    ForceAssertion, JournalEvent, JournalRuleError, Prepare, RecordedEffect, Replacement,
};

/// Reads one journal line as a record of `cell`'s journal. A final CR or LF is JSON whitespace.
///
/// The cell comes from the journal's directory, never from the bytes: every embedded operation
/// must name it and its change's plugin as owner. It refuses with `Unparseable` anything that is
/// not one strict format 1 record, with `UnsupportedFormat` an object whose `format` is not the
/// integer 1, with `GenerationZero` generation 0, with `InvalidSource` a bad source and with
/// `InvalidEffect` a bad effect or operation. It checks no other record of the journal.
pub fn parse_record(cell: &CellId, line: &[u8]) -> Result<CellJournalRecord, JournalRuleError> {
    let text = std::str::from_utf8(line).map_err(|error| unparseable(error.to_string()))?;
    if let Ok(Object(Probe { format })) = serde_json::from_str::<Object<Probe>>(text)
        && !format
            .as_ref()
            .is_some_and(|value| value.as_u64() == Some(CELL_JOURNAL_FORMAT))
    {
        let found = format.map_or_else(|| "missing".to_string(), |value| value.to_string());
        return Err(JournalRuleError::new(
            CellJournalFaultKind::UnsupportedFormat,
            format!("format {found} is not {CELL_JOURNAL_FORMAT}"),
        ));
    }
    let Object(wire) =
        serde_json::from_str::<Object<WireRecord>>(text).map_err(|e| unparseable(e.to_string()))?;
    if wire.generation == 0 {
        return Err(JournalRuleError::new(
            CellJournalFaultKind::GenerationZero,
            "generation 0 appears in no record",
        ));
    }
    let event = match wire.event.0 {
        WireEvent::Prepare(Object(prepare)) => JournalEvent::Prepare(prepare_from(cell, prepare)?),
        WireEvent::Commit {} => JournalEvent::Commit,
        WireEvent::Abort {} => JournalEvent::Abort,
        WireEvent::Finish {} => JournalEvent::Finish,
    };
    Ok(CellJournalRecord {
        generation: wire.generation,
        event,
    })
}

/// Writes `record` as one compact JSON object and a final LF, the bytes `parse_record` reads
/// back. It checks no cell rule, and returns the encoder's error for a value that cannot be
/// encoded, such as an operation its own `validate` refuses.
pub fn encode_record(record: &CellJournalRecord) -> Result<Vec<u8>, serde_json::Error> {
    let event = match &record.event {
        JournalEvent::Prepare(prepare) => WireEvent::Prepare(Object(WirePrepare {
            changes: prepare
                .changes
                .iter()
                .map(wire_change)
                .collect::<Result<_, _>>()?,
            force: prepare.force.as_ref().map(|force| {
                Object(WireForce {
                    operator: force.operator.clone(),
                    reason: force.reason.clone(),
                })
            }),
        })),
        JournalEvent::Commit => WireEvent::Commit {},
        JournalEvent::Abort => WireEvent::Abort {},
        JournalEvent::Finish => WireEvent::Finish {},
    };
    let mut bytes = serde_json::to_vec(&WireRecord {
        format: CELL_JOURNAL_FORMAT,
        generation: record.generation,
        event: Object(event),
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn wire_change(change: &Change) -> Result<Object<WireChange>, serde_json::Error> {
    let replacement = match &change.replacement {
        None => None,
        Some(replacement) => Some(Object(WireReplacement {
            mock: replacement.mock,
            source: to_raw_value(&replacement.source)?,
            effects: replacement
                .effects
                .iter()
                .map(|recorded| {
                    Ok(Object(WireEffect {
                        effect: to_raw_value(&recorded.effect)?,
                        operation: to_raw_value(&recorded.operation)?,
                    }))
                })
                .collect::<Result<_, serde_json::Error>>()?,
        })),
    };
    Ok(Object(WireChange {
        plugin: change.plugin.clone(),
        replacement,
    }))
}

fn unparseable(detail: impl Into<String>) -> JournalRuleError {
    JournalRuleError::new(CellJournalFaultKind::Unparseable, detail)
}

fn prepare_from(cell: &CellId, wire: WirePrepare) -> Result<Prepare, JournalRuleError> {
    if wire.changes.is_empty() {
        return Err(unparseable("a prepare has no changes"));
    }
    let mut plugins = BTreeSet::new();
    if let Some(Object(change)) = wire.changes.iter().find(|c| !plugins.insert(&c.0.plugin)) {
        return Err(unparseable(format!(
            "plugin {:?} changes twice",
            change.plugin.as_str()
        )));
    }
    let force = match wire.force {
        Some(Object(force))
            if force.operator.trim().is_empty() || force.reason.trim().is_empty() =>
        {
            return Err(unparseable("a force needs an operator and a reason"));
        }
        Some(Object(WireForce { operator, reason })) => Some(ForceAssertion { operator, reason }),
        None => None,
    };
    let changes = wire
        .changes
        .into_iter()
        .map(|Object(change)| change_from(cell, change))
        .collect::<Result<_, _>>()?;
    Ok(Prepare { changes, force })
}

fn change_from(cell: &CellId, wire: WireChange) -> Result<Change, JournalRuleError> {
    let Some(Object(replacement)) = wire.replacement else {
        return Ok(Change {
            plugin: wire.plugin,
            replacement: None,
        });
    };
    let source: AttachmentSource = serde_json::from_str(replacement.source.get())
        .map_err(|e| JournalRuleError::new(CellJournalFaultKind::InvalidSource, e.to_string()))?;
    source.validate_member(&wire.plugin)?;
    let owner = CellOwner {
        cell: cell.clone(),
        plugin: wire.plugin,
    };
    let effects = replacement
        .effects
        .into_iter()
        .map(|Object(effect)| effect_from(&owner, effect))
        .collect::<Result<_, _>>()?;
    Ok(Change {
        plugin: owner.plugin,
        replacement: Some(Replacement {
            mock: replacement.mock,
            source,
            effects,
        }),
    })
}

fn effect_from(owner: &CellOwner, wire: WireEffect) -> Result<RecordedEffect, JournalRuleError> {
    let invalid = |e: serde_json::Error| {
        JournalRuleError::new(CellJournalFaultKind::InvalidEffect, e.to_string())
    };
    let recorded = RecordedEffect {
        effect: serde_json::from_str(wire.effect.get()).map_err(invalid)?,
        operation: serde_json::from_str(wire.operation.get()).map_err(invalid)?,
    };
    recorded.validate(owner)?;
    Ok(recorded)
}

struct Object<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(ObjectOnly::new(deserializer)).map(Object)
    }
}

impl<T: Serialize> Serialize for Object<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

fn required_nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}

fn mock_text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<MockMode, D::Error> {
    let text = String::deserialize(deserializer)?;
    MockMode::parse(&text)
        .ok_or_else(|| D::Error::custom(format_args!("unknown mock mode {text:?}")))
}

#[derive(Deserialize)]
struct Probe {
    format: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRecord {
    format: u64,
    generation: u64,
    event: Object<WireEvent>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "lowercase")]
enum WireEvent {
    Prepare(Object<WirePrepare>),
    Commit {},
    Abort {},
    Finish {},
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePrepare {
    changes: Vec<Object<WireChange>>,
    #[serde(deserialize_with = "required_nullable")]
    force: Option<Object<WireForce>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireForce {
    operator: String,
    reason: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireChange {
    plugin: PluginId,
    #[serde(deserialize_with = "required_nullable")]
    replacement: Option<Object<WireReplacement>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireReplacement {
    #[serde(deserialize_with = "mock_text")]
    mock: MockMode,
    source: Box<RawValue>,
    effects: Vec<Object<WireEffect>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEffect {
    effect: Box<RawValue>,
    operation: Box<RawValue>,
}
