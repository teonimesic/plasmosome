//! Typed reversibility: a plugin's effects, their inverses, and the two-phase
//! closure that decides whether detach is safe or needs an operator `Force`.
//! A caller pushes effects onto a `Ledger`, closes it, and detaches the result.

use std::fmt;
use std::io::Write;
use std::path::Path;

use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use plasmosome_backend::{
    BackendError, CellId, CellOwner, DrainSpec, EnforcementBackend, Handle, ObjectOnly, PluginId,
    UniverseRemoval,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inverse {
    pub description: String,
    pub via: InverseVia,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Inverse", deny_unknown_fields)]
struct InverseShape {
    description: String,
    via: InverseVia,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InverseVia {
    Backend(Handle),
    Universe(UniverseRemoval),
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "InverseVia", deny_unknown_fields)]
enum InverseViaShape {
    Backend(Handle),
    Universe(UniverseRemoval),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compensation {
    pub witness: UniverseRemoval,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Compensation", deny_unknown_fields)]
struct CompensationShape {
    witness: UniverseRemoval,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outbox {
    pub channel: String,
    pub payload: String,
    pub published: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Outbox")]
struct OutboxShape {
    channel: String,
    payload: String,
    published: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub assertion: String,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Policy")]
struct PolicyShape {
    assertion: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reversibility {
    Exact(Inverse),
    Compensating(Compensation),
    Delayed(Outbox),
    External(Policy),
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Reversibility", deny_unknown_fields)]
enum ReversibilityShape {
    Exact(Inverse),
    Compensating(Compensation),
    Delayed(Outbox),
    External(Policy),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    pub description: String,
    pub reversibility: Reversibility,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Effect", deny_unknown_fields)]
struct EffectShape {
    description: String,
    reversibility: Reversibility,
}

impl Effect {
    /// Records an effect whose inverse undoes it exactly.
    ///
    /// An `InverseVia::Universe` removal is replayed on behalf of the ledger's
    /// own plugin in the cell its detach names, so it must name an object that
    /// owner holds. A removal naming another owner's object, including the same
    /// plugin's object in another cell, is refused at detach, and refusing it
    /// stops the replay — pass `InverseVia::Backend` for anything granted to
    /// someone else.
    pub fn exact(description: impl Into<String>, via: InverseVia) -> Effect {
        let text: String = description.into();
        Effect {
            description: text.clone(),
            reversibility: Reversibility::Exact(Inverse {
                description: text,
                via,
            }),
        }
    }

    /// Records an effect that cannot be undone exactly, with the removal that
    /// stands as its compensation.
    ///
    /// The witness is replayed on behalf of the ledger's own plugin in the cell
    /// its detach names, and must name an object that owner holds. A
    /// compensation cannot retract another owner's object.
    pub fn compensating(description: impl Into<String>, witness: UniverseRemoval) -> Effect {
        Effect {
            description: description.into(),
            reversibility: Reversibility::Compensating(Compensation { witness }),
        }
    }

    pub fn delayed_unpublished(channel: &str, payload: &str) -> Effect {
        Effect {
            description: format!("delayed publication on {channel}"),
            reversibility: Reversibility::Delayed(Outbox {
                channel: channel.to_string(),
                payload: payload.to_string(),
                published: false,
            }),
        }
    }

    pub fn delayed_published(channel: &str, payload: &str) -> Effect {
        Effect {
            description: format!("delayed publication on {channel} (already published)"),
            reversibility: Reversibility::Delayed(Outbox {
                channel: channel.to_string(),
                payload: payload.to_string(),
                published: true,
            }),
        }
    }

    pub fn external(assertion: &str) -> Effect {
        Effect {
            description: format!("external emission ({assertion})"),
            reversibility: Reversibility::External(Policy {
                assertion: assertion.to_string(),
            }),
        }
    }
}

#[derive(Debug)]
pub struct Ledger {
    plugin: PluginId,
    effects: Vec<Effect>,
}

impl Ledger {
    pub fn new(plugin: impl Into<PluginId>) -> Ledger {
        Ledger {
            plugin: plugin.into(),
            effects: Vec::new(),
        }
    }

    pub fn push(&mut self, effect: Effect) {
        self.effects.push(effect);
    }

    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    pub fn len(&self) -> usize {
        self.effects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    pub fn close(self) -> Closure {
        let outstanding = self
            .effects
            .iter()
            .any(|effect| match &effect.reversibility {
                Reversibility::Exact(_) | Reversibility::Compensating(_) => false,
                Reversibility::Delayed(outbox) => outbox.published,
                Reversibility::External(_) => true,
            });
        let pending = self.effects.len();
        if outstanding {
            Closure::OutstandingExternal(ForcedLedger {
                plugin: self.plugin,
                effects: self.effects,
                pending,
                asserted: Vec::new(),
            })
        } else {
            Closure::ExternalFree(SealedLedger {
                plugin: self.plugin,
                effects: self.effects,
                pending,
                asserted: Vec::new(),
            })
        }
    }
}

pub enum Closure {
    ExternalFree(SealedLedger),
    OutstandingExternal(ForcedLedger),
}

#[derive(Debug)]
pub struct SealedLedger {
    plugin: PluginId,
    effects: Vec<Effect>,
    pending: usize,
    asserted: Vec<String>,
}

#[derive(Debug)]
pub struct ForcedLedger {
    plugin: PluginId,
    effects: Vec<Effect>,
    pending: usize,
    asserted: Vec<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Force {
    operator: &'static str,
    reason: String,
}

impl Force {
    pub fn operator_asserted(operator: &'static str, reason: impl Into<String>) -> Force {
        Force {
            operator,
            reason: reason.into(),
        }
    }

    pub fn assertion_line(&self) -> String {
        format!("operator `{}` asserted: {}", self.operator, self.reason)
    }
}

impl fmt::Debug for Force {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Force(operator: `{}`)", self.operator)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachReport {
    pub plugin: PluginId,
    pub replayed: Vec<String>,
    pub delayed_discarded: usize,
    pub asserted: Vec<String>,
    pub forced: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "DetachReport")]
struct DetachReportShape {
    plugin: PluginId,
    replayed: Vec<String>,
    delayed_discarded: usize,
    asserted: Vec<String>,
    forced: Option<String>,
}

macro_rules! object_serde {
    ($($record:ident through $shape:ident),* $(,)?) => {$(
        impl Serialize for $record {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                $shape::serialize(self, serializer)
            }
        }

        impl<'de> Deserialize<'de> for $record {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                $shape::deserialize(ObjectOnly::new(deserializer))
            }
        }
    )*};
}

object_serde!(
    Inverse through InverseShape,
    InverseVia through InverseViaShape,
    Compensation through CompensationShape,
    Outbox through OutboxShape,
    Policy through PolicyShape,
    Reversibility through ReversibilityShape,
    Effect through EffectShape,
    DetachReport through DetachReportShape,
);

impl DetachReport {
    pub fn new(plugin: impl Into<PluginId>) -> DetachReport {
        DetachReport {
            plugin: plugin.into(),
            replayed: Vec::new(),
            delayed_discarded: 0,
            asserted: Vec::new(),
            forced: None,
        }
    }
}

impl DetachReport {
    pub fn is_quiet(&self) -> bool {
        self.replayed.is_empty()
            && self.delayed_discarded == 0
            && self.asserted.is_empty()
            && self.forced.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetachError {
    Backend(BackendError),
}

impl fmt::Display for DetachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DetachError::Backend(e) => write!(f, "ledger replay failed: {e}"),
        }
    }
}

impl std::error::Error for DetachError {}

impl From<BackendError> for DetachError {
    fn from(value: BackendError) -> Self {
        DetachError::Backend(value)
    }
}

impl SealedLedger {
    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    /// Replays the pending effects last first. `cell` is the cell this plugin
    /// is attached to: every universe and compensation removal names the owner
    /// `{cell, plugin}`, and every removal and revoke uses `drain`. The cell is
    /// never read from the log or defaulted. On an error the pending cursor stays
    /// on the failed effect, so a later detach resumes there without replaying
    /// what already succeeded. Every call for one ledger, including each resume,
    /// must pass the same `cell`: the log does not record it, so this format
    /// cannot detect a different one.
    pub fn detach(
        &mut self,
        backend: &mut dyn EnforcementBackend,
        cell: &CellId,
        drain: DrainSpec,
    ) -> Result<DetachReport, DetachError> {
        let SealedLedger {
            plugin,
            effects,
            pending,
            asserted,
        } = self;
        let owner = CellOwner {
            cell: cell.clone(),
            plugin: plugin.clone(),
        };
        replay(&owner, effects, pending, asserted, backend, drain, None)
    }

    pub fn unseal(self) -> Ledger {
        Ledger {
            plugin: self.plugin,
            effects: self.effects,
        }
    }
}

impl ForcedLedger {
    pub fn plugin(&self) -> &PluginId {
        &self.plugin
    }

    pub fn external_assertions(&self) -> Vec<String> {
        self.effects
            .iter()
            .filter_map(|effect| match &effect.reversibility {
                Reversibility::External(policy) => Some(policy.assertion.clone()),
                Reversibility::Delayed(outbox) if outbox.published => {
                    Some(effect.description.clone())
                }
                _ => None,
            })
            .collect()
    }

    /// Replays the pending effects like `SealedLedger::detach`, with the same
    /// `cell` and `drain` rules, and records the operator's `force` in the report.
    /// Every call for one ledger, including each resume, must pass the same `cell`;
    /// this format cannot detect a different one.
    pub fn detach_forced(
        &mut self,
        backend: &mut dyn EnforcementBackend,
        cell: &CellId,
        drain: DrainSpec,
        force: Force,
    ) -> Result<DetachReport, DetachError> {
        let ForcedLedger {
            plugin,
            effects,
            pending,
            asserted,
        } = self;
        let owner = CellOwner {
            cell: cell.clone(),
            plugin: plugin.clone(),
        };
        replay(
            &owner,
            effects,
            pending,
            asserted,
            backend,
            drain,
            Some(force),
        )
    }

    pub fn unseal(self) -> Ledger {
        Ledger {
            plugin: self.plugin,
            effects: self.effects,
        }
    }
}

fn replay(
    owner: &CellOwner,
    effects: &[Effect],
    pending: &mut usize,
    asserted: &mut Vec<String>,
    backend: &mut dyn EnforcementBackend,
    drain: DrainSpec,
    forced: Option<Force>,
) -> Result<DetachReport, DetachError> {
    let mut report = DetachReport::new(owner.plugin.clone());
    for index in (0..*pending).rev() {
        let effect = &effects[index];
        match &effect.reversibility {
            Reversibility::Exact(inverse) => match &inverse.via {
                InverseVia::Backend(handle) => {
                    backend.revoke(*handle, drain)?;
                }
                InverseVia::Universe(removal) => {
                    backend.apply_removal(removal.clone(), owner, drain)?;
                }
            },
            Reversibility::Compensating(compensation) => {
                backend.apply_removal(compensation.witness.clone(), owner, drain)?;
            }
            Reversibility::Delayed(outbox) => {
                if outbox.published {
                    asserted.push(effect.description.clone());
                    *pending = index;
                    continue;
                }
                report.delayed_discarded += 1;
                *pending = index;
                continue;
            }
            Reversibility::External(policy) => {
                asserted.push(policy.assertion.clone());
                *pending = index;
                continue;
            }
        }
        report.replayed.push(effect.description.clone());
        *pending = index;
    }
    report.asserted = asserted.clone();
    report.forced = forced.map(|f| f.assertion_line());
    Ok(report)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    pub format: u8,
    pub plugin: PluginId,
    pub effect: Effect,
}

impl Serialize for LogRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.format != 2 {
            return Err(S::Error::custom(format!(
                "unsupported ledger format {}; expected 2",
                self.format
            )));
        }

        #[derive(Serialize)]
        struct Wire<'a> {
            format: u8,
            plugin: &'a PluginId,
            effect: &'a Effect,
        }

        Wire {
            format: self.format,
            plugin: &self.plugin,
            effect: &self.effect,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LogRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(expecting = "struct LogRecord", deny_unknown_fields)]
        struct Wire {
            format: u8,
            plugin: PluginId,
            effect: Effect,
        }

        let wire = Wire::deserialize(ObjectOnly::new(deserializer))?;
        if wire.format != 2 {
            return Err(D::Error::custom(format!(
                "unsupported ledger format {}; expected 2",
                wire.format
            )));
        }
        Ok(LogRecord {
            format: wire.format,
            plugin: wire.plugin,
            effect: wire.effect,
        })
    }
}

impl Ledger {
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    /// Appends one record per effect to `path`, creating it and its parent directories when
    /// missing. Every record is encoded before the file is touched, so an effect that cannot
    /// be encoded returns `InvalidData` and leaves the file, or its absence, as it was.
    pub fn append_to_file(&self, path: &Path) -> std::io::Result<usize> {
        let records = self.encode()?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(&records)?;
        file.flush()?;
        Ok(self.effects.len())
    }

    /// Writes one record per effect. Every record is encoded first, so an effect that cannot
    /// be encoded returns `InvalidData` and writes nothing.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> std::io::Result<usize> {
        writer.write_all(&self.encode()?)?;
        writer.flush()?;
        Ok(self.effects.len())
    }

    fn encode(&self) -> std::io::Result<Vec<u8>> {
        let mut records = Vec::new();
        for (index, effect) in self.effects.iter().enumerate() {
            let record = LogRecord {
                format: 2,
                plugin: self.plugin.clone(),
                effect: effect.clone(),
            };
            serde_json::to_writer(&mut records, &record).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "effect {} of {} cannot be encoded: {error}",
                        index + 1,
                        self.effects.len()
                    ),
                )
            })?;
            records.push(b'\n');
        }
        Ok(records)
    }

    pub fn open_file(path: &Path) -> std::io::Result<Ledger> {
        let bytes = std::fs::read(path)?;
        let mut plugin: Option<PluginId> = None;
        let mut effects = Vec::new();
        for (index, framed) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
            let newline_ended = framed.ends_with(b"\n");
            let line = framed.strip_suffix(b"\n").unwrap_or(framed);
            let parsed = match std::str::from_utf8(line) {
                Ok(text) => serde_json::from_str::<LogRecord>(text),
                Err(error) if !newline_ended && error.error_len().is_none() => {
                    serde_json::from_slice::<LogRecord>(line)
                }
                Err(error) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "invalid UTF-8 in ledger record on line {}: {error}",
                            index + 1
                        ),
                    ));
                }
            };
            let record = match parsed {
                Ok(record) => record,
                Err(error) if !newline_ended && error.is_eof() => break,
                Err(error) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("invalid ledger record on line {}: {error}", index + 1),
                    ));
                }
            };
            match &plugin {
                Some(existing) if *existing != record.plugin => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "ledger log mixes plugins on line {}: `{existing}` and `{}`",
                            index + 1,
                            record.plugin
                        ),
                    ));
                }
                Some(_) | None => {}
            }
            if plugin.is_none() {
                plugin = Some(record.plugin.clone());
            }
            effects.push(record.effect);
        }
        let plugin = plugin.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "ledger log holds no record",
            )
        })?;
        Ok(Ledger { plugin, effects })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plasmosome_backend::{
        Capability, CellId, CellOwner, Diff, FakeBackend, Grant, GrantId, GrantKind, UniverseOp,
    };
    use std::time::Duration;

    fn cell() -> CellId {
        CellId::from("cell-1")
    }

    fn cell_owner(plugin: &str) -> CellOwner {
        CellOwner {
            cell: cell(),
            plugin: PluginId::from(plugin),
        }
    }

    fn detach_either(
        closure: &mut Closure,
        backend: &mut FakeBackend,
        cell: &CellId,
        drain: DrainSpec,
    ) -> Result<DetachReport, DetachError> {
        match closure {
            Closure::ExternalFree(sealed) => sealed.detach(backend, cell, drain),
            Closure::OutstandingExternal(forced) => forced.detach_forced(
                backend,
                cell,
                drain,
                Force::operator_asserted("test", "the emission was approved"),
            ),
        }
    }

    fn universe_removal_effects() -> [fn(&str, UniverseRemoval) -> Effect; 2] {
        [
            |description, removal| Effect::exact(description, InverseVia::Universe(removal)),
            |description, removal| Effect::compensating(description, removal),
        ]
    }

    fn grant_uds(backend: &mut FakeBackend, path: &str) -> (Handle, UniverseRemoval) {
        let entry = backend.grant(Grant {
            owner: cell_owner("network"),
            capability: Capability::UdsSocket {
                path: path.to_string(),
            },
            kind: GrantKind::Hot,
        });
        (entry.handle, entry.removal())
    }

    fn grant_file(backend: &mut FakeBackend, path: &str) -> (Handle, UniverseRemoval) {
        let entry = backend.grant(Grant {
            owner: cell_owner("github-pr"),
            capability: Capability::SessionFile {
                path: path.to_string(),
            },
            kind: GrantKind::Hot,
        });
        (entry.handle, entry.removal())
    }

    fn populate(backend: &mut FakeBackend) -> Vec<(Handle, UniverseRemoval)> {
        vec![
            grant_uds(backend, "/run/ak/egressd.uds"),
            grant_uds(backend, "/run/ak/github.uds"),
            grant_file(backend, "/skills/pr.md"),
        ]
    }

    #[test]
    fn a_universe_inverse_naming_another_plugins_object_is_refused_and_leaves_it_standing() {
        let mut backend = FakeBackend::new();
        let op = UniverseOp::WriteSessionFile {
            id: GrantId::new(),
            path: "/skills/pr.md".to_string(),
            owner: cell_owner("workspace-bind"),
        };
        let removal = op.removal();
        backend.apply(op).unwrap();
        let mut ledger = Ledger::new("github-pr");
        ledger.push(Effect::exact(
            "a skill file this plugin did not write",
            InverseVia::Universe(removal),
        ));
        let Closure::ExternalFree(mut sealed) = ledger.close() else {
            panic!("a single exact effect closes as ExternalFree");
        };
        let error = sealed
            .detach(
                &mut backend,
                &cell(),
                DrainSpec::graceful(std::time::Duration::from_millis(1)),
            )
            .expect_err("a plugin may not withdraw an object another plugin owns");
        assert!(
            matches!(
                error,
                DetachError::Backend(BackendError::UnknownObject { .. })
            ),
            "refusal names the object the plugin does not hold: {error:?}"
        );
        assert_eq!(
            backend.snapshot_os_state().len(),
            1,
            "the object its real owner holds must still be standing"
        );
    }

    #[test]
    fn detach_replays_effects_in_reverse_push_order() {
        let mut backend = FakeBackend::new();
        let objects = populate(&mut backend);
        let mut ledger = Ledger::new("network");
        for (index, (handle, _)) in objects.iter().enumerate() {
            ledger.push(Effect::exact(
                format!("effect {index}"),
                InverseVia::Backend(*handle),
            ));
        }
        let Closure::ExternalFree(mut sealed) = ledger.close() else {
            panic!("an Exact-only ledger must close as ExternalFree");
        };
        let report = sealed
            .detach(
                &mut backend,
                &cell(),
                DrainSpec::graceful(std::time::Duration::from_millis(1)),
            )
            .unwrap();
        assert_eq!(report.plugin, PluginId::from("network"));
        assert_eq!(
            report.replayed,
            vec!["effect 2", "effect 1", "effect 0"],
            "LIFO: last pushed replays first"
        );
        assert!(backend.snapshot_os_state().is_empty());
    }

    #[test]
    fn replay_over_exact_compensating_and_delayed_produces_an_empty_diff() {
        let mut backend = FakeBackend::new();
        backend
            .apply(UniverseOp::SetProxyMap {
                id: GrantId::new(),
                host: "api.github.com".to_string(),
                route: "staged".to_string(),
                owner: cell_owner("github-pr"),
            })
            .unwrap();
        let before = backend.snapshot_os_state();
        let (handle_a, _) = grant_uds(&mut backend, "/run/ak/egressd.uds");
        let (handle_b, _) = grant_file(&mut backend, "/skills/pr.md");
        let mut ledger = Ledger::new("github-pr");
        ledger.push(Effect::exact(
            "egress socket",
            InverseVia::Backend(handle_a),
        ));
        ledger.push(Effect::exact(
            "injected skill file",
            InverseVia::Backend(handle_b),
        ));
        let op = UniverseOp::SetProxyMap {
            id: GrantId::new(),
            host: "api.github.com".to_string(),
            route: "staged".to_string(),
            owner: cell_owner("github-pr"),
        };
        ledger.push(Effect::compensating(
            "posted a comment; compensation retracts the staged row",
            op.removal(),
        ));
        backend.apply(op).unwrap();
        ledger.push(Effect::delayed_unpublished(
            "outbox/github",
            "post-comment payload",
        ));
        let Closure::ExternalFree(mut sealed) = ledger.close() else {
            panic!("Exact/Compensating/Delayed closes as ExternalFree");
        };
        let report = sealed
            .detach(
                &mut backend,
                &cell(),
                DrainSpec::graceful(std::time::Duration::from_millis(1)),
            )
            .unwrap();
        assert_eq!(
            report.delayed_discarded, 1,
            "an unpublished outbox entry is dropped, never published"
        );
        assert!(report.asserted.is_empty());
        let after = backend.snapshot_os_state();
        assert!(
            Diff::between(&before, &after).is_empty(),
            "replay must restore the pre-attach universe"
        );
    }

    #[test]
    fn an_outstanding_external_entry_closes_as_forced_and_has_no_safe_detach() {
        let mut ledger = Ledger::new("github-pr");
        ledger.push(Effect::external("the PR comment already left the host"));
        let closure = ledger.close();
        let Closure::OutstandingExternal(forced) = closure else {
            panic!("an External entry must route the ledger to OutstandingExternal");
        };
        let mut forced = forced;
        let mut backend = FakeBackend::new();
        let before = backend.snapshot_os_state();
        let report = forced
            .detach_forced(
                &mut backend,
                &cell(),
                DrainSpec::forcing(),
                Force::operator_asserted("stefano", "the emission was approved"),
            )
            .unwrap();
        assert_eq!(report.asserted.len(), 1);
        assert_eq!(report.asserted[0], "the PR comment already left the host");
        assert_eq!(
            report.forced.as_deref(),
            Some("operator `stefano` asserted: the emission was approved")
        );
        assert!(Diff::between(&before, &backend.snapshot_os_state()).is_empty());
    }

    #[test]
    fn a_published_delayed_entry_is_no_longer_reversible() {
        let mut ledger = Ledger::new("github");
        ledger.push(Effect::delayed_published("outbox/github", "merge payload"));
        let Closure::OutstandingExternal(_) = ledger.close() else {
            panic!("a published outbox entry has crossed the reversible boundary");
        };
    }

    #[test]
    fn an_unpublished_delayed_entry_still_closes_safely() {
        let mut ledger = Ledger::new("github");
        ledger.push(Effect::delayed_unpublished(
            "outbox/github",
            "merge payload",
        ));
        assert!(matches!(ledger.close(), Closure::ExternalFree(_)));
    }

    #[test]
    fn force_is_constructed_only_by_naming_the_operator() {
        let force = Force::operator_asserted("stefano", "rotation window");
        assert_eq!(
            force.assertion_line(),
            "operator `stefano` asserted: rotation window"
        );
    }

    #[test]
    fn a_failed_replay_leaves_the_ledger_retryable_with_a_forcing_drain() {
        let mut backend = FakeBackend::new();
        let objects = populate(&mut backend);
        let stuck_handle = objects[0].0;
        backend.mark_stuck(stuck_handle);
        let mut ledger = Ledger::new("network");
        for (index, (handle, _)) in objects.iter().enumerate() {
            ledger.push(Effect::exact(
                format!("effect {index}"),
                InverseVia::Backend(*handle),
            ));
        }
        let Closure::ExternalFree(mut sealed) = ledger.close() else {
            panic!("Exact-only ledger closes as ExternalFree");
        };
        let graceful = DrainSpec::graceful(std::time::Duration::from_millis(2));
        assert!(
            sealed.detach(&mut backend, &cell(), graceful).is_err(),
            "stuck handle must refuse graceful drain"
        );
        assert!(!backend.snapshot_os_state().is_empty());
        let forced = DrainSpec::forcing();
        let report = sealed.detach(&mut backend, &cell(), forced).unwrap();
        assert_eq!(
            report.replayed,
            vec!["effect 0"],
            "the retry resumes at the stuck entry; entries already replayed must not replay twice"
        );
        assert!(
            backend.snapshot_os_state().is_empty(),
            "the pre-registered force deadline reclaims it"
        );
    }

    #[test]
    fn a_sealed_ledger_can_be_returned_to_its_open_form() {
        let mut ledger = Ledger::new("github");
        ledger.push(Effect::delayed_unpublished("outbox/github", "payload"));
        let Closure::ExternalFree(sealed) = ledger.close() else {
            panic!("must seal");
        };
        let reopened = sealed.unseal();
        assert_eq!(reopened.plugin(), &PluginId::from("github"));
        assert_eq!(reopened.len(), 1);
        assert!(matches!(reopened.close(), Closure::ExternalFree(_)));
    }

    #[test]
    fn a_forced_ledger_names_its_external_entries_before_any_force_is_given() {
        let mut ledger = Ledger::new("github");
        ledger.push(Effect::delayed_unpublished("outbox/github", "payload"));
        ledger.push(Effect::external("emission one"));
        ledger.push(Effect::external("emission two"));
        let Closure::OutstandingExternal(forced) = ledger.close() else {
            panic!("must route to forced");
        };
        assert_eq!(
            forced.external_assertions(),
            vec!["emission one", "emission two"]
        );
        let reopened = forced.unseal();
        assert_eq!(reopened.len(), 3);
    }

    #[test]
    fn a_forced_detach_can_be_taken_again_without_replaying_applied_entries() {
        let mut backend = FakeBackend::new();
        let objects = populate(&mut backend);
        let stuck_handle = objects[0].0;
        backend.mark_stuck(stuck_handle);
        let mut ledger = Ledger::new("network");
        for (index, (handle, _)) in objects.iter().enumerate() {
            ledger.push(Effect::exact(
                format!("effect {index}"),
                InverseVia::Backend(*handle),
            ));
        }
        ledger.push(Effect::external("emission"));
        let Closure::OutstandingExternal(forced) = ledger.close() else {
            panic!("an External entry must route the ledger to forced closure");
        };
        let mut forced = forced;
        let graceful = DrainSpec::graceful(std::time::Duration::from_millis(2));
        let first_force = Force::operator_asserted("t", "r");
        assert!(
            forced
                .detach_forced(&mut backend, &cell(), graceful, first_force)
                .is_err()
        );
        let report = forced
            .detach_forced(
                &mut backend,
                &cell(),
                DrainSpec::forcing(),
                Force::operator_asserted("t", "r"),
            )
            .unwrap();
        assert_eq!(report.replayed, vec!["effect 0"]);
        assert_eq!(report.asserted, vec!["emission"]);
        assert!(backend.snapshot_os_state().is_empty());
    }

    #[test]
    fn a_graceful_detach_keeps_a_stuck_universe_inverse_pending_until_force_replays_it_once() {
        for make_effect in universe_removal_effects() {
            for with_external in [false, true] {
                let owner = cell_owner("github-pr");
                let stuck = UniverseOp::WriteSessionFile {
                    id: GrantId::new(),
                    path: "/skills/pr.md".to_string(),
                    owner: owner.clone(),
                };
                let healthy = UniverseOp::WriteSessionFile {
                    id: GrantId::new(),
                    path: "/skills/pr.md".to_string(),
                    owner,
                };
                let address = Handle {
                    class: stuck.class(),
                    id: stuck.id(),
                };
                let mut backend = FakeBackend::new();
                backend.apply(stuck.clone()).unwrap();
                backend.apply(healthy.clone()).unwrap();
                backend.mark_stuck(address);
                let mut ledger = Ledger::new("github-pr");
                if with_external {
                    ledger.push(Effect::external("the PR comment already left the host"));
                }
                ledger.push(make_effect("stuck skill file", stuck.removal()));
                ledger.push(make_effect("healthy skill file", healthy.removal()));
                let mut closure = ledger.close();
                assert_eq!(
                    detach_either(
                        &mut closure,
                        &mut backend,
                        &cell(),
                        DrainSpec::graceful(Duration::from_millis(4))
                    )
                    .unwrap_err(),
                    DetachError::Backend(BackendError::DrainTimedOut {
                        handle: address,
                        deadline_ms: 4,
                    })
                );
                assert_eq!(
                    backend.snapshot_os_state().objects().collect::<Vec<_>>(),
                    vec![&stuck.object()]
                );
                let resumed =
                    detach_either(&mut closure, &mut backend, &cell(), DrainSpec::forcing())
                        .unwrap();
                assert_eq!(resumed.replayed, vec!["stuck skill file"]);
                assert!(backend.snapshot_os_state().is_empty());
                let again =
                    detach_either(&mut closure, &mut backend, &cell(), DrainSpec::forcing())
                        .unwrap();
                assert!(again.replayed.is_empty());
            }
        }
    }

    #[test]
    fn a_universe_inverse_naming_the_same_plugins_object_in_another_cell_is_refused() {
        for make_effect in universe_removal_effects() {
            for with_external in [false, true] {
                let elsewhere = CellId::from("cell-2");
                let op = UniverseOp::WriteSessionFile {
                    id: GrantId::new(),
                    path: "/skills/pr.md".to_string(),
                    owner: CellOwner {
                        cell: elsewhere.clone(),
                        plugin: PluginId::from("github-pr"),
                    },
                };
                let mut backend = FakeBackend::new();
                backend.apply(op.clone()).unwrap();
                let mut ledger = Ledger::new("github-pr");
                if with_external {
                    ledger.push(Effect::external("the PR comment already left the host"));
                }
                ledger.push(make_effect("skill file in cell-2", op.removal()));
                let mut closure = ledger.close();
                for drain in [
                    DrainSpec::graceful(Duration::from_millis(1)),
                    DrainSpec::forcing(),
                ] {
                    assert_eq!(
                        detach_either(&mut closure, &mut backend, &cell(), drain).unwrap_err(),
                        DetachError::Backend(BackendError::UnknownObject {
                            class: "session-file",
                            key: "session//skills/pr.md".to_string(),
                            owner: cell_owner("github-pr"),
                            id: op.id(),
                        })
                    );
                    assert_eq!(
                        backend.snapshot_os_state().objects().collect::<Vec<_>>(),
                        vec![&op.object()]
                    );
                }
                let report =
                    detach_either(&mut closure, &mut backend, &elsewhere, DrainSpec::forcing())
                        .unwrap();
                assert_eq!(report.replayed, vec!["skill file in cell-2"]);
                assert!(backend.snapshot_os_state().is_empty());
            }
        }
    }

    fn sequence_accepted<T>(record: &str, value: &T, at: &str, fields: &[&str]) -> Vec<String>
    where
        T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let encoded = serde_json::to_value(value).expect("a valid record encodes");
        assert_eq!(
            &serde_json::from_value::<T>(encoded.clone()).expect("its object decodes"),
            value
        );
        let mut array = encoded;
        let fields_at = array
            .pointer_mut(at)
            .expect("the record sits at its pointer");
        assert_eq!(
            fields_at.as_object().map(serde_json::Map::len),
            Some(fields.len()),
            "{fields_at} must hold exactly the fields {fields:?}"
        );
        *fields_at = serde_json::Value::Array(
            fields
                .iter()
                .map(|field| fields_at[*field].clone())
                .collect(),
        );
        let mut accepted = Vec::new();
        let through_value = serde_json::from_value::<T>(array.clone()).map(drop);
        let through_text = serde_json::from_str::<T>(&array.to_string()).map(drop);
        for (path, outcome) in [("value", through_value), ("text", through_text)] {
            match outcome {
                Err(error)
                    if error.is_data() && error.to_string().contains("invalid type: sequence") => {}
                other => accepted.push(format!("{record} at {at:?} through {path}: {other:?}")),
            }
        }
        accepted
    }

    #[test]
    fn a_positional_log_record_is_refused_by_its_public_name() {
        for error in [
            serde_json::from_str::<LogRecord>("[]").unwrap_err(),
            serde_json::from_value::<LogRecord>(serde_json::json!([])).unwrap_err(),
        ] {
            assert!(
                error
                    .to_string()
                    .contains("invalid type: sequence, expected struct LogRecord"),
                "{error}"
            );
        }
    }

    #[test]
    fn every_ledger_record_decodes_from_an_object_and_refuses_a_positional_array() {
        let removal = UniverseRemoval {
            id: GrantId::new(),
            capability: Capability::SessionFile {
                path: "/skills/pr.md".to_string(),
            },
        };
        let exact = Effect::exact("skill file", InverseVia::Universe(removal.clone()));
        let Reversibility::Exact(inverse) = exact.reversibility.clone() else {
            panic!("an exact effect carries an inverse");
        };
        let delayed = Effect::delayed_published("outbox/github", "payload");
        let Reversibility::Delayed(outbox) = delayed.reversibility.clone() else {
            panic!("a delayed effect carries an outbox");
        };
        let mut report = DetachReport::new("github-pr");
        report.replayed.push("skill file".to_string());
        let mut accepted = Vec::new();
        accepted.extend(sequence_accepted(
            "Inverse",
            &inverse,
            "",
            &["description", "via"],
        ));
        accepted.extend(sequence_accepted(
            "Compensation",
            &Compensation { witness: removal },
            "",
            &["witness"],
        ));
        accepted.extend(sequence_accepted(
            "Outbox",
            &outbox,
            "",
            &["channel", "payload", "published"],
        ));
        accepted.extend(sequence_accepted(
            "Policy",
            &Policy {
                assertion: "the comment left the host".to_string(),
            },
            "",
            &["assertion"],
        ));
        accepted.extend(sequence_accepted(
            "Effect",
            &exact,
            "",
            &["description", "reversibility"],
        ));
        accepted.extend(sequence_accepted(
            "DetachReport",
            &report,
            "",
            &[
                "plugin",
                "replayed",
                "delayed_discarded",
                "asserted",
                "forced",
            ],
        ));
        accepted.extend(sequence_accepted(
            "LogRecord",
            &LogRecord {
                format: 2,
                plugin: PluginId::from("github-pr"),
                effect: exact,
            },
            "",
            &["format", "plugin", "effect"],
        ));
        assert!(
            accepted.is_empty(),
            "these positional arrays decoded:\n{}",
            accepted.join("\n")
        );
    }
}
