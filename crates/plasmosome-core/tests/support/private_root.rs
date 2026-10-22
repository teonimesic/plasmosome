use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use plasmosome_core::private_socket::PrivateDir;
use tempfile::TempDir;

#[cfg(target_os = "macos")]
const SUN_PATH_MAX: usize = 103;
#[cfg(target_os = "linux")]
const SUN_PATH_MAX: usize = 107;

const NAME_ROOM: usize = 16;

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn root_bases() -> Vec<PathBuf> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let mut buffer = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: `buffer` is a live Vec of buffer.len() bytes, and confstr writes at most that many.
    let length = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    assert!(
        length > 1 && length <= buffer.len(),
        "confstr(_CS_DARWIN_USER_TEMP_DIR) returned {length}"
    );
    buffer.truncate(length - 1);
    let base = PathBuf::from(OsStr::from_bytes(&buffer));
    vec![
        base.canonicalize()
            .unwrap_or_else(|error| panic!("{} does not resolve: {error}", base.display())),
    ]
}

#[cfg(target_os = "macos")]
const ROOT_HINT: &str = "the integration tests make their roots under the per-user temp \
directory that confstr(_CS_DARWIN_USER_TEMP_DIR) reports; its ancestors must be owned by root \
or by this user, writable by neither group nor other, and carry no ACL allow entry granting \
add_file, add_subdirectory, delete_child, delete, writesecurity or chown";

#[cfg(target_os = "linux")]
fn root_bases() -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|base| !base.is_empty())
        .map(PathBuf::from)
        .into_iter()
        .collect();
    bases.push(PathBuf::from(env!("CARGO_TARGET_TMPDIR")));
    bases
}

#[cfg(target_os = "linux")]
const ROOT_HINT: &str = "the integration tests make their roots under XDG_RUNTIME_DIR when it \
is set, else under the target directory's tmp; set XDG_RUNTIME_DIR to a short path whose \
ancestors are owned by root or by this user and writable by neither group nor other";

/// A 0700 directory that `PrivateDir::open` accepts, with room for a socket name of 16 bytes.
/// Dropping it opens up every directory below it, strips ACLs on macOS, and removes it.
pub struct TestRoot(TempDir);

impl TestRoot {
    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        open_up(self.path());
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("/bin/chmod")
            .args(["-R", "-N"])
            .arg(self.path())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

fn open_up(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if !metadata.is_dir() {
        return;
    }
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        open_up(&entry.path());
    }
}

/// Makes a private test root: on macOS under the canonical per-user temp directory, on Linux
/// under `XDG_RUNTIME_DIR` when it is set, else under `CARGO_TARGET_TMPDIR`. Panics naming
/// every base that cannot hold one, and the rule each broke.
pub fn private_root() -> TestRoot {
    TestRoot(private_dir())
}

/// Makes the same directory as [`private_root`], as a plain `TempDir`. Dropping it only removes
/// it: it opens up no mode and, on macOS, starts no `/bin/chmod` to strip ACLs. Use it where the
/// tests change no mode and plant no ACL, and where one of them holds a descriptor without
/// close-on-exec, which a process another test started would inherit.
pub fn private_dir() -> TempDir {
    let mut refusals = Vec::new();
    for base in root_bases() {
        let root = match tempfile::Builder::new().prefix("ps").tempdir_in(&base) {
            Ok(root) => root,
            Err(error) => {
                refusals.push(format!("{}: {error}", base.display()));
                continue;
            }
        };
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))
            .expect("the test root is made private");
        let length = root.path().as_os_str().len();
        match PrivateDir::open(root.path()) {
            Err(refusal) => refusals.push(format!("{}: {refusal}", base.display())),
            Ok(_) if length + 1 + NAME_ROOM > SUN_PATH_MAX => refusals.push(format!(
                "{}: a {length}-byte root leaves fewer than {NAME_ROOM} bytes for a socket name",
                base.display()
            )),
            Ok(_) => return root,
        }
    }
    panic!(
        "no base can hold a private test root:\n{}\n{ROOT_HINT}",
        refusals.join("\n")
    );
}
