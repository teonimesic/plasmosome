//! The repository-wide guards that have nowhere else to run — see `tests/`.
//!
//! Each one refuses something that cannot be taken back or would be read as a promise: a crate
//! reaching a registry under a name this project has not claimed, a binary name two packages
//! both answer to, test scaffolding shipping outside `[dev-dependencies]`, a commit crediting a
//! model, the private research corpus reaching a public tree, and a skill a tool cannot find.
//!
//! Nothing here pins a design. What the kernel's crates may depend on, and what its process seam
//! carries, are decisions this repository is still making; the place to write them down is the
//! spec and the crate's own notes, not a test that fails before either exists.

use std::path::{Path, PathBuf};

/// The absolute path to the root of the workspace this crate is checked into.
///
/// Guards address the files they inspect by their path from that root. The caller must not assume
/// the process working directory matches it, and must not call this from a crate moved to a
/// different depth in the tree.
///
/// The path is baked in when the binary is compiled, so it is right wherever that binary was built
/// and wrong only for one that outlived a move of its own checkout — renaming the directory was
/// observed not to be enough, on its own, to get such a binary replaced. Panics in that case
/// naming the stale path and the rebuild, rather than letting each guard fail as though the file it
/// inspects were missing.
pub fn workspace_root() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the guards crate sits two levels below the workspace root")
        .to_path_buf();
    assert!(root.is_dir(), "{}", stale_root_message(&root));
    root
}

fn stale_root_message(root: &Path) -> String {
    format!(
        "the workspace root is `{}`, baked into this binary when it was compiled, and there is no \
         directory there now. This binary outlived a move of the checkout it was built in. Rebuild \
         before reading the gate as red: `cargo clean -p plasmosome-guards`. Until then every \
         guard reports the file it inspects as unreadable and blames that file.",
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
