use crate::backend::Capability;
use crate::recipe::{
    BrokerLaunch, FileAccess, MountRecipe, ProxyRecipe, ProxyTransport, SessionFileRecipe,
    UdsRecipe,
};
use crate::universe::{CellOwner, GrantId, UniverseOp};

pub(crate) fn session_file(path: &str) -> Capability {
    Capability::SessionFile {
        path: path.to_string(),
        recipe: session_recipe(),
    }
}

pub(crate) fn uds_socket(path: &str) -> Capability {
    Capability::UdsSocket {
        path: path.to_string(),
        recipe: uds_recipe(),
    }
}

pub(crate) fn proxy_map(host: &str, route: &str) -> Capability {
    Capability::ProxyMap {
        host: host.to_string(),
        route: route.to_string(),
        recipe: proxy_recipe(),
    }
}

pub(crate) fn broker(name: &str) -> Capability {
    Capability::Broker {
        name: name.to_string(),
        launch: launch(),
    }
}

pub(crate) fn mount(source: &str, target: &str) -> Capability {
    Capability::Mount {
        source: source.to_string(),
        target: target.to_string(),
        recipe: mount_recipe(),
    }
}

pub(crate) fn session_recipe() -> SessionFileRecipe {
    SessionFileRecipe {
        contents: Vec::new(),
        mode: FileAccess::ReadOnly,
        guest_path: "/run/plasmosome/session".to_string(),
    }
}

pub(crate) fn uds_recipe() -> UdsRecipe {
    UdsRecipe {
        upstream: "/run/plasmosome/upstream.sock".to_string(),
        guest_path: "/run/plasmosome/guest.sock".to_string(),
    }
}

pub(crate) fn proxy_recipe() -> ProxyRecipe {
    ProxyRecipe {
        transport: ProxyTransport::Tcp,
        destination: "upstream.plasmosome.test".to_string(),
        port: 443,
        allow_private: false,
    }
}

pub(crate) fn mount_recipe() -> MountRecipe {
    MountRecipe {
        access: FileAccess::ReadOnly,
    }
}

pub(crate) fn launch() -> BrokerLaunch {
    BrokerLaunch {
        command: vec!["/usr/libexec/plasmosome-broker".to_string()],
        control_socket: "/run/plasmosome/broker.control".to_string(),
        data_socket: "/run/plasmosome/broker.data".to_string(),
    }
}

