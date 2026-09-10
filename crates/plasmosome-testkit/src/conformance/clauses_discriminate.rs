use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use super as conformance;
use plasmosome_backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, Grant, GrantId, GrantKind, Handle,
    LedgerEntry, OsObject, OsState, PluginId, RevokePolicy, UniverseClass, UniverseOp,
    UniverseRemoval,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Defect {
    None,
    RevokeReturnsAStranger,
    UnknownHandleReportsSuccess,
    RevokeKeepsTheObject,
    RevokeTidiesTheWholeUniverse,
    GrantMaterializesAShadow,
    OneHandleForEveryLiveGrant,
    RemovalIsANoOp,
    ARevokedHandleRevokesAgain,
    ForcedRevokeIsALie,
    ALedgerKeyedByClass,
    ARevokedHandleIsReissued,
    RevokeTakesAnotherResourceOfClass,
    RevokesOnlyInGrantOrder,
    RevokesOnlyInReversePushOrder,
    AMirrorOfItsOwnLedger,
    RevokeTakesWrongOwner,
    IdenticalGrantsCollapse,
    RevokeTakesWrongInstance,
    AppliedRemovalsOnlyInGrantOrder,
    AppliedRemovalsOnlyInReverseOrder,
    ApplyRemovalDeletesOtherClasses,
    GrantSubstitutesLiveOwner,
    GrantSubstitutesMountSource,
    GrantSubstitutesProxyRoute,
    GrantSubstitutesBrokerName,
    GrantPanics,
    SnapshotPanicsLikeAssertion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InfrastructureFailure {
    Factory,
    Grant,
}

const ASSERTION_SHAPED_INFRASTRUCTURE_PANIC: &str = "assertion failed: infrastructure sentinel";

struct DefectiveBackend {
    defect: Defect,
    state: OsState,
    ledger: BTreeMap<Handle, LedgerEntry>,
    spent: BTreeMap<Handle, LedgerEntry>,
    freed: Vec<Handle>,
    grant_order: Vec<Handle>,
    applied: Vec<UniverseOp>,
    planted: Vec<OsObject>,
    first_returned: Option<Handle>,
}

impl DefectiveBackend {
    fn carrying(defect: Defect) -> DefectiveBackend {
        DefectiveBackend {
            defect,
            state: OsState::new(),
            ledger: BTreeMap::new(),
            spent: BTreeMap::new(),
            freed: Vec::new(),
            grant_order: Vec::new(),
            applied: Vec::new(),
            planted: Vec::new(),
            first_returned: None,
        }
    }

    fn mirrors_its_ledger(&self) -> bool {
        self.defect == Defect::AMirrorOfItsOwnLedger
    }

    fn mint(&mut self, class: UniverseClass) -> Handle {
        if self.defect == Defect::ARevokedHandleIsReissued
            && let Some(handle) = self.freed.pop()
        {
            return handle;
        }
        Handle {
            class,
            id: GrantId::new(),
        }
    }

    fn mirrored_state(&self) -> OsState {
        let mut state = OsState::new();
        for entry in self.ledger.values() {
            state.insert(entry.object()).unwrap();
        }
        for op in &self.applied {
            state.insert(op.object()).unwrap();
        }
        for object in &self.planted {
            state.insert(object.clone()).unwrap();
        }
        state
    }

    fn refuses_grant_order(&self, handle: Handle) -> bool {
        self.defect == Defect::RevokesOnlyInGrantOrder
            && self
                .grant_order
                .first()
                .is_some_and(|first| *first != handle)
    }

    fn refuses_reverse_order(&self, handle: Handle) -> bool {
        self.defect == Defect::RevokesOnlyInReversePushOrder
            && self.grant_order.last().is_some_and(|last| *last != handle)
    }

    fn withdraw(&mut self, entry: &LedgerEntry, policy: RevokePolicy) -> Result<(), BackendError> {
        match self.defect {
            Defect::AMirrorOfItsOwnLedger | Defect::RevokeKeepsTheObject => Ok(()),
            Defect::RevokeTidiesTheWholeUniverse => {
                self.state = OsState::new();
                Ok(())
            }
            Defect::ForcedRevokeIsALie if policy == RevokePolicy::Force => Ok(()),
            Defect::RevokeTakesAnotherResourceOfClass => self.take_another_resource_of_class(entry),
            Defect::RevokeTakesWrongOwner => self.take_wrong_owner(entry),
            Defect::RevokeTakesWrongInstance => self.take_wrong_instance(entry),
            _ => self.apply_removal(entry.removal(), &entry.plugin),
        }
    }

    fn take_another_resource_of_class(&mut self, entry: &LedgerEntry) -> Result<(), BackendError> {
        let object = self
            .state
            .objects()
            .find(|held| {
                held.class() == entry.capability.class() && held.capability != entry.capability
            })
            .cloned();
        self.remove_selected(object.or_else(|| Some(entry.object())), entry)
    }

    fn take_wrong_owner(&mut self, entry: &LedgerEntry) -> Result<(), BackendError> {
        let object = self
            .state
            .objects()
            .find(|held| held.capability == entry.capability && held.owner != entry.plugin)
            .cloned();
        self.remove_selected(object.or_else(|| Some(entry.object())), entry)
    }

    fn take_wrong_instance(&mut self, entry: &LedgerEntry) -> Result<(), BackendError> {
        let object = self
            .state
            .objects()
            .find(|held| {
                held.capability == entry.capability
                    && held.owner == entry.plugin
                    && held.id != entry.handle.id
            })
            .cloned();
        self.remove_selected(object.or_else(|| Some(entry.object())), entry)
    }

    fn remove_selected(
        &mut self,
        object: Option<OsObject>,
        entry: &LedgerEntry,
    ) -> Result<(), BackendError> {
        let Some(object) = object else {
            return Err(unknown_object(&entry.removal(), &entry.plugin));
        };
        self.state
            .remove(&removal_of(&object), &object.owner)
            .map(|_| ())
            .ok_or_else(|| unknown_object(&entry.removal(), &entry.plugin))
    }

    fn applied_order_refuses(&self, removal: &UniverseRemoval) -> bool {
        let Some(index) = self.applied.iter().position(|op| op.id() == removal.id) else {
            return false;
        };
        match self.defect {
            Defect::AppliedRemovalsOnlyInGrantOrder => index != 0,
            Defect::AppliedRemovalsOnlyInReverseOrder => index + 1 != self.applied.len(),
            _ => false,
        }
    }
}

impl EnforcementBackend for DefectiveBackend {
    fn grant(&mut self, mut grant: Grant) -> LedgerEntry {
        if self.defect == Defect::GrantPanics {
            std::panic::panic_any(InfrastructureFailure::Grant);
        }
        if self.defect == Defect::GrantSubstitutesLiveOwner
            && let Some(owner) = self
                .ledger
                .values()
                .find(|entry| entry.capability == grant.capability)
                .map(|entry| entry.plugin.clone())
        {
            grant.plugin = owner;
        }
        let substitutes_colliding_capability = matches!(
            (self.defect, grant.capability.class()),
            (Defect::GrantSubstitutesMountSource, UniverseClass::Mount)
                | (Defect::GrantSubstitutesProxyRoute, UniverseClass::ProxyMap)
                | (Defect::GrantSubstitutesBrokerName, UniverseClass::BrokerPid)
        );
        if substitutes_colliding_capability
            && let Some(capability) = self
                .ledger
                .values()
                .find(|entry| {
                    entry.capability.class() == grant.capability.class()
                        && entry.capability.key() == grant.capability.key()
                        && entry.capability != grant.capability
                })
                .map(|entry| entry.capability.clone())
        {
            grant.capability = capability;
        }
        let handle = self.mint(grant.capability.class());
        let entry = LedgerEntry {
            handle,
            plugin: grant.plugin,
            capability: grant.capability,
            kind: grant.kind,
        };
        if !self.mirrors_its_ledger() {
            let duplicate = self.defect == Defect::IdenticalGrantsCollapse
                && self
                    .state
                    .objects()
                    .any(|held| held.owner == entry.plugin && held.capability == entry.capability);
            if !duplicate {
                self.state.insert(entry.object()).unwrap();
            }
            if self.defect == Defect::GrantMaterializesAShadow {
                self.state.insert(shadow_of(&entry)).unwrap();
            }
        }
        if self.defect == Defect::ALedgerKeyedByClass
            && let Some(previous) = self
                .ledger
                .keys()
                .find(|held| held.class == entry.handle.class)
                .copied()
        {
            self.ledger.remove(&previous);
            self.grant_order.retain(|held| *held != previous);
        }
        self.ledger.insert(handle, entry.clone());
        self.grant_order.push(handle);
        if self.defect == Defect::OneHandleForEveryLiveGrant {
            let duplicate = self.first_returned.get_or_insert(handle);
            LedgerEntry {
                handle: *duplicate,
                ..entry
            }
        } else {
            entry
        }
    }

    fn revoke(&mut self, handle: Handle, drain: DrainSpec) -> Result<LedgerEntry, BackendError> {
        if self.refuses_grant_order(handle) || self.refuses_reverse_order(handle) {
            return Err(BackendError::UnknownHandle { handle });
        }
        let Some(entry) = self.ledger.get(&handle).cloned() else {
            return match self.defect {
                Defect::UnknownHandleReportsSuccess => Ok(a_stranger(handle)),
                Defect::ARevokedHandleRevokesAgain => self
                    .spent
                    .get(&handle)
                    .cloned()
                    .ok_or(BackendError::UnknownHandle { handle }),
                _ => Err(BackendError::UnknownHandle { handle }),
            };
        };
        self.withdraw(&entry, drain.policy)?;
        self.ledger.remove(&handle);
        self.grant_order.retain(|held| *held != handle);
        self.freed.push(handle);
        self.spent.insert(handle, entry.clone());
        if self.defect == Defect::RevokeReturnsAStranger {
            return Ok(a_stranger(handle));
        }
        Ok(entry)
    }

    fn snapshot_os_state(&self) -> OsState {
        if self.defect == Defect::SnapshotPanicsLikeAssertion {
            std::panic::panic_any(ASSERTION_SHAPED_INFRASTRUCTURE_PANIC);
        }
        if self.mirrors_its_ledger() {
            self.mirrored_state()
        } else {
            self.state.clone()
        }
    }

    fn apply(&mut self, op: UniverseOp) -> Result<(), BackendError> {
        if self.mirrors_its_ledger() {
            self.applied.push(op);
            return Ok(());
        }
        let object = op.object();
        if self.defect == Defect::IdenticalGrantsCollapse
            && self
                .state
                .objects()
                .any(|held| held.owner == object.owner && held.capability == object.capability)
        {
            self.applied.push(op);
            return Ok(());
        }
        self.state.insert(object)?;
        self.applied.push(op);
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
        if self.applied_order_refuses(&removal) {
            return Err(unknown_object(&removal, owner));
        }
        if self.mirrors_its_ledger() {
            if remove_recorded(&mut self.applied, &removal, owner)
                || remove_planted(&mut self.planted, &removal, owner)
            {
                self.ledger.remove(&Handle {
                    class: removal.class(),
                    id: removal.id,
                });
                return Ok(());
            }
            return Err(unknown_object(&removal, owner));
        }
        let removed_class = removal.class();
        if self.state.remove(&removal, owner).is_none() {
            return Err(unknown_object(&removal, owner));
        }
        self.applied.retain(|op| op.id() != removal.id);
        self.ledger.remove(&Handle {
            class: removed_class,
            id: removal.id,
        });
        if self.defect == Defect::ApplyRemovalDeletesOtherClasses {
            let survivors: Vec<OsObject> = self
                .state
                .objects()
                .filter(|object| object.class() == removed_class)
                .cloned()
                .collect();
            let mut damaged = OsState::new();
            for object in survivors {
                damaged.insert(object).unwrap();
            }
            self.state = damaged;
        }
        Ok(())
    }

    fn plant(&mut self, object: OsObject) -> Result<(), BackendError> {
        if self.mirrors_its_ledger() {
            self.planted.push(object);
            return Ok(());
        }
        self.state.insert(object).map(|_| ())
    }
}

fn removal_of(object: &OsObject) -> UniverseRemoval {
    UniverseRemoval {
        id: object.id,
        capability: object.capability.clone(),
    }
}

fn unknown_object(removal: &UniverseRemoval, owner: &PluginId) -> BackendError {
    BackendError::UnknownObject {
        class: removal.class().as_str(),
        key: removal.key(),
        owner: owner.clone(),
        id: removal.id,
    }
}

fn remove_recorded(
    applied: &mut Vec<UniverseOp>,
    removal: &UniverseRemoval,
    owner: &PluginId,
) -> bool {
    let position = applied.iter().position(|op| {
        let object = op.object();
        object.id == removal.id && object.owner == *owner && object.capability == removal.capability
    });
    position.is_some_and(|index| {
        applied.remove(index);
        true
    })
}

fn remove_planted(
    planted: &mut Vec<OsObject>,
    removal: &UniverseRemoval,
    owner: &PluginId,
) -> bool {
    let position = planted.iter().position(|object| {
        object.id == removal.id && object.owner == *owner && object.capability == removal.capability
    });
    position.is_some_and(|index| {
        planted.remove(index);
        true
    })
}

fn shadow_of(entry: &LedgerEntry) -> OsObject {
    OsObject {
        id: GrantId::new(),
        owner: entry.plugin.clone(),
        capability: Capability::SessionFile {
            path: format!("shadow/{}", entry.handle.id),
        },
    }
}

fn a_stranger(handle: Handle) -> LedgerEntry {
    let capability = match handle.class {
        UniverseClass::SessionFile => Capability::SessionFile {
            path: "stranger.md".to_string(),
        },
        UniverseClass::UdsPath => Capability::UdsSocket {
            path: "/stranger.uds".to_string(),
        },
        UniverseClass::ProxyMap => Capability::ProxyMap {
            host: "stranger.test".to_string(),
            route: "stranger".to_string(),
        },
        UniverseClass::BrokerPid => Capability::Broker {
            pid: 999,
            name: "stranger".to_string(),
        },
        UniverseClass::Mount => Capability::Mount {
            source: "/stranger".to_string(),
            target: "/stranger".to_string(),
        },
    };
    LedgerEntry {
        handle,
        plugin: PluginId::from("stranger"),
        capability,
        kind: GrantKind::Hot,
    }
}

fn carrying(defect: Defect) -> impl Fn() -> DefectiveBackend {
    move || DefectiveBackend::carrying(defect)
}

fn factory_panics() -> DefectiveBackend {
    std::panic::panic_any(InfrastructureFailure::Factory)
}

fn assert_rejected(run: impl FnOnce()) {
    conformance::take_contract_failure();
    let result = catch_unwind(AssertUnwindSafe(run));
    let contract_failed = conformance::take_contract_failure();
    match (result, contract_failed) {
        (Err(_), true) => {}
        (Err(payload), false) => resume_unwind(payload),
        (Ok(()), false) => panic!("the defective backend passed the clause that should reject it"),
        (Ok(()), true) => panic!("a conformance failure was recorded without a panic"),
    }
}

fn run_all(defect: Defect) {
    conformance::grant_is_replayable(carrying(defect));
    conformance::revoke_unknown_handle_is_error(carrying(defect));
    conformance::drained_revoke_removes_object(carrying(defect));
    conformance::planted_residue_survives_unrelated_revoke(carrying(defect));
    conformance::snapshot_never_invents_objects(carrying(defect));
    conformance::live_grants_hold_distinct_handles(carrying(defect));
    conformance::apply_and_removal_reach_the_universe(carrying(defect));
    conformance::revoke_of_a_revoked_handle_is_error(carrying(defect));
    conformance::revoke_takes_its_owners_object(carrying(defect));
    conformance::repeated_grants_are_independently_removable(carrying(defect));
}

#[test]
fn defect_free_backend_passes_every_clause() {
    run_all(Defect::None);
}

#[test]
fn mirror_oracle_passes_every_clause_without_proving_os_enforcement() {
    run_all(Defect::AMirrorOfItsOwnLedger);
}

#[test]
fn infrastructure_and_fixture_panics_cannot_satisfy_clause_witnesses() {
    let factory = catch_unwind(AssertUnwindSafe(|| {
        assert_rejected(|| conformance::snapshot_never_invents_objects(factory_panics))
    }))
    .expect_err("a factory panic must escape the clause witness");
    assert_eq!(
        factory.downcast_ref::<InfrastructureFailure>(),
        Some(&InfrastructureFailure::Factory)
    );

    let grant = catch_unwind(AssertUnwindSafe(|| {
        assert_rejected(|| conformance::grant_is_replayable(carrying(Defect::GrantPanics)))
    }))
    .expect_err("a backend panic must escape the clause witness");
    assert_eq!(
        grant.downcast_ref::<InfrastructureFailure>(),
        Some(&InfrastructureFailure::Grant)
    );

    let snapshot = catch_unwind(AssertUnwindSafe(|| {
        assert_rejected(|| {
            conformance::snapshot_never_invents_objects(carrying(
                Defect::SnapshotPanicsLikeAssertion,
            ))
        })
    }))
    .expect_err("an assertion-shaped snapshot panic must escape the clause witness");
    assert_eq!(
        snapshot.downcast_ref::<&str>().copied(),
        Some(ASSERTION_SHAPED_INFRASTRUCTURE_PANIC)
    );

    let fixture = catch_unwind(AssertUnwindSafe(|| {
        assert_rejected(|| {
            conformance::apply_and_removal_reach_the_universe_with(carrying(Defect::None), || {
                std::panic::panic_any(98_u64)
            })
        })
    }))
    .expect_err("a fixture identity panic must escape the clause witness");
    assert_eq!(fixture.downcast_ref::<u64>(), Some(&98_u64));
}

#[test]
fn established_clauses_reject_their_distinct_faults() {
    assert_rejected(|| conformance::grant_is_replayable(carrying(Defect::RevokeReturnsAStranger)));
    assert_rejected(|| {
        conformance::revoke_unknown_handle_is_error(carrying(Defect::UnknownHandleReportsSuccess))
    });
    assert_rejected(|| {
        conformance::drained_revoke_removes_object(carrying(Defect::RevokeKeepsTheObject))
    });
    assert_rejected(|| {
        conformance::planted_residue_survives_unrelated_revoke(carrying(
            Defect::RevokeTidiesTheWholeUniverse,
        ))
    });
    assert_rejected(|| {
        conformance::snapshot_never_invents_objects(carrying(Defect::GrantMaterializesAShadow))
    });
    assert_rejected(|| {
        conformance::live_grants_hold_distinct_handles(carrying(Defect::OneHandleForEveryLiveGrant))
    });
    assert_rejected(|| {
        conformance::apply_and_removal_reach_the_universe(carrying(Defect::RemovalIsANoOp))
    });
    assert_rejected(|| {
        conformance::revoke_of_a_revoked_handle_is_error(carrying(
            Defect::ARevokedHandleRevokesAgain,
        ))
    });
    assert_rejected(|| {
        conformance::drained_revoke_removes_object(carrying(Defect::ForcedRevokeIsALie))
    });
    assert_rejected(|| {
        conformance::live_grants_hold_distinct_handles(carrying(Defect::ALedgerKeyedByClass))
    });
    assert_rejected(|| {
        conformance::revoke_of_a_revoked_handle_is_error(carrying(Defect::ARevokedHandleIsReissued))
    });
    assert_rejected(|| {
        conformance::live_grants_hold_distinct_handles(carrying(
            Defect::RevokeTakesAnotherResourceOfClass,
        ))
    });
}

#[test]
fn revoke_order_witnesses_are_independent() {
    assert_rejected(|| {
        conformance::live_grants_hold_distinct_handles(carrying(Defect::RevokesOnlyInGrantOrder))
    });
    assert_rejected(|| {
        conformance::live_grants_hold_distinct_handles(carrying(
            Defect::RevokesOnlyInReversePushOrder,
        ))
    });
    assert_rejected(|| {
        conformance::revoke_takes_its_owners_object(carrying(Defect::RevokesOnlyInGrantOrder))
    });
    assert_rejected(|| {
        conformance::revoke_takes_its_owners_object(carrying(Defect::RevokesOnlyInReversePushOrder))
    });
    assert_rejected(|| {
        conformance::repeated_grants_are_independently_removable(carrying(
            Defect::RevokesOnlyInGrantOrder,
        ))
    });
    assert_rejected(|| {
        conformance::repeated_grants_are_independently_removable(carrying(
            Defect::RevokesOnlyInReversePushOrder,
        ))
    });
}

#[test]
fn applied_removal_preserves_cross_class_survivors() {
    assert_rejected(|| {
        conformance::apply_and_removal_reach_the_universe(carrying(
            Defect::ApplyRemovalDeletesOtherClasses,
        ))
    });
}

#[test]
fn owner_and_instance_clauses_reject_exact_selection_faults() {
    assert_rejected(|| {
        conformance::revoke_takes_its_owners_object(carrying(Defect::RevokeTakesWrongOwner))
    });
    assert_rejected(|| {
        conformance::revoke_takes_its_owners_object(carrying(Defect::GrantSubstitutesLiveOwner))
    });
    for defect in [
        Defect::GrantSubstitutesMountSource,
        Defect::GrantSubstitutesProxyRoute,
        Defect::GrantSubstitutesBrokerName,
    ] {
        assert_rejected(|| {
            conformance::repeated_grants_are_independently_removable(carrying(defect))
        });
    }
    assert_rejected(|| {
        conformance::repeated_grants_are_independently_removable(carrying(
            Defect::IdenticalGrantsCollapse,
        ))
    });
    assert_rejected(|| {
        conformance::repeated_grants_are_independently_removable(carrying(
            Defect::RevokeTakesWrongInstance,
        ))
    });
}

#[test]
fn applied_removal_order_witnesses_are_independent() {
    assert_rejected(|| {
        conformance::apply_and_removal_reach_the_universe(carrying(
            Defect::AppliedRemovalsOnlyInGrantOrder,
        ))
    });
    assert_rejected(|| {
        conformance::apply_and_removal_reach_the_universe(carrying(
            Defect::AppliedRemovalsOnlyInReverseOrder,
        ))
    });
}
