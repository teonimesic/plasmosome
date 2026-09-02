use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// A stable refusal raised while validating one persisted freshness observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreshnessError {
    code: &'static str,
}

impl FreshnessError {
    /// Returns the stable machine-readable refusal code.
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for FreshnessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for FreshnessError {}

fn refusal() -> FreshnessError {
    FreshnessError {
        code: "invalid_freshness",
    }
}

/// The recorded relation between one local generation and the last observed remote generation.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteRelation {
    /// Local and remote were equal at the recorded observation time.
    Equivalent,
    /// A newer remote generation was observed but is not installed locally.
    Ahead,
    /// No trustworthy equality observation is available.
    Unknown,
}

/// The operation ids that exist locally but have not been confirmed remotely.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PendingMutations {
    /// Ordered semantic operation ids.
    pub operation_ids: Vec<String>,
}

/// The persisted facts required to classify a local read without a clock or network request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationState {
    /// The UTC timestamp of an equality-confirming synchronization, if one is known.
    pub last_successful_sync_at: Option<String>,
    /// The committed embedded-Dolt generation used by the read.
    pub local_generation: String,
    /// The last observed remote `refs/dolt/data` generation, if known.
    pub remote_generation: Option<String>,
    /// The UTC timestamp at which that remote generation was observed, if known.
    pub remote_observed_at: Option<String>,
    /// The local generation compared at that observation, if known.
    pub observed_local_generation: Option<String>,
    /// The recorded remote relation.
    pub remote_relation: RemoteRelation,
    /// The ordered pending operation ids.
    pub pending_mutations: PendingMutations,
}

/// The six freshness classifications specified for local projections.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    /// Local and remote were equal at the recorded time.
    SynchronizedAsOf,
    /// A newer remote state was observed.
    Stale,
    /// Remote equality is unknown.
    Unknown,
    /// Local operations remain unpublished while equality is known.
    Unpublished,
    /// Local operations remain unpublished and a newer remote state was observed.
    StaleWithUnpublished,
    /// Local operations remain unpublished and remote equality is unknown.
    UnknownWithUnpublished,
}

/// The complete freshness envelope returned by every local projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FreshnessEnvelope {
    /// The UTC timestamp of the last known successful synchronization.
    pub last_successful_sync_at: Option<String>,
    /// The local embedded-Dolt generation read by this response.
    pub local_generation: String,
    /// The last observed remote `refs/dolt/data` generation.
    pub remote_generation: Option<String>,
    /// The UTC timestamp of the remote observation.
    pub remote_observed_at: Option<String>,
    /// The pending operation count and ordered identifiers.
    pub pending_mutations: PendingMutationEnvelope,
    /// The derived freshness classification.
    pub freshness: Freshness,
}

/// A serialized pending-operation projection with an explicit count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PendingMutationEnvelope {
    /// The number of pending operations.
    pub count: usize,
    /// The ordered pending operation ids.
    pub operation_ids: Vec<String>,
}

fn lower_hex_generation(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Accepts one complete commit token emitted by the pinned `bd vc status` command.
///
/// The transport owns the commit encoding, so the local reader preserves that complete token
/// rather than guessing a different hash grammar. Whitespace would make the persisted value a
/// presentation artifact rather than the exact status value and is never a valid token.
pub(crate) fn full_nonblank_commit(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(char::is_whitespace)
}

/// Returns whether a timestamp has canonical UTC syntax and a real calendar date/time.
pub fn canonical_utc(value: &str) -> bool {
    if value.len() != 20 || !value.ends_with('Z') {
        return false;
    }
    let bytes = value.as_bytes();
    if !matches!(bytes.get(4), Some(b'-'))
        || !matches!(bytes.get(7), Some(b'-'))
        || !matches!(bytes.get(10), Some(b'T'))
        || !matches!(bytes.get(13), Some(b':'))
        || !matches!(bytes.get(16), Some(b':'))
    {
        return false;
    }
    let number = |range: std::ops::Range<usize>| {
        std::str::from_utf8(&bytes[range])
            .ok()
            .filter(|value| value.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|value| value.parse::<u32>().ok())
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        number(0..4),
        number(5..7),
        number(8..10),
        number(11..13),
        number(14..16),
        number(17..19),
    ) else {
        return false;
    };
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

fn valid_pending(ids: &[String]) -> bool {
    let mut seen = BTreeSet::new();
    ids.iter()
        .all(|id| !id.trim().is_empty() && seen.insert(id.as_str()))
}

/// Validates persisted freshness facts without consulting a clock or remote.
pub fn validate(state: &ObservationState) -> Result<(), FreshnessError> {
    if !full_nonblank_commit(&state.local_generation)
        || !valid_pending(&state.pending_mutations.operation_ids)
        || [
            state.last_successful_sync_at.as_deref(),
            state.remote_observed_at.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| !canonical_utc(value))
    {
        return Err(refusal());
    }
    let observed = (
        state.remote_generation.as_deref(),
        state.remote_observed_at.as_deref(),
        state.observed_local_generation.as_deref(),
    );
    match state.remote_relation {
        RemoteRelation::Unknown => match observed {
            (None, None, None) => Ok(()),
            (Some(remote), Some(_), Some(local))
                if lower_hex_generation(remote) && full_nonblank_commit(local) =>
            {
                Ok(())
            }
            _ => Err(refusal()),
        },
        RemoteRelation::Equivalent | RemoteRelation::Ahead => {
            let (Some(remote), Some(observed_at), Some(observed_local)) = observed else {
                return Err(refusal());
            };
            if !lower_hex_generation(remote)
                || !canonical_utc(observed_at)
                || !full_nonblank_commit(observed_local)
            {
                return Err(refusal());
            }
            if matches!(state.remote_relation, RemoteRelation::Equivalent)
                && state.last_successful_sync_at.as_deref() != Some(observed_at)
            {
                return Err(refusal());
            }
            if state.pending_mutations.operation_ids.is_empty()
                && observed_local != state.local_generation
            {
                return Err(refusal());
            }
            Ok(())
        }
    }
}

/// Validates one persisted observation and returns its six-state local freshness envelope.
pub fn classify(state: ObservationState) -> Result<FreshnessEnvelope, FreshnessError> {
    validate(&state)?;
    let pending = !state.pending_mutations.operation_ids.is_empty();
    let freshness = match (pending, state.remote_relation) {
        (false, RemoteRelation::Equivalent) => Freshness::SynchronizedAsOf,
        (false, RemoteRelation::Ahead) => Freshness::Stale,
        (false, RemoteRelation::Unknown) => Freshness::Unknown,
        (true, RemoteRelation::Equivalent) => Freshness::Unpublished,
        (true, RemoteRelation::Ahead) => Freshness::StaleWithUnpublished,
        (true, RemoteRelation::Unknown) => Freshness::UnknownWithUnpublished,
    };
    Ok(FreshnessEnvelope {
        last_successful_sync_at: state.last_successful_sync_at,
        local_generation: state.local_generation,
        remote_generation: state.remote_generation,
        remote_observed_at: state.remote_observed_at,
        pending_mutations: PendingMutationEnvelope {
            count: state.pending_mutations.operation_ids.len(),
            operation_ids: state.pending_mutations.operation_ids,
        },
        freshness,
    })
}
