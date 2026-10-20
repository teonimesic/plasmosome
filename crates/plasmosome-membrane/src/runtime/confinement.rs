/// The operations a helper's sandbox must deny before it counts as confined, in the order
/// `observe` asks about them.
pub const REQUIRED_DENIALS: [&str; 5] = [
    "process-fork",
    "network-outbound",
    "network-inbound",
    "network-bind",
    "file-write-create",
];

/// What the supervisor saw of one process's Darwin sandbox, asked from outside that process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confinement {
    /// The process was confined in the properties `observe` checks.
    Confined(ConfinementEvidence),
    /// The kernel answered, and the answers fall short of confinement.
    Unconfined {
        /// Whether the kernel reported the process sandboxed.
        sandboxed: bool,
        /// The operations of `REQUIRED_DENIALS` the kernel did not report denied.
        allowed: Vec<&'static str>,
    },
    /// The observation could not be made or trusted.
    Unverifiable {
        /// Why.
        reason: Unverifiable,
    },
}

/// Why an observation could not be made or trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unverifiable {
    /// The supervisor itself is sandboxed.
    SupervisorSandboxed,
    /// A sandbox query failed.
    CheckFailed {
        /// The operation asked about, or `None` for the sandboxed query.
        operation: Option<&'static str>,
        /// The errno the query set.
        errno: i32,
    },
    /// The process could not be read as running before or after the sandbox queries.
    ProcessUnreadable {
        /// The errno the read set.
        errno: i32,
    },
    /// The pid named a different process after the sandbox queries than before them.
    ProcessReplaced,
    /// This platform has no supervisor-side sandbox observation.
    UnsupportedPlatform,
}

/// What a `Confined` observation found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfinementEvidence {
    pid: libc::pid_t,
    denied: Vec<&'static str>,
    binding: PolicyBinding,
}

impl ConfinementEvidence {
    /// The pid that was observed.
    pub fn pid(&self) -> libc::pid_t {
        todo!()
    }

    /// The operations the kernel reported denied.
    pub fn denied(&self) -> &[&'static str] {
        todo!()
    }

    /// How the observation relates to the verified host policy.
    pub fn binding(&self) -> PolicyBinding {
        todo!()
    }
}

/// How a confinement observation relates to the verified host policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyBinding {
    /// Only properties of the installed sandbox were observed.
    PropertiesObserved,
}

/// Observes the Darwin sandbox of the process `pid` from this process.
pub fn observe(pid: libc::pid_t) -> Confinement {
    let _ = pid;
    todo!()
}

/// One answer from `sandbox_check`: the value it returned, or the errno it set when it returned
/// -1.
pub(crate) type Answer = Result<libc::c_int, i32>;

/// One read of the observed process's start time, as seconds and microseconds, or the errno the
/// read set.
pub(crate) type Started = Result<(u64, u64), i32>;

/// Everything one observation read, in the order it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Readings {
    pub(crate) own: Answer,
    pub(crate) before: Started,
    pub(crate) sandboxed: Answer,
    pub(crate) operations: Vec<(&'static str, Answer)>,
    pub(crate) after: Started,
}

