//! The cell membrane: one host-side supervisor per cell, owning the VMM fork
//! and the broker daemons it supervises. Its binary, `membraned`, spawns the
//! brokers its config names and answers `membrane.status` for them on a
//! private control socket.

pub mod brokers;
pub mod control;
pub mod daemon;
pub mod exec;
pub mod readiness;
pub mod vmm;
