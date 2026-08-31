use std::time::Duration;

use plasmosome_backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, Grant, Handle, OsObject, OsState,
    PluginId, UniverseClass, UniverseOp,
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
