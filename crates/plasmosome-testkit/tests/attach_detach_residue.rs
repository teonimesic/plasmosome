use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::Duration;

use plasmosome_backend::{
    BackendError, CellId, CellOwner, Diff, DrainSpec, EnforcementBackend, FakeBackend, GrantId,
    LedgerEntry, MockMode, PluginId, ResidueReport,
};
use plasmosome_core::ToolRegistry;
use plasmosome_core::manifest::PlasmidManifest;
use plasmosome_ledger::{Closure, DetachError, Ledger};
use plasmosome_testkit::builders::{
    DesiredStateBuilder, GrantClosure, ManifestBuilder, PreparedGrant, mount, proxy_map, uds_socket,
};

fn github_pr_closure(cell: &CellId, plugin: &PluginId, host: &str) -> GrantClosure {
    GrantClosure::for_owner(CellOwner {
        cell: cell.clone(),
        plugin: plugin.clone(),
    })
    .hot(mount("/src/repo", "/workspace"))
    .hot(uds_socket("/workspace/run/egressd.uds"))
    .hot(proxy_map(host, "splice"))
}

fn prepare_then_grant(
    backend: &mut FakeBackend,
    log: &Path,
    plugin: &PluginId,
    prepared: Vec<PreparedGrant>,
) -> Ledger {
    let mut ledger = Ledger::new(plugin.clone());
    for grant in &prepared {
        ledger.push(grant.backend_effect());
    }
    ledger
        .append_to_file(log)
        .expect("the prepared closure reaches the log");
    let durable = Ledger::open_file(log).expect("the log reopens");
    assert_eq!(
        durable.effects(),
        ledger.effects(),
        "the log must hold every inverse, in order, before the first grant"
    );
    assert!(backend.snapshot_os_state().unwrap().is_empty());
    for grant in prepared {
        let entry = backend
            .grant(grant.operation.clone(), grant.kind)
            .expect("a prepared grant succeeds on the fake");
        assert_eq!(
            entry,
            LedgerEntry {
                handle: grant.handle(),
                owner: grant.operation.owner().clone(),
                capability: grant.operation.capability(),
                kind: grant.kind,
            }
        );
    }
    durable
}

#[test]
fn attach_then_detach_leaves_no_residue_after_lifo_replay() {
    let manifest = ManifestBuilder::new("github-pr", "Read and comment on pull requests.")
        .tool("pr.read", "Read a pull request.")
        .tool("pr.comment", "Post a comment on a pull request.")
        .host("api.github.com")
        .drain_ms(750)
        .build();
    let plugin = PluginId::from(manifest.id.as_str());
    let cell_id = CellId::from("cell-1");
    let owner = CellOwner {
        cell: cell_id.clone(),
        plugin: plugin.clone(),
    };

    let registry = ToolRegistry::new();
    registry
        .register(&owner, &manifest.provides_tools)
        .expect("no other plasmid holds these names in cell-1");
    assert_eq!(registry.list(&cell_id), vec!["pr.comment", "pr.read"]);
    assert_eq!(registry.lookup(&cell_id, "pr.read").unwrap().owner, owner);

    let desired = DesiredStateBuilder::at_generation(1)
        .cell("cell-1", "researcher")
        .plasmid_in("cell-1", &manifest.id, MockMode::Simulate)
        .build();
    let cell = desired
        .cells
        .get(&cell_id)
        .expect("the builder declared cell-1");
    assert_eq!(cell.plasmids[0].plasmid, manifest.id);

    let mut backend = FakeBackend::new();
    let before = backend.snapshot_os_state().unwrap();
    assert!(before.is_empty());

    let host = &manifest.network.as_ref().expect("a host").hosts[0];
    let prepared = github_pr_closure(&cell_id, &plugin, host).prepare();
    let attached: Vec<String> = prepared
        .iter()
        .map(|grant| grant.backend_effect().description)
        .collect();
    let directory = tempfile::tempdir().unwrap();
    let ledger = prepare_then_grant(
        &mut backend,
        &directory.path().join("github-pr.ledger"),
        &plugin,
        prepared,
    );
    assert_eq!(ledger.len(), 3);
    let granted = backend.snapshot_os_state().unwrap();
    assert_eq!(
        (granted.state().len(), granted.grants().len()),
        (3, 3),
        "every prepared grant stands and keeps its issued record"
    );

    let Closure::ExternalFree(mut sealed) = ledger.close() else {
        panic!("a ledger of exact inverses closes without an operator assertion");
    };
    let drain = DrainSpec::graceful(Duration::from_millis(
        manifest.drain_ms.expect("the builder set a drain deadline"),
    ));
    let report = sealed
        .detach(&mut backend, &cell_id, drain)
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
        registry.withdraw(&owner),
        vec!["pr.comment".to_string(), "pr.read".to_string()]
    );
    assert!(registry.is_empty());

    let after = backend.snapshot_os_state().unwrap();
    let residue =
        ResidueReport::from_diff(Diff::between(before.state(), after.state()), Vec::new());
    assert_eq!(residue, ResidueReport::Empty, "{residue}");
    assert!(after.is_empty());
}

