use std::time::Duration;

use plasmosome_backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, Grant, Handle, LedgerEntry, OsObject,
    OsState, PluginId, RevokePolicy, UniverseClass, UniverseOp, UniverseRemoval,
};

use crate::builders::GrantSequence;

const CONFORMANCE_PLUGIN: &str = "conformance";
const SECOND_PLUGIN: &str = "conformance-second";
const DRAIN: Duration = Duration::from_millis(50);

/// Every grant returns a ledger entry that describes the grant it came from, and
/// that entry's handle revokes back to the same entry. A backend that can create
/// something it cannot hand back fails here.
pub fn grant_is_replayable<B: EnforcementBackend>(make: impl Fn() -> B) {
    let mut backend = make();
    for grant in sample_grants() {
        let entry = backend.grant(grant.clone());
        assert_eq!(
            entry.plugin, grant.plugin,
            "a ledger entry must name the plugin that asked for the grant"
        );
        assert_eq!(
            entry.capability, grant.capability,
            "a ledger entry must carry the capability it granted"
        );
        assert_eq!(
            entry.kind, grant.kind,
            "a ledger entry must keep the grant's hot vs generation-bound kind"
        );
        match backend.revoke(entry.handle, DrainSpec::graceful(DRAIN)) {
            Ok(replayed) => assert_eq!(
                replayed, entry,
                "revoking a handle must return the entry the grant issued"
            ),
            Err(error) => panic!(
                "the handle {} a grant just issued did not revoke: {error}",
                entry.handle
            ),
        }
    }
}

/// Revoking a handle no grant ever issued is `UnknownHandle`, never a success
/// and never some other error. A backend that reports success here lets a caller
/// believe a capability was withdrawn when nothing was.
pub fn revoke_unknown_handle_is_error<B: EnforcementBackend>(make: impl Fn() -> B) {
    let mut backend = make();
    let grant = one_grant();
    let object = materialized(&grant);
    let live = backend.grant(grant);
    let unknown = Handle(live.handle.raw().wrapping_add(1_000_000));
    match backend.revoke(unknown, DrainSpec::graceful(DRAIN)) {
        Err(BackendError::UnknownHandle { handle }) => assert_eq!(
            handle, unknown,
            "the error must name the handle that was asked for"
        ),
        Err(other) => {
            panic!("revoking the never-granted handle {unknown} must be UnknownHandle, got {other}")
        }
        Ok(entry) => {
            panic!("revoking the never-granted handle {unknown} reported success: {entry:?}")
        }
    }
    assert!(
        backend
            .snapshot_os_state()
            .contains(object.class, &object.key),
        "a failed revoke must leave the live grant's {} alone",
        object.describe()
    );
}

/// After a revoke drains, the object the grant materialized is gone from the
/// snapshot, under both drain policies. A backend that forgets the handle but
/// leaves the object is exactly the leak this kernel exists to prevent, and a
/// forced revoke that reports success while withdrawing nothing is that same
/// leak wearing a success report.
pub fn drained_revoke_removes_object<B: EnforcementBackend>(make: impl Fn() -> B) {
    for drain in [DrainSpec::graceful(DRAIN), DrainSpec::forcing()] {
        let mut backend = make();
        for grant in sample_grants() {
            let object = materialized(&grant);
            let entry = backend.grant(grant);
            assert!(
                backend
                    .snapshot_os_state()
                    .contains(object.class, &object.key),
                "a grant must materialize {}",
                object.describe()
            );
            backend.revoke(entry.handle, drain).unwrap_or_else(|error| {
                panic!(
                    "a {} revoke of {} failed: {error}",
                    policy_of(drain),
                    entry.handle
                )
            });
            assert!(
                !backend
                    .snapshot_os_state()
                    .contains(object.class, &object.key),
                "a {} revoke left {} behind",
                policy_of(drain),
                object.describe()
            );
        }
    }
}

/// An object nobody granted survives every revoke. Residue detection is only
/// worth anything if a revoke cannot tidy away what it never created.
pub fn planted_residue_survives_unrelated_revoke<B: EnforcementBackend>(make: impl Fn() -> B) {
    let mut backend = make();
    let residue = residue_object();
    backend.plant(residue.clone());
    for grant in sample_grants() {
        let entry = backend.grant(grant);
        backend
            .revoke(entry.handle, DrainSpec::graceful(DRAIN))
            .unwrap_or_else(|error| {
                panic!("a graceful revoke of {} failed: {error}", entry.handle)
            });
        assert!(
            backend
                .snapshot_os_state()
                .contains(residue.class, &residue.key),
            "revoking {} removed the unrelated {}",
            entry.handle,
            residue.describe()
        );
    }
    let state = backend.snapshot_os_state();
    assert_eq!(
        state.len(),
        1,
        "after every grant was revoked only the planted residue may remain, found {state:?}"
    );
}

