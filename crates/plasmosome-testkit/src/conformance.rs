#[cfg(test)]
use std::cell::Cell;
use std::time::Duration;

use plasmosome_backend::{
    BackendError, Capability, CellId, CellOwner, DrainSpec, EnforcementBackend, FileAccess,
    GrantId, GrantKind, Handle, LedgerEntry, OsObject, OsState, PluginId, RevokePolicy,
    UniverseClass, UniverseOp, UniverseRemoval,
};

use crate::builders::{
    GrantClosure, PreparedGrant, broker, mount, proxy_map, session_file, uds_socket,
};

const DRAIN: Duration = Duration::from_millis(50);

#[cfg(test)]
thread_local! {
    static CONTRACT_FAILURE_RECORDED: Cell<bool> = const { Cell::new(false) };
}

macro_rules! record_contract_failure {
    () => {{
        #[cfg(test)]
        CONTRACT_FAILURE_RECORDED.with(|recorded| recorded.set(true));
    }};
}

macro_rules! contract_assert {
    ($condition:expr $(,)?) => {{
        let condition = $condition;
        if !condition {
            record_contract_failure!();
            panic!("assertion failed: {}", stringify!($condition));
        }
    }};
    ($condition:expr, $($arg:tt)+) => {{
        let condition = $condition;
        if !condition {
            record_contract_failure!();
            panic!($($arg)+);
        }
    }};
}

macro_rules! contract_assert_eq {
    ($left:expr, $right:expr $(,)?) => {{
        let left = &$left;
        let right = &$right;
        if !(*left == *right) {
            record_contract_failure!();
            panic!(
                "assertion `left == right` failed\n  left: {left:?}\n right: {right:?}"
            );
        }
    }};
    ($left:expr, $right:expr, $($arg:tt)+) => {{
        let left = &$left;
        let right = &$right;
        if !(*left == *right) {
            record_contract_failure!();
            panic!(
                "assertion `left == right` failed: {}\n  left: {left:?}\n right: {right:?}",
                format_args!($($arg)+)
            );
        }
    }};
}

macro_rules! contract_assert_ne {
    ($left:expr, $right:expr $(,)?) => {{
        let left = &$left;
        let right = &$right;
        if *left == *right {
            record_contract_failure!();
            panic!("assertion `left != right` failed\n  left: {left:?}\n right: {right:?}");
        }
    }};
    ($left:expr, $right:expr, $($arg:tt)+) => {{
        let left = &$left;
        let right = &$right;
        if *left == *right {
            record_contract_failure!();
            panic!(
                "assertion `left != right` failed: {}\n  left: {left:?}\n right: {right:?}",
                format_args!($($arg)+)
            );
        }
    }};
}

macro_rules! contract_unwrap {
    ($result:expr) => {{
        match $result {
            Ok(value) => value,
            Err(error) => {
                record_contract_failure!();
                panic!("called `Result::unwrap()` on an `Err` value: {error:?}");
            }
        }
    }};
}

macro_rules! contract_unwrap_err {
    ($result:expr) => {{
        match $result {
            Err(error) => error,
            Ok(value) => {
                record_contract_failure!();
                panic!("called `Result::unwrap_err()` on an `Ok` value: {value:?}");
            }
        }
    }};
}

macro_rules! contract_expect {
    ($result:expr, $message:literal) => {{
        match $result {
            Ok(value) => value,
            Err(error) => {
                record_contract_failure!();
                panic!("{}: {error:?}", $message);
            }
        }
    }};
}

macro_rules! contract_panic {
    ($($arg:tt)*) => {{
        record_contract_failure!();
        panic!($($arg)*);
    }};
}

#[cfg(test)]
fn take_contract_failure() -> bool {
    CONTRACT_FAILURE_RECORDED.with(|recorded| recorded.replace(false))
}

/// Checks that a grant issues exactly the address, owner, capability and kind its caller
/// prepared, and that either revoke policy returns that entry.
pub fn grant_is_replayable<B: EnforcementBackend>(make: impl Fn() -> B) {
    for drain in drains() {
        let mut backend = make();
        for prepared in sample_grants() {
            let entry = grant_prepared(&mut backend, &prepared);
            contract_assert_eq!(
                entry.handle,
                prepared.handle(),
                "a grant must issue exactly the address its caller prepared"
            );
            contract_assert_eq!(&entry.owner, prepared.operation.owner());
            contract_assert_eq!(entry.capability, prepared.operation.capability());
            contract_assert_eq!(entry.kind, prepared.kind);
            contract_assert_eq!(
                backend.revoke(entry.handle, drain).unwrap_or_else(|error| {
                    contract_panic!(
                        "the handle {} a grant just issued did not survive a {} revoke: {error}",
                        entry.handle,
                        policy_of(drain)
                    )
                }),
                entry,
                "revoking a handle must return the entry the grant issued"
            );
        }
    }
}

/// Checks that a never-issued exact address is refused without changing live state.
pub fn revoke_unknown_handle_is_error<B: EnforcementBackend>(make: impl Fn() -> B) {
    for drain in drains() {
        let mut backend = make();
        let live = grant_prepared(&mut backend, &one_grant());
        let live_object = live.object();
        let unknown = Handle {
            class: live.handle.class,
            id: GrantId::new(),
        };
        contract_assert_eq!(
            contract_unwrap_err!(backend.revoke(unknown, drain)),
            BackendError::UnknownHandle { handle: unknown },
            "revoking the never-granted handle must return UnknownHandle"
        );
        assert_exact_state(
            &backend.snapshot_os_state(),
            &[live_object],
            "a failed revoke must preserve the live grant",
        );
    }
}

/// Checks that either successful revoke policy removes the exact granted object.
pub fn drained_revoke_removes_object<B: EnforcementBackend>(make: impl Fn() -> B) {
    for drain in drains() {
        let mut backend = make();
        for prepared in sample_grants() {
            let entry = grant_prepared(&mut backend, &prepared);
            let object = entry.object();
            backend.revoke(entry.handle, drain).unwrap_or_else(|error| {
                contract_panic!(
                    "a {} revoke of {} failed: {error}",
                    policy_of(drain),
                    entry.handle
                )
            });
            contract_assert!(
                !holds_exact(&backend.snapshot_os_state(), &object),
                "a {} revoke left {} standing",
                policy_of(drain),
                object.describe()
            );
        }
    }
}

