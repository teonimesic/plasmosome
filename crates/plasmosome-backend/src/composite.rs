use crate::backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, Grant, Handle, LedgerEntry,
};
use crate::universe::{CellOwner, OsObject, OsState, UniverseClass, UniverseOp, UniverseRemoval};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leaf {
    Network,
    Filesystem,
    Broker,
}

pub struct CompositeBackend {
    network: Box<dyn EnforcementBackend>,
    filesystem: Box<dyn EnforcementBackend>,
    broker: Box<dyn EnforcementBackend>,
}

impl CompositeBackend {
    pub fn new(
        network: Box<dyn EnforcementBackend>,
        filesystem: Box<dyn EnforcementBackend>,
        broker: Box<dyn EnforcementBackend>,
    ) -> Result<CompositeBackend, BackendError> {
        validate_leaf(Leaf::Network, network.as_ref())?;
        validate_leaf(Leaf::Filesystem, filesystem.as_ref())?;
        validate_leaf(Leaf::Broker, broker.as_ref())?;
        Ok(CompositeBackend {
            network,
            filesystem,
            broker,
        })
    }

    fn leaf_for(&mut self, capability: &Capability) -> &mut dyn EnforcementBackend {
        self.leaf_for_class(capability.class())
    }

    fn leaf_for_class(&mut self, class: UniverseClass) -> &mut dyn EnforcementBackend {
        match class {
            UniverseClass::ProxyMap | UniverseClass::UdsPath => self.network.as_mut(),
            UniverseClass::SessionFile | UniverseClass::Mount => self.filesystem.as_mut(),
            UniverseClass::BrokerPid => self.broker.as_mut(),
        }
    }

    pub fn leaf_snapshot(&self, leaf: Leaf) -> OsState {
        match leaf {
            Leaf::Network => self.network.snapshot_os_state(),
            Leaf::Filesystem => self.filesystem.snapshot_os_state(),
            Leaf::Broker => self.broker.snapshot_os_state(),
        }
    }
}

impl EnforcementBackend for CompositeBackend {
    fn grant(&mut self, grant: Grant) -> LedgerEntry {
        self.leaf_for(&grant.capability).grant(grant)
    }

    fn revoke(&mut self, handle: Handle, drain: DrainSpec) -> Result<LedgerEntry, BackendError> {
        self.leaf_for_class(handle.class).revoke(handle, drain)
    }

    fn snapshot_os_state(&self) -> OsState {
        let mut union = self.network.snapshot_os_state();
        for object in self.filesystem.snapshot_os_state().objects() {
            union
                .insert(object.clone())
                .expect("validated class ownership keeps leaf addresses disjoint");
        }
        for object in self.broker.snapshot_os_state().objects() {
            union
                .insert(object.clone())
                .expect("validated class ownership keeps leaf addresses disjoint");
        }
        union
    }

    fn apply(&mut self, op: UniverseOp) -> Result<(), BackendError> {
        self.leaf_for_class(op.class()).apply(op)
    }

    fn apply_removal(
        &mut self,
        removal: UniverseRemoval,
        owner: &CellOwner,
        drain: DrainSpec,
    ) -> Result<(), BackendError> {
        self.leaf_for_class(removal.class())
            .apply_removal(removal, owner, drain)
    }

    fn plant(&mut self, object: OsObject) -> Result<(), BackendError> {
        self.leaf_for_class(object.class()).plant(object)
    }
}

fn validate_leaf(leaf: Leaf, backend: &dyn EnforcementBackend) -> Result<(), BackendError> {
    for object in backend.snapshot_os_state().objects() {
        if !class_belongs_to(leaf, object.class()) {
            return Err(BackendError::Fault(format!(
                "{leaf:?} leaf observes out-of-class {}",
                object.describe()
            )));
        }
    }
    Ok(())
}

