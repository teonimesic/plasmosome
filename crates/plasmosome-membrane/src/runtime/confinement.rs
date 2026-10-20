/// The operations a helper's sandbox must deny for `observe` to report it confined, in the order
/// `observe` asks about them.
///
/// Each is asked with no filter argument, and a denial says only that. A policy that allows the
/// operation under a filter, even one matching every argument, can still answer denied. The
/// qualified host policy allows outbound connections to Unix sockets and answers `network-outbound`
/// denied. Measured on macOS 26.6.2, so does a policy allowing `network*` to any remote IP.
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
    /// The same process was read running on both sides of the queries, the kernel reported it
    /// sandboxed, and every answer for each operation in `REQUIRED_DENIALS`, asked with no
    /// filter, was a denial. It does not show that the verified host policy is installed: much
    /// looser profiles read `Confined` too; see `PolicyBinding`.
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
        /// The errno it set: `ESRCH` for an absent pid or a zombie.
        errno: i32,
    },
    /// `proc_pidinfo` returned a byte count other than the size of the process's unique
    /// identifier record, before or after the sandbox queries. It sets no errno for that, so
    /// none is reported.
    ProcessInfoWrongSize {
        /// The byte count it returned.
        bytes: i32,
    },
    /// The process read after the sandbox queries has a different unique id from the one read
    /// before them, so another process took the pid while it was observed.
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
    unique_id: u64,
    denied: Vec<&'static str>,
    binding: PolicyBinding,
}

impl ConfinementEvidence {
    /// The pid that was observed.
    pub fn pid(&self) -> libc::pid_t {
        self.pid
    }