/// Checks that revoking grants never removes an unrelated planted observation.
pub fn planted_residue_survives_unrelated_revoke<B: EnforcementBackend>(make: impl Fn() -> B) {
    for drain in drains() {
        let mut backend = make();
        let residue = residue_objects().remove(0);
        contract_expect!(
            backend.plant(residue.clone()),
            "the residue fixture must plant"
        );
        for prepared in sample_grants() {
            let entry = grant_prepared(&mut backend, &prepared);
            contract_unwrap!(backend.revoke(entry.handle, drain));
            contract_assert!(
                holds_exact(&backend.snapshot_os_state(), &residue),
                "a {} revoke of {} removed the unrelated {}",
                policy_of(drain),
                entry.handle,
                residue.describe()
            );
        }
        assert_exact_state(
            &backend.snapshot_os_state(),
            &[residue],
            "only planted residue may remain",
        );
    }
}

/// Checks that snapshots contain every and only requested grant or planted object; that a
/// grant, apply or plant of an invalid operation of any class, at a fresh address or at an issued
/// one, returns `InvalidOperation` and leaves the state as it was just before that call; that a
/// revoke of the handle of a grant refused at a fresh address returns `UnknownHandle` and changes
/// nothing; that the state does not change between two of those calls either, so an effect that
/// lands after its refusal returned is seen; and that a backend fresh from the factory, made
/// after another instance was used, observes nothing.
pub fn snapshot_never_invents_objects<B: EnforcementBackend>(make: impl Fn() -> B) {
    let mut backend = make();
    let mut expected = Vec::new();
    let mut issued = Vec::new();
    for prepared in sample_grants() {
        let entry = grant_prepared(&mut backend, &prepared);
        expected.push(requested_object(&entry, &prepared));
        issued.push(prepared.operation);
    }
    let residue = residue_objects().remove(0);
    contract_expect!(
        backend.plant(residue.clone()),
        "the residue fixture must plant"
    );
    expected.push(residue);
    let mut seen = backend.snapshot_os_state();
    assert_exact_state(&seen, &expected, "the snapshot lost or invented an object");
    let at_issued: Vec<UniverseOp> = issued
        .iter()
        .map(|standing| {
            op_for(
                standing.id(),
                standing.owner().clone(),
                invalid_capability(&standing.capability()),
            )
        })
        .collect();
    let fresh = invalid_operations();
    for invalid in fresh.iter().chain(&at_issued) {
        let before = unchanged_since(&backend, &seen);
        contract_assert_eq!(
            backend.grant(invalid.clone(), GrantKind::Hot),
            Err(invalid_refusal(invalid)),
            "a grant of an invalid {} operation must be refused before any change",
            invalid.class().as_str()
        );
        seen = backend.snapshot_os_state();
        contract_assert_eq!(
            seen,
            before,
            "a refused invalid {} grant changed the state",
            invalid.class().as_str()
        );
    }
    for invalid in &fresh {
        let before = unchanged_since(&backend, &seen);
        contract_assert_eq!(
            backend.revoke(invalid.handle(), DrainSpec::forcing()),
            Err(BackendError::UnknownHandle {
                handle: invalid.handle()
            }),
            "a refused invalid {} grant must issue no record",
            invalid.class().as_str()
        );
        seen = backend.snapshot_os_state();
        contract_assert_eq!(
            seen,
            before,
            "a revoke of a refused invalid {} grant's handle changed the state",
            invalid.class().as_str()
        );
    }
    for invalid in invalid_operations().iter().chain(&at_issued) {
        let before = unchanged_since(&backend, &seen);
        contract_assert_eq!(
            backend.apply(invalid.clone()),
            Err(invalid_refusal(invalid)),
            "an apply of an invalid {} operation must be refused before any change",
            invalid.class().as_str()
        );
        seen = backend.snapshot_os_state();
        contract_assert_eq!(
            seen,
            before,
            "a refused invalid {} apply changed the state",
            invalid.class().as_str()
        );
    }
    for invalid in invalid_operations().iter().chain(&at_issued) {
        let before = unchanged_since(&backend, &seen);
        contract_assert_eq!(
            backend.plant(invalid.object()),
            Err(invalid_refusal(invalid)),
            "a plant of an invalid {} object must be refused before any change",
            invalid.class().as_str()
        );
        seen = backend.snapshot_os_state();
        contract_assert_eq!(
            seen,
            before,
            "a refused invalid {} plant changed the state",
            invalid.class().as_str()
        );
    }
    contract_assert!(
        make().snapshot_os_state().is_empty(),
        "a backend fresh from the factory must observe nothing an earlier instance did"
    );
}

/// Checks distinct live handles, both revoke orders, and complete survivor snapshots.
pub fn live_grants_hold_distinct_handles<B: EnforcementBackend>(make: impl Fn() -> B) {
    for order in orders() {
        let mut backend = make();
        let mut live = Vec::new();
        for prepared in grants_with_two_of_one_class() {
            let entry = grant_prepared(&mut backend, &prepared);
            contract_assert!(
                live.iter()
                    .all(|(held, _): &(LedgerEntry, OsObject)| held.handle != entry.handle),
                "a live grant is already holding {}",
                entry.handle
            );
            live.push((entry.clone(), entry.object()));
        }
        let mut expected: Vec<OsObject> = live.iter().map(|(_, object)| object.clone()).collect();
        for (entry, object) in order.arrange(live) {
            backend
                .revoke(entry.handle, DrainSpec::graceful(DRAIN))
                .unwrap_or_else(|error| {
                    contract_panic!(
                        "the live grant did not revoke through {} on the {} pass: {error}",
                        entry.handle,
                        order.name()
                    )
                });
            remove_expected(&mut expected, &object);
            assert_exact_state(
                &backend.snapshot_os_state(),
                &expected,
                "a revoke must withdraw its own exact object and preserve all survivors",
            );
        }
    }
}

/// Checks exact apply/removal behavior with same-key neighbours in both orders.
pub fn apply_and_removal_reach_the_universe<B: EnforcementBackend>(make: impl Fn() -> B) {
    apply_and_removal_reach_the_universe_with(make, GrantId::new);
}

