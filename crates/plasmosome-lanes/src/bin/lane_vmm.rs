use std::process::ExitCode;

#[cfg(not(target_os = "macos"))]
fn main() -> ExitCode {
    eprintln!("lane-vmm: refused: runs only on macOS");
    ExitCode::from(2)
}

#[cfg(target_os = "macos")]
fn main() -> ExitCode {
    match macos::run() {
        Ok(never) => match never {},
        Err((code, message)) => {
            eprintln!("lane-vmm: {message}");
            ExitCode::from(code)
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::convert::Infallible;
    use std::io::{Read, Write};
    use std::os::unix::process::parent_id;
    use std::path::PathBuf;
    use std::time::Duration;

    use plasmosome_lanes::bundle::{
        KERNEL, LIBKRUN, PINNED_BUNDLE_ID, is_inside, unverified_images, verify_bundle,
    };
    use plasmosome_lanes::config::{Confinement, LaneConfig};
    use plasmosome_lanes::dyld::{LIBKRUNFW, dyld_variables, libkrunfw_search, planted};
    use plasmosome_lanes::krun::{self, Libkrun};
    use plasmosome_lanes::selfcheck::{Check, Phase, passed, self_check};

    const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

    const KRUN_FAILED: u8 = 1;
    const CONFIG_REFUSED: u8 = 64;
    const BUNDLE_REFUSED: u8 = 65;
    const LOAD_REFUSED: u8 = 69;
    const SELF_CHECK_FAILED: u8 = 70;
    const CONFINEMENT_FAILED: u8 = 71;
    const PARENT_GONE: u8 = 75;

    const PARENT_POLL: Duration = Duration::from_millis(100);

    type Refusal = (u8, String);

    fn fail<T>(code: u8, message: impl std::fmt::Display) -> Result<T, Refusal> {
        Err((code, message.to_string()))
    }

    fn read_config() -> Result<LaneConfig, Refusal> {
        let mut text = String::new();
        std::io::stdin()
            .take(MAX_CONFIG_BYTES)
            .read_to_string(&mut text)
            .or_else(|error| fail(CONFIG_REFUSED, format!("refused: stdin: {error}")))?;
        LaneConfig::from_json(&text)
            .or_else(|error| fail(CONFIG_REFUSED, format!("refused: {error}")))
    }

    fn report(label: &str, checks: &[Check]) {
        for check in checks {
            eprintln!(
                "lane-vmm: {label} {}: expected {}, observed {}, {}",
                check.name,
                if check.expected_allowed {
                    "allowed"
                } else {
                    "refused"
                },
                match &check.observed {
                    Ok(()) => "allowed".to_string(),
                    Err(errno) => errno.clone(),
                },
                if check.holds() { "holds" } else { "FAILS" }
            );
        }
    }

    fn confine(confinement: &Confinement) -> Result<(), Refusal> {
        let profile = std::fs::read_to_string(&confinement.profile).or_else(|error| {
            fail(
                CONFINEMENT_FAILED,
                format!("{}: {error}", confinement.profile.display()),
            )
        })?;
        let control = self_check(
            Phase::Unconfined,
            &confinement.allowed,
            &confinement.outside,
        );
        report("control", &control);
        if !passed(Phase::Unconfined, &control) {
            return fail(
                SELF_CHECK_FAILED,
                "control failed: the directories refuse without a sandbox; the VM was not started",
            );
        }
        krun::confine(&profile).or_else(|error| fail(CONFINEMENT_FAILED, error))?;
        let checks = self_check(Phase::Confined, &confinement.allowed, &confinement.outside);
        report("self-check", &checks);
        if !passed(Phase::Confined, &checks) {
            return fail(
                SELF_CHECK_FAILED,
                "self-check failed; the VM was not started",
            );
        }
        Ok(())
    }

    fn exit_when_the_parent_is_gone() {
        let parent = parent_id();
        std::thread::spawn(move || {
            loop {
                let now = parent_id();
                if now != parent || now == 1 {
                    let _ = writeln!(
                        std::io::stderr(),
                        "lane-vmm: the process that started it is gone (parent {parent}, now {now}); exiting"
                    );
                    std::process::exit(PARENT_GONE.into());
                }
                std::thread::sleep(PARENT_POLL);
            }
        });
    }

    #[allow(unsafe_code)]
    fn images() -> Vec<PathBuf> {
        // SAFETY: until krun_start_enter, lane-vmm's only other thread is the parent watch. It
        // reads its parent's pid, sleeps, writes to stderr and exits, and none of that unloads
        // an image. libkrun v1.19.4 starts no thread before then and keeps libkrunfw open in a
        // static for good, so no image is unloaded during the call.
        unsafe { krun::loaded_images() }
    }

    fn refuse_loader_inputs() -> Result<(), Refusal> {
        let variables = dyld_variables(std::env::vars_os());
        if !variables.is_empty() {
            return fail(
                LOAD_REFUSED,
                format!(
                    "refused: inherited dyld variables {variables:?}; start lane-vmm with an empty environment"
                ),
            );
        }
        let cwd = std::env::current_dir().or_else(|error| {
            fail(
                LOAD_REFUSED,
                format!("refused: no working directory: {error}"),
            )
        })?;
        let found = planted(&libkrunfw_search(&cwd));
        if !found.is_empty() {
            return fail(
                LOAD_REFUSED,
                format!(
                    "refused: libkrun would load {LIBKRUNFW} by leaf name, and these are on its search list: {}",
                    found
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
        Ok(())
    }

    pub fn run() -> Result<Infallible, Refusal> {
        exit_when_the_parent_is_gone();
        let config = read_config()?;
        refuse_loader_inputs()?;
        if is_inside(&config.bundle, &config.root_image) {
            return fail(
                CONFIG_REFUSED,
                format!(
                    "refused: root_image {} is inside the bundle, and the root disk is writable; give a copy",
                    config.root_image.display()
                ),
            );
        }
        let bundle = verify_bundle(&config.bundle, PINNED_BUNDLE_ID)
            .or_else(|error| fail(BUNDLE_REFUSED, format!("refused: {error}")))?;
        eprintln!(
            "lane-vmm: bundle {} matches its SHA256SUMS",
            config.bundle.display()
        );
        let kernel = bundle
            .path(KERNEL)
            .ok_or((BUNDLE_REFUSED, format!("refused: {KERNEL} is not listed")))?;
        let lib = Libkrun::open(&bundle).or_else(|error| fail(LOAD_REFUSED, error))?;
        let krun = |error: krun::KrunError| (KRUN_FAILED, error.to_string());
        lib.init_log().map_err(krun)?;
        let ctx = lib.create_ctx().map_err(krun)?;
        let images = images();
        let executable = std::env::current_exe().or_else(|error| {
            fail(
                LOAD_REFUSED,
                format!("refused: no executable path: {error}"),
            )
        })?;
        let unverified = unverified_images(&images, &executable, lib.path(), krun::in_shared_cache);
        if !unverified.is_empty() {
            return fail(
                LOAD_REFUSED,
                format!(
                    "refused: images outside the bundle and the system were loaded: {unverified:?}"
                ),
            );
        }
        eprintln!(
            "lane-vmm: {} images loaded, each the executable, {LIBKRUN}, a shared-cache image or under /usr/lib",
            images.len()
        );
        ctx.set_vm_config(config.vcpus, config.memory_mib)
            .map_err(krun)?;
        ctx.set_kernel(&kernel, &config.initramfs).map_err(krun)?;
        ctx.add_root_disk(&config.root_image, false).map_err(krun)?;
        ctx.disable_implicit_init().map_err(krun)?;
        ctx.disable_implicit_vsock().map_err(krun)?;
        ctx.add_vsock(0).map_err(krun)?;
        for share in &config.shares {
            ctx.add_virtiofs4(
                &share.tag,
                &share.path,
                share.shm_size,
                share.read_only,
                share.semantics.krun_value(),
            )
            .map_err(krun)?;
        }
        ctx.set_console_output(&config.console).map_err(krun)?;
        if let Some(confinement) = &config.confinement {
            confine(confinement)?;
        }
        eprintln!("lane-vmm: starting run {}", config.run_id);
        Err(krun(ctx.start_enter()))
    }
}
