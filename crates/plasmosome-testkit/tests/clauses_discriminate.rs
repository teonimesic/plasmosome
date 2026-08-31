use std::collections::BTreeMap;

use plasmosome_backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, Grant, GrantKind, Handle, LedgerEntry,
    OsObject, OsState, PluginId, RevokePolicy, UniverseClass, UniverseOp, UniverseRemoval,
};
use plasmosome_testkit::conformance;

/// The one way a `DefectiveBackend` departs from the `EnforcementBackend`
/// contract. `None` departs in no way at all and must pass every clause, so a
/// clause that panics against any other variant panicked because of that
/// variant and not because the backend around it is sloppy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Defect {
    None,
    /// Revoke withdraws the right capability and hands back an entry that
    /// describes a different one.
    RevokeReturnsAStranger,
    /// Revoke of a handle no grant ever issued reports success.
    UnknownHandleReportsSuccess,
    /// Revoke forgets the handle and leaves the object it materialized.
    RevokeKeepsTheObject,
    /// Revoke empties the universe instead of removing its own object.
    RevokeTidiesTheWholeUniverse,
    /// Every grant materializes a second object nobody asked for.
    GrantMaterializesAShadow,
    /// Every grant is issued the same handle number.
    OneHandleForEveryLiveGrant,
    /// `apply_removal` reports success and removes nothing.
    RemovalIsANoOp,
    /// A handle stays revocable after it was revoked.
    ARevokedHandleRevokesAgain,
    /// A forced revoke reports success and withdraws nothing.
    ForcedRevokeIsALie,
    /// The ledger is keyed by capability class, so a second grant of a class
    /// displaces the first and either handle revokes whichever is left.
    ALedgerKeyedByClass,
    /// A revoked handle number goes back in the pool and is granted again.
    ARevokedHandleIsReissued,
    /// `apply_removal` takes every object of the removal's class, not the one
    /// the removal names.
    RemovalNukesTheClass,
    /// Every applied op lands under an owner nobody asked for.
    EveryOpLandsUnderAnImpostor,
    /// A forced revoke empties the universe instead of removing its own object.
    ForcedRevokeNukesUniverse,
    /// A forced revoke withdraws the right object and hands back an entry that
    /// describes a different one.
    ForcedRevokeReturnsStranger,
    /// A forced revoke of a handle no grant ever issued reports success.
    ForcedRevokeOfUnknownHandleOk,
    /// A forced revoke withdraws the object and leaves the handle revocable.
    ForcedRevokeKeepsHandleAlive,
    /// The ledger is keyed by handle honestly, and the withdrawal takes the last
    /// object of the revoked capability's class rather than the one that grant
    /// materialized.
    RevokeTakesLastOfClass,
    /// `apply_removal` takes every object of the removal's class for four of the
    /// five classes and removes exactly the named session file.
    ClassNukeSparingSessionFiles,
    /// Freed handle numbers go into a first-in first-out pool that is drawn from
    /// only once two numbers are waiting in it.
    HandleRecyclerDepthTwo,
    /// Answers `UnknownHandle` for any handle that still has an older live grant
    /// ahead of it, so a set of live grants revokes only in grant order.
    RevokesOnlyInGrantOrder,
    /// Answers `UnknownHandle` for any handle that still has a newer live grant
    /// behind it, so a set of live grants revokes only in reverse push order.
    RevokesOnlyInReversePushOrder,
    /// A refused revoke puts back every object its earlier revokes withdrew.
    ARefusedRevokeRestoresWhatItWithdrew,
    /// A refused revoke empties the universe, tearing down a ledger it has
    /// decided is past repairing.
    ARefusedRevokeClearsTheUniverse,
    /// Holds no universe of its own and answers every snapshot from its ledger,
    /// so what it reports is what it was asked to do. No clause can catch this
    /// one, and the test that runs it is what keeps that limit checkable.
    AMirrorOfItsOwnLedger,
}

/// A backend that keeps a ledger by handle and a universe of the objects its
/// grants materialized, carrying exactly one defect. Construct it with
/// `carrying`; hand `conformance` a closure that constructs a fresh one.
struct DefectiveBackend {
    defect: Defect,
    state: OsState,
    ledger: BTreeMap<u64, LedgerEntry>,
    spent: BTreeMap<u64, LedgerEntry>,
    class_of_handle: BTreeMap<u64, u64>,
    freed: Vec<u64>,
    applied: Vec<UniverseOp>,
    planted: Vec<OsObject>,
    next_handle: u64,
}

