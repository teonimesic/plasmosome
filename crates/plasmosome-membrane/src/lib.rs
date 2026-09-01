//! The cell membrane: one host-side supervisor per cell, holding the VMM fork,
//! the netstack shim, the vsock bridges, and egressd/dnsd supervision. Its
//! binary, `membraned`, spawns the brokers its config names and answers
//! `membrane.status` for them on a private control socket.

pub mod brokers;
pub mod control;
pub mod daemon;
pub mod exec;
pub mod readiness;
pub mod vmm;
