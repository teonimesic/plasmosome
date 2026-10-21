use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::rc::Rc;

use super as conformance;
use crate::builders::{broker, mount, proxy_map, session_file, uds_socket};
use plasmosome_backend::{
    BackendError, Capability, CellId, CellOwner, CompositeBackend, DrainSpec, EnforcementBackend,
    FileAccess, GrantId, GrantKind, Handle, LedgerEntry, OsObject, OsState, PluginId, RevokePolicy,
    UniverseClass, UniverseOp, UniverseRemoval,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Defect {
    None,
    RevokeReturnsAStranger(UniverseClass),
    RevokeReturnsAnotherRecipe(UniverseClass),
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
    GrantSubstitutesBrokerLaunch,
    GrantSubstitutesRequestedOwner,
    GrantSubstitutesRequestedCapability(UniverseClass),
    GrantSubstitutesRecipe(UniverseClass),
    GrantReturnsAnotherRecipe(UniverseClass),
    GrantRecordsAnotherRecipe(UniverseClass),
    GrantPanics,
    SnapshotPanicsLikeAssertion,
    RevokeTakesTheSamePluginInAnotherCell,
    RemovalComparesOnlyThePlugin,
    IgnoresDrainTimeouts,
    TimeoutReleasesBeforeRefusing,
    ForceRefusedWhilePeerStands,
    StallBlocksEqualPeers,
    ApplyRemovalIgnoresDrain,
    RevokeIgnoresDrain,
    DrainsBeforeResolving,
    DrainsBeforeResolvingTheNamedOwner,
    RemovalComparesOnlyTheCell,
    StallKeyedByPlugin,
    ZeroDeadlineForces,
    TimeoutReportsAZeroDeadline,
    StallKeyedByCell,
    ZeroDeadlineNeverReleases,
    ApplyRemovalDropsTheRecordOnTimeout,
    ApplyRemovalLeavesAStaleHandle,
    StallKeyedByPluginAboveZero,
    ForcedApplyRemovalLeavesAStaleHandle,
    GracefulApplyRemovalLeavesAStaleHandle,
    ForceRefusedWhileANeighbourStands,
    ForceRefusedWhileThePluginStandsElsewhere,
    GrantMintsItsOwnId,
    RetryRewritesTheIssuedRecord,
    DirectApplyPromotesToGrant,
    PlantedResiduePromotesToGrant,
    RetryIsRefused,
    RetryIgnores(Field),
    LiveGrantLosesItsRecordTo(Call),
    RecordsItsOwnRecoveryLog,
    AcceptsInvalidOperations(Call),
    SwallowsInvalidOperations(Call),
    ValidatesAfterTheEffect(Call),
    RefusesAnotherLaunchAtAnOccupiedEndpoint,
    GrantReplacesAnAppliedObject,
    GrantReplacesAPlantedObject,
    AcceptsInvalidOperationsOf(UniverseClass, Call),
    RefusalRollsBackItsAddress(Call),
    ChecksTheAddressBeforeValidating(Call),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Kind,
    Plugin,
    Cell,
    Recipe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Call {
    Grant,
    Apply,
    Plant,
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
    recovery_log: Rc<RefCell<Vec<LedgerEntry>>>,
}

impl DefectiveBackend {
    fn carrying(defect: Defect) -> DefectiveBackend {
        DefectiveBackend::recovering(defect, Rc::default())
    }

    fn recovering(defect: Defect, recovery_log: Rc<RefCell<Vec<LedgerEntry>>>) -> DefectiveBackend {
        let mut backend = DefectiveBackend {
            defect,
            state: OsState::new(),
            ledger: BTreeMap::new(),
            spent: BTreeMap::new(),
            freed: Vec::new(),
            grant_order: Vec::new(),
            applied: Vec::new(),
            planted: Vec::new(),
            first_returned: None,
            recovery_log,
        };
        if defect == Defect::RecordsItsOwnRecoveryLog {
            let recovered = backend.recovery_log.borrow().clone();
            for entry in recovered {
                backend.state.insert(entry.object()).unwrap();
                backend.grant_order.push(entry.handle);
                backend.ledger.insert(entry.handle, entry);
            }
        }
        backend
    }

    fn mirrors_its_ledger(&self) -> bool {
        self.defect == Defect::AMirrorOfItsOwnLedger
    }

    fn address_for(&mut self, operation: &UniverseOp) -> Handle {
        match self.defect {
            Defect::GrantMintsItsOwnId => Handle {
                class: operation.class(),
                id: GrantId::new(),
            },
            Defect::ARevokedHandleIsReissued => {
                self.freed.pop().unwrap_or_else(|| operation.handle())
            }
            _ => operation.handle(),
        }
    }

    fn loses_a_live_record_to(&mut self, call: Call, handle: Handle) {
        if self.defect == Defect::LiveGrantLosesItsRecordTo(call) {
            self.ledger.remove(&handle);
            self.grant_order.retain(|held| *held != handle);
        }
    }

    fn validated(&mut self, operation: &UniverseOp, call: Call) -> Result<bool, BackendError> {
        if self.defect == Defect::ChecksTheAddressBeforeValidating(call)
            && self.observed().objects().any(|held| {
                held.class() == operation.class()
                    && held.id == operation.id()
                    && *held != operation.object()
            })
        {
            return Err(identity_conflict(operation.handle()));
        }
        let Err(error) = operation.validate() else {
            return Ok(true);
        };
        let refusal = BackendError::InvalidOperation {
            class: operation.class().as_str(),
            id: operation.id(),
            error,
        };
        match self.defect {
            Defect::AcceptsInvalidOperations(accepted) if accepted == call => Ok(true),
            Defect::AcceptsInvalidOperationsOf(class, accepted)
                if class == operation.class() && accepted == call =>
            {
                Ok(true)
            }
            Defect::SwallowsInvalidOperations(swallowed) if swallowed == call => Ok(false),
            Defect::ValidatesAfterTheEffect(late) if late == call => {
                let _ = self.state.insert(operation.object());
                Err(refusal)
            }
            Defect::RefusalRollsBackItsAddress(rollback) if rollback == call => {
                self.drop_address(operation.handle());
                Err(refusal)
            }
            _ => Err(refusal),
        }
    }

    fn drop_address(&mut self, handle: Handle) {
        let standing: Vec<OsObject> = self
            .state
            .objects()
            .filter(|held| held.class() == handle.class && held.id == handle.id)
            .cloned()
            .collect();
        for object in standing {
            self.state.remove(&removal_of(&object), &object.owner);
        }
    }

    fn observed(&self) -> OsState {
        if self.mirrors_its_ledger() {
            self.mirrored_state()
        } else {
            self.state.clone()
        }
    }

    fn refuse_an_occupied_endpoint(&self, capability: &Capability) -> Result<(), BackendError> {
        if self.defect != Defect::RefusesAnotherLaunchAtAnOccupiedEndpoint {
            return Ok(());
        }
        let Capability::Broker { launch, .. } = capability else {
            return Ok(());
        };
        let endpoints = [&launch.control_socket, &launch.data_socket];
        let occupied = self
            .observed()
            .objects()
            .any(|held| match &held.capability {
                Capability::Broker {
                    launch: standing, ..
                } => {
                    standing != launch
                        && [&standing.control_socket, &standing.data_socket]
                            .iter()
                            .any(|endpoint| endpoints.contains(endpoint))
                }
                _ => false,
            });
        if occupied {
            return Err(BackendError::Fault(
                "another launch holds an endpoint of this broker".to_string(),
            ));
        }
        Ok(())
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
            Defect::RevokeTakesTheSamePluginInAnotherCell => {
                self.take_same_plugin_in_another_cell(entry)
            }
            _ => self.remove_exact(&entry.removal(), &entry.owner),
        }
    }

    fn take_same_plugin_in_another_cell(
        &mut self,
        entry: &LedgerEntry,
    ) -> Result<(), BackendError> {
        let object = self
            .state
            .objects()
            .find(|held| {
                held.capability == entry.capability
                    && held.owner.plugin == entry.owner.plugin
                    && held.owner.cell != entry.owner.cell
            })
            .cloned();
        self.remove_selected(object.or_else(|| Some(entry.object())), entry)
    }

    fn selects(&self, removal: &UniverseRemoval, owner: &CellOwner) -> bool {
        if self.mirrors_its_ledger() {
            return self.mirrored_state().selects(removal, owner);
        }
        if self.defect == Defect::RemovalComparesOnlyThePlugin {
            return self.state.objects().any(|held| {
                held.id == removal.id
                    && held.capability == removal.capability
                    && held.owner.plugin == owner.plugin
            });
        }
        if self.defect == Defect::RemovalComparesOnlyTheCell {
            return self.state.objects().any(|held| {
                held.id == removal.id
                    && held.capability == removal.capability
                    && held.owner.cell == owner.cell
            });
        }
        self.state.selects(removal, owner)
    }

    fn drain_refusal(
        &self,
        owner: &CellOwner,
        capability: &Capability,
        address: Handle,
        drain: DrainSpec,
    ) -> Option<BackendError> {
        let stalled = conformance::stalled_owner();
        if drain.policy == RevokePolicy::Force {
            let blocks = |held: &OsObject| match self.defect {
                Defect::ForceRefusedWhilePeerStands => true,
                Defect::ForceRefusedWhileANeighbourStands => held.owner.cell == owner.cell,
                Defect::ForceRefusedWhileThePluginStandsElsewhere => {
                    held.owner.plugin == owner.plugin
                }
                _ => false,
            };
            let refused = *owner == stalled
                && self.state.objects().any(|held| {
                    held.capability == *capability && held.owner != *owner && blocks(held)
                });
            return refused.then(|| {
                BackendError::Fault("another holder still uses the resource".to_string())
            });
        }
        let stalled_stands = || {
            self.state
                .objects()
                .any(|held| held.owner == stalled && held.capability == *capability)
        };
        let times_out = match self.defect {
            Defect::IgnoresDrainTimeouts => false,
            Defect::ZeroDeadlineForces if drain.deadline.is_zero() => false,
            Defect::StallKeyedByPlugin => owner.plugin == stalled.plugin && stalled_stands(),
            Defect::StallKeyedByCell => owner.cell == stalled.cell && stalled_stands(),
            Defect::StallKeyedByPluginAboveZero if !drain.deadline.is_zero() => {
                owner.plugin == stalled.plugin
            }
            Defect::ZeroDeadlineNeverReleases if drain.deadline.is_zero() => true,
            Defect::StallBlocksEqualPeers => *owner == stalled || stalled_stands(),
            _ => *owner == stalled,
        };
        times_out.then_some(BackendError::DrainTimedOut {
            handle: address,
            deadline_ms: if self.defect == Defect::TimeoutReportsAZeroDeadline {
                0
            } else {
                drain.deadline_ms()
            },
        })
    }

    fn drained(
        &mut self,
        owner: &CellOwner,
        capability: &Capability,
        address: Handle,
        drain: DrainSpec,
        release: impl FnOnce(&mut Self) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        let Some(refusal) = self.drain_refusal(owner, capability, address, drain) else {
            return release(self);
        };
        if self.defect == Defect::TimeoutReleasesBeforeRefusing {
            release(self)?;
        }
        Err(refusal)
    }

    fn remove_exact(
        &mut self,
        removal: &UniverseRemoval,
        owner: &CellOwner,
    ) -> Result<(), BackendError> {
        let address = Handle {
            class: removal.class(),
            id: removal.id,
        };
        if self.mirrors_its_ledger() {
            let granted = self
                .ledger
                .get(&address)
                .is_some_and(|entry| holds_exactly(&entry.object(), removal, owner));
            if granted
                || remove_recorded(&mut self.applied, removal, owner)
                || remove_planted(&mut self.planted, removal, owner)
            {
                self.ledger.remove(&address);
                return Ok(());
            }
            return Err(unknown_object(removal, owner));
        }
        let holder = match self.defect {
            Defect::RemovalComparesOnlyThePlugin | Defect::RemovalComparesOnlyTheCell => self
                .state
                .objects()
                .find(|held| held.id == removal.id && held.capability == removal.capability)
                .map_or_else(|| owner.clone(), |held| held.owner.clone()),
            _ => owner.clone(),
        };
        if self.state.remove(removal, &holder).is_none() {
            return Err(unknown_object(removal, owner));
        }
        self.applied.retain(|op| op.id() != removal.id);
        if self.defect != Defect::ApplyRemovalLeavesAStaleHandle {
            self.ledger.remove(&address);
        }
        if self.defect == Defect::ApplyRemovalDeletesOtherClasses {
            let survivors: Vec<OsObject> = self
                .state
                .objects()
                .filter(|object| object.class() == address.class)
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
            .find(|held| held.capability == entry.capability && held.owner != entry.owner)
            .cloned();
        self.remove_selected(object.or_else(|| Some(entry.object())), entry)
    }

    fn take_wrong_instance(&mut self, entry: &LedgerEntry) -> Result<(), BackendError> {
        let object = self
            .state
            .objects()
            .find(|held| {
                held.capability == entry.capability
                    && held.owner == entry.owner
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
            return Err(unknown_object(&entry.removal(), &entry.owner));
        };
        self.state
            .remove(&removal_of(&object), &object.owner)
            .map(|_| ())
            .ok_or_else(|| unknown_object(&entry.removal(), &entry.owner))
    }

    fn drain_before_resolving(
        &self,
        removal: &UniverseRemoval,
        owner: &CellOwner,
        address: Handle,
        drain: DrainSpec,
    ) -> Option<BackendError> {
        let drained_owner = match self.defect {
            Defect::DrainsBeforeResolving => self
                .state
                .objects()
                .find(|held| held.class() == address.class && held.id == address.id)?
                .owner
                .clone(),
            Defect::DrainsBeforeResolvingTheNamedOwner => owner.clone(),
            _ => return None,
        };
        self.drain_refusal(&drained_owner, &removal.capability, address, drain)
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
    fn grant(
        &mut self,
        operation: UniverseOp,
        kind: GrantKind,
    ) -> Result<LedgerEntry, BackendError> {
        if self.defect == Defect::GrantPanics {
            std::panic::panic_any(InfrastructureFailure::Grant);
        }
        if !self.validated(&operation, Call::Grant)? {
            return Ok(LedgerEntry {
                handle: operation.handle(),
                owner: operation.owner().clone(),
                capability: operation.capability(),
                kind,
            });
        }
        let mut owner = operation.owner().clone();
        let mut capability = operation.capability();
        if self.defect == Defect::GrantSubstitutesRequestedOwner {
            owner.plugin = PluginId::from("substituted-owner");
        }
        if self.defect == Defect::GrantSubstitutesRequestedCapability(capability.class()) {
            capability = substituted_capability(&capability);
        }
        if self.defect == Defect::GrantSubstitutesRecipe(capability.class()) {
            capability = substituted_recipe(&capability);
        }
        if self.defect == Defect::GrantSubstitutesLiveOwner
            && let Some(live) = self
                .ledger
                .values()
                .find(|entry| entry.capability == capability)
                .map(|entry| entry.owner.clone())
        {
            owner = live;
        }
        let substitutes_colliding_capability = matches!(
            (self.defect, capability.class()),
            (Defect::GrantSubstitutesMountSource, UniverseClass::Mount)
                | (Defect::GrantSubstitutesProxyRoute, UniverseClass::ProxyMap)
                | (
                    Defect::GrantSubstitutesBrokerLaunch,
                    UniverseClass::BrokerPid
                )
        );
        if substitutes_colliding_capability
            && let Some(colliding) = self
                .ledger
                .values()
                .find(|entry| {
                    entry.capability.class() == capability.class()
                        && entry.capability.key() == capability.key()
                        && entry.capability != capability
                })
                .map(|entry| entry.capability.clone())
        {
            capability = colliding;
        }
        let handle = self.address_for(&operation);
        let mut entry = LedgerEntry {
            handle,
            owner,
            capability,
            kind,
        };
        if let Some(issued) = self.ledger.get(&handle) {
            let retried = match self.defect {
                Defect::RetryIgnores(field) => same_ignoring(issued, &entry, field),
                _ => *issued == entry,
            };
            if retried && self.defect != Defect::RetryIsRefused {
                return Ok(issued.clone());
            }
            if self.defect != Defect::RetryRewritesTheIssuedRecord {
                return Err(identity_conflict(handle));
            }
            self.ledger.insert(handle, entry.clone());
            return Ok(entry);
        }
        let observed = self.observed();
        if observed
            .objects()
            .any(|held| held.class() == handle.class && held.id == handle.id)
        {
            let applied = self.applied.iter().any(|op| op.handle() == handle);
            let identical = observed.objects().any(|held| *held == entry.object());
            let promotes = match self.defect {
                Defect::DirectApplyPromotesToGrant => applied,
                Defect::PlantedResiduePromotesToGrant => !applied,
                _ => false,
            } && identical;
            let replaces = match self.defect {
                Defect::GrantReplacesAnAppliedObject => applied,
                Defect::GrantReplacesAPlantedObject => !applied,
                _ => false,
            } && !identical;
            if replaces {
                self.drop_address(handle);
                self.state.insert(entry.object())?;
            } else if !promotes {
                return Err(identity_conflict(handle));
            }
            self.ledger.insert(handle, entry.clone());
            self.grant_order.push(handle);
            return Ok(entry);
        }
        self.refuse_an_occupied_endpoint(&entry.capability)?;
        if !self.mirrors_its_ledger() {
            let duplicate = self.defect == Defect::IdenticalGrantsCollapse
                && self
                    .state
                    .objects()
                    .any(|held| held.owner == entry.owner && held.capability == entry.capability);
            if !duplicate {
                let mut object = entry.object();
                if self.defect == Defect::GrantRecordsAnotherRecipe(object.class()) {
                    object.capability = substituted_recipe(&object.capability);
                }
                self.state.insert(object)?;
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
        if self.defect == Defect::GrantReturnsAnotherRecipe(entry.capability.class()) {
            entry.capability = substituted_recipe(&entry.capability);
        }
        self.ledger.insert(handle, entry.clone());
        self.grant_order.push(handle);
        if self.defect == Defect::RecordsItsOwnRecoveryLog {
            self.recovery_log.borrow_mut().push(entry.clone());
        }
        if self.defect == Defect::OneHandleForEveryLiveGrant {
            let duplicate = self.first_returned.get_or_insert(handle);
            Ok(LedgerEntry {
                handle: *duplicate,
                ..entry
            })
        } else {
            Ok(entry)
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
        let drain = if self.defect == Defect::RevokeIgnoresDrain {
            DrainSpec::forcing()
        } else {
            drain
        };
        self.drained(&entry.owner, &entry.capability, handle, drain, |backend| {
            backend.withdraw(&entry, drain.policy)
        })?;
        self.ledger.remove(&handle);
        self.grant_order.retain(|held| *held != handle);
        self.recovery_log
            .borrow_mut()
            .retain(|recorded| recorded.handle != handle);
        self.freed.push(handle);
        self.spent.insert(handle, entry.clone());
        if self.defect == Defect::RevokeReturnsAStranger(handle.class) {
            return Ok(a_stranger(handle));
        }
        if self.defect == Defect::RevokeReturnsAnotherRecipe(handle.class) {
            return Ok(LedgerEntry {
                capability: substituted_recipe(&entry.capability),
                ..entry
            });
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
        if !self.validated(&op, Call::Apply)? {
            return Ok(());
        }
        if self.mirrors_its_ledger() {
            if self.mirrored_state().insert(op.object())? {
                self.applied.push(op);
            }
            return Ok(());
        }
        self.refuse_an_occupied_endpoint(&op.capability())?;
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
        if !self.state.insert(object)? {
            self.loses_a_live_record_to(Call::Apply, op.handle());
        }
        self.applied.push(op);
        Ok(())
    }

    fn apply_removal(
        &mut self,
        removal: UniverseRemoval,
        owner: &CellOwner,
        drain: DrainSpec,
    ) -> Result<(), BackendError> {
        if self.defect == Defect::RemovalIsANoOp {
            return Ok(());
        }
        let address = Handle {
            class: removal.class(),
            id: removal.id,
        };
        if let Some(refusal) = self.drain_before_resolving(&removal, owner, address, drain) {
            return Err(refusal);
        }
        if self.applied_order_refuses(&removal) || !self.selects(&removal, owner) {
            return Err(unknown_object(&removal, owner));
        }
        if self.defect == Defect::ApplyRemovalDropsTheRecordOnTimeout {
            self.ledger.remove(&address);
            self.grant_order.retain(|held| *held != address);
        }
        let drain = if self.defect == Defect::ApplyRemovalIgnoresDrain
            && !self.ledger.contains_key(&address)
        {
            DrainSpec::forcing()
        } else {
            drain
        };
        let record = self.ledger.get(&address).cloned();
        let removed = self.drained(owner, &removal.capability, address, drain, |backend| {
            backend.remove_exact(&removal, owner)
        });
        let keeps_the_record = match self.defect {
            Defect::ForcedApplyRemovalLeavesAStaleHandle => drain.policy == RevokePolicy::Force,
            Defect::GracefulApplyRemovalLeavesAStaleHandle => {
                drain.policy == RevokePolicy::Graceful
            }
            _ => false,
        };
        if keeps_the_record
            && removed.is_ok()
            && let Some(record) = record
        {
            self.ledger.insert(address, record);
        }
        removed
    }

    fn plant(&mut self, object: OsObject) -> Result<(), BackendError> {
        if !self.validated(&object.operation(), Call::Plant)? {
            return Ok(());
        }
        if self.mirrors_its_ledger() {
            if self.mirrored_state().insert(object.clone())? {
                self.planted.push(object);
            }
            return Ok(());
        }
        let handle = object.operation().handle();
        if !self.state.insert(object)? {
            self.loses_a_live_record_to(Call::Plant, handle);
        }
        Ok(())
    }
}

fn removal_of(object: &OsObject) -> UniverseRemoval {
    UniverseRemoval {
        id: object.id,
        capability: object.capability.clone(),
    }
}

fn same_ignoring(issued: &LedgerEntry, retried: &LedgerEntry, field: Field) -> bool {
    let mut relaxed = retried.clone();
    match field {
        Field::Kind => relaxed.kind = issued.kind,
        Field::Plugin => relaxed.owner.plugin = issued.owner.plugin.clone(),
        Field::Cell => relaxed.owner.cell = issued.owner.cell.clone(),
        Field::Recipe => {
            if selection_of(&relaxed.capability) == selection_of(&issued.capability) {
                relaxed.capability = issued.capability.clone();
            }
        }
    }
    relaxed == *issued
}

fn identity_conflict(handle: Handle) -> BackendError {
    BackendError::IdentityConflict {
        class: handle.class.as_str(),
        id: handle.id,
    }
}

fn unknown_object(removal: &UniverseRemoval, owner: &CellOwner) -> BackendError {
    BackendError::UnknownObject {
        class: removal.class().as_str(),
        key: removal.key(),
        owner: owner.clone(),
        id: removal.id,
    }
}

fn holds_exactly(object: &OsObject, removal: &UniverseRemoval, owner: &CellOwner) -> bool {
    object.id == removal.id && object.owner == *owner && object.capability == removal.capability
}

fn remove_recorded(
    applied: &mut Vec<UniverseOp>,
    removal: &UniverseRemoval,
    owner: &CellOwner,
) -> bool {
    let position = applied
        .iter()
        .position(|op| holds_exactly(&op.object(), removal, owner));
    position.is_some_and(|index| {
        applied.remove(index);
        true
    })
}

fn remove_planted(
    planted: &mut Vec<OsObject>,
    removal: &UniverseRemoval,
    owner: &CellOwner,
) -> bool {
    let position = planted
        .iter()
        .position(|object| holds_exactly(object, removal, owner));
    position.is_some_and(|index| {
        planted.remove(index);
        true
    })
}

fn substituted_capability(capability: &Capability) -> Capability {
    match capability {
        Capability::SessionFile { .. } => session_file("/substituted-session"),
        Capability::UdsSocket { .. } => uds_socket("/substituted.uds"),
        Capability::ProxyMap { host, .. } => proxy_map(host, "substituted-route"),
        Capability::Broker { .. } => broker("substituted-broker"),
        Capability::Mount { target, .. } => mount("/substituted", target),
    }
}

fn substituted_recipe(capability: &Capability) -> Capability {
    let mut substitute = capability.clone();
    match &mut substitute {
        Capability::SessionFile { recipe, .. } => recipe.mode = flipped(recipe.mode),
        Capability::UdsSocket { recipe, .. } => {
            recipe.guest_path = format!("{}.substituted", recipe.guest_path);
        }
        Capability::ProxyMap { recipe, .. } => recipe.port = recipe.port % u16::MAX + 1,
        Capability::Broker { launch, .. } => {
            launch.data_socket = format!("{}.substituted", launch.data_socket);
        }
        Capability::Mount { recipe, .. } => recipe.access = flipped(recipe.access),
    }
    substitute
}

fn flipped(access: FileAccess) -> FileAccess {
    match access {
        FileAccess::ReadOnly => FileAccess::ReadWrite,
        FileAccess::ReadWrite => FileAccess::ReadOnly,
    }
}

fn shadow_of(entry: &LedgerEntry) -> OsObject {
    OsObject {
        id: GrantId::new(),
        owner: entry.owner.clone(),
        capability: session_file(&format!("/shadow/{}", entry.handle.id)),
    }
}

fn a_stranger(handle: Handle) -> LedgerEntry {
    let capability = match handle.class {
        UniverseClass::SessionFile => session_file("/stranger.md"),
        UniverseClass::UdsPath => uds_socket("/stranger.uds"),
        UniverseClass::ProxyMap => proxy_map("stranger.test", "stranger"),
        UniverseClass::BrokerPid => broker("stranger"),
        UniverseClass::Mount => mount("/stranger", "/stranger"),
    };
    LedgerEntry {
        handle,
        owner: CellOwner {
            cell: CellId::from("stranger-cell"),
            plugin: PluginId::from("stranger"),
        },
        capability,
        kind: GrantKind::Hot,
    }
}

fn carrying(defect: Defect) -> impl Fn() -> DefectiveBackend {
    move || DefectiveBackend::carrying(defect)
}

fn carrying_a_shared_log() -> impl Fn() -> DefectiveBackend {
    let log = Rc::new(RefCell::new(Vec::new()));
    move || DefectiveBackend::recovering(Defect::RecordsItsOwnRecoveryLog, Rc::clone(&log))
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

const CLASSES: [UniverseClass; 5] = [
    UniverseClass::SessionFile,
    UniverseClass::UdsPath,
    UniverseClass::ProxyMap,
    UniverseClass::BrokerPid,
    UniverseClass::Mount,
];

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
    conformance::graceful_timeouts_preserve_the_selected_holding(carrying(defect));
    conformance::grant_retry_returns_the_original_entry(carrying(defect));
    conformance::fresh_backends_share_no_authority(carrying(defect));
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
    for class in CLASSES {
        assert_rejected(|| {
            conformance::grant_is_replayable(carrying(Defect::RevokeReturnsAStranger(class)))
        });
    }
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
fn snapshot_clause_rejects_returned_owner_substitution() {
    assert_rejected(|| {
        conformance::snapshot_never_invents_objects(carrying(
            Defect::GrantSubstitutesRequestedOwner,
        ))
    });
}

#[test]
fn snapshot_clause_rejects_returned_capability_substitution() {
    for class in CLASSES {
        assert_rejected(|| {
            conformance::snapshot_never_invents_objects(carrying(
                Defect::GrantSubstitutesRequestedCapability(class),
            ))
        });
    }
}

#[test]
fn a_substitute_and_a_stranger_stand_in_only_for_their_own_class() {
    for class in CLASSES {
        let mut substituting =
            DefectiveBackend::carrying(Defect::GrantSubstitutesRequestedCapability(class));
        let mut estranged = DefectiveBackend::carrying(Defect::RevokeReturnsAStranger(class));
        for prepared in conformance::sample_grants() {
            let targeted = prepared.operation.class() == class;
            let entry = substituting
                .grant(prepared.operation.clone(), prepared.kind)
                .unwrap();
            assert_eq!(
                entry.capability != prepared.operation.capability(),
                targeted,
                "{prepared:?}"
            );
            let entry = estranged
                .grant(prepared.operation.clone(), prepared.kind)
                .unwrap();
            let returned = estranged
                .revoke(entry.handle, DrainSpec::forcing())
                .unwrap();
            assert_eq!(returned != entry, targeted, "{prepared:?}");
        }
    }
}

#[test]
fn snapshot_clause_rejects_a_grant_that_substitutes_a_recipe_in_any_class() {
    for class in CLASSES {
        assert_rejected(|| {
            conformance::snapshot_never_invents_objects(carrying(Defect::GrantSubstitutesRecipe(
                class,
            )))
        });
    }
}

#[test]
fn snapshot_clause_rejects_an_entry_that_returns_another_recipe_in_any_class() {
    for class in CLASSES {
        assert_rejected(|| {
            conformance::snapshot_never_invents_objects(carrying(
                Defect::GrantReturnsAnotherRecipe(class),
            ))
        });
    }
}

#[test]
fn snapshot_clause_rejects_an_object_recorded_with_another_recipe_in_any_class() {
    for class in CLASSES {
        assert_rejected(|| {
            conformance::snapshot_never_invents_objects(carrying(
                Defect::GrantRecordsAnotherRecipe(class),
            ))
        });
    }
}

#[test]
fn replay_clause_rejects_a_revoke_that_returns_another_recipe_in_any_class() {
    for class in CLASSES {
        assert_rejected(|| {
            conformance::grant_is_replayable(carrying(Defect::RevokeReturnsAnotherRecipe(class)))
        });
    }
}

#[test]
fn a_recipe_defect_changes_only_the_recipe_of_its_own_class() {
    for class in CLASSES {
        for (defect, returns_another, records_another) in [
            (Defect::GrantSubstitutesRecipe(class), true, true),
            (Defect::GrantReturnsAnotherRecipe(class), true, false),
            (Defect::GrantRecordsAnotherRecipe(class), false, true),
        ] {
            let mut backend = DefectiveBackend::carrying(defect);
            for capability in conformance::sample_capabilities() {
                let request = conformance::hot_grant(conformance::conformance_owner(), &capability);
                let entry = backend
                    .grant(request.operation.clone(), request.kind)
                    .unwrap();
                let snapshot = backend.snapshot_os_state();
                let object = snapshot
                    .objects()
                    .find(|held| held.id == entry.handle.id)
                    .expect("the grant must stand");
                assert_eq!(
                    (entry.handle.class, &entry.owner, entry.kind, &object.owner),
                    (
                        capability.class(),
                        request.operation.owner(),
                        request.kind,
                        request.operation.owner()
                    ),
                    "{defect:?}"
                );
                let targeted = capability.class() == class;
                let context = format!("{defect:?} on {capability:?}");
                only_the_recipe_differs(
                    &entry.capability,
                    &capability,
                    targeted && returns_another,
                    &context,
                );
                only_the_recipe_differs(
                    &object.capability,
                    &capability,
                    targeted && records_another,
                    &context,
                );
            }
        }
        let mut revoking = DefectiveBackend::carrying(Defect::RevokeReturnsAnotherRecipe(class));
        for prepared in conformance::sample_grants() {
            let entry = revoking
                .grant(prepared.operation.clone(), prepared.kind)
                .unwrap();
            assert_eq!(entry.handle.class, prepared.operation.class());
            assert_eq!(
                (&entry.owner, entry.kind, &entry.capability),
                (
                    prepared.operation.owner(),
                    prepared.kind,
                    &prepared.operation.capability()
                )
            );
            let returned = revoking.revoke(entry.handle, DrainSpec::forcing()).unwrap();
            let snapshot = revoking.snapshot_os_state();
            assert!(
                !snapshot.objects().any(|held| held.id == entry.handle.id),
                "a revoke must withdraw the object: {prepared:?}"
            );
            assert_eq!(
                (returned.handle, &returned.owner, returned.kind),
                (entry.handle, &entry.owner, entry.kind)
            );
            let targeted = prepared.operation.class() == class;
            only_the_recipe_differs(
                &returned.capability,
                &entry.capability,
                targeted,
                &format!("{prepared:?}"),
            );
        }
    }
    for access in [FileAccess::ReadOnly, FileAccess::ReadWrite] {
        assert_ne!(flipped(access), access);
    }
}

fn only_the_recipe_differs(
    observed: &Capability,
    expected: &Capability,
    changed: bool,
    context: &str,
) {
    assert_eq!(observed != expected, changed, "{context}");
    assert_eq!(selection_of(observed), selection_of(expected), "{context}");
    assert_eq!(observed.validate(), Ok(()), "{context}");
}

fn selection_of(capability: &Capability) -> (UniverseClass, Vec<&str>) {
    let fields = match capability {
        Capability::SessionFile { path, .. } | Capability::UdsSocket { path, .. } => vec![path],
        Capability::ProxyMap { host, route, .. } => vec![host, route],
        Capability::Broker { name, .. } => vec![name],
        Capability::Mount { source, target, .. } => vec![source, target],
    };
    (
        capability.class(),
        fields.into_iter().map(String::as_str).collect(),
    )
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
    assert_rejected(|| {
        conformance::fresh_backends_share_no_authority(carrying(Defect::RevokesOnlyInGrantOrder))
    });
    assert_rejected(|| {
        conformance::fresh_backends_share_no_authority(carrying(
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
        Defect::GrantSubstitutesBrokerLaunch,
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

#[test]
fn owner_clause_rejects_revoking_the_same_plugin_in_another_cell() {
    assert_rejected(|| {
        conformance::revoke_takes_its_owners_object(carrying(
            Defect::RevokeTakesTheSamePluginInAnotherCell,
        ))
    });
}

#[test]
fn applied_removal_clause_rejects_a_plugin_only_owner_comparison() {
    assert_rejected(|| {
        conformance::apply_and_removal_reach_the_universe(carrying(
            Defect::RemovalComparesOnlyThePlugin,
        ))
    });
}

#[test]
fn graceful_timeout_clause_rejects_an_ignored_timeout() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::IgnoresDrainTimeouts,
        ))
    });
}

#[test]
fn graceful_timeout_clause_rejects_a_release_before_the_refusal() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::TimeoutReleasesBeforeRefusing,
        ))
    });
}

#[test]
fn stalled_first_pass_rejects_force_refused_while_a_peer_stands() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::ForceRefusedWhilePeerStands,
        ))
    });
}

