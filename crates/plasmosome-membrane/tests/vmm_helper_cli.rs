#![cfg(all(target_os = "macos", feature = "vmm-helper"))]

use std::ffi::CString;
use std::fs;
use std::io::{Read, Seek, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

struct Layout {
    binary: PathBuf,
    _dir: tempfile::TempDir,
}

static LAYOUT: Mutex<Weak<Layout>> = Mutex::new(Weak::new());

fn layout() -> Arc<Layout> {
    layout_from(&LAYOUT)
}

fn layout_from(cache: &Mutex<Weak<Layout>>) -> Arc<Layout> {
    let mut cached = cache.lock().unwrap();
    if let Some(layout) = cached.upgrade() {
        return layout;
    }
    let layout = Arc::new(build_layout());
    *cached = Arc::downgrade(&layout);
    layout
}

fn build_layout() -> Layout {
    let dir = tempfile::Builder::new()
        .prefix("plasmosome-vmm-")
        .tempdir()
        .unwrap();
    let bin = dir.path().join("bin");
    let lib = dir.path().join("lib");
    fs::create_dir(&bin).unwrap();
    fs::create_dir(&lib).unwrap();
    let binary = fs::canonicalize(bin).unwrap().join("plasmosome-vmm");
    fs::copy(env!("CARGO_BIN_EXE_plasmosome-vmm"), &binary).unwrap();
    let library =
        fs::read(Path::new(env!("PLASMOSOME_LIBKRUN_DIR_AT_BUILD")).join("libkrun.1.19.4.dylib"))
            .unwrap();
    fs::write(lib.join("libkrun.1.19.4.dylib"), library).unwrap();
    let mut child = Command::new(&binary)
        .env_clear()
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    assert_eq!(wait(&mut child, Duration::from_secs(60)).code(), Some(70));
    Layout { binary, _dir: dir }
}

fn wait(child: &mut Child, budget: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(status) = child.try_wait().unwrap_or_else(|error| {
            let _ = child.kill();
            let _ = child.wait();
            panic!("owned helper wait failed: {error}");
        }) {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("owned helper exceeded {budget:?}; killed and reaped");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[derive(Clone, Copy, Default)]
enum Descriptors {
    #[default]
    Valid,
    Extra,
    MissingRecipe,
    RegularRecipe,
    RegularStderr,
    PipeStdin,
    PipeStdout,
}

#[allow(unsafe_code)]
fn refusal(
    bytes: Vec<u8>,
    args: &[&str],
    environment: Option<&'static std::ffi::CStr>,
    descriptors: Descriptors,
    cwd: &Path,
    code: i32,
    stage: &str,
) -> String {
    let layout = layout();
    let binary = &layout.binary;
    // SAFETY: getdtablesize takes no arguments. This test process never lowers its fd limit.
    let close_limit = unsafe { libc::getdtablesize() };
    assert!(close_limit > 4);
    let program = CString::new(binary.as_os_str().as_bytes()).unwrap();
    let argv: Vec<_> = std::iter::once(program.clone())
        .chain(args.iter().map(|arg| CString::new(*arg).unwrap()))
        .collect();
    assert!(argv.len() < 5);
    let (reader, mut writer) = std::io::pipe().unwrap();
    let mut regular = tempfile::tempfile().unwrap();
    regular.write_all(&bytes).unwrap();
    regular.rewind().unwrap();
    let source = if matches!(descriptors, Descriptors::RegularRecipe) {
        regular.as_raw_fd()
    } else {
        reader.as_raw_fd()
    };
    let mut diagnostics = tempfile::tempfile().unwrap();
    let mut command = Command::new(binary);
    command
        .args(args)
        .env_clear()
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if matches!(descriptors, Descriptors::RegularStderr) {
        command.stderr(diagnostics.try_clone().unwrap());
    }
    if matches!(descriptors, Descriptors::PipeStdin) {
        command.stdin(Stdio::piped());
    }
    if matches!(descriptors, Descriptors::PipeStdout) {
        command.stdout(Stdio::piped());
    }
    // SAFETY: only async-signal-safe dup2, fcntl, close and execve run here. Strings were
    // allocated before fork and stay live; stack arrays are terminated. Explicit execve passes
    // exactly the supplied host environment. Source stays open through spawn; 3 and 4 are reserved.
    unsafe {
        command.pre_exec(move || {
            if matches!(descriptors, Descriptors::MissingRecipe) {
                libc::close(3);
            } else if libc::dup2(source, 3) < 0 || libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if matches!(descriptors, Descriptors::Extra) && libc::dup2(3, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let first_closed = if matches!(descriptors, Descriptors::Extra) {
                5
            } else {
                4
            };
            for fd in first_closed..close_limit {
                libc::close(fd);
            }
            let mut pointers = [std::ptr::null(); 5];
            for (index, arg) in argv.iter().enumerate() {
                pointers[index] = arg.as_ptr();
            }
            let env = [
                environment.map_or(std::ptr::null(), |variable| variable.as_ptr()),
                std::ptr::null(),
            ];
            libc::execve(program.as_ptr(), pointers.as_ptr(), env.as_ptr());
            Err(std::io::Error::last_os_error())
        });
    }
    let mut child = command.spawn().unwrap();
    drop(command);
    drop(reader);
    drop(regular);
    let output = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.read_to_end(&mut bytes).unwrap();
            bytes
        })
    });
    let written = std::thread::spawn(move || {
        let _ = writer.write_all(&bytes);
    });
    let status = wait(&mut child, Duration::from_secs(5));
    written.join().unwrap();
    let bytes = if let Some(output) = output {
        output.join().unwrap()
    } else {
        diagnostics.rewind().unwrap();
        let mut bytes = Vec::new();
        diagnostics.read_to_end(&mut bytes).unwrap();
        bytes
    };
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(status.code(), Some(code), "{text}");
    let refused: Vec<_> = text
        .lines()
        .filter(|line| line.contains("refused "))
        .collect();
    assert_eq!(refused.len(), 1, "{text}");
    assert!(
        refused[0].starts_with(&format!("plasmosome-vmm: refused {stage} exit={code}:")),
        "{text}"
    );
    assert!(!text.contains("calling krun_"), "{text}");
    assert!(text.lines().all(|line| line.len() < 1024), "{text}");
    text
}

