use std::collections::{BTreeMap, BTreeSet};

use crate::backend::{
    BackendError, DrainSpec, EnforcementBackend, Grant, Handle, LedgerEntry, RevokePolicy,
};
use crate::universe::{CellOwner, GrantId, OsObject, OsState, UniverseOp, UniverseRemoval};

#[derive(Debug, Default)]
pub struct FakeBackend {
    state: OsState,
    grants: BTreeMap<Handle, LedgerEntry>,
    stuck_handles: BTreeSet<Handle>,
    stalled_owners: BTreeSet<CellOwner>,
    grants_issued: Vec<Grant>,
    revocations: Vec<Handle>,
    apply_fault: Option<(CellOwner, String)>,
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

    /// Makes every later `apply` of an operation owned by exactly `owner` fail with
    /// `BackendError::Fault(cause)` before any change. The same plugin in another cell applies.
    pub fn fail_apply_for_owner(&mut self, owner: CellOwner, cause: &str) {
        self.apply_fault = Some((owner, cause.to_string()));
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

    pub fn plant_residue(&mut self, object: OsObject) -> Result<(), BackendError> {
        self.plant(object)
    }

    pub fn grants_issued(&self) -> &[Grant] {
        &self.grants_issued
    }

    pub fn revocations(&self) -> &[Handle] {
        &self.revocations
    }

    fn mint_with(&self, grant: &Grant, mut next_id: impl FnMut() -> GrantId) -> LedgerEntry {
        loop {
            let id = next_id();
            if !self.state.contains_id(id) {
                return LedgerEntry {
                    handle: Handle {
                        class: grant.capability.class(),
                        id,
                    },
                    owner: grant.owner.clone(),
                    capability: grant.capability.clone(),
                    kind: grant.kind,
                };
            }
        }
    }

    fn grant_with(&mut self, grant: Grant, next_id: impl FnMut() -> GrantId) -> LedgerEntry {
        let entry = self.mint_with(&grant, next_id);
        self.state
            .insert(entry.object())
            .expect("a freshly checked grant identity cannot conflict");
        self.grants.insert(entry.handle, entry.clone());
        self.grants_issued.push(grant);
        entry
    }
}

impl EnforcementBackend for FakeBackend {
    fn grant(&mut self, grant: Grant) -> LedgerEntry {
        self.grant_with(grant, GrantId::new)
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

    fn snapshot_os_state(&self) -> OsState {
        self.state.clone()
    }

    fn apply(&mut self, op: UniverseOp) -> Result<(), BackendError> {
        let object = op.object();
        if let Some((owner, cause)) = &self.apply_fault
            && object.owner == *owner
        {
            return Err(BackendError::Fault(cause.clone()));
        }
        self.state.insert(object).map(|_| ())
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
        if !self.state.selects(&removal, owner) {
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
        self.grants.remove(&handle);
        self.stuck_handles.remove(&handle);
        Ok(())
    }

    fn plant(&mut self, object: OsObject) -> Result<(), BackendError> {
        self.state.insert(object).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Capability, GrantKind};
    use crate::universe::{CellId, CellOwner, PluginId, UniverseClass};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::time::Duration;

    fn grant(plugin: &str, capability: Capability) -> Grant {
        Grant {
            owner: cell_owner("cell-1", plugin),
            capability,
            kind: GrantKind::Hot,
        }
    }

    fn capabilities() -> Vec<Capability> {
        vec![
            Capability::SessionFile {
                path: "skills/pr.md".to_string(),
            },
            Capability::UdsSocket {
                path: "/run/ak/egressd.uds".to_string(),
            },
            Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "splice".to_string(),
            },
            Capability::Broker {
                pid: 4242,
                name: "egressd".to_string(),
            },
            Capability::Mount {
                source: "/secrets".to_string(),
                target: "/workspace".to_string(),
            },
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
                    let first = backend.grant(grant("owner", capability.clone()));
                    let second = backend.grant(grant("owner", capability));
                    assert_ne!(first.handle, second.handle);
                    assert_eq!(backend.snapshot_os_state().len(), 2);
                    let (taken, survivor) = if reverse {
                        (second, first)
                    } else {
                        (first, second)
                    };
                    backend.revoke(taken.handle, drain).unwrap();
                    assert_eq!(
                        backend.snapshot_os_state().objects().collect::<Vec<_>>(),
                        vec![&survivor.object()]
                    );
                    backend.revoke(survivor.handle, drain).unwrap();
                    assert!(backend.snapshot_os_state().is_empty());
                }
            }
        }
    }

