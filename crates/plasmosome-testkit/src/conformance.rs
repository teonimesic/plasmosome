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
/// that entry's handle revokes back to the same entry under both drain policies.
/// A backend that can create something it cannot hand back fails here, and so
/// does one that hands back a stranger only when the drain is forced.
pub fn grant_is_replayable<B: EnforcementBackend>(make: impl Fn() -> B) {
    for drain in [DrainSpec::graceful(DRAIN), DrainSpec::forcing()] {
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
            match backend.revoke(entry.handle, drain) {
                Ok(replayed) => assert_eq!(
                    replayed,
                    entry,
                    "revoking a handle must return the entry the grant issued, and the {} revoke of {} did not",
                    policy_of(drain),
                    entry.handle
                ),
                Err(error) => panic!(
                    "the handle {} a grant just issued did not survive a {} revoke: {error}",
                    entry.handle,
                    policy_of(drain)
                ),
            }
        }
    }
}

/// Revoking a handle no grant ever issued is `UnknownHandle` under both drain
/// policies, never a success and never some other error. A backend that reports
/// success here lets a caller believe a capability was withdrawn when nothing
/// was, and a forced revoke is the drain a caller reaches for when it has
/// already stopped believing the graceful one.
pub fn revoke_unknown_handle_is_error<B: EnforcementBackend>(make: impl Fn() -> B) {
    for drain in [DrainSpec::graceful(DRAIN), DrainSpec::forcing()] {
        let mut backend = make();
        let grant = one_grant();
        let object = materialized(&grant);
        let live = backend.grant(grant);
        let unknown = Handle(live.handle.raw().wrapping_add(1_000_000));
        match backend.revoke(unknown, drain) {
            Err(BackendError::UnknownHandle { handle }) => assert_eq!(
                handle,
                unknown,
                "the error from a {} revoke must name the handle that was asked for",
                policy_of(drain)
            ),
            Err(other) => panic!(
                "revoking the never-granted handle {unknown} through a {} revoke must be UnknownHandle, got {other}",
                policy_of(drain)
            ),
            Ok(entry) => panic!(
                "revoking the never-granted handle {unknown} through a {} revoke reported success: {entry:?}",
                policy_of(drain)
            ),
        }
        assert!(
            backend
                .snapshot_os_state()
                .contains(object.class, &object.key),
            "a failed {} revoke must leave the live grant's {} alone",
            policy_of(drain),
            object.describe()
        );
    }
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

/// An object nobody granted survives every revoke, under both drain policies.
/// Residue detection is only worth anything if a revoke cannot tidy away what it
/// never created, and a forced revoke that clears the universe instead of its own
/// object destroys another plugin's capabilities while reporting success.
pub fn planted_residue_survives_unrelated_revoke<B: EnforcementBackend>(make: impl Fn() -> B) {
    for drain in [DrainSpec::graceful(DRAIN), DrainSpec::forcing()] {
        let mut backend = make();
        let residue = residue_object();
        backend.plant(residue.clone());
        for grant in sample_grants() {
            let entry = backend.grant(grant);
            backend.revoke(entry.handle, drain).unwrap_or_else(|error| {
                panic!(
                    "a {} revoke of {} failed: {error}",
                    policy_of(drain),
                    entry.handle
                )
            });
            assert!(
                backend
                    .snapshot_os_state()
                    .contains(residue.class, &residue.key),
                "a {} revoke of {} removed the unrelated {}",
                policy_of(drain),
                entry.handle,
                residue.describe()
            );
        }
        let state = backend.snapshot_os_state();
        assert_eq!(
            state.len(),
            1,
            "only the planted residue may remain after every {} revoke, found {state:?}",
            policy_of(drain)
        );
    }
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
/// class, which two plugins each holding a session file produce every day. The
/// live set is revoked twice: once in the reverse push order a ledger replay
/// walks on detach, and once in grant order. A backend that accepts a revoke
/// only for the oldest live handle satisfies the grant-order pass and then
/// strands every effect below the first one a detach reaches for. A backend
/// that reissues a live handle breaks a ledger replay at the second revoke: the
/// handle is already spent, `UnknownHandle` aborts the detach, and every effect
/// below it stays granted. A backend that keys its ledger by class instead of by
/// handle strands one of the two session files the same way, and one that
/// withdraws whichever object of the class it finds first takes the wrong
/// plugin's session file while still leaving the universe empty at the end.
pub fn live_grants_hold_distinct_handles<B: EnforcementBackend>(make: impl Fn() -> B) {
    for order in [RevokeOrder::ReversePush, RevokeOrder::GrantOrder] {
        let mut backend = make();
        let mut live: Vec<(LedgerEntry, OsObject)> = Vec::new();
        for grant in grants_with_two_of_one_class() {
            let object = materialized(&grant);
            let entry = backend.grant(grant);
            if let Some((held, _)) = live.iter().find(|(held, _)| held.handle == entry.handle) {
                panic!(
                    "the grant of {} for `{}` was issued {}, the handle the live grant of {} for `{}` is already holding",
                    entry.capability.class_str(),
                    entry.plugin,
                    entry.handle,
                    held.capability.class_str(),
                    held.plugin
                );
            }
            live.push((entry, object));
        }
        for (entry, object) in order.arrange(live) {
            backend
                .revoke(entry.handle, DrainSpec::graceful(DRAIN))
                .unwrap_or_else(|error| {
                    panic!(
                        "the live grant of {} for `{}` did not revoke through {} on the {} pass: {error}",
                        entry.capability.class_str(),
                        entry.plugin,
                        entry.handle,
                        order.name()
                    )
                });
            let after = backend.snapshot_os_state();
            assert!(
                !after.contains(object.class, &object.key),
                "revoking {} on the {} pass left {} standing; a revoke must withdraw the object its own grant materialized",
                entry.handle,
                order.name(),
                object.describe()
            );
        }
        let remaining = backend.snapshot_os_state();
        assert!(
            remaining.is_empty(),
            "revoking every live grant in {} must empty the universe, found {remaining:?}",
            order.name()
        );
    }
}

/// `apply` puts an object in the universe under the owner the op names, and
/// `apply_removal` takes that same object away again and nothing else, for
/// every class the universe models, with one planted residue standing in every
/// one of those classes. A ledger reaches `apply_removal` for
/// `InverseVia::Universe` and for every compensating effect, so a backend that
/// refuses either fails every detach that reaches one; one that applies under
/// an owner of its own choosing makes every residue report attribute a leak to
/// the wrong plugin; and one that takes the whole class instead of the object
/// tidies away residue it never created, whichever class it spares.
pub fn apply_and_removal_reach_the_universe<B: EnforcementBackend>(make: impl Fn() -> B) {
    let mut backend = make();
    let residues = residue_objects();
    for residue in &residues {
        backend.plant(residue.clone());
    }
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
        backend
            .apply_removal(removal, &object.owner)
            .unwrap_or_else(|error| {
                panic!("removing the applied {} failed: {error}", object.describe())
            });
        let removed = backend.snapshot_os_state();
        assert!(
            !removed.contains(object.class, &object.key),
            "an applied removal left {} behind",
            object.describe()
        );
        for residue in &residues {
            assert!(
                removed.contains(residue.class, &residue.key),
                "removing the applied {} also took the unrelated {}",
                object.describe(),
                residue.describe()
            );
        }
    }
    let remaining = backend.snapshot_os_state();
    assert_eq!(
        remaining.len(),
        residues.len(),
        "every applied op was removed again, so only the {} planted residues may remain, found {remaining:?}",
        residues.len()
    );
}

/// Revoking a handle that was granted and then revoked is `UnknownHandle`
/// naming the handle the caller passed in, under both drain policies, and after
/// two handles have been freed rather than one. A partially replayed ledger
/// resumes over handles an earlier pass already withdrew while the cell keeps
/// granting, so a backend that hands a freed handle number out again turns that
/// resumed replay into a revoke of whichever live grant now holds it. Two freed
/// handles are what an ordinary free list needs before it starts reusing, and a
/// forced revoke that withdraws the object without retiring the handle leaves
/// the same number pointing at nothing. The two are probed most recently
/// revoked first, the order a replay resuming over a detach walks them in, so a
/// free list that reused the older number is caught with the newer one still
/// answering as it should.
pub fn revoke_of_a_revoked_handle_is_error<B: EnforcementBackend>(make: impl Fn() -> B) {
    for drain in [DrainSpec::graceful(DRAIN), DrainSpec::forcing()] {
        let mut backend = make();
        let mut grants = sample_grants();
        let later = grants.remove(2);
        let later_object = materialized(&later);
        let mut spent: Vec<(LedgerEntry, OsObject)> = Vec::new();
        for grant in grants.into_iter().take(2) {
            let object = materialized(&grant);
            let entry = backend.grant(grant);
            backend.revoke(entry.handle, drain).unwrap_or_else(|error| {
                panic!(
                    "a {} revoke of {} failed: {error}",
                    policy_of(drain),
                    entry.handle
                )
            });
            spent.push((entry, object));
        }
        let later_entry = backend.grant(later);
        for (entry, object) in spent.into_iter().rev() {
            match backend.revoke(entry.handle, drain) {
                Err(BackendError::UnknownHandle { handle }) => assert_eq!(
                    handle,
                    entry.handle,
                    "a {} revoke of a spent handle must name the handle the caller asked for, not {handle}",
                    policy_of(drain)
                ),
                Err(other) => panic!(
                    "revoking the already-revoked handle {} through a {} revoke must be UnknownHandle, got {other}",
                    entry.handle,
                    policy_of(drain)
                ),
                Ok(replayed) => panic!(
                    "revoking the already-revoked handle {} through a {} revoke reported success: {replayed:?}",
                    entry.handle,
                    policy_of(drain)
                ),
            }
            let after = backend.snapshot_os_state();
            assert!(
                !after.contains(object.class, &object.key),
                "the refused {} revoke of the already-revoked {} must leave {} withdrawn",
                policy_of(drain),
                entry.handle,
                object.describe()
            );
            assert!(
                after.contains(later_object.class, &later_object.key),
                "the refused {} revoke of the already-revoked {} took {} from the live grant holding {}",
                policy_of(drain),
                entry.handle,
                later_object.describe(),
                later_entry.handle
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RevokeOrder {
    ReversePush,
    GrantOrder,
}

impl RevokeOrder {
    fn name(self) -> &'static str {
        match self {
            RevokeOrder::ReversePush => "reverse-push-order",
            RevokeOrder::GrantOrder => "grant-order",
        }
    }

    fn arrange(self, mut live: Vec<(LedgerEntry, OsObject)>) -> Vec<(LedgerEntry, OsObject)> {
        if self == RevokeOrder::ReversePush {
            live.reverse();
        }
        live
    }
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

fn residue_objects() -> Vec<OsObject> {
    let abandoned = PluginId::from("abandoned");
    vec![
        residue_object(),
        OsObject {
            class: UniverseClass::UdsPath,
            key: "/run/abandoned/orphan.uds".to_string(),
            owner: abandoned.clone(),
        },
        OsObject {
            class: UniverseClass::ProxyMap,
            key: "abandoned.plasmosome.test".to_string(),
            owner: abandoned.clone(),
        },
        OsObject {
            class: UniverseClass::BrokerPid,
            key: "broker/31337".to_string(),
            owner: abandoned.clone(),
        },
        OsObject {
            class: UniverseClass::Mount,
            key: "/abandoned".to_string(),
            owner: abandoned,
        },
    ]
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
