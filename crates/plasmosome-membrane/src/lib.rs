//! The cell membrane: one host-side supervisor per cell. It holds the VMM
//! fork, the netstack shim, the vsock bridges, and egressd/dnsd supervision
//! — everything the controller (`plasmosome-core`) must never own (86 §4
//! rules 1, 5, 6). Its binary, `membraned`, spawns the brokers its config
//! names and answers `membrane.status` for them on a private control socket;
//! the shim and the vsock bridges land in the next P1 step.
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
//!
//! Broker supervision (`brokers`): a cell's brokers are one `vmm::VmmChild`
//! each, so they inherit that same kill-and-reap-on-drop guarantee. The set
//! answers ready only once every broker answers its own control socket, and
//! asks again on every call. One `status` call spends one deadline across the
//! whole set, so a cell with many brokers answers as fast as a cell with one.

pub mod brokers;
pub mod control;
pub mod daemon;
pub mod exec;
pub mod readiness;
pub mod vmm;