    #[test]
    fn colliding_mount_targets_and_proxy_hosts_keep_complete_resources() {
        let mut backend = FakeBackend::new();
        let first_mount = backend.grant(grant(
            "owner",
            Capability::Mount {
                source: "/secrets".to_string(),
                target: "/workspace".to_string(),
            },
        ));
        let second_mount = backend.grant(grant(
            "owner",
            Capability::Mount {
                source: "/code".to_string(),
                target: "/workspace".to_string(),
            },
        ));
        let first_route = backend.grant(grant(
            "owner",
            Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "audit".to_string(),
            },
        ));
        let second_route = backend.grant(grant(
            "owner",
            Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "deploy".to_string(),
            },
        ));
        assert_eq!(backend.snapshot_os_state().len(), 4);
        backend
            .revoke(second_mount.handle, DrainSpec::forcing())
            .unwrap();
        backend
            .revoke(first_route.handle, DrainSpec::forcing())
            .unwrap();
        let standing = backend.snapshot_os_state();
        assert_eq!(
            standing.objects().cloned().collect::<Vec<_>>(),
            vec![second_route.object(), first_mount.object()]
        );
    }

    #[test]
    fn live_broker_revoke_preserves_equal_planted_residue() {
        let capability = Capability::Broker {
            pid: 31337,
            name: "egressd".to_string(),
        };
        let residue = OsObject {
            id: GrantId::new(),
            owner: cell_owner("cell-1", "network"),
            capability: capability.clone(),
        };
        let mut backend = FakeBackend::new();
        backend.plant_residue(residue.clone()).unwrap();
        let live = backend.grant(grant("network", capability));
        backend.revoke(live.handle, DrainSpec::forcing()).unwrap();
        assert_eq!(
            backend.snapshot_os_state().objects().collect::<Vec<_>>(),
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
        assert!(backend.snapshot_os_state().is_empty());
    }

    #[test]
    fn exact_removal_refusals_preserve_neighbouring_holdings() {
        let capability = Capability::ProxyMap {
            host: "api.github.com".to_string(),
            route: "splice".to_string(),
        };
        let mut backend = FakeBackend::new();
        let first = backend.grant(grant("deploy", capability.clone()));
        let second = backend.grant(grant("deploy", capability.clone()));
        let before = backend.snapshot_os_state();
        let wrong_capability = UniverseRemoval {
            id: first.handle.id,
            capability: Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "other".to_string(),
            },
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
            assert_eq!(backend.snapshot_os_state(), before);
        }
        backend.revoke(first.handle, DrainSpec::forcing()).unwrap();
        assert!(matches!(
            backend.revoke(first.handle, DrainSpec::forcing()),
            Err(BackendError::UnknownHandle { .. })
        ));
        assert_eq!(
            backend.snapshot_os_state().objects().collect::<Vec<_>>(),
            vec![&second.object()]
        );
    }

    #[test]
    fn graceful_timeout_preserves_peers_and_force_takes_only_requested_grant() {
        let capability = Capability::UdsSocket {
            path: "/run/ak/egressd.uds".to_string(),
        };
        let mut backend = FakeBackend::new();
        let stuck = backend.grant(grant("network", capability.clone()));
        let peer = backend.grant(grant("network", capability));
        backend.mark_stuck(stuck.handle);
        let before = backend.snapshot_os_state();
        assert!(matches!(
            backend.revoke(stuck.handle, DrainSpec::graceful(Duration::from_millis(2))),
            Err(BackendError::DrainTimedOut { .. })
        ));
        assert_eq!(backend.snapshot_os_state(), before);
        backend.revoke(stuck.handle, DrainSpec::forcing()).unwrap();
        assert_eq!(
            backend.snapshot_os_state().objects().collect::<Vec<_>>(),
            vec![&peer.object()]
        );
    }

    #[test]
    fn recorded_apply_is_idempotent_and_changed_payload_conflicts() {
        let id = GrantId::new();
        let op = UniverseOp::WriteSessionFile {
            id,
            path: "skills/pr.md".to_string(),
            owner: cell_owner("cell-1", "github-pr"),
        };
        let mut backend = FakeBackend::new();
        backend.apply(op.clone()).unwrap();
        backend.apply(op.clone()).unwrap();
        assert_eq!(backend.snapshot_os_state().len(), 1);
        let peer = UniverseOp::WriteSessionFile {
            id: GrantId::new(),
            path: "skills/pr.md".to_string(),
            owner: cell_owner("cell-1", "github-pr"),
        };
        backend.apply(peer).unwrap();
        assert_eq!(backend.snapshot_os_state().len(), 2);
        let conflict = UniverseOp::WriteSessionFile {
            id,
            path: "skills/pr.md".to_string(),
            owner: cell_owner("cell-1", "audit"),
        };
        let before = backend.snapshot_os_state();
        assert!(matches!(
            backend.apply(conflict),
            Err(BackendError::IdentityConflict { .. })
        ));
        assert_eq!(backend.snapshot_os_state(), before);
    }

    #[test]
    fn direct_removal_retires_handle_and_reapply_does_not_revive_it() {
        let mut backend = FakeBackend::new();
        let entry = backend.grant(grant(
            "github-pr",
            Capability::SessionFile {
                path: "skills/pr.md".to_string(),
            },
        ));
        backend
            .apply_removal(entry.removal(), &entry.owner, DrainSpec::forcing())
            .unwrap();
        backend
            .apply(UniverseOp::WriteSessionFile {
                id: entry.handle.id,
                path: "skills/pr.md".to_string(),
                owner: entry.owner.clone(),
            })
            .unwrap();
        assert!(matches!(
            backend.revoke(entry.handle, DrainSpec::forcing()),
            Err(BackendError::UnknownHandle { .. })
        ));
        assert_eq!(backend.snapshot_os_state().len(), 1);
        let fresh = backend.grant(grant("github-pr", entry.capability));
        assert_ne!(fresh.handle, entry.handle);
        assert_eq!(backend.snapshot_os_state().len(), 2);
    }

    #[test]
    fn identity_source_failure_happens_before_any_backend_mutation() {
        let mut backend = FakeBackend::new();
        let existing = backend.grant(grant(
            "network",
            Capability::Broker {
                pid: 7,
                name: "standing".to_string(),
            },
        ));
        let before = backend.snapshot_os_state();
        let grants_before = backend.grants_issued().to_vec();
        let result = catch_unwind(AssertUnwindSafe(|| {
            backend.grant_with(
                grant(
                    "network",
                    Capability::Broker {
                        pid: 8,
                        name: "new".to_string(),
                    },
                ),
                || panic!("deterministic random source failure"),
            )
        }));
        assert!(result.is_err());
        assert_eq!(backend.snapshot_os_state(), before);
        assert_eq!(backend.grants_issued(), grants_before);
        assert_eq!(backend.peek_entry(existing.handle).unwrap(), existing);
    }

    fn cell_owner(cell: &str, plugin: &str) -> CellOwner {
        CellOwner {
            cell: CellId::from(cell),
            plugin: PluginId::from(plugin),
        }
    }

    fn mount_by(owner: &CellOwner) -> UniverseOp {
        UniverseOp::AddMount {
            id: GrantId::new(),
            source: "/code".to_string(),
            target: "/workspace".to_string(),
            owner: owner.clone(),
        }
    }

    fn address_of(op: &UniverseOp) -> Handle {
        Handle {
            class: op.class(),
            id: op.id(),
        }
    }

    fn grant_to(owner: &CellOwner, capability: Capability) -> Grant {
        Grant {
            owner: owner.clone(),
            capability,
            kind: GrantKind::Hot,
        }
    }

    #[test]
    fn graceful_removal_of_a_stuck_grant_keeps_the_holding_its_record_and_its_peer() {
        let network = cell_owner("cell-1", "network");
        let capability = Capability::UdsSocket {
            path: "/run/ak/egressd.uds".to_string(),
        };
        let mut backend = FakeBackend::new();
        let stuck = backend.grant(grant_to(&network, capability.clone()));
        let peer = backend.grant(grant_to(&network, capability));
        backend.mark_stuck(stuck.handle);
        let before = backend.snapshot_os_state();
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
        assert_eq!(backend.snapshot_os_state(), before);
        assert_eq!(backend.peek_entry(stuck.handle).unwrap(), stuck);
        backend
            .apply_removal(stuck.removal(), &network, DrainSpec::forcing())
            .unwrap();
        assert_eq!(
            backend.snapshot_os_state().objects().collect::<Vec<_>>(),
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
        let address = address_of(&op);
        let mut backend = FakeBackend::new();
        backend.apply(op.clone()).unwrap();
        backend.mark_stuck(address);
        let before = backend.snapshot_os_state();
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
        assert_eq!(backend.snapshot_os_state(), before);
        backend
            .apply_removal(op.removal(), &workspace, DrainSpec::forcing())
            .unwrap();
        assert!(backend.snapshot_os_state().is_empty());
    }

    #[test]
    fn a_forced_removal_clears_the_stuck_mark_so_a_later_holding_drains() {
        let workspace = cell_owner("cell-1", "workspace");
        let op = mount_by(&workspace);
        let mut backend = FakeBackend::new();
        backend.apply(op.clone()).unwrap();
        backend.mark_stuck(address_of(&op));
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
        assert!(backend.snapshot_os_state().is_empty());
    }

    #[test]
    fn a_stalled_owner_times_out_every_graceful_withdrawal_but_not_its_peers() {
        let stalled = cell_owner("cell-1", "stalled");
        let peer_owner = cell_owner("cell-1", "network");
        let same_plugin_elsewhere = cell_owner("cell-2", "stalled");
        let capability = Capability::ProxyMap {
            host: "api.github.com".to_string(),
            route: "splice".to_string(),
        };
        let mut backend = FakeBackend::new();
        backend.stall_graceful_drains_for_owner(stalled.clone());
        let granted = backend.grant(grant_to(&stalled, capability.clone()));
        let op = UniverseOp::SetProxyMap {
            id: GrantId::new(),
            host: "api.github.com".to_string(),
            route: "splice".to_string(),
            owner: stalled.clone(),
        };
        backend.apply(op.clone()).unwrap();
        let peer = backend.grant(grant_to(&peer_owner, capability.clone()));
        let elsewhere = backend.grant(grant_to(&same_plugin_elsewhere, capability));
        let before = backend.snapshot_os_state();
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
                handle: address_of(&op),
                deadline_ms: 9,
            }
        );
        assert_eq!(backend.snapshot_os_state(), before);
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
            backend.snapshot_os_state().objects().collect::<Vec<_>>(),
            vec![&op.object()]
        );
        backend
            .apply_removal(op.removal(), &stalled, DrainSpec::forcing())
            .unwrap();
        assert!(backend.snapshot_os_state().is_empty());
    }

    #[test]
    fn a_zero_graceful_deadline_checks_once_and_never_forces() {
        let workspace = cell_owner("cell-1", "workspace");
        let op = mount_by(&workspace);
        let mut backend = FakeBackend::new();
        let stuck = backend.grant(grant_to(
            &workspace,
            Capability::SessionFile {
                path: "skills/pr.md".to_string(),
            },
        ));
        backend.apply(op.clone()).unwrap();
        backend.mark_stuck(address_of(&op));
        backend.mark_stuck(stuck.handle);
        let before = backend.snapshot_os_state();
        let zero = DrainSpec::graceful(Duration::ZERO);
        assert_eq!(
            backend
                .apply_removal(op.removal(), &workspace, zero)
                .unwrap_err(),
            BackendError::DrainTimedOut {
                handle: address_of(&op),
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
        assert_eq!(backend.snapshot_os_state(), before);
        assert_eq!(backend.peek_entry(stuck.handle).unwrap(), stuck);
        let drained = backend.grant(grant_to(
            &workspace,
            Capability::SessionFile {
                path: "skills/drained.md".to_string(),
            },
        ));
        assert_eq!(backend.revoke(drained.handle, zero).unwrap(), drained);
        assert_eq!(backend.snapshot_os_state(), before);
    }

    #[test]
    fn a_timeout_reports_its_deadline_in_whole_milliseconds_rounded_up() {
        let workspace = cell_owner("cell-1", "workspace");
        let op = mount_by(&workspace);
        let mut backend = FakeBackend::new();
        backend.apply(op.clone()).unwrap();
        backend.mark_stuck(address_of(&op));
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
                    handle: address_of(&op),
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
        backend.mark_stuck(address_of(&op));
        let before = backend.snapshot_os_state();
        let other_source = UniverseRemoval {
            id: op.id(),
            capability: Capability::Mount {
                source: "/secrets".to_string(),
                target: "/workspace".to_string(),
            },
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
                assert_eq!(backend.snapshot_os_state(), before);
            }
        }
    }

    #[test]
    fn apply_faults_are_keyed_by_the_cell_qualified_owner() {
        let doomed = cell_owner("cell-1", "doomed");
        let mut backend = FakeBackend::new();
        backend.fail_apply_for_owner(doomed.clone(), "injected refusal");
        let elsewhere = UniverseOp::WriteSessionFile {
            id: GrantId::new(),
            path: "skills/pr.md".to_string(),
            owner: cell_owner("cell-2", "doomed"),
        };
        backend.apply(elsewhere.clone()).unwrap();
        let before = backend.snapshot_os_state();
        assert_eq!(
            backend
                .apply(UniverseOp::WriteSessionFile {
                    id: GrantId::new(),
                    path: "skills/pr.md".to_string(),
                    owner: doomed,
                })
                .unwrap_err(),
            BackendError::Fault("injected refusal".to_string())
        );
        assert_eq!(backend.snapshot_os_state(), before);
        assert_eq!(
            before.objects().collect::<Vec<_>>(),
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
}
