use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::net::IpAddr;

use serde_json::{Map, Value};

use super::{Lane, Manifest, Observation, Rule, RuleKind, RunRefusal};

#[derive(Clone, Copy)]
enum Numbering {
    Across(RuleKind),
    Rank,
    Within,
    None,
}

fn numbering(kind: RuleKind) -> Numbering {
    match kind {
        RuleKind::Pid
        | RuleKind::BootId
        | RuleKind::MountId
        | RuleKind::PeerGroup
        | RuleKind::Device
        | RuleKind::Inode
        | RuleKind::EphemeralPort
        | RuleKind::ResolvedAddress => Numbering::Across(kind),
        RuleKind::ParentMountId => Numbering::Across(RuleKind::MountId),
        RuleKind::SystemTime => Numbering::Rank,
        RuleKind::MemoryCounter | RuleKind::LoadCounter => Numbering::Within,
        RuleKind::Duration => Numbering::None,
    }
}

pub(super) fn normalize(
    manifest: &Manifest,
    lane: Lane,
    observation: &Observation,
) -> Result<Map<String, Value>, RunRefusal> {
    let mut fields = observation.fields.clone();
    let mut rules: Vec<&Rule> = manifest
        .rules
        .iter()
        .filter(|rule| rule.probe == observation.probe)
        .collect();
    rules.sort_by(|left, right| left.field.cmp(&right.field));
    let mut across: BTreeMap<RuleKind, Vec<Value>> = BTreeMap::new();
    for rule in rules {
        let Some(value) = fields.get_mut(&rule.field) else {
            continue;
        };
        let values: Vec<&mut Value> = match value {
            Value::Array(items) => items.iter_mut().collect(),
            one => vec![one],
        };
        let how = numbering(rule.kind);
        let ranked = matches!(how, Numbering::Rank);
        let fits = values.iter().all(|value| holds(rule.kind, value));
        let mixed = ranked
            && values.iter().any(|value| value.is_f64())
            && values.iter().any(|value| !value.is_f64());
        if !fits || mixed {
            return Err(RunRefusal::RuleCannotHold {
                lane,
                probe: rule.probe.clone(),
                field: rule.field.clone(),
                kind: rule.kind,
            });
        }
        let mut within = Vec::new();
        let (kind, seen) = match how {
            Numbering::Across(shared) => (shared, across.entry(shared).or_default()),
            Numbering::Rank | Numbering::Within => (rule.kind, &mut within),
            Numbering::None => {
                for value in values {
                    *value = Value::String(format!("<{}>", rule.kind.name()));
                }
                continue;
            }
        };
        for value in &values {
            if !seen.contains(value) {
                seen.push(Value::clone(value));
            }
        }
        if ranked {
            seen.sort_by(earlier);
        }
        for value in values {
            let number = 1 + seen.iter().take_while(|seen| **seen != *value).count();
            *value = Value::String(format!("<{}:{number}>", kind.name()));
        }
    }
    Ok(fields)
}

fn holds(kind: RuleKind, value: &Value) -> bool {
    match kind {
        RuleKind::Pid
        | RuleKind::MountId
        | RuleKind::ParentMountId
        | RuleKind::PeerGroup
        | RuleKind::Device
        | RuleKind::Inode
        | RuleKind::MemoryCounter => value.is_u64(),
        RuleKind::EphemeralPort => value
            .as_u64()
            .is_some_and(|port| port <= u64::from(u16::MAX)),
        RuleKind::SystemTime | RuleKind::Duration | RuleKind::LoadCounter => value.is_number(),
        RuleKind::BootId => value.as_str().is_some_and(is_uuid),
        RuleKind::ResolvedAddress => value
            .as_str()
            .is_some_and(|text| text.parse::<IpAddr>().is_ok()),
    }
}

fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.iter().map(|group| group.len()).eq([8, 4, 4, 4, 12])
        && groups
            .iter()
            .all(|group| group.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn earlier(left: &Value, right: &Value) -> Ordering {
    match (whole(left), whole(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => fraction(left).total_cmp(&fraction(right)),
    }
}

fn whole(value: &Value) -> Option<i128> {
    value
        .as_i64()
        .map(i128::from)
        .or_else(|| value.as_u64().map(i128::from))
}

fn fraction(value: &Value) -> f64 {
    value.as_f64().unwrap_or_default()
}