fn recipe(helper: &Path) -> serde_json::Value {
    let artifact = |path: &str| serde_json::json!({"path": path, "sha256": "1".repeat(64)});
    serde_json::json!({
        "version": 1, "vcpus": 1, "memory_mib": 512, "architecture": std::env::consts::ARCH,
        "kernel_format": if cfg!(target_arch = "x86_64") {"elf"} else {"raw"},
        "kernel": artifact("/nonexistent/kernel"), "initramfs": artifact("/nonexistent/initramfs"),
        "root_image": artifact("/nonexistent/root"), "helper": artifact(helper.to_str().unwrap()),
        "libraries": [artifact("/nonexistent/libkrun.dylib")], "host_policy": artifact("/nonexistent/host"),
        "guest_policy": artifact("/nonexistent/guest"), "writable_root": "/nonexistent/write",
        "control_path": "/nonexistent/control", "data_path": "/nonexistent/data"
    })
}

#[test]
fn argv_and_any_environment_refuse_before_native_calls() {
    let _layout = layout();
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec![],
        vec!["--runtime-fd=4"],
        vec!["--runtime-fd=3", "extra"],
    ] {
        refusal(
            vec![],
            &args,
            None,
            Descriptors::Valid,
            dir.path(),
            64,
            "invocation",
        );
    }
    for variable in [
        c"PLASMOSOME_FORBIDDEN=not-a-diagnostic-value",
        c"__CF_USER_TEXT_ENCODING=not-a-diagnostic-value",
    ] {
        let text = refusal(
            vec![],
            &["--runtime-fd=3"],
            Some(variable),
            Descriptors::Valid,
            dir.path(),
            64,
            "invocation",
        );
        let name = variable.to_str().unwrap().split('=').next().unwrap();
        assert!(text.contains(name));
        assert!(!text.contains("not-a-diagnostic-value"));
    }
}

