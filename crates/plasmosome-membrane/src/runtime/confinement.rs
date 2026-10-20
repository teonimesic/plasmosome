/// The operations a helper's sandbox must deny for `observe` to report it confined, in the order
/// `observe` asks about them.
///
/// Each is asked with no filter argument, so a denial is the sandbox's answer for the operation in
/// general. A policy can still allow it for some arguments: the qualified host policy allows
/// outbound connections to Unix sockets and still answers `network-outbound` denied.
pub const REQUIRED_DENIALS: [&str; 5] = [
    "process-fork",
    "network-outbound",
    "network-inbound",
    "network-bind",
    "file-write-create",
];

/// What the supervisor saw of one process's Darwin sandbox, asked from outside that process.
///
/// Only `Confined` lets readiness go ahead. Treat both other values as a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confinement {
    /// The process was read running on both sides of the queries, the kernel reported it
    /// sandboxed, and it reported every operation in `REQUIRED_DENIALS` denied.
    Confined(ConfinementEvidence),
    /// The kernel answered every query, and the answers fall short: the process is not
    /// sandboxed, or an operation in `REQUIRED_DENIALS` is not denied. A sandbox that allows
    /// everything still reports itself sandboxed.
    Unconfined {
        /// Whether the kernel reported the process sandboxed.
        sandboxed: bool,
        /// The operations of `REQUIRED_DENIALS` the kernel did not report denied, in that order.
        allowed: Vec<&'static str>,
    },
    /// The observation could not be made, or its answers cannot be trusted.
    Unverifiable {
        /// Why.
        reason: Unverifiable,
    },
}

/// Why an observation could not be made or trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unverifiable {
    /// This process is sandboxed itself, so its view of another process is not trusted.
    SupervisorSandboxed,
    /// `sandbox_check` returned -1 instead of an answer.
    CheckFailed {
        /// The operation asked about, or `None` for a query of whether a process is sandboxed,
        /// this one's or the observed one's.
        operation: Option<&'static str>,
        /// The errno it set.
        errno: i32,
    },
    /// `proc_pidinfo` could not read the process before or after the sandbox queries. The
    /// kernel answers every sandbox query about an absent pid or a zombie as denied, so those
    /// answers count only when the process is read running on both sides of them.
    ProcessUnreadable {
        /// The errno it set: `ESRCH` for an absent pid or a zombie, `EPERM` for another user's
        /// process.
        errno: i32,
    },
    /// The process read after the sandbox queries started at a different time from the one read
    /// before them, so the pid was reused while it was observed.
    ProcessReplaced,
    /// This platform has no supervisor-side sandbox observation. Every pid off Darwin gives
    /// this.
    UnsupportedPlatform,
}

/// What a `Confined` observation found. Only `observe` makes one. It does not name the installed
/// policy; see `PolicyBinding`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfinementEvidence {
    pid: libc::pid_t,
    denied: Vec<&'static str>,
    binding: PolicyBinding,
}

impl ConfinementEvidence {
    /// The pid that was observed.
    pub fn pid(&self) -> libc::pid_t {
        self.pid
    }

    /// The operations the kernel reported denied: all of `REQUIRED_DENIALS`.
    pub fn denied(&self) -> &[&'static str] {
        &self.denied
    }

    /// How the observation relates to the verified host policy: always `PropertiesObserved`.
    pub fn binding(&self) -> PolicyBinding {
        self.binding
    }
}

/// How a confinement observation relates to the verified host policy the helper applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyBinding {
    /// Only properties of the installed sandbox were observed. Darwin has no call that returns a
    /// process's installed profile, so this does not show that the verified policy is the one
    /// installed: a bare `(deny default)` profile is observed the same way. Spec 001 §4.2 asks
    /// for evidence binding the verified policy to the process. This is less than that, and owner
    /// gate O-3 on native task plasmosome-qep decides whether it is enough.
    PropertiesObserved,
}