#[test]
fn an_identity_source_that_fails_during_allocation_leaves_no_log_and_no_holding() {
    let cell = CellId::from("cell-1");
    let plugin = PluginId::from("github-pr");
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("github-pr.ledger");
    let mut backend = FakeBackend::new();
    let mut drawn = 0;
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let prepared = github_pr_closure(&cell, &plugin, "api.github.com").prepare_with(|| {
            drawn += 1;
            if drawn == 2 {
                panic!("the system random source failed");
            }
            GrantId::new()
        });
        prepare_then_grant(&mut backend, &log, &plugin, prepared)
    }));
    let payload = outcome.expect_err("the identity source panics on its second draw");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("the system random source failed")
    );
    assert_eq!(drawn, 2);
    assert!(!log.exists(), "a failed allocation must write no log");
    assert!(backend.snapshot_os_state().unwrap().is_empty());
}

#[test]
fn declared_descriptions_reach_registry_lookup_until_withdrawal() {
    let source = r#"
id = "github-pr"
description = "Read and comment on pull requests."
impl.wasm = "github-pr.wasm"

[provides."github:tools".tools]
"pr.read" = " Read the title and review state. "
"pr.comment" = "Post a comment on a pull request."
"#;
    let manifest = PlasmidManifest::parse(source).unwrap();
    let registry = ToolRegistry::new();
    let cell = CellId::from("cell-1");
    let owner = CellOwner {
        cell: cell.clone(),
        plugin: PluginId::from(manifest.id.as_str()),
    };
    registry
        .register(&owner, &manifest.provides_tools)
        .expect("no other plasmid holds these names in cell-1");
    assert_eq!(
        registry.lookup(&cell, "pr.read").unwrap().description,
        " Read the title and review state. "
    );
    assert_eq!(
        registry.lookup(&cell, "pr.comment").unwrap().description,
        "Post a comment on a pull request."
    );
    registry.withdraw(&owner);
    assert_eq!(
        registry.lookup(&cell, "pr.read"),
        Err(plasmosome_core::LookupError::UnknownTool {
            cell,
            tool: "pr.read".into()
        })
    );
}

#[test]
fn a_detach_stopped_by_a_failed_withdrawal_leaves_no_standing_residue_but_an_unfinished_account() {
    let cell = CellId::from("cell-1");
    let plugin = PluginId::from("github-pr");
    let directory = tempfile::tempdir().unwrap();
    let mut backend = FakeBackend::new();
    let before = backend.snapshot_os_state().unwrap();
    let prepared = github_pr_closure(&cell, &plugin, "api.github.com").prepare();
    let first = prepared[0].handle();
    let ledger = prepare_then_grant(
        &mut backend,
        &directory.path().join("github-pr.ledger"),
        &plugin,
        prepared,
    );
    backend.fail_next_withdrawal(first, "unmount busy");
    let Closure::ExternalFree(mut sealed) = ledger.close() else {
        panic!("a ledger of exact inverses closes without an operator assertion");
    };
    let drain = DrainSpec::graceful(Duration::from_millis(50));
    let stopped = sealed
        .detach(&mut backend, &cell, drain)
        .expect_err("the withdrawal replayed last fails after its release");
    assert!(
        matches!(
            &stopped,
            DetachError::Backend(BackendError::IncompleteEffect { class, id, .. })
                if *class == first.class.as_str() && *id == first.id
        ),
        "{stopped}"
    );
    let after = backend.snapshot_os_state().unwrap();
    let residue =
        ResidueReport::from_diff(Diff::between(before.state(), after.state()), Vec::new());
    assert_eq!(residue, ResidueReport::Empty, "{residue}");
    assert!(
        !after.is_empty(),
        "the unfinished holding and its issued record must remain in the account"
    );
    let resumed = sealed
        .detach(&mut backend, &cell, drain)
        .expect("a retried detach finishes the exact cleanup");
    assert_eq!(resumed.replayed.len(), 1);
    assert!(backend.snapshot_os_state().unwrap().is_empty());
}