fn apply_and_removal_reach_the_universe_with<B, I>(make: impl Fn() -> B, mut new_id: I)
where
    B: EnforcementBackend,
    I: FnMut() -> GrantId,
{
    for capability in sample_capabilities() {
        for drain in drains() {
            for order in orders() {
                let mut backend = make();
                let mut expected: Vec<OsObject> = residue_objects_with(&mut new_id)
                    .into_iter()
                    .filter(|object| object.class() != capability.class())
                    .collect();
                let residues = [
                    OsObject {
                        id: new_id(),
                        owner: second_owner(),
                        capability: capability.clone(),
                    },
                    OsObject {
                        id: new_id(),
                        owner: other_cell(&conformance_owner()),
                        capability: capability.clone(),
                    },
                ];
                expected.extend(residues.iter().cloned());
                for object in &expected {
                    contract_expect!(
                        backend.plant(object.clone()),
                        "the residue fixture must plant"
                    );
                }
                let first = op_for(new_id(), conformance_owner(), capability.clone());
                let second = op_for(new_id(), conformance_owner(), capability.clone());
                contract_unwrap!(backend.apply(first.clone()));
                contract_unwrap!(backend.apply(second.clone()));
                expected.extend([first.object(), second.object()]);
                assert_exact_state(
                    &backend.snapshot_os_state(),
                    &expected,
                    "applied operations must coexist with all unrelated observations",
                );
                let arranged = order.arrange_ops(vec![first, second]);
                for (index, op) in arranged.into_iter().enumerate() {
                    let object = op.object();
                    for wrong_owner in [other_cell(&object.owner), second_owner()] {
                        contract_assert_eq!(
                            backend.apply_removal(op.removal(), &wrong_owner, drain),
                            Err(BackendError::UnknownObject {
                                class: object.class().as_str(),
                                key: object.key(),
                                owner: wrong_owner.clone(),
                                id: object.id,
                            }),
                            "a {} removal naming {wrong_owner} took the applied {} on the {} pass",
                            policy_of(drain),
                            object.describe(),
                            order.name()
                        );
                        assert_exact_state(
                            &backend.snapshot_os_state(),
                            &expected,
                            "a refused removal naming another owner changed the state",
                        );
                    }
                    backend
                        .apply_removal(op.removal(), &object.owner, drain)
                        .unwrap_or_else(|error| {
                            contract_panic!(
                                "the {} removal of the applied {} on the {} pass failed: {error}",
                                policy_of(drain),
                                object.describe(),
                                order.name()
                            )
                        });
                    remove_expected(&mut expected, &object);
                    let state = backend.snapshot_os_state();
                    assert_exact_state(
                        &state,
                        &expected,
                        "an applied removal selected the wrong instance or damaged unrelated state",
                    );
                    contract_assert!(
                        !holds_exact(&state, &object),
                        "an applied removal left its exact object standing"
                    );
                    let owner_still_holds_key =
                        owner_holds_key(&state, &object.owner, object.class(), &object.key());
                    contract_assert_eq!(
                        owner_still_holds_key,
                        index == 0,
                        "owner key membership must remain only while that owner has another holding"
                    );
                    for residue in &residues {
                        contract_assert!(holds_exact(&state, residue));
                    }
                }
            }
        }
    }
}

/// Checks that spent handles stay spent after later grants under either policy.
pub fn revoke_of_a_revoked_handle_is_error<B: EnforcementBackend>(make: impl Fn() -> B) {
    for drain in drains() {
        let mut backend = make();
        let mut spent = Vec::new();
        for prepared in sample_grants().into_iter().take(2) {
            let entry = grant_prepared(&mut backend, &prepared);
            let object = entry.object();
            contract_unwrap!(backend.revoke(entry.handle, drain));
            spent.push((entry, object));
        }
        let live = grant_prepared(&mut backend, &sample_grants().remove(2));
        for (entry, spent_object) in spent.into_iter().rev() {
            contract_assert_eq!(
                contract_unwrap_err!(backend.revoke(entry.handle, drain)),
                BackendError::UnknownHandle {
                    handle: entry.handle
                },
                "revoking the already-revoked handle must return UnknownHandle"
            );
            let state = backend.snapshot_os_state();
            contract_assert!(!holds_exact(&state, &spent_object));
            contract_assert!(
                holds_exact(&state, &live.object()),
                "a refused spent-handle revoke took the live neighbour"
            );
        }
    }
}

/// Checks that revoking one owner never takes an equal capability from another owner.
pub fn revoke_takes_its_owners_object<B: EnforcementBackend>(make: impl Fn() -> B) {
    let owner_pairs = [
        (owned_by("audit-owner"), owned_by("deploy-owner")),
        (conformance_owner(), other_cell(&conformance_owner())),
    ];
    for (first_owner, second_owner) in owner_pairs {
        for capability in sample_capabilities() {
            for drain in drains() {
                for order in orders() {
                    let mut backend = make();
                    let first_request = hot_grant(first_owner.clone(), &capability);
                    let first = grant_prepared(&mut backend, &first_request);
                    let first_object = requested_object(&first, &first_request);
                    let second_request = hot_grant(second_owner.clone(), &capability);
                    let second = grant_prepared(&mut backend, &second_request);
                    let second_object = requested_object(&second, &second_request);
                    let mut expected = vec![first_object.clone(), second_object.clone()];
                    for (entry, object) in
                        order.arrange(vec![(first, first_object), (second, second_object)])
                    {
                        backend.revoke(entry.handle, drain).unwrap_or_else(|error| {
                            contract_panic!(
                                "the {} revoke on the {} pass did not revoke its owner's object: {error}",
                                policy_of(drain),
                                order.name()
                            )
                        });
                        remove_expected(&mut expected, &object);
                        assert_exact_state(
                            &backend.snapshot_os_state(),
                            &expected,
                            "revoke took another owner's object",
                        );
                    }
                }
            }
        }
    }
}

