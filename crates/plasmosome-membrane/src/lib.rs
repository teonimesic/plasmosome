//! The cell membrane: one host-side supervisor per cell. It holds the VMM
//! fork, the netstack shim, the vsock bridges, and egressd/dnsd supervision
//! — everything the controller (`plasmosome-core`) must never own (86 §4
//! rules 1, 5, 6). P1 freeze groundwork: the crate is reserved with its
//! binary name (`membraned`) and the F9 readiness contract; the VMM/shim/
//! broker machinery lands in the next P1 step.
//!
//! Readiness (F9, measured): a supervisor is ready when its control socket
//! *answers* a control-`status` request — not when the process is alive and
//! not when a listener accepts. A confinement-profile bug leaves a broker
//! half-alive (control UDS dead, data listener still answering), so
//! `readiness::probe` treats accept-without-answer as not ready.
//!
//! VMM lifecycle (`vmm`): the supervisor owns its forked VMM child end to end
//! — fork, non-blocking state poll, kill, and reap on drop — so a dropped
//! handle never leaves an orphaned hypervisor process behind.

pub mod readiness;
pub mod vmm;
