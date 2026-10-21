use std::ffi::OsString;

/// Where the command line reads the two environment variables spec 025 lets change what a
/// control command does. The binary passes the process's own environment; a test passes its own.
/// Only socket selection reads it, so help, the version and `not_served` never do.
pub trait Env: Sync {
    /// `HOME`, or `None` when it is unset.
    fn home(&self) -> Option<OsString>;

    /// `PLASMOSOME_KERNEL`, or `None` when it is unset.
    fn kernel(&self) -> Option<OsString>;
}

/// The process's own environment, which only `main_with` passes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ProcessEnv;

impl Env for ProcessEnv {
    fn home(&self) -> Option<OsString> {
        std::env::var_os("HOME")
    }

    fn kernel(&self) -> Option<OsString> {
        std::env::var_os("PLASMOSOME_KERNEL")
    }
}
