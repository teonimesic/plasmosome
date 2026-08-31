use std::time::Duration;

use plasmosome_backend::{
    Capability, Diff, DrainSpec, EnforcementBackend, FakeBackend, PluginId, ResidueReport,
};
use plasmosome_core::state::MockMode;
use plasmosome_core::{CellId, ToolRegistry};
use plasmosome_ledger::{Closure, Ledger};
use plasmosome_testkit::builders::{
    DesiredStateBuilder, GrantSequence, ManifestBuilder, exact_backend_effect,
};

#[test]
fn attach_then_detach_leaves_no_residue_after_lifo_replay() {
    let manifest = ManifestBuilder::new("github-pr")
        .tool("pr.read")
        .tool("pr.comment")
        .host("api.github.com")
        .drain_ms(750)
        .build();
    let plugin = PluginId::from(manifest.id.as_str());

    let registry = ToolRegistry::new();
    registry.register(&plugin, &manifest.provides_tools);
    assert_eq!(registry.list(), vec!["pr.comment", "pr.read"]);
    assert_eq!(registry.lookup("pr.read").unwrap().plugin, plugin);

    let desired = DesiredStateBuilder::at_generation(1)
        .cell("cell-1", "researcher")
        .plasmid_in("cell-1", &manifest.id, MockMode::Simulate)
        .build();
    let cell = desired
        .cells
        .get(&CellId::from("cell-1"))
        .expect("the builder declared cell-1");
    assert_eq!(cell.plasmids[0].plasmid, manifest.id);

    let mut backend = FakeBackend::new();
    let before = backend.snapshot_os_state();
    assert!(before.is_empty());

    let mut ledger = Ledger::new(plugin.clone());
    let mut attached = Vec::new();
    for grant in GrantSequence::for_plugin(&manifest.id)
        .hot(Capability::Mount {
            source: "/src/repo".to_string(),
            target: "/workspace".to_string(),
        })
        .hot(Capability::UdsSocket {
            path: "/workspace/run/egressd.uds".to_string(),
        })
        .hot(Capability::ProxyMap {
            host: manifest.network.as_ref().expect("a host").hosts[0].clone(),
            route: "splice".to_string(),
        })
        .into_grants()
    {
        let effect = exact_backend_effect(&backend.grant(grant));
        attached.push(effect.description.clone());
        ledger.push(effect);
    }
    assert_eq!(ledger.len(), 3);
    assert_eq!(backend.snapshot_os_state().len(), 3);

    let Closure::ExternalFree(mut sealed) = ledger.close() else {
        panic!("a ledger of exact inverses closes without an operator assertion");
    };
    let drain = DrainSpec::graceful(Duration::from_millis(
        manifest.drain_ms.expect("the builder set a drain deadline"),
    ));
    let report = sealed
        .detach(&mut backend, drain)
        .expect("replaying exact inverses cannot fail");

    let mut lifo = attached;
    lifo.reverse();
    assert_eq!(
        report.replayed, lifo,
        "detach must replay the ledger last effect first"
    );
    assert!(report.asserted.is_empty());
    assert_eq!(report.delayed_discarded, 0);
    assert!(report.forced.is_none());

    assert_eq!(
        registry.withdraw_plugin(&plugin),
        vec!["pr.comment".to_string(), "pr.read".to_string()]
    );
    assert!(registry.is_empty());

    let after = backend.snapshot_os_state();
    let residue = ResidueReport::from_diff(Diff::between(&before, &after), Vec::new());
    assert_eq!(residue, ResidueReport::Empty, "{residue}");
    assert!(after.is_empty());
}
