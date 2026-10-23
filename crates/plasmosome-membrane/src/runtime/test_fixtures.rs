use super::recipe::{PlatformLimits, RuntimeRecipe};
use serde_json::{Value, json};

pub(crate) mod fake {
    mod compiled {
        include!("../../tests/support/runtime_fake.rs");
    }

    #[cfg(target_os = "macos")]
    pub(crate) use compiled::owned_wait;

    pub(crate) fn runtime_fake(mode: &str) -> std::path::PathBuf {
        compiled::runtime_fake(mode)
    }
}

#[cfg(target_os = "macos")]
pub(crate) mod host_policy {
    include!("../../tests/support/host_policy.rs");
}

pub(crate) mod newc;

#[cfg(target_os = "macos")]
pub(crate) mod bundle;
#[cfg(target_os = "macos")]
pub(crate) use bundle::FixtureBundle;

pub(crate) fn artifact(path: &str, digit: char) -> Value {
    json!({"path": path, "sha256": digit.to_string().repeat(64)})
}

pub(crate) fn recipe_value(helper: &str) -> Value {
    let kernel_format = if std::env::consts::ARCH == "x86_64" {
        "elf"
    } else {
        "raw"
    };
    json!({
        "version": 1,
        "vcpus": 1,
        "memory_mib": 512,
        "kernel": artifact("/opt/cell/kernel", '1'),
        "kernel_format": kernel_format,
        "initramfs": artifact("/opt/cell/initramfs", '2'),
        "root_image": artifact("/opt/cell/root.raw", '3'),
        "writable_root": "/var/cell/c1/root.raw",
        "control_path": "/var/cell/c1/control.sock",
        "data_path": "/var/cell/c1/data.sock",
        "helper": artifact(helper, '4'),
        "libraries": [artifact("/opt/cell/lib/libkrun.1.dylib", '5')],
        "host_policy": artifact("/opt/cell/host.sb", '6'),
        "guest_policy": artifact("/opt/cell/guest.json", '7'),
        "architecture": std::env::consts::ARCH,
    })
}

pub(crate) fn recipe(value: &Value) -> RuntimeRecipe {
    let limits = PlatformLimits::host().expect("this host reports its limits");
    RuntimeRecipe::parse(format!("{value}\n").as_bytes(), &limits)
        .unwrap_or_else(|refusal| panic!("the test recipe is valid on this host: {refusal}"))
}

/// A new 0700 directory that core's `PrivateDir::open` accepts. Its base is the per-user temp
/// directory `confstr(_CS_DARWIN_USER_TEMP_DIR)` names on macOS, and on Linux
/// `XDG_RUNTIME_DIR` when it is set, then the test binary's directory, whose ancestors are
/// usually private where the shared temp directory is not.
pub(crate) fn private_root() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let mut refusals = Vec::new();
    for base in tree::root_bases() {
        let root = match tempfile::Builder::new().prefix("ep").tempdir_in(&base) {
            Ok(root) => root,
            Err(error) => {
                refusals.push(format!("{}: {error}", base.display()));
                continue;
            }
        };
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("the test root is made private");
        match plasmosome_core::private_socket::PrivateDir::open(root.path()) {
            Ok(_) => return root,
            Err(refusal) => refusals.push(format!("{}: {refusal}", base.display())),
        }
    }
    panic!(
        "no base can hold a private test root:\n{}",
        refusals.join("\n")
    );
}

pub(crate) mod tree {
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use plasmosome_core::nofollow::{Walk, WalkRefusal};
    use tempfile::TempDir;

    pub(crate) struct ArtifactTree {
        path: PathBuf,
        _dir: TempDir,
    }