impl DefectiveBackend {
    fn carrying(defect: Defect) -> DefectiveBackend {
        DefectiveBackend {
            defect,
            state: OsState::new(),
            ledger: BTreeMap::new(),
            spent: BTreeMap::new(),
            class_of_handle: BTreeMap::new(),
            freed: Vec::new(),
            applied: Vec::new(),
            planted: Vec::new(),
            next_handle: 0,
        }
    }

    fn mirrors_its_ledger(&self) -> bool {
        self.defect == Defect::AMirrorOfItsOwnLedger
    }

    fn mirrored_state(&self) -> OsState {
        let mut mirror = OsState::new();
        for entry in self.ledger.values() {
            mirror.insert(object_of(entry));
        }
        for op in &self.applied {
            mirror.insert(op.object());
        }
        for object in &self.planted {
            mirror.insert(object.clone());
        }
        mirror
    }

    fn mint(&mut self) -> Handle {
        match self.defect {
            Defect::ARevokedHandleIsReissued => {
                if let Some(reissued) = self.freed.pop() {
                    return Handle(reissued);
                }
            }
            Defect::HandleRecyclerDepthTwo if self.freed.len() >= 2 => {
                return Handle(self.freed.remove(0));
            }
            _ => {}
        }
        self.next_handle += 1;
        Handle(self.next_handle)
    }

    fn refuses_out_of_grant_order(&self, handle: Handle) -> bool {
        self.defect == Defect::RevokesOnlyInGrantOrder
            && self
                .ledger
                .keys()
                .next()
                .is_some_and(|oldest| *oldest < handle.raw())
    }

    fn refuses_out_of_reverse_push_order(&self, handle: Handle) -> bool {
        self.defect == Defect::RevokesOnlyInReversePushOrder
            && self
                .ledger
                .keys()
                .next_back()
                .is_some_and(|newest| *newest > handle.raw())
    }

    fn restore_everything_withdrawn(&mut self) {
        let withdrawn: Vec<OsObject> = self.spent.values().map(object_of).collect();
        for object in withdrawn {
            self.state.insert(object);
        }
    }

    fn ledger_key(&self, handle: Handle, capability: &Capability) -> u64 {
        match self.defect {
            Defect::ALedgerKeyedByClass => class_index(capability),
            _ => handle.raw(),
        }
    }

    fn withdraw(&mut self, entry: &LedgerEntry, policy: RevokePolicy) -> Result<(), BackendError> {
        match self.defect {
            Defect::AMirrorOfItsOwnLedger => Ok(()),
            Defect::RevokeKeepsTheObject => Ok(()),
            Defect::RevokeTidiesTheWholeUniverse => {
                self.state = OsState::new();
                Ok(())
            }
            Defect::ForcedRevokeNukesUniverse if policy == RevokePolicy::Force => {
                self.state = OsState::new();
                Ok(())
            }
            Defect::ForcedRevokeKeepsHandleAlive if policy == RevokePolicy::Force => {
                let _ = self.apply_removal(removal_of(&entry.capability), &entry.plugin);
                Ok(())
            }
            Defect::RevokeTakesLastOfClass => {
                self.take_the_last_of_class(&entry.capability, &entry.plugin)
            }
            _ => self.apply_removal(removal_of(&entry.capability), &entry.plugin),
        }
    }

    fn take_the_last_of_class(
        &mut self,
        capability: &Capability,
        owner: &PluginId,
    ) -> Result<(), BackendError> {
        let removal = removal_of(capability);
        let class = removal.class();
        let last = self
            .state
            .objects()
            .filter(|held| held.class == class)
            .map(|held| (held.key.clone(), held.owner.clone()))
            .last();
        match last {
            Some((key, held_by)) => {
                self.state.remove(class, &key, &held_by);
                Ok(())
            }
            None => Err(BackendError::UnknownObject {
                class: class.as_str(),
                key: removal.key(),
                owner: owner.clone(),
            }),
        }
    }
}