#[test]
fn peer_first_pass_rejects_a_stall_that_blocks_equal_peers() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::StallBlocksEqualPeers,
        ))
    });
}

#[test]
fn revoke_leg_rejects_a_revoke_that_ignores_the_drain() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::RevokeIgnoresDrain,
        ))
    });
}

#[test]
fn apply_removal_leg_rejects_an_apply_removal_that_ignores_the_drain() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::ApplyRemovalIgnoresDrain,
        ))
    });
}

#[test]
fn wrong_owner_step_rejects_draining_the_holder_before_resolving() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::DrainsBeforeResolving,
        ))
    });
}

#[test]
fn stalled_owner_step_rejects_draining_the_named_owner_before_resolving() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::DrainsBeforeResolvingTheNamedOwner,
        ))
    });
}

#[test]
fn applied_removal_clause_rejects_a_cell_only_owner_comparison() {
    assert_rejected(|| {
        conformance::apply_and_removal_reach_the_universe(carrying(
            Defect::RemovalComparesOnlyTheCell,
        ))
    });
}

#[test]
fn other_cell_step_rejects_a_stall_keyed_by_plugin() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::StallKeyedByPlugin,
        ))
    });
}

#[test]
fn zero_deadline_step_rejects_a_zero_deadline_that_forces() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::ZeroDeadlineForces,
        ))
    });
}

