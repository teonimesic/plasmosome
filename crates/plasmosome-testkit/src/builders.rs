use std::collections::BTreeMap;

use plasmosome_backend::{
    BrokerLaunch, Capability, CellId, CellOwner, FileAccess, GrantId, GrantKind, Handle, MockMode,
    MountRecipe, OsObject, ProxyRecipe, ProxyTransport, SessionFileRecipe, UdsRecipe, UniverseOp,
};
use plasmosome_core::manifest::{NetworkSpec, PlasmidManifest, ToolDeclaration};
use plasmosome_core::reconciler::{DesiredCell, DesiredState};
use plasmosome_core::state::{GenomeName, PlasmidRecord};
use plasmosome_ledger::{Effect, InverseVia};

const DEFAULT_HOST: &str = "api.plasmosome.test";
const DEFAULT_PORT: u16 = 443;
const DEFAULT_DRAIN_MS: u64 = 750;

/// Builds a `PlasmidManifest` without going through the TOML grammar, so a test
/// states only the fields it is about. The manifest always declares one network
/// host and a drain deadline; callers that care about either replace them.
pub struct ManifestBuilder {
    id: String,
    description: String,
    version: String,
    tools: Vec<ToolDeclaration>,
    hosts: Vec<String>,
    drain_ms: u64,
}

impl ManifestBuilder {
    /// Returns a builder for plasmid `id`, using `description` as its purpose, with no tools.
    /// Inputs are copied unchanged into trusted test data; use the manifest parser when
    /// declaration grammar must be validated.
    pub fn new(id: &str, description: &str) -> ManifestBuilder {
        ManifestBuilder {
            id: id.to_string(),
            description: description.to_string(),
            version: "0.1.0".to_string(),
            tools: Vec::new(),
            hosts: Vec::new(),
            drain_ms: DEFAULT_DRAIN_MS,
        }
    }

    /// Consumes the builder, appends the `tool` name and its `description` in declaration
    /// order, and returns the updated builder. Inputs are copied without grammar validation;
    /// callers must not treat building a fixture as proof that its TOML declaration parses.
    pub fn tool(mut self, tool: &str, description: &str) -> ManifestBuilder {
        self.tools.push(ToolDeclaration {
            name: tool.to_string(),
            description: description.to_string(),
        });
        self
    }

    /// Replaces the default host on the first call, then appends.
    pub fn host(mut self, host: &str) -> ManifestBuilder {
        self.hosts.push(host.to_string());
        self
    }

    pub fn drain_ms(mut self, drain_ms: u64) -> ManifestBuilder {
        self.drain_ms = drain_ms;
        self
    }

    pub fn build(self) -> PlasmidManifest {
        let hosts = if self.hosts.is_empty() {
            vec![DEFAULT_HOST.to_string()]
        } else {
            self.hosts
        };
        PlasmidManifest {
            id: self.id,
            description: self.description,
            version: self.version,
            wasm: None,
            network: Some(NetworkSpec {
                hosts,
                ports: vec![DEFAULT_PORT],
                pin_cidrs: Vec::new(),
            }),
            requires: Vec::new(),
            provides_tools: self.tools,
            secrets: Vec::new(),
            commands: None,
            workspace: None,
            mock: None,
            model: None,
            drain_ms: Some(self.drain_ms),
        }
    }
}

/// One grant its caller has prepared: the complete operation, at the fresh identity the caller
/// chose, and the kind of grant to ask for. Its inverse is known before the grant is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedGrant {
    pub operation: UniverseOp,
    pub kind: GrantKind,
}

impl PreparedGrant {
    /// The exact address the grant will hold: the operation's class and identity.
    pub fn handle(&self) -> Handle {
        self.operation.handle()
    }

    /// The `Exact` effect that undoes this grant by revoking its handle. The description names
    /// the owner, the class and the handle, so a replay report distinguishes two grants of the
    /// same capability class. Record it before calling `grant`.
    pub fn backend_effect(&self) -> Effect {
        Effect::exact(
            format!(
                "{} granted {} as {}",
                self.operation.owner(),
                self.operation.class().as_str(),
                self.handle()
            ),
            InverseVia::Backend(self.handle()),
        )
    }
}

/// Collects the capabilities one owner asks for, in attach order, and prepares them. Replaying
/// the matching ledger runs their inverses in the opposite order.
pub struct GrantClosure {
    owner: CellOwner,
    requests: Vec<(Capability, GrantKind)>,
}

impl GrantClosure {
    /// Starts an empty closure whose every grant is owned by `owner`, the plugin as attached to
    /// one cell.
    pub fn for_owner(owner: CellOwner) -> GrantClosure {
        GrantClosure {
            owner,
            requests: Vec::new(),
        }
    }