impl EnforcementBackend for DefectiveBackend {
    fn grant(&mut self, grant: Grant) -> LedgerEntry {
        let handle = self.mint();
        let entry = LedgerEntry {
            handle,
            plugin: grant.plugin,
            capability: grant.capability,
            kind: grant.kind,
        };
        if !self.mirrors_its_ledger() {
            self.state.insert(object_of(&entry));
        }
        if self.defect == Defect::GrantMaterializesAShadow {
            self.state.insert(shadow_of(&entry));
        }
        let key = self.ledger_key(handle, &entry.capability);
        self.class_of_handle.insert(handle.raw(), key);
        self.ledger.insert(key, entry.clone());
        if self.defect == Defect::OneHandleForEveryLiveGrant {
            return LedgerEntry {
                handle: Handle(1),
                ..entry
            };
        }
        entry
    }

    fn revoke(&mut self, handle: Handle, drain: DrainSpec) -> Result<LedgerEntry, BackendError> {
        let forced = drain.policy == RevokePolicy::Force;
        if self.refuses_out_of_grant_order(handle) || self.refuses_out_of_reverse_push_order(handle)
        {
            return Err(BackendError::UnknownHandle { handle });
        }
        let key = match self.defect {
            Defect::ALedgerKeyedByClass => match self.class_of_handle.get(&handle.raw()) {
                Some(class) => *class,
                None => return Err(BackendError::UnknownHandle { handle }),
            },
            _ => handle.raw(),
        };
        let Some(entry) = self.ledger.remove(&key) else {
            return match self.defect {
                Defect::UnknownHandleReportsSuccess => Ok(a_stranger(handle)),
                Defect::ForcedRevokeOfUnknownHandleOk if forced => Ok(a_stranger(handle)),
                Defect::ARevokedHandleRevokesAgain => self
                    .spent
                    .get(&handle.raw())
                    .cloned()
                    .ok_or(BackendError::UnknownHandle { handle }),
                Defect::ARefusedRevokeRestoresWhatItWithdrew => {
                    self.restore_everything_withdrawn();
                    Err(BackendError::UnknownHandle { handle })
                }
                Defect::ARefusedRevokeClearsTheUniverse => {
                    self.state = OsState::new();
                    Err(BackendError::UnknownHandle { handle })
                }
                _ => Err(BackendError::UnknownHandle { handle }),
            };
        };
        if self.defect != Defect::ForcedRevokeIsALie || !forced {
            self.withdraw(&entry, drain.policy)?;
        }
        if self.defect == Defect::ForcedRevokeKeepsHandleAlive && forced {
            self.ledger.insert(key, entry.clone());
        } else {
            self.freed.push(handle.raw());
            self.spent.insert(handle.raw(), entry.clone());
        }
        if self.defect == Defect::RevokeReturnsAStranger
            || (self.defect == Defect::ForcedRevokeReturnsStranger && forced)
        {
            return Ok(a_stranger(handle));
        }
        Ok(entry)
    }

    fn snapshot_os_state(&self) -> OsState {
        if self.mirrors_its_ledger() {
            return self.mirrored_state();
        }
        self.state.clone()
    }

    fn apply(&mut self, op: UniverseOp) -> Result<(), BackendError> {
        if self.mirrors_its_ledger() {
            self.applied.push(op);
            return Ok(());
        }
        let mut object = op.object();
        if self.defect == Defect::EveryOpLandsUnderAnImpostor {
            object.owner = PluginId::from("impostor");
        }
        self.state.insert(object);
        Ok(())
    }

    fn apply_removal(
        &mut self,
        removal: UniverseRemoval,
        owner: &PluginId,
    ) -> Result<(), BackendError> {
        if self.defect == Defect::RemovalIsANoOp {
            return Ok(());
        }
        let (class, key) = (removal.class(), removal.key());
        if self.mirrors_its_ledger() {
            let recorded = self
                .applied
                .iter()
                .position(|op| op.object().class == class && op.object().key == key);
            return match recorded {
                Some(index) => {
                    self.applied.remove(index);
                    Ok(())
                }
                None => Err(BackendError::UnknownObject {
                    class: class.as_str(),
                    key,
                    owner: owner.clone(),
                }),
            };
        }
        let nukes_the_class = match self.defect {
            Defect::RemovalNukesTheClass => true,
            Defect::ClassNukeSparingSessionFiles => class != UniverseClass::SessionFile,
            _ => false,
        };
        if nukes_the_class {
            let doomed: Vec<(String, PluginId)> = self
                .state
                .objects()
                .filter(|held| held.class == class)
                .map(|held| (held.key.clone(), held.owner.clone()))
                .collect();
            let struck = doomed.len();
            for (doomed_key, held_by) in doomed {
                self.state.remove(class, &doomed_key, &held_by);
            }
            return if struck == 0 {
                Err(BackendError::UnknownObject {
                    class: class.as_str(),
                    key,
                    owner: owner.clone(),
                })
            } else {
                Ok(())
            };
        }
        self.state
            .remove(class, &key, owner)
            .map(|_| ())
            .ok_or(BackendError::UnknownObject {
                class: class.as_str(),
                key,
                owner: owner.clone(),
            })
    }

