mod diagnostics;
#[cfg(target_os = "macos")]
mod libkrun;
#[cfg(target_os = "macos")]
mod loader;
#[cfg(target_os = "macos")]
mod pre_native;

use plasmosome_membrane::runtime::vmm_helper::{HelperExit, HelperRefusal};
use std::io::Write;

fn diagnostic(message: impl std::fmt::Display) {
    let _ = std::io::stderr().write_all(diagnostics::line(message).as_bytes());
}

fn main() {
    let refusal = run().unwrap_err();
    diagnostic(&refusal);
    std::process::exit(refusal.exit.code());
}

#[cfg(not(target_os = "macos"))]
fn run() -> Result<std::convert::Infallible, HelperRefusal> {
    Err(HelperRefusal {
        exit: HelperExit::UnsupportedHost,
        detail: "the VMM helper requires macOS".into(),
    })
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn run() -> Result<std::convert::Infallible, HelperRefusal> {
    use plasmosome_membrane::runtime::{artifacts::verify_recipe_artifacts, vmm_helper::*};
    use std::fs::File;
    use std::os::fd::FromRawFd;
    use std::os::unix::fs::MetadataExt;

    let refuse = |exit, error: &dyn std::fmt::Display| HelperRefusal {
        exit,
        detail: error.to_string(),
    };
    let entry_environment = exec_environment();
    let descriptors = observe_descriptors().map_err(|e| refuse(HelperExit::Descriptors, &e))?;
    let null = std::fs::metadata("/dev/null").map_err(|e| refuse(HelperExit::Descriptors, &e))?;
    judge_descriptors(&descriptors, null.rdev())?;
    diagnostic("descriptors ok");
    judge_invocation(&std::env::args_os().collect::<Vec<_>>(), entry_environment?)?;
    diagnostic("invocation ok");
    loader::refuse_loader_inputs()?;
    // SAFETY: the descriptor judge established that 3 is the inherited recipe pipe. This is
    // its only owner, and close consumes that ownership before any later descriptor is opened.
    let source = unsafe { File::from_raw_fd(3) };
    let bytes = read_recipe(&source);
    let closed = close_recipe(source);
    let bytes = bytes?;
    closed?;
    let limits = judge_host_limits(plasmosome_membrane::runtime::recipe::PlatformLimits::host())?;
    let recipe = parse_recipe(&bytes, &limits)?;
    diagnostic("recipe ok");
    let executable = std::env::current_exe().map_err(|e| refuse(HelperExit::Verify, &e))?;
    judge_self(&executable, &recipe)?;
    let verified = verify_recipe_artifacts(&recipe).map_err(|e| refuse(HelperExit::Verify, &e))?;
    diagnostic("artifacts ok");
    let (_root, facts) = writable_root_facts(recipe.writable_root())?;
    // SAFETY: geteuid has no preconditions and keeps no pointers.
    judge_writable_root(&facts, unsafe { libc::geteuid() })?;
    let policy = read_host_policy(verified.host_policy.file())?;
    apply_host_policy(&policy)?;
    probe_denials()?;
    diagnostic("host_policy ok");
    pre_native::finish(
        || {
            for artifact in [
                &verified.kernel,
                &verified.initramfs,
                &verified.root_image,
                &verified.helper,
            ]
            .into_iter()
            .chain(&verified.libraries)
            .chain([&verified.host_policy, &verified.guest_policy])
            {
                artifact
                    .reconfirm()
                    .map_err(|e| refuse(HelperExit::Verify, &e))?;
            }
            reconfirm_writable_root(recipe.writable_root(), &facts)
        },
        loader::refuse_loader_inputs,
        || {
            // SAFETY: before any libkrun call only this helper thread runs. otool shows no
            // __mod_init_func or __init_offsets in either pinned libkrun or this linked helper.
            // Clean sealed system libraries are the platform boundary. No firmware
            // leaf-name load happens before create, so O-4/O-6's later planting window is irrelevant here.
            let images = unsafe { loaded_images() };
            judge_loaded_images(&images, &recipe)?;
            diagnostic("inventory ok");
            Ok(())
        },
        || {
            run_plan(
                &krun_plan(&recipe),
                |step, context| {
                    diagnostic(format_args!("calling {}", step.name()));
                    libkrun::call(step, context)
                },
                || {
                    // SAFETY: the helper starts no thread before start_enter. Pinned libkrun 1.19.4
                    // init_log/create start none, import no dlclose, and retain firmware in a static.
                    // This holds provided no foreign image entered between the leaf check and create.
                    // A Homebrew-rpath writer has the helper's UID (O-4); the cryptex parent may admit any
                    // local user (unmeasured O-6, outside O-4). Such an image's initializers already ran
                    // with the helper's authority; the unload race adds none. Main accepts that proviso
                    // under both gates. The inventory still refuses the foreign image after the fact.
                    judge_loaded_images(&unsafe { loaded_images() }, &recipe)?;
                    diagnostic("inventory after_create ok");
                    Ok(())
                },
            )
        },
    )
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn exec_environment() -> Result<bool, HelperRefusal> {
    let refuse = |detail: &str| HelperRefusal {
        exit: HelperExit::Invocation,
        detail: detail.into(),
    };
    // SAFETY: Darwin's kernel exec layout puts envp after argv[argc]'s NULL. _NSGetArgc
    // and _NSGetArgv expose that original vector on the main thread's initial stack, which
    // lives for this process. The helper never writes it. Check the count and argv terminator
    // before reading envp[0]; only its first entry is inspected. Kernel strings end in NUL.
    // System initializers may replace the live environ, but not this original stack vector.
    unsafe {
        let argc = libc::_NSGetArgc();
        let argv = libc::_NSGetArgv();
        if argc.is_null() || argv.is_null() || *argc < 0 || (*argv).is_null() {
            return Err(refuse("the original exec argument vector is unavailable"));
        }
        let count = *argc as usize;
        let argv = *argv;
        if !(*argv.add(count)).is_null() {
            return Err(refuse("the original argv has no terminator at argc"));
        }
        let first = *argv.add(count + 1);
        if first.is_null() {
            return Ok(true);
        }
        let bytes = std::ffi::CStr::from_ptr(first).to_bytes();
        let name = bytes.split(|byte| *byte == b'=').next().unwrap_or_default();
        Err(HelperRefusal {
            exit: HelperExit::Invocation,
            detail: format!(
                "exec environment contains {:?}; it must be empty",
                String::from_utf8_lossy(name)
            ),
        })
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn close_recipe(source: std::fs::File) -> Result<(), HelperRefusal> {
    use std::os::fd::IntoRawFd;
    // SAFETY: into_raw_fd transfers the only owner, and close is never retried, even on EINTR.
    if unsafe { libc::close(source.into_raw_fd()) } == 0 {
        Ok(())
    } else {
        Err(HelperRefusal {
            exit: HelperExit::Recipe,
            detail: std::io::Error::last_os_error().to_string(),
        })
    }
}
