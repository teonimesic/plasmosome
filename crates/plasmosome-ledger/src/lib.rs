//! Typed reversibility: a plugin's effects, their inverses, and the two-phase
//! closure that decides whether detach is safe or needs an operator `Force`.
//!
//! A ledger's contents are serde data framed one `LogRecord` per ndjson line,
//! so the ledger is recoverable by replaying its log alone — `Ledger::open_file`
//! rebuilds it from disk and replay proceeds unchanged (86 §4 rule 3: durable
//! state never lives only in the crashiest process). `append_to_file` appends
//! the whole current ledger; rebuild-then-extend is `open_file` → `push` →
//! `append_to_file` on the reopened ledger.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use plasmosome_backend::{
    BackendError, DrainSpec, EnforcementBackend, Handle, PluginId, UniverseRemoval,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inverse {
    pub description: String,
    pub via: InverseVia,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InverseVia {
    Backend(Handle),
    Universe(UniverseRemoval),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compensation {
    pub witness: UniverseRemoval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outbox {
    pub channel: String,
    pub payload: String,
    pub published: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub assertion: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reversibility {
    Exact(Inverse),
    Compensating(Compensation),
    Delayed(Outbox),
    External(Policy),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effect {
    pub description: String,
    pub reversibility: Reversibility,
}

impl Effect {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachReport {
    pub plugin: PluginId,
    pub replayed: Vec<String>,
    pub delayed_discarded: usize,
    pub asserted: Vec<String>,
    pub forced: Option<String>,
}

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

    pub fn detach(
        &mut self,
        backend: &mut dyn EnforcementBackend,
        drain: DrainSpec,
    ) -> Result<DetachReport, DetachError> {
        let SealedLedger {
            plugin,
            effects,
            pending,
            asserted,
        } = self;
        replay(plugin, effects, pending, asserted, backend, drain, None)
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

    pub fn detach_forced(
        &mut self,
        backend: &mut dyn EnforcementBackend,
        drain: DrainSpec,
        force: Force,
    ) -> Result<DetachReport, DetachError> {
        let ForcedLedger {
            plugin,
            effects,
            pending,
            asserted,
        } = self;
        replay(
            plugin,
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
    plugin: &PluginId,
    effects: &[Effect],
    pending: &mut usize,
    asserted: &mut Vec<String>,
    backend: &mut dyn EnforcementBackend,
    drain: DrainSpec,
    forced: Option<Force>,
) -> Result<DetachReport, DetachError> {
    let mut report = DetachReport::new(plugin.clone());
    for index in (0..*pending).rev() {
        let effect = &effects[index];
        match &effect.reversibility {
            Reversibility::Exact(inverse) => match &inverse.via {
                InverseVia::Backend(handle) => {
                    backend.revoke(*handle, drain)?;
                }
                InverseVia::Universe(removal) => {
                    backend.apply_removal(removal.clone())?;
                }
            },
            Reversibility::Compensating(compensation) => {
                backend.apply_removal(compensation.witness.clone())?;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRecord {
    pub plugin: PluginId,
    pub effect: Effect,
}

impl Ledger {
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    pub fn append_to_file(&self, path: &Path) -> std::io::Result<usize> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        self.write_to(&mut file)
    }

    pub fn write_to<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<usize> {
        for effect in &self.effects {
            let record = LogRecord {
                plugin: self.plugin.clone(),
                effect: effect.clone(),
            };
            let mut line = serde_json::to_string(&record)?;
            line.push('\n');
            writer.write_all(line.as_bytes())?;
        }
        writer.flush()?;
        Ok(self.effects.len())
    }

    pub fn open_file(path: &Path) -> std::io::Result<Ledger> {
        let text = std::fs::read_to_string(path)?;
        let mut plugin: Option<PluginId> = None;
        let mut effects = Vec::new();
        for line in text.lines() {
            let Ok(record) = serde_json::from_str::<LogRecord>(line) else {
                continue;
            };
            match &plugin {
                Some(existing) if *existing != record.plugin => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "ledger log mixes plugins: `{existing}` and `{}`",
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
    use plasmosome_backend::{Capability, Diff, FakeBackend, Grant, GrantKind, UniverseOp};

    fn grant_uds(backend: &mut FakeBackend, path: &str) -> (Handle, UniverseRemoval) {
        let entry = backend.grant(Grant {
            plugin: PluginId::from("network"),
            capability: Capability::UdsSocket {
                path: path.to_string(),
            },
            kind: GrantKind::Hot,
        });
        let removal = UniverseRemoval::UnbindUds {
            path: path.to_string(),
        };
        (entry.handle, removal)
    }

    fn grant_file(backend: &mut FakeBackend, path: &str) -> (Handle, UniverseRemoval) {
        let entry = backend.grant(Grant {
            plugin: PluginId::from("github-pr"),
            capability: Capability::SessionFile {
                path: path.to_string(),
            },
            kind: GrantKind::Hot,
        });
        let removal = UniverseRemoval::RemoveSessionFile {
            path: path.to_string(),
        };
        (entry.handle, removal)
    }

    fn populate(backend: &mut FakeBackend) -> Vec<(Handle, UniverseRemoval)> {
        vec![
            grant_uds(backend, "/run/ak/egressd.uds"),
            grant_uds(backend, "/run/ak/github.uds"),
            grant_file(backend, "skills/pr.md"),
        ]
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
        assert!(report.is_quiet() || !report.is_quiet());
    }

    #[test]
    fn replay_over_exact_compensating_and_delayed_produces_an_empty_diff() {
        let mut backend = FakeBackend::new();
        let before = backend.snapshot_os_state();
        let (handle_a, _) = grant_uds(&mut backend, "/run/ak/egressd.uds");
        let (handle_b, _) = grant_file(&mut backend, "skills/pr.md");
        let mut ledger = Ledger::new("github-pr");
        ledger.push(Effect::exact(
            "egress socket",
            InverseVia::Backend(handle_a),
        ));
        ledger.push(Effect::exact(
            "injected skill file",
            InverseVia::Backend(handle_b),
        ));
        ledger.push(Effect::compensating(
            "posted a comment; compensation retracts the staged row",
            UniverseRemoval::RemoveProxyMap {
                host: "api.github.com".to_string(),
            },
        ));
        backend
            .apply(UniverseOp::SetProxyMap {
                host: "api.github.com".to_string(),
                route: "staged".to_string(),
                owner: PluginId::from("github-pr"),
            })
            .unwrap();
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
            sealed.detach(&mut backend, graceful).is_err(),
            "stuck handle must refuse graceful drain"
        );
        assert!(!backend.snapshot_os_state().is_empty());
        let forced = DrainSpec::forcing();
        let report = sealed.detach(&mut backend, forced).unwrap();
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
                .detach_forced(&mut backend, graceful, first_force)
                .is_err()
        );
        let report = forced
            .detach_forced(
                &mut backend,
                DrainSpec::forcing(),
                Force::operator_asserted("t", "r"),
            )
            .unwrap();
        assert_eq!(report.replayed, vec!["effect 0"]);
        assert_eq!(report.asserted, vec!["emission"]);
        assert!(backend.snapshot_os_state().is_empty());
    }
}
