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
