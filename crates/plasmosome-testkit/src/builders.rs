use std::collections::BTreeMap;

use plasmosome_backend::{Capability, Grant, GrantKind, LedgerEntry, PluginId};
use plasmosome_core::manifest::{NetworkSpec, PlasmidManifest};
use plasmosome_core::reconciler::{DesiredCell, DesiredState};
use plasmosome_core::state::{CellId, GenomeName, MockMode, PlasmidRecord};
use plasmosome_ledger::{Effect, InverseVia};

const DEFAULT_HOST: &str = "api.plasmosome.test";
const DEFAULT_PORT: u16 = 443;
const DEFAULT_DRAIN_MS: u64 = 750;

/// Builds a `PlasmidManifest` without going through the TOML grammar, so a test
/// states only the fields it is about. The manifest always declares one network
/// host and a drain deadline; callers that care about either replace them.
pub struct ManifestBuilder {
    id: String,
    version: String,
    tools: Vec<String>,
    hosts: Vec<String>,
    drain_ms: u64,
}

impl ManifestBuilder {
    /// Starts a manifest for the plasmid `id`, version `0.1.0`, with no tools.
    pub fn new(id: &str) -> ManifestBuilder {
        ManifestBuilder {
            id: id.to_string(),
            version: "0.1.0".to_string(),
            tools: Vec::new(),
            hosts: Vec::new(),
            drain_ms: DEFAULT_DRAIN_MS,
        }
    }

    /// Adds a tool the plasmid provides, in the order tools are declared.
    pub fn tool(mut self, tool: &str) -> ManifestBuilder {
        self.tools.push(tool.to_string());
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

/// Collects the grants one plugin asks for, in attach order. Replaying the
/// matching ledger runs their inverses in the opposite order.
pub struct GrantSequence {
    plugin: PluginId,
    grants: Vec<Grant>,
}

impl GrantSequence {
    pub fn for_plugin(plugin: &str) -> GrantSequence {
        GrantSequence {
            plugin: PluginId::from(plugin),
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
            plugin: self.plugin.clone(),
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
            entry.plugin,
            entry.capability.class_str(),
            entry.handle
        ),
        InverseVia::Backend(entry.handle),
    )
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