/// Checks repeated equal grants, complete resource collisions, broker residue, and that a granted
/// holding removed through `apply_removal`, gracefully or by force, retires its handle.
pub fn repeated_grants_are_independently_removable<B: EnforcementBackend>(make: impl Fn() -> B) {
    let mut pairs: Vec<(Capability, Capability)> = sample_capabilities()
        .into_iter()
        .map(|capability| (capability.clone(), capability))
        .collect();
    pairs.push((
        mount("/secrets", "/workspace"),
        mount("/code", "/workspace"),
    ));
    pairs.push((
        proxy_map("routes.plasmosome.test", "audit"),
        proxy_map("routes.plasmosome.test", "deploy"),
    ));
    pairs.push((broker("egressd"), relaunched_broker()));

    for (first_capability, second_capability) in pairs {
        for drain in drains() {
            for order in orders() {
                let mut backend = make();
                let first_request = hot_grant(conformance_owner(), &first_capability);
                let first = grant_prepared(&mut backend, &first_request);
                let first_object = requested_object(&first, &first_request);
                let second_request = hot_grant(conformance_owner(), &second_capability);
                let second = grant_prepared(&mut backend, &second_request);
                let second_object = requested_object(&second, &second_request);
                contract_assert_ne!(first.handle, second.handle);
                let mut expected = vec![first_object.clone(), second_object.clone()];
                assert_exact_state(
                    &backend.snapshot_os_state(),
                    &expected,
                    "two requested holdings must coexist with their complete requested payloads",
                );
                for (entry, object) in
                    order.arrange(vec![(first, first_object), (second, second_object)])
                {
                    backend.revoke(entry.handle, drain).unwrap_or_else(|error| {
                        contract_panic!(
                            "repeated grant did not revoke on the {} {} pass: {error}",
                            policy_of(drain),
                            order.name()
                        )
                    });
                    remove_expected(&mut expected, &object);
                    assert_exact_state(
                        &backend.snapshot_os_state(),
                        &expected,
                        "repeated-grant revoke selected the wrong instance",
                    );
                }
            }
        }
    }

    let capability = broker("egressd");
    for drain in drains() {
        let mut backend = make();
        let residue = OsObject {
            id: GrantId::new(),
            owner: conformance_owner(),
            capability: capability.clone(),
        };
        contract_expect!(backend.plant(residue.clone()), "broker residue must plant");
        let live_request = hot_grant(residue.owner.clone(), &capability);
        let live = grant_prepared(&mut backend, &live_request);
        let live_object = requested_object(&live, &live_request);
        assert_exact_state(
            &backend.snapshot_os_state(),
            &[residue.clone(), live_object],
            "live broker and equal residue must coexist exactly",
        );
        contract_unwrap!(backend.revoke(live.handle, drain));
        assert_exact_state(
            &backend.snapshot_os_state(),
            std::slice::from_ref(&residue),
            "live broker revoke must preserve equal residue",
        );
        contract_unwrap!(backend.apply_removal(
            plasmosome_backend::UniverseRemoval {
                id: residue.id,
                capability: residue.capability.clone(),
            },
            &residue.owner,
            drain,
        ));
        contract_assert!(backend.snapshot_os_state().is_empty());
        let removed = grant_prepared(&mut backend, &hot_grant(residue.owner.clone(), &capability));
        contract_unwrap!(backend.apply_removal(removed.removal(), &removed.owner, drain));
        contract_assert!(backend.snapshot_os_state().is_empty());
        contract_assert_eq!(
            backend.revoke(removed.handle, DrainSpec::forcing()),
            Err(BackendError::UnknownHandle {
                handle: removed.handle
            }),
            "a successful {} apply_removal of a granted holding must retire its handle",
            policy_of(drain)
        );
    }
}

/// Checks, for every sample capability, both kinds and both revoke policies, that a retry of a
/// standing issued grant returns the original entry and changes nothing; that another kind,
/// owner or capability at an issued address returns `IdentityConflict` and changes nothing; that
/// a grant at the address of an applied or planted object, with the identical payload, another
/// owner or another capability, returns `IdentityConflict`, changes nothing and issues no record;
/// and that applying or planting the identical object of a live grant changes
/// nothing, so its revoke still returns the original entry. The clause withdraws one holding and
/// leaves the rest standing, so it has no withdrawal order to walk.
pub fn grant_retry_returns_the_original_entry<B: EnforcementBackend>(make: impl Fn() -> B) {
    for capability in sample_capabilities() {
        for kind in [GrantKind::Hot, GrantKind::GenerationBound] {
            for drain in drains() {
                let mut backend = make();
                let first = PreparedGrant {
                    operation: op_for(GrantId::new(), conformance_owner(), capability.clone()),
                    kind,
                };
                let entry = grant_prepared(&mut backend, &first);
                let issued_object = requested_object(&entry, &first);
                let mut expected = vec![issued_object.clone()];
                contract_assert_eq!(
                    backend.grant(first.operation.clone(), kind),
                    Ok(entry.clone()),
                    "a retry of the standing {} grant must return the original entry",
                    capability.class_str()
                );
                assert_exact_state(
                    &backend.snapshot_os_state(),
                    &expected,
                    "a retry changed the state",
                );
                let conflict = BackendError::IdentityConflict {
                    class: first.operation.class().as_str(),
                    id: first.operation.id(),
                };
                let mut changed = vec![(
                    first.operation.clone(),
                    other_kind(kind),
                    "another kind".to_string(),
                )];
                changed.push((
                    op_for(first.operation.id(), second_owner(), capability.clone()),
                    kind,
                    "another plugin".to_string(),
                ));
                changed.push((
                    op_for(
                        first.operation.id(),
                        other_cell(&conformance_owner()),
                        capability.clone(),
                    ),
                    kind,
                    "the same plugin in another cell".to_string(),
                ));
                for other in changed_capabilities(&capability) {
                    let label = format!("the capability {other:?}");
                    changed.push((
                        op_for(first.operation.id(), conformance_owner(), other),
                        kind,
                        label,
                    ));
                }
                for (operation, changed_kind, change) in changed {
                    contract_assert_eq!(
                        backend.grant(operation, changed_kind),
                        Err(conflict.clone()),
                        "a grant with {change} at an issued address must conflict"
                    );
                    assert_exact_state(
                        &backend.snapshot_os_state(),
                        &expected,
                        "a conflicting grant changed the state",
                    );
                }
                let applied = op_for(GrantId::new(), conformance_owner(), capability.clone());
                contract_unwrap!(backend.apply(applied.clone()));
                expected.push(applied.object());
                let residue = OsObject {
                    id: GrantId::new(),
                    owner: abandoned_owner(),
                    capability: capability.clone(),
                };
                contract_expect!(
                    backend.plant(residue.clone()),
                    "the residue fixture must plant"
                );
                expected.push(residue.clone());
                for (standing, origin) in [(applied, "applied"), (residue.operation(), "planted")] {
                    let mut requests = vec![
                        (standing.clone(), "the identical payload".to_string()),
                        (
                            op_for(standing.id(), second_owner(), capability.clone()),
                            "another owner".to_string(),
                        ),
                    ];
                    for other in changed_capabilities(&capability) {
                        let label = format!("the capability {other:?}");
                        requests.push((
                            op_for(standing.id(), standing.owner().clone(), other),
                            label,
                        ));
                    }
                    for (operation, payload) in requests {
                        contract_assert_eq!(
                            backend.grant(operation.clone(), kind),
                            Err(BackendError::IdentityConflict {
                                class: operation.class().as_str(),
                                id: operation.id(),
                            }),
                            "a grant of {payload} at an {origin} address must conflict"
                        );
                        assert_exact_state(
                            &backend.snapshot_os_state(),
                            &expected,
                            "a refused grant at a standing address changed the state",
                        );
                        contract_assert_eq!(
                            backend.revoke(operation.handle(), DrainSpec::forcing()),
                            Err(BackendError::UnknownHandle {
                                handle: operation.handle()
                            }),
                            "a grant of {payload} must not issue a record for an {origin} object"
                        );
                    }
                }
                contract_unwrap!(backend.apply(first.operation.clone()));
                contract_unwrap!(backend.plant(issued_object.clone()));
                assert_exact_state(
                    &backend.snapshot_os_state(),
                    &expected,
                    "applying or planting a live grant's own object must change nothing",
                );
                contract_assert_eq!(
                    backend.revoke(entry.handle, drain).unwrap_or_else(|error| {
                        contract_panic!(
                            "a {} revoke of the retried grant failed: {error}",
                            policy_of(drain)
                        )
                    }),
                    entry,
                    "revoking a retried grant must return the original entry and its kind"
                );
                remove_expected(&mut expected, &issued_object);
                assert_exact_state(
                    &backend.snapshot_os_state(),
                    &expected,
                    "revoking the retried grant must take only its own holding",
                );
            }
        }
    }
}