    fn plant(&mut self, object: OsObject) {
        if self.mirrors_its_ledger() {
            self.planted.push(object);
            return;
        }
        self.state.insert(object);
    }
}

fn object_of(entry: &LedgerEntry) -> OsObject {
    let owner = entry.plugin.clone();
    let op = match &entry.capability {
        Capability::SessionFile { path } => UniverseOp::WriteSessionFile {
            path: path.clone(),
            owner,
        },
        Capability::UdsSocket { path } => UniverseOp::BindUds {
            path: path.clone(),
            owner,
        },
        Capability::ProxyMap { host, route } => UniverseOp::SetProxyMap {
            host: host.clone(),
            route: route.clone(),
            owner,
        },
        Capability::Broker { pid, name } => UniverseOp::SpawnBroker {
            pid: *pid,
            name: name.clone(),
            owner,
        },
        Capability::Mount { source, target } => UniverseOp::AddMount {
            source: source.clone(),
            target: target.clone(),
            owner,
        },
    };
    op.object()
}

fn removal_of(capability: &Capability) -> UniverseRemoval {
    match capability {
        Capability::SessionFile { path } => {
            UniverseRemoval::RemoveSessionFile { path: path.clone() }
        }
        Capability::UdsSocket { path } => UniverseRemoval::UnbindUds { path: path.clone() },
        Capability::ProxyMap { host, .. } => UniverseRemoval::RemoveProxyMap { host: host.clone() },
        Capability::Broker { pid, .. } => UniverseRemoval::KillBroker { pid: *pid },
        Capability::Mount { target, .. } => UniverseRemoval::RemoveMount {
            target: target.clone(),
        },
    }
}

fn class_index(capability: &Capability) -> u64 {
    match capability {
        Capability::SessionFile { .. } => 0,
        Capability::UdsSocket { .. } => 1,
        Capability::ProxyMap { .. } => 2,
        Capability::Broker { .. } => 3,
        Capability::Mount { .. } => 4,
    }
}

fn shadow_of(entry: &LedgerEntry) -> OsObject {
    OsObject {
        class: UniverseClass::SessionFile,
        key: format!("session/shadow/{}", entry.handle.raw()),
        owner: entry.plugin.clone(),
    }
}

fn a_stranger(handle: Handle) -> LedgerEntry {
    LedgerEntry {
        handle,
        plugin: PluginId::from("stranger"),
        capability: Capability::SessionFile {
            path: "stranger.md".to_string(),
        },
        kind: GrantKind::Hot,
    }
}

fn carrying(defect: Defect) -> impl Fn() -> DefectiveBackend {
    move || DefectiveBackend::carrying(defect)
}

#[test]
fn a_backend_with_no_defect_passes_every_clause() {
    conformance::grant_is_replayable(carrying(Defect::None));
    conformance::revoke_unknown_handle_is_error(carrying(Defect::None));
    conformance::drained_revoke_removes_object(carrying(Defect::None));
    conformance::planted_residue_survives_unrelated_revoke(carrying(Defect::None));
    conformance::snapshot_never_invents_objects(carrying(Defect::None));
    conformance::live_grants_hold_distinct_handles(carrying(Defect::None));
    conformance::apply_and_removal_reach_the_universe(carrying(Defect::None));
    conformance::revoke_of_a_revoked_handle_is_error(carrying(Defect::None));
}