#[test]
fn timeout_step_rejects_a_timeout_that_misreports_its_deadline() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::TimeoutReportsAZeroDeadline,
        ))
    });
}

#[test]
fn neighbour_step_rejects_a_stall_keyed_by_cell() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::StallKeyedByCell,
        ))
    });
}

#[test]
fn zero_release_step_rejects_a_zero_deadline_that_never_releases() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::ZeroDeadlineNeverReleases,
        ))
    });
}

#[test]
fn granted_removal_timeout_step_rejects_dropping_the_record() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::ApplyRemovalDropsTheRecordOnTimeout,
        ))
    });
}

#[test]
fn granted_removal_step_rejects_a_stale_handle() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::ApplyRemovalLeavesAStaleHandle,
        ))
    });
}

#[test]
fn above_zero_other_cell_step_rejects_a_plugin_keyed_stall_above_zero() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::StallKeyedByPluginAboveZero,
        ))
    });
}

#[test]
fn forced_granted_removal_step_rejects_a_stale_handle() {
    assert_rejected(|| {
        conformance::repeated_grants_are_independently_removable(carrying(
            Defect::ForcedApplyRemovalLeavesAStaleHandle,
        ))
    });
}

#[test]
fn peer_removal_step_rejects_a_graceful_stale_handle() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::GracefulApplyRemovalLeavesAStaleHandle,
        ))
    });
}

