use std::collections::BTreeMap;

use plasmosome_backend::{Capability, CellId, CellOwner, Grant, GrantKind, LedgerEntry, MockMode};
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

/// Collects the grants one owner asks for, in attach order. Replaying the
/// matching ledger runs their inverses in the opposite order.
pub struct GrantSequence {
    owner: CellOwner,
    grants: Vec<Grant>,
}

impl GrantSequence {
    /// Starts an empty sequence whose every grant is owned by `owner`, the plugin as attached to
    /// one cell.
    pub fn for_owner(owner: CellOwner) -> GrantSequence {
        GrantSequence {
            owner,
            grants: Vec::new(),
        }
    }

    /// Appends a capability the backend may change on a running cell.
    pub fn hot(self, capability: Capability) -> GrantSequence {
        self.push(capability, GrantKind::Hot)
    }

    /// Appends a capability fixed for the life of the cell's generation.
    pub fn generation_bound(self, capability: Capability) -> GrantSequence {
        self.push(capability, GrantKind::GenerationBound)
    }

    pub fn into_grants(self) -> Vec<Grant> {
        self.grants
    }

    fn push(mut self, capability: Capability, kind: GrantKind) -> GrantSequence {
        self.grants.push(Grant {
            owner: self.owner.clone(),
            capability,
            kind,
        });
        self
    }
}

/// The `Exact` effect that undoes `entry` by revoking its handle. The
/// description names the handle, so a replay report distinguishes two grants of
/// the same capability class.
pub fn exact_backend_effect(entry: &LedgerEntry) -> Effect {
    Effect::exact(
        format!(
            "{} granted {} as {}",
            entry.owner,
            entry.capability.class_str(),
            entry.handle
        ),
        InverseVia::Backend(entry.handle),
    )
}

/// Returns `Capability::SessionFile` holding exactly `path`. Nothing is checked or normalized,
/// so a test may pass an invalid path.
/// Test support only: production capabilities will come from spec 017's manifest resolver.
pub fn session_file(path: &str) -> Capability {
    Capability::SessionFile {
        path: path.to_string(),
    }
}

/// Returns `Capability::UdsSocket` holding exactly `path`. Nothing is checked or normalized, so
/// a test may pass an invalid path.
/// Test support only: production capabilities will come from spec 017's manifest resolver.
pub fn uds_socket(path: &str) -> Capability {
    Capability::UdsSocket {
        path: path.to_string(),
    }
}

/// Returns `Capability::ProxyMap` holding exactly `host` and `route`. Nothing is checked or
/// normalized, so a test may pass an invalid name.
/// Test support only: production capabilities will come from spec 017's manifest resolver.
pub fn proxy_map(host: &str, route: &str) -> Capability {
    Capability::ProxyMap {
        host: host.to_string(),
        route: route.to_string(),
    }
}

/// Returns `Capability::Broker` holding exactly `pid` and `name`. Nothing is checked or
/// normalized, so a test may pass an invalid name.
/// Test support only: production capabilities will come from spec 017's manifest resolver.
pub fn broker(pid: u32, name: &str) -> Capability {
    Capability::Broker {
        pid,
        name: name.to_string(),
    }
}

/// Returns `Capability::Mount` holding exactly `source` and `target`. Nothing is checked or
/// normalized, so a test may pass an invalid path.
/// Test support only: production capabilities will come from spec 017's manifest resolver.
pub fn mount(source: &str, target: &str) -> Capability {
    Capability::Mount {
        source: source.to_string(),
        target: target.to_string(),
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
    use super::*;

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
}
