use std::time::Duration;

use plasmosome_backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, Grant, Handle, LedgerEntry, OsObject,
    OsState, PluginId, UniverseClass, UniverseOp, UniverseRemoval,
};

use crate::builders::GrantSequence;

const CONFORMANCE_PLUGIN: &str = "conformance";
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

/// After a graceful revoke drains, the object the grant materialized is gone
/// from the snapshot. A backend that forgets the handle but leaves the object is
/// exactly the leak this kernel exists to prevent.
pub fn drained_revoke_removes_object<B: EnforcementBackend>(make: impl Fn() -> B) {
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
        backend
            .revoke(entry.handle, DrainSpec::graceful(DRAIN))
            .unwrap_or_else(|error| {
                panic!("a graceful revoke of {} failed: {error}", entry.handle)
            });
        assert!(
            !backend
                .snapshot_os_state()
                .contains(object.class, &object.key),
            "a drained revoke left {} behind",
            object.describe()
        );
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
/// every one of them still revokes. A backend that reissues a live handle
/// breaks a ledger replay at the second revoke: the handle is already spent,
/// `UnknownHandle` aborts the detach, and every effect below it stays granted.
pub fn live_grants_hold_distinct_handles<B: EnforcementBackend>(make: impl Fn() -> B) {
    let mut backend = make();
    let mut live: Vec<LedgerEntry> = Vec::new();
    for grant in sample_grants() {
        let entry = backend.grant(grant);
        if let Some(held) = live.iter().find(|held| held.handle == entry.handle) {
            panic!(
                "the grant of {} was issued {}, the handle the live grant of {} is already holding",
                entry.capability.class_str(),
                entry.handle,
                held.capability.class_str()
            );
        }
        live.push(entry);
    }
    for entry in live {
        backend
            .revoke(entry.handle, DrainSpec::graceful(DRAIN))
            .unwrap_or_else(|error| {
                panic!(
                    "the live grant of {} did not revoke through {}: {error}",
                    entry.capability.class_str(),
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

/// `apply` puts an object in the universe and `apply_removal` takes that same
/// object away again, for every class the universe models. A ledger reaches
/// `apply_removal` for `InverseVia::Universe` and for every compensating
/// effect, so a backend that refuses either fails every detach that reaches one.
pub fn apply_and_removal_reach_the_universe<B: EnforcementBackend>(make: impl Fn() -> B) {
    let mut backend = make();
    for (op, removal) in universe_pairs() {
        let object = op.object();
        backend
            .apply(op)
            .unwrap_or_else(|error| panic!("applying {} failed: {error}", object.describe()));
        assert!(
            backend
                .snapshot_os_state()
                .contains(object.class, &object.key),
            "an applied op must materialize {}",
            object.describe()
        );
        backend.apply_removal(removal).unwrap_or_else(|error| {
            panic!("removing the applied {} failed: {error}", object.describe())
        });
        assert!(
            !backend
                .snapshot_os_state()
                .contains(object.class, &object.key),
            "an applied removal left {} behind",
            object.describe()
        );
    }
    let remaining = backend.snapshot_os_state();
    assert!(
        remaining.is_empty(),
        "every applied op was removed again, so the universe must be empty, found {remaining:?}"
    );
}

/// Revoking a handle that was granted and then revoked is `UnknownHandle`
/// naming the handle the caller passed in. A partially replayed ledger resumes
/// over handles an earlier pass already withdrew, so it must be able to tell a
/// spent handle from a live one, and the error must name the handle it holds.
pub fn revoke_of_a_revoked_handle_is_error<B: EnforcementBackend>(make: impl Fn() -> B) {
    let mut backend = make();
    let grant = one_grant();
    let object = materialized(&grant);
    let entry = backend.grant(grant);
    backend
        .revoke(entry.handle, DrainSpec::graceful(DRAIN))
        .unwrap_or_else(|error| panic!("a graceful revoke of {} failed: {error}", entry.handle));
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
    assert!(
        !backend
            .snapshot_os_state()
            .contains(object.class, &object.key),
        "the second revoke of {} put {} back in the universe",
        entry.handle,
        object.describe()
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

fn one_grant() -> Grant {
    sample_grants().remove(0)
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