#[test]
fn graceful_granted_removal_step_rejects_a_stale_handle() {
    assert_rejected(|| {
        conformance::repeated_grants_are_independently_removable(carrying(
            Defect::GracefulApplyRemovalLeavesAStaleHandle,
        ))
    });
}

fn revoke_after_one_removal(
    defect: Defect,
    drain: DrainSpec,
) -> (Handle, Result<LedgerEntry, BackendError>) {
    let mut backend = DefectiveBackend::carrying(defect);
    let request = conformance::hot_grant(conformance::conformance_owner(), &broker("egressd"));
    let entry = backend.grant(request.operation, request.kind).unwrap();
    backend
        .apply_removal(entry.removal(), &entry.owner, drain)
        .unwrap();
    let revoked = backend.revoke(entry.handle, DrainSpec::forcing());
    (entry.handle, revoked)
}

#[test]
fn graceful_stale_handle_defect_spares_a_forced_removal() {
    let (handle, revoked) = revoke_after_one_removal(
        Defect::GracefulApplyRemovalLeavesAStaleHandle,
        DrainSpec::forcing(),
    );
    assert_eq!(revoked, Err(BackendError::UnknownHandle { handle }));
}

#[test]
fn forced_stale_handle_defect_spares_a_graceful_removal() {
    let (handle, revoked) = revoke_after_one_removal(
        Defect::ForcedApplyRemovalLeavesAStaleHandle,
        DrainSpec::graceful(conformance::DRAIN),
    );
    assert_eq!(revoked, Err(BackendError::UnknownHandle { handle }));
}

