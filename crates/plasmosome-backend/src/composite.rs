use std::collections::BTreeMap;

use crate::backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, Grant, Handle, LedgerEntry,
};
use crate::universe::{OsObject, OsState, UniverseClass, UniverseOp, UniverseRemoval};

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
    routes: BTreeMap<u64, Leaf>,
    next_handle: u64,
}

impl CompositeBackend {
    pub fn new(
        network: Box<dyn EnforcementBackend>,
        filesystem: Box<dyn EnforcementBackend>,
        broker: Box<dyn EnforcementBackend>,
    ) -> CompositeBackend {
        CompositeBackend {
            network,
            filesystem,
            broker,
            routes: BTreeMap::new(),
            next_handle: 0,
        }
    }

    fn leaf_for(&mut self, capability: &Capability) -> &mut dyn EnforcementBackend {
        match capability {
            Capability::ProxyMap { .. } | Capability::UdsSocket { .. } => self.network.as_mut(),
            Capability::SessionFile { .. } | Capability::Mount { .. } => self.filesystem.as_mut(),
            Capability::Broker { .. } => self.broker.as_mut(),
        }
    }

    fn leaf_named(&mut self, leaf: Leaf) -> &mut dyn EnforcementBackend {
        match leaf {
            Leaf::Network => self.network.as_mut(),
            Leaf::Filesystem => self.filesystem.as_mut(),
            Leaf::Broker => self.broker.as_mut(),
        }
    }

    fn mint_handle(&mut self, leaf: Leaf) -> Handle {
        self.next_handle += 1;
        self.routes.insert(self.next_handle, leaf);
        Handle(self.next_handle)
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
        let leaf = match grant.capability {
            Capability::ProxyMap { .. } | Capability::UdsSocket { .. } => Leaf::Network,
            Capability::SessionFile { .. } | Capability::Mount { .. } => Leaf::Filesystem,
            Capability::Broker { .. } => Leaf::Broker,
        };
        let mut entry = self.leaf_named(leaf).grant(grant);
        entry.handle = self.mint_handle(leaf);
        entry
    }

    fn revoke(&mut self, handle: Handle, drain: DrainSpec) -> Result<LedgerEntry, BackendError> {
        let leaf = self
            .routes
            .get(&handle.raw())
            .copied()
            .ok_or(BackendError::UnknownHandle { handle })?;
        let entry = self.leaf_named(leaf).revoke(handle, drain)?;
        self.routes.remove(&handle.raw());
        Ok(entry)
    }

    fn snapshot_os_state(&self) -> OsState {
        let mut union = self.network.snapshot_os_state();
        for object in self.filesystem.snapshot_os_state().objects() {
            union.insert(object.clone());
        }
        for object in self.broker.snapshot_os_state().objects() {
            union.insert(object.clone());
        }
        union
    }

    fn apply(&mut self, op: UniverseOp) -> Result<(), BackendError> {
        self.leaf_for(&capability_of_op(&op)).apply(op)
    }

    fn apply_removal(&mut self, removal: UniverseRemoval) -> Result<(), BackendError> {
        let mut last = self.network.apply_removal(removal.clone());
        if last.is_ok() {
            return Ok(());
        }
        last = self.filesystem.apply_removal(removal.clone());
        if last.is_ok() {
            return Ok(());
        }
        self.broker.apply_removal(removal)
    }

    fn plant(&mut self, object: OsObject) {
        match object.class {
            UniverseClass::ProxyMap | UniverseClass::UdsPath => self.network.plant(object),
            UniverseClass::SessionFile | UniverseClass::Mount => self.filesystem.plant(object),
            UniverseClass::BrokerPid => self.broker.plant(object),
        }
    }
}

