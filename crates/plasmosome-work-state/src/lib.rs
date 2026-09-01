//! Verification and disposable contract-test support for the pinned Beads release.

pub mod command;
pub mod contract;
pub mod pin;

pub use contract::{run_contract, ContractRequest, ContractResult};
