use serde::Serialize;

use crate::state::CellStatus;

/// What the controller's `flock(LOCK_EX | LOCK_NB)` on a fresh descriptor found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockFact {
    /// Another holder has the lock: a live supervisor.
    Held,
    /// The controller got the lock, and holds it until the outcome is carried out. A missing
    /// `supervisor.lock` that the controller created exclusively and locked is free.
    Free,
}

/// What the cell's `launched` marker says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerFact {
    /// No marker: no runtime was launched, or it was cleanly ended.
    Absent,
    /// A marker whose boot session is not this boot's, whether or not it records a clean stop.
    EarlierBoot,
    /// A marker from this boot that records a clean stop.
    ThisBootStopped,
    /// A marker from this boot with no clean stop.
    ThisBootRunning,
    /// A marker that exists but cannot be read or parsed.
    Unreadable,
}

/// What the cell's journal holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalFact {
    /// A journal spec 008 quarantines.
    Corrupt,
    /// Nothing settled and nothing pending. A missing journal is empty.
    Empty,
    /// A settled attachment or a pending operation. `external_obligation` is true when the
    /// journal holds an outstanding external obligation: spec 008's published delayed effect or
    /// external assertion, whether it is settled or a pending change retires it. Only an
    /// operator's Force discharges it, and retiring never does.
    Holdings { external_obligation: bool },
}

/// Why a cell is set aside. Serializes in snake case, such as `supervisor_lost`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetAsideReason {
    /// The supervisor gave no answer by the cap.
    Unresponsive,
    /// The supervisor still reported `germinating` at the cap.
    Germinating,
    /// The cell would retire, but its journal holds an outstanding external obligation.
    ExternalObligations,
    /// The lock is free, but the marker is from this boot with no clean stop, or unreadable.
    SupervisorLost,
    /// No marker, but the journal holds a settled attachment or a pending operation.
    HoldingsWithoutSupervisor,
}

/// What the controller does with one cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckOutcome {
    /// Query the supervisor within the cap and pass each result to [`after_query`].
    Query,
    /// Quarantine the cell under spec 008, reporting its lock state and marker.
    Quarantine,
    /// Retire the cell, writing every settled holding and pending operation to the session log
    /// as residue.
    RetireWithResidue,
    /// Set the cell aside for this reason.
    SetAside(SetAsideReason),
    /// Retire the cell with a session log line.
    RetireEmpty,
}

/// Decides one cell from its lock, marker and journal, by the first row of spec 023 §8's table
/// that matches. The caller gathers the facts and carries out the outcome.
pub const fn decide(lock: LockFact, marker: MarkerFact, journal: JournalFact) -> CheckOutcome {
    match (lock, marker, journal) {
        (LockFact::Held, _, _) => CheckOutcome::Query,
        (LockFact::Free, _, JournalFact::Corrupt) => CheckOutcome::Quarantine,
        (
            LockFact::Free,
            MarkerFact::EarlierBoot | MarkerFact::ThisBootStopped,
            JournalFact::Empty
            | JournalFact::Holdings {
                external_obligation: false,
            },
        ) => CheckOutcome::RetireWithResidue,
        (
            LockFact::Free,
            MarkerFact::EarlierBoot | MarkerFact::ThisBootStopped,
            JournalFact::Holdings {
                external_obligation: true,
            },
        ) => CheckOutcome::SetAside(SetAsideReason::ExternalObligations),
        (LockFact::Free, MarkerFact::ThisBootRunning | MarkerFact::Unreadable, _) => {
            CheckOutcome::SetAside(SetAsideReason::SupervisorLost)
        }
        (LockFact::Free, MarkerFact::Absent, JournalFact::Empty) => CheckOutcome::RetireEmpty,
        (LockFact::Free, MarkerFact::Absent, JournalFact::Holdings { .. }) => {
            CheckOutcome::SetAside(SetAsideReason::HoldingsWithoutSupervisor)
        }
    }
}

