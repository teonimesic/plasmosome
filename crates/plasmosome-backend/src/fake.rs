use std::collections::{BTreeMap, BTreeSet};

use crate::backend::{
    BackendError, DrainSpec, EnforcementBackend, GrantKind, Handle, LedgerEntry, RevokePolicy,
};
use crate::snapshot::{EnforcementSnapshot, IncompleteEffect};
use crate::universe::{CellOwner, OsObject, OsState, UniverseOp, UniverseRemoval};

#[derive(Debug, Default)]
pub struct FakeBackend {
    state: OsState,
    grants: BTreeMap<Handle, LedgerEntry>,
    incomplete: BTreeMap<Handle, Unfinished>,
    stuck_handles: BTreeSet<Handle>,
    stalled_owners: BTreeSet<CellOwner>,
    revocations: Vec<Handle>,
    refused_owners: BTreeMap<CellOwner, String>,
    doomed_owners: BTreeMap<CellOwner, String>,
    failing_withdrawals: BTreeMap<Handle, String>,
    observation_fault: Option<String>,
}

#[derive(Debug)]
struct Unfinished {
    operation: UniverseOp,
    detail: String,
}

impl Unfinished {
    fn error(&self) -> BackendError {
        BackendError::IncompleteEffect {
            class: self.operation.class().as_str(),
            id: self.operation.id(),
            detail: self.detail.clone(),
        }
    }
}

impl FakeBackend {
    pub fn new() -> FakeBackend {
        FakeBackend::default()
    }

    pub fn peek_entry(&self, handle: Handle) -> Result<LedgerEntry, BackendError> {
        self.grants
            .get(&handle)
            .cloned()
            .ok_or(BackendError::UnknownHandle { handle })
    }

    /// Makes every later `grant` and `apply` of a valid operation owned by exactly `owner` fail
    /// with `BackendError::Fault(cause)` before any change. The same plugin in another cell, and
    /// every withdrawal, are not affected. An invalid operation still returns `InvalidOperation`.
    pub fn fail_before_effect_for_owner(&mut self, owner: CellOwner, cause: &str) {
        self.refused_owners.insert(owner, cause.to_string());
    }

    /// Makes every later `grant` and `apply` of a valid operation owned by exactly `owner`, at an
    /// address that holds nothing, fail after its first effect. The call returns
    /// `BackendError::IncompleteEffect` carrying `detail` and issues no record, and the snapshot
    /// lists the operation in `incomplete` until an exact `apply_removal` clears it. The same
    /// plugin in another cell is not affected. An invalid operation still returns
    /// `InvalidOperation`, and `fail_before_effect_for_owner` refuses first.
    pub fn fail_after_effect_for_owner(&mut self, owner: CellOwner, detail: &str) {
        self.doomed_owners.insert(owner, detail.to_string());
    }

    /// Makes the next withdrawal at this exact address, through `revoke` or `apply_removal` and
    /// under either policy, fail after it released something. The call returns
    /// `BackendError::IncompleteEffect` carrying `detail`. A standing holding moves to
    /// `incomplete` and keeps its issued record. A graceful timeout comes first and leaves the
    /// failure armed.
    pub fn fail_next_withdrawal(&mut self, handle: Handle, detail: &str) {
        self.failing_withdrawals.insert(handle, detail.to_string());
    }

    /// Makes every graceful withdrawal of the holding at this exact address time out, through
    /// either `revoke` or `apply_removal`. A forced withdrawal still succeeds.
    pub fn mark_stuck(&mut self, handle: Handle) {
        self.stuck_handles.insert(handle);
    }

    /// Makes every graceful withdrawal of a holding owned by exactly `owner` time out before any
    /// release, through either `revoke` or `apply_removal`. Peers of other owners, including the
    /// same plugin in another cell, are not stalled. A forced withdrawal still succeeds.
    pub fn stall_graceful_drains_for_owner(&mut self, owner: CellOwner) {
        self.stalled_owners.insert(owner);
    }

    /// Makes the next `snapshot_os_state` return `BackendError::Fault(cause)`, once, with no
    /// other effect. The snapshot after it observes normally.
    pub fn fail_next_observation(&mut self, cause: &str) {
        self.observation_fault = Some(cause.to_string());
    }

    pub fn plant_residue(&mut self, object: OsObject) -> Result<(), BackendError> {
        self.plant(object)
    }

    pub fn revocations(&self) -> &[Handle] {
        &self.revocations
    }

    fn admitted(&self, operation: &UniverseOp) -> Result<(), BackendError> {
        checked(&operation.object())?;
        if let Some(cause) = self.refused_owners.get(operation.owner()) {
            return Err(BackendError::Fault(cause.clone()));
        }
        match self.incomplete.get(&operation.handle()) {
            Some(unfinished) if unfinished.operation == *operation => Err(unfinished.error()),
            Some(_) => Err(identity_conflict(operation.handle())),
            None => Ok(()),
        }
    }

    fn occupied(&self, handle: Handle) -> bool {
        self.state
            .objects()
            .any(|held| held.class() == handle.class && held.id == handle.id)
    }

    fn fail_after_the_first_effect(&mut self, operation: &UniverseOp) -> Result<(), BackendError> {
        let Some(detail) = self.doomed_owners.get(operation.owner()).cloned() else {
            return Ok(());
        };
        let unfinished = Unfinished {
            operation: operation.clone(),
            detail,
        };
        let error = unfinished.error();
        self.incomplete.insert(operation.handle(), unfinished);
        Err(error)
    }
}

impl EnforcementBackend for FakeBackend {
    fn grant(
        &mut self,
        operation: UniverseOp,
        kind: GrantKind,
    ) -> Result<LedgerEntry, BackendError> {
        self.admitted(&operation)?;
        let object = operation.object();
        let entry = LedgerEntry {
            handle: operation.handle(),
            owner: object.owner.clone(),
            capability: object.capability.clone(),
            kind,
        };
        if let Some(issued) = self.grants.get(&entry.handle) {
            return if *issued == entry {
                Ok(issued.clone())
            } else {
                Err(identity_conflict(entry.handle))
            };
        }
        if self.occupied(entry.handle) {
            return Err(identity_conflict(entry.handle));
        }
        self.fail_after_the_first_effect(&operation)?;
        self.state.insert(object)?;
        self.grants.insert(entry.handle, entry.clone());
        Ok(entry)
    }

    fn revoke(&mut self, handle: Handle, drain: DrainSpec) -> Result<LedgerEntry, BackendError> {
        let entry = self
            .grants
            .get(&handle)
            .cloned()
            .ok_or(BackendError::UnknownHandle { handle })?;
        self.apply_removal(entry.removal(), &entry.owner, drain)?;
        self.revocations.push(handle);
        Ok(entry)
    }

