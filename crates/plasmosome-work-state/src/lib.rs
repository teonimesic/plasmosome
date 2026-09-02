//! Verification and disposable contract-test support for the pinned Beads release.

pub mod command;
pub mod contract;
pub mod document;
pub mod pin;
pub mod shadow;

pub use contract::{ContractRequest, ContractResult, run_contract};
