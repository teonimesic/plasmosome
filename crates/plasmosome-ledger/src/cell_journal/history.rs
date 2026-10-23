use std::collections::{BTreeMap, BTreeSet};

use plasmosome_backend::{CellId, Handle, PluginId};

use super::{
    CellJournalFaultKind as Kind, CellJournalRecord, JournalEvent, JournalRuleError, Prepare,
    RecordedEffect, Replacement,
};

/// The accepted history of one cell. Pass individually validated records to `check` or `admit`.
/// A refusal leaves the history unchanged; no operating-system authority is inferred here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellHistory {
    cell: CellId,
    last_generation: u64,
    open: Option<OpenTransaction>,
    committed: BTreeMap<PluginId, Attachment>,
    identities: BTreeMap<Handle, Identity>,
}

/// A prepare that has not finished, with its decision if one has been recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTransaction {
    pub generation: u64,
    pub prepare: Prepare,
    pub decision: Option<Decision>,
}

/// The mutually exclusive decision recorded after prepare and before finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Commit,
    Abort,
}

/// A committed replacement and the generation that published it, even before finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub generation: u64,
    pub replacement: Replacement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    plugin: PluginId,
    effect: RecordedEffect,
    state: State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Standing,
    Introduced,
    Retired,
}

impl CellHistory {
    /// Starts an empty history for `cell`, at generation zero.
    pub fn new(cell: CellId) -> Self {
        Self {
            cell,
            last_generation: 0,
            open: None,
            committed: BTreeMap::new(),
            identities: BTreeMap::new(),
        }
    }

    /// The cell supplied when this history was created.
    pub fn cell(&self) -> &CellId {
        &self.cell
    }

    /// The last prepared generation, including an aborted or unfinished generation.
    pub fn last_generation(&self) -> u64 {
        self.last_generation
    }

    /// The next generation number, or `None` after exhaustion. This does not authorize a prepare
    /// while another transaction is open.
    pub fn next_generation(&self) -> Option<u64> {
        self.last_generation.checked_add(1)
    }

    /// The unfinished transaction, including its decision if recorded.
    pub fn open_transaction(&self) -> Option<&OpenTransaction> {
        self.open.as_ref()
    }

    /// The replacements published by commit. Abort preserves this map; finish does not change it.
    pub fn committed(&self) -> &BTreeMap<PluginId, Attachment> {
        &self.committed
    }

    /// Every recorded address and its plugin, including retired addresses. Use only an entirely
    /// accepted history for cross-cell identity checks.
    pub fn addresses(&self) -> impl Iterator<Item = (&Handle, &PluginId)> {
        self.identities
            .iter()
            .map(|(address, identity)| (address, &identity.plugin))
    }

    /// Checks an individually validated record against this history without changing it.
    /// Returns the first generation, event-order or identity rule it breaks.
    pub fn check(&self, record: &CellJournalRecord) -> Result<(), JournalRuleError> {
        let refusal = match (&self.open, &record.event) {
            _ if record.generation == 0 => Some((Kind::GenerationZero, "generation zero")),
            (None, JournalEvent::Prepare(_))
                if self.next_generation() != Some(record.generation) =>
            {
                Some((Kind::GenerationOrder, "prepare is not the next generation"))
            }
            (None, JournalEvent::Prepare(_)) => None,
            (None, _) => Some((Kind::EventOrder, "terminal event without prepare")),
            (Some(_), JournalEvent::Prepare(_)) => {
                Some((Kind::EventOrder, "prepare before finish"))
            }
            (Some(open), _) if open.generation != record.generation => Some((
                Kind::GenerationOrder,
                "terminal event has another generation",
            )),
            (Some(open), JournalEvent::Finish) if open.decision.is_none() => {
                Some((Kind::EventOrder, "finish without a decision"))
            }
            (Some(open), JournalEvent::Commit | JournalEvent::Abort) if open.decision.is_some() => {
                Some((Kind::EventOrder, "decision already recorded"))
            }
            _ => None,
        };
        if let Some((kind, detail)) = refusal {
            return Err(JournalRuleError::new(kind, detail));
        }
        if let JournalEvent::Prepare(prepare) = &record.event {
            self.check_identities(prepare)?;
        }
        Ok(())
    }

    fn check_identities(&self, prepare: &Prepare) -> Result<(), JournalRuleError> {
        let invalid = |detail| JournalRuleError::new(Kind::InvalidEffect, detail);
        let mut seen = BTreeSet::new();
        for change in &prepare.changes {
            let Some(replacement) = &change.replacement else {
                if !self.committed.contains_key(&change.plugin) {
                    return Err(invalid("removal without a committed attachment"));
                }
                continue;
            };
            for effect in &replacement.effects {
                let Some(address) = effect.address() else {
                    continue;
                };
                if !seen.insert(address) {
                    return Err(invalid("address repeated in prepare"));
                }
                if let Some(identity) = self.identities.get(&address) {
                    if identity.effect != *effect {
                        return Err(invalid("address has a changed payload"));
                    }
                    if identity.plugin != change.plugin || identity.state != State::Standing {
                        return Err(invalid("address is retired or belongs to another holding"));
                    }
                }
            }
        }
        Ok(())
    }