    fn snapshot_os_state(&mut self) -> Result<EnforcementSnapshot, BackendError> {
        if let Some(cause) = self.observation_fault.take() {
            return Err(BackendError::Fault(cause));
        }
        EnforcementSnapshot::new(
            self.state.clone(),
            self.grants.values().cloned().collect(),
            self.incomplete
                .values()
                .map(|unfinished| IncompleteEffect {
                    operation: unfinished.operation.clone(),
                })
                .collect(),
        )
    }

    fn apply(&mut self, op: UniverseOp) -> Result<(), BackendError> {
        self.admitted(&op)?;
        if !self.occupied(op.handle()) {
            self.fail_after_the_first_effect(&op)?;
        }
        self.state.insert(op.object()).map(|_| ())
    }

    fn apply_removal(
        &mut self,
        removal: UniverseRemoval,
        owner: &CellOwner,
        drain: DrainSpec,
    ) -> Result<(), BackendError> {
        let handle = Handle {
            class: removal.class(),
            id: removal.id,
        };
        let selected = OsObject {
            id: removal.id,
            owner: owner.clone(),
            capability: removal.capability.clone(),
        };
        let unfinished = self
            .incomplete
            .get(&handle)
            .is_some_and(|unfinished| unfinished.operation.object() == selected);
        if !unfinished && !self.state.selects(&removal, owner) {
            return Err(BackendError::UnknownObject {
                class: handle.class.as_str(),
                key: removal.key(),
                owner: owner.clone(),
                id: handle.id,
            });
        }
        let stalled = self.stuck_handles.contains(&handle) || self.stalled_owners.contains(owner);
        if drain.policy == RevokePolicy::Graceful && stalled {
            return Err(BackendError::DrainTimedOut {
                handle,
                deadline_ms: drain.deadline_ms(),
            });
        }
        self.state.remove(&removal, owner);
        if let Some(detail) = self.failing_withdrawals.remove(&handle) {
            let failed = Unfinished {
                operation: selected.operation(),
                detail,
            };
            let error = failed.error();
            self.incomplete.insert(handle, failed);
            return Err(error);
        }
        self.incomplete.remove(&handle);
        self.grants.remove(&handle);
        self.stuck_handles.remove(&handle);
        Ok(())
    }

    fn plant(&mut self, object: OsObject) -> Result<(), BackendError> {
        checked(&object)?;
        let handle = object.operation().handle();
        if self.incomplete.contains_key(&handle) {
            return Err(identity_conflict(handle));
        }
        self.state.insert(object).map(|_| ())
    }
}

fn identity_conflict(handle: Handle) -> BackendError {
    BackendError::IdentityConflict {
        class: handle.class.as_str(),
        id: handle.id,
    }
}