#[test]
fn neighbour_after_the_force_rejects_force_refused_while_a_neighbour_stands() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::ForceRefusedWhileANeighbourStands,
        ))
    });
}

#[test]
fn other_cell_after_the_force_rejects_force_refused_while_the_plugin_stands_elsewhere() {
    assert_rejected(|| {
        conformance::graceful_timeouts_preserve_the_selected_holding(carrying(
            Defect::ForceRefusedWhileThePluginStandsElsewhere,
        ))
    });
}

#[test]
fn a_substitute_and_a_stranger_keep_the_class_they_stand_in_for() {
    for capability in conformance::sample_capabilities() {
        let substituted = substituted_capability(&capability);
        assert_eq!(substituted.class(), capability.class(), "{capability:?}");
        assert_ne!(substituted, capability);
    }
    for class in CLASSES {
        let handle = Handle {
            class,
            id: GrantId::new(),
        };
        let stranger = a_stranger(handle);
        assert_eq!(stranger.handle, handle);
        assert_eq!(stranger.capability.class(), class);
    }
}

#[test]
fn the_class_loops_run_every_universe_class() {
    let (mut session, mut socket, mut proxy, mut broker, mut mount) =
        (false, false, false, false, false);
    for class in CLASSES {
        match class {
            UniverseClass::SessionFile => session = true,
            UniverseClass::UdsPath => socket = true,
            UniverseClass::ProxyMap => proxy = true,
            UniverseClass::BrokerPid => broker = true,
            UniverseClass::Mount => mount = true,
        }
    }
    assert!(
        session && socket && proxy && broker && mount,
        "the class loops leave out a universe class: {CLASSES:?}"
    );
}