pub(crate) fn judge(pid: libc::pid_t, readings: &Readings) -> Confinement {
    let _ = (pid, readings);
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const PID: libc::pid_t = 4242;
    const STARTED: (u64, u64) = (1_792_518_707, 399_673);

    fn answering(answer: Answer) -> Vec<(&'static str, Answer)> {
        REQUIRED_DENIALS
            .iter()
            .map(|operation| (*operation, answer))
            .collect()
    }

    fn confined() -> Readings {
        Readings {
            own: Ok(0),
            before: Ok(STARTED),
            sandboxed: Ok(1),
            operations: answering(Ok(1)),
            after: Ok(STARTED),
        }
    }

    fn with_operation(operation: &str, answer: Answer) -> Readings {
        let mut readings = confined();
        for (name, value) in &mut readings.operations {
            if *name == operation {
                *value = answer;
            }
        }
        readings
    }

    fn unverifiable(reason: Unverifiable) -> Confinement {
        Confinement::Unverifiable { reason }
    }

    #[test]
    fn a_sandboxed_process_denying_every_required_operation_is_confined() {
        let Confinement::Confined(evidence) = judge(PID, &confined()) else {
            panic!("every rule holds, so the process is confined");
        };
        assert_eq!(evidence.pid(), PID);
        assert_eq!(evidence.denied(), REQUIRED_DENIALS);
        assert_eq!(evidence.binding(), PolicyBinding::PropertiesObserved);
    }

    #[test]
    fn a_sandbox_that_denies_nothing_is_unconfined() {
        let readings = Readings {
            operations: answering(Ok(0)),
            ..confined()
        };
        assert_eq!(
            judge(PID, &readings),
            Confinement::Unconfined {
                sandboxed: true,
                allowed: REQUIRED_DENIALS.to_vec(),
            },
            "allow-default answers sandboxed and allows all five"
        );
    }

    #[test]
    fn each_required_operation_left_allowed_is_unconfined_and_named() {
        for operation in REQUIRED_DENIALS {
            assert_eq!(
                judge(PID, &with_operation(operation, Ok(0))),
                Confinement::Unconfined {
                    sandboxed: true,
                    allowed: vec![operation],
                },
                "{operation} answered allowed"
            );
        }
    }

    #[test]
    fn an_answer_other_than_one_is_not_a_denial() {
        for value in [2, -2, libc::c_int::MAX, libc::c_int::MIN] {
            assert_eq!(
                judge(PID, &with_operation("network-bind", Ok(value))),
                Confinement::Unconfined {
                    sandboxed: true,
                    allowed: vec!["network-bind"],
                },
                "network-bind answered {value}"
            );
        }
    }

    #[test]
    fn a_required_operation_that_was_not_asked_is_not_a_denial() {
        let mut readings = confined();
        readings
            .operations
            .retain(|(operation, _)| *operation != "network-inbound");
        assert_eq!(
            judge(PID, &readings),
            Confinement::Unconfined {
                sandboxed: true,
                allowed: vec!["network-inbound"],
            }
        );
        readings.operations.clear();
        assert_eq!(
            judge(PID, &readings),
            Confinement::Unconfined {
                sandboxed: true,
                allowed: REQUIRED_DENIALS.to_vec(),
            }
        );
    }

    #[test]
    fn a_process_the_kernel_does_not_report_sandboxed_is_unconfined() {
        for value in [0, 2, -2] {
            assert_eq!(
                judge(
                    PID,
                    &Readings {
                        sandboxed: Ok(value),
                        ..confined()
                    }
                ),
                Confinement::Unconfined {
                    sandboxed: false,
                    allowed: Vec::new(),
                },
                "the sandboxed query answered {value}, though every operation answered denied"
            );
        }
        let unsandboxed = Readings {
            sandboxed: Ok(0),
            operations: answering(Ok(0)),
            ..confined()
        };
        assert_eq!(
            judge(PID, &unsandboxed),
            Confinement::Unconfined {
                sandboxed: false,
                allowed: REQUIRED_DENIALS.to_vec(),
            }
        );
    }

    #[test]
    fn a_failed_query_is_unverifiable_and_named_never_a_denial() {
        assert_eq!(
            judge(
                PID,
                &Readings {
                    sandboxed: Err(libc::EINVAL),
                    ..confined()
                }
            ),
            unverifiable(Unverifiable::CheckFailed {
                operation: None,
                errno: libc::EINVAL,
            })
        );
        assert_eq!(
            judge(
                PID,
                &Readings {
                    own: Err(libc::EPERM),
                    ..confined()
                }
            ),
            unverifiable(Unverifiable::CheckFailed {
                operation: None,
                errno: libc::EPERM,
            })
        );
        for operation in REQUIRED_DENIALS {
            assert_eq!(
                judge(PID, &with_operation(operation, Err(libc::EINVAL))),
                unverifiable(Unverifiable::CheckFailed {
                    operation: Some(operation),
                    errno: libc::EINVAL,
                }),
                "{operation} failed"
            );
        }
    }

    #[test]
    fn a_failed_query_outranks_an_allowed_answer() {
        let mut readings = with_operation("process-fork", Ok(0));
        readings.operations[4].1 = Err(libc::EINVAL);
        readings.sandboxed = Ok(0);
        assert_eq!(
            judge(PID, &readings),
            unverifiable(Unverifiable::CheckFailed {
                operation: Some("file-write-create"),
                errno: libc::EINVAL,
            })
        );
    }

    #[test]
    fn a_sandboxed_supervisor_is_not_trusted() {
        for value in [1, 2, -2] {
            assert_eq!(
                judge(
                    PID,
                    &Readings {
                        own: Ok(value),
                        ..confined()
                    }
                ),
                unverifiable(Unverifiable::SupervisorSandboxed),
                "the supervisor's own sandboxed query answered {value}"
            );
        }
    }

    #[test]
    fn a_process_not_read_running_on_both_sides_of_the_queries_is_unverifiable() {
        for errno in [libc::ESRCH, libc::EPERM] {
            for readings in [
                Readings {
                    before: Err(errno),
                    ..confined()
                },
                Readings {
                    after: Err(errno),
                    ..confined()
                },
            ] {
                assert_eq!(
                    judge(PID, &readings),
                    unverifiable(Unverifiable::ProcessUnreadable { errno }),
                    "a dead pid answers 1 to every sandbox query, so those answers are not used: {readings:?}"
                );
            }
        }
    }

    #[test]
    fn a_pid_naming_a_later_process_after_the_queries_is_unverifiable() {
        for after in [
            (STARTED.0, STARTED.1 + 1),
            (STARTED.0 + 1, STARTED.1),
            (STARTED.0 - 1, STARTED.1),
        ] {
            assert_eq!(
                judge(
                    PID,
                    &Readings {
                        after: Ok(after),
                        ..confined()
                    }
                ),
                unverifiable(Unverifiable::ProcessReplaced),
                "the process read after the queries started at {after:?}"
            );
        }
    }

    fn answer(usual: libc::c_int) -> impl Strategy<Value = Answer> {
        prop_oneof![
            8 => Just(Ok(usual)),
            1 => (-3..4).prop_map(Ok),
            1 => (1..120).prop_map(Err),
        ]
    }

    fn started() -> impl Strategy<Value = Started> {
        prop_oneof![
            8 => Just(Ok(STARTED)),
            1 => (0..3u64, 0..3u64).prop_map(Ok),
            1 => (1..120).prop_map(Err),
        ]
    }

    fn readings() -> impl Strategy<Value = Readings> {
        (
            answer(0),
            started(),
            answer(1),
            prop::collection::vec(prop::option::weighted(0.95, answer(1)), 5),
            started(),
        )
            .prop_map(|(own, before, sandboxed, answers, after)| Readings {
                own,
                before,
                sandboxed,
                operations: REQUIRED_DENIALS
                    .iter()
                    .zip(answers)
                    .filter_map(|(operation, answer)| answer.map(|answer| (*operation, answer)))
                    .collect(),
                after,
            })
    }

    proptest! {
        #[test]
        fn confined_only_when_every_rule_holds(readings in readings()) {
            let every_rule_holds = readings.own == Ok(0)
                && readings.before.is_ok()
                && readings.before == readings.after
                && readings.sandboxed == Ok(1)
                && readings.operations.len() == REQUIRED_DENIALS.len()
                && readings.operations.iter().all(|(_, answer)| *answer == Ok(1));
            prop_assert_eq!(
                matches!(judge(PID, &readings), Confinement::Confined(_)),
                every_rule_holds,
                "{:?}",
                readings
            );
        }
    }
}