/// One query to a held lock's supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryResult {
    /// The supervisor answered with this lifecycle, within the answer cap.
    Answered { lifecycle: CellStatus },
    /// No answer within the cap. A caller maps a malformed answer here too.
    NoAnswer,
}

/// What the controller does after one query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryOutcome {
    /// The supervisor is live: attach it to the cell. This does not promise the cell is served:
    /// spec 008's journal replay can still quarantine it.
    Adopt,
    /// Query again before the cap.
    KeepWaiting,
    /// Set the cell aside as `unresponsive` or `germinating`.
    SetAside(SetAsideReason),
}

/// Decides what one query's result means. `cap_reached` is true once the answer cap has passed.
/// A supervisor that answers any lifecycle but `germinating` is adopted, `dead` included.
///
/// Pass only an answer that arrived within the cap. Once the cap has passed, send no new query:
/// call this once more with the last result that arrived within the cap, or `NoAnswer` if there
/// was none, and `cap_reached` true. A germinating cell whose cap passes between two queries is
/// then set aside as `germinating`, not `unresponsive`.
pub const fn after_query(result: QueryResult, cap_reached: bool) -> QueryOutcome {
    match (result, cap_reached) {
        (
            QueryResult::Answered {
                lifecycle: CellStatus::Ready | CellStatus::Draining | CellStatus::Dead,
            },
            _,
        ) => QueryOutcome::Adopt,
        (
            QueryResult::Answered {
                lifecycle: CellStatus::Germinating,
            }
            | QueryResult::NoAnswer,
            false,
        ) => QueryOutcome::KeepWaiting,
        (
            QueryResult::Answered {
                lifecycle: CellStatus::Germinating,
            },
            true,
        ) => QueryOutcome::SetAside(SetAsideReason::Germinating),
        (QueryResult::NoAnswer, true) => QueryOutcome::SetAside(SetAsideReason::Unresponsive),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const _: CheckOutcome = decide(LockFact::Free, MarkerFact::Absent, JournalFact::Empty);
    const _: QueryOutcome = after_query(QueryResult::NoAnswer, true);

    const HELD: LockFact = LockFact::Held;
    const FREE: LockFact = LockFact::Free;

    const ABSENT: MarkerFact = MarkerFact::Absent;
    const EARLIER: MarkerFact = MarkerFact::EarlierBoot;
    const STOPPED: MarkerFact = MarkerFact::ThisBootStopped;
    const RUNNING: MarkerFact = MarkerFact::ThisBootRunning;
    const UNREADABLE: MarkerFact = MarkerFact::Unreadable;

    const CORRUPT: JournalFact = JournalFact::Corrupt;
    const EMPTY: JournalFact = JournalFact::Empty;
    const HOLDINGS: JournalFact = JournalFact::Holdings {
        external_obligation: false,
    };
    const OBLIGATED: JournalFact = JournalFact::Holdings {
        external_obligation: true,
    };

    const QUERY: CheckOutcome = CheckOutcome::Query;
    const QUARANTINE: CheckOutcome = CheckOutcome::Quarantine;
    const RETIRE_RESIDUE: CheckOutcome = CheckOutcome::RetireWithResidue;
    const RETIRE_EMPTY: CheckOutcome = CheckOutcome::RetireEmpty;
    const OBLIGATIONS: CheckOutcome = CheckOutcome::SetAside(SetAsideReason::ExternalObligations);
    const LOST: CheckOutcome = CheckOutcome::SetAside(SetAsideReason::SupervisorLost);
    const NO_SUPERVISOR: CheckOutcome =
        CheckOutcome::SetAside(SetAsideReason::HoldingsWithoutSupervisor);

    const TABLE: [(LockFact, MarkerFact, JournalFact, CheckOutcome); 40] = [
        (HELD, ABSENT, CORRUPT, QUERY),
        (HELD, ABSENT, EMPTY, QUERY),
        (HELD, ABSENT, HOLDINGS, QUERY),
        (HELD, ABSENT, OBLIGATED, QUERY),
        (HELD, EARLIER, CORRUPT, QUERY),
        (HELD, EARLIER, EMPTY, QUERY),
        (HELD, EARLIER, HOLDINGS, QUERY),
        (HELD, EARLIER, OBLIGATED, QUERY),
        (HELD, STOPPED, CORRUPT, QUERY),
        (HELD, STOPPED, EMPTY, QUERY),
        (HELD, STOPPED, HOLDINGS, QUERY),
        (HELD, STOPPED, OBLIGATED, QUERY),
        (HELD, RUNNING, CORRUPT, QUERY),
        (HELD, RUNNING, EMPTY, QUERY),
        (HELD, RUNNING, HOLDINGS, QUERY),
        (HELD, RUNNING, OBLIGATED, QUERY),
        (HELD, UNREADABLE, CORRUPT, QUERY),
        (HELD, UNREADABLE, EMPTY, QUERY),
        (HELD, UNREADABLE, HOLDINGS, QUERY),
        (HELD, UNREADABLE, OBLIGATED, QUERY),
        (FREE, ABSENT, CORRUPT, QUARANTINE),
        (FREE, ABSENT, EMPTY, RETIRE_EMPTY),
        (FREE, ABSENT, HOLDINGS, NO_SUPERVISOR),
        (FREE, ABSENT, OBLIGATED, NO_SUPERVISOR),
        (FREE, EARLIER, CORRUPT, QUARANTINE),
        (FREE, EARLIER, EMPTY, RETIRE_RESIDUE),
        (FREE, EARLIER, HOLDINGS, RETIRE_RESIDUE),
        (FREE, EARLIER, OBLIGATED, OBLIGATIONS),
        (FREE, STOPPED, CORRUPT, QUARANTINE),
        (FREE, STOPPED, EMPTY, RETIRE_RESIDUE),
        (FREE, STOPPED, HOLDINGS, RETIRE_RESIDUE),
        (FREE, STOPPED, OBLIGATED, OBLIGATIONS),
        (FREE, RUNNING, CORRUPT, QUARANTINE),
        (FREE, RUNNING, EMPTY, LOST),
        (FREE, RUNNING, HOLDINGS, LOST),
        (FREE, RUNNING, OBLIGATED, LOST),
        (FREE, UNREADABLE, CORRUPT, QUARANTINE),
        (FREE, UNREADABLE, EMPTY, LOST),
        (FREE, UNREADABLE, HOLDINGS, LOST),
        (FREE, UNREADABLE, OBLIGATED, LOST),
    ];

    fn position(lock: LockFact, marker: MarkerFact, journal: JournalFact) -> usize {
        let lock = match lock {
            LockFact::Held => 0,
            LockFact::Free => 1,
        };
        let marker = match marker {
            MarkerFact::Absent => 0,
            MarkerFact::EarlierBoot => 1,
            MarkerFact::ThisBootStopped => 2,
            MarkerFact::ThisBootRunning => 3,
            MarkerFact::Unreadable => 4,
        };
        let journal = match journal {
            JournalFact::Corrupt => 0,
            JournalFact::Empty => 1,
            JournalFact::Holdings {
                external_obligation: false,
            } => 2,
            JournalFact::Holdings {
                external_obligation: true,
            } => 3,
        };
        lock * 20 + marker * 4 + journal
    }

    fn is_retirement(outcome: CheckOutcome) -> bool {
        matches!(
            outcome,
            CheckOutcome::RetireWithResidue | CheckOutcome::RetireEmpty
        )
    }

    #[test]
    fn every_combination_has_one_row() {
        let positions: Vec<usize> = TABLE
            .iter()
            .map(|&(lock, marker, journal, _)| position(lock, marker, journal))
            .collect();
        assert_eq!(
            positions,
            (0..40).collect::<Vec<_>>(),
            "the table names every lock, marker and journal combination once, in order"
        );
        let wrong: Vec<_> = TABLE
            .iter()
            .map(|&(lock, marker, journal, table)| {
                (
                    (lock, marker, journal),
                    table,
                    decide(lock, marker, journal),
                )
            })
            .filter(|(_, table, decided)| table != decided)
            .collect();
        assert!(
            wrong.is_empty(),
            "rows as ((lock, marker, journal), table, decide): {wrong:#?}"
        );
    }

    #[test]
    fn a_clean_stop_from_this_boot_retires() {
        assert_eq!(decide(FREE, STOPPED, HOLDINGS), RETIRE_RESIDUE);
        assert_eq!(decide(FREE, STOPPED, EMPTY), RETIRE_RESIDUE);
    }

    #[test]
    fn this_boot_without_a_clean_stop_is_supervisor_lost() {
        assert_eq!(decide(FREE, RUNNING, HOLDINGS), LOST);
        assert_eq!(decide(FREE, RUNNING, EMPTY), LOST);
    }

    #[test]
    fn external_obligations_are_never_retired() {
        assert_eq!(decide(FREE, EARLIER, OBLIGATED), OBLIGATIONS);
        assert_eq!(decide(FREE, STOPPED, OBLIGATED), OBLIGATIONS);
        let retired: Vec<_> = TABLE
            .iter()
            .filter(|&&(lock, marker, journal, _)| {
                journal == OBLIGATED && is_retirement(decide(lock, marker, journal))
            })
            .collect();
        assert!(
            retired.is_empty(),
            "retired with an obligation: {retired:?}"
        );
    }

    #[test]
    fn absent_marker_with_holdings_is_set_aside() {
        assert_eq!(decide(FREE, ABSENT, HOLDINGS), NO_SUPERVISOR);
        assert_eq!(decide(FREE, ABSENT, OBLIGATED), NO_SUPERVISOR);
        assert_eq!(decide(FREE, ABSENT, EMPTY), RETIRE_EMPTY);
    }

    fn answered(lifecycle: CellStatus) -> QueryResult {
        QueryResult::Answered { lifecycle }
    }

    #[test]
    fn an_answered_lifecycle_other_than_germinating_is_adopted() {
        for lifecycle in [CellStatus::Ready, CellStatus::Draining, CellStatus::Dead] {
            for cap_reached in [false, true] {
                assert_eq!(
                    after_query(answered(lifecycle), cap_reached),
                    QueryOutcome::Adopt,
                    "{lifecycle:?}, cap reached {cap_reached}"
                );
            }
        }
    }

    #[test]
    fn germinating_keeps_waiting_until_the_cap() {
        assert_eq!(
            after_query(answered(CellStatus::Germinating), false),
            QueryOutcome::KeepWaiting
        );
        assert_eq!(
            after_query(answered(CellStatus::Germinating), true),
            QueryOutcome::SetAside(SetAsideReason::Germinating)
        );
    }

    #[test]
    fn no_answer_keeps_waiting_until_the_cap() {
        assert_eq!(
            after_query(QueryResult::NoAnswer, false),
            QueryOutcome::KeepWaiting
        );
        assert_eq!(
            after_query(QueryResult::NoAnswer, true),
            QueryOutcome::SetAside(SetAsideReason::Unresponsive)
        );
    }

    #[test]
    fn set_aside_reasons_carry_their_wire_names() {
        let names: Vec<serde_json::Value> = [
            SetAsideReason::Unresponsive,
            SetAsideReason::Germinating,
            SetAsideReason::ExternalObligations,
            SetAsideReason::SupervisorLost,
            SetAsideReason::HoldingsWithoutSupervisor,
        ]
        .iter()
        .map(|reason| serde_json::to_value(reason).expect("a reason serializes"))
        .collect();
        assert_eq!(
            names,
            [
                "unresponsive",
                "germinating",
                "external_obligations",
                "supervisor_lost",
                "holdings_without_supervisor",
            ]
        );
    }
}