#[test]
fn the_replay_clause_rejects_a_grant_that_ignores_its_supplied_address() {
    assert_rejected(|| conformance::grant_is_replayable(carrying(Defect::GrantMintsItsOwnId)));
    assert_rejected(|| {
        conformance::grant_is_replayable(carrying(Defect::ARevokedHandleIsReissued))
    });
}

#[test]
fn the_retry_clause_rejects_a_rewritten_record() {
    assert_rejected(|| {
        conformance::grant_retry_returns_the_original_entry(carrying(
            Defect::RetryRewritesTheIssuedRecord,
        ))
    });
}

#[test]
fn the_retry_clause_rejects_a_grant_that_promotes_an_applied_or_planted_object() {
    for defect in [
        Defect::DirectApplyPromotesToGrant,
        Defect::PlantedResiduePromotesToGrant,
    ] {
        assert_rejected(|| conformance::grant_retry_returns_the_original_entry(carrying(defect)));
    }
}

#[test]
fn the_retry_clause_rejects_a_grant_that_replaces_an_applied_or_planted_object() {
    for defect in [
        Defect::GrantReplacesAnAppliedObject,
        Defect::GrantReplacesAPlantedObject,
    ] {
        assert_rejected(|| conformance::grant_retry_returns_the_original_entry(carrying(defect)));
    }
}

#[test]
fn a_replacing_grant_takes_over_only_a_changed_object_of_its_own_origin() {
    for (defect, replaced) in [
        (Defect::GrantReplacesAnAppliedObject, Call::Apply),
        (Defect::GrantReplacesAPlantedObject, Call::Plant),
    ] {
        for origin in [Call::Apply, Call::Plant] {
            let standing = conformance::op_for(
                GrantId::new(),
                conformance::conformance_owner(),
                mount("/src", "/work"),
            );
            let changed = conformance::op_for(
                standing.id(),
                conformance::second_owner(),
                mount("/src", "/work"),
            );
            let mut backend = DefectiveBackend::carrying(defect);
            call_with(&mut backend, origin, &standing).unwrap();
            assert_eq!(
                backend.grant(standing.clone(), GrantKind::Hot),
                Err(identity_conflict(standing.handle())),
                "{defect:?} must not promote an identical {origin:?} object"
            );
            let outcome = backend.grant(changed.clone(), GrantKind::Hot);
            let mut expected = OsState::new();
            if origin == replaced {
                assert_eq!(outcome.map(|entry| entry.handle), Ok(changed.handle()));
                expected.insert(changed.object()).unwrap();
            } else {
                assert_eq!(outcome, Err(identity_conflict(changed.handle())));
                expected.insert(standing.object()).unwrap();
            }
            assert_eq!(
                backend.snapshot_os_state(),
                expected,
                "{defect:?} {origin:?}"
            );
        }
    }
}

