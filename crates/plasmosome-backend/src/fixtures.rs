use crate::backend::Capability;
use crate::universe::{CellOwner, GrantId, UniverseOp};

pub(crate) fn session_file(path: &str) -> Capability {
    Capability::SessionFile {
        path: path.to_string(),
    }
}

pub(crate) fn uds_socket(path: &str) -> Capability {
    Capability::UdsSocket {
        path: path.to_string(),
    }
}

pub(crate) fn proxy_map(host: &str, route: &str) -> Capability {
    Capability::ProxyMap {
        host: host.to_string(),
        route: route.to_string(),
    }
}

pub(crate) fn broker(pid: u32, name: &str) -> Capability {
    Capability::Broker {
        pid,
        name: name.to_string(),
    }
}

pub(crate) fn mount(source: &str, target: &str) -> Capability {
    Capability::Mount {
        source: source.to_string(),
        target: target.to_string(),
    }
}

pub(crate) fn op(id: GrantId, owner: CellOwner, capability: Capability) -> UniverseOp {
    match capability {
        Capability::SessionFile { path } => UniverseOp::WriteSessionFile { id, path, owner },
        Capability::UdsSocket { path } => UniverseOp::BindUds { id, path, owner },
        Capability::ProxyMap { host, route } => UniverseOp::SetProxyMap {
            id,
            host,
            route,
            owner,
        },
        Capability::Broker { pid, name } => UniverseOp::SpawnBroker {
            id,
            pid,
            name,
            owner,
        },
        Capability::Mount { source, target } => UniverseOp::AddMount {
            id,
            source,
            target,
            owner,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universe::{CellId, OsObject, PluginId};

    fn owner(cell: &str, plugin: &str) -> CellOwner {
        CellOwner {
            cell: CellId::from(cell),
            plugin: PluginId::from(plugin),
        }
    }

    fn assert_op_builds(
        capability: Capability,
        literal: impl FnOnce(GrantId, CellOwner) -> UniverseOp,
    ) {
        let id = GrantId::new();
        let owner = owner("cell-7", "audit");
        let built = op(id, owner.clone(), capability.clone());
        assert_eq!(built, literal(id, owner.clone()));
        assert_eq!(built.id(), id);
        assert_eq!(built.class(), capability.class());
        assert_eq!(
            built.object(),
            OsObject {
                id,
                owner,
                capability,
            }
        );
    }

    #[test]
    fn session_file_holds_exactly_its_path_unchecked() {
        for path in [
            "/skills/pr.md",
            "skills/../pr.md/",
            "/skills/pr\0.md",
            "",
            " /skills/pr.md\n",
            "/skills/pr.md/",
            "/Skills/PR.md",
        ] {
            assert_eq!(
                session_file(path),
                Capability::SessionFile {
                    path: path.to_string(),
                }
            );
        }
    }

    #[test]
    fn uds_socket_holds_exactly_its_path_unchecked() {
        for path in [
            "/run/plasmosome/egressd.uds",
            "run//egressd.uds",
            "",
            "\t/run/plasmosome/egressd.uds ",
            "/run/plasmosome/egressd.uds/",
            "/RUN/Plasmosome/EgressD.uds",
        ] {
            assert_eq!(
                uds_socket(path),
                Capability::UdsSocket {
                    path: path.to_string(),
                }
            );
        }
    }

    #[test]
    fn proxy_map_holds_exactly_its_host_and_route_in_place() {
        for (host, route) in [
            ("api.github.com", "splice"),
            ("splice", "api.github.com"),
            ("api.github\0.com", "../relative/route/"),
            (" api.github.com\t", "\nsplice "),
            ("API.GitHub.com", "Splice"),
            ("api.github.com.", "/splice/"),
        ] {
            assert_eq!(
                proxy_map(host, route),
                Capability::ProxyMap {
                    host: host.to_string(),
                    route: route.to_string(),
                }
            );
        }
    }

    #[test]
    fn broker_holds_exactly_its_pid_and_name() {
        for (pid, name) in [
            (4242, "egressd"),
            (0, "egress\0d"),
            (u32::MAX, ""),
            (7, " egressd\n"),
            (31337, "EgressD"),
        ] {
            assert_eq!(
                broker(pid, name),
                Capability::Broker {
                    pid,
                    name: name.to_string(),
                }
            );
        }
    }

    #[test]
    fn mount_holds_exactly_its_source_and_target_in_place() {
        for (source, target) in [
            ("/srv/repo", "/workspace"),
            ("/workspace", "/srv/repo"),
            ("srv/repo", "/workspace/"),
            ("/srv/repo/", "/workspace/"),
            (" /srv/repo", "/workspace\n"),
            ("/SRV/Repo", "/Workspace"),
        ] {
            assert_eq!(
                mount(source, target),
                Capability::Mount {
                    source: source.to_string(),
                    target: target.to_string(),
                }
            );
        }
    }

    #[test]
    fn op_writes_a_session_file_with_its_id_owner_and_path() {
        assert_op_builds(
            Capability::SessionFile {
                path: "/skills/pr.md".to_string(),
            },
            |id, owner| UniverseOp::WriteSessionFile {
                id,
                path: "/skills/pr.md".to_string(),
                owner,
            },
        );
    }

    #[test]
    fn op_binds_a_socket_with_its_id_owner_and_path() {
        assert_op_builds(
            Capability::UdsSocket {
                path: "/run/plasmosome/egressd.uds".to_string(),
            },
            |id, owner| UniverseOp::BindUds {
                id,
                path: "/run/plasmosome/egressd.uds".to_string(),
                owner,
            },
        );
    }

    #[test]
    fn op_sets_a_proxy_map_with_its_id_owner_host_and_route() {
        assert_op_builds(
            Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "splice".to_string(),
            },
            |id, owner| UniverseOp::SetProxyMap {
                id,
                host: "api.github.com".to_string(),
                route: "splice".to_string(),
                owner,
            },
        );
    }

    #[test]
    fn op_spawns_a_broker_with_its_id_owner_pid_and_name() {
        assert_op_builds(
            Capability::Broker {
                pid: 4242,
                name: "egressd".to_string(),
            },
            |id, owner| UniverseOp::SpawnBroker {
                id,
                pid: 4242,
                name: "egressd".to_string(),
                owner,
            },
        );
    }

    #[test]
    fn op_adds_a_mount_with_its_id_owner_source_and_target() {
        assert_op_builds(
            Capability::Mount {
                source: "/srv/repo".to_string(),
                target: "/workspace".to_string(),
            },
            |id, owner| UniverseOp::AddMount {
                id,
                source: "/srv/repo".to_string(),
                target: "/workspace".to_string(),
                owner,
            },
        );
    }
}
