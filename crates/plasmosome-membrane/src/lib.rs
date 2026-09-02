//! The cell membrane: one host-side supervisor per cell. It owns the VMM fork
//! and the broker daemons it supervises — what the controller
//! (`plasmosome-core`) must never own. Its binary, `membraned`, spawns the
//! brokers its config names and answers `membrane.status` for them on a
//! private control socket.

pub mod brokers;
pub mod control;
pub mod daemon;
pub mod exec;
pub mod readiness;
pub mod vmm;
