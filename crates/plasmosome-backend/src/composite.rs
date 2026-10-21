use crate::backend::{BackendError, DrainSpec, EnforcementBackend, GrantKind, Handle, LedgerEntry};
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
    fn grant(
        &mut self,
        operation: UniverseOp,
        kind: GrantKind,
    ) -> Result<LedgerEntry, BackendError> {
        self.leaf_for_class(operation.class())
            .grant(operation, kind)
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
    use crate::backend::{Capability, GrantKind};
    use crate::fake::FakeBackend;
    use crate::fixtures::{self, broker, mount, proxy_map, session_file, uds_socket};
    use crate::snapshot::{EnforcementSnapshot, IncompleteEffect};
    use crate::universe::{CellId, CellOwner, GrantId, PluginId};
    use std::collections::VecDeque;
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

    fn prepared(plugin: &str, capability: crate::backend::Capability) -> UniverseOp {
        fixtures::op(GrantId::new(), cell_owner(plugin), capability)
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
        let file = backend
            .grant(
                prepared("github-pr", session_file("/skills/pr.md")),
                GrantKind::Hot,
            )
            .unwrap();
        let proxy = backend
            .grant(
                prepared("network", proxy_map("api.github.com", "splice")),
                GrantKind::Hot,
            )
            .unwrap();
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
        let entry = network
            .grant(
                prepared("network", uds_socket("/run/ak/egressd.uds")),
                GrantKind::Hot,
            )
            .unwrap();
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
        let socket = || prepared("network", uds_socket("/run/ak/egressd.uds"));
        let stale = FakeBackend::new()
            .grant(socket(), GrantKind::Hot)
            .unwrap()
            .handle;
        let mut backend = composite();
        let live = backend.grant(socket(), GrantKind::Hot).unwrap();
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

    const LEAVES: [Leaf; 3] = [Leaf::Network, Leaf::Filesystem, Leaf::Broker];

    type Observation = Result<EnforcementSnapshot, BackendError>;

    struct ScriptedLeaf {
        observations: VecDeque<Observation>,
    }

    fn scripted(
        observations: impl IntoIterator<Item = Observation>,
    ) -> Box<dyn EnforcementBackend> {
        Box::new(ScriptedLeaf {
            observations: observations.into_iter().collect(),
        })
    }

    impl EnforcementBackend for ScriptedLeaf {
        fn grant(&mut self, _: UniverseOp, _: GrantKind) -> Result<LedgerEntry, BackendError> {
            Err(BackendError::Unimplemented("scripted leaf grant"))
        }

        fn revoke(&mut self, _: Handle, _: DrainSpec) -> Result<LedgerEntry, BackendError> {
            Err(BackendError::Unimplemented("scripted leaf revoke"))
        }

        fn snapshot_os_state(&mut self) -> Observation {
            self.observations
                .pop_front()
                .unwrap_or(Err(BackendError::Unimplemented(
                    "no scripted observation left",
                )))
        }

        fn apply(&mut self, _: UniverseOp) -> Result<(), BackendError> {
            Err(BackendError::Unimplemented("scripted leaf apply"))
        }

        fn apply_removal(
            &mut self,
            _: UniverseRemoval,
            _: &CellOwner,
            _: DrainSpec,
        ) -> Result<(), BackendError> {
            Err(BackendError::Unimplemented("scripted leaf apply_removal"))
        }

        fn plant(&mut self, _: OsObject) -> Result<(), BackendError> {
            Err(BackendError::Unimplemented("scripted leaf plant"))
        }
    }

    fn composite_with(
        leaf: Leaf,
        odd_one: Box<dyn EnforcementBackend>,
    ) -> Result<CompositeBackend, BackendError> {
        match leaf {
            Leaf::Network => CompositeBackend::new(odd_one, fake(), fake()),
            Leaf::Filesystem => CompositeBackend::new(fake(), odd_one, fake()),
            Leaf::Broker => CompositeBackend::new(fake(), fake(), odd_one),
        }
    }

    fn issued(op: &UniverseOp) -> LedgerEntry {
        LedgerEntry {
            handle: op.handle(),
            owner: op.owner().clone(),
            capability: op.capability(),
            kind: GrantKind::Hot,
        }
    }

    fn unfinished(op: &UniverseOp) -> IncompleteEffect {
        IncompleteEffect {
            operation: op.clone(),
        }
    }

    fn own_capabilities(leaf: Leaf) -> [Capability; 4] {
        match leaf {
            Leaf::Network => [
                uds_socket("/run/ak/egressd.uds"),
                proxy_map("api.github.com", "splice"),
                proxy_map("api.github.com", "audit"),
                uds_socket("/run/ak/auditd.uds"),
            ],
            Leaf::Filesystem => [
                mount("/code", "/workspace"),
                session_file("/skills/pr.md"),
                mount("/secrets", "/workspace"),
                session_file("/skills/review.md"),
            ],
            Leaf::Broker => [
                broker("egressd"),
                broker("auditd"),
                broker("relayd"),
                broker("proxyd"),
            ],
        }
    }

    fn foreign_capability(leaf: Leaf) -> Capability {
        match leaf {
            Leaf::Network => session_file("/skills/foreign.md"),
            Leaf::Filesystem => proxy_map("foreign.github.com", "splice"),
            Leaf::Broker => mount("/foreign", "/workspace"),
        }
    }

    fn rows_of(leaf: Leaf) -> EnforcementSnapshot {
        let [granted, applied, withdrawal_failed, first_effect_failed] =
            own_capabilities(leaf).map(|capability| prepared(&format!("{leaf:?}"), capability));
        let mut state = OsState::new();
        state.insert(granted.object()).unwrap();
        state.insert(applied.object()).unwrap();
        EnforcementSnapshot::new(
            state,
            vec![issued(&granted), issued(&withdrawal_failed)],
            vec![
                unfinished(&withdrawal_failed),
                unfinished(&first_effect_failed),
            ],
        )
        .unwrap()
    }

    fn union(parts: &[EnforcementSnapshot]) -> EnforcementSnapshot {
        let mut state = OsState::new();
        let mut grants = Vec::new();
        let mut incomplete = Vec::new();
        for part in parts {
            for object in part.state().objects() {
                state.insert(object.clone()).unwrap();
            }
            grants.extend(part.grants().iter().cloned());
            incomplete.extend(part.incomplete().iter().cloned());
        }
        EnforcementSnapshot::new(state, grants, incomplete).unwrap()
    }

    #[test]
    fn a_healthy_union_keeps_every_row_of_every_leaf_in_address_order() {
        let parts = LEAVES.map(rows_of);
        let [network, filesystem, broker] = parts
            .clone()
            .map(|part| scripted([Ok(part.clone()), Ok(part)]));
        let mut backend = CompositeBackend::new(network, filesystem, broker).unwrap();
        let snapshot = backend.snapshot_os_state().unwrap();
        assert_eq!(snapshot, union(&parts));
        assert_eq!(
            (
                snapshot.state().len(),
                snapshot.grants().len(),
                snapshot.incomplete().len()
            ),
            (6, 6, 6)
        );
        for part in &parts {
            for object in part.state().objects() {
                assert!(snapshot.state().objects().any(|held| held == object));
            }
            for entry in part.grants() {
                assert!(snapshot.grants().contains(entry));
            }
            for effect in part.incomplete() {
                assert!(snapshot.incomplete().contains(effect));
            }
        }
        assert!(
            snapshot
                .grants()
                .windows(2)
                .all(|pair| pair[0].handle < pair[1].handle)
        );
        assert!(
            snapshot
                .incomplete()
                .windows(2)
                .all(|pair| pair[0].handle() < pair[1].handle())
        );
    }

    #[test]
    fn a_failed_leaf_observation_refuses_construction_whichever_leaf_fails() {
        for leaf in LEAVES {
            let failed = BackendError::Fault(format!("{leaf:?} observer unreachable"));
            assert_eq!(
                composite_with(leaf, scripted([Err(failed.clone())])).err(),
                Some(failed)
            );
        }
    }

    #[test]
    fn a_failed_leaf_observation_refuses_a_later_snapshot_and_the_next_one_is_complete() {
        for failing in LEAVES {
            let parts = LEAVES.map(rows_of);
            let failed = BackendError::Fault(format!("{failing:?} observer unreachable"));
            let [network, filesystem, broker] = LEAVES.map(|leaf| {
                let part = parts[leaf as usize].clone();
                let later = if leaf == failing {
                    Err(failed.clone())
                } else {
                    Ok(part.clone())
                };
                scripted([Ok(part.clone()), later, Ok(part)])
            });
            let mut backend = CompositeBackend::new(network, filesystem, broker).unwrap();
            assert_eq!(backend.snapshot_os_state(), Err(failed), "{failing:?}");
            assert_eq!(
                backend.snapshot_os_state(),
                Ok(union(&parts)),
                "{failing:?}"
            );
        }
    }

    #[test]
    fn a_fake_leaf_armed_to_fail_its_next_observation_refuses_construction() {
        for leaf in LEAVES {
            let mut armed = FakeBackend::new();
            armed.fail_next_observation("fake observer unreachable");
            assert_eq!(
                composite_with(leaf, Box::new(armed)).err(),
                Some(BackendError::Fault("fake observer unreachable".to_string()))
            );
        }
    }

    fn foreign_rows(leaf: Leaf) -> [(EnforcementSnapshot, String); 3] {
        let foreign = || prepared("foreign", foreign_capability(leaf));
        let (applied, recorded, unrecorded) = (foreign(), foreign(), foreign());
        let mut state = OsState::new();
        state.insert(applied.object()).unwrap();
        [
            (
                EnforcementSnapshot::new(state, Vec::new(), Vec::new()).unwrap(),
                format!(
                    "{leaf:?} leaf observes out-of-class {}",
                    applied.object().describe()
                ),
            ),
            (
                EnforcementSnapshot::new(
                    OsState::new(),
                    vec![issued(&recorded)],
                    vec![unfinished(&recorded)],
                )
                .unwrap(),
                format!(
                    "{leaf:?} leaf observes out-of-class issued record {} owned by `{}`",
                    recorded.handle(),
                    recorded.owner()
                ),
            ),
            (
                EnforcementSnapshot::new(OsState::new(), Vec::new(), vec![unfinished(&unrecorded)])
                    .unwrap(),
                format!(
                    "{leaf:?} leaf observes out-of-class incomplete effect {} owned by `{}`",
                    unrecorded.handle(),
                    unrecorded.owner()
                ),
            ),
        ]
    }

    #[test]
    fn an_out_of_class_row_in_any_collection_refuses_construction_and_every_later_snapshot() {
        for leaf in LEAVES {
            for (foreign, message) in foreign_rows(leaf) {
                let refused = Some(BackendError::Fault(message));
                assert_eq!(
                    composite_with(leaf, scripted([Ok(foreign.clone())])).err(),
                    refused,
                    "construction"
                );
                let mut backend = composite_with(
                    leaf,
                    scripted([
                        Ok(EnforcementSnapshot::empty()),
                        Ok(foreign.clone()),
                        Ok(foreign),
                    ]),
                )
                .unwrap();
                assert_eq!(
                    backend.snapshot_os_state().err(),
                    refused,
                    "first later call"
                );
                assert_eq!(
                    backend.snapshot_os_state().err(),
                    refused,
                    "second later call"
                );
            }
        }
    }

    #[test]
    fn a_leaf_that_reports_another_leafs_row_refuses_the_whole_union() {
        let parts = LEAVES.map(rows_of);
        let echoed_row = parts[0].state().objects().next().unwrap().clone();
        let mut echoing = parts[2].state().clone();
        echoing.insert(echoed_row.clone()).unwrap();
        let echo = EnforcementSnapshot::new(
            echoing,
            parts[2].grants().to_vec(),
            parts[2].incomplete().to_vec(),
        )
        .unwrap();
        let mut backend = CompositeBackend::new(
            scripted([Ok(parts[0].clone()), Ok(parts[0].clone())]),
            scripted([Ok(parts[1].clone()), Ok(parts[1].clone())]),
            scripted([Ok(parts[2].clone()), Ok(echo)]),
        )
        .unwrap();
        assert_eq!(
            backend.snapshot_os_state(),
            Err(BackendError::Fault(format!(
                "Broker leaf observes out-of-class {}",
                echoed_row.describe()
            )))
        );
    }
}
