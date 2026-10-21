#![cfg(target_os = "macos")]

use std::ffi::{CString, OsStr};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use plasmosome_membrane::runtime::artifacts::{ArtifactFault, verify_recipe_artifacts};
use plasmosome_membrane::runtime::digest::Digest;
use plasmosome_membrane::runtime::recipe::{ArtifactRole, PlatformLimits, RuntimeRecipe};

const ARTIFACTS: [(&str, &str); 7] = [
    ("kernel", "kernel"),
    ("initramfs", "initramfs"),
    ("root_image", "rootfs.raw"),
    ("helper", "krun-helper"),
    ("libraries", "libkrun.dylib"),
    ("host_policy", "host-policy"),
    ("guest_policy", "guest-policy"),
];

struct Bundle {
    paths: Vec<PathBuf>,
    _dir: tempfile::TempDir,
}

impl Bundle {
    fn new() -> Bundle {
        let dir = tempfile::Builder::new()
            .prefix("aep")
            .tempdir_in(user_temp_dir())
            .expect("a bundle directory");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).expect("chmod");
        let paths = ARTIFACTS
            .iter()
            .map(|(_, name)| {
                let path = dir.path().join(name);
                fs::write(&path, name.as_bytes()).expect("write");
                fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).expect("chmod");
                assert_eq!(chflags(&path, libc::UF_IMMUTABLE), 0, "chflags uchg");
                path
            })
            .collect();
        Bundle { paths, _dir: dir }
    }

    fn recipe(&self, tampered: Option<&str>) -> RuntimeRecipe {
        let mut value = serde_json::json!({
            "version": 1,
            "vcpus": 1,
            "memory_mib": 512,
            "kernel_format": if std::env::consts::ARCH == "x86_64" { "elf" } else { "raw" },
            "writable_root": "/var/cell/c1/root.raw",
            "control_path": "/var/cell/c1/control.sock",
            "data_path": "/var/cell/c1/data.sock",
            "architecture": std::env::consts::ARCH,
        });
        for ((field, name), path) in ARTIFACTS.iter().zip(&self.paths) {
            let pinned = if tampered == Some(*field) {
                Digest::of(b"tampered")
            } else {
                Digest::of(name.as_bytes())
            };
            let artifact = serde_json::json!({
                "path": path.to_str().expect("UTF-8"),
                "sha256": pinned.hex(),
            });
            value[*field] = if *field == "libraries" {
                serde_json::json!([artifact])
            } else {
                artifact
            };
        }
        let limits = PlatformLimits::host().expect("this host reports its limits");
        RuntimeRecipe::parse(format!("{value}\n").as_bytes(), &limits)
            .unwrap_or_else(|refusal| panic!("the test recipe is valid on this host: {refusal}"))
    }
}

impl Drop for Bundle {
    fn drop(&mut self) {
        for path in &self.paths {
            chflags(path, 0);
        }
    }
}

#[allow(unsafe_code)]
fn user_temp_dir() -> PathBuf {
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
    fs::canonicalize(OsStr::from_bytes(&buffer)).expect("the per-user temp directory resolves")
}

#[allow(unsafe_code)]
fn chflags(path: &Path, flags: libc::c_uint) -> libc::c_int {
    let name = CString::new(path.as_os_str().as_bytes()).expect("no NUL");
    // SAFETY: `name` is a CString that outlives the call, and chflags keeps no pointer to it.
    unsafe { libc::chflags(name.as_ptr(), flags) }
}

fn open_descriptors() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir("/dev/fd")
        .expect("/dev/fd lists this process's descriptors")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

#[test]
fn a_refused_recipe_and_a_dropped_one_leave_every_descriptor_as_it_was() {
    let bundle = Bundle::new();
    let (tampered, intact) = (bundle.recipe(Some("root_image")), bundle.recipe(None));
    let before = open_descriptors();
    let refusal = verify_recipe_artifacts(&tampered).expect_err("the root image is tampered");
    assert_eq!(refusal.role, ArtifactRole::RootImage);
    assert_eq!(
        refusal.fault,
        ArtifactFault::DigestMismatch {
            expected: Digest::of(b"tampered"),
            actual: Digest::of(b"rootfs.raw"),
        }
    );
    assert_eq!(open_descriptors(), before, "after the refusal");
    let verified = verify_recipe_artifacts(&intact).expect("every artifact verifies");
    let held = open_descriptors().len().saturating_sub(before.len());
    assert!(held >= ARTIFACTS.len(), "{held} descriptors held");
    drop(verified);
    assert_eq!(open_descriptors(), before, "after the drop");
}