    /// Admits an individually validated record. A refusal leaves every part of the history
    /// unchanged. This records desired history only; it performs no effects or cleanup.
    pub fn admit(&mut self, record: &CellJournalRecord) -> Result<(), JournalRuleError> {
        self.check(record)?;
        self.apply(record);
        Ok(())
    }

    fn apply(&mut self, record: &CellJournalRecord) {
        match &record.event {
            JournalEvent::Prepare(prepare) => {
                self.last_generation = record.generation;
                for change in &prepare.changes {
                    for effect in change.replacement.iter().flat_map(|r| &r.effects) {
                        if let Some(address) = effect.address() {
                            self.identities.entry(address).or_insert_with(|| Identity {
                                plugin: change.plugin.clone(),
                                effect: effect.clone(),
                                state: State::Introduced,
                            });
                        }
                    }
                }
                self.open = Some(OpenTransaction {
                    generation: record.generation,
                    prepare: prepare.clone(),
                    decision: None,
                });
            }
            JournalEvent::Commit => {
                let open = self.open.as_mut().expect("checked prepare");
                for change in &open.prepare.changes {
                    let retained: BTreeSet<_> = change
                        .replacement
                        .iter()
                        .flat_map(|r| &r.effects)
                        .filter_map(RecordedEffect::address)
                        .collect();
                    if let Some(old) = self.committed.get(&change.plugin) {
                        for address in old
                            .replacement
                            .effects
                            .iter()
                            .filter_map(RecordedEffect::address)
                        {
                            if !retained.contains(&address) {
                                self.identities
                                    .get_mut(&address)
                                    .expect("recorded identity")
                                    .state = State::Retired;
                            }
                        }
                    }
                    for address in retained {
                        self.identities
                            .get_mut(&address)
                            .expect("recorded identity")
                            .state = State::Standing;
                    }
                    match &change.replacement {
                        Some(replacement) => {
                            self.committed.insert(
                                change.plugin.clone(),
                                Attachment {
                                    generation: record.generation,
                                    replacement: replacement.clone(),
                                },
                            );
                        }
                        None => {
                            self.committed.remove(&change.plugin);
                        }
                    }
                }
                open.decision = Some(Decision::Commit);
            }
            JournalEvent::Abort => {
                for identity in self.identities.values_mut() {
                    if identity.state == State::Introduced {
                        identity.state = State::Retired;
                    }
                }
                self.open.as_mut().expect("checked prepare").decision = Some(Decision::Abort);
            }
            JournalEvent::Finish => self.open = None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_last_generation(cell: CellId, generation: u64) -> Self {
        Self {
            last_generation: generation,
            ..Self::new(cell)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell_journal::{AttachmentSource, Change};
    use crate::{Effect, InverseVia};
    use plasmosome_backend::{GrantId, MockMode, UniverseClass};

    fn prepare(generation: u64) -> CellJournalRecord {
        CellJournalRecord {
            generation,
            event: JournalEvent::Prepare(Prepare {
                changes: vec![Change {
                    plugin: PluginId::from("p"),
                    replacement: Some(Replacement {
                        mock: MockMode::Simulate,
                        source: AttachmentSource::Local,
                        effects: vec![],
                    }),
                }],
                force: None,
            }),
        }
    }

    #[test]
    fn aborted_introductions_are_retired() {
        let mut history = CellHistory::new(CellId::from("c"));
        let mut record = prepare(1);
        let address = Handle {
            class: UniverseClass::Mount,
            id: GrantId::new(),
        };
        let JournalEvent::Prepare(ref mut prepare) = record.event else {
            unreachable!()
        };
        prepare.changes[0]
            .replacement
            .as_mut()
            .unwrap()
            .effects
            .push(RecordedEffect {
                effect: Effect::exact("opaque", InverseVia::Backend(address)),
                operation: None,
            });
        history.admit(&record).unwrap();
        assert_eq!(history.identities[&address].state, State::Introduced);
        history
            .admit(&CellJournalRecord {
                generation: 1,
                event: JournalEvent::Abort,
            })
            .unwrap();
        assert_eq!(history.identities[&address].state, State::Retired);
    }

    #[test]
    fn prepare_at_max_is_legal_then_next_is_none() {
        let mut history = CellHistory::with_last_generation(CellId::from("c"), u64::MAX - 1);
        history.admit(&prepare(u64::MAX)).unwrap();
        for event in [JournalEvent::Commit, JournalEvent::Finish] {
            history
                .admit(&CellJournalRecord {
                    generation: u64::MAX,
                    event,
                })
                .unwrap();
        }
        assert_eq!(history.next_generation(), None);
        for generation in [1, u64::MAX] {
            assert_eq!(
                history.admit(&prepare(generation)).unwrap_err().kind,
                Kind::GenerationOrder
            );
        }
    }
}
