//! The cell membrane: one host-side supervisor per cell, owning the VMM fork
//! and the broker daemons it supervises. Its binary, `membraned`, spawns the
//! brokers its config names and answers `membrane.status` for them on a
//! private control socket.

pub mod brokers;
pub mod cell_args;
pub mod cell_start;
pub mod control;
pub mod daemon;
pub mod exec;
pub mod readiness;
/// Parts of the spec 001 §4.2 hardware runtime: SHA-256 digests, canonical recipe paths, the
/// outcome of removing a path, the 4090 hello requests and reply judge, the strictly parsed
/// `RuntimeRecipe` that launches a cell, the supervisor's own observation of a helper's Darwin
/// sandbox, the exec of the VMM helper that recipe names, the supervisor's control and data
/// listeners, and the transcript of the helper's diagnostics. The paths and the recipe are
/// `plasmosome-core`'s, re-exported here.
pub mod runtime;
mod spawn_lock;
pub mod vmm;
