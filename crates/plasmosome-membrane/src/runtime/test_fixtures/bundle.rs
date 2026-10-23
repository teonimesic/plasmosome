use super::{fake, host_policy, newc, recipe, recipe_value, tree};
use crate::runtime::digest::Digest;
use crate::runtime::recipe::{ArtifactRole, PlatformLimits, RuntimeRecipe};
use serde_json::{Value, json};
use std::fs;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// A test-owned immutable runtime bundle. Keep it until its helper, listeners and verified
/// artifacts have been dropped. Dropping it clears its files' flags and removes its tree.
pub(crate) struct FixtureBundle {
    dir: tempfile::TempDir,
    value: Value,
    files: Vec<PathBuf>,
}

impl FixtureBundle {
    /// Builds one of the C fake's shim modes under the default temporary directory. Panics on
    /// setup failure or socket paths too long for Darwin. It never boots a guest.
    pub(crate) fn build(mode: &str) -> Self {
        let helper = fake::runtime_fake(mode);
        let mut random = [0; 4];
        fs::File::open("/dev/urandom")
            .unwrap()
            .read_exact(&mut random)
            .unwrap();
        let prefix = format!("qf-{:08x}", u32::from_ne_bytes(random));
        let base = fs::canonicalize(std::env::temp_dir()).unwrap();
        let dir = tempfile::Builder::new()
            .prefix(&prefix)
            .rand_bytes(0)
            .tempdir_in(base)
            .unwrap();
        tree::set_mode(dir.path(), 0o700);
        for name in ["bin", "lib", "boot", "base", "policy", "cell", "markers"] {
            let path = dir.path().join(name);
            fs::create_dir(&path).unwrap();
            tree::set_mode(&path, 0o700);
        }
        let path = dir.path().join(format!("bin/{mode}"));
        let value = recipe_value(path.to_str().unwrap());
        let mut bundle = Self {
            dir,
            value,
            files: Vec::new(),
        };
        let policy = newc::POLICY;
        bundle.put(
            "helper",
            &format!("bin/{mode}"),
            &fs::read(&helper).unwrap(),
            0o555,
        );
        let library = helper
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("lib/libplasmosome-fake.dylib");
        let listed = bundle.put(
            "library",
            "lib/libplasmosome-fake.dylib",
            &fs::read(library).unwrap(),
            0o444,
        );
        bundle.value["libraries"] = json!([listed]);
        bundle.put("kernel", "boot/kernel", b"fake kernel", 0o444);
        bundle.put(
            "initramfs",
            "boot/initramfs",
            &newc::encode(&newc::bound(newc::INIT, policy)),
            0o444,
        );
        bundle.put("root_image", "base/rootfs.raw", &vec![0x5a; 1 << 20], 0o444);
        bundle.put("guest_policy", "policy/guest-policy", policy, 0o444);
        let work = format!("(subpath \"{}\")", bundle.path().join("cell").display());
        let host = host_policy::HOST_POLICY.replace(host_policy::WORK_SUBPATH, &work) + "\0";
        bundle.put("host_policy", "policy/host-policy", host.as_bytes(), 0o444);
        use std::os::unix::process::CommandExt;
        let mut child = crate::spawn_lock::spawn_guarded(
            std::process::Command::new(bundle.path().join(format!("bin/{mode}"))).arg0("warmup"),
        )
        .unwrap();
        let status = fake::owned_wait::exit_within(&mut child, fake::owned_wait::SETUP_BOUND)
            .unwrap()
            .expect("the copied helper exits within the setup bound");
        assert_eq!(status.code(), Some(64));
        let limits = PlatformLimits::host().unwrap();
        bundle.value["architecture"] = json!(limits.architecture().as_str());
        bundle.value["memory_mib"] = json!(64);
        for (key, name) in [
            ("writable_root", "root.raw"),
            ("control_path", "control.uds"),
            ("data_path", "data.uds"),
        ] {
            let path = bundle.path().join("cell").join(name);
            assert!(
                path.as_os_str().as_bytes().len() < 104,
                "socket path is too long: {}",
                path.display()
            );
            bundle.value[key] = json!(path);
        }
        bundle
    }

    fn put(&mut self, key: &str, relative: &str, bytes: &[u8], mode: u32) -> Value {
        let path = self.path().join(relative);
        fs::write(&path, bytes).unwrap();
        tree::set_mode(&path, mode);
        self.files.push(path.clone());
        tree::set_flags(&path, libc::UF_IMMUTABLE);
        let artifact = json!({"path": path, "sha256": Digest::of(bytes).hex()});
        if key != "library" {
            self.value[key] = artifact.clone();
        }
        artifact
    }

    /// The bundle's canonical root, including its private `cell` and `markers` directories.
    pub(crate) fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Parses the bundle's recipe with actual host limits and pinned artifact digests.
    pub(crate) fn recipe(&self) -> RuntimeRecipe {
        recipe(&self.value)
    }

    /// Changes only the selected pin. The artifact's bytes and protection remain intact.
    pub(crate) fn with_tampered_digest(mut self, role: ArtifactRole) -> Self {
        let artifact = match role {
            ArtifactRole::Kernel => &mut self.value["kernel"],
            ArtifactRole::Initramfs => &mut self.value["initramfs"],
            ArtifactRole::RootImage => &mut self.value["root_image"],
            ArtifactRole::Helper => &mut self.value["helper"],
            ArtifactRole::Library(index) => &mut self.value["libraries"][index],
            ArtifactRole::HostPolicy => &mut self.value["host_policy"],
            ArtifactRole::GuestPolicy => &mut self.value["guest_policy"],
        };
        let digest = artifact["sha256"].as_str().unwrap();
        let changed = format!(
            "{}{}",
            if digest.starts_with('0') { '1' } else { '0' },
            &digest[1..]
        );
        artifact["sha256"] = json!(changed);
        self
    }

    /// Replaces the archive with a correctly pinned archive holding another guest policy.
    pub(crate) fn with_policy_entry_mismatch(mut self) -> Self {
        let path = self.path().join("boot/initramfs");
        tree::set_flags(&path, 0);
        tree::set_mode(&path, 0o600);
        self.files.retain(|file| file != &path);
        self.put(
            "initramfs",
            "boot/initramfs",
            &newc::encode(&newc::bound(newc::INIT, b"different policy")),
            0o444,
        );
        self
    }

    /// Occupies the control endpoint with a regular file that bind must preserve.
    pub(crate) fn with_occupied_control_path(self) -> Self {
        fs::write(self.recipe().control_path().as_path(), b"occupied").unwrap();
        self
    }
}

impl Drop for FixtureBundle {
    fn drop(&mut self) {
        for path in &self.files {
            tree::set_flags(path, 0);
        }
    }
}

#[path = "bundle_tests.rs"]
mod tests;
