#[cfg(test)]
use std::cell::Cell;
use std::time::Duration;

use plasmosome_backend::{
    BackendError, Capability, DrainSpec, EnforcementBackend, Grant, GrantId, GrantKind, Handle,
    LedgerEntry, OsObject, OsState, PluginId, RevokePolicy, UniverseClass, UniverseOp,
};

use crate::builders::GrantSequence;

const CONFORMANCE_PLUGIN: &str = "conformance";
const SECOND_PLUGIN: &str = "conformance-second";
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

/// Checks that a grant's exact ledger entry is returned by either revoke policy.
pub fn grant_is_replayable<B: EnforcementBackend>(make: impl Fn() -> B) {
    for drain in drains() {
        let mut backend = make();
        for grant in sample_grants() {
            let entry = backend.grant(grant.clone());
            contract_assert_eq!(entry.plugin, grant.plugin);
            contract_assert_eq!(entry.capability, grant.capability);
            contract_assert_eq!(entry.kind, grant.kind);
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
        let live = backend.grant(one_grant());
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
        for grant in sample_grants() {
            let entry = backend.grant(grant);
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
        for grant in sample_grants() {
            let entry = backend.grant(grant);
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

/// Checks that snapshots contain every and only requested grant or planted object.
pub fn snapshot_never_invents_objects<B: EnforcementBackend>(make: impl Fn() -> B) {
    contract_assert!(make().snapshot_os_state().is_empty());
    let mut backend = make();
    let mut expected = Vec::new();
    for grant in sample_grants() {
        let entry = backend.grant(grant.clone());
        expected.push(requested_object(&entry, &grant));
    }
    let residue = residue_objects().remove(0);
    contract_expect!(
        backend.plant(residue.clone()),
        "the residue fixture must plant"
    );
    expected.push(residue);
    assert_exact_state(
        &backend.snapshot_os_state(),
        &expected,
        "the snapshot lost or invented an object",
    );
}

/// Checks distinct live handles, both revoke orders, and complete survivor snapshots.
pub fn live_grants_hold_distinct_handles<B: EnforcementBackend>(make: impl Fn() -> B) {
    for order in orders() {
        let mut backend = make();
        let mut live = Vec::new();
        for grant in grants_with_two_of_one_class() {
            let entry = backend.grant(grant);
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
        for order in orders() {
            let mut backend = make();
            let mut expected: Vec<OsObject> = residue_objects_with(&mut new_id)
                .into_iter()
                .filter(|object| object.class() != capability.class())
                .collect();
            let residue = OsObject {
                id: new_id(),
                owner: PluginId::from(SECOND_PLUGIN),
                capability: capability.clone(),
            };
            expected.push(residue.clone());
            for object in &expected {
                contract_expect!(
                    backend.plant(object.clone()),
                    "the residue fixture must plant"
                );
            }
            let first = op_for(new_id(), CONFORMANCE_PLUGIN, capability.clone());
            let second = op_for(new_id(), CONFORMANCE_PLUGIN, capability.clone());
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
                backend
                    .apply_removal(op.removal(), &object.owner)
                    .unwrap_or_else(|error| {
                        contract_panic!(
                            "removing the applied {} on the {} pass failed: {error}",
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
                contract_assert!(holds_exact(&state, &residue));
            }
        }
    }
}

/// Checks that spent handles stay spent after later grants under either policy.
pub fn revoke_of_a_revoked_handle_is_error<B: EnforcementBackend>(make: impl Fn() -> B) {
    for drain in drains() {
        let mut backend = make();
        let mut spent = Vec::new();
        for grant in sample_grants().into_iter().take(2) {
            let entry = backend.grant(grant);
            let object = entry.object();
            contract_unwrap!(backend.revoke(entry.handle, drain));
            spent.push((entry, object));
        }
        let live = backend.grant(sample_grants().remove(2));
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
    for capability in sample_capabilities() {
        for drain in drains() {
            for order in orders() {
                let mut backend = make();
                let audit_request = Grant {
                    plugin: PluginId::from("audit-owner"),
                    capability: capability.clone(),
                    kind: GrantKind::Hot,
                };
                let audit = backend.grant(audit_request.clone());
                let audit_object = requested_object(&audit, &audit_request);
                let deploy_request = Grant {
                    plugin: PluginId::from("deploy-owner"),
                    capability: capability.clone(),
                    kind: GrantKind::Hot,
                };
                let deploy = backend.grant(deploy_request.clone());
                let deploy_object = requested_object(&deploy, &deploy_request);
                let mut expected = vec![audit_object.clone(), deploy_object.clone()];
                for (entry, object) in
                    order.arrange(vec![(audit, audit_object), (deploy, deploy_object)])
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

/// Checks repeated equal grants, complete resource collisions, and broker residue.
pub fn repeated_grants_are_independently_removable<B: EnforcementBackend>(make: impl Fn() -> B) {
    let mut pairs: Vec<(Capability, Capability)> = sample_capabilities()
        .into_iter()
        .map(|capability| (capability.clone(), capability))
        .collect();
    pairs.push((
        Capability::Mount {
            source: "/secrets".to_string(),
            target: "/workspace".to_string(),
        },
        Capability::Mount {
            source: "/code".to_string(),
            target: "/workspace".to_string(),
        },
    ));
    pairs.push((
        Capability::ProxyMap {
            host: "routes.plasmosome.test".to_string(),
            route: "audit".to_string(),
        },
        Capability::ProxyMap {
            host: "routes.plasmosome.test".to_string(),
            route: "deploy".to_string(),
        },
    ));
    pairs.push((
        Capability::Broker {
            pid: 5150,
            name: "egressd".to_string(),
        },
        Capability::Broker {
            pid: 5150,
            name: "auditd".to_string(),
        },
    ));

    for (first_capability, second_capability) in pairs {
        for drain in drains() {
            for order in orders() {
                let mut backend = make();
                let first_request = Grant {
                    plugin: PluginId::from(CONFORMANCE_PLUGIN),
                    capability: first_capability.clone(),
                    kind: GrantKind::Hot,
                };
                let first = backend.grant(first_request.clone());
                let first_object = requested_object(&first, &first_request);
                let second_request = Grant {
                    plugin: PluginId::from(CONFORMANCE_PLUGIN),
                    capability: second_capability.clone(),
                    kind: GrantKind::Hot,
                };
                let second = backend.grant(second_request.clone());
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

    let capability = Capability::Broker {
        pid: 31337,
        name: "egressd".to_string(),
    };
    for drain in drains() {
        let mut backend = make();
        let residue = OsObject {
            id: GrantId::new(),
            owner: PluginId::from(CONFORMANCE_PLUGIN),
            capability: capability.clone(),
        };
        contract_expect!(backend.plant(residue.clone()), "broker residue must plant");
        let live_request = Grant {
            plugin: residue.owner.clone(),
            capability: capability.clone(),
            kind: GrantKind::Hot,
        };
        let live = backend.grant(live_request.clone());
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
        ));
        contract_assert!(backend.snapshot_os_state().is_empty());
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

fn sample_grants() -> Vec<Grant> {
    let mut sequence = GrantSequence::for_plugin(CONFORMANCE_PLUGIN);
    for capability in sample_capabilities() {
        sequence = if matches!(&capability, Capability::Broker { .. }) {
            sequence.generation_bound(capability)
        } else {
            sequence.hot(capability)
        };
    }
    sequence.into_grants()
}

fn sample_capabilities() -> Vec<Capability> {
    vec![
        Capability::UdsSocket {
            path: "/run/conformance/egressd.uds".to_string(),
        },
        Capability::ProxyMap {
            host: "api.plasmosome.test".to_string(),
            route: "splice".to_string(),
        },
        Capability::Broker {
            pid: 4242,
            name: "egressd".to_string(),
        },
        Capability::Mount {
            source: "/src/conformance".to_string(),
            target: "/workspace".to_string(),
        },
        Capability::SessionFile {
            path: "skills/pr.md".to_string(),
        },
    ]
}

fn grants_with_two_of_one_class() -> Vec<Grant> {
    let mut grants = sample_grants();
    grants.push(Grant {
        plugin: PluginId::from(SECOND_PLUGIN),
        capability: Capability::SessionFile {
            path: "skills/review.md".to_string(),
        },
        kind: GrantKind::Hot,
    });
    grants
}

fn one_grant() -> Grant {
    sample_grants().remove(0)
}

fn requested_object(entry: &LedgerEntry, request: &Grant) -> OsObject {
    contract_assert_eq!(
        entry.handle.class,
        request.capability.class(),
        "a grant handle must name the requested capability class"
    );
    contract_assert_eq!(
        entry.plugin,
        request.plugin,
        "a grant must retain its requested owner"
    );
    contract_assert_eq!(
        entry.capability,
        request.capability,
        "a grant must retain its complete requested capability"
    );
    contract_assert_eq!(
        entry.kind,
        request.kind,
        "a grant must retain its requested lifecycle kind"
    );
    OsObject {
        id: entry.handle.id,
        owner: request.plugin.clone(),
        capability: request.capability.clone(),
    }
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
            owner: PluginId::from("abandoned"),
            capability,
        })
        .collect()
}

fn op_for(id: GrantId, owner: &str, capability: Capability) -> UniverseOp {
    let owner = PluginId::from(owner);
    match capability {
        Capability::SessionFile { path } => UniverseOp::WriteSessionFile { id, path, owner },
        Capability::UdsSocket { path } => UniverseOp::BindUds { id, path, owner },
        Capability::ProxyMap { host, route } => UniverseOp::SetProxyMap {
            id,
            host,
            route,
            owner,
        },
        Capability::Broker { pid, name } => UniverseOp::SpawnBroker {
            id,
            pid,
            name,
            owner,
        },
        Capability::Mount { source, target } => UniverseOp::AddMount {
            id,
            source,
            target,
            owner,
        },
    }
}

fn holds_exact(state: &OsState, expected: &OsObject) -> bool {
    state.objects().any(|object| object == expected)
}

fn owner_holds_key(state: &OsState, owner: &PluginId, class: UniverseClass, key: &str) -> bool {
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