/// Observes, from this process, the Darwin sandbox of the running process `pid`.
///
/// Pass the pid of a process this process keeps from being reaped, such as the helper child it
/// owns. `observe` asks `sandbox_check` whether this process is sandboxed, whether `pid` is, and
/// whether each operation in `REQUIRED_DENIALS` is denied. It reads the start time of `pid` with
/// `proc_pidinfo` before and after those queries. It changes nothing.
///
/// Only `Confined` lets readiness go ahead. It shows properties of the sandbox, not the policy:
/// nothing about which profile is installed, about arguments a policy allows for a denied
/// operation, about descriptors the process already holds, or about the images it loaded.
/// `sandbox_check` is declared in no SDK header. If a later macOS makes it fail, the result is
/// `Unverifiable`; the crate's macOS tests check the answers it gives today.
#[cfg(target_os = "macos")]
pub fn observe(pid: libc::pid_t) -> Confinement {
    judge(pid, &darwin::read(pid))
}

/// Off Darwin there is no supervisor-side sandbox observation, so every `pid` gives
/// `Unverifiable` with `UnsupportedPlatform`.
#[cfg(not(target_os = "macos"))]
pub fn observe(_pid: libc::pid_t) -> Confinement {
    Confinement::Unverifiable {
        reason: Unverifiable::UnsupportedPlatform,
    }
}

/// One answer from `sandbox_check`: the value it returned, or the errno it set when it returned
/// -1.
#[cfg(any(target_os = "macos", test))]
pub(crate) type Answer = Result<libc::c_int, i32>;

/// One read of the observed process's start time, as seconds and microseconds, or the errno the
/// read set.
#[cfg(any(target_os = "macos", test))]
pub(crate) type Started = Result<(u64, u64), i32>;