/// A snapshot holds exactly what was granted or planted. A backend that reports
/// an object no caller asked for makes every residue report a false alarm.
pub fn snapshot_never_invents_objects<B: EnforcementBackend>(make: impl Fn() -> B) {
    assert!(
        make().snapshot_os_state().is_empty(),
        "a backend that granted nothing must report an empty universe"
    );
    let mut backend = make();
    let mut expected = OsState::new();
    for grant in sample_grants() {
        expected.insert(materialized(&grant));
        backend.grant(grant);
    }
    let residue = residue_object();
    expected.insert(residue.clone());
    backend.plant(residue);
    let snapshot = backend.snapshot_os_state();
    for object in snapshot.objects() {
        assert!(
            expected.objects().any(|known| known == object),
            "the snapshot holds {}, which was never granted or planted",
            object.describe()
        );
    }
    assert_eq!(
        snapshot.len(),
        expected.len(),
        "the snapshot lost an object that was granted or planted"
    );
}

/// No two grants that are live at the same moment hold the same handle, and
/// every one of them still revokes — including two grants of one capability
/// class, which two plugins each holding a session file produce every day. A
/// backend that reissues a live handle breaks a ledger replay at the second
/// revoke: the handle is already spent, `UnknownHandle` aborts the detach, and
/// every effect below it stays granted. A backend that keys its ledger by class
/// instead of by handle strands the first of the two the same way.
pub fn live_grants_hold_distinct_handles<B: EnforcementBackend>(make: impl Fn() -> B) {
    let mut backend = make();
    let mut live: Vec<LedgerEntry> = Vec::new();
    for grant in grants_with_two_of_one_class() {
        let entry = backend.grant(grant);
        if let Some(held) = live.iter().find(|held| held.handle == entry.handle) {
            panic!(
                "the grant of {} for `{}` was issued {}, the handle the live grant of {} for `{}` is already holding",
                entry.capability.class_str(),
                entry.plugin,
                entry.handle,
                held.capability.class_str(),
                held.plugin
            );
        }
        live.push(entry);
    }
    for entry in live {
        backend
            .revoke(entry.handle, DrainSpec::graceful(DRAIN))
            .unwrap_or_else(|error| {
                panic!(
                    "the live grant of {} for `{}` did not revoke through {}: {error}",
                    entry.capability.class_str(),
                    entry.plugin,
                    entry.handle
                )
            });
    }
    let remaining = backend.snapshot_os_state();
    assert!(
        remaining.is_empty(),
        "revoking every live grant must empty the universe, found {remaining:?}"
    );
}

/// `apply` puts an object in the universe under the owner the op names, and
/// `apply_removal` takes that same object away again and nothing else, for
/// every class the universe models. A ledger reaches `apply_removal` for
/// `InverseVia::Universe` and for every compensating effect, so a backend that
/// refuses either fails every detach that reaches one; one that applies under
/// an owner of its own choosing makes every residue report attribute a leak to
/// the wrong plugin; and one that takes the whole class instead of the object
/// tidies away residue it never created.
pub fn apply_and_removal_reach_the_universe<B: EnforcementBackend>(make: impl Fn() -> B) {
    let mut backend = make();
    let residue = residue_object();
    backend.plant(residue.clone());
    for (op, removal) in universe_pairs() {
        let object = op.object();
        backend
            .apply(op)
            .unwrap_or_else(|error| panic!("applying {} failed: {error}", object.describe()));
        let applied = backend.snapshot_os_state();
        assert!(
            applied.objects().any(|held| *held == object),
            "an applied op must materialize {}; the snapshot holds that key under {:?}",
            object.describe(),
            applied.owner_of(object.class, &object.key)
        );
        backend.apply_removal(removal).unwrap_or_else(|error| {
            panic!("removing the applied {} failed: {error}", object.describe())
        });
        let removed = backend.snapshot_os_state();
        assert!(
            !removed.contains(object.class, &object.key),
            "an applied removal left {} behind",
            object.describe()
        );
        assert!(
            removed.contains(residue.class, &residue.key),
            "removing the applied {} also took the unrelated {}",
            object.describe(),
            residue.describe()
        );
    }
    let remaining = backend.snapshot_os_state();
    assert_eq!(
        remaining.len(),
        1,
        "every applied op was removed again, so only the planted {} may remain, found {remaining:?}",
        residue.describe()
    );
}