/// Checks that a backend fresh from the factory observes nothing an earlier instance did and
/// refuses that instance's handles with `UnknownHandle`, even while it holds an equal grant of
/// its own, and that the earlier instance keeps what it granted and can revoke it in both orders.
pub fn fresh_backends_share_no_authority<B: EnforcementBackend>(make: impl Fn() -> B) {
    for (order, drain) in orders()
        .into_iter()
        .flat_map(|order| drains().map(|drain| (order, drain)))
    {
        let mut first = make();
        let earlier: Vec<LedgerEntry> = sample_grants()
            .iter()
            .map(|prepared| grant_prepared(&mut first, prepared))
            .collect();
        let earlier_objects: Vec<OsObject> = earlier.iter().map(LedgerEntry::object).collect();
        let mut second = make();
        contract_assert!(
            second.snapshot_os_state().is_empty(),
            "a backend fresh from the factory must observe nothing an earlier instance did"
        );
        assert_unknown_everywhere(&mut second, &earlier);
        let mut own = Vec::new();
        for entry in &earlier {
            let equal = hot_grant(entry.owner.clone(), &entry.capability);
            own.push(requested_object(
                &grant_prepared(&mut second, &equal),
                &equal,
            ));
        }
        assert_unknown_everywhere(&mut second, &earlier);
        assert_exact_state(
            &second.snapshot_os_state(),
            &own,
            "a fresh backend must hold exactly its own grants",
        );
        assert_exact_state(
            &first.snapshot_os_state(),
            &earlier_objects,
            "a later instance changed what the earlier one holds",
        );
        for (entry, _) in order.arrange(earlier.into_iter().zip(earlier_objects).collect()) {
            contract_assert_eq!(
                first.revoke(entry.handle, drain).unwrap_or_else(|error| {
                    contract_panic!(
                        "the earlier instance could not {} revoke {} on the {} pass: {error}",
                        policy_of(drain),
                        entry.handle,
                        order.name()
                    )
                }),
                entry,
                "the earlier instance must still revoke its own grant on the {} pass",
                order.name()
            );
        }
        contract_assert!(
            first.snapshot_os_state().is_empty(),
            "the earlier instance kept a holding after the {} pass",
            order.name()
        );
    }
}

fn assert_unknown_everywhere<B: EnforcementBackend>(backend: &mut B, entries: &[LedgerEntry]) {
    for entry in entries {
        contract_assert_eq!(
            backend.revoke(entry.handle, DrainSpec::forcing()),
            Err(BackendError::UnknownHandle {
                handle: entry.handle
            }),
            "a backend must refuse a handle another instance issued"
        );
    }
}

/// The owner whose graceful withdrawals never finish. A factory passed to
/// `graceful_timeouts_preserve_the_selected_holding` must return a backend in which every
/// graceful withdrawal of a holding owned by exactly this owner, through `revoke` or
/// `apply_removal` and with any deadline including zero, times out before any release, while a
/// forced withdrawal succeeds. Every other owner must drain normally under any deadline, including
/// another plugin in this cell and this plugin in another cell. `FakeBackend` is armed with
/// `stall_graceful_drains_for_owner(stalled_owner())`.
pub fn stalled_owner() -> CellOwner {
    CellOwner {
        cell: CellId::from("conformance-stalled-cell"),
        plugin: PluginId::from("conformance-stalled"),
    }
}