pub(crate) fn op(id: GrantId, owner: CellOwner, capability: Capability) -> UniverseOp {
    match capability {
        Capability::SessionFile { path, recipe } => UniverseOp::WriteSessionFile {
            id,
            path,
            recipe,
            owner,
        },
        Capability::UdsSocket { path, recipe } => UniverseOp::BindUds {
            id,
            path,
            recipe,
            owner,
        },
        Capability::ProxyMap {
            host,
            route,
            recipe,
        } => UniverseOp::SetProxyMap {
            id,
            host,
            route,
            recipe,
            owner,
        },
        Capability::Broker { name, launch } => UniverseOp::SpawnBroker {
            id,
            name,
            launch,
            owner,
        },
        Capability::Mount {
            source,
            target,
            recipe,
        } => UniverseOp::AddMount {
            id,
            source,
            target,
            recipe,
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
    fn session_file_holds_exactly_its_path_unchecked_and_the_default_recipe() {
        for path in [
            "/skills/pr.md",
            "skills/../pr.md/",
            "/skills/pr\0.md",
            "",
            " /skills/pr.md\n",
            "/skills/pr.md/",
            "/Skills/PR.md",
            "/skills//pr.md",
        ] {
            assert_eq!(
                session_file(path),
                Capability::SessionFile {
                    path: path.to_string(),
                    recipe: SessionFileRecipe {
                        contents: Vec::new(),
                        mode: FileAccess::ReadOnly,
                        guest_path: "/run/plasmosome/session".to_string(),
                    },
                }
            );
        }
    }

    #[test]
    fn uds_socket_holds_exactly_its_path_unchecked_and_the_default_recipe() {
        for path in [
            "/run/plasmosome/egressd.uds",
            "run//egressd.uds",
            "",
            "\t/run/plasmosome/egressd.uds ",
            "/run/plasmosome/egressd.uds/",
            "/RUN/Plasmosome/EgressD.uds",
            "/run/egress\0d.uds",
        ] {
            assert_eq!(
                uds_socket(path),
                Capability::UdsSocket {
                    path: path.to_string(),
                    recipe: UdsRecipe {
                        upstream: "/run/plasmosome/upstream.sock".to_string(),
                        guest_path: "/run/plasmosome/guest.sock".to_string(),
                    },
                }
            );
        }
    }

    #[test]
    fn proxy_map_holds_exactly_its_host_and_route_in_place_and_the_default_recipe() {
        for (host, route) in [
            ("api.github.com", "splice"),
            ("splice", "api.github.com"),
            ("api.github\0.com", "../relative/route/"),
            (" api.github.com\t", "\nsplice "),
            ("API.GitHub.com", "Splice"),
            ("api.github.com.", "/splice/"),
            ("", "spl\0ice//"),
            ("api.github.com", ""),
        ] {
            assert_eq!(
                proxy_map(host, route),
                Capability::ProxyMap {
                    host: host.to_string(),
                    route: route.to_string(),
                    recipe: ProxyRecipe {
                        transport: ProxyTransport::Tcp,
                        destination: "upstream.plasmosome.test".to_string(),
                        port: 443,
                        allow_private: false,
                    },
                }
            );
        }
    }

    #[test]
    fn broker_holds_exactly_its_name_and_the_default_launch() {
        for name in ["egressd", "egress\0d", "", " egressd\n", "EgressD"] {
            assert_eq!(
                broker(name),
                Capability::Broker {
                    name: name.to_string(),
                    launch: BrokerLaunch {
                        command: vec!["/usr/libexec/plasmosome-broker".to_string()],
                        control_socket: "/run/plasmosome/broker.control".to_string(),
                        data_socket: "/run/plasmosome/broker.data".to_string(),
                    },
                }
            );
        }
    }

    #[test]
    fn mount_holds_exactly_its_source_and_target_in_place_and_the_default_recipe() {
        for (source, target) in [
            ("/srv/repo", "/workspace"),
            ("/workspace", "/srv/repo"),
            ("srv/repo", "/workspace/"),
            ("/srv/repo/", "/workspace/"),
            (" /srv/repo", "/workspace\n"),
            ("/SRV/Repo", "/Workspace"),
            ("/srv/re\0po\n", " /work//space"),
            ("", ""),
            ("/srv//repo", "/work\0space"),
        ] {
            assert_eq!(
                mount(source, target),
                Capability::Mount {
                    source: source.to_string(),
                    target: target.to_string(),
                    recipe: MountRecipe {
                        access: FileAccess::ReadOnly,
                    },
                }
            );
        }
    }

    #[test]
    fn every_factory_given_valid_values_returns_a_capability_that_validates() {
        for capability in [
            session_file("/skills/pr.md"),
            uds_socket("/run/plasmosome/egressd.uds"),
            proxy_map("api.github.com", "splice"),
            broker("egressd"),
            mount("/srv/repo", "/workspace"),
        ] {
            assert_eq!(capability.validate(), Ok(()), "{capability:?}");
        }
    }

    #[test]
    fn op_writes_a_session_file_with_its_id_owner_path_and_recipe() {
        assert_op_builds(session_file("/skills/pr.md"), |id, owner| {
            UniverseOp::WriteSessionFile {
                id,
                path: "/skills/pr.md".to_string(),
                recipe: session_recipe(),
                owner,
            }
        });
    }

    #[test]
    fn op_binds_a_socket_with_its_id_owner_path_and_recipe() {
        assert_op_builds(uds_socket("/run/plasmosome/egressd.uds"), |id, owner| {
            UniverseOp::BindUds {
                id,
                path: "/run/plasmosome/egressd.uds".to_string(),
                recipe: uds_recipe(),
                owner,
            }
        });
    }

    #[test]
    fn op_sets_a_proxy_map_with_its_id_owner_host_route_and_recipe() {
        assert_op_builds(proxy_map("api.github.com", "splice"), |id, owner| {
            UniverseOp::SetProxyMap {
                id,
                host: "api.github.com".to_string(),
                route: "splice".to_string(),
                recipe: proxy_recipe(),
                owner,
            }
        });
    }

    #[test]
    fn op_spawns_a_broker_with_its_id_owner_name_and_launch() {
        assert_op_builds(broker("egressd"), |id, owner| UniverseOp::SpawnBroker {
            id,
            name: "egressd".to_string(),
            launch: launch(),
            owner,
        });
    }

    #[test]
    fn op_adds_a_mount_with_its_id_owner_source_target_and_recipe() {
        assert_op_builds(mount("/srv/repo", "/workspace"), |id, owner| {
            UniverseOp::AddMount {
                id,
                source: "/srv/repo".to_string(),
                target: "/workspace".to_string(),
                recipe: mount_recipe(),
                owner,
            }
        });
    }
}
