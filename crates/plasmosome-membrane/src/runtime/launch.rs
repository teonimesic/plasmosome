use crate::runtime::recipe::RuntimeRecipe;
use crate::vmm::Launch;
use std::ffi::{CStr, CString};
use std::os::fd::OwnedFd;

/// The exit code of a launched child whose setup before `execve` failed: the
/// signal reset, a `dup2` onto descriptors 0 to 3, closing the descriptors
/// above 3, or `chdir("/")`.
pub const SETUP_FAILED: i32 = 125;

/// The exit code of a launched child whose `execve` of the helper returned.
pub const EXEC_FAILED: i32 = 127;

/// The pinned VMM helper, laid out in the parent so that a child forked by
/// `VmmChild::spawn` execs it with exactly what spec 001 §4.2 names: argv
/// `[helper.path, "--runtime-fd=3"]`, an empty environment, descriptors 0
/// and 1 on `/dev/null`, 2 on the diagnostic pipe, 3 on a pipe that holds the
/// recipe's one ndjson line and then end of file, and no other descriptor.
///
/// Get one from [`HelperExec::prepare`] and pass it to `VmmChild::spawn`,
/// which forks, starts a new session and runs `launch` in the child. Never
/// call `launch` yourself: in the calling process it replaces descriptors 0
/// to 3, closes every other descriptor and execs the helper in place of the
/// caller. Dropping it closes the parent's copies of what the child would
/// have received, which `VmmChild::spawn` does after the fork.
///
/// The child resets every signal to its default action and unblocks them
/// all, sets umask 077 and the working directory `/`, and execs. It
/// allocates nothing and takes no lock, so the parent may have other
/// threads. It exits [`SETUP_FAILED`] when its setup fails and
/// [`EXEC_FAILED`] when `execve` returns; both differ from the helper's own
/// codes, 64 to 69 and 86, and from `VmmChild`'s 70 and 71.
pub struct HelperExec {
    program: CString,
    dev_null: OwnedFd,
    diagnostics_write: OwnedFd,
    recipe_read: OwnedFd,
}

/// A helper ready to launch, and the read end of its diagnostic pipe.
#[derive(Debug)]
pub struct PreparedHelper {
    /// Pass to `VmmChild::spawn`.
    pub exec: HelperExec,
    /// The read end of the pipe the helper gets as descriptor 2, close-on-exec
    /// and nonblocking. It reaches end of file once the helper and everything
    /// it gave descriptor 2 to have exited or closed it. Keep reading it: a
    /// helper that fills the pipe blocks on its next diagnostic.
    pub diagnostics: OwnedFd,
}

/// Why a helper could not be prepared. Nothing was forked, and every
/// descriptor the preparation made is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchRefusal {
    /// Opening or reading the status of `/dev/null` failed with `errno`.
    DevNull { errno: i32 },
    /// `/dev/null` opened, but it is not the character device that
    /// `/dev/null` names.
    NotDevNull,
    /// Creating the two pipes and moving their ends above descriptor 9 failed
    /// with `errno`.
    Pipe { errno: i32 },
    /// A pipe end was still inheritable when creation returned, so every end
    /// was closed.
    PipeInheritable,
    /// The call `op` on a descriptor failed with `errno`.
    Descriptor { op: &'static str, errno: i32 },
    /// The recipe pipe took `accepted` of the `bytes` of the recipe line
    /// without blocking.
    RecipeExceedsPipe { bytes: usize, accepted: usize },
    /// The child could not be given a way to close every descriptor above 3:
    /// on Linux the kernel refused `close_range`, and on Darwin
    /// `kern.maxfilesperproc` could not be read; `errno` says why.
    CloseLimit { errno: i32 },
}

impl HelperExec {
    /// Lays out the exec of `recipe`'s helper. Every allocation and every
    /// call that can fail happens here, in the parent.
    ///
    /// It opens `/dev/null`, creates the diagnostic and recipe pipes holding
    /// the write side of the spawn lock, moves every descriptor it keeps to
    /// 10 or above, writes `recipe.to_ndjson()` into the recipe pipe without
    /// blocking and closes that pipe's write end, so the helper reads the
    /// line and then end of file. A line the pipe cannot take whole refuses
    /// with [`LaunchRefusal::RecipeExceedsPipe`]. A pipe takes about 64 KiB
    /// on the supported hosts, so this also keeps the line within the
    /// 1,048,576 bytes, newline included, that the helper reads, the bound
    /// `RuntimeRecipe::to_ndjson` leaves to its launcher.
    ///
    /// It does not open, check or hash the helper or any artifact: verify
    /// them before preparing. On Darwin, run no fork outside
    /// `VmmChild::spawn` while it runs, since a pipe end exists for a moment
    /// before it is close-on-exec and only `VmmChild::spawn` waits for that.
    pub fn prepare(recipe: &RuntimeRecipe) -> Result<PreparedHelper, LaunchRefusal> {
        let _ = recipe;
        todo!()
    }
}

fn open_null(path: &CStr) -> Result<OwnedFd, LaunchRefusal> {
    let _ = path;
    todo!()
}

impl Launch for HelperExec {
    fn launch(self) -> ! {
        let _ = (
            &self.program,
            &self.dev_null,
            &self.diagnostics_write,
            &self.recipe_read,
        );
        todo!()
    }
}

impl std::fmt::Debug for HelperExec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HelperExec")
            .field("program", &self.program)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for LaunchRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::error::Error for LaunchRefusal {}
