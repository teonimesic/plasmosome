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

use std::ffi::OsStr;
use std::path::Path;

mod workspace;

const BUILD_ROOT: &str = include_str!(concat!(env!("OUT_DIR"), "/workspace-root"));

/// Check the workspace selected by this process's working directory.
///
/// Cargo runs tests in their package directory; directly invoked binaries use their actual cwd.
/// Resolve once and pass this root to every repository-reading helper, keeping scratch fixture
/// paths separate. Cargo must be available through `CARGO` or `PATH`.
///
/// A relocated checking component reports `StaleTarget` before executing the check against the
/// selected tree. Its observations remain visible, but even a successful check then fails:
/// rebuild the component and its consuming test binaries in the selected workspace.
pub fn check_workspace(check: impl FnOnce(&Path)) {
    let directory = std::env::current_dir().unwrap_or_else(|error| {
        panic!("WorkspaceRootUnavailable: cannot capture the process working directory: {error}")
    });
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let root = workspace::locate_workspace(&directory, &cargo).unwrap_or_else(|error| {
        panic!(
            "WorkspaceRootUnavailable: cannot resolve workspace from {}: {error}",
            directory.display()
        )
    });
    let stale = root.as_os_str() != OsStr::new(BUILD_ROOT);
    if stale {
        eprintln!(
            "StaleTarget: the checking component was built in {BUILD_ROOT}, but this invocation \
             selects {}. Rebuild plasmosome-guards and its consuming test binaries for the \
             selected workspace; the following check still inspects that workspace.",
            root.display()
        );
    }
    check(&root);
    assert!(
        !stale,
        "StaleTarget: the consumer passed, but relocated build output cannot certify this workspace"
    );
}