fn capability_of_op(op: &UniverseOp) -> Capability {
    match op {
        UniverseOp::WriteSessionFile { path, .. } => Capability::SessionFile { path: path.clone() },
        UniverseOp::BindUds { path, .. } => Capability::UdsSocket { path: path.clone() },
        UniverseOp::SetProxyMap { host, route, .. } => Capability::ProxyMap {
            host: host.clone(),
            route: route.clone(),
        },
        UniverseOp::SpawnBroker { pid, name, .. } => Capability::Broker {
            pid: *pid,
            name: name.clone(),
        },
        UniverseOp::AddMount { source, target, .. } => Capability::Mount {
            source: source.clone(),
            target: target.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::GrantKind;
    use crate::fake::FakeBackend;
    use crate::universe::{Diff, PluginId, UniverseClass};

    fn fake() -> Box<dyn EnforcementBackend> {
        Box::new(FakeBackend::new())
    }

    #[test]
    fn proxy_and_socket_capabilities_route_to_the_network_leaf() {
        let mut composite = CompositeBackend::new(fake(), fake(), fake());
        composite.grant(Grant {
            plugin: PluginId::from("github"),
            capability: Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "splice".to_string(),
            },
            kind: GrantKind::Hot,
        });
        composite.grant(Grant {
            plugin: PluginId::from("network"),
            capability: Capability::UdsSocket {
                path: "/run/ak/egressd.uds".to_string(),
            },
            kind: GrantKind::Hot,
        });
        assert_eq!(composite.leaf_snapshot(Leaf::Network).len(), 2);
        assert_eq!(composite.leaf_snapshot(Leaf::Filesystem).len(), 0);
    }

    #[test]
    fn session_files_route_to_the_filesystem_leaf_and_brokers_to_the_broker_leaf() {
        let mut composite = CompositeBackend::new(fake(), fake(), fake());
        let file = composite.grant(Grant {
            plugin: PluginId::from("github-pr"),
            capability: Capability::SessionFile {
                path: "skills/pr.md".to_string(),
            },
            kind: GrantKind::Hot,
        });
        let broker = composite.grant(Grant {
            plugin: PluginId::from("network"),
            capability: Capability::Broker {
                pid: 9,
                name: "egressd".to_string(),
            },
            kind: GrantKind::Hot,
        });
        assert_ne!(file.handle, broker.handle);
        assert_eq!(composite.leaf_snapshot(Leaf::Filesystem).len(), 1);
        assert_eq!(composite.leaf_snapshot(Leaf::Broker).len(), 1);
        assert!(
            composite
                .snapshot_os_state()
                .contains(UniverseClass::SessionFile, "session/skills/pr.md")
        );
        assert!(
            composite
                .snapshot_os_state()
                .contains(UniverseClass::BrokerPid, "broker/9")
        );
    }

    #[test]
    fn the_composite_snapshot_is_the_union_of_its_leaves() {
        let mut network = FakeBackend::new();
        let mut filesystem = FakeBackend::new();
        network.grant(Grant {
            plugin: PluginId::from("network"),
            capability: Capability::UdsSocket {
                path: "/run/ak/egressd.uds".to_string(),
            },
            kind: GrantKind::Hot,
        });
        filesystem.grant(Grant {
            plugin: PluginId::from("github-pr"),
            capability: Capability::SessionFile {
                path: "skills/pr.md".to_string(),
            },
            kind: GrantKind::Hot,
        });
        let composite = CompositeBackend::new(Box::new(network), Box::new(filesystem), fake());
        assert_eq!(composite.snapshot_os_state().len(), 2);
    }

    #[test]
    fn a_grant_revoked_through_the_composite_leaves_no_residue() {
        let mut composite = CompositeBackend::new(fake(), fake(), fake());
        let before = composite.snapshot_os_state();
        let entry = composite.grant(Grant {
            plugin: PluginId::from("workspace-bind"),
            capability: Capability::Mount {
                source: "~/repo".to_string(),
                target: "/workspace".to_string(),
            },
            kind: GrantKind::Hot,
        });
        composite
            .revoke(
                entry.handle,
                DrainSpec::graceful(std::time::Duration::from_millis(1)),
            )
            .unwrap();
        assert!(Diff::between(&before, &composite.snapshot_os_state()).is_empty());
    }
}
