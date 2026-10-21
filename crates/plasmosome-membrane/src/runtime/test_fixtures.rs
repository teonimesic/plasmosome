use super::recipe::{PlatformLimits, RuntimeRecipe};
use serde_json::{Value, json};

pub(crate) mod fake {
    include!("../../tests/support/runtime_fake.rs");
}

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
    for base in root_bases() {
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

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn root_bases() -> Vec<std::path::PathBuf> {
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
    let base = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(&buffer));
    vec![
        base.canonicalize()
            .unwrap_or_else(|error| panic!("{} does not resolve: {error}", base.display())),
    ]
}

#[cfg(target_os = "linux")]
fn root_bases() -> Vec<std::path::PathBuf> {
    let mut bases: Vec<std::path::PathBuf> = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|base| !base.is_empty())
        .map(std::path::PathBuf::from)
        .into_iter()
        .collect();
    let binary = std::env::current_exe().expect("the test binary knows its own path");
    bases.extend(binary.parent().map(std::path::Path::to_path_buf));
    bases
}