/// Checks that a graceful timeout, with a 50 ms or a zero deadline, keeps the selected holding,
/// its issued record and every peer; that a removal naming the wrong owner is refused before any
/// drain, whichever side is stalled; that the peer, another plugin in the stalled cell and the
/// same plugin in another cell each drain normally under a 50 ms and under a zero deadline; that
/// `apply_removal` of a granted holding keeps its handle on a timeout and retires it on success;
/// and that Force withdraws only the stalled holding, both while every other holding stands and
/// after all of them are gone. The factory's backend must stall every graceful withdrawal owned
/// by `stalled_owner()`.
pub fn graceful_timeouts_preserve_the_selected_holding<B: EnforcementBackend>(
    make: impl Fn() -> B,
) {
    for capability in sample_capabilities() {
        for granted in [true, false] {
            for (order, normal) in withdrawal_passes() {
                let mut backend = make();
                let peer_request = hot_grant(conformance_owner(), &capability);
                let peer = grant_prepared(&mut backend, &peer_request);
                let peer_object = requested_object(&peer, &peer_request);
                let (stalled, stalled_entry) =
                    hold(&mut backend, stalled_owner(), &capability, granted);
                let (elsewhere, elsewhere_entry) = hold(
                    &mut backend,
                    other_cell(&stalled_owner()),
                    &capability,
                    granted,
                );
                let (neighbour, neighbour_entry) = hold(
                    &mut backend,
                    neighbour_of(&stalled_owner()),
                    &capability,
                    granted,
                );
                let pass = format!(
                    "{} {} pass draining the others within {normal:?}",
                    if granted { "revoke" } else { "apply_removal" },
                    order.name()
                );
                let all = [
                    peer_object.clone(),
                    stalled.clone(),
                    elsewhere.clone(),
                    neighbour.clone(),
                ];
                assert_exact_state(
                    &backend.snapshot_os_state(),
                    &all,
                    "a stalled holding must coexist with its equal peers",
                );
                let address = Handle {
                    class: stalled.class(),
                    id: stalled.id,
                };
                if granted {
                    let refused = backend.apply_removal(
                        removal_of(&stalled),
                        &stalled_owner(),
                        DrainSpec::graceful(DRAIN),
                    );
                    contract_assert!(
                        matches!(refused, Err(BackendError::DrainTimedOut { handle, .. }) if handle == address),
                        "a graceful apply_removal of the stalled grant must time out at its address on the {pass}, not {refused:?}"
                    );
                    assert_exact_state(
                        &backend.snapshot_os_state(),
                        &all,
                        "a timed-out apply_removal of a grant must keep the holding and every peer",
                    );
                }
                for deadline in [DRAIN, Duration::ZERO] {
                    let drain = DrainSpec::graceful(deadline);
                    contract_assert_eq!(
                        withdraw(
                            &mut backend,
                            &stalled,
                            stalled_entry.as_ref(),
                            granted,
                            drain
                        ),
                        Err(BackendError::DrainTimedOut {
                            handle: address,
                            deadline_ms: drain.deadline_ms(),
                        }),
                        "a graceful withdrawal of the stalled holding with a {deadline:?} deadline must time out, its handle still issued, on the {pass}"
                    );
                    assert_exact_state(
                        &backend.snapshot_os_state(),
                        &all,
                        "a graceful timeout must keep the selected holding and every peer",
                    );
                }
                for (holding, wrong_owner) in [
                    (&stalled, conformance_owner()),
                    (&peer_object, stalled_owner()),
                ] {
                    contract_assert_eq!(
                        backend.apply_removal(
                            removal_of(holding),
                            &wrong_owner,
                            DrainSpec::graceful(DRAIN)
                        ),
                        Err(BackendError::UnknownObject {
                            class: holding.class().as_str(),
                            key: holding.key(),
                            owner: wrong_owner.clone(),
                            id: holding.id,
                        }),
                        "a removal of {} naming {wrong_owner} must be refused before any drain on the {pass}",
                        holding.describe()
                    );
                    assert_exact_state(
                        &backend.snapshot_os_state(),
                        &all,
                        "a refused wrong-owner removal changed the state",
                    );
                }
                let forced_stalled = (&stalled, stalled_entry.as_ref(), DrainSpec::forcing());
                let graceful_peer = (&peer_object, Some(&peer), DrainSpec::graceful(normal));
                let graceful_elsewhere = (
                    &elsewhere,
                    elsewhere_entry.as_ref(),
                    DrainSpec::graceful(normal),
                );
                let graceful_neighbour = (
                    &neighbour,
                    neighbour_entry.as_ref(),
                    DrainSpec::graceful(normal),
                );
                let steps = match order {
                    WithdrawalOrder::StalledFirst => [
                        forced_stalled,
                        graceful_peer,
                        graceful_elsewhere,
                        graceful_neighbour,
                    ],
                    WithdrawalOrder::PeerFirst => [
                        graceful_peer,
                        graceful_elsewhere,
                        graceful_neighbour,
                        forced_stalled,
                    ],
                };
                let mut remaining = all.to_vec();
                for (object, entry, drain) in steps {
                    withdraw(&mut backend, object, entry, granted, drain).unwrap_or_else(|error| {
                        contract_panic!(
                            "withdrawing {} with {} failed on the {pass}: {error}",
                            object.describe(),
                            policy_of(drain)
                        )
                    });
                    remove_expected(&mut remaining, object);
                    assert_exact_state(
                        &backend.snapshot_os_state(),
                        &remaining,
                        "a withdrawal must take only its own holding",
                    );
                }
            }
        }
    }
}

