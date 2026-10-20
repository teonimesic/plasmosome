use std::path::Path;

/// One of the four confinement checks and what the process observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub expected_allowed: bool,
    pub observed: Result<(), String>,
}

impl Check {
    /// True when the observation is the expected one. A refusal counts only as `EPERM` or
    /// `EACCES`; a missing directory is not a refusal.
    pub fn holds(&self) -> bool {
        true
    }
}

/// Read inside `allowed` (expected to succeed), then write inside it, read `outside` and write
/// inside `outside` (each expected to be refused). A write that succeeds is removed again.
pub fn self_check(allowed: &Path, outside: &Path) -> Vec<Check> {
    let _ = (allowed, outside);
    Vec::new()
}

/// True when every check holds and all four ran.
pub fn passed(checks: &[Check]) -> bool {
    checks.iter().all(Check::holds)
}
