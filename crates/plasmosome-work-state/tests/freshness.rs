use plasmosome_work_state::freshness::{
    Freshness, ObservationState, PendingMutations, RemoteRelation, classify,
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

    let mut duplicate_pending = observation(RemoteRelation::Unknown, &["one", "one"]);
    duplicate_pending
        .pending_mutations
        .operation_ids
        .push(" ".into());
    invalids.push(duplicate_pending);

    for state in invalids {
        assert_eq!(classify(state).unwrap_err().code(), "invalid_freshness");
    }
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