    /// Appends a capability the backend may change on a running cell.
    pub fn hot(mut self, capability: Capability) -> GrantClosure {
        self.requests.push((capability, GrantKind::Hot));
        self
    }

    /// Appends a capability fixed for the life of the cell's generation.
    pub fn generation_bound(mut self, capability: Capability) -> GrantClosure {
        self.requests.push((capability, GrantKind::GenerationBound));
        self
    }

    /// Prepares every grant, in attach order, each at a fresh `GrantId::new()`.
    pub fn prepare(self) -> Vec<PreparedGrant> {
        self.prepare_with(GrantId::new)
    }

    /// Prepares every grant with identities from `next_id`, in attach order. It draws every
    /// identity before it builds any operation, so a source that fails panics before anything
    /// is prepared.
    pub fn prepare_with(self, mut next_id: impl FnMut() -> GrantId) -> Vec<PreparedGrant> {
        let ids: Vec<GrantId> = self.requests.iter().map(|_| next_id()).collect();
        let owner = self.owner;
        self.requests
            .into_iter()
            .zip(ids)
            .map(|((capability, kind), id)| PreparedGrant {
                operation: OsObject {
                    id,
                    owner: owner.clone(),
                    capability,
                }
                .operation(),
                kind,
            })
            .collect()
    }
}

/// Returns `Capability::SessionFile` holding exactly `path` and a fixed recipe: empty contents,
/// read-only, at guest path `/run/plasmosome/session`. The path is neither checked nor
/// normalized, so a test may pass an invalid one; the recipe passes `validate`.
/// Test support only: production capabilities will come from spec 017's manifest resolver.
pub fn session_file(path: &str) -> Capability {
    Capability::SessionFile {
        path: path.to_string(),
        recipe: SessionFileRecipe {
            contents: Vec::new(),
            mode: FileAccess::ReadOnly,
            guest_path: "/run/plasmosome/session".to_string(),
        },
    }
}

/// Returns `Capability::UdsSocket` holding exactly `path` and a fixed recipe: upstream
/// `/run/plasmosome/upstream.sock`, guest path `/run/plasmosome/guest.sock`. The path is neither
/// checked nor normalized, so a test may pass an invalid one; the recipe passes `validate`.
/// Test support only: production capabilities will come from spec 017's manifest resolver.
pub fn uds_socket(path: &str) -> Capability {
    Capability::UdsSocket {
        path: path.to_string(),
        recipe: UdsRecipe {
            upstream: "/run/plasmosome/upstream.sock".to_string(),
            guest_path: "/run/plasmosome/guest.sock".to_string(),
        },
    }
}

/// Returns `Capability::ProxyMap` holding exactly `host` and `route` and a fixed recipe: TCP to
/// `upstream.plasmosome.test` port 443, private addresses not allowed. The names are neither
/// checked nor normalized, so a test may pass an invalid one; the recipe passes `validate`.
/// Test support only: production capabilities will come from spec 017's manifest resolver.
pub fn proxy_map(host: &str, route: &str) -> Capability {
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
}

/// Returns `Capability::Broker` holding exactly `name` and a fixed launch: the command
/// `/usr/libexec/plasmosome-broker`, control socket `/run/plasmosome/broker.control` and data
/// socket `/run/plasmosome/broker.data`. The name is neither checked nor normalized, so a test
/// may pass an invalid one; the launch passes `validate`.
/// Test support only: production capabilities will come from spec 017's manifest resolver.
pub fn broker(name: &str) -> Capability {
    Capability::Broker {
        name: name.to_string(),
        launch: BrokerLaunch {
            command: vec!["/usr/libexec/plasmosome-broker".to_string()],
            control_socket: "/run/plasmosome/broker.control".to_string(),
            data_socket: "/run/plasmosome/broker.data".to_string(),
        },
    }
}

/// Returns `Capability::Mount` holding exactly `source` and `target` and a read-only recipe.
/// The paths are neither checked nor normalized, so a test may pass an invalid one.
/// Test support only: production capabilities will come from spec 017's manifest resolver.
pub fn mount(source: &str, target: &str) -> Capability {
    Capability::Mount {
        source: source.to_string(),
        target: target.to_string(),
        recipe: MountRecipe {
            access: FileAccess::ReadOnly,
        },
    }
}

/// Builds a `DesiredState` one cell and one plasmid at a time. A plasmid named
/// for a cell that was never declared creates that cell without a genome.
pub struct DesiredStateBuilder {
    generation: u64,
    cells: BTreeMap<CellId, DesiredCell>,
}

impl DesiredStateBuilder {
    pub fn at_generation(generation: u64) -> DesiredStateBuilder {
        DesiredStateBuilder {
            generation,
            cells: BTreeMap::new(),
        }
    }

