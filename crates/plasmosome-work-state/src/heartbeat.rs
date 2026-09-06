//! Typed, read-only heartbeat observation and guarded application seams.

use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

pub use crate::document::DocumentKind;
use crate::freshness::{Freshness, FreshnessEnvelope};

/// The lifecycle outcome of one bounded observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationState {
    Refused,
    Idle,
    Proposed,
}

/// A typed document projection supplied by the local work-state reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeartbeatRecord {
    pub document_key: String,
    pub kind: DocumentKind,
    pub lifecycle: String,
    pub intent_ids: Vec<String>,
    pub spec_ids: Vec<String>,
    pub state_version: u64,
    pub generation: String,
}

/// Inputs to a single read-only heartbeat sweep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeartbeatInput {
    pub repository_key: String,
    pub actor: String,
    pub session: String,
    pub operation_id: String,
    pub observed_at: String,
    pub freshness: FreshnessEnvelope,
    pub records: Vec<HeartbeatRecord>,
}

/// A safe, typed action proposal. It grants no lifecycle or dispatch authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProposedAction {
    pub action_id: String,
    pub observation_id: String,
    pub target_key: String,
    pub expected_state_version: u64,
    pub expected_generation: String,
    pub required_authority: String,
}

/// A non-authorizing request to create or repair a missing layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlannerRequest {
    pub request_id: String,
    pub layer: String,
    pub target_key: String,
}

/// Complete result of one observation, including explicit refusal state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HeartbeatObservation {
    pub operation_id: String,
    pub observation_id: String,
    pub state: ObservationState,
    pub freshness: FreshnessEnvelope,
    pub checked_keys: Vec<String>,
    pub actions: Vec<ProposedAction>,
    pub planner_requests: Vec<PlannerRequest>,
    pub refusal: Option<String>,
}

/// Authority presented by the caller at application time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeartbeatAuthority {
    pub actor: String,
    pub session: String,
    pub generation: String,
}

/// Cooperative cancellation token for one heartbeat operation.
#[derive(Clone, Default, Debug)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancelled() -> Self {
        let value = Self::default();
        value.0.store(true, Ordering::Release);
        value
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Receipt outcome for an application attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptOutcome {
    Applied,
    AlreadyApplied,
    Refused,
    Cancelled,
    Failed,
}

/// An append-only outcome returned by the application seam.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HeartbeatReceipt {
    pub operation_id: String,
    pub action_id: String,
    pub observation_id: String,
    pub target_key: String,
    pub outcome: ReceiptOutcome,
    pub diagnostic_code: String,
}

/// The existing authoritative writer/effect boundary, injected by the caller.
pub trait HeartbeatApplySeam {
    fn receipt(&self, operation_id: &str) -> Option<HeartbeatReceipt>;
    /// Rechecks the document version, lifecycle, writer fence, and publication authority.
    fn revalidate(
        &self,
        action: &ProposedAction,
        authority: &HeartbeatAuthority,
    ) -> Result<(), &'static str>;
    fn apply(
        &mut self,
        action: &ProposedAction,
        authority: &HeartbeatAuthority,
    ) -> Result<(), &'static str>;
    fn record_receipt(&mut self, receipt: HeartbeatReceipt);
}

/// A deterministic test seam; production adapters must route these methods to Spec 014.
#[derive(Default)]
pub struct RecordingHeartbeatSeam {
    pub calls: usize,
    receipts: BTreeMap<String, HeartbeatReceipt>,
}
impl HeartbeatApplySeam for RecordingHeartbeatSeam {
    fn receipt(&self, operation_id: &str) -> Option<HeartbeatReceipt> {
        self.receipts.get(operation_id).cloned()
    }
    fn apply(
        &mut self,
        _action: &ProposedAction,
        _authority: &HeartbeatAuthority,
    ) -> Result<(), &'static str> {
        self.calls += 1;
        Ok(())
    }
    fn revalidate(
        &self,
        _action: &ProposedAction,
        _authority: &HeartbeatAuthority,
    ) -> Result<(), &'static str> {
        Ok(())
    }
    fn record_receipt(&mut self, receipt: HeartbeatReceipt) {
        self.receipts.insert(receipt.operation_id.clone(), receipt);
    }
}

fn id(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part.len().to_string().as_bytes());
        digest.update(b":");
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