    /// The kernel's unique id of the process that was observed, read before and after the
    /// sandbox queries. Unlike a pid, the kernel never gives it to another process while the
    /// system runs.
    pub fn unique_id(&self) -> u64 {
        self.unique_id
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
    /// Only the answers for `REQUIRED_DENIALS`, asked with no filter, were observed. Darwin has no
    /// call that returns a process's installed profile, so this does not show that the verified
    /// policy is the one installed. A bare `(deny default)` profile is observed the same way, and
    /// so are looser ones: `(allow default)` with the five denied, or `(deny default)` allowing
    /// file writes under `/` and `network*` for any IP. Spec 001 §4.2 asks that substituting
    /// another policy fail, and for evidence binding the verified policy to the process; this
    /// cannot give either alone. Owner gate O-3 on native task plasmosome-qep decides what is
    /// enough.
    PropertiesObserved,
}

/// Observes, from this process, the Darwin sandbox of the running process `pid`.
///
/// Pass the pid of a process this process keeps from being reaped, such as the helper child it
/// owns. `observe` asks `sandbox_check` whether this process is sandboxed, whether `pid` is, and
/// whether each operation in `REQUIRED_DENIALS` is denied. It reads the kernel's unique id of
/// `pid` with `proc_pidinfo` before and after those queries, and the evidence keeps it. It changes
/// nothing.
///
/// Only `Confined` lets readiness go ahead. It claims only that the five operations answer denied
/// with no filter: nothing about which profile is installed, about arguments a policy allows for
/// a denied operation, about operations outside the five, about descriptors the process already
/// holds, or about the images it loaded.
/// `sandbox_check` is declared in no SDK header. If a later macOS makes it fail, the result is
/// `Unverifiable`; the crate's macOS tests check the answers it gives today.
#[cfg(target_os = "macos")]
pub fn observe(pid: libc::pid_t) -> Confinement {
    judge(
        pid,
        &read(pid, std::process::id().cast_signed(), &mut darwin::Darwin),
    )
}

/// Off Darwin there is no supervisor-side sandbox observation, so every `pid` gives
/// `Unverifiable` with `UnsupportedPlatform`.
#[cfg(not(target_os = "macos"))]
pub fn observe(_pid: libc::pid_t) -> Confinement {
    Confinement::Unverifiable {
        reason: Unverifiable::UnsupportedPlatform,
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) type Answer = Result<libc::c_int, i32>;

#[cfg(any(target_os = "macos", test))]
pub(crate) const UNIQUE_IDENTIFIERS_SIZE: libc::c_int = 56;

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PidInfo {
    pub(crate) written: libc::c_int,
    pub(crate) unique_id: u64,
    pub(crate) errno: i32,
}

#[cfg(any(target_os = "macos", test))]
pub(crate) type Identity = Result<u64, Unverifiable>;

#[cfg(any(target_os = "macos", test))]
pub(crate) trait Kernel {
    fn sandbox_check(&mut self, pid: libc::pid_t, operation: Option<&str>) -> Answer;

    fn unique_identifiers(&mut self, pid: libc::pid_t) -> PidInfo;
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Readings {
    pub(crate) own: Answer,
    pub(crate) before: Identity,
    pub(crate) sandboxed: Answer,
    pub(crate) operations: Vec<(&'static str, Answer)>,
    pub(crate) after: Identity,
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn read(pid: libc::pid_t, own: libc::pid_t, kernel: &mut impl Kernel) -> Readings {
    let own = kernel.sandbox_check(own, None);
    let before = identity(kernel.unique_identifiers(pid));
    let sandboxed = kernel.sandbox_check(pid, None);
    let operations = REQUIRED_DENIALS
        .into_iter()
        .map(|operation| (operation, kernel.sandbox_check(pid, Some(operation))))
        .collect();
    let after = identity(kernel.unique_identifiers(pid));
    Readings {
        own,
        before,
        sandboxed,
        operations,
        after,
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn identity(info: PidInfo) -> Identity {
    match info.written {
        UNIQUE_IDENTIFIERS_SIZE => Ok(info.unique_id),
        written if written <= 0 => Err(Unverifiable::ProcessUnreadable { errno: info.errno }),
        bytes => Err(Unverifiable::ProcessInfoWrongSize { bytes }),
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn judge(pid: libc::pid_t, readings: &Readings) -> Confinement {
    let unique_id = match trusted(readings) {
        Ok(unique_id) => unique_id,
        Err(reason) => return Confinement::Unverifiable { reason },
    };
    let sandboxed = readings.sandboxed == Ok(1);
    let allowed: Vec<&'static str> = REQUIRED_DENIALS
        .into_iter()
        .filter(|required| !denied(readings, required))
        .collect();
    if !sandboxed || !allowed.is_empty() {
        return Confinement::Unconfined { sandboxed, allowed };
    }
    Confinement::Confined(ConfinementEvidence {
        pid,
        unique_id,
        denied: REQUIRED_DENIALS.to_vec(),
        binding: PolicyBinding::PropertiesObserved,
    })
}

#[cfg(any(target_os = "macos", test))]
fn denied(readings: &Readings, required: &str) -> bool {
    let mut answers = readings
        .operations
        .iter()
        .filter(|(operation, _)| *operation == required)
        .map(|(_, answer)| *answer)
        .peekable();
    answers.peek().is_some() && answers.all(|answer| answer == Ok(1))
}

#[cfg(any(target_os = "macos", test))]
fn trusted(readings: &Readings) -> Result<u64, Unverifiable> {
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
    let before = readings.before?;
    let after = readings.after?;
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
        None => Ok(before),
    }
}

#[cfg(target_os = "macos")]
mod darwin {
    use super::{Answer, Kernel, PidInfo, UNIQUE_IDENTIFIERS_SIZE};
    use std::ffi::{c_char, c_int};

    const PROC_PIDUNIQIDENTIFIERINFO: c_int = 17;

    #[repr(C)]
    struct UniqueIdentifiers {
        _executable_uuid: [u8; 16],
        unique_id: u64,
        parent_unique_id: u64,
        _id_version: i32,
        _original_parent_id_version: i32,
        _reserved: [u64; 2],
    }

    const _: () = assert!(size_of::<UniqueIdentifiers>() == UNIQUE_IDENTIFIERS_SIZE as usize);

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

    pub(super) struct Darwin;

    impl Kernel for Darwin {
        #[allow(unsafe_code)]
        fn sandbox_check(&mut self, pid: libc::pid_t, operation: Option<&str>) -> Answer {
            let name = operation.map(|operation| [operation.as_bytes(), b"\0"].concat());
            let pointer = name
                .as_ref()
                .map_or(std::ptr::null(), |name| name.as_ptr().cast::<c_char>());
            // SAFETY: `pointer` is null or points into `name`, a buffer ending in NUL that lives
            // until this function returns, and sandbox_check reads it during the call only.
            // Filter type 0 names no filter, so no variadic argument is passed or read.
            let value = unsafe { sandbox_check(pid, pointer, 0) };
            if value == -1 {
                Err(last_errno())
            } else {
                Ok(value)
            }
        }

        fn unique_identifiers(&mut self, pid: libc::pid_t) -> PidInfo {
            let (written, record, errno) = unique_identifier_record(pid);
            PidInfo {
                written,
                unique_id: record.unique_id,
                errno,
            }
        }
    }

    #[allow(unsafe_code)]
    fn unique_identifier_record(pid: libc::pid_t) -> (c_int, UniqueIdentifiers, i32) {
        // SAFETY: UniqueIdentifiers is a plain C struct of integers, for which all zeroes is a
        // valid value.
        let mut record: UniqueIdentifiers = unsafe { std::mem::zeroed() };
        // SAFETY: `record` is a local UniqueIdentifiers, laid out as XNU's struct
        // proc_uniqidentifierinfo from its private proc_info_private.h, which no SDK header
        // declares, and UNIQUE_IDENTIFIERS_SIZE is its exact size. For PROC_PIDUNIQIDENTIFIERINFO
        // proc_pidinfo writes at most that many bytes into it, and it keeps no pointer.
        let written = unsafe {
            libc::proc_pidinfo(
                pid,
                PROC_PIDUNIQIDENTIFIERINFO,
                0,
                (&raw mut record).cast(),
                UNIQUE_IDENTIFIERS_SIZE,
            )
        };
        let errno = last_errno();
        (written, record, errno)
    }

    fn last_errno() -> i32 {
        std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::EIO)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        use crate::runtime::confinement::{Kernel, UNIQUE_IDENTIFIERS_SIZE};
        use std::process::{Command, Stdio};

        #[test]
        fn a_query_the_kernel_cannot_answer_reads_as_its_errno_never_a_value() {
            let own = std::process::id().cast_signed();
            for operation in ["process-exec", "no-such-operation"] {
                assert_eq!(
                    Darwin.sandbox_check(own, Some(operation)),
                    Err(libc::EINVAL),
                    "{operation} needs a filter or is unknown, so sandbox_check returns -1"
                );
            }
        }

        #[test]
        fn the_unique_identifier_record_has_the_kernels_size_and_layout() {
            let own = std::process::id().cast_signed();
            let mine = Darwin.unique_identifiers(own);
            assert_eq!(
                mine.written, UNIQUE_IDENTIFIERS_SIZE,
                "the kernel fills exactly the record's size: {mine:?}"
            );
            let launchd = Darwin.unique_identifiers(1);
            assert_eq!(
                (launchd.written, launchd.unique_id),
                (UNIQUE_IDENTIFIERS_SIZE, 1),
                "launchd is the first process the kernel starts"
            );
            let mut child = Command::new("/bin/cat")
                .stdin(Stdio::piped())
                .spawn()
                .expect("cat starts");
            let pid = child.id().cast_signed();
            let theirs = Darwin.unique_identifiers(pid);
            let (written, record, _) = unique_identifier_record(pid);
            drop(child.stdin.take());
            child.wait().expect("cat ends at the end of its input");
            assert_eq!(written, UNIQUE_IDENTIFIERS_SIZE);
            assert_eq!(theirs.unique_id, record.unique_id);
            assert_ne!(theirs.unique_id, mine.unique_id);
            assert_eq!(
                record.parent_unique_id, mine.unique_id,
                "a child's record names this process as its parent"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const PID: libc::pid_t = 4242;
    const FIVE: [&str; 5] = [
        "process-fork",
        "network-outbound",
        "network-inbound",
        "network-bind",
        "file-write-create",
    ];
    const OTHER_NAMES: [&str; 3] = ["file-read-data", "process-exec", "mach-lookup"];
    const UNIQUE_ID: u64 = 90_210;
    const OWN: libc::pid_t = 77;
    const STALE: i32 = 9999;

    fn answering(answer: Answer) -> Vec<(&'static str, Answer)> {
        REQUIRED_DENIALS
            .iter()
            .map(|operation| (*operation, answer))
            .collect()
    }

    fn confined() -> Readings {
        Readings {
            own: Ok(0),
            before: Ok(UNIQUE_ID),
            sandboxed: Ok(1),
            operations: answering(Ok(1)),
            after: Ok(UNIQUE_ID),
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
        assert_eq!(evidence.unique_id(), UNIQUE_ID);
        assert_eq!(evidence.denied(), FIVE);
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
            unconfined(true, &FIVE),
            "allow-default answers sandboxed and allows all five"
        );
    }

    #[test]
    fn each_required_operation_left_allowed_is_unconfined_and_named() {
        for operation in FIVE {
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
        assert_eq!(judge(PID, &readings), unconfined(true, &FIVE));
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
        assert_eq!(judge(PID, &unsandboxed), unconfined(false, &FIVE));
    }

    #[test]
    fn any_allowed_answer_for_a_required_operation_is_not_a_denial() {
        let mut readings = confined();
        readings.operations.push(("process-fork", Ok(0)));
        assert_eq!(
            judge(PID, &readings),
            unconfined(true, &["process-fork"]),
            "a second, allowed answer for process-fork after its denial"
        );
        let mut readings = confined();
        readings.operations.insert(0, ("network-bind", Ok(0)));
        assert_eq!(
            judge(PID, &readings),
            unconfined(true, &["network-bind"]),
            "an allowed answer for network-bind before its denial"
        );
        let mut readings = confined();
        readings.operations.reverse();
        readings.operations.push(("file-read-data", Ok(0)));
        assert!(
            matches!(judge(PID, &readings), Confinement::Confined(_)),
            "the order of the answers and an answer for another operation change nothing"
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
        for operation in FIVE {
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
    fn a_failed_query_for_another_operation_is_unverifiable_too() {
        for operation in OTHER_NAMES {
            let mut readings = confined();
            readings.operations.push((operation, Err(libc::EINVAL)));
            assert_eq!(
                judge(PID, &readings),
                unverifiable(Unverifiable::CheckFailed {
                    operation: Some(operation),
                    errno: libc::EINVAL,
                }),
                "five denials and a failed query for {operation}"
            );
        }
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
        for reason in [
            Unverifiable::ProcessUnreadable { errno: libc::ESRCH },
            Unverifiable::ProcessUnreadable { errno: libc::EPERM },
            Unverifiable::ProcessInfoWrongSize { bytes: 10 },
        ] {
            for readings in [
                Readings {
                    before: Err(reason),
                    ..confined()
                },
                Readings {
                    after: Err(reason),
                    ..confined()
                },
            ] {
                assert_eq!(
                    judge(PID, &readings),
                    unverifiable(reason),
                    "a dead pid answers 1 to every sandbox query, so those answers are not used: {readings:?}"
                );
            }
        }
    }

    #[test]
    fn a_pid_naming_another_process_after_the_queries_is_unverifiable() {
        for after in [UNIQUE_ID + 1, UNIQUE_ID - 1, 0] {
            assert_eq!(
                judge(
                    PID,
                    &Readings {
                        after: Ok(after),
                        ..confined()
                    }
                ),
                unverifiable(Unverifiable::ProcessReplaced),
                "the process read after the queries has unique id {after}"
            );
        }
    }

    #[test]
    fn a_record_of_the_full_size_gives_the_process_unique_id() {
        assert_eq!(
            identity(PidInfo {
                written: UNIQUE_IDENTIFIERS_SIZE,
                unique_id: UNIQUE_ID,
                errno: STALE,
            }),
            Ok(UNIQUE_ID)
        );
        for (written, errno) in [(0, libc::ESRCH), (-1, libc::EPERM)] {
            assert_eq!(
                identity(PidInfo {
                    written,
                    unique_id: UNIQUE_ID,
                    errno,
                }),
                Err(Unverifiable::ProcessUnreadable { errno }),
                "proc_pidinfo returned {written}"
            );
        }
    }

    #[test]
    fn a_record_of_another_size_names_its_size_not_a_stale_errno() {
        for bytes in [
            1,
            10,
            UNIQUE_IDENTIFIERS_SIZE - 1,
            UNIQUE_IDENTIFIERS_SIZE + 1,
        ] {
            assert_eq!(
                identity(PidInfo {
                    written: bytes,
                    unique_id: UNIQUE_ID,
                    errno: STALE,
                }),
                Err(Unverifiable::ProcessInfoWrongSize { bytes }),
                "proc_pidinfo returned {bytes} bytes and left an earlier errno"
            );
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Life {
        Running,
        Dead,
        Replaced,
    }

    struct FakeKernel {
        allows: bool,
        fate: Life,
        life: Life,
        record_bytes: libc::c_int,
        own_queries: usize,
    }

    impl FakeKernel {
        fn new(allows: bool, fate: Life) -> FakeKernel {
            FakeKernel {
                allows,
                fate,
                life: Life::Running,
                record_bytes: UNIQUE_IDENTIFIERS_SIZE,
                own_queries: 0,
            }
        }
    }

    impl Kernel for FakeKernel {
        fn sandbox_check(&mut self, pid: libc::pid_t, operation: Option<&str>) -> Answer {
            if pid == OWN {
                self.own_queries += 1;
                return Ok(0);
            }
            assert_eq!(pid, PID, "the fake knows only the observer and one process");
            if operation.is_some() && self.life == Life::Running {
                self.life = self.fate;
            }
            match (self.life, operation) {
                (Life::Running, Some(_)) if self.allows => Ok(0),
                _ => Ok(1),
            }
        }

        fn unique_identifiers(&mut self, pid: libc::pid_t) -> PidInfo {
            assert_eq!(pid, PID, "only the observed process is read");
            match self.life {
                Life::Running => PidInfo {
                    written: self.record_bytes,
                    unique_id: UNIQUE_ID,
                    errno: STALE,
                },
                Life::Replaced => PidInfo {
                    written: UNIQUE_IDENTIFIERS_SIZE,
                    unique_id: UNIQUE_ID + 1,
                    errno: STALE,
                },
                Life::Dead => PidInfo {
                    written: 0,
                    unique_id: 0,
                    errno: libc::ESRCH,
                },
            }
        }
    }

    fn observed(kernel: &mut FakeKernel) -> Confinement {
        judge(PID, &read(PID, OWN, kernel))
    }

    #[test]
    fn a_running_process_is_judged_on_what_the_kernel_answers() {
        let Confinement::Confined(evidence) = observed(&mut FakeKernel::new(false, Life::Running))
        else {
            panic!("a running process that denies the five is confined");
        };
        assert_eq!((evidence.pid(), evidence.unique_id()), (PID, UNIQUE_ID));
        assert_eq!(
            observed(&mut FakeKernel::new(true, Life::Running)),
            unconfined(true, &FIVE)
        );
    }

    #[test]
    fn a_process_that_dies_during_the_queries_is_unverifiable() {
        assert_eq!(
            observed(&mut FakeKernel::new(true, Life::Dead)),
            unverifiable(Unverifiable::ProcessUnreadable { errno: libc::ESRCH }),
            "an allow-default process that dies at its first operation query answers 1 to the rest"
        );
    }

    #[test]
    fn a_pid_taken_by_another_process_during_the_queries_is_unverifiable() {
        assert_eq!(
            observed(&mut FakeKernel::new(true, Life::Replaced)),
            unverifiable(Unverifiable::ProcessReplaced),
            "an allow-default process replaced at its first operation query by one that denies the five"
        );
    }

    #[test]
    fn a_record_of_another_size_stops_the_observation() {
        for bytes in [10, UNIQUE_IDENTIFIERS_SIZE - 8] {
            let mut kernel = FakeKernel {
                record_bytes: bytes,
                ..FakeKernel::new(false, Life::Running)
            };
            assert_eq!(
                observed(&mut kernel),
                unverifiable(Unverifiable::ProcessInfoWrongSize { bytes })
            );
        }
    }

    #[test]
    fn every_observation_asks_whether_the_observer_is_sandboxed() {
        let mut kernel = FakeKernel::new(false, Life::Running);
        for observations in 1..=3 {
            observed(&mut kernel);
            assert_eq!(kernel.own_queries, observations);
        }
    }

    fn answer(usual: libc::c_int) -> impl Strategy<Value = Answer> {
        prop_oneof![
            8 => Just(Ok(usual)),
            1 => (-3..4).prop_map(Ok),
            1 => (1..120).prop_map(Err),
        ]
    }

    fn identities() -> impl Strategy<Value = Identity> {
        prop_oneof![
            8 => Just(Ok(UNIQUE_ID)),
            1 => (0..3u64).prop_map(Ok),
            1 => (1..120).prop_map(|errno| Err(Unverifiable::ProcessUnreadable { errno })),
            1 => (1..56).prop_map(|bytes| Err(Unverifiable::ProcessInfoWrongSize { bytes })),
        ]
    }

    fn operations() -> impl Strategy<Value = Vec<(&'static str, Answer)>> {
        (
            prop::collection::vec(prop::option::weighted(0.95, answer(1)), 5),
            prop::collection::vec(
                (
                    prop::sample::select(FIVE.into_iter().chain(OTHER_NAMES).collect::<Vec<_>>()),
                    answer(1),
                ),
                0..4,
            ),
        )
            .prop_map(|(required, extra)| {
                FIVE.into_iter()
                    .zip(required)
                    .filter_map(|(operation, answer)| answer.map(|answer| (operation, answer)))
                    .chain(extra)
                    .collect::<Vec<_>>()
            })
            .prop_shuffle()
    }

    fn readings() -> impl Strategy<Value = Readings> {
        (
            answer(0),
            identities(),
            answer(1),
            operations(),
            identities(),
        )
            .prop_map(|(own, before, sandboxed, operations, after)| Readings {
                own,
                before,
                sandboxed,
                operations,
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
                && readings.operations.iter().all(|(_, answer)| answer.is_ok())
                && FIVE.iter().all(|required| {
                    let mut answers = readings
                        .operations
                        .iter()
                        .filter(|(operation, _)| operation == required)
                        .map(|(_, answer)| *answer)
                        .peekable();
                    answers.peek().is_some() && answers.all(|answer| answer == Ok(1))
                });
            prop_assert_eq!(
                matches!(judge(PID, &readings), Confinement::Confined(_)),
                every_rule_holds,
                "{:?}",
                readings
            );
        }
    }
}
