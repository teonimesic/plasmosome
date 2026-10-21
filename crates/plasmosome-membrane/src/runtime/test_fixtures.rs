use super::recipe::{PlatformLimits, RuntimeRecipe};
use serde_json::{Value, json};

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
                let dir = match tempfile::Builder::new().prefix("aep").tempdir_in(&base) {
                    Ok(dir) => dir,
                    Err(error) => {
                        refusals.push(format!("{}: {error}", base.display()));
                        continue;
                    }
                };
                let path = fs::canonicalize(dir.path()).expect("the tree resolves");
                set_mode(&path, 0o700);
                let walked = Walk::from_root(&path)
                    .and_then(|walk| walk.open_directory(|_| Ok::<(), WalkRefusal>(())));
                match walked {
                    Ok(_) => return ArtifactTree { path, _dir: dir },
                    Err(refusal) => refusals.push(format!("{}: {refusal}", base.display())),
                }
            }
            panic!(
                "no base holds a tree the walk from / accepts; on macOS the base is the per-user \
                 temp directory confstr(_CS_DARWIN_USER_TEMP_DIR) names, on Linux \
                 XDG_RUNTIME_DIR when it is set, then the test binary's directory:\n{}",
                refusals.join("\n")
            );
        }

        pub(crate) fn path(&self) -> &Path {
            &self.path
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
            let _ = std::process::Command::new("/bin/chmod")
                .args(["-R", "-N"])
                .arg(&self.path)
                .stderr(std::process::Stdio::null())
                .status();
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
        clear_flags(path);
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
        let outcome = chflags(path, flags);
        assert_eq!(
            outcome,
            0,
            "chflags {flags:#x} {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        );
    }

    #[cfg(target_os = "macos")]
    fn clear_flags(path: &Path) {
        let _ = chflags(path, 0);
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
        let status = std::process::Command::new("/bin/chmod")
            .arg("+a")
            .arg(rule)
            .arg(path)
            .status()
            .expect("chmod +a runs");
        assert!(status.success(), "chmod +a {rule:?} failed");
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    fn root_bases() -> Vec<PathBuf> {
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
    fn root_bases() -> Vec<PathBuf> {
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