#[test]
fn the_retry_clause_rejects_a_refused_retry_and_a_retry_that_ignores_any_field() {
    assert_rejected(|| {
        conformance::grant_retry_returns_the_original_entry(carrying(Defect::RetryIsRefused))
    });
    for field in [Field::Kind, Field::Plugin, Field::Cell, Field::Recipe] {
        assert_rejected(|| {
            conformance::grant_retry_returns_the_original_entry(carrying(Defect::RetryIgnores(
                field,
            )))
        });
    }
}

#[test]
fn the_retry_clause_rejects_an_apply_or_plant_that_drops_a_live_grants_record() {
    for call in [Call::Apply, Call::Plant] {
        assert_rejected(|| {
            conformance::grant_retry_returns_the_original_entry(carrying(
                Defect::LiveGrantLosesItsRecordTo(call),
            ))
        });
    }
}

#[test]
fn a_retry_defect_relaxes_only_its_own_field() {
    let first = conformance::hot_grant(conformance::conformance_owner(), &broker("egressd"));
    let mut relaunched = broker("egressd");
    if let Capability::Broker { launch, .. } = &mut relaunched {
        launch.data_socket = format!("{}.changed", launch.data_socket);
    }
    let changes = [
        (
            Field::Kind,
            first.operation.clone(),
            GrantKind::GenerationBound,
        ),
        (
            Field::Plugin,
            conformance::op_for(
                first.operation.id(),
                conformance::second_owner(),
                broker("egressd"),
            ),
            GrantKind::Hot,
        ),
        (
            Field::Cell,
            conformance::op_for(
                first.operation.id(),
                conformance::other_cell(&conformance::conformance_owner()),
                broker("egressd"),
            ),
            GrantKind::Hot,
        ),
        (
            Field::Recipe,
            conformance::op_for(
                first.operation.id(),
                conformance::conformance_owner(),
                relaunched,
            ),
            GrantKind::Hot,
        ),
    ];
    for (relaxed, _, _) in &changes {
        let mut backend = DefectiveBackend::carrying(Defect::RetryIgnores(*relaxed));
        let entry = backend.grant(first.operation.clone(), first.kind).unwrap();
        for (field, operation, kind) in &changes {
            let outcome = backend.grant(operation.clone(), *kind);
            if field == relaxed {
                assert_eq!(outcome, Ok(entry.clone()), "{relaxed:?}");
            } else {
                assert_eq!(
                    outcome,
                    Err(identity_conflict(entry.handle)),
                    "{relaxed:?} must still compare {field:?}"
                );
            }
        }
        let renamed = conformance::op_for(
            first.operation.id(),
            conformance::conformance_owner(),
            broker("renamed"),
        );
        assert_eq!(
            backend.grant(renamed, first.kind),
            Err(identity_conflict(entry.handle)),
            "{relaxed:?} must still compare the name"
        );
    }
}

#[test]
fn both_fresh_instance_checks_reject_a_backend_that_recovers_from_its_own_log() {
    assert_rejected(|| conformance::fresh_backends_share_no_authority(carrying_a_shared_log()));
    assert_rejected(|| conformance::snapshot_never_invents_objects(carrying_a_shared_log()));
}

#[test]
fn a_recovery_log_carries_grants_and_revokes_into_the_next_instance() {
    let make = carrying_a_shared_log();
    let mut first = make();
    let kept = conformance::hot_grant(conformance::conformance_owner(), &broker("egressd"));
    let gone = conformance::hot_grant(conformance::conformance_owner(), &mount("/src", "/work"));
    let kept_entry = first.grant(kept.operation.clone(), kept.kind).unwrap();
    let gone_entry = first.grant(gone.operation.clone(), gone.kind).unwrap();
    first
        .revoke(gone_entry.handle, DrainSpec::forcing())
        .unwrap();
    let mut second = make();
    let mut expected = OsState::new();
    expected.insert(kept.operation.object()).unwrap();
    assert_eq!(second.snapshot_os_state(), expected);
    assert_eq!(
        second.revoke(kept_entry.handle, DrainSpec::forcing()),
        Ok(kept_entry)
    );
}

#[test]
fn the_snapshot_clause_rejects_any_call_that_mishandles_an_invalid_operation() {
    for call in CALLS {
        for defect in [
            Defect::AcceptsInvalidOperations(call),
            Defect::SwallowsInvalidOperations(call),
            Defect::ValidatesAfterTheEffect(call),
        ] {
            assert_rejected(|| conformance::snapshot_never_invents_objects(carrying(defect)));
        }
    }
}

const CALLS: [Call; 3] = [Call::Grant, Call::Apply, Call::Plant];

fn call_with(
    backend: &mut DefectiveBackend,
    call: Call,
    operation: &UniverseOp,
) -> Result<(), BackendError> {
    match call {
        Call::Grant => backend
            .grant(operation.clone(), GrantKind::Hot)
            .map(|entry| assert_eq!(entry.handle, operation.handle())),
        Call::Apply => backend.apply(operation.clone()),
        Call::Plant => backend.plant(operation.object()),
    }
}

fn invalid_of_every_class() -> Vec<UniverseOp> {
    let mut proxy = proxy_map("api.plasmosome.test", "splice");
    if let Capability::ProxyMap { recipe, .. } = &mut proxy {
        recipe.port = 0;
    }
    [
        session_file("relative/skills/pr.md"),
        uds_socket("relative/egressd.uds"),
        proxy,
        broker("egress\0d"),
        mount("/src", "/workspace/"),
    ]
    .into_iter()
    .map(|capability| {
        conformance::op_for(GrantId::new(), conformance::conformance_owner(), capability)
    })
    .collect()
}

#[test]
fn the_invalid_fixtures_cover_every_class_and_each_one_fails_validation() {
    let invalid = invalid_of_every_class();
    assert_eq!(
        invalid.iter().map(UniverseOp::class).collect::<Vec<_>>(),
        CLASSES
    );
    for operation in invalid {
        assert!(operation.validate().is_err(), "{operation:?}");
    }
}

#[test]
fn the_snapshot_clause_rejects_an_invalid_operation_accepted_for_any_one_class() {
    for class in CLASSES {
        for call in CALLS {
            assert_rejected(|| {
                conformance::snapshot_never_invents_objects(carrying(
                    Defect::AcceptsInvalidOperationsOf(class, call),
                ))
            });
        }
    }
}

