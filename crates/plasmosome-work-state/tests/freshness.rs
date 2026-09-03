use plasmosome_work_state::freshness::{
    Freshness, ObservationState, PendingMutations, RemoteRelation, classify,
    record_failed_sync_observation,
};

const LOCAL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const REMOTE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const TIME: &str = "2026-09-02T12:34:56Z";

fn observation(relation: RemoteRelation, pending: &[&str]) -> ObservationState {
    let observed = match relation {
        RemoteRelation::Unknown => (None, None, None, None),
        RemoteRelation::Equivalent => (
            Some(REMOTE.into()),
            Some(TIME.into()),
            Some(LOCAL.into()),
            Some(TIME.into()),
        ),
        RemoteRelation::Ahead => (
            Some(REMOTE.into()),
            Some(TIME.into()),
            Some(LOCAL.into()),
            None,
        ),
    };
    ObservationState {
        last_successful_sync_at: observed.3,
        local_generation: LOCAL.into(),
        remote_generation: observed.0,
        remote_observed_at: observed.1,
        observed_local_generation: observed.2,
        remote_relation: relation,
        pending_mutations: PendingMutations {
            operation_ids: pending.iter().map(|value| (*value).into()).collect(),
        },
    }
}

#[test]
fn freshness_classifies_the_six_spec_states() {
    let cases: &[(RemoteRelation, &[&str], Freshness)] = &[
        (RemoteRelation::Equivalent, &[], Freshness::SynchronizedAsOf),
        (RemoteRelation::Ahead, &[], Freshness::Stale),
        (RemoteRelation::Unknown, &[], Freshness::Unknown),
        (
            RemoteRelation::Equivalent,
            &["operation-1"],
            Freshness::Unpublished,
        ),
        (
            RemoteRelation::Ahead,
            &["operation-1"],
            Freshness::StaleWithUnpublished,
        ),
        (
            RemoteRelation::Unknown,
            &["operation-1"],
            Freshness::UnknownWithUnpublished,
        ),
    ];

    for case in cases {
        let relation = case.0.clone();
        let pending = case.1;
        let expected = case.2.clone();
        let envelope = classify(observation(relation, pending)).expect("valid observation");
        assert_eq!(envelope.freshness, expected);
        assert_eq!(envelope.pending_mutations.count, pending.len());
        assert_eq!(
            envelope.pending_mutations.operation_ids,
            pending
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn invalid_or_partial_observation_state_is_refused() {
    let mut invalids = Vec::new();

    let mut bad_time = observation(RemoteRelation::Equivalent, &[]);
    bad_time.remote_observed_at = Some("2026-02-30T12:34:56Z".into());
    invalids.push(bad_time);

    let mut partial = observation(RemoteRelation::Unknown, &[]);
    partial.remote_generation = Some(REMOTE.into());
    invalids.push(partial);

    let mut equivalent_without_sync = observation(RemoteRelation::Equivalent, &[]);
    equivalent_without_sync.last_successful_sync_at = None;
    invalids.push(equivalent_without_sync);

    let mut stale_without_base = observation(RemoteRelation::Ahead, &[]);
    stale_without_base.observed_local_generation = Some("different".into());
    invalids.push(stale_without_base);

    let duplicate_pending = observation(RemoteRelation::Unknown, &["one", "one"]);
    invalids.push(duplicate_pending);

    let blank_pending = observation(RemoteRelation::Unknown, &["one", " "]);
    invalids.push(blank_pending);

    for state in invalids {
        assert_eq!(classify(state).unwrap_err().code(), "invalid_freshness");
    }
}

#[test]
fn unknown_without_remote_observation_refuses_a_lone_successful_sync_timestamp() {
    let mut state = observation(RemoteRelation::Unknown, &[]);
    state.last_successful_sync_at = Some(TIME.into());

    assert_eq!(classify(state).unwrap_err().code(), "invalid_freshness");
}

#[test]
fn unknown_preserves_a_complete_observation_with_historical_sync() {
    let mut state = observation(RemoteRelation::Ahead, &[]);
    state.remote_relation = RemoteRelation::Unknown;
    state.last_successful_sync_at = Some("2026-09-01T12:34:56Z".into());

    let freshness = classify(state).expect("a complete historical observation remains valid");
    assert_eq!(freshness.freshness, Freshness::Unknown);
    assert_eq!(
        freshness.last_successful_sync_at.as_deref(),
        Some("2026-09-01T12:34:56Z")
    );
}

#[test]
fn equivalent_reobservation_may_postdate_successful_sync() {
    let mut state = observation(RemoteRelation::Equivalent, &[]);
    state.last_successful_sync_at = Some("2026-09-01T12:34:56Z".into());

    let freshness = classify(state).expect("later equivalent observation remains valid");
    assert_eq!(freshness.freshness, Freshness::SynchronizedAsOf);
    assert_eq!(
        freshness.last_successful_sync_at.as_deref(),
        Some("2026-09-01T12:34:56Z")
    );
    assert_eq!(freshness.remote_observed_at.as_deref(), Some(TIME));
}

#[test]
fn failed_sync_records_unknown_without_erasing_history() {
    let prior = observation(RemoteRelation::Equivalent, &["operation-1"]);
    let updated = record_failed_sync_observation(
        &prior,
        "cccccccccccccccccccccccccccccccccccccccc",
        "2026-09-02T12:35:00Z",
    )
    .expect("a complete failed-sync observation is valid");

    assert_eq!(
        updated.last_successful_sync_at,
        prior.last_successful_sync_at
    );
    assert_eq!(updated.local_generation, prior.local_generation);
    assert_eq!(updated.pending_mutations, prior.pending_mutations);
    assert_eq!(
        updated.remote_generation.as_deref(),
        Some("cccccccccccccccccccccccccccccccccccccccc")
    );
    assert_eq!(
        updated.remote_observed_at.as_deref(),
        Some("2026-09-02T12:35:00Z")
    );
    assert_eq!(updated.observed_local_generation.as_deref(), Some(LOCAL));
    assert_eq!(updated.remote_relation, RemoteRelation::Unknown);
    assert_eq!(
        classify(updated)
            .expect("updated observation classifies")
            .freshness,
        Freshness::UnknownWithUnpublished
    );
}

#[test]
fn pending_at_the_last_equivalent_generation_remains_unpublished() {
    let prior = observation(
        RemoteRelation::Equivalent,
        &["pending-first", "pending-second"],
    );
    let updated = record_failed_sync_observation(&prior, REMOTE, "2026-09-02T12:35:00Z")
        .expect("a later observation of the already-equivalent remote is valid");

    assert_eq!(
        updated.last_successful_sync_at,
        prior.last_successful_sync_at
    );
    assert_eq!(updated.remote_relation, RemoteRelation::Equivalent);
    assert_eq!(updated.remote_generation.as_deref(), Some(REMOTE));
    assert_eq!(
        updated.remote_observed_at.as_deref(),
        Some("2026-09-02T12:35:00Z")
    );
    assert_eq!(updated.observed_local_generation.as_deref(), Some(LOCAL));
    assert_eq!(
        updated.pending_mutations.operation_ids,
        vec!["pending-first", "pending-second"]
    );
    assert_eq!(classify(updated).unwrap().freshness, Freshness::Unpublished);
}

#[test]
fn pending_at_a_different_or_unknown_generation_is_unknown_with_unpublished() {
    let prior = observation(
        RemoteRelation::Equivalent,
        &["pending-first", "pending-second"],
    );
    let changed = record_failed_sync_observation(
        &prior,
        "cccccccccccccccccccccccccccccccccccccccc",
        "2026-09-02T12:35:00Z",
    )
    .expect("a later different remote observation is valid");
    assert_eq!(
        changed.last_successful_sync_at,
        prior.last_successful_sync_at
    );
    assert_eq!(changed.remote_relation, RemoteRelation::Unknown);
    assert_eq!(
        changed.pending_mutations.operation_ids,
        vec!["pending-first", "pending-second"]
    );
    assert_eq!(
        classify(changed).unwrap().freshness,
        Freshness::UnknownWithUnpublished
    );

    let unknown_prior = observation(
        RemoteRelation::Unknown,
        &["pending-first", "pending-second"],
    );
    let unknown = record_failed_sync_observation(
        &unknown_prior,
        "dddddddddddddddddddddddddddddddddddddddd",
        "2026-09-02T12:35:00Z",
    )
    .expect("an unknown prior state may retain a complete new observation");
    assert_eq!(unknown.last_successful_sync_at, None);
    assert_eq!(unknown.remote_relation, RemoteRelation::Unknown);
    assert_eq!(
        unknown.pending_mutations.operation_ids,
        vec!["pending-first", "pending-second"]
    );
    assert_eq!(
        classify(unknown).unwrap().freshness,
        Freshness::UnknownWithUnpublished
    );
}

#[test]
fn failed_sync_observation_refuses_a_regressing_timestamp() {
    let prior = observation(RemoteRelation::Equivalent, &[]);

    assert_eq!(
        record_failed_sync_observation(
            &prior,
            "cccccccccccccccccccccccccccccccccccccccc",
            "2026-09-01T12:34:56Z",
        )
        .unwrap_err()
        .code(),
        "invalid_freshness"
    );
}

#[test]
fn freshness_refuses_noncanonical_local_commit_forms() {
    let mut local = observation(RemoteRelation::Unknown, &[]);
    local.local_generation = format!(" {LOCAL}");
    assert_eq!(classify(local).unwrap_err().code(), "invalid_freshness");

    let mut observed = observation(RemoteRelation::Equivalent, &["operation-1"]);
    observed.observed_local_generation = Some(format!("{LOCAL} "));
    assert_eq!(classify(observed).unwrap_err().code(), "invalid_freshness");
}