/// Observes local state without mutation, approval, claiming, starting, or dispatching.
pub fn heartbeat_observe(input: HeartbeatInput) -> HeartbeatObservation {
    let observation_id = id(&[
        &input.repository_key,
        &input.operation_id,
        &input.freshness.local_generation,
        &input.observed_at,
    ]);
    let checked_keys = input
        .records
        .iter()
        .map(|r| r.document_key.clone())
        .collect::<Vec<_>>();
    let base = |state, refusal, actions, planner_requests| HeartbeatObservation {
        operation_id: input.operation_id.clone(),
        observation_id: observation_id.clone(),
        state,
        freshness: input.freshness.clone(),
        checked_keys: checked_keys.clone(),
        actions,
        planner_requests,
        refusal,
    };
    if !matches!(input.freshness.freshness, Freshness::SynchronizedAsOf) {
        return base(
            ObservationState::Refused,
            Some(
                match input.freshness.freshness {
                    Freshness::Unknown | Freshness::UnknownWithUnpublished => "unknown_freshness",
                    Freshness::Stale | Freshness::StaleWithUnpublished => "stale_observation",
                    Freshness::Unpublished => "unpublished_state",
                    Freshness::SynchronizedAsOf => unreachable!(),
                }
                .into(),
            ),
            vec![],
            vec![],
        );
    }
    for record in &input.records {
        if !matches!(record.kind, DocumentKind::Task)
            || !matches!(
                record.lifecycle.as_str(),
                "todo" | "planned" | "in_progress" | "in_review" | "done"
            )
        {
            return base(
                ObservationState::Refused,
                Some("malformed_status".into()),
                vec![],
                vec![],
            );
        }
        if record.spec_ids.is_empty()
            || record.intent_ids.is_empty()
            || record.generation != input.freshness.local_generation
        {
            return base(
                ObservationState::Refused,
                Some("link_conflict".into()),
                vec![],
                vec![],
            );
        }
    }
    let actions = input
        .records
        .iter()
        .filter(|r| r.lifecycle == "planned")
        .map(|r| ProposedAction {
            action_id: id(&[
                &observation_id,
                &r.document_key,
                &r.state_version.to_string(),
                &r.generation,
            ]),
            observation_id: observation_id.clone(),
            target_key: r.document_key.clone(),
            expected_state_version: r.state_version,
            expected_generation: r.generation.clone(),
            required_authority: format!("task:{}", r.document_key),
        })
        .collect::<Vec<_>>();
    base(
        if actions.is_empty() {
            ObservationState::Idle
        } else {
            ObservationState::Proposed
        },
        None,
        actions,
        vec![],
    )
}

/// Applies one proposal only after exact freshness and authority revalidation.
pub fn heartbeat_apply<S: HeartbeatApplySeam>(
    observation: &HeartbeatObservation,
    action: &ProposedAction,
    authority: &HeartbeatAuthority,
    cancellation: &Cancellation,
    seam: &mut S,
) -> HeartbeatReceipt {
    let receipt = |outcome: ReceiptOutcome, code: &str| HeartbeatReceipt {
        operation_id: observation.operation_id.clone(),
        action_id: action.action_id.clone(),
        observation_id: action.observation_id.clone(),
        target_key: action.target_key.clone(),
        outcome,
        diagnostic_code: code.into(),
    };
    if let Some(existing) = seam.receipt(&observation.operation_id) {
        if existing.observation_id == action.observation_id
            && existing.target_key == action.target_key
        {
            return HeartbeatReceipt {
                outcome: ReceiptOutcome::AlreadyApplied,
                ..existing
            };
        }
        return receipt(ReceiptOutcome::Refused, "authority_conflict");
    }
    if cancellation.is_cancelled() {
        let result = receipt(ReceiptOutcome::Cancelled, "cancelled");
        seam.record_receipt(result.clone());
        return result;
    }
    if observation.state != ObservationState::Proposed
        || observation.observation_id != action.observation_id
    {
        return receipt(ReceiptOutcome::Refused, "stale_observation");
    }
    if !matches!(observation.freshness.freshness, Freshness::SynchronizedAsOf)
        || action.expected_generation != observation.freshness.local_generation
        || authority.generation != action.expected_generation
    {
        return receipt(ReceiptOutcome::Refused, "generation_conflict");
    }
    if authority.actor.is_empty() || authority.session.is_empty() {
        return receipt(ReceiptOutcome::Refused, "authority_conflict");
    }
    if let Err(code) = seam.revalidate(action, authority) {
        let outcome = if matches!(
            code,
            "state_version_conflict" | "generation_conflict" | "stale_observation"
        ) {
            ReceiptOutcome::Refused
        } else {
            ReceiptOutcome::Failed
        };
        return receipt(outcome, code);
    }
    let result = match seam.apply(action, authority) {
        Ok(()) => receipt(ReceiptOutcome::Applied, "ok"),
        Err(code) => receipt(ReceiptOutcome::Failed, code),
    };
    seam.record_receipt(result.clone());
    result
}