/// Revoking a handle that was granted and then revoked is `UnknownHandle`
/// naming the handle the caller passed in, even after a later grant has taken
/// a handle number of its own. A partially replayed ledger resumes over handles
/// an earlier pass already withdrew while the cell keeps granting, so a backend
/// that hands a freed handle number out again turns that resumed replay into a
/// revoke of whichever live grant now holds it.
pub fn revoke_of_a_revoked_handle_is_error<B: EnforcementBackend>(make: impl Fn() -> B) {
    let mut backend = make();
    let mut grants = sample_grants();
    let later = grants.remove(1);
    let grant = grants.remove(0);
    let object = materialized(&grant);
    let later_object = materialized(&later);
    let entry = backend.grant(grant);
    backend
        .revoke(entry.handle, DrainSpec::graceful(DRAIN))
        .unwrap_or_else(|error| panic!("a graceful revoke of {} failed: {error}", entry.handle));
    let later_entry = backend.grant(later);
    match backend.revoke(entry.handle, DrainSpec::graceful(DRAIN)) {
        Err(BackendError::UnknownHandle { handle }) => assert_eq!(
            handle, entry.handle,
            "the error must name the revoked handle the caller asked for, not {handle}"
        ),
        Err(other) => panic!(
            "revoking the already-revoked handle {} must be UnknownHandle, got {other}",
            entry.handle
        ),
        Ok(replayed) => panic!(
            "revoking the already-revoked handle {} reported success: {replayed:?}",
            entry.handle
        ),
    }
    let after = backend.snapshot_os_state();
    assert!(
        !after.contains(object.class, &object.key),
        "the second revoke of {} put {} back in the universe",
        entry.handle,
        object.describe()
    );
    assert!(
        after.contains(later_object.class, &later_object.key),
        "the second revoke of {} took {} from the live grant holding {}",
        entry.handle,
        later_object.describe(),
        later_entry.handle
    );
}

fn sample_grants() -> Vec<Grant> {
    GrantSequence::for_plugin(CONFORMANCE_PLUGIN)
        .hot(Capability::UdsSocket {
            path: "/run/conformance/egressd.uds".to_string(),
        })
        .hot(Capability::ProxyMap {
            host: "api.plasmosome.test".to_string(),
            route: "splice".to_string(),
        })
        .generation_bound(Capability::Broker {
            pid: 4242,
            name: "egressd".to_string(),
        })
        .hot(Capability::Mount {
            source: "/src/conformance".to_string(),
            target: "/workspace".to_string(),
        })
        .hot(Capability::SessionFile {
            path: "skills/pr.md".to_string(),
        })
        .into_grants()
}

fn universe_pairs() -> Vec<(UniverseOp, UniverseRemoval)> {
    let owner = PluginId::from(CONFORMANCE_PLUGIN);
    vec![
        (
            UniverseOp::WriteSessionFile {
                path: "notes/applied.md".to_string(),
                owner: owner.clone(),
            },
            UniverseRemoval::RemoveSessionFile {
                path: "notes/applied.md".to_string(),
            },
        ),
        (
            UniverseOp::BindUds {
                path: "/run/conformance/applied.uds".to_string(),
                owner: owner.clone(),
            },
            UniverseRemoval::UnbindUds {
                path: "/run/conformance/applied.uds".to_string(),
            },
        ),
        (
            UniverseOp::SetProxyMap {
                host: "applied.plasmosome.test".to_string(),
                route: "splice".to_string(),
                owner: owner.clone(),
            },
            UniverseRemoval::RemoveProxyMap {
                host: "applied.plasmosome.test".to_string(),
            },
        ),
        (
            UniverseOp::SpawnBroker {
                pid: 909,
                name: "applied".to_string(),
                owner: owner.clone(),
            },
            UniverseRemoval::KillBroker { pid: 909 },
        ),
        (
            UniverseOp::AddMount {
                source: "/src/applied".to_string(),
                target: "/applied".to_string(),
                owner,
            },
            UniverseRemoval::RemoveMount {
                target: "/applied".to_string(),
            },
        ),
    ]
}

fn grants_with_two_of_one_class() -> Vec<Grant> {
    let mut grants = sample_grants();
    grants.extend(
        GrantSequence::for_plugin(SECOND_PLUGIN)
            .hot(Capability::SessionFile {
                path: "skills/review.md".to_string(),
            })
            .into_grants(),
    );
    grants
}

fn one_grant() -> Grant {
    sample_grants().remove(0)
}

fn policy_of(drain: DrainSpec) -> &'static str {
    match drain.policy {
        RevokePolicy::Graceful => "drained",
        RevokePolicy::Force => "forced",
    }
}

fn residue_object() -> OsObject {
    OsObject {
        class: UniverseClass::SessionFile,
        key: "session/cache/abandoned-token".to_string(),
        owner: PluginId::from("abandoned"),
    }
}

fn materialized(grant: &Grant) -> OsObject {
    let owner = grant.plugin.clone();
    let op = match &grant.capability {
        Capability::SessionFile { path } => UniverseOp::WriteSessionFile {
            path: path.clone(),
            owner,
        },
        Capability::UdsSocket { path } => UniverseOp::BindUds {
            path: path.clone(),
            owner,
        },
        Capability::ProxyMap { host, route } => UniverseOp::SetProxyMap {
            host: host.clone(),
            route: route.clone(),
            owner,
        },
        Capability::Broker { pid, name } => UniverseOp::SpawnBroker {
            pid: *pid,
            name: name.clone(),
            owner,
        },
        Capability::Mount { source, target } => UniverseOp::AddMount {
            source: source.clone(),
            target: target.clone(),
            owner,
        },
    };
    op.object()
}