    pub fn cell(mut self, id: &str, genome: &str) -> DesiredStateBuilder {
        let entry = self.cells.entry(CellId::from(id)).or_insert(DesiredCell {
            genome: None,
            plasmids: Vec::new(),
        });
        entry.genome = Some(GenomeName::from(genome));
        self
    }

    pub fn plasmid_in(mut self, cell: &str, plasmid: &str, mock: MockMode) -> DesiredStateBuilder {
        self.cells
            .entry(CellId::from(cell))
            .or_insert(DesiredCell {
                genome: None,
                plasmids: Vec::new(),
            })
            .plasmids
            .push(PlasmidRecord {
                plasmid: plasmid.to_string(),
                mock,
            });
        self
    }

    pub fn build(self) -> DesiredState {
        DesiredState {
            generation: self.generation,
            cells: self.cells,
        }
    }
}

#[cfg(test)]
mod tests {
    use plasmosome_backend::{PluginId, UniverseClass};

    use super::*;

    fn github_pr_in(cell: &str) -> CellOwner {
        CellOwner {
            cell: CellId::from(cell),
            plugin: PluginId::from("github-pr"),
        }
    }

    #[test]
    fn a_closure_prepares_each_capability_in_attach_order_at_the_identity_drawn_for_it() {
        let owner = github_pr_in("cell-1");
        let requested = [
            (mount("/src/repo", "/workspace"), GrantKind::Hot),
            (broker("egressd"), GrantKind::GenerationBound),
            (session_file("/skills/pr.md"), GrantKind::Hot),
        ];
        let ids: Vec<GrantId> = requested.iter().map(|_| GrantId::new()).collect();
        let mut source = ids.clone().into_iter();
        let prepared = GrantClosure::for_owner(owner.clone())
            .hot(requested[0].0.clone())
            .generation_bound(requested[1].0.clone())
            .hot(requested[2].0.clone())
            .prepare_with(|| source.next().expect("one identity per capability"));
        assert_eq!(source.next(), None, "every identity drawn, and no more");
        assert_eq!(prepared.len(), requested.len());
        for ((grant, (capability, kind)), id) in prepared.iter().zip(requested).zip(ids) {
            assert_eq!(grant.operation.id(), id);
            assert_eq!(grant.operation.owner(), &owner);
            assert_eq!(grant.operation.capability(), capability);
            assert_eq!(grant.kind, kind);
        }
    }

    #[test]
    fn prepare_draws_a_fresh_identity_for_every_grant() {
        let closure = || {
            GrantClosure::for_owner(github_pr_in("cell-1"))
                .hot(session_file("/skills/pr.md"))
                .hot(session_file("/skills/pr.md"))
        };
        let mut ids: Vec<GrantId> = closure()
            .prepare()
            .into_iter()
            .chain(closure().prepare())
            .map(|grant| grant.operation.id())
            .collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4);
    }

    #[test]
    fn an_empty_closure_draws_no_identity() {
        let prepared = GrantClosure::for_owner(github_pr_in("cell-1"))
            .prepare_with(|| panic!("an empty closure must not draw an identity"));
        assert!(prepared.is_empty());
    }

    #[test]
    fn a_prepared_grant_names_its_exact_address_and_the_revoke_that_undoes_it() {
        let grant = GrantClosure::for_owner(github_pr_in("cell-1"))
            .hot(uds_socket("/run/plasmosome/egressd.uds"))
            .prepare()
            .remove(0);
        let handle = Handle {
            class: UniverseClass::UdsPath,
            id: grant.operation.id(),
        };
        assert_eq!(grant.handle(), handle);
        assert_eq!(
            grant.backend_effect(),
            Effect::exact(
                format!(
                    "\"cell-1\"/\"github-pr\" granted uds-path as uds-path/{}",
                    handle.id
                ),
                InverseVia::Backend(handle),
            )
        );
    }

    #[test]
    fn the_inverse_description_tells_two_grants_of_one_class_apart() {
        let prepared = GrantClosure::for_owner(github_pr_in("cell-1"))
            .hot(session_file("/skills/pr.md"))
            .hot(session_file("/skills/review.md"))
            .prepare();
        assert_ne!(
            prepared[0].backend_effect().description,
            prepared[1].backend_effect().description
        );
        let elsewhere = GrantClosure::for_owner(github_pr_in("cell-2"))
            .hot(session_file("/skills/pr.md"))
            .prepare_with(|| prepared[0].operation.id());
        assert_ne!(
            elsewhere[0].backend_effect().description,
            prepared[0].backend_effect().description,
            "the description names the owner's cell"
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
}