#[test]
fn a_one_class_invalid_operation_defect_accepts_only_its_own_class_through_its_own_call() {
    for class in CLASSES {
        for defective in CALLS {
            for operation in invalid_of_every_class() {
                for call in CALLS {
                    let mut backend = DefectiveBackend::carrying(
                        Defect::AcceptsInvalidOperationsOf(class, defective),
                    );
                    let accepted = operation.class() == class && call == defective;
                    let outcome = call_with(&mut backend, call, &operation);
                    assert_eq!(
                        outcome.is_ok(),
                        accepted,
                        "{class:?} {defective:?} {call:?}"
                    );
                    assert_eq!(
                        !backend.snapshot_os_state().is_empty(),
                        accepted,
                        "{class:?} {defective:?} {call:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_snapshot_clause_rejects_a_composite_whose_network_leaf_accepts_invalid_grants() {
    let composite = |network: Defect| {
        move || {
            CompositeBackend::new(
                Box::new(DefectiveBackend::carrying(network)),
                Box::new(DefectiveBackend::carrying(Defect::None)),
                Box::new(DefectiveBackend::carrying(Defect::None)),
            )
            .unwrap()
        }
    };
    conformance::snapshot_never_invents_objects(composite(Defect::None));
    assert_rejected(|| {
        conformance::snapshot_never_invents_objects(composite(Defect::AcceptsInvalidOperations(
            Call::Grant,
        )))
    });
}

#[test]
fn the_snapshot_clause_rejects_a_refusal_that_clears_its_address_or_checks_it_first() {
    for call in CALLS {
        for defect in [
            Defect::RefusalRollsBackItsAddress(call),
            Defect::ChecksTheAddressBeforeValidating(call),
        ] {
            assert_rejected(|| conformance::snapshot_never_invents_objects(carrying(defect)));
        }
    }
}

#[test]
fn an_occupied_address_defect_changes_only_its_own_call() {
    for defective in CALLS {
        for defect in [
            Defect::RefusalRollsBackItsAddress(defective),
            Defect::ChecksTheAddressBeforeValidating(defective),
        ] {
            for call in CALLS {
                let mut backend = DefectiveBackend::carrying(defect);
                let standing = conformance::hot_grant(
                    conformance::conformance_owner(),
                    &session_file("/var/lib/plasmosome/conformance/skills/pr.md"),
                );
                let entry = backend
                    .grant(standing.operation.clone(), standing.kind)
                    .unwrap();
                let invalid = conformance::op_for(
                    standing.operation.id(),
                    conformance::conformance_owner(),
                    session_file("relative/skills/pr.md"),
                );
                let outcome = call_with(&mut backend, call, &invalid);
                let mut expected = OsState::new();
                expected.insert(standing.operation.object()).unwrap();
                if call == defective && defect == Defect::RefusalRollsBackItsAddress(call) {
                    expected = OsState::new();
                }
                if call == defective && defect == Defect::ChecksTheAddressBeforeValidating(call) {
                    assert_eq!(outcome, Err(identity_conflict(invalid.handle())));
                } else {
                    assert!(
                        matches!(outcome, Err(BackendError::InvalidOperation { .. })),
                        "{defect:?} through {call:?}: {outcome:?}"
                    );
                }
                assert_eq!(backend.snapshot_os_state(), expected, "{defect:?} {call:?}");
                assert_eq!(backend.ledger.get(&entry.handle), Some(&entry));
            }
        }
    }
}

#[test]
fn an_invalid_operation_defect_mishandles_only_its_own_call_in_its_own_way() {
    let invalid = invalid_of_every_class().remove(0);
    for defective in CALLS {
        for (defect, refused, changed) in [
            (Defect::AcceptsInvalidOperations(defective), false, true),
            (Defect::SwallowsInvalidOperations(defective), false, false),
            (Defect::ValidatesAfterTheEffect(defective), true, true),
        ] {
            for call in CALLS {
                let mut backend = DefectiveBackend::carrying(defect);
                let outcome = call_with(&mut backend, call, &invalid);
                let (refused, changed) = if call == defective {
                    (refused, changed)
                } else {
                    (true, false)
                };
                assert_eq!(
                    matches!(outcome, Err(BackendError::InvalidOperation { .. })),
                    refused,
                    "{defect:?} through {call:?}: {outcome:?}"
                );
                assert_eq!(
                    !backend.snapshot_os_state().is_empty(),
                    changed,
                    "{defect:?} through {call:?}"
                );
            }
        }
    }
}

#[test]
fn a_backend_that_refuses_another_launch_at_an_occupied_endpoint_passes_every_clause() {
    run_all(Defect::RefusesAnotherLaunchAtAnOccupiedEndpoint);
}

#[test]
fn the_endpoint_guard_refuses_another_launch_at_either_socket_of_a_standing_broker() {
    let standing = broker("egressd");
    let Capability::Broker { launch, .. } = &standing else {
        unreachable!("broker returns a broker");
    };
    let relaunch = |change: &dyn Fn(&mut plasmosome_backend::BrokerLaunch)| {
        let mut capability = broker("egressd-again");
        if let Capability::Broker { launch, .. } = &mut capability {
            launch.command.push("--again".to_string());
            launch.control_socket = "/run/plasmosome/again.control".to_string();
            launch.data_socket = "/run/plasmosome/again.data".to_string();
            change(launch);
        }
        capability
    };
    let refused = [
        relaunch(&|again| again.control_socket = launch.control_socket.clone()),
        relaunch(&|again| again.data_socket = launch.data_socket.clone()),
        relaunch(&|again| again.control_socket = launch.data_socket.clone()),
        relaunch(&|again| again.data_socket = launch.control_socket.clone()),
    ];
    let accepted = [broker("egressd-same-launch"), relaunch(&|_| {})];
    for through_grant in [true, false] {
        let mut backend =
            DefectiveBackend::carrying(Defect::RefusesAnotherLaunchAtAnOccupiedEndpoint);
        let first = conformance::hot_grant(conformance::conformance_owner(), &standing);
        backend.grant(first.operation, first.kind).unwrap();
        let mut create = |capability: &Capability| {
            let request = conformance::hot_grant(conformance::second_owner(), capability);
            if through_grant {
                backend.grant(request.operation, request.kind).map(|_| ())
            } else {
                backend.apply(request.operation)
            }
        };
        for capability in &refused {
            let outcome = create(capability);
            assert!(
                matches!(outcome, Err(BackendError::Fault(_))),
                "{capability:?} through grant {through_grant}: {outcome:?}"
            );
        }
        for capability in &accepted {
            create(capability).unwrap();
        }
        assert_eq!(backend.snapshot_os_state().len(), 3);
        let planted = conformance::hot_grant(conformance::second_owner(), &refused[0]);
        backend.plant(planted.operation.object()).unwrap();
    }
}

#[test]
fn the_relaunched_broker_has_its_own_command_and_sockets() {
    let (
        Capability::Broker { name, launch },
        Capability::Broker {
            name: relaunched_name,
            launch: relaunched,
        },
    ) = (broker("egressd"), conformance::relaunched_broker())
    else {
        unreachable!("both are brokers");
    };
    assert_eq!(relaunched_name, name);
    assert_ne!(relaunched.command, launch.command);
    let first = [&launch.control_socket, &launch.data_socket];
    for endpoint in [&relaunched.control_socket, &relaunched.data_socket] {
        assert!(!first.contains(&endpoint), "{endpoint} is already taken");
    }
    assert_ne!(relaunched.control_socket, relaunched.data_socket);
}