    impl ArtifactTree {
        pub(crate) fn new() -> ArtifactTree {
            let mut refusals = Vec::new();
            for base in root_bases() {
                let made = tempfile::Builder::new().prefix("aep").tempdir_in(&base);
                let walked = made.map_err(|error| error.to_string()).and_then(|dir| {
                    let path = fs::canonicalize(dir.path()).expect("the tree resolves");
                    set_mode(&path, 0o700);
                    let walk = Walk::from_root(&path).map_err(|refusal| refusal.to_string())?;
                    walk.open_directory(|_| Ok::<(), WalkRefusal>(()))
                        .map_err(|refusal| refusal.to_string())?;
                    Ok(ArtifactTree { path, _dir: dir })
                });
                match walked {
                    Ok(tree) => return tree,
                    Err(refusal) => refusals.push(format!("{}: {refusal}", base.display())),
                }
            }
            panic!("no base holds a tree the walk from / accepts: {refusals:?}");
        }

        pub(crate) fn join(&self, relative: &str) -> PathBuf {
            self.path.join(relative)
        }

        pub(crate) fn dir(&self, relative: &str, mode: u32) -> PathBuf {
            let path = self.join(relative);
            fs::create_dir(&path).expect("mkdir");
            set_mode(&path, mode);
            path
        }

        pub(crate) fn file(&self, relative: &str, bytes: &[u8], mode: u32) -> PathBuf {
            let path = self.join(relative);
            fs::write(&path, bytes).expect("write");
            set_mode(&path, mode);
            path
        }

        #[cfg(target_os = "macos")]
        pub(crate) fn immutable_file(&self, relative: &str, bytes: &[u8]) -> PathBuf {
            let path = self.file(relative, bytes, 0o444);
            set_flags(&path, libc::UF_IMMUTABLE);
            path
        }
    }

    impl Drop for ArtifactTree {
        fn drop(&mut self) {
            open_up(&self.path);
            #[cfg(target_os = "macos")]
            {
                let _ = crate::spawn_lock::spawn_guarded(
                    std::process::Command::new("/bin/chmod")
                        .args(["-R", "-N"])
                        .arg(&self.path)
                        .stderr(std::process::Stdio::null()),
                )
                .and_then(|mut child| child.wait());
            }
        }
    }

    fn open_up(path: &Path) {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return;
        };
        if metadata.file_type().is_symlink() {
            return;
        }
        #[cfg(target_os = "macos")]
        chflags(path, 0);
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

    pub(crate) fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn set_flags(path: &Path, flags: libc::c_uint) {
        let error = (chflags(path, flags) != 0).then(std::io::Error::last_os_error);
        assert!(
            error.is_none(),
            "chflags {flags:#x} {}: {error:?}",
            path.display()
        );
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    fn chflags(path: &Path, flags: libc::c_uint) -> libc::c_int {
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("no NUL");
        // SAFETY: `name` is a CString that outlives the call, and chflags keeps no pointer to it.
        unsafe { libc::chflags(name.as_ptr(), flags) }
    }

    #[allow(unsafe_code)]
    pub(crate) fn make_fifo(path: &Path) {
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("no NUL");
        // SAFETY: `name` is a CString that outlives the call, and mkfifo keeps no pointer to it.
        let outcome = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
        assert_eq!(outcome, 0, "mkfifo: {}", std::io::Error::last_os_error());
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn add_acl(path: &Path, rule: &str) {
        let status = crate::spawn_lock::spawn_guarded(
            std::process::Command::new("/bin/chmod")
                .arg("+a")
                .arg(rule)
                .arg(path),
        )
        .and_then(|mut child| child.wait())
        .expect("chmod +a runs");
        assert!(status.success(), "chmod +a {rule:?} failed");
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    pub(crate) fn root_bases() -> Vec<PathBuf> {
        use std::ffi::OsStr;

        let mut buffer = vec![0u8; libc::PATH_MAX as usize];
        // SAFETY: `buffer` is a live Vec of buffer.len() bytes, and confstr writes at most that
        // many.
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

    #[cfg(not(target_os = "macos"))]
    pub(crate) fn root_bases() -> Vec<PathBuf> {
        let mut bases: Vec<PathBuf> = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|base| !base.is_empty())
            .map(PathBuf::from)
            .into_iter()
            .collect();
        let binary = std::env::current_exe().expect("the test binary knows its own path");
        bases.extend(binary.parent().map(Path::to_path_buf));
        bases
    }
}