/// Everything one observation read, in the order it was read.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Readings {
    pub(crate) own: Answer,
    pub(crate) before: Started,
    pub(crate) sandboxed: Answer,
    pub(crate) operations: Vec<(&'static str, Answer)>,
    pub(crate) after: Started,
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn judge(pid: libc::pid_t, readings: &Readings) -> Confinement {
    if let Err(reason) = trusted(readings) {
        return Confinement::Unverifiable { reason };
    }
    let sandboxed = readings.sandboxed == Ok(1);
    let allowed: Vec<&'static str> = REQUIRED_DENIALS
        .into_iter()
        .filter(|required| {
            !readings
                .operations
                .iter()
                .any(|(operation, answer)| operation == required && *answer == Ok(1))
        })
        .collect();
    if !sandboxed || !allowed.is_empty() {
        return Confinement::Unconfined { sandboxed, allowed };
    }
    Confinement::Confined(ConfinementEvidence {
        pid,
        denied: REQUIRED_DENIALS.to_vec(),
        binding: PolicyBinding::PropertiesObserved,
    })
}

#[cfg(any(target_os = "macos", test))]
fn trusted(readings: &Readings) -> Result<(), Unverifiable> {
    match readings.own {
        Ok(0) => {}
        Ok(_) => return Err(Unverifiable::SupervisorSandboxed),
        Err(errno) => {
            return Err(Unverifiable::CheckFailed {
                operation: None,
                errno,
            });
        }
    }
    let before = readings
        .before
        .map_err(|errno| Unverifiable::ProcessUnreadable { errno })?;
    let after = readings
        .after
        .map_err(|errno| Unverifiable::ProcessUnreadable { errno })?;
    if before != after {
        return Err(Unverifiable::ProcessReplaced);
    }
    if let Err(errno) = readings.sandboxed {
        return Err(Unverifiable::CheckFailed {
            operation: None,
            errno,
        });
    }
    match readings
        .operations
        .iter()
        .find_map(|(operation, answer)| answer.err().map(|errno| (*operation, errno)))
    {
        Some((operation, errno)) => Err(Unverifiable::CheckFailed {
            operation: Some(operation),
            errno,
        }),
        None => Ok(()),
    }
}

#[cfg(target_os = "macos")]
mod darwin {
    use super::{Answer, REQUIRED_DENIALS, Readings, Started};
    use std::ffi::{c_char, c_int};

    const INFO_SIZE: c_int = size_of::<libc::proc_bsdinfo>() as c_int;

    #[allow(unsafe_code)]
    unsafe extern "C" {
        /// libsystem_sandbox's query of a process's sandbox. libSystem re-exports it, and no SDK
        /// header declares it. With a null `operation` it returns 1 when `pid` is sandboxed and 0
        /// when not. With an operation name it returns 1 when the sandbox denies that operation and
        /// 0 when it allows it. It returns -1 and sets errno when it cannot answer, as for an
        /// unknown name or an operation that needs a filter argument. `filter_type` 0 names no
        /// filter, and then it reads no variadic argument. It reads the NUL-terminated
        /// `operation` during the call only and keeps no pointer. Measured on macOS 26.6.2, it
        /// returns 1 to every query about an absent pid or a zombie, unknown names included.
        fn sandbox_check(
            pid: libc::pid_t,
            operation: *const c_char,
            filter_type: c_int,
            ...
        ) -> c_int;
    }

    pub(super) fn read(pid: libc::pid_t) -> Readings {
        let own = check(std::process::id().cast_signed(), None);
        let before = started(pid);
        let sandboxed = check(pid, None);
        let operations = REQUIRED_DENIALS
            .into_iter()
            .map(|operation| (operation, check(pid, Some(operation))))
            .collect();
        let after = started(pid);
        Readings {
            own,
            before,
            sandboxed,
            operations,
            after,
        }
    }

    #[allow(unsafe_code)]
    fn check(pid: libc::pid_t, operation: Option<&str>) -> Answer {
        let name = operation.map(|operation| [operation.as_bytes(), b"\0"].concat());
        let pointer = name
            .as_ref()
            .map_or(std::ptr::null(), |name| name.as_ptr().cast::<c_char>());
        // SAFETY: `pointer` is null or points into `name`, a buffer ending in NUL that lives until
        // this function returns, and sandbox_check reads it during the call only. Filter type 0
        // names no filter, so no variadic argument is passed or read.
        let value = unsafe { sandbox_check(pid, pointer, 0) };
        if value == -1 {
            Err(last_errno())
        } else {
            Ok(value)
        }
    }

    #[allow(unsafe_code)]
    fn started(pid: libc::pid_t) -> Started {
        // SAFETY: proc_bsdinfo is a plain C struct, for which all zeroes is a valid value.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a local proc_bsdinfo and INFO_SIZE is its exact size, so proc_pidinfo
        // writes at most that many bytes into it, and it keeps no pointer.
        let written = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&raw mut info).cast(),
                INFO_SIZE,
            )
        };
        if written == INFO_SIZE {
            Ok((info.pbi_start_tvsec, info.pbi_start_tvusec))
        } else {
            Err(last_errno())
        }
    }

    fn last_errno() -> i32 {
        std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::EIO)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_query_the_kernel_cannot_answer_reads_as_its_errno_never_a_value() {
            let own = std::process::id().cast_signed();
            for operation in ["process-exec", "no-such-operation"] {
                assert_eq!(
                    check(own, Some(operation)),
                    Err(libc::EINVAL),
                    "{operation} needs a filter or is unknown, so sandbox_check returns -1"
                );
            }
        }
    }
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

    fn unconfined(sandboxed: bool, allowed: &[&'static str]) -> Confinement {
        Confinement::Unconfined {
            sandboxed,
            allowed: allowed.to_vec(),
        }
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
            unconfined(true, &REQUIRED_DENIALS),
            "allow-default answers sandboxed and allows all five"
        );
    }

    #[test]
    fn each_required_operation_left_allowed_is_unconfined_and_named() {
        for operation in REQUIRED_DENIALS {
            assert_eq!(
                judge(PID, &with_operation(operation, Ok(0))),
                unconfined(true, &[operation]),
                "{operation} answered allowed"
            );
        }
    }

    #[test]
    fn an_answer_other_than_one_is_not_a_denial() {
        for value in [2, -2, libc::c_int::MAX, libc::c_int::MIN] {
            assert_eq!(
                judge(PID, &with_operation("network-bind", Ok(value))),
                unconfined(true, &["network-bind"]),
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
            unconfined(true, &["network-inbound"])
        );
        readings.operations.clear();
        assert_eq!(judge(PID, &readings), unconfined(true, &REQUIRED_DENIALS));
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
                unconfined(false, &[]),
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
            unconfined(false, &REQUIRED_DENIALS)
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
