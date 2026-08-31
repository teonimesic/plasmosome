use std::collections::{BTreeMap, BTreeSet};

use crate::backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, Grant, Handle, LedgerEntry,
    RevokePolicy,
};
use crate::universe::{OsObject, OsState, PluginId, UniverseOp, UniverseRemoval};

#[derive(Debug, Default)]
pub struct FakeBackend {
    state: OsState,
    grants: BTreeMap<u64, LedgerEntry>,
    next_handle: u64,
    stuck_handles: BTreeSet<u64>,
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
            .get(&handle.raw())
            .cloned()
            .ok_or(BackendError::UnknownHandle { handle })
    }

    pub fn fail_apply_for_owner(&mut self, owner: &str, cause: &str) {
        self.apply_fault = Some((owner.to_string(), cause.to_string()));
    }

    pub fn mark_stuck(&mut self, handle: Handle) {
        self.stuck_handles.insert(handle.raw());
    }

    pub fn plant_residue(&mut self, object: OsObject) {
        self.state.insert(object);
    }

    pub fn grants_issued(&self) -> &[Grant] {
        &self.grants_issued
    }

    pub fn revocations(&self) -> &[Handle] {
        &self.revocations
    }

    fn mint(&mut self, grant: &Grant) -> LedgerEntry {
        self.next_handle += 1;
        LedgerEntry {
            handle: Handle(self.next_handle),
            plugin: grant.plugin.clone(),
            capability: grant.capability.clone(),
            kind: grant.kind,
        }
    }

    fn materialize(&mut self, entry: &LedgerEntry) {
        let op = match &entry.capability {
            Capability::SessionFile { path } => UniverseOp::WriteSessionFile {
                path: path.clone(),
                owner: entry.plugin.clone(),
            },
            Capability::UdsSocket { path } => UniverseOp::BindUds {
                path: path.clone(),
                owner: entry.plugin.clone(),
            },
            Capability::ProxyMap { host, route } => UniverseOp::SetProxyMap {
                host: host.clone(),
                route: route.clone(),
                owner: entry.plugin.clone(),
            },
            Capability::Broker { pid, name } => UniverseOp::SpawnBroker {
                pid: *pid,
                name: name.clone(),
                owner: entry.plugin.clone(),
            },
            Capability::Mount { source, target } => UniverseOp::AddMount {
                source: source.clone(),
                target: target.clone(),
                owner: entry.plugin.clone(),
            },
        };
        self.state.insert(op.object());
    }
}

impl EnforcementBackend for FakeBackend {
    fn grant(&mut self, grant: Grant) -> LedgerEntry {
        let entry = self.mint(&grant);
        self.grants_issued.push(grant);
        self.materialize(&entry);
        self.grants.insert(entry.handle.raw(), entry.clone());
        entry
    }

    fn revoke(&mut self, handle: Handle, drain: DrainSpec) -> Result<LedgerEntry, BackendError> {
        let entry = self
            .grants
            .get(&handle.raw())
            .cloned()
            .ok_or(BackendError::UnknownHandle { handle })?;
        if drain.policy == RevokePolicy::Graceful && self.stuck_handles.contains(&handle.raw()) {
            return Err(BackendError::DrainTimedOut {
                handle,
                deadline_ms: drain.deadline.as_millis() as u64,
            });
        }
        let removal = removal_of(&entry);
        self.apply_removal(removal, &entry.plugin)?;
        self.grants.remove(&handle.raw());
        self.stuck_handles.remove(&handle.raw());
        self.revocations.push(handle);
        Ok(entry)
    }

    fn snapshot_os_state(&self) -> OsState {
        self.state.clone()
    }

    fn apply(&mut self, op: UniverseOp) -> Result<(), BackendError> {
        if let Some((owner, cause)) = &self.apply_fault
            && op.object().owner.as_str() == owner
        {
            return Err(BackendError::Fault(cause.clone()));
        }
        self.state.insert(op.object());
        Ok(())
    }

    fn apply_removal(
        &mut self,
        removal: UniverseRemoval,
        owner: &PluginId,
    ) -> Result<(), BackendError> {
        let (class, key) = (removal.class(), removal.key());
        self.state
            .remove(class, &key, owner)
            .map(|_| ())
            .ok_or_else(|| BackendError::UnknownObject {
                class: class.as_str(),
                key,
                owner: owner.clone(),
            })
    }

    fn plant(&mut self, object: OsObject) {
        self.state.insert(object);
    }
}

