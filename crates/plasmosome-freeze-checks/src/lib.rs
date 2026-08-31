//! The 86 §4 must-not-bake-in rules as always-on tests, and the repository-wide checks that have
//! nowhere else to run — see `tests/`.
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
//!
//! Alongside them sit checks that are not architectural rules but have the
//! same shape — a property of the repository that must keep holding, checkable
//! on every run. Skill discoverability and the commit guards under `.githooks`
//! are here because this is where checks-as-tests live, and because putting
//! them here costs no new CI step.

use std::path::{Path, PathBuf};

pub mod shared_memory;

/// The absolute path to the root of the workspace this crate is checked into.
///
/// Rules address the files they inspect by their path from that root. The caller must not assume
/// the process working directory matches it, and must not call this from a crate moved to a
/// different depth in the tree.
///
/// The path is baked in when the binary is compiled, so it is right wherever that binary was built
/// and wrong only for one that outlived a move of its own checkout — renaming the directory was
/// observed not to be enough, on its own, to get such a binary replaced. Panics in that case
/// naming the stale path and the rebuild, rather than letting each rule fail as though the file it
/// inspects were missing.
pub fn workspace_root() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the checks crate sits two levels below the workspace root")
        .to_path_buf();
    assert!(root.is_dir(), "{}", stale_root_message(&root));
    root
}

fn stale_root_message(root: &Path) -> String {
    format!(
        "the workspace root is `{}`, baked into this binary when it was compiled, and there is no \
         directory there now. This binary outlived a move of the checkout it was built in. Rebuild \
         before reading the gate as red: `cargo clean -p plasmosome-freeze-checks`. Until then \
         every rule reports the file it inspects as unreadable and blames that file.",
        root.display()
    )
}

#[cfg(test)]
mod tests {
    use super::stale_root_message;
    use std::path::Path;

    #[test]
    fn the_stale_root_message_names_the_baked_path_and_the_rebuild() {
        let message = stale_root_message(Path::new("/moved/away/plasmosome"));
        assert!(message.contains("/moved/away/plasmosome"), "got {message}");
        assert!(message.contains("cargo clean"), "got {message}");
        assert!(
            message.contains("move"),
            "the message must name the cause, not the symptom, got {message}"
        );
    }
}
