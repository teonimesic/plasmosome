//! The 86 §4 must-not-bake-in rules as always-on tests — see `tests/`.
//!
//! The controller-side crates (`plasmosome-core`, `plasmosome-backend`,
//! `plasmosome-ledger`) stay free of VMM/netstack/membrane dependencies
//! (rule 1), move state only as serde data with no shared memory across the
//! seam (rule 2), and keep durable state replayable from the log (rule 3 —
//! proven in `plasmosome-ledger/tests/replayable_from_log.rs`; desired-state
//! generation and convergence in `plasmosome-core/src/reconciler.rs`).
//!
//! Rule status at this freeze point: rules 1–3 enforced by this crate's
//! tests; rule 4 (residue observed off the wire) and rule 5 (no
//! death-tethering; per-cell vs per-host brokers as an explicit parameter)
//! bind when `plasmosome-membrane` gains its supervisor machinery; rule 6
//! (entitlement only on the HVF-entering process) binds when the macOS
//! signing step returns. They are recorded as open enforcement points, not
//! satisfied here.
