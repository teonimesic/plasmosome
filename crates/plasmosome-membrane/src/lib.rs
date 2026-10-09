//! The cell membrane: one host-side supervisor per cell, owning the VMM fork
//! and the broker daemons it supervises. Its binary, `membraned`, spawns the
//! brokers its config names and answers `membrane.status` for them on a
//! private control socket.

pub mod brokers;
pub mod control;
pub mod daemon;
pub mod exec;
pub mod readiness;
/// Parts of the spec 001 §4.2 hardware runtime: SHA-256 digests, canonical recipe paths, the
/// outcome of removing a path, and the 4090 hello requests and reply judge.
pub mod runtime;
mod spawn_lock;
pub mod vmm;