fn class_belongs_to(leaf: Leaf, class: UniverseClass) -> bool {
    matches!(
        (leaf, class),
        (
            Leaf::Network,
            UniverseClass::ProxyMap | UniverseClass::UdsPath
        ) | (
            Leaf::Filesystem,
            UniverseClass::SessionFile | UniverseClass::Mount
        ) | (Leaf::Broker, UniverseClass::BrokerPid)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::GrantKind;
    use crate::fake::FakeBackend;
    use crate::fixtures::{self, broker, mount, proxy_map, session_file, uds_socket};
    use crate::universe::{CellId, CellOwner, GrantId, PluginId};
    use std::time::Duration;

    fn fake() -> Box<dyn EnforcementBackend> {
        Box::new(FakeBackend::new())
    }

    fn cell_owner(plugin: &str) -> CellOwner {
        CellOwner {
            cell: CellId::from("cell-1"),
            plugin: PluginId::from(plugin),
        }
    }

    fn composite() -> CompositeBackend {
        CompositeBackend::new(fake(), fake(), fake()).expect("empty leaves are valid")
    }

    #[test]
    fn a_prepared_grant_reaches_the_leaf_of_its_class_at_its_own_address() {
        for (op, leaf) in [
            (
                fixtures::op(
                    GrantId::new(),
                    cell_owner("network"),
                    proxy_map("api.github.com", "splice"),
                ),
                Leaf::Network,
            ),
            (
                fixtures::op(
                    GrantId::new(),
                    cell_owner("network"),
                    uds_socket("/run/ak/egressd.uds"),
                ),
                Leaf::Network,
            ),
            (
                fixtures::op(
                    GrantId::new(),
                    cell_owner("github-pr"),
                    session_file("/skills/pr.md"),
                ),
                Leaf::Filesystem,
            ),
            (
                fixtures::op(
                    GrantId::new(),
                    cell_owner("workspace"),
                    mount("/code", "/workspace"),
                ),
                Leaf::Filesystem,
            ),
            (
                fixtures::op(GrantId::new(), cell_owner("network"), broker("egressd")),
                Leaf::Broker,
            ),
        ] {
            let mut backend = composite();
            let entry = backend.grant(op.clone(), GrantKind::Hot).unwrap();
            assert_eq!(entry.handle, op.handle());
            assert_eq!(entry.object(), op.object());
            assert_eq!(
                backend.leaf_snapshot(leaf).objects().collect::<Vec<_>>(),
                vec![&op.object()]
            );
            assert_eq!(backend.grant(op.clone(), GrantKind::Hot).unwrap(), entry);
            assert_eq!(
                backend.revoke(op.handle(), DrainSpec::forcing()).unwrap(),
                entry
            );
            assert!(backend.snapshot_os_state().is_empty());
        }
    }

    #[test]
    fn a_leafs_refusal_of_a_prepared_grant_comes_back_unchanged() {
        let owner = cell_owner("workspace");
        let doomed = cell_owner("doomed");
        let mut filesystem = FakeBackend::new();
        filesystem.fail_before_effect_for_owner(doomed.clone(), "leaf refused");
        let mut backend =
            CompositeBackend::new(fake(), Box::new(filesystem), fake()).expect("valid leaves");
        let applied = fixtures::op(GrantId::new(), owner.clone(), mount("/code", "/workspace"));
        backend.apply(applied.clone()).unwrap();
        let before = backend.snapshot_os_state();
        assert_eq!(
            backend.grant(applied.clone(), GrantKind::Hot),
            Err(BackendError::IdentityConflict {
                class: "mount",
                id: applied.id(),
            })
        );
        let refused = fixtures::op(GrantId::new(), doomed, session_file("/skills/pr.md"));
        assert_eq!(
            backend.grant(refused.clone(), GrantKind::Hot),
            Err(BackendError::Fault("leaf refused".to_string()))
        );
        assert_eq!(backend.snapshot_os_state(), before);
        for handle in [applied.handle(), refused.handle()] {
            assert_eq!(
                backend.revoke(handle, DrainSpec::forcing()),
                Err(BackendError::UnknownHandle { handle })
            );
        }
    }

    #[test]
    fn handles_route_by_class_without_rewriting_identity() {
        let mut backend = composite();
        let file = backend.grant(Grant {
            owner: cell_owner("github-pr"),
            capability: session_file("/skills/pr.md"),
            kind: GrantKind::Hot,
        });
        let proxy = backend.grant(Grant {
            owner: cell_owner("network"),
            capability: proxy_map("api.github.com", "splice"),
            kind: GrantKind::Hot,
        });
        assert_ne!(file.handle.id, proxy.handle.id);
        assert_eq!(file.handle.class, UniverseClass::SessionFile);
        assert_eq!(proxy.handle.class, UniverseClass::ProxyMap);
        assert!(
            backend
                .leaf_snapshot(Leaf::Filesystem)
                .objects()
                .any(|object| object == &file.object())
        );
        assert!(
            backend
                .leaf_snapshot(Leaf::Network)
                .objects()
                .any(|object| object == &proxy.object())
        );
        assert_eq!(
            backend.revoke(proxy.handle, DrainSpec::forcing()).unwrap(),
            proxy
        );
        assert_eq!(backend.snapshot_os_state().len(), 1);
    }

    #[test]
    fn failed_revoke_preserves_the_original_handle_and_leaf_state() {
        let mut network = FakeBackend::new();
        let entry = network.grant(Grant {
            owner: cell_owner("network"),
            capability: uds_socket("/run/ak/egressd.uds"),
            kind: GrantKind::Hot,
        });
        network.mark_stuck(entry.handle);
        let mut backend =
            CompositeBackend::new(Box::new(network), fake(), fake()).expect("valid leaves");
        let before = backend.snapshot_os_state();
        assert_eq!(
            backend
                .revoke(entry.handle, DrainSpec::graceful(Duration::from_millis(2)))
                .unwrap_err(),
            BackendError::DrainTimedOut {
                handle: entry.handle,
                deadline_ms: 2,
            }
        );
        assert_eq!(backend.snapshot_os_state(), before);
        backend.revoke(entry.handle, DrainSpec::forcing()).unwrap();
        assert!(backend.snapshot_os_state().is_empty());
    }

    #[test]
    fn a_handle_from_another_backend_cannot_withdraw_an_equal_grant() {
        let grant = Grant {
            owner: cell_owner("network"),
            capability: uds_socket("/run/ak/egressd.uds"),
            kind: GrantKind::Hot,
        };
        let stale = FakeBackend::new().grant(grant.clone()).handle;
        let mut backend = composite();
        let live = backend.grant(grant);
        assert_ne!(stale, live.handle);
        assert_eq!(
            backend.revoke(stale, DrainSpec::forcing()).unwrap_err(),
            BackendError::UnknownHandle { handle: stale }
        );
        assert_eq!(
            backend.snapshot_os_state().objects().collect::<Vec<_>>(),
            vec![&live.object()]
        );
    }

    #[test]
    fn apply_removal_routes_the_owner_and_drain_to_the_owning_leaf() {
        let owner = CellOwner {
            cell: CellId::from("cell-1"),
            plugin: PluginId::from("workspace"),
        };
        let elsewhere = CellOwner {
            cell: CellId::from("cell-2"),
            plugin: PluginId::from("workspace"),
        };
        let op = fixtures::op(GrantId::new(), owner.clone(), mount("/code", "/workspace"));
        let mut filesystem = FakeBackend::new();
        filesystem.stall_graceful_drains_for_owner(owner.clone());
        let mut backend =
            CompositeBackend::new(fake(), Box::new(filesystem), fake()).expect("valid leaves");
        backend.apply(op.clone()).unwrap();
        let before = backend.snapshot_os_state();
        let graceful = DrainSpec::graceful(Duration::from_millis(3));
        assert_eq!(
            backend
                .apply_removal(op.removal(), &elsewhere, graceful)
                .unwrap_err(),
            BackendError::UnknownObject {
                class: "mount",
                key: "/workspace".to_string(),
                owner: elsewhere,
                id: op.id(),
            }
        );
        assert_eq!(
            backend
                .apply_removal(op.removal(), &owner, graceful)
                .unwrap_err(),
            BackendError::DrainTimedOut {
                handle: Handle {
                    class: UniverseClass::Mount,
                    id: op.id(),
                },
                deadline_ms: 3,
            }
        );
        assert_eq!(backend.snapshot_os_state(), before);
        backend
            .apply_removal(op.removal(), &owner, DrainSpec::forcing())
            .unwrap();
        assert!(backend.snapshot_os_state().is_empty());
    }

    #[test]
    fn constructor_refuses_out_of_class_initial_observations() {
        let mut network = FakeBackend::new();
        network
            .plant(OsObject {
                id: GrantId::new(),
                owner: cell_owner("workspace"),
                capability: session_file("/skills/pr.md"),
            })
            .unwrap();
        assert!(matches!(
            CompositeBackend::new(Box::new(network), fake(), fake()),
            Err(BackendError::Fault(message)) if message.contains("out-of-class")
        ));
    }

    #[test]
    fn apply_and_plant_preserve_exact_ids_and_reject_alias_conflicts() {
        let id = GrantId::new();
        let op = fixtures::op(id, cell_owner("workspace"), mount("/code", "/workspace"));
        let expected = op.object();
        let removal = UniverseRemoval {
            id,
            capability: expected.capability.clone(),
        };
        let mut backend = composite();
        backend.apply(op).unwrap();
        backend.plant(expected.clone()).unwrap();
        assert_eq!(
            backend
                .leaf_snapshot(Leaf::Filesystem)
                .objects()
                .collect::<Vec<_>>(),
            vec![&expected]
        );

        let conflict = OsObject {
            owner: cell_owner("audit"),
            ..expected.clone()
        };
        let before_conflict = backend.snapshot_os_state();
        assert_eq!(
            backend.plant(conflict).unwrap_err(),
            BackendError::IdentityConflict { class: "mount", id }
        );
        assert_eq!(backend.snapshot_os_state(), before_conflict);

        backend
            .apply_removal(removal.clone(), &expected.owner, DrainSpec::forcing())
            .unwrap();
        assert!(backend.snapshot_os_state().is_empty());
        backend.plant(expected.clone()).unwrap();
        assert_eq!(
            backend
                .leaf_snapshot(Leaf::Filesystem)
                .objects()
                .collect::<Vec<_>>(),
            vec![&expected]
        );
        backend
            .apply_removal(removal, &expected.owner, DrainSpec::forcing())
            .unwrap();
        assert!(backend.snapshot_os_state().is_empty());
    }
}
