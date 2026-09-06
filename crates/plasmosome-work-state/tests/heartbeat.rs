use plasmosome_work_state::freshness::{Freshness, FreshnessEnvelope, PendingMutationEnvelope};
use plasmosome_work_state::heartbeat::*;

fn envelope(freshness: Freshness) -> FreshnessEnvelope {
    FreshnessEnvelope {
        last_successful_sync_at: Some("2026-09-06T12:00:00Z".into()),
        local_generation: "a".repeat(40),
        remote_generation: Some("a".repeat(40)),
        remote_observed_at: Some("2026-09-06T12:00:00Z".into()),
        pending_mutations: PendingMutationEnvelope {
            count: 0,
            operation_ids: vec![],
        },
        freshness,
    }
}

fn input() -> HeartbeatInput {
    HeartbeatInput {
        repository_key: "repo:test".into(),
        actor: "agent".into(),
        session: "session".into(),
        operation_id: "heartbeat-1".into(),
        observed_at: "2026-09-06T12:00:01Z".into(),
        freshness: envelope(Freshness::SynchronizedAsOf),
        records: vec![HeartbeatRecord {
            document_key: "task:049".into(),
            kind: DocumentKind::Task,
            lifecycle: "planned".into(),
            intent_ids: vec!["015".into()],
            spec_ids: vec!["015".into()],
            state_version: 3,
            generation: "a".repeat(40),
        }],
    }
}

#[test]
fn observation_is_deterministic_and_proposes_reconciliation() {
    let first = heartbeat_observe(input());
    let second = heartbeat_observe(input());
    assert_eq!(first, second);
    assert_eq!(first.state, ObservationState::Proposed);
    assert_eq!(first.actions.len(), 1);
    assert_eq!(first.actions[0].expected_state_version, 3);
}

#[test]
fn unsafe_freshness_is_an_explicit_refusal() {
    let mut value = input();
    value.freshness.freshness = Freshness::Unknown;
    let observed = heartbeat_observe(value);
    assert_eq!(observed.state, ObservationState::Refused);
    assert_eq!(observed.refusal.as_deref(), Some("unknown_freshness"));
    assert!(observed.actions.is_empty());
}

#[test]
fn malformed_status_and_links_are_refused() {
    let mut value = input();
    value.records[0].lifecycle = "running".into();
    let observed = heartbeat_observe(value);
    assert_eq!(observed.refusal.as_deref(), Some("malformed_status"));
    let mut value = input();
    value.records[0].spec_ids.clear();
    assert_eq!(
        heartbeat_observe(value).refusal.as_deref(),
        Some("link_conflict")
    );
}

#[test]
fn apply_revalidates_and_replays_receipt() {
    let observed = heartbeat_observe(input());
    let action = observed.actions[0].clone();
    let mut seam = RecordingHeartbeatSeam::default();
    let authority = HeartbeatAuthority {
        actor: "agent".into(),
        session: "session".into(),
        generation: "a".repeat(40),
    };
    let cancellation = Cancellation::default();
    let first = heartbeat_apply(&observed, &action, &authority, &cancellation, &mut seam);
    let second = heartbeat_apply(&observed, &action, &authority, &cancellation, &mut seam);
    assert_eq!(first.outcome, ReceiptOutcome::Applied);
    assert_eq!(second.outcome, ReceiptOutcome::AlreadyApplied);
    assert_eq!(first.operation_id, second.operation_id);
    assert_eq!(seam.calls, 1);
}

#[test]
fn cancellation_before_effect_has_no_call() {
    let observed = heartbeat_observe(input());
    let mut seam = RecordingHeartbeatSeam::default();
    let authority = HeartbeatAuthority {
        actor: "agent".into(),
        session: "session".into(),
        generation: "a".repeat(40),
    };
    let cancellation = Cancellation::cancelled();
    let receipt = heartbeat_apply(
        &observed,
        &observed.actions[0],
        &authority,
        &cancellation,
        &mut seam,
    );
    assert_eq!(receipt.outcome, ReceiptOutcome::Cancelled);
    assert_eq!(seam.calls, 0);
}