fn removal_of(entry: &LedgerEntry) -> UniverseRemoval {
    match &entry.capability {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::GrantKind;
    use crate::universe::UniverseClass;
    use std::time::Duration;

    fn grant(plugin: &str, capability: Capability) -> Grant {
        Grant {
            plugin: PluginId::from(plugin),
            capability,
            kind: GrantKind::Hot,
        }
    }

    fn egressd_socket() -> Capability {
        Capability::UdsSocket {
            path: "/run/ak/session-1/egressd.uds".to_string(),
        }
    }

    #[test]
    fn grant_materializes_the_object_and_returns_a_ledger_entry() {
        let mut backend = FakeBackend::new();
        let entry = backend.grant(grant("network", egressd_socket()));
        assert_eq!(entry.handle, Handle(1));
        assert_eq!(entry.plugin, PluginId::from("network"));
        assert_eq!(entry.kind, GrantKind::Hot);
        let state = backend.snapshot_os_state();
        assert!(state.contains(UniverseClass::UdsPath, "/run/ak/session-1/egressd.uds"));
        assert_eq!(
            state
                .owner_of(UniverseClass::UdsPath, "/run/ak/session-1/egressd.uds")
                .as_ref()
                .map(PluginId::as_str),
            Some("network")
        );
    }

    #[test]
    fn a_revoke_takes_the_object_its_own_grant_created() {
        let mut backend = FakeBackend::new();
        backend.grant(grant(
            "audit",
            Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "audit-proxy:8080".to_string(),
            },
        ));
        let deploy = backend.grant(grant(
            "deploy",
            Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "deploy-proxy:9090".to_string(),
            },
        ));
        backend
            .revoke(deploy.handle, DrainSpec::forcing())
            .expect("deploy's own grant is revocable");
        let state = backend.snapshot_os_state();
        let owners: Vec<&str> = state.objects().map(|o| o.owner.as_str()).collect();
        assert_eq!(
            owners,
            vec!["audit"],
            "revoking deploy's proxy map must leave audit's standing and take deploy's"
        );
    }

    #[test]
    fn every_grant_class_lands_in_its_universe_slot() {
        let mut backend = FakeBackend::new();
        let grants = vec![
            grant(
                "github-pr",
                Capability::SessionFile {
                    path: "skills/pr.md".to_string(),
                },
            ),
            grant(
                "network",
                Capability::UdsSocket {
                    path: "/run/ak/egressd.uds".to_string(),
                },
            ),
            grant(
                "github",
                Capability::ProxyMap {
                    host: "api.github.com".to_string(),
                    route: "splice".to_string(),
                },
            ),
            grant(
                "network",
                Capability::Broker {
                    pid: 4242,
                    name: "egressd".to_string(),
                },
            ),
            grant(
                "workspace-bind",
                Capability::Mount {
                    source: "~/code/repo".to_string(),
                    target: "/workspace".to_string(),
                },
            ),
        ];
        for g in grants {
            backend.grant(g);
        }
        let state = backend.snapshot_os_state();
        assert_eq!(state.len(), 5);
        assert!(state.contains(UniverseClass::SessionFile, "session/skills/pr.md"));
        assert!(state.contains(UniverseClass::UdsPath, "/run/ak/egressd.uds"));
        assert!(state.contains(UniverseClass::ProxyMap, "api.github.com"));
        assert!(state.contains(UniverseClass::BrokerPid, "broker/4242"));
        assert!(state.contains(UniverseClass::Mount, "/workspace"));
    }

    #[test]
    fn handles_are_unique_across_grants() {
        let mut backend = FakeBackend::new();
        let a = backend.grant(grant("network", egressd_socket()));
        let b = backend.grant(grant(
            "network",
            Capability::Broker {
                pid: 7,
                name: "egressd".to_string(),
            },
        ));
        assert_ne!(a.handle, b.handle);
    }

    #[test]
    fn revoke_removes_exactly_what_its_grant_added() {
        let mut backend = FakeBackend::new();
        let before = backend.snapshot_os_state();
        let entry = backend.grant(grant(
            "github",
            Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "splice".to_string(),
            },
        ));
        let removed = backend
            .revoke(entry.handle, DrainSpec::graceful(Duration::from_millis(2)))
            .unwrap();
        assert_eq!(removed.capability, entry.capability);
        assert!(Diff::between(&before, &backend.snapshot_os_state()).is_empty());
    }

    #[test]
    fn revoke_of_a_never_granted_handle_is_a_named_error() {
        let mut backend = FakeBackend::new();
        let err = backend
            .revoke(Handle(99), DrainSpec::graceful(Duration::from_millis(1)))
            .unwrap_err();
        assert_eq!(err, BackendError::UnknownHandle { handle: Handle(99) });
    }

    #[test]
    fn a_stuck_handle_times_out_under_graceful_drain() {
        let mut backend = FakeBackend::new();
        let entry = backend.grant(grant(
            "github",
            Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "splice".to_string(),
            },
        ));
        backend.mark_stuck(entry.handle);
        let err = backend
            .revoke(entry.handle, DrainSpec::graceful(Duration::from_millis(2)))
            .unwrap_err();
        assert_eq!(
            err,
            BackendError::DrainTimedOut {
                handle: entry.handle,
                deadline_ms: 2
            }
        );
        assert!(
            backend
                .snapshot_os_state()
                .contains(UniverseClass::ProxyMap, "api.github.com")
        );
    }

    #[test]
    fn force_revoke_reclaims_a_stuck_handle() {
        let mut backend = FakeBackend::new();
        let entry = backend.grant(grant("network", egressd_socket()));
        backend.mark_stuck(entry.handle);
        backend.revoke(entry.handle, DrainSpec::forcing()).unwrap();
        assert!(
            !backend
                .snapshot_os_state()
                .contains(UniverseClass::UdsPath, "/run/ak/session-1/egressd.uds")
        );
    }

    #[test]
    fn generation_bound_grants_are_recorded_with_their_kind() {
        let mut backend = FakeBackend::new();
        let entry = backend.grant(Grant {
            plugin: PluginId::from("resources"),
            capability: Capability::Broker {
                pid: 1,
                name: "cpu-slice".to_string(),
            },
            kind: GrantKind::GenerationBound,
        });
        assert_eq!(entry.kind, GrantKind::GenerationBound);
        assert_eq!(backend.grants_issued()[0].kind, GrantKind::GenerationBound);
    }

    #[test]
    fn planted_residue_is_invisible_to_the_ledger_but_present_in_the_snapshot() {
        let mut backend = FakeBackend::new();
        backend.grant(grant(
            "github-pr",
            Capability::SessionFile {
                path: "skills/pr.md".to_string(),
            },
        ));
        backend.plant_residue(OsObject {
            class: UniverseClass::SessionFile,
            key: "session/cache/github-tokens".to_string(),
            owner: PluginId::from("github"),
        });
        let state = backend.snapshot_os_state();
        assert_eq!(state.len(), 2);
        assert!(state.contains(UniverseClass::SessionFile, "session/cache/github-tokens"));
    }

    #[test]
    fn apply_and_apply_removal_drive_the_universe_directly() {
        let mut backend = FakeBackend::new();
        backend
            .apply(UniverseOp::WriteSessionFile {
                path: "skills/pr.md".to_string(),
                owner: PluginId::from("github-pr"),
            })
            .unwrap();
        assert!(
            backend
                .snapshot_os_state()
                .contains(UniverseClass::SessionFile, "session/skills/pr.md")
        );
        backend
            .apply_removal(
                UniverseRemoval::RemoveSessionFile {
                    path: "skills/pr.md".to_string(),
                },
                &PluginId::from("github-pr"),
            )
            .unwrap();
        assert!(backend.snapshot_os_state().is_empty());
    }

    #[test]
    fn a_removal_names_the_asking_plasmid_and_never_claims_the_key_is_free() {
        let mut backend = FakeBackend::new();
        backend.plant(OsObject {
            class: UniverseClass::BrokerPid,
            key: "broker/1234".to_string(),
            owner: PluginId::from("audit"),
        });
        let held_by_another = backend
            .apply_removal(
                UniverseRemoval::KillBroker { pid: 1234 },
                &PluginId::from("network"),
            )
            .unwrap_err();
        assert_eq!(
            held_by_another.to_string(),
            "`network` holds no broker-pid object `broker/1234`",
            "the error says what is true of the asking plasmid, never that the object is absent"
        );
        assert!(
            backend
                .snapshot_os_state()
                .contains(UniverseClass::BrokerPid, "broker/1234"),
            "a refused removal must leave the other plasmid's object where it was"
        );
    }

    #[test]
    fn removing_an_absent_object_is_a_named_error() {
        let mut backend = FakeBackend::new();
        let err = backend
            .apply_removal(
                UniverseRemoval::KillBroker { pid: 1234 },
                &PluginId::from("network"),
            )
            .unwrap_err();
        assert!(matches!(
            err,
            BackendError::UnknownObject {
                class: "broker-pid",
                ..
            }
        ));
    }

    use crate::universe::Diff;
}
