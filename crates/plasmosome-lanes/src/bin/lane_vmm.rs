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
    use std::io::Read;

    use plasmosome_lanes::bundle::{KERNEL, LIBKRUN, unverified_images, verify_bundle};
    use plasmosome_lanes::config::{Confinement, LaneConfig};
    use plasmosome_lanes::krun::{self, Libkrun};
    use plasmosome_lanes::selfcheck::{passed, self_check};

    const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

    const KRUN_FAILED: u8 = 1;
    const CONFIG_REFUSED: u8 = 64;
    const BUNDLE_REFUSED: u8 = 65;
    const LOAD_REFUSED: u8 = 69;
    const SELF_CHECK_FAILED: u8 = 70;
    const CONFINEMENT_FAILED: u8 = 71;

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

    fn confine(confinement: &Confinement) -> Result<(), Refusal> {
        let profile = std::fs::read_to_string(&confinement.profile).or_else(|error| {
            fail(
                CONFINEMENT_FAILED,
                format!("{}: {error}", confinement.profile.display()),
            )
        })?;
        krun::confine(&profile).or_else(|error| fail(CONFINEMENT_FAILED, error))?;
        let checks = self_check(&confinement.allowed, &confinement.outside);
        for check in &checks {
            eprintln!(
                "lane-vmm: self-check {}: expected {}, observed {}, {}",
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
        if !passed(&checks) {
            return fail(
                SELF_CHECK_FAILED,
                "self-check failed; the VM was not started",
            );
        }
        Ok(())
    }

    pub fn run() -> Result<Infallible, Refusal> {
        let config = read_config()?;
        let bundle = verify_bundle(&config.bundle)
            .or_else(|error| fail(BUNDLE_REFUSED, format!("refused: {error}")))?;
        eprintln!(
            "lane-vmm: bundle {} matches its SHA256SUMS",
            config.bundle.display()
        );
        let libkrun_path = bundle
            .path(LIBKRUN)
            .ok_or((BUNDLE_REFUSED, format!("refused: {LIBKRUN} is not listed")))?;
        let kernel = bundle
            .path(KERNEL)
            .ok_or((BUNDLE_REFUSED, format!("refused: {KERNEL} is not listed")))?;
        let lib = Libkrun::open(&libkrun_path).or_else(|error| fail(LOAD_REFUSED, error))?;
        let krun = |error: krun::KrunError| (KRUN_FAILED, error.to_string());
        lib.init_log().map_err(krun)?;
        let ctx = lib.create_ctx().map_err(krun)?;
        let images = krun::loaded_images();
        let unverified = unverified_images(&images, lib.path());
        if !unverified.is_empty() {
            return fail(
                LOAD_REFUSED,
                format!(
                    "refused: images outside the bundle and the system were loaded: {unverified:?}"
                ),
            );
        }
        eprintln!(
            "lane-vmm: {} images loaded, each the executable, {LIBKRUN} or a system image",
            images.len()
        );
        ctx.set_vm_config(config.vcpus, config.memory_mib)
            .map_err(krun)?;
        ctx.set_kernel(&kernel, &config.initramfs).map_err(krun)?;
        ctx.add_root_disk(&config.root_image, true).map_err(krun)?;
        ctx.disable_implicit_init().map_err(krun)?;
        ctx.disable_implicit_vsock().map_err(krun)?;
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