fn hold<B: EnforcementBackend>(
    backend: &mut B,
    owner: CellOwner,
    capability: &Capability,
    granted: bool,
) -> (OsObject, Option<LedgerEntry>) {
    if granted {
        let request = hot_grant(owner, capability);
        let entry = grant_prepared(backend, &request);
        (requested_object(&entry, &request), Some(entry))
    } else {
        let op = op_for(GrantId::new(), owner, capability.clone());
        contract_unwrap!(backend.apply(op.clone()));
        (op.object(), None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WithdrawalOrder {
    StalledFirst,
    PeerFirst,
}

impl WithdrawalOrder {
    fn name(self) -> &'static str {
        match self {
            WithdrawalOrder::StalledFirst => "stalled-first",
            WithdrawalOrder::PeerFirst => "peer-first",
        }
    }
}

fn withdrawal_passes() -> impl Iterator<Item = (WithdrawalOrder, Duration)> {
    [WithdrawalOrder::StalledFirst, WithdrawalOrder::PeerFirst]
        .into_iter()
        .flat_map(|order| [DRAIN, Duration::ZERO].map(|normal| (order, normal)))
}

fn withdraw<B: EnforcementBackend>(
    backend: &mut B,
    object: &OsObject,
    entry: Option<&LedgerEntry>,
    through_revoke: bool,
    drain: DrainSpec,
) -> Result<(), BackendError> {
    match entry {
        Some(entry) if through_revoke => {
            let returned = backend.revoke(entry.handle, drain)?;
            contract_assert_eq!(
                returned,
                *entry,
                "revoking a handle must return the entry the grant issued"
            );
        }
        _ => {
            backend.apply_removal(removal_of(object), &object.owner, drain)?;
            if let Some(entry) = entry {
                contract_assert_eq!(
                    backend.revoke(entry.handle, DrainSpec::forcing()),
                    Err(BackendError::UnknownHandle {
                        handle: entry.handle
                    }),
                    "a successful apply_removal of a granted holding must retire its handle"
                );
            }
        }
    }
    Ok(())
}

fn hot_grant(owner: CellOwner, capability: &Capability) -> PreparedGrant {
    GrantClosure::for_owner(owner)
        .hot(capability.clone())
        .prepare()
        .remove(0)
}

fn grant_prepared<B: EnforcementBackend>(backend: &mut B, prepared: &PreparedGrant) -> LedgerEntry {
    contract_unwrap!(backend.grant(prepared.operation.clone(), prepared.kind))
}

fn relaunched_broker() -> Capability {
    let mut relaunched = broker("egressd");
    if let Capability::Broker { launch, .. } = &mut relaunched {
        launch.command.push("--audit".to_string());
        launch.control_socket = "/run/plasmosome/audit.control".to_string();
        launch.data_socket = "/run/plasmosome/audit.data".to_string();
    }
    relaunched
}

fn other_kind(kind: GrantKind) -> GrantKind {
    match kind {
        GrantKind::Hot => GrantKind::GenerationBound,
        GrantKind::GenerationBound => GrantKind::Hot,
    }
}

fn changed_capabilities(capability: &Capability) -> [Capability; 2] {
    let mut recipe_changed = capability.clone();
    let key_changed = match &mut recipe_changed {
        Capability::SessionFile { recipe, .. } => {
            recipe.guest_path = format!("{}.changed", recipe.guest_path);
            session_file("/var/lib/plasmosome/conformance/skills/changed.md")
        }
        Capability::UdsSocket { recipe, .. } => {
            recipe.guest_path = format!("{}.changed", recipe.guest_path);
            uds_socket("/run/conformance/changed.uds")
        }
        Capability::ProxyMap { recipe, .. } => {
            recipe.port = recipe.port % u16::MAX + 1;
            proxy_map("changed.plasmosome.test", "splice")
        }
        Capability::Broker { launch, .. } => {
            launch.data_socket = format!("{}.changed", launch.data_socket);
            broker("changed")
        }
        Capability::Mount { recipe, .. } => {
            recipe.access = match recipe.access {
                FileAccess::ReadOnly => FileAccess::ReadWrite,
                FileAccess::ReadWrite => FileAccess::ReadOnly,
            };
            mount("/src/changed", "/workspace")
        }
    };
    [key_changed, recipe_changed]
}

fn invalid_operations() -> Vec<UniverseOp> {
    sample_capabilities()
        .iter()
        .map(|capability| {
            op_for(
                GrantId::new(),
                conformance_owner(),
                invalid_capability(capability),
            )
        })
        .collect()
}

fn invalid_capability(capability: &Capability) -> Capability {
    let mut invalid = capability.clone();
    match &mut invalid {
        Capability::SessionFile { path, .. } | Capability::UdsSocket { path, .. } => {
            *path = path.trim_start_matches('/').to_string();
        }
        Capability::ProxyMap { recipe, .. } => recipe.port = 0,
        Capability::Broker { name, .. } => name.push('\0'),
        Capability::Mount { target, .. } => target.push('/'),
    }
    invalid
}

fn invalid_refusal(operation: &UniverseOp) -> BackendError {
    BackendError::InvalidOperation {
        class: operation.class().as_str(),
        id: operation.id(),
        error: operation
            .validate()
            .expect_err("the invalid operation fixture must fail validation"),
    }
}

fn removal_of(object: &OsObject) -> UniverseRemoval {
    UniverseRemoval {
        id: object.id,
        capability: object.capability.clone(),
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

    fn arrange_ops(self, mut ops: Vec<UniverseOp>) -> Vec<UniverseOp> {
        if self == RevokeOrder::ReversePush {
            ops.reverse();
        }
        ops
    }
}

fn drains() -> [DrainSpec; 2] {
    [DrainSpec::graceful(DRAIN), DrainSpec::forcing()]
}

fn orders() -> [RevokeOrder; 2] {
    [RevokeOrder::ReversePush, RevokeOrder::GrantOrder]
}

fn conformance_cell() -> CellId {
    CellId::from("conformance-cell")
}

fn owned_by(plugin: &str) -> CellOwner {
    CellOwner {
        cell: conformance_cell(),
        plugin: PluginId::from(plugin),
    }
}

fn conformance_owner() -> CellOwner {
    owned_by("conformance")
}

fn second_owner() -> CellOwner {
    owned_by("conformance-second")
}

fn abandoned_owner() -> CellOwner {
    owned_by("abandoned")
}

fn neighbour_of(owner: &CellOwner) -> CellOwner {
    CellOwner {
        cell: owner.cell.clone(),
        plugin: PluginId::from("conformance-neighbour"),
    }
}

fn other_cell(owner: &CellOwner) -> CellOwner {
    CellOwner {
        cell: CellId::from("conformance-other-cell"),
        plugin: owner.plugin.clone(),
    }
}

fn sample_grants() -> Vec<PreparedGrant> {
    let mut closure = GrantClosure::for_owner(conformance_owner());
    for capability in sample_capabilities() {
        closure = if matches!(&capability, Capability::Broker { .. }) {
            closure.generation_bound(capability)
        } else {
            closure.hot(capability)
        };
    }
    closure.prepare()
}

fn sample_capabilities() -> Vec<Capability> {
    vec![
        uds_socket("/run/conformance/egressd.uds"),
        proxy_map("api.plasmosome.test", "splice"),
        broker("egressd"),
        mount("/src/conformance", "/workspace"),
        session_file("/var/lib/plasmosome/conformance/skills/pr.md"),
    ]
}

fn grants_with_two_of_one_class() -> Vec<PreparedGrant> {
    let mut grants = sample_grants();
    grants.push(hot_grant(
        second_owner(),
        &session_file("/var/lib/plasmosome/conformance/skills/review.md"),
    ));
    grants
}

fn one_grant() -> PreparedGrant {
    sample_grants().remove(0)
}

fn requested_object(entry: &LedgerEntry, request: &PreparedGrant) -> OsObject {
    contract_assert_eq!(
        entry.handle,
        request.handle(),
        "a grant must issue exactly the address its caller prepared"
    );
    contract_assert_eq!(
        &entry.owner,
        request.operation.owner(),
        "a grant must retain its requested owner"
    );
    contract_assert_eq!(
        entry.capability,
        request.operation.capability(),
        "a grant must retain its complete requested capability"
    );
    contract_assert_eq!(
        entry.kind,
        request.kind,
        "a grant must retain its requested lifecycle kind"
    );
    request.operation.object()
}

fn policy_of(drain: DrainSpec) -> &'static str {
    match drain.policy {
        RevokePolicy::Graceful => "drained",
        RevokePolicy::Force => "forced",
    }
}

fn residue_objects() -> Vec<OsObject> {
    residue_objects_with(GrantId::new)
}

fn residue_objects_with(mut new_id: impl FnMut() -> GrantId) -> Vec<OsObject> {
    sample_capabilities()
        .into_iter()
        .map(|capability| OsObject {
            id: new_id(),
            owner: abandoned_owner(),
            capability,
        })
        .collect()
}

fn op_for(id: GrantId, owner: CellOwner, capability: Capability) -> UniverseOp {
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

fn holds_exact(state: &OsState, expected: &OsObject) -> bool {
    state.objects().any(|object| object == expected)
}

fn owner_holds_key(state: &OsState, owner: &CellOwner, class: UniverseClass, key: &str) -> bool {
    state
        .objects()
        .any(|object| &object.owner == owner && object.class() == class && object.key() == key)
}

fn remove_expected(expected: &mut Vec<OsObject>, removed: &OsObject) {
    let index = expected
        .iter()
        .position(|object| object == removed)
        .expect("the removed exact object must be expected");
    expected.remove(index);
}

fn unchanged_since<B: EnforcementBackend>(backend: &B, seen: &OsState) -> OsState {
    let now = backend.snapshot_os_state();
    contract_assert_eq!(
        now,
        *seen,
        "the state changed after a refused call returned"
    );
    now
}

fn assert_exact_state(actual: &OsState, expected: &[OsObject], context: &str) {
    let mut expected_state = OsState::new();
    for object in expected {
        expected_state
            .insert(object.clone())
            .expect("expected conformance objects must have unique addresses");
    }
    contract_assert_eq!(actual, &expected_state, "{context}");
}

#[cfg(test)]
mod clauses_discriminate;

#[cfg(test)]
mod tests {
    use plasmosome_backend::{
        BrokerLaunch, FileAccess, MountRecipe, ProxyRecipe, ProxyTransport, SessionFileRecipe,
        UdsRecipe,
    };

    use super::*;

    fn assert_op_for_builds(
        capability: Capability,
        literal: impl FnOnce(GrantId, CellOwner) -> UniverseOp,
    ) {
        let id = GrantId::new();
        let owner = CellOwner {
            cell: CellId::from("cell-7"),
            plugin: PluginId::from("audit"),
        };
        let built = op_for(id, owner.clone(), capability.clone());
        assert_eq!(built, literal(id, owner.clone()));
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
    fn op_for_writes_a_session_file_with_its_id_owner_path_and_recipe() {
        let recipe = SessionFileRecipe {
            contents: b"review the PR".to_vec(),
            mode: FileAccess::ReadWrite,
            guest_path: "/home/agent/skills/pr.md".to_string(),
        };
        assert_op_for_builds(
            Capability::SessionFile {
                path: "/skills/pr.md".to_string(),
                recipe: recipe.clone(),
            },
            |id, owner| UniverseOp::WriteSessionFile {
                id,
                path: "/skills/pr.md".to_string(),
                recipe,
                owner,
            },
        );
    }

    #[test]
    fn op_for_binds_a_socket_with_its_id_owner_path_and_recipe() {
        let recipe = UdsRecipe {
            upstream: "/run/egressd/upstream.sock".to_string(),
            guest_path: "/run/agent/egressd.sock".to_string(),
        };
        assert_op_for_builds(
            Capability::UdsSocket {
                path: "/run/plasmosome/egressd.uds".to_string(),
                recipe: recipe.clone(),
            },
            |id, owner| UniverseOp::BindUds {
                id,
                path: "/run/plasmosome/egressd.uds".to_string(),
                recipe,
                owner,
            },
        );
    }

    #[test]
    fn op_for_sets_a_proxy_map_with_its_id_owner_host_route_and_recipe() {
        let recipe = ProxyRecipe {
            transport: ProxyTransport::Udp,
            destination: "dns.plasmosome.test".to_string(),
            port: 53,
            allow_private: true,
        };
        assert_op_for_builds(
            Capability::ProxyMap {
                host: "api.github.com".to_string(),
                route: "splice".to_string(),
                recipe: recipe.clone(),
            },
            |id, owner| UniverseOp::SetProxyMap {
                id,
                host: "api.github.com".to_string(),
                route: "splice".to_string(),
                recipe,
                owner,
            },
        );
    }

    #[test]
    fn op_for_spawns_a_broker_with_its_id_owner_name_and_launch() {
        let launch = BrokerLaunch {
            command: vec!["/usr/libexec/egressd".to_string(), "--serve".to_string()],
            control_socket: "/run/plasmosome/egressd.control".to_string(),
            data_socket: "/run/plasmosome/egressd.data".to_string(),
        };
        assert_op_for_builds(
            Capability::Broker {
                name: "egressd".to_string(),
                launch: launch.clone(),
            },
            |id, owner| UniverseOp::SpawnBroker {
                id,
                name: "egressd".to_string(),
                launch,
                owner,
            },
        );
    }

    #[test]
    fn op_for_adds_a_mount_with_its_id_owner_source_target_and_recipe() {
        let recipe = MountRecipe {
            access: FileAccess::ReadWrite,
        };
        assert_op_for_builds(
            Capability::Mount {
                source: "/srv/repo".to_string(),
                target: "/workspace".to_string(),
                recipe: recipe.clone(),
            },
            |id, owner| UniverseOp::AddMount {
                id,
                source: "/srv/repo".to_string(),
                target: "/workspace".to_string(),
                recipe,
                owner,
            },
        );
    }
}