fn checked(object: &OsObject) -> Result<(), BackendError> {
    object
        .capability
        .validate()
        .map_err(|error| BackendError::InvalidOperation {
            class: object.class().as_str(),
            id: object.id,
            error,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Capability;
    use crate::fixtures::{self, broker, mount, proxy_map, session_file, uds_socket};
    use crate::recipe::{BrokerLaunch, ProxyRecipe, RecipeError};
    use crate::snapshot::{EnforcementSnapshot, IncompleteEffect};
    use crate::universe::{CellId, CellOwner, GrantId, PluginId, UniverseClass};
    use std::time::Duration;

    fn grant(backend: &mut FakeBackend, plugin: &str, capability: Capability) -> LedgerEntry {
        grant_to(backend, &cell_owner("cell-1", plugin), capability)
    }

    fn capabilities() -> Vec<Capability> {
        vec![
            session_file("/skills/pr.md"),
            uds_socket("/run/ak/egressd.uds"),
            proxy_map("api.github.com", "splice"),
            broker("egressd"),
            mount("/secrets", "/workspace"),
        ]
    }

    #[test]
    fn identical_grants_are_independent_in_both_orders_and_policies() {
        for drain in [
            DrainSpec::graceful(Duration::from_millis(2)),
            DrainSpec::forcing(),
        ] {
            for reverse in [false, true] {
                for capability in capabilities() {
                    let mut backend = FakeBackend::new();
                    let first = grant(&mut backend, "owner", capability.clone());
                    let second = grant(&mut backend, "owner", capability);
                    assert_ne!(first.handle, second.handle);
                    assert_eq!(backend.snapshot_os_state().unwrap().state().len(), 2);
                    let (taken, survivor) = if reverse {
                        (second, first)
                    } else {
                        (first, second)
                    };
                    backend.revoke(taken.handle, drain).unwrap();
                    assert_eq!(
                        backend
                            .snapshot_os_state()
                            .unwrap()
                            .state()
                            .objects()
                            .collect::<Vec<_>>(),
                        vec![&survivor.object()]
                    );
                    backend.revoke(survivor.handle, drain).unwrap();
                    assert!(backend.snapshot_os_state().unwrap().is_empty());
                }
            }
        }
    }

    #[test]
    fn colliding_mount_targets_and_proxy_hosts_keep_complete_resources() {
        let mut backend = FakeBackend::new();
        let first_mount = grant(&mut backend, "owner", mount("/secrets", "/workspace"));
        let second_mount = grant(&mut backend, "owner", mount("/code", "/workspace"));
        let first_route = grant(&mut backend, "owner", proxy_map("api.github.com", "audit"));
        let second_route = grant(&mut backend, "owner", proxy_map("api.github.com", "deploy"));
        assert_eq!(backend.snapshot_os_state().unwrap().state().len(), 4);
        backend
            .revoke(second_mount.handle, DrainSpec::forcing())
            .unwrap();
        backend
            .revoke(first_route.handle, DrainSpec::forcing())
            .unwrap();
        let standing = backend.snapshot_os_state().unwrap();
        assert_eq!(
            standing.state().objects().cloned().collect::<Vec<_>>(),
            vec![second_route.object(), first_mount.object()]
        );
    }

    #[test]
    fn live_broker_revoke_preserves_equal_planted_residue() {
        let capability = broker("egressd");
        let residue = OsObject {
            id: GrantId::new(),
            owner: cell_owner("cell-1", "network"),
            capability: capability.clone(),
        };
        let mut backend = FakeBackend::new();
        backend.plant_residue(residue.clone()).unwrap();
        let live = grant(&mut backend, "network", capability);
        backend.revoke(live.handle, DrainSpec::forcing()).unwrap();
        assert_eq!(
            backend
                .snapshot_os_state()
                .unwrap()
                .state()
                .objects()
                .collect::<Vec<_>>(),
            vec![&residue]
        );
        backend
            .apply_removal(
                UniverseRemoval {
                    id: residue.id,
                    capability: residue.capability.clone(),
                },
                &residue.owner,
                DrainSpec::forcing(),
            )
            .unwrap();
        assert!(backend.snapshot_os_state().unwrap().is_empty());
    }

    #[test]
    fn exact_removal_refusals_preserve_neighbouring_holdings() {
        let capability = proxy_map("api.github.com", "splice");
        let mut backend = FakeBackend::new();
        let first = grant(&mut backend, "deploy", capability.clone());
        let second = grant(&mut backend, "deploy", capability.clone());
        let before = backend.snapshot_os_state().unwrap();
        let wrong_capability = UniverseRemoval {
            id: first.handle.id,
            capability: proxy_map("api.github.com", "other"),
        };
        for (removal, owner) in [
            (first.removal(), cell_owner("cell-1", "audit")),
            (wrong_capability, cell_owner("cell-1", "deploy")),
            (
                UniverseRemoval {
                    id: GrantId::new(),
                    capability,
                },
                cell_owner("cell-1", "deploy"),
            ),
        ] {
            assert!(matches!(
                backend.apply_removal(removal, &owner, DrainSpec::forcing()),
                Err(BackendError::UnknownObject { .. })
            ));
            assert_eq!(backend.snapshot_os_state().unwrap(), before);
        }
        backend.revoke(first.handle, DrainSpec::forcing()).unwrap();
        assert!(matches!(
            backend.revoke(first.handle, DrainSpec::forcing()),
            Err(BackendError::UnknownHandle { .. })
        ));
        assert_eq!(
            backend
                .snapshot_os_state()
                .unwrap()
                .state()
                .objects()
                .collect::<Vec<_>>(),
            vec![&second.object()]
        );
    }

    #[test]
    fn graceful_timeout_preserves_peers_and_force_takes_only_requested_grant() {
        let capability = uds_socket("/run/ak/egressd.uds");
        let mut backend = FakeBackend::new();
        let stuck = grant(&mut backend, "network", capability.clone());
        let peer = grant(&mut backend, "network", capability);
        backend.mark_stuck(stuck.handle);
        let before = backend.snapshot_os_state().unwrap();
        assert!(matches!(
            backend.revoke(stuck.handle, DrainSpec::graceful(Duration::from_millis(2))),
            Err(BackendError::DrainTimedOut { .. })
        ));
        assert_eq!(backend.snapshot_os_state().unwrap(), before);
        backend.revoke(stuck.handle, DrainSpec::forcing()).unwrap();
        assert_eq!(
            backend
                .snapshot_os_state()
                .unwrap()
                .state()
                .objects()
                .collect::<Vec<_>>(),
            vec![&peer.object()]
        );
    }

    #[test]
    fn recorded_apply_is_idempotent_and_changed_payload_conflicts() {
        let id = GrantId::new();
        let op = fixtures::op(
            id,
            cell_owner("cell-1", "github-pr"),
            session_file("/skills/pr.md"),
        );
        let mut backend = FakeBackend::new();
        backend.apply(op.clone()).unwrap();
        backend.apply(op.clone()).unwrap();
        assert_eq!(backend.snapshot_os_state().unwrap().state().len(), 1);
        let peer = fixtures::op(
            GrantId::new(),
            cell_owner("cell-1", "github-pr"),
            session_file("/skills/pr.md"),
        );
        backend.apply(peer).unwrap();
        assert_eq!(backend.snapshot_os_state().unwrap().state().len(), 2);
        let conflict = fixtures::op(
            id,
            cell_owner("cell-1", "audit"),
            session_file("/skills/pr.md"),
        );
        let before = backend.snapshot_os_state().unwrap();
        assert!(matches!(
            backend.apply(conflict),
            Err(BackendError::IdentityConflict { .. })
        ));
        assert_eq!(backend.snapshot_os_state().unwrap(), before);
    }

    #[test]
    fn direct_removal_retires_handle_and_reapply_does_not_revive_it() {
        let mut backend = FakeBackend::new();
        let entry = grant(&mut backend, "github-pr", session_file("/skills/pr.md"));
        backend
            .apply_removal(entry.removal(), &entry.owner, DrainSpec::forcing())
            .unwrap();
        backend
            .apply(fixtures::op(
                entry.handle.id,
                entry.owner.clone(),
                session_file("/skills/pr.md"),
            ))
            .unwrap();
        assert!(matches!(
            backend.revoke(entry.handle, DrainSpec::forcing()),
            Err(BackendError::UnknownHandle { .. })
        ));
        assert_eq!(backend.snapshot_os_state().unwrap().state().len(), 1);
        let fresh = grant(&mut backend, "github-pr", entry.capability);
        assert_ne!(fresh.handle, entry.handle);
        assert_eq!(backend.snapshot_os_state().unwrap().state().len(), 2);
    }

    fn cell_owner(cell: &str, plugin: &str) -> CellOwner {
        CellOwner {
            cell: CellId::from(cell),
            plugin: PluginId::from(plugin),
        }
    }

    fn mount_by(owner: &CellOwner) -> UniverseOp {
        fixtures::op(GrantId::new(), owner.clone(), mount("/code", "/workspace"))
    }

    fn grant_to(
        backend: &mut FakeBackend,
        owner: &CellOwner,
        capability: Capability,
    ) -> LedgerEntry {
        backend
            .grant(
                fixtures::op(GrantId::new(), owner.clone(), capability),
                GrantKind::Hot,
            )
            .unwrap()
    }

    #[test]
    fn graceful_removal_of_a_stuck_grant_keeps_the_holding_its_record_and_its_peer() {
        let network = cell_owner("cell-1", "network");
        let capability = uds_socket("/run/ak/egressd.uds");
        let mut backend = FakeBackend::new();
        let stuck = grant_to(&mut backend, &network, capability.clone());
        let peer = grant_to(&mut backend, &network, capability);
        backend.mark_stuck(stuck.handle);
        let before = backend.snapshot_os_state().unwrap();
        assert_eq!(
            backend
                .apply_removal(
                    stuck.removal(),
                    &network,
                    DrainSpec::graceful(Duration::from_millis(7))
                )
                .unwrap_err(),
            BackendError::DrainTimedOut {
                handle: stuck.handle,
                deadline_ms: 7,
            }
        );
        assert_eq!(backend.snapshot_os_state().unwrap(), before);
        assert_eq!(backend.peek_entry(stuck.handle).unwrap(), stuck);
        backend
            .apply_removal(stuck.removal(), &network, DrainSpec::forcing())
            .unwrap();
        assert_eq!(
            backend
                .snapshot_os_state()
                .unwrap()
                .state()
                .objects()
                .collect::<Vec<_>>(),
            vec![&peer.object()]
        );
        assert_eq!(
            backend.peek_entry(stuck.handle).unwrap_err(),
            BackendError::UnknownHandle {
                handle: stuck.handle
            }
        );
        assert_eq!(backend.peek_entry(peer.handle).unwrap(), peer);
    }

    #[test]
    fn a_stuck_applied_address_times_out_graceful_removal_until_forced() {
        let workspace = cell_owner("cell-1", "workspace");
        let op = mount_by(&workspace);
        let address = op.handle();
        let mut backend = FakeBackend::new();
        backend.apply(op.clone()).unwrap();
        backend.mark_stuck(address);
        let before = backend.snapshot_os_state().unwrap();
        assert_eq!(
            backend
                .apply_removal(
                    op.removal(),
                    &workspace,
                    DrainSpec::graceful(Duration::from_millis(3))
                )
                .unwrap_err(),
            BackendError::DrainTimedOut {
                handle: address,
                deadline_ms: 3,
            }
        );
        assert_eq!(backend.snapshot_os_state().unwrap(), before);
        backend
            .apply_removal(op.removal(), &workspace, DrainSpec::forcing())
            .unwrap();
        assert!(backend.snapshot_os_state().unwrap().is_empty());
    }

    #[test]
    fn a_forced_removal_clears_the_stuck_mark_so_a_later_holding_drains() {
        let workspace = cell_owner("cell-1", "workspace");
        let op = mount_by(&workspace);
        let mut backend = FakeBackend::new();
        backend.apply(op.clone()).unwrap();
        backend.mark_stuck(op.handle());
        backend
            .apply_removal(op.removal(), &workspace, DrainSpec::forcing())
            .unwrap();
        backend.apply(op.clone()).unwrap();
        backend
            .apply_removal(
                op.removal(),
                &workspace,
                DrainSpec::graceful(Duration::from_millis(3)),
            )
            .unwrap();
        assert!(backend.snapshot_os_state().unwrap().is_empty());
    }

    #[test]
    fn a_stalled_owner_times_out_every_graceful_withdrawal_but_not_its_peers() {
        let stalled = cell_owner("cell-1", "stalled");
        let peer_owner = cell_owner("cell-1", "network");
        let same_plugin_elsewhere = cell_owner("cell-2", "stalled");
        let capability = proxy_map("api.github.com", "splice");
        let mut backend = FakeBackend::new();
        backend.stall_graceful_drains_for_owner(stalled.clone());
        let granted = grant_to(&mut backend, &stalled, capability.clone());
        let op = fixtures::op(
            GrantId::new(),
            stalled.clone(),
            proxy_map("api.github.com", "splice"),
        );
        backend.apply(op.clone()).unwrap();
        let peer = grant_to(&mut backend, &peer_owner, capability.clone());
        let elsewhere = grant_to(&mut backend, &same_plugin_elsewhere, capability);
        let before = backend.snapshot_os_state().unwrap();
        let graceful = DrainSpec::graceful(Duration::from_millis(9));
        assert_eq!(
            backend.revoke(granted.handle, graceful).unwrap_err(),
            BackendError::DrainTimedOut {
                handle: granted.handle,
                deadline_ms: 9,
            }
        );
        assert_eq!(
            backend
                .apply_removal(op.removal(), &stalled, graceful)
                .unwrap_err(),
            BackendError::DrainTimedOut {
                handle: op.handle(),
                deadline_ms: 9,
            }
        );
        assert_eq!(backend.snapshot_os_state().unwrap(), before);
        assert_eq!(backend.peek_entry(granted.handle).unwrap(), granted);
        assert_eq!(backend.revoke(peer.handle, graceful).unwrap(), peer);
        assert_eq!(
            backend.revoke(elsewhere.handle, graceful).unwrap(),
            elsewhere
        );
        assert_eq!(
            backend
                .revoke(granted.handle, DrainSpec::forcing())
                .unwrap(),
            granted
        );
        assert_eq!(
            backend
                .snapshot_os_state()
                .unwrap()
                .state()
                .objects()
                .collect::<Vec<_>>(),
            vec![&op.object()]
        );
        backend
            .apply_removal(op.removal(), &stalled, DrainSpec::forcing())
            .unwrap();
        assert!(backend.snapshot_os_state().unwrap().is_empty());
    }

    #[test]
    fn a_zero_graceful_deadline_checks_once_and_never_forces() {
        let workspace = cell_owner("cell-1", "workspace");
        let op = mount_by(&workspace);
        let mut backend = FakeBackend::new();
        let stuck = grant_to(&mut backend, &workspace, session_file("/skills/pr.md"));
        backend.apply(op.clone()).unwrap();
        backend.mark_stuck(op.handle());
        backend.mark_stuck(stuck.handle);
        let before = backend.snapshot_os_state().unwrap();
        let zero = DrainSpec::graceful(Duration::ZERO);
        assert_eq!(
            backend
                .apply_removal(op.removal(), &workspace, zero)
                .unwrap_err(),
            BackendError::DrainTimedOut {
                handle: op.handle(),
                deadline_ms: 0,
            }
        );
        assert_eq!(
            backend.revoke(stuck.handle, zero).unwrap_err(),
            BackendError::DrainTimedOut {
                handle: stuck.handle,
                deadline_ms: 0,
            }
        );
        assert_eq!(backend.snapshot_os_state().unwrap(), before);
        assert_eq!(backend.peek_entry(stuck.handle).unwrap(), stuck);
        let drained = grant_to(&mut backend, &workspace, session_file("/skills/drained.md"));
        assert_eq!(backend.revoke(drained.handle, zero).unwrap(), drained);
        assert_eq!(backend.snapshot_os_state().unwrap(), before);
    }

    #[test]
    fn a_timeout_reports_its_deadline_in_whole_milliseconds_rounded_up() {
        let workspace = cell_owner("cell-1", "workspace");
        let op = mount_by(&workspace);
        let mut backend = FakeBackend::new();
        backend.apply(op.clone()).unwrap();
        backend.mark_stuck(op.handle());
        for (deadline, deadline_ms) in [
            (Duration::ZERO, 0),
            (Duration::from_nanos(1), 1),
            (Duration::from_micros(999), 1),
            (Duration::from_millis(1), 1),
            (Duration::from_micros(1_001), 2),
            (Duration::from_millis(7), 7),
        ] {
            assert_eq!(
                backend
                    .apply_removal(op.removal(), &workspace, DrainSpec::graceful(deadline))
                    .unwrap_err(),
                BackendError::DrainTimedOut {
                    handle: op.handle(),
                    deadline_ms,
                },
                "{deadline:?}"
            );
        }
    }

    #[test]
    fn exact_removal_resolves_owner_and_capability_before_draining() {
        let stalled = cell_owner("cell-1", "workspace");
        let op = mount_by(&stalled);
        let mut backend = FakeBackend::new();
        backend.stall_graceful_drains_for_owner(stalled.clone());
        backend.apply(op.clone()).unwrap();
        backend.mark_stuck(op.handle());
        let before = backend.snapshot_os_state().unwrap();
        let other_source = UniverseRemoval {
            id: op.id(),
            capability: mount("/secrets", "/workspace"),
        };
        for drain in [
            DrainSpec::graceful(Duration::from_millis(5)),
            DrainSpec::forcing(),
        ] {
            for (removal, owner) in [
                (op.removal(), cell_owner("cell-2", "workspace")),
                (op.removal(), cell_owner("cell-1", "audit")),
                (other_source.clone(), stalled.clone()),
            ] {
                assert_eq!(
                    backend
                        .apply_removal(removal.clone(), &owner, drain)
                        .unwrap_err(),
                    BackendError::UnknownObject {
                        class: "mount",
                        key: "/workspace".to_string(),
                        owner,
                        id: op.id(),
                    }
                );
                assert_eq!(backend.snapshot_os_state().unwrap(), before);
            }
        }
    }

    #[test]
    fn apply_faults_are_keyed_by_the_cell_qualified_owner() {
        let doomed = cell_owner("cell-1", "doomed");
        let mut backend = FakeBackend::new();
        backend.fail_before_effect_for_owner(doomed.clone(), "injected refusal");
        let elsewhere = fixtures::op(
            GrantId::new(),
            cell_owner("cell-2", "doomed"),
            session_file("/skills/pr.md"),
        );
        backend.apply(elsewhere.clone()).unwrap();
        let before = backend.snapshot_os_state().unwrap();
        assert_eq!(
            backend
                .apply(fixtures::op(
                    GrantId::new(),
                    doomed,
                    session_file("/skills/pr.md")
                ))
                .unwrap_err(),
            BackendError::Fault("injected refusal".to_string())
        );
        assert_eq!(backend.snapshot_os_state().unwrap(), before);
        assert_eq!(
            before.state().objects().collect::<Vec<_>>(),
            vec![&elsewhere.object()]
        );
    }

    #[test]
    fn unknown_handle_is_an_exact_address_error() {
        let mut backend = FakeBackend::new();
        let handle = Handle {
            class: UniverseClass::BrokerPid,
            id: GrantId::new(),
        };
        assert_eq!(
            backend
                .revoke(handle, DrainSpec::graceful(Duration::from_millis(1)))
                .unwrap_err(),
            BackendError::UnknownHandle { handle }
        );
    }

    fn invalid_operations(owner: &CellOwner) -> Vec<(UniverseOp, RecipeError)> {
        vec![
            (
                fixtures::op(GrantId::new(), owner.clone(), session_file("skills/pr.md")),
                RecipeError::NotAbsolute {
                    field: "path",
                    value: "skills/pr.md".to_string(),
                },
            ),
            (
                fixtures::op(GrantId::new(), owner.clone(), uds_socket("run/egressd.uds")),
                RecipeError::NotAbsolute {
                    field: "path",
                    value: "run/egressd.uds".to_string(),
                },
            ),
            (
                fixtures::op(GrantId::new(), owner.clone(), mount("/code", "/workspace/")),
                RecipeError::NotCanonical {
                    field: "target",
                    value: "/workspace/".to_string(),
                },
            ),
            (
                fixtures::op(GrantId::new(), owner.clone(), broker("egress\0d")),
                RecipeError::ContainsNul { field: "name" },
            ),
            (
                fixtures::op(
                    GrantId::new(),
                    owner.clone(),
                    Capability::ProxyMap {
                        host: "api.github.com".to_string(),
                        route: "splice".to_string(),
                        recipe: ProxyRecipe {
                            port: 0,
                            ..fixtures::proxy_recipe()
                        },
                    },
                ),
                RecipeError::ZeroPort,
            ),
            (
                fixtures::op(
                    GrantId::new(),
                    owner.clone(),
                    Capability::Broker {
                        name: "egressd".to_string(),
                        launch: BrokerLaunch {
                            data_socket: fixtures::launch().control_socket,
                            ..fixtures::launch()
                        },
                    },
                ),
                RecipeError::SharedEndpoint {
                    path: fixtures::launch().control_socket,
                },
            ),
        ]
    }

    #[test]
    fn apply_and_plant_refuse_an_invalid_value_before_any_fault_or_change() {
        let owner = cell_owner("cell-1", "github-pr");
        let mut backend = FakeBackend::new();
        backend.apply(mount_by(&owner)).unwrap();
        backend.fail_before_effect_for_owner(owner.clone(), "injected refusal");
        let before = backend.snapshot_os_state().unwrap();
        for (op, error) in invalid_operations(&owner) {
            let rule = error.to_string();
            let expected = BackendError::InvalidOperation {
                class: op.class().as_str(),
                id: op.id(),
                error,
            };
            assert_eq!(
                expected.to_string(),
                format!(
                    "invalid {} operation {}: {rule}",
                    op.class().as_str(),
                    op.id()
                )
            );
            assert_eq!(backend.apply(op.clone()).unwrap_err(), expected);
            assert_eq!(backend.snapshot_os_state().unwrap(), before);
            assert_eq!(backend.plant(op.object()).unwrap_err(), expected);
            assert_eq!(backend.snapshot_os_state().unwrap(), before);
        }
        assert_eq!(
            backend.apply(mount_by(&owner)).unwrap_err(),
            BackendError::Fault("injected refusal".to_string())
        );
        assert_eq!(backend.snapshot_os_state().unwrap(), before);
    }

    fn issued(op: &UniverseOp, kind: GrantKind) -> LedgerEntry {
        LedgerEntry {
            handle: op.handle(),
            owner: op.owner().clone(),
            capability: op.capability(),
            kind,
        }
    }

    fn conflict(op: &UniverseOp) -> BackendError {
        BackendError::IdentityConflict {
            class: op.class().as_str(),
            id: op.id(),
        }
    }

    #[test]
    fn a_grant_issues_exactly_the_supplied_address_owner_capability_and_kind() {
        let owner = cell_owner("cell-1", "github-pr");
        for kind in [GrantKind::Hot, GrantKind::GenerationBound] {
            for capability in capabilities() {
                let op = fixtures::op(GrantId::new(), owner.clone(), capability);
                let mut backend = FakeBackend::new();
                let entry = backend.grant(op.clone(), kind).unwrap();
                assert_eq!(entry, issued(&op, kind));
                assert_eq!(
                    backend
                        .snapshot_os_state()
                        .unwrap()
                        .state()
                        .objects()
                        .collect::<Vec<_>>(),
                    vec![&op.object()]
                );
                assert_eq!(backend.peek_entry(op.handle()).unwrap(), entry);
            }
        }
    }

    #[test]
    fn a_retry_of_a_standing_grant_returns_the_original_entry_and_changes_nothing() {
        let owner = cell_owner("cell-1", "network");
        for kind in [GrantKind::Hot, GrantKind::GenerationBound] {
            for capability in capabilities() {
                let op = fixtures::op(GrantId::new(), owner.clone(), capability);
                let mut backend = FakeBackend::new();
                let first = backend.grant(op.clone(), kind).unwrap();
                let before = backend.snapshot_os_state().unwrap();
                assert_eq!(backend.grant(op.clone(), kind).unwrap(), first);
                assert_eq!(backend.snapshot_os_state().unwrap(), before);
                assert_eq!(
                    backend.revoke(op.handle(), DrainSpec::forcing()).unwrap(),
                    first
                );
                assert!(backend.snapshot_os_state().unwrap().is_empty());
            }
        }
    }

    #[test]
    fn a_changed_kind_owner_or_capability_at_an_issued_address_conflicts() {
        let owner = cell_owner("cell-1", "network");
        let op = fixtures::op(GrantId::new(), owner.clone(), mount("/code", "/workspace"));
        let mut backend = FakeBackend::new();
        let original = backend.grant(op.clone(), GrantKind::Hot).unwrap();
        let before = backend.snapshot_os_state().unwrap();
        assert_eq!(
            backend.grant(op.clone(), GrantKind::GenerationBound),
            Err(conflict(&op))
        );
        for changed in [
            fixtures::op(
                op.id(),
                cell_owner("cell-2", "network"),
                mount("/code", "/workspace"),
            ),
            fixtures::op(
                op.id(),
                cell_owner("cell-1", "audit"),
                mount("/code", "/workspace"),
            ),
            fixtures::op(op.id(), owner.clone(), mount("/secrets", "/workspace")),
        ] {
            assert_eq!(
                backend.grant(changed.clone(), GrantKind::Hot),
                Err(conflict(&changed))
            );
        }
        assert_eq!(backend.snapshot_os_state().unwrap(), before);
        assert_eq!(backend.peek_entry(op.handle()).unwrap(), original);
        assert_eq!(
            backend.revoke(op.handle(), DrainSpec::forcing()).unwrap(),
            original
        );
    }

    fn another_of_its_class(capability: &Capability) -> Capability {
        match capability {
            Capability::SessionFile { .. } => session_file("/skills/other.md"),
            Capability::UdsSocket { .. } => uds_socket("/run/ak/other.uds"),
            Capability::ProxyMap { .. } => proxy_map("other.github.com", "splice"),
            Capability::Broker { .. } => broker("other"),
            Capability::Mount { .. } => mount("/other", "/workspace"),
        }
    }

    #[test]
    fn a_grant_never_promotes_or_replaces_an_applied_or_planted_object() {
        let owner = cell_owner("cell-1", "workspace");
        for capability in capabilities() {
            let applied = fixtures::op(GrantId::new(), owner.clone(), capability.clone());
            let planted = OsObject {
                id: GrantId::new(),
                owner: owner.clone(),
                capability: capability.clone(),
            };
            let mut backend = FakeBackend::new();
            backend.apply(applied.clone()).unwrap();
            backend.plant(planted.clone()).unwrap();
            let before = backend.snapshot_os_state().unwrap();
            for standing in [applied.clone(), planted.operation()] {
                let requests = [
                    standing.clone(),
                    fixtures::op(
                        standing.id(),
                        cell_owner("cell-1", "other"),
                        capability.clone(),
                    ),
                    fixtures::op(
                        standing.id(),
                        owner.clone(),
                        another_of_its_class(&capability),
                    ),
                ];
                for op in requests {
                    for kind in [GrantKind::Hot, GrantKind::GenerationBound] {
                        assert_eq!(
                            backend.grant(op.clone(), kind),
                            Err(conflict(&op)),
                            "{op:?}"
                        );
                        assert_eq!(backend.snapshot_os_state().unwrap(), before);
                    }
                    assert_eq!(
                        backend.revoke(op.handle(), DrainSpec::forcing()),
                        Err(BackendError::UnknownHandle {
                            handle: op.handle()
                        }),
                        "{op:?}"
                    );
                }
                assert_eq!(backend.snapshot_os_state().unwrap(), before);
            }
        }
    }

    #[test]
    fn applying_or_planting_a_live_grants_own_object_changes_nothing() {
        let owner = cell_owner("cell-1", "network");
        for capability in capabilities() {
            let op = fixtures::op(GrantId::new(), owner.clone(), capability);
            let mut backend = FakeBackend::new();
            let entry = backend
                .grant(op.clone(), GrantKind::GenerationBound)
                .unwrap();
            let before = backend.snapshot_os_state().unwrap();
            backend.apply(op.clone()).unwrap();
            backend.plant(op.object()).unwrap();
            assert_eq!(backend.snapshot_os_state().unwrap(), before);
            assert_eq!(
                backend.revoke(op.handle(), DrainSpec::forcing()).unwrap(),
                entry
            );
            assert!(backend.snapshot_os_state().unwrap().is_empty());
        }
    }

    #[test]
    fn grant_refuses_an_invalid_operation_before_a_fault_or_any_change() {
        let owner = cell_owner("cell-1", "github-pr");
        let mut backend = FakeBackend::new();
        let standing: Vec<LedgerEntry> = capabilities()
            .into_iter()
            .map(|capability| grant_to(&mut backend, &owner, capability))
            .collect();
        backend.fail_before_effect_for_owner(owner.clone(), "injected refusal");
        let before = backend.snapshot_os_state().unwrap();
        for (fresh, error) in invalid_operations(&owner) {
            let issued = standing
                .iter()
                .find(|entry| entry.handle.class == fresh.class())
                .unwrap();
            let at_the_issued_address = OsObject {
                id: issued.handle.id,
                ..fresh.object()
            }
            .operation();
            for op in [fresh.clone(), at_the_issued_address] {
                assert_eq!(
                    backend.grant(op.clone(), GrantKind::Hot),
                    Err(BackendError::InvalidOperation {
                        class: op.class().as_str(),
                        id: op.id(),
                        error: error.clone(),
                    }),
                    "{op:?}"
                );
                assert_eq!(backend.snapshot_os_state().unwrap(), before);
            }
            assert_eq!(
                backend.revoke(fresh.handle(), DrainSpec::forcing()),
                Err(BackendError::UnknownHandle {
                    handle: fresh.handle()
                })
            );
        }
        for entry in standing {
            assert_eq!(backend.peek_entry(entry.handle).unwrap(), entry);
        }
    }

    #[test]
    fn a_pre_effect_fault_refuses_grant_and_apply_for_exactly_that_owner() {
        let doomed = cell_owner("cell-1", "doomed");
        let elsewhere = cell_owner("cell-2", "doomed");
        let mut backend = FakeBackend::new();
        let standing = backend.grant(mount_by(&doomed), GrantKind::Hot).unwrap();
        backend.fail_before_effect_for_owner(doomed.clone(), "injected refusal");
        let before = backend.snapshot_os_state().unwrap();
        let refused = BackendError::Fault("injected refusal".to_string());
        for capability in capabilities() {
            let op = fixtures::op(GrantId::new(), doomed.clone(), capability.clone());
            assert_eq!(
                backend.grant(op.clone(), GrantKind::Hot),
                Err(refused.clone())
            );
            assert_eq!(backend.apply(op.clone()), Err(refused.clone()));
            assert_eq!(backend.snapshot_os_state().unwrap(), before);
            assert_eq!(
                backend.revoke(op.handle(), DrainSpec::forcing()),
                Err(BackendError::UnknownHandle {
                    handle: op.handle()
                })
            );
            let peer = fixtures::op(GrantId::new(), elsewhere.clone(), capability);
            assert_eq!(
                backend.grant(peer.clone(), GrantKind::Hot).unwrap(),
                issued(&peer, GrantKind::Hot)
            );
            backend.revoke(peer.handle(), DrainSpec::forcing()).unwrap();
        }
        assert_eq!(backend.snapshot_os_state().unwrap(), before);
        assert_eq!(backend.peek_entry(standing.handle).unwrap(), standing);
        assert_eq!(
            backend
                .revoke(standing.handle, DrainSpec::forcing())
                .unwrap(),
            standing
        );
    }

    fn standing(objects: &[OsObject]) -> OsState {
        let mut state = OsState::new();
        for object in objects {
            state.insert(object.clone()).unwrap();
        }
        state
    }

    #[test]
    fn a_snapshot_reports_a_record_for_each_live_grant_and_none_for_applied_or_planted_objects() {
        let owner = cell_owner("cell-1", "github-pr");
        let mut backend = FakeBackend::new();
        let hot = grant_to(&mut backend, &owner, session_file("/skills/pr.md"));
        let bound = backend
            .grant(
                fixtures::op(GrantId::new(), owner.clone(), broker("egressd")),
                GrantKind::GenerationBound,
            )
            .unwrap();
        let applied = mount_by(&owner);
        backend.apply(applied.clone()).unwrap();
        let planted = OsObject {
            id: GrantId::new(),
            owner: owner.clone(),
            capability: proxy_map("api.github.com", "splice"),
        };
        backend.plant(planted.clone()).unwrap();
        assert_eq!(
            backend.snapshot_os_state(),
            Ok(EnforcementSnapshot::new(
                standing(&[
                    hot.object(),
                    bound.object(),
                    applied.object(),
                    planted.clone()
                ]),
                vec![hot.clone(), bound.clone()],
                Vec::new(),
            )
            .unwrap())
        );
        backend.revoke(hot.handle, DrainSpec::forcing()).unwrap();
        backend
            .apply_removal(bound.removal(), &bound.owner, DrainSpec::forcing())
            .unwrap();
        assert_eq!(
            backend.snapshot_os_state(),
            Ok(EnforcementSnapshot::new(
                standing(&[applied.object(), planted]),
                Vec::new(),
                Vec::new()
            )
            .unwrap())
        );
    }

    #[test]
    fn an_armed_observation_fails_once_with_no_other_effect_and_then_observes_normally() {
        let owner = cell_owner("cell-1", "network");
        let mut backend = FakeBackend::new();
        let entry = grant_to(&mut backend, &owner, uds_socket("/run/ak/egressd.uds"));
        backend.fail_next_observation("observer unreachable");
        let applied = mount_by(&owner);
        backend.apply(applied.clone()).unwrap();
        let failed = backend.snapshot_os_state().unwrap_err();
        assert_eq!(
            failed,
            BackendError::Fault("observer unreachable".to_string())
        );
        assert_eq!(failed.to_string(), "backend fault: observer unreachable");
        let complete = EnforcementSnapshot::new(
            standing(&[entry.object(), applied.object()]),
            vec![entry.clone()],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(backend.snapshot_os_state(), Ok(complete.clone()));
        assert_eq!(backend.snapshot_os_state(), Ok(complete));
        assert_eq!(backend.peek_entry(entry.handle).unwrap(), entry);
    }

    const PROBE: &str = "readiness probe failed";

    fn incomplete(op: &UniverseOp, detail: &str) -> BackendError {
        BackendError::IncompleteEffect {
            class: op.class().as_str(),
            id: op.id(),
            detail: detail.to_string(),
        }
    }

    fn account(
        objects: &[OsObject],
        grants: &[LedgerEntry],
        unfinished: &[&UniverseOp],
    ) -> EnforcementSnapshot {
        let incomplete = unfinished
            .iter()
            .map(|op| IncompleteEffect {
                operation: (*op).clone(),
            })
            .collect();
        EnforcementSnapshot::new(standing(objects), grants.to_vec(), incomplete).unwrap()
    }

    #[derive(Debug, Clone, Copy)]
    enum Entry {
        Grant,
        Apply,
    }

    fn create(
        backend: &mut FakeBackend,
        entry: Entry,
        op: &UniverseOp,
    ) -> Result<(), BackendError> {
        match entry {
            Entry::Grant => backend.grant(op.clone(), GrantKind::Hot).map(|_| ()),
            Entry::Apply => backend.apply(op.clone()),
        }
    }

    fn unknown_object(op: &UniverseOp) -> BackendError {
        BackendError::UnknownObject {
            class: op.class().as_str(),
            key: op.capability().key(),
            owner: op.owner().clone(),
            id: op.id(),
        }
    }

    #[test]
    fn a_failure_after_the_first_effect_reports_that_operation_until_its_exact_removal() {
        let doomed = cell_owner("cell-1", "doomed");
        let network = cell_owner("cell-1", "network");
        for capability in capabilities() {
            for entry in [Entry::Grant, Entry::Apply] {
                let mut backend = FakeBackend::new();
                backend.fail_after_effect_for_owner(doomed.clone(), PROBE);
                let peer = grant_to(&mut backend, &network, capability.clone());
                let first = backend.snapshot_os_state().unwrap();
                let op = fixtures::op(GrantId::new(), doomed.clone(), capability.clone());
                assert_eq!(
                    create(&mut backend, entry, &op),
                    Err(incomplete(&op, PROBE))
                );
                let failed = account(&[peer.object()], std::slice::from_ref(&peer), &[&op]);
                assert_eq!(backend.snapshot_os_state(), Ok(failed.clone()));
                for retry in [Entry::Grant, Entry::Apply] {
                    assert_eq!(
                        create(&mut backend, retry, &op),
                        Err(incomplete(&op, PROBE)),
                        "{entry:?} then {retry:?}"
                    );
                }
                for other in [
                    fixtures::op(op.id(), network.clone(), capability.clone()),
                    fixtures::op(op.id(), doomed.clone(), another_of_its_class(&capability)),
                ] {
                    for retry in [Entry::Grant, Entry::Apply] {
                        assert_eq!(create(&mut backend, retry, &other), Err(conflict(&other)));
                    }
                    assert_eq!(backend.plant(other.object()), Err(conflict(&other)));
                    assert_eq!(
                        backend.apply_removal(other.removal(), other.owner(), DrainSpec::forcing()),
                        Err(unknown_object(&other))
                    );
                }
                assert_eq!(backend.plant(op.object()), Err(conflict(&op)));
                assert_eq!(
                    backend.revoke(op.handle(), DrainSpec::forcing()),
                    Err(BackendError::UnknownHandle {
                        handle: op.handle()
                    })
                );
                assert_eq!(backend.snapshot_os_state(), Ok(failed));
                backend
                    .apply_removal(op.removal(), &doomed, DrainSpec::forcing())
                    .unwrap();
                assert_eq!(backend.snapshot_os_state(), Ok(first));
            }
        }
    }

    #[test]
    fn a_post_effect_fault_follows_validation_and_a_pre_effect_refusal_and_keys_on_the_cell() {
        let doomed = cell_owner("cell-1", "doomed");
        let mut backend = FakeBackend::new();
        backend.fail_after_effect_for_owner(doomed.clone(), PROBE);
        for entry in [Entry::Grant, Entry::Apply] {
            let elsewhere = fixtures::op(
                GrantId::new(),
                cell_owner("cell-2", "doomed"),
                broker("egressd"),
            );
            create(&mut backend, entry, &elsewhere).unwrap();
            for (invalid, error) in invalid_operations(&doomed) {
                let before = backend.snapshot_os_state().unwrap();
                assert_eq!(
                    create(&mut backend, entry, &invalid),
                    Err(BackendError::InvalidOperation {
                        class: invalid.class().as_str(),
                        id: invalid.id(),
                        error,
                    })
                );
                assert_eq!(backend.snapshot_os_state().unwrap(), before);
            }
        }
        backend.fail_before_effect_for_owner(doomed.clone(), "injected refusal");
        let before = backend.snapshot_os_state().unwrap();
        for entry in [Entry::Grant, Entry::Apply] {
            assert_eq!(
                create(&mut backend, entry, &mount_by(&doomed)),
                Err(BackendError::Fault("injected refusal".to_string()))
            );
        }
        assert_eq!(backend.snapshot_os_state().unwrap(), before);
    }

    #[test]
    fn an_incomplete_effect_keeps_its_marker_through_a_timeout_and_a_failed_cleanup() {
        let doomed = cell_owner("cell-1", "doomed");
        for entry in [Entry::Grant, Entry::Apply] {
            let mut backend = FakeBackend::new();
            backend.fail_after_effect_for_owner(doomed.clone(), PROBE);
            backend.stall_graceful_drains_for_owner(doomed.clone());
            let op = mount_by(&doomed);
            assert_eq!(
                create(&mut backend, entry, &op),
                Err(incomplete(&op, PROBE))
            );
            let failed = account(&[], &[], &[&op]);
            assert_eq!(backend.snapshot_os_state(), Ok(failed.clone()));
            assert_eq!(
                backend.apply_removal(
                    op.removal(),
                    &doomed,
                    DrainSpec::graceful(Duration::from_millis(4))
                ),
                Err(BackendError::DrainTimedOut {
                    handle: op.handle(),
                    deadline_ms: 4,
                })
            );
            assert_eq!(backend.snapshot_os_state(), Ok(failed.clone()));
            backend.fail_next_withdrawal(op.handle(), "unmount busy");
            assert_eq!(
                backend.apply_removal(op.removal(), &doomed, DrainSpec::forcing()),
                Err(incomplete(&op, "unmount busy"))
            );
            assert_eq!(backend.snapshot_os_state(), Ok(failed));
            backend
                .apply_removal(op.removal(), &doomed, DrainSpec::forcing())
                .unwrap();
            assert!(backend.snapshot_os_state().unwrap().is_empty());
        }
    }

    #[test]
    fn a_withdrawal_that_fails_after_release_keeps_the_record_until_an_exact_cleanup() {
        let owner = cell_owner("cell-1", "network");
        let relay = "relay end still open";
        for capability in capabilities() {
            for (fails_in_revoke, cleans_up_in_revoke) in
                [(true, true), (true, false), (false, true), (false, false)]
            {
                let mut backend = FakeBackend::new();
                let failing = grant_to(&mut backend, &owner, capability.clone());
                let peer = grant_to(&mut backend, &owner, capability.clone());
                let op = failing.object().operation();
                backend.fail_next_withdrawal(failing.handle, relay);
                let refused = if fails_in_revoke {
                    backend
                        .revoke(failing.handle, DrainSpec::forcing())
                        .map(|_| ())
                } else {
                    backend.apply_removal(failing.removal(), &owner, DrainSpec::forcing())
                };
                assert_eq!(refused, Err(incomplete(&op, relay)));
                let failed = account(&[peer.object()], &[failing.clone(), peer.clone()], &[&op]);
                assert_eq!(backend.snapshot_os_state(), Ok(failed.clone()));
                for retry in [Entry::Grant, Entry::Apply] {
                    assert_eq!(
                        create(&mut backend, retry, &op),
                        Err(incomplete(&op, relay))
                    );
                }
                assert_eq!(backend.snapshot_os_state(), Ok(failed));
                let drain = DrainSpec::graceful(Duration::from_millis(3));
                if cleans_up_in_revoke {
                    assert_eq!(backend.revoke(failing.handle, drain), Ok(failing.clone()));
                } else {
                    backend
                        .apply_removal(failing.removal(), &owner, drain)
                        .unwrap();
                }
                assert_eq!(
                    backend.snapshot_os_state(),
                    Ok(account(&[peer.object()], std::slice::from_ref(&peer), &[]))
                );
                assert_eq!(
                    backend.revoke(failing.handle, DrainSpec::forcing()),
                    Err(BackendError::UnknownHandle {
                        handle: failing.handle
                    })
                );
            }
        }
    }

    #[test]
    fn a_graceful_timeout_comes_before_a_release_that_fails_and_an_applied_holding_has_no_record() {
        let owner = cell_owner("cell-1", "workspace");
        let op = mount_by(&owner);
        let mut backend = FakeBackend::new();
        backend.apply(op.clone()).unwrap();
        backend.mark_stuck(op.handle());
        backend.fail_next_withdrawal(op.handle(), "unmount busy");
        let before = backend.snapshot_os_state().unwrap();
        assert!(matches!(
            backend.apply_removal(
                op.removal(),
                &owner,
                DrainSpec::graceful(Duration::from_millis(2))
            ),
            Err(BackendError::DrainTimedOut { .. })
        ));
        assert_eq!(backend.snapshot_os_state(), Ok(before));
        let failed = backend
            .apply_removal(op.removal(), &owner, DrainSpec::forcing())
            .unwrap_err();
        assert_eq!(failed, incomplete(&op, "unmount busy"));
        assert_eq!(
            failed.to_string(),
            format!("mount operation {} is incomplete: unmount busy", op.id())
        );
        assert_eq!(backend.snapshot_os_state(), Ok(account(&[], &[], &[&op])));
        assert_eq!(
            backend.revoke(op.handle(), DrainSpec::forcing()),
            Err(BackendError::UnknownHandle {
                handle: op.handle()
            })
        );
        backend
            .apply_removal(op.removal(), &owner, DrainSpec::forcing())
            .unwrap();
        assert!(backend.snapshot_os_state().unwrap().is_empty());
    }
}