/// Pins the limit of this seam: a backend answering every snapshot from its own
/// ledger passes all eight clauses, so no clause can separate enforcing from
/// reporting. If this test ever fails, the seam grew a real oracle — delete this
/// test and the README paragraph it belongs to. Never weaken the clause that
/// caught it.
#[test]
fn snapshot_os_state_is_the_only_oracle_a_clause_has() {
    conformance::grant_is_replayable(carrying(Defect::AMirrorOfItsOwnLedger));
    conformance::revoke_unknown_handle_is_error(carrying(Defect::AMirrorOfItsOwnLedger));
    conformance::drained_revoke_removes_object(carrying(Defect::AMirrorOfItsOwnLedger));
    conformance::planted_residue_survives_unrelated_revoke(carrying(Defect::AMirrorOfItsOwnLedger));
    conformance::snapshot_never_invents_objects(carrying(Defect::AMirrorOfItsOwnLedger));
    conformance::live_grants_hold_distinct_handles(carrying(Defect::AMirrorOfItsOwnLedger));
    conformance::apply_and_removal_reach_the_universe(carrying(Defect::AMirrorOfItsOwnLedger));
    conformance::revoke_of_a_revoked_handle_is_error(carrying(Defect::AMirrorOfItsOwnLedger));
}

#[test]
#[should_panic(expected = "revoking a handle must return the entry the grant issued")]
fn grant_is_replayable_catches_a_revoke_that_returns_a_stranger() {
    conformance::grant_is_replayable(carrying(Defect::RevokeReturnsAStranger));
}

#[test]
#[should_panic(expected = "revoking the never-granted handle")]
fn revoke_unknown_handle_is_error_catches_a_success_report() {
    conformance::revoke_unknown_handle_is_error(carrying(Defect::UnknownHandleReportsSuccess));
}

#[test]
#[should_panic(expected = "a drained revoke left")]
fn drained_revoke_removes_object_catches_a_revoke_that_keeps_the_object() {
    conformance::drained_revoke_removes_object(carrying(Defect::RevokeKeepsTheObject));
}

#[test]
#[should_panic(expected = "removed the unrelated")]
fn planted_residue_survives_unrelated_revoke_catches_a_revoke_that_tidies_the_universe() {
    conformance::planted_residue_survives_unrelated_revoke(carrying(
        Defect::RevokeTidiesTheWholeUniverse,
    ));
}

#[test]
#[should_panic(expected = "which was never granted or planted")]
fn snapshot_never_invents_objects_catches_a_grant_that_materializes_a_shadow() {
    conformance::snapshot_never_invents_objects(carrying(Defect::GrantMaterializesAShadow));
}

#[test]
#[should_panic(expected = "is already holding")]
fn live_grants_hold_distinct_handles_catches_one_handle_issued_twice() {
    conformance::live_grants_hold_distinct_handles(carrying(Defect::OneHandleForEveryLiveGrant));
}

#[test]
#[should_panic(expected = "an applied removal left")]
fn apply_and_removal_reach_the_universe_catches_a_removal_that_removes_nothing() {
    conformance::apply_and_removal_reach_the_universe(carrying(Defect::RemovalIsANoOp));
}

#[test]
#[should_panic(expected = "revoking the already-revoked handle")]
fn revoke_of_a_revoked_handle_is_error_catches_a_handle_that_revokes_twice() {
    conformance::revoke_of_a_revoked_handle_is_error(carrying(Defect::ARevokedHandleRevokesAgain));
}

#[test]
#[should_panic(expected = "a forced revoke left")]
fn drained_revoke_removes_object_catches_a_forced_revoke_that_withdraws_nothing() {
    conformance::drained_revoke_removes_object(carrying(Defect::ForcedRevokeIsALie));
}

#[test]
#[should_panic(expected = "did not revoke through h5 on the reverse-push-order pass")]
fn live_grants_hold_distinct_handles_catches_a_ledger_keyed_by_class() {
    conformance::live_grants_hold_distinct_handles(carrying(Defect::ALedgerKeyedByClass));
}

#[test]
#[should_panic(expected = "revoking the already-revoked handle")]
fn revoke_of_a_revoked_handle_is_error_catches_a_reissued_handle_number() {
    conformance::revoke_of_a_revoked_handle_is_error(carrying(Defect::ARevokedHandleIsReissued));
}

