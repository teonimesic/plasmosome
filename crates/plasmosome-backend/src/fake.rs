use std::collections::{BTreeMap, BTreeSet};

use crate::backend::{
    BackendError, DrainSpec, EnforcementBackend, Grant, Handle, LedgerEntry, RevokePolicy,
};
use crate::universe::{GrantId, OsObject, OsState, PluginId, UniverseOp, UniverseRemoval};

#[derive(Debug, Default)]
pub struct FakeBackend {
    state: OsState,
    grants: BTreeMap<Handle, LedgerEntry>,
    stuck_handles: BTreeSet<Handle>,
    grants_issued: Vec<Grant>,
    revocations: Vec<Handle>,
    apply_fault: Option<(String, String)>,
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

    pub fn fail_apply_for_owner(&mut self, owner: &str, cause: &str) {
        self.apply_fault = Some((owner.to_string(), cause.to_string()));
    }

    pub fn mark_stuck(&mut self, handle: Handle) {
        self.stuck_handles.insert(handle);
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
                    plugin: grant.plugin.clone(),
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
        if drain.policy == RevokePolicy::Graceful && self.stuck_handles.contains(&handle) {
            return Err(BackendError::DrainTimedOut {
                handle,
                deadline_ms: drain.deadline.as_millis() as u64,
            });
        }
        self.apply_removal(entry.removal(), &entry.plugin)?;
        self.revocations.push(handle);
        Ok(entry)
    }

    fn snapshot_os_state(&self) -> OsState {
        self.state.clone()
    }

    fn apply(&mut self, op: UniverseOp) -> Result<(), BackendError> {
        let object = op.object();
        if let Some((owner, cause)) = &self.apply_fault
            && object.owner.as_str() == owner
        {
            return Err(BackendError::Fault(cause.clone()));
        }
        self.state.insert(object).map(|_| ())
    }

    fn apply_removal(
        &mut self,
        removal: UniverseRemoval,
        owner: &PluginId,
    ) -> Result<(), BackendError> {
        let (class, key, id) = (removal.class(), removal.key(), removal.id);
        self.state
            .remove(&removal, owner)
            .map(|_| {
                let handle = Handle { class, id };
                self.grants.remove(&handle);
                self.stuck_handles.remove(&handle);
            })
            .ok_or_else(|| BackendError::UnknownObject {
                class: class.as_str(),
                key,
                owner: owner.clone(),
                id,
            })
    }

    fn plant(&mut self, object: OsObject) -> Result<(), BackendError> {
        self.state.insert(object).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Capability, GrantKind};
    use crate::universe::UniverseClass;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::time::Duration;

    fn grant(plugin: &str, capability: Capability) -> Grant {
        Grant {
            plugin: PluginId::from(plugin),
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
            owner: PluginId::from("network"),
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
            (first.removal(), PluginId::from("audit")),
            (wrong_capability, PluginId::from("deploy")),
            (
                UniverseRemoval {
                    id: GrantId::new(),
                    capability,
                },
                PluginId::from("deploy"),
            ),
        ] {
            assert!(matches!(
                backend.apply_removal(removal, &owner),
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
            owner: PluginId::from("github-pr"),
        };
        let mut backend = FakeBackend::new();
        backend.apply(op.clone()).unwrap();
        backend.apply(op.clone()).unwrap();
        assert_eq!(backend.snapshot_os_state().len(), 1);
        let peer = UniverseOp::WriteSessionFile {
            id: GrantId::new(),
            path: "skills/pr.md".to_string(),
            owner: PluginId::from("github-pr"),
        };
        backend.apply(peer).unwrap();
        assert_eq!(backend.snapshot_os_state().len(), 2);
        let conflict = UniverseOp::WriteSessionFile {
            id,
            path: "skills/pr.md".to_string(),
            owner: PluginId::from("audit"),
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
            .apply_removal(entry.removal(), &entry.plugin)
            .unwrap();
        backend
            .apply(UniverseOp::WriteSessionFile {
                id: entry.handle.id,
                path: "skills/pr.md".to_string(),
                owner: entry.plugin.clone(),
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