#[test]
fn every_descriptor_rule_is_checked_in_the_real_process() {
    let _layout = layout();
    let dir = tempfile::tempdir().unwrap();
    for descriptors in [
        Descriptors::Extra,
        Descriptors::MissingRecipe,
        Descriptors::RegularRecipe,
        Descriptors::RegularStderr,
        Descriptors::PipeStdin,
        Descriptors::PipeStdout,
    ] {
        refusal(
            b"{}\n".to_vec(),
            &["--runtime-fd=3"],
            None,
            descriptors,
            dir.path(),
            70,
            "descriptors",
        );
    }
}

#[test]
fn malformed_multiple_and_unbounded_recipes_refuse() {
    let layout = layout();
    let dir = tempfile::tempdir().unwrap();
    for bytes in [b"{\n".to_vec(), b"{}\n{}\n".to_vec(), vec![b' '; 1_048_578]] {
        refusal(
            bytes,
            &["--runtime-fd=3"],
            None,
            Descriptors::Valid,
            dir.path(),
            65,
            "recipe",
        );
    }
    let mut value = recipe(&layout.binary);
    value["architecture"] = "x86_64".into();
    value["kernel_format"] = "raw".into();
    refusal(
        format!("{value}\n").into_bytes(),
        &["--runtime-fd=3"],
        None,
        Descriptors::Valid,
        dir.path(),
        86,
        "boot-pair",
    );
}

#[test]
fn another_helper_and_unverified_artifacts_refuse() {
    let layout = layout();
    let dir = tempfile::tempdir().unwrap();
    for path in [Path::new("/nonexistent/helper"), &layout.binary] {
        let text = refusal(
            format!("{}\n", recipe(path)).into_bytes(),
            &["--runtime-fd=3"],
            None,
            Descriptors::Valid,
            dir.path(),
            66,
            "verify",
        );
        assert!(text.contains(if path == layout.binary {
            "kernel"
        } else {
            "this process runs"
        }));
    }
}

#[test]
fn planted_file_and_dangling_link_refuse_without_loading_them() {
    let _layout = layout();
    for dangling in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let planted = dir.path().join("libkrunfw.5.dylib");
        if dangling {
            symlink("missing-target", &planted).unwrap();
        } else {
            fs::write(&planted, b"never executable").unwrap();
        }
        let text = refusal(
            vec![],
            &["--runtime-fd=3"],
            None,
            Descriptors::Valid,
            dir.path(),
            68,
            "inventory",
        );
        assert!(text.contains("libkrunfw leaf search"));
    }
}

#[test]
fn a_long_refusal_is_one_line_within_one_kib() {
    let layout = layout();
    let dir = tempfile::tempdir().unwrap();
    let mut value = recipe(&layout.binary);
    let key = format!("{}\nsecond-line", "é".repeat(1100));
    value[key] = serde_json::Value::Null;
    refusal(
        format!("{value}\n").into_bytes(),
        &["--runtime-fd=3"],
        None,
        Descriptors::Valid,
        dir.path(),
        65,
        "recipe",
    );
}

#[test]
fn the_last_layout_owner_removes_the_copied_binary_and_library() {
    let cache = Mutex::new(Weak::new());
    let first = layout_from(&cache);
    let root = first._dir.path().to_path_buf();
    let second = layout_from(&cache);
    assert!(Arc::ptr_eq(&first, &second));
    drop(first);
    assert!(root.exists());
    drop(second);
    let removed = !root.exists();
    if !removed {
        fs::remove_dir_all(&root).unwrap();
    }
    assert!(removed, "the copied layout outlived its last owner");
    assert!(cache.lock().unwrap().upgrade().is_none());
}