#[test]
#[should_panic(expected = "also took the unrelated")]
fn apply_and_removal_reach_the_universe_catches_a_removal_that_takes_the_whole_class() {
    conformance::apply_and_removal_reach_the_universe(carrying(Defect::RemovalNukesTheClass));
}

#[test]
#[should_panic(expected = "an applied op must materialize")]
fn apply_and_removal_reach_the_universe_catches_an_op_applied_under_an_impostor() {
    conformance::apply_and_removal_reach_the_universe(carrying(
        Defect::EveryOpLandsUnderAnImpostor,
    ));
}

#[test]
#[should_panic(expected = "removed the unrelated")]
fn planted_residue_survives_unrelated_revoke_catches_a_forced_revoke_that_nukes_the_universe() {
    conformance::planted_residue_survives_unrelated_revoke(carrying(
        Defect::ForcedRevokeNukesUniverse,
    ));
}

#[test]
#[should_panic(expected = "revoking a handle must return the entry the grant issued")]
fn grant_is_replayable_catches_a_forced_revoke_that_returns_a_stranger() {
    conformance::grant_is_replayable(carrying(Defect::ForcedRevokeReturnsStranger));
}

#[test]
#[should_panic(expected = "revoking the never-granted handle")]
fn revoke_unknown_handle_is_error_catches_a_forced_success_report() {
    conformance::revoke_unknown_handle_is_error(carrying(Defect::ForcedRevokeOfUnknownHandleOk));
}

#[test]
#[should_panic(expected = "revoking the already-revoked handle")]
fn revoke_of_a_revoked_handle_is_error_catches_a_forced_revoke_that_keeps_the_handle_alive() {
    conformance::revoke_of_a_revoked_handle_is_error(carrying(
        Defect::ForcedRevokeKeepsHandleAlive,
    ));
}

#[test]
#[should_panic(expected = "must withdraw the object its own grant materialized")]
fn live_grants_hold_distinct_handles_catches_a_revoke_that_takes_another_object_of_its_class() {
    conformance::live_grants_hold_distinct_handles(carrying(Defect::RevokeTakesLastOfClass));
}

#[test]
#[should_panic(expected = "also took the unrelated")]
fn apply_and_removal_reach_the_universe_catches_a_class_nuke_that_spares_session_files() {
    conformance::apply_and_removal_reach_the_universe(carrying(
        Defect::ClassNukeSparingSessionFiles,
    ));
}

#[test]
#[should_panic(expected = "revoking the already-revoked handle")]
fn revoke_of_a_revoked_handle_is_error_catches_a_free_list_that_recycles_at_depth_two() {
    conformance::revoke_of_a_revoked_handle_is_error(carrying(Defect::HandleRecyclerDepthTwo));
}

#[test]
#[should_panic(expected = "did not revoke through h6 on the reverse-push-order pass")]
fn live_grants_hold_distinct_handles_catches_a_backend_that_only_revokes_in_grant_order() {
    conformance::live_grants_hold_distinct_handles(carrying(Defect::RevokesOnlyInGrantOrder));
}

#[test]
#[should_panic(expected = "of the already-revoked h2 must leave")]
fn revoke_of_a_revoked_handle_is_error_catches_a_refused_revoke_that_restores_the_object() {
    conformance::revoke_of_a_revoked_handle_is_error(carrying(
        Defect::ARefusedRevokeRestoresWhatItWithdrew,
    ));
}

#[test]
#[should_panic(expected = "from the live grant holding")]
fn revoke_of_a_revoked_handle_is_error_catches_a_refused_revoke_that_clears_the_universe() {
    conformance::revoke_of_a_revoked_handle_is_error(carrying(
        Defect::ARefusedRevokeClearsTheUniverse,
    ));
}

#[test]
#[should_panic(expected = "did not revoke through h1 on the grant-order pass")]
fn live_grants_hold_distinct_handles_catches_a_backend_that_only_revokes_in_reverse_order() {
    conformance::live_grants_hold_distinct_handles(carrying(Defect::RevokesOnlyInReversePushOrder));
}

#[test]
#[should_panic(expected = "must empty the universe")]
fn live_grants_hold_distinct_handles_catches_a_grant_that_materializes_a_shadow() {
    conformance::live_grants_hold_distinct_handles(carrying(Defect::GrantMaterializesAShadow));
}
