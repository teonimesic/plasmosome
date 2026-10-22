use std::ffi::{CStr, CString, OsString};
use std::fmt;
use std::io::Read;
use std::path::Path;

use plasmosome_core::runtime::supervisor_config::MAX_RECORD_BYTES;

#[cfg(any(target_os = "macos", test))]
use super::closure::{self, ClosureFault};
use super::recipe::{HostLimitsError, PlatformLimits, RecipeRefusal, RuntimeRecipe};

/// The one argument the helper takes after its own path.
pub const RUNTIME_FD_ARG: &str = "--runtime-fd=3";

/// The guest kernel command line, the same on both admitted architectures (spec 001 §4.2).
pub const GUEST_CMDLINE: &str = "rdinit=/init panic=-1";

/// The most bytes the helper accepts from descriptor 3: a recipe line at its bound and its
/// newline.
pub const MAX_RECIPE_BYTES: usize = MAX_RECORD_BYTES + 1;

/// The most bytes of host policy the helper accepts, its final NUL included.
pub const MAX_HOST_POLICY_BYTES: usize = 64 * 1024;

const GUEST_CMDLINE_C: &CStr = c"rdinit=/init panic=-1";
const LOG_TO_STDERR: i32 = 2;
const KRUN_LOG_LEVEL_WARN: u32 = 2;
const KRUN_LOG_STYLE_NEVER: u32 = 2;
const KRUN_LOG_OPTION_NO_ENV: u32 = 1;
const KRUN_DISK_FORMAT_RAW: u32 = 0;
const KRUN_SYNC_FULL: u32 = 2;
const CONTROL_PORT: u32 = 4090;
const DATA_PORT: u32 = 4091;

/// Why the helper stopped, as its exit code. 64 to 69 and 86 are the qualified helper's codes.
/// None is 71, spec 018's `setsid` failure, or 125 and 127, the launch child's own. 70 is also
/// the code `VmmChild` exits with when a panic escapes its child, so only the helper's
/// `refused descriptors` line tells the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperExit {
    /// The argv is not `[path, "--runtime-fd=3"]`, or the environment is not empty.
    Invocation = 64,
    /// The recipe is too long, unreadable or not one recipe record, or closing descriptor 3
    /// failed.
    Recipe = 65,
    /// The helper is not the recipe's, an artifact failed verification, or the writable root is
    /// not private.
    Verify = 66,
    /// The host policy is too long, unreadable or not one NUL-terminated profile, was not
    /// applied, or did not deny.
    HostPolicy = 67,
    /// A loaded image is not the helper, a listed library or a sealed system library.
    Inventory = 68,
    /// A libkrun call failed, or `krun_start_enter` returned.
    Krun = 69,
    /// The open descriptors are not `/dev/null`, `/dev/null`, a pipe and a pipe.
    Descriptors = 70,
    /// This host cannot run the helper: it is not macOS, or it will not report its limits.
    UnsupportedHost = 72,
    /// The recipe pairs `x86_64` with kernel format `raw`.
    UnadmittedBootPair = 86,
}

const EXITS: [HelperExit; 9] = [
    HelperExit::Invocation,
    HelperExit::Recipe,
    HelperExit::Verify,
    HelperExit::HostPolicy,
    HelperExit::Inventory,
    HelperExit::Krun,
    HelperExit::Descriptors,
    HelperExit::UnsupportedHost,
    HelperExit::UnadmittedBootPair,
];

impl HelperExit {
    /// The process exit code.
    pub fn code(self) -> i32 {
        self as i32
    }

    /// The exit whose code is `code`, or `None` for a code the helper never uses.
    pub fn from_code(code: i32) -> Option<HelperExit> {
        EXITS.into_iter().find(|exit| exit.code() == code)
    }

    /// The stage its `refused` line names, such as `descriptors`.
    pub fn stage(self) -> &'static str {
        match self {
            HelperExit::Invocation => "invocation",
            HelperExit::Recipe => "recipe",
            HelperExit::Verify => "verify",
            HelperExit::HostPolicy => "host-policy",
            HelperExit::Inventory => "inventory",
            HelperExit::Krun => "krun",
            HelperExit::Descriptors => "descriptors",
            HelperExit::UnsupportedHost => "unsupported-host",
            HelperExit::UnadmittedBootPair => "boot-pair",
        }
    }
}

/// A refusal: the exit the helper takes and what it found. It shows as
/// `refused <stage> exit=<code>: <detail>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperRefusal {
    pub exit: HelperExit,
    pub detail: String,
}

impl fmt::Display for HelperRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (stage, code) = (self.exit.stage(), self.exit.code());
        write!(f, "refused {stage} exit={code}: {}", self.detail)
    }
}

impl std::error::Error for HelperRefusal {}

fn refuse(exit: HelperExit, detail: impl Into<String>) -> HelperRefusal {
    HelperRefusal {
        exit,
        detail: detail.into(),
    }
}

/// Checks the helper's own invocation: `args`, its program name first, must be that name and
/// [`RUNTIME_FD_ARG`] alone, and `env_is_empty` must hold. Refuses `Invocation`.
pub fn judge_invocation(args: &[OsString], env_is_empty: bool) -> Result<(), HelperRefusal> {
    if !matches!(args, [_, flag] if flag == RUNTIME_FD_ARG) {
        return Err(refuse(
            HelperExit::Invocation,
            format!(
                "argv must be exactly the helper's path and {RUNTIME_FD_ARG}; it has {} arguments",
                args.len()
            ),
        ));
    }
    if !env_is_empty {
        return Err(refuse(
            HelperExit::Invocation,
            "the environment must be empty",
        ));
    }
    Ok(())
}

/// What `fstat` says a descriptor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorKind {
    CharDevice,
    Fifo,
    Socket,
    Regular,
    Directory,
    Other,
}

/// One open descriptor: its number, its kind and, for a device, its `st_rdev`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptorFacts {
    pub fd: i32,
    pub kind: DescriptorKind,
    pub rdev: u64,
}

/// Checks every descriptor the helper has open, in any order, against spec 001 §4.2: exactly 0
/// to 3, with 0 and 1 the character device whose `st_rdev` is `dev_null_rdev`, and 2 and 3 pipes.
/// Refuses `Descriptors`, naming the lowest of 0 to 3 that is wrong or missing, and only then the
/// lowest other descriptor.
pub fn judge_descriptors(
    open: &[DescriptorFacts],
    dev_null_rdev: u64,
) -> Result<(), HelperRefusal> {
    let refused = |detail: String| Err(refuse(HelperExit::Descriptors, detail));
    for fd in 0..=3 {
        let Some(facts) = open.iter().find(|facts| facts.fd == fd) else {
            return refused(format!("descriptor {fd} is not open"));
        };
        let null = facts.kind == DescriptorKind::CharDevice && facts.rdev == dev_null_rdev;
        match (fd < 2, null, facts.kind == DescriptorKind::Fifo) {
            (true, false, _) => {
                return refused(format!(
                    "descriptor {fd} is {}, not /dev/null",
                    kind_name(facts.kind)
                ));
            }
            (false, _, false) => {
                return refused(format!(
                    "descriptor {fd} is {}, not a pipe",
                    kind_name(facts.kind)
                ));
            }
            _ => {}
        }
    }
    match open
        .iter()
        .map(|facts| facts.fd)
        .filter(|fd| !(0..=3).contains(fd))
        .min()
    {
        Some(fd) => refused(format!("descriptor {fd} is open; only 0 to 3 may be")),
        None => Ok(()),
    }
}

fn kind_name(kind: DescriptorKind) -> &'static str {
    match kind {
        DescriptorKind::CharDevice => "a character device",
        DescriptorKind::Fifo => "a pipe",
        DescriptorKind::Socket => "a socket",
        DescriptorKind::Regular => "a regular file",
        DescriptorKind::Directory => "a directory",
        DescriptorKind::Other => "another kind of file",
    }
}

/// Reads `source`, descriptor 3, to its end. More than [`MAX_RECIPE_BYTES`] refuses `Recipe`
/// once one byte more has been read, so a source that never ends is not read forever. A read
/// error refuses `Recipe` too. Whether the bytes are a recipe is [`parse_recipe`]'s to say.
pub fn read_recipe(source: impl Read) -> Result<Vec<u8>, HelperRefusal> {
    read_bounded(source, MAX_RECIPE_BYTES, HelperExit::Recipe, "the recipe")
}

/// Reads the verified host policy from `source`, at most [`MAX_HOST_POLICY_BYTES`], and checks
/// it with [`judge_host_policy_bytes`]. Refuses `HostPolicy`.
pub fn read_host_policy(source: impl Read) -> Result<CString, HelperRefusal> {
    let bytes = read_bounded(
        source,
        MAX_HOST_POLICY_BYTES,
        HelperExit::HostPolicy,
        "the host policy",
    )?;
    judge_host_policy_bytes(&bytes)
}

fn read_bounded(
    source: impl Read,
    max: usize,
    exit: HelperExit,
    what: &str,
) -> Result<Vec<u8>, HelperRefusal> {
    let mut bytes = Vec::new();
    source
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| refuse(exit, format!("reading {what} failed: {error}")))?;
    if bytes.len() > max {
        return Err(refuse(exit, format!("{what} is longer than {max} bytes")));
    }
    Ok(bytes)
}

/// Takes what `PlatformLimits::host` returned. A host that would not report its limits refuses
/// `UnsupportedHost`, never `Recipe`: the recipe was not at fault.
pub fn judge_host_limits(
    limits: Result<PlatformLimits, HostLimitsError>,
) -> Result<PlatformLimits, HelperRefusal> {
    limits.map_err(|error| refuse(HelperExit::UnsupportedHost, error.to_string()))
}

/// Parses `bytes` with `RuntimeRecipe::parse` under `limits`. `UnadmittedBootPair` refuses with
/// its own exit, 86; every other refusal is `Recipe`. The detail is the refusal's text.
pub fn parse_recipe(bytes: &[u8], limits: &PlatformLimits) -> Result<RuntimeRecipe, HelperRefusal> {
    RuntimeRecipe::parse(bytes, limits).map_err(|refusal| {
        let exit = match refusal {
            RecipeRefusal::UnadmittedBootPair => HelperExit::UnadmittedBootPair,
            _ => HelperExit::Recipe,
        };
        refuse(exit, refusal.to_string())
    })
}

/// Checks that `current_exe`, the path this process was executed from, is the recipe's helper.
/// It compares path components, so `.`, a doubled `/` and a trailing `/` are folded away in the
/// helper's own path. Each such spelling names the helper, or with a trailing `/` no file at all.
/// A `..` part does not pass. Refuses `Verify`.
pub fn judge_self(current_exe: &Path, recipe: &RuntimeRecipe) -> Result<(), HelperRefusal> {
    let helper = recipe.helper().path();
    if current_exe == helper.as_path() {
        return Ok(());
    }
    Err(refuse(
        HelperExit::Verify,
        format!(
            "this process runs {current_exe:?}, not the recipe's helper {:?}",
            helper.as_str()
        ),
    ))
}

/// What `fstat` says of the writable root: whether it is a regular file, its owner, its mode,
/// its link count, whether `uchg` or `schg` is set, and its device and inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WritableRootFacts {
    pub regular: bool,
    pub uid: u32,
    pub mode: u32,
    pub nlink: u64,
    pub immutable: bool,
    pub dev: u64,
    pub ino: u64,
}

/// Checks the writable root as plasmosome-bbl creates it: a regular file owned by `euid`, with
/// permission bits exactly 0600, one link, and neither `uchg` nor `schg`. Refuses `Verify`,
/// naming the first rule it breaks. It does not compare the identity; the caller keeps that.
pub fn judge_writable_root(facts: &WritableRootFacts, euid: u32) -> Result<(), HelperRefusal> {
    let mode = facts.mode & 0o7777;
    let broken = if !facts.regular {
        "it is not a regular file".to_owned()
    } else if facts.uid != euid {
        format!(
            "it is owned by uid {}, not the effective uid {euid}",
            facts.uid
        )
    } else if mode != 0o600 {
        format!("it has mode {mode:04o}, not 0600")
    } else if facts.nlink != 1 {
        format!("it has {} links, not 1", facts.nlink)
    } else if facts.immutable {
        "it is immutable".to_owned()
    } else {
        return Ok(());
    };
    Err(refuse(
        HelperExit::Verify,
        format!("the writable root is refused: {broken}"),
    ))
}

/// Checks the host policy's bytes: nonempty UTF-8 text ending in its only NUL. Returns them as
/// the profile `sandbox_init` takes. Refuses `HostPolicy`.
pub fn judge_host_policy_bytes(bytes: &[u8]) -> Result<CString, HelperRefusal> {
    let refused =
        |detail: &str| refuse(HelperExit::HostPolicy, format!("the host policy {detail}"));
    let profile = CString::from_vec_with_nul(bytes.to_vec())
        .map_err(|_| refused("must hold exactly one NUL, as its last byte"))?;
    if profile.is_empty() {
        return Err(refused("is empty"));
    }
    profile.to_str().map_err(|_| refused("is not UTF-8"))?;
    Ok(profile)
}

/// An operation the host policy must deny, tried once the policy is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeniedProbe {
    FreshFileWrite,
    TcpConnect,
}

/// Judges one denial probe: `Ok(())` when the operation succeeded, or the errno it failed with.
/// Only `EPERM` shows the policy denied it. Success or any other errno refuses `HostPolicy`,
/// naming the probe, because a missing directory or file mode is not confinement.
pub fn judge_denial(probe: DeniedProbe, outcome: Result<(), i32>) -> Result<(), HelperRefusal> {
    let name = match probe {
        DeniedProbe::FreshFileWrite => "a fresh file write",
        DeniedProbe::TcpConnect => "a TCP connect",
    };
    let found = match outcome {
        Err(libc::EPERM) => return Ok(()),
        Ok(()) => "it succeeded".to_owned(),
        Err(errno) => format!("it failed with errno {errno}, not EPERM"),
    };
    Err(refuse(
        HelperExit::HostPolicy,
        format!("the host policy did not deny {name}: {found}"),
    ))
}

/// How many loaded images passed, and why: the helper or a listed library, or a sealed system
/// library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageCount {
    pub total: usize,
    pub system: usize,
    pub listed: usize,
}

/// Judges the images dyld has loaded into the helper, by the names `_dyld_get_image_name` gives.
/// Each must be the recipe's helper, one of its libraries, or a system name: canonical ASCII
/// under `/usr/lib/` or `/System/Library/` whose deepest existing part is on the sealed system
/// volume. A name under the helper's directory may climb out of it with leading `..` parts, as
/// dyld writes an `@executable_path/../` load; any other `..` matches nothing. Under the helper's
/// directory, `.`, a doubled `/` and a trailing `/` are folded away too. Each such name names the
/// same file, or with a trailing `/` no file at all; anywhere else they match nothing. Refuses
/// `Inventory` naming the first other image. It also refuses when no image is the helper: dyld
/// always lists the helper's own image, so a list without it is incomplete.
#[cfg(target_os = "macos")]
pub fn judge_loaded_images(
    images: &[String],
    recipe: &RuntimeRecipe,
) -> Result<ImageCount, HelperRefusal> {
    judge_images_with(images, recipe, closure::on_sealed_volume)
}

#[cfg(any(target_os = "macos", test))]
fn judge_images_with(
    images: &[String],
    recipe: &RuntimeRecipe,
    sealed: impl Fn(&str) -> Result<bool, ClosureFault>,
) -> Result<ImageCount, HelperRefusal> {
    let helper = recipe.helper().path();
    let helper_dir = closure::directory(helper);
    let mut count = ImageCount {
        total: images.len(),
        system: 0,
        listed: 0,
    };
    let mut helper_seen = false;
    for name in images {
        let resolved = resolved_name(name, helper_dir);
        helper_seen |= resolved.as_deref() == Some(helper.as_str());
        let listed = std::iter::once(helper)
            .chain(recipe.libraries().iter().map(|library| library.path()))
            .any(|path| resolved.as_deref() == Some(path.as_str()));
        if listed {
            count.listed += 1;
            continue;
        }
        let refused = |detail: String| refuse(HelperExit::Inventory, format!("{name:?} {detail}"));
        if closure::is_system_name(name) {
            if sealed(name).map_err(|fault| refused(format!("could not be judged: {fault}")))? {
                count.system += 1;
                continue;
            }
            return Err(refused(
                "is a system name off the sealed system volume".to_owned(),
            ));
        }
        return Err(refused(
            "is not the helper, a listed library or a sealed system library".to_owned(),
        ));
    }
    if !helper_seen {
        return Err(refuse(
            HelperExit::Inventory,
            format!(
                "the helper {:?} is not among the {} loaded images",
                helper.as_str(),
                images.len()
            ),
        ));
    }
    Ok(count)
}

#[cfg(any(target_os = "macos", test))]
fn resolved_name(name: &str, helper_dir: &str) -> Option<String> {
    let under = name.strip_prefix(helper_dir).and_then(|rest| {
        rest.strip_prefix('/')
            .or(helper_dir.ends_with('/').then_some(rest))
    });
    match under {
        Some(rest) if !closure::climbs_named_directory(rest) => {
            Some(closure::join(helper_dir, rest))
        }
        Some(_) => None,
        None => super::path::RecipePath::parse(name)
            .is_ok()
            .then(|| name.to_owned()),
    }
}

/// One libkrun call of the helper's plan, with the arguments spec 001 §4.2 fixes. A call after
/// `CreateCtx` also takes the context, which [`run_plan`] passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KrunCall {
    InitLog {
        target_fd: i32,
        level: u32,
        style: u32,
        options: u32,
    },
    CreateCtx,
    SetVmConfig {
        vcpus: u8,
        ram_mib: u32,
    },
    SetKernel {
        kernel: CString,
        format: u32,
        initramfs: CString,
        cmdline: CString,
    },
    AddDisk3 {
        block_id: CString,
        path: CString,
        format: u32,
        read_only: bool,
        direct_io: bool,
        sync_mode: u32,
    },
    DisableImplicitInit,
    DisableImplicitConsole,
    DisableImplicitVsock,
    AddVsock {
        tsi_features: u32,
    },
    /// `krun_set_env` with a non-null array holding only the terminating NULL.
    SetEnvEmpty,
    /// `krun_set_port_map` with a non-null array holding only the terminating NULL.
    SetPortMapEmpty,
    AddVsockPort2 {
        port: u32,
        path: CString,
        listen: bool,
    },
    StartEnter,
}

impl KrunCall {
    /// The libkrun function this call makes, such as `krun_add_disk3`.
    pub fn name(&self) -> &'static str {
        match self {
            KrunCall::InitLog { .. } => "krun_init_log",
            KrunCall::CreateCtx => "krun_create_ctx",
            KrunCall::SetVmConfig { .. } => "krun_set_vm_config",
            KrunCall::SetKernel { .. } => "krun_set_kernel",
            KrunCall::AddDisk3 { .. } => "krun_add_disk3",
            KrunCall::DisableImplicitInit => "krun_disable_implicit_init",
            KrunCall::DisableImplicitConsole => "krun_disable_implicit_console",
            KrunCall::DisableImplicitVsock => "krun_disable_implicit_vsock",
            KrunCall::AddVsock { .. } => "krun_add_vsock",
            KrunCall::SetEnvEmpty => "krun_set_env",
            KrunCall::SetPortMapEmpty => "krun_set_port_map",
            KrunCall::AddVsockPort2 { .. } => "krun_add_vsock_port2",
            KrunCall::StartEnter => "krun_start_enter",
        }
    }
}

/// The 14 libkrun calls spec 001 §4.2 fixes for `recipe`, in order: logging to descriptor 2 at
/// warn with no style and no environment; the context; the recipe's vCPUs and memory; its kernel,
/// kernel format and initramfs with [`GUEST_CMDLINE`]; one RAW read-write disk `root` on the
/// writable root, with no direct IO and full sync; implicit init, console and vsock off; vsock
/// with no TSI; an empty guest environment and port map; 4090 to the control path and 4091 to the
/// data path, neither listening; and start. Nothing can fail: every string is a parsed path or a
/// fixed text.
pub fn krun_plan(recipe: &RuntimeRecipe) -> Vec<KrunCall> {
    let text = |path: &super::path::RecipePath| path.as_c_str().to_owned();
    let port = |port, path| KrunCall::AddVsockPort2 {
        port,
        path: text(path),
        listen: false,
    };
    vec![
        KrunCall::InitLog {
            target_fd: LOG_TO_STDERR,
            level: KRUN_LOG_LEVEL_WARN,
            style: KRUN_LOG_STYLE_NEVER,
            options: KRUN_LOG_OPTION_NO_ENV,
        },
        KrunCall::CreateCtx,
        KrunCall::SetVmConfig {
            vcpus: recipe.vcpus(),
            ram_mib: recipe.memory_mib(),
        },
        KrunCall::SetKernel {
            kernel: text(recipe.kernel().path()),
            format: recipe.kernel_format().krun_value(),
            initramfs: text(recipe.initramfs().path()),
            cmdline: GUEST_CMDLINE_C.to_owned(),
        },
        KrunCall::AddDisk3 {
            block_id: c"root".to_owned(),
            path: text(recipe.writable_root()),
            format: KRUN_DISK_FORMAT_RAW,
            read_only: false,
            direct_io: false,
            sync_mode: KRUN_SYNC_FULL,
        },
        KrunCall::DisableImplicitInit,
        KrunCall::DisableImplicitConsole,
        KrunCall::DisableImplicitVsock,
        KrunCall::AddVsock { tsi_features: 0 },
        KrunCall::SetEnvEmpty,
        KrunCall::SetPortMapEmpty,
        port(CONTROL_PORT, recipe.control_path()),
        port(DATA_PORT, recipe.data_path()),
        KrunCall::StartEnter,
    ]
}

/// Makes the calls of `plan` in order through `call`, which gets the context for every call
/// after `CreateCtx` and `None` before it, and returns what libkrun returned. `after_create` runs
/// once, as soon as `CreateCtx` returns a context of 0 or more and before the next call; its
/// refusal is returned as it is. A negative context, or any other call that returns nonzero,
/// stops at once with `Krun` naming the call (and, for `krun_add_vsock_port2`, its port) and the
/// value, and no later call is made. `krun_start_enter` returns only on error, so whatever it
/// returns refuses `Krun`, and so does a plan that ends without it: there is no success to
/// return.
pub fn run_plan(
    plan: &[KrunCall],
    mut call: impl FnMut(&KrunCall, Option<u32>) -> i32,
    mut after_create: impl FnMut() -> Result<(), HelperRefusal>,
) -> HelperRefusal {
    let mut context = None;
    for step in plan {
        let value = call(step, context);
        let failed = || {
            let name = match step {
                KrunCall::AddVsockPort2 { port, .. } => format!("{}({port})", step.name()),
                _ => step.name().to_owned(),
            };
            refuse(HelperExit::Krun, format!("{name} returned {value}"))
        };
        match step {
            KrunCall::CreateCtx => {
                let Ok(created) = u32::try_from(value) else {
                    return failed();
                };
                context = Some(created);
                if let Err(refusal) = after_create() {
                    return refusal;
                }
            }
            KrunCall::StartEnter => return failed(),
            _ if value != 0 => return failed(),
            _ => {}
        }
    }
    refuse(HelperExit::Krun, "the plan ended without krun_start_enter")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_fixtures::{recipe, recipe_value};
    use HelperExit::*;

    const HELPER: &str = "/opt/cell/bin/krun-helper";
    const LIBRARY: &str = "/opt/cell/lib/libkrun.1.dylib";

    fn fixture() -> RuntimeRecipe {
        recipe(&recipe_value(HELPER))
    }

    fn exit_of<T: fmt::Debug>(outcome: Result<T, HelperRefusal>) -> HelperExit {
        outcome.expect_err("refused").exit
    }

    fn strings(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn exit_codes_are_the_qualified_helpers_and_distinct() {
        let codes = EXITS.map(HelperExit::code);
        assert_eq!(codes, [64, 65, 66, 67, 68, 69, 70, 72, 86]);
        for exit in EXITS {
            assert_eq!(HelperExit::from_code(exit.code()), Some(exit));
        }
        for code in [0, 1, 63, 71, 73, 125, 126, 127] {
            assert_eq!(HelperExit::from_code(code), None, "{code}");
        }
        let mut stages = EXITS.map(HelperExit::stage).to_vec();
        stages.sort_unstable();
        stages.dedup();
        assert_eq!(stages.len(), EXITS.len());
        let shown = refuse(Descriptors, "descriptor 4 is open; only 0 to 3 may be").to_string();
        assert_eq!(
            shown,
            "refused descriptors exit=70: descriptor 4 is open; only 0 to 3 may be"
        );
    }

    #[test]
    fn invocation_needs_exactly_the_runtime_fd_argument() {
        assert_eq!(
            judge_invocation(&strings(&[HELPER, RUNTIME_FD_ARG]), true),
            Ok(())
        );
        let cases: [&[&str]; 7] = [
            &[],
            &[HELPER],
            &[HELPER, "--runtime-fd=4"],
            &[HELPER, "--runtime-fd=3 "],
            &[HELPER, ""],
            &[HELPER, RUNTIME_FD_ARG, RUNTIME_FD_ARG],
            &[RUNTIME_FD_ARG],
        ];
        for args in cases {
            assert_eq!(
                exit_of(judge_invocation(&strings(args), true)),
                Invocation,
                "{args:?}"
            );
        }
    }

    #[test]
    fn invocation_refuses_any_environment() {
        let refusal = judge_invocation(&strings(&[HELPER, RUNTIME_FD_ARG]), false);
        assert_eq!(
            refusal,
            Err(refuse(Invocation, "the environment must be empty"))
        );
    }

    const NULL: u64 = 0x0300_0002;

    fn facts(fd: i32, kind: DescriptorKind, rdev: u64) -> DescriptorFacts {
        DescriptorFacts { fd, kind, rdev }
    }

    fn launched() -> Vec<DescriptorFacts> {
        use DescriptorKind::{CharDevice, Fifo};
        vec![
            facts(3, Fifo, 0),
            facts(0, CharDevice, NULL),
            facts(2, Fifo, 0),
            facts(1, CharDevice, NULL),
        ]
    }

    fn descriptor_refusal(open: &[DescriptorFacts]) -> String {
        let refusal = judge_descriptors(open, NULL).expect_err("refused");
        assert_eq!(refusal.exit, Descriptors);
        refusal.detail
    }

    #[test]
    fn descriptors_must_be_exactly_zero_to_three() {
        assert_eq!(judge_descriptors(&launched(), NULL), Ok(()));
        let mut extra = launched();
        extra.push(facts(9, DescriptorKind::Regular, 0));
        extra.push(facts(4, DescriptorKind::Socket, 0));
        assert_eq!(
            descriptor_refusal(&extra),
            "descriptor 4 is open; only 0 to 3 may be"
        );
        let missing: Vec<_> = launched()
            .into_iter()
            .filter(|facts| facts.fd != 2)
            .collect();
        assert_eq!(descriptor_refusal(&missing), "descriptor 2 is not open");
    }

    #[test]
    fn stdin_and_stdout_must_be_dev_null() {
        for (fd, kind, rdev, shown) in [
            (
                0,
                DescriptorKind::CharDevice,
                NULL + 1,
                "a character device",
            ),
            (1, DescriptorKind::Regular, NULL, "a regular file"),
            (0, DescriptorKind::Fifo, 0, "a pipe"),
        ] {
            let mut open = launched();
            open.retain(|facts| facts.fd != fd);
            open.push(facts(fd, kind, rdev));
            assert_eq!(
                descriptor_refusal(&open),
                format!("descriptor {fd} is {shown}, not /dev/null")
            );
        }
    }

    #[test]
    fn diagnostics_and_recipe_must_be_pipes() {
        for (fd, kind, shown) in [
            (2, DescriptorKind::Socket, "a socket"),
            (3, DescriptorKind::Regular, "a regular file"),
            (3, DescriptorKind::CharDevice, "a character device"),
            (2, DescriptorKind::Directory, "a directory"),
            (3, DescriptorKind::Other, "another kind of file"),
        ] {
            let mut open = launched();
            open.retain(|facts| facts.fd != fd);
            open.push(facts(fd, kind, NULL));
            assert_eq!(
                descriptor_refusal(&open),
                format!("descriptor {fd} is {shown}, not a pipe")
            );
        }
    }

    struct Broken;

    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from_raw_os_error(libc::EIO))
        }
    }

    #[test]
    fn recipe_read_stops_one_byte_past_the_frame() {
        assert_eq!(MAX_RECIPE_BYTES, 1_048_577);
        let whole = vec![b'x'; MAX_RECIPE_BYTES];
        assert_eq!(read_recipe(whole.as_slice()), Ok(whole.clone()));
        let longer = vec![b'x'; MAX_RECIPE_BYTES + 1];
        let refused = Err(refuse(Recipe, "the recipe is longer than 1048577 bytes"));
        assert_eq!(read_recipe(longer.as_slice()), refused);
        assert_eq!(read_recipe(std::io::repeat(b'x')), refused);
        let broken = read_recipe(Broken).expect_err("a read error refuses");
        assert_eq!(broken.exit, Recipe);
        assert!(
            broken.detail.starts_with("reading the recipe failed: "),
            "{broken}"
        );
    }

    #[test]
    fn x86_64_raw_exits_86_before_anything_else() {
        let mut value = recipe_value(HELPER);
        value["architecture"] = "x86_64".into();
        value["kernel_format"] = "raw".into();
        let limits = PlatformLimits::host().expect("this host reports its limits");
        let refusal = parse_recipe(format!("{value}\n").as_bytes(), &limits);
        assert_eq!(exit_of(refusal), UnadmittedBootPair);
    }

    #[test]
    fn every_other_parse_refusal_exits_65() {
        let limits = PlatformLimits::host().expect("this host reports its limits");
        let line = format!("{}\n", recipe_value(HELPER));
        assert_eq!(parse_recipe(line.as_bytes(), &limits), Ok(fixture()));
        let mut zero = recipe_value(HELPER);
        zero["vcpus"] = 0.into();
        let mut unknown = recipe_value(HELPER);
        unknown["extra"] = 1.into();
        for record in [
            b"garbage\n".to_vec(),
            line.trim_end().as_bytes().to_vec(),
            format!("{line}{line}").into_bytes(),
            format!("{zero}\n").into_bytes(),
            format!("{unknown}\n").into_bytes(),
            Vec::new(),
        ] {
            let refusal = parse_recipe(&record, &limits).expect_err("refused");
            assert_eq!(refusal.exit, Recipe, "{refusal}");
            let parsed = RuntimeRecipe::parse(&record, &limits).expect_err("refused");
            assert_eq!(refusal.detail, parsed.to_string());
        }
    }

    #[test]
    fn a_host_whose_limits_cannot_be_read_exits_72() {
        let limits = PlatformLimits::host().expect("this host reports its limits");
        assert_eq!(judge_host_limits(Ok(limits)), Ok(limits));
        for error in [
            HostLimitsError::Unavailable {
                limit: "physical memory",
            },
            HostLimitsError::UnknownArchitecture {
                found: "riscv64".to_owned(),
            },
        ] {
            let refused = Err(refuse(UnsupportedHost, error.to_string()));
            assert_eq!(judge_host_limits(Err(error)), refused);
        }
    }

    #[test]
    fn self_must_be_the_recipe_helper_path() {
        let recipe = fixture();
        assert_eq!(judge_self(Path::new(HELPER), &recipe), Ok(()));
        for other in [
            "/opt/cell/bin/other",
            "/opt/cell/bin/../bin/krun-helper",
            "krun-helper",
        ] {
            assert_eq!(
                exit_of(judge_self(Path::new(other), &recipe)),
                Verify,
                "{other}"
            );
        }
    }

    const PRIVATE: WritableRootFacts = WritableRootFacts {
        regular: true,
        uid: 501,
        mode: 0o100600,
        nlink: 1,
        immutable: false,
        dev: 1,
        ino: 2,
    };

    #[test]
    fn writable_root_must_be_private_single_link_and_mutable() {
        assert_eq!(judge_writable_root(&PRIVATE, 501), Ok(()));
        for (facts, broken) in [
            (
                WritableRootFacts {
                    mode: 0o100644,
                    ..PRIVATE
                },
                "it has mode 0644, not 0600",
            ),
            (
                WritableRootFacts {
                    mode: 0o104600,
                    ..PRIVATE
                },
                "it has mode 4600, not 0600",
            ),
            (
                WritableRootFacts {
                    mode: 0o100400,
                    ..PRIVATE
                },
                "it has mode 0400, not 0600",
            ),
            (
                WritableRootFacts {
                    nlink: 2,
                    ..PRIVATE
                },
                "it has 2 links, not 1",
            ),
            (
                WritableRootFacts {
                    nlink: 0,
                    ..PRIVATE
                },
                "it has 0 links, not 1",
            ),
            (
                WritableRootFacts { uid: 0, ..PRIVATE },
                "it is owned by uid 0, not the effective uid 501",
            ),
            (
                WritableRootFacts {
                    immutable: true,
                    ..PRIVATE
                },
                "it is immutable",
            ),
            (
                WritableRootFacts {
                    regular: false,
                    ..PRIVATE
                },
                "it is not a regular file",
            ),
        ] {
            let detail = format!("the writable root is refused: {broken}");
            assert_eq!(
                judge_writable_root(&facts, 501),
                Err(refuse(Verify, detail))
            );
        }
    }

    #[test]
    fn host_policy_needs_one_trailing_nul() {
        let profile = judge_host_policy_bytes(b"(version 1)(deny default)\0");
        assert_eq!(profile, Ok(c"(version 1)(deny default)".to_owned()));
        let cases: [(&[u8], &str); 6] = [
            (
                b"(version 1)",
                "must hold exactly one NUL, as its last byte",
            ),
            (
                b"(ver\0sion 1)\0",
                "must hold exactly one NUL, as its last byte",
            ),
            (
                b"(version 1)\0\0",
                "must hold exactly one NUL, as its last byte",
            ),
            (b"", "must hold exactly one NUL, as its last byte"),
            (b"\0", "is empty"),
            (b"(version \xff)\0", "is not UTF-8"),
        ];
        for (bytes, broken) in cases {
            let detail = format!("the host policy {broken}");
            assert_eq!(
                judge_host_policy_bytes(bytes),
                Err(refuse(HostPolicy, detail))
            );
        }
    }

    #[test]
    fn host_policy_read_stops_one_byte_past_its_bound() {
        let mut whole = vec![b' '; MAX_HOST_POLICY_BYTES];
        whole[MAX_HOST_POLICY_BYTES - 1] = 0;
        let profile = read_host_policy(whole.as_slice()).expect("64 KiB with its NUL is read");
        assert_eq!(profile.as_bytes_with_nul().len(), MAX_HOST_POLICY_BYTES);
        whole.insert(0, b' ');
        let refusal = read_host_policy(whole.as_slice());
        let detail = "the host policy is longer than 65536 bytes";
        assert_eq!(refusal, Err(refuse(HostPolicy, detail)));
        assert_eq!(exit_of(read_host_policy(Broken)), HostPolicy);
        let unterminated = read_host_policy(&b"(version 1)"[..]);
        let detail = "the host policy must hold exactly one NUL, as its last byte";
        assert_eq!(unterminated, Err(refuse(HostPolicy, detail)));
    }

    #[test]
    fn eperm_is_the_only_denial() {
        for probe in [DeniedProbe::FreshFileWrite, DeniedProbe::TcpConnect] {
            assert_eq!(judge_denial(probe, Err(libc::EPERM)), Ok(()));
        }
    }

    #[test]
    fn a_probe_that_succeeds_refuses() {
        let refusal = judge_denial(DeniedProbe::TcpConnect, Ok(()));
        let detail = "the host policy did not deny a TCP connect: it succeeded";
        assert_eq!(refusal, Err(refuse(HostPolicy, detail)));
    }

    #[test]
    fn a_probe_failing_with_another_errno_refuses() {
        for errno in [libc::ENOENT, libc::EACCES, libc::ECONNREFUSED] {
            let refusal = judge_denial(DeniedProbe::FreshFileWrite, Err(errno));
            let detail = format!(
                "the host policy did not deny a fresh file write: it failed with errno {errno}, not EPERM"
            );
            assert_eq!(refusal, Err(refuse(HostPolicy, detail)));
        }
    }

    fn names(images: &[&str]) -> Vec<String> {
        images.iter().map(|name| (*name).to_owned()).collect()
    }

    fn sealed(_: &str) -> Result<bool, ClosureFault> {
        Ok(true)
    }

    #[test]
    fn listed_and_system_images_pass() {
        let images = names(&[
            HELPER,
            LIBRARY,
            "/usr/lib/libSystem.B.dylib",
            "/System/Library/Frameworks/Hypervisor.framework/Versions/A/Hypervisor",
        ]);
        let count = judge_images_with(&images, &fixture(), sealed);
        let expected = ImageCount {
            total: 4,
            system: 2,
            listed: 2,
        };
        assert_eq!(count, Ok(expected));
    }

    #[test]
    fn dotdot_in_a_listed_name_is_resolved() {
        let images = names(&[
            HELPER,
            "/opt/cell/bin/../lib/libkrun.1.dylib",
            "/opt/cell/bin/./krun-helper",
        ]);
        let count = judge_images_with(&images, &fixture(), sealed).expect("both resolve");
        assert_eq!((count.total, count.listed), (3, 3));
        let at_root = recipe(&recipe_value("/krun-helper"));
        let images = names(&["/krun-helper", "/../opt/cell/lib/libkrun.1.dylib"]);
        let count = judge_images_with(&images, &at_root, sealed).expect("both resolve");
        assert_eq!(count.listed, 2);
    }

    #[test]
    fn an_unlisted_image_refuses_by_name() {
        for name in [
            "/opt/homebrew/opt/llvm/lib/libkrunfw.5.dylib",
            "/usr/libx/y.dylib",
            "/usr/lib/../local/z.dylib",
            "/opt/x/../cell/lib/libkrun.1.dylib",
            "/opt/cell/bin/sub/../../lib/libkrun.1.dylib",
            "/opt/cell/binx/../lib/libkrun.1.dylib",
            "/opt/cell/binkrun-helper",
            "/opt/cell/bin../lib/libkrun.1.dylib",
            "/opt/cell/bin/../../../usr/lib/libSystem.B.dylib",
            "/opt/homebrew/opt/llvm/lib/libkrun.1.dylib",
            "libkrunfw.5.dylib",
            "/usr/lib/libSyst\u{e9}m.dylib",
        ] {
            let refusal = judge_images_with(&names(&[HELPER, name]), &fixture(), sealed);
            let detail =
                format!("{name:?} is not the helper, a listed library or a sealed system library");
            assert_eq!(refusal, Err(refuse(Inventory, detail)));
        }
    }

    #[test]
    fn the_first_bad_image_is_named() {
        let images = names(&[HELPER, "/opt/x/a.dylib", "/opt/x/b.dylib"]);
        let refusal = judge_images_with(&images, &fixture(), sealed);
        let detail =
            "\"/opt/x/a.dylib\" is not the helper, a listed library or a sealed system library";
        assert_eq!(refusal, Err(refuse(Inventory, detail)));
    }

    #[test]
    fn an_inventory_without_the_helper_refuses() {
        let system_only = names(&["/usr/lib/libSystem.B.dylib"]);
        for images in [Vec::new(), system_only, names(&[LIBRARY])] {
            let refusal = judge_images_with(&images, &fixture(), sealed);
            let detail = format!(
                "the helper {HELPER:?} is not among the {} loaded images",
                images.len()
            );
            assert_eq!(refusal, Err(refuse(Inventory, detail)));
        }
    }

    #[test]
    fn a_system_name_off_the_sealed_volume_refuses() {
        let images = names(&["/usr/lib/cron/libx.dylib"]);
        let refusal = judge_images_with(&images, &fixture(), |_| Ok(false));
        let detail = "\"/usr/lib/cron/libx.dylib\" is a system name off the sealed system volume";
        assert_eq!(refusal, Err(refuse(Inventory, detail)));
        let fault = ClosureFault::Io {
            op: "statfs",
            errno: libc::EIO,
        };
        let refusal = judge_images_with(&images, &fixture(), |_| Err(fault.clone()));
        let detail = format!("\"/usr/lib/cron/libx.dylib\" could not be judged: {fault}");
        assert_eq!(refusal, Err(refuse(Inventory, detail)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn loaded_images_are_judged_against_the_real_sealed_volume() {
        let images = names(&[HELPER, "/usr/lib/libSystem.B.dylib"]);
        assert!(judge_loaded_images(&images, &fixture()).is_ok());
        let off = names(&["/System/Library/Caches/libx.dylib"]);
        assert_eq!(exit_of(judge_loaded_images(&off, &fixture())), Inventory);
    }

    fn text(path: &str) -> CString {
        CString::new(path).expect("no NUL")
    }

    fn port(port: u32, path: &str) -> KrunCall {
        KrunCall::AddVsockPort2 {
            port,
            path: text(path),
            listen: false,
        }
    }

    fn root_disk() -> KrunCall {
        KrunCall::AddDisk3 {
            block_id: text("root"),
            path: text("/var/cell/c1/root.raw"),
            format: 0,
            read_only: false,
            direct_io: false,
            sync_mode: 2,
        }
    }

    #[test]
    fn plan_is_the_pinned_fourteen_calls() {
        let recipe = fixture();
        let expected = vec![
            KrunCall::InitLog {
                target_fd: 2,
                level: 2,
                style: 2,
                options: 1,
            },
            KrunCall::CreateCtx,
            KrunCall::SetVmConfig {
                vcpus: 1,
                ram_mib: 512,
            },
            KrunCall::SetKernel {
                kernel: text("/opt/cell/kernel"),
                format: recipe.kernel_format().krun_value(),
                initramfs: text("/opt/cell/initramfs"),
                cmdline: text(GUEST_CMDLINE),
            },
            root_disk(),
            KrunCall::DisableImplicitInit,
            KrunCall::DisableImplicitConsole,
            KrunCall::DisableImplicitVsock,
            KrunCall::AddVsock { tsi_features: 0 },
            KrunCall::SetEnvEmpty,
            KrunCall::SetPortMapEmpty,
            port(4090, "/var/cell/c1/control.sock"),
            port(4091, "/var/cell/c1/data.sock"),
            KrunCall::StartEnter,
        ];
        assert_eq!(krun_plan(&recipe), expected);
        assert_eq!(GUEST_CMDLINE, "rdinit=/init panic=-1");
    }

    #[test]
    fn kernel_format_reaches_set_kernel() {
        for (format, value) in [("elf", 1), ("image_zstd", 5), ("pe_gz", 2)] {
            let mut recipe_text = recipe_value(HELPER);
            recipe_text["kernel_format"] = format.into();
            let plan = krun_plan(&recipe(&recipe_text));
            assert!(
                matches!(&plan[3], KrunCall::SetKernel { format, .. } if *format == value),
                "{plan:?}"
            );
        }
    }

    #[test]
    fn vm_config_is_the_recipes() {
        let mut value = recipe_value(HELPER);
        value["vcpus"] = 2.into();
        value["memory_mib"] = 1024.into();
        let plan = krun_plan(&recipe(&value));
        let expected = KrunCall::SetVmConfig {
            vcpus: 2,
            ram_mib: 1024,
        };
        assert_eq!(plan[2], expected);
    }

    #[test]
    fn ports_map_control_then_data() {
        let plan = krun_plan(&fixture());
        let ports: Vec<_> = plan
            .iter()
            .filter(|call| matches!(call, KrunCall::AddVsockPort2 { .. }))
            .collect();
        let expected = [
            port(4090, "/var/cell/c1/control.sock"),
            port(4091, "/var/cell/c1/data.sock"),
        ];
        assert_eq!(ports, expected.iter().collect::<Vec<_>>());
    }

    #[test]
    fn exactly_one_disk_with_the_fixed_flags() {
        let plan = krun_plan(&fixture());
        let disks: Vec<_> = plan
            .iter()
            .filter(|call| matches!(call, KrunCall::AddDisk3 { .. }))
            .collect();
        assert_eq!(disks, [&root_disk()]);
    }

    const CALL_NAMES: [&str; 14] = [
        "krun_init_log",
        "krun_create_ctx",
        "krun_set_vm_config",
        "krun_set_kernel",
        "krun_add_disk3",
        "krun_disable_implicit_init",
        "krun_disable_implicit_console",
        "krun_disable_implicit_vsock",
        "krun_add_vsock",
        "krun_set_env",
        "krun_set_port_map",
        "krun_add_vsock_port2",
        "krun_add_vsock_port2",
        "krun_start_enter",
    ];

    #[test]
    fn call_names_are_the_libkrun_functions() {
        let plan = krun_plan(&fixture());
        assert_eq!(
            plan.iter().map(KrunCall::name).collect::<Vec<_>>(),
            CALL_NAMES
        );
    }

    struct Run {
        calls: Vec<(&'static str, Option<u32>)>,
        after_create_ran: usize,
        refusal: HelperRefusal,
    }

    fn run(
        plan: &[KrunCall],
        failing: Option<(usize, i32)>,
        after: Result<(), HelperRefusal>,
    ) -> Run {
        let mut calls = Vec::new();
        let ran = std::cell::Cell::new(0);
        let refusal = run_plan(
            plan,
            |step, context| {
                calls.push((step.name(), context));
                match failing {
                    Some((at, value)) if at == calls.len() - 1 => value,
                    _ if matches!(step, KrunCall::CreateCtx) => 7,
                    _ => 0,
                }
            },
            || {
                ran.set(ran.get() + 1);
                after.clone()
            },
        );
        Run {
            calls,
            after_create_ran: ran.get(),
            refusal,
        }
    }

    #[test]
    fn run_plan_passes_the_created_context() {
        let outcome = run(&krun_plan(&fixture()), None, Ok(()));
        let contexts: Vec<_> = outcome.calls.iter().map(|(_, context)| *context).collect();
        let mut expected = vec![None, None];
        expected.extend([Some(7); 12]);
        assert_eq!(contexts, expected);
        let called: Vec<_> = outcome.calls.iter().map(|(name, _)| *name).collect();
        assert_eq!(called, CALL_NAMES);
        assert_eq!(outcome.after_create_ran, 1);
        assert_eq!(outcome.refusal, refuse(Krun, "krun_start_enter returned 0"));
    }

    #[test]
    fn after_create_runs_between_create_and_vm_config() {
        let log = std::cell::RefCell::new(Vec::new());
        run_plan(
            &krun_plan(&fixture()),
            |step, _| {
                log.borrow_mut().push(step.name());
                0
            },
            || {
                log.borrow_mut().push("after_create");
                Ok(())
            },
        );
        let first = [
            "krun_init_log",
            "krun_create_ctx",
            "after_create",
            "krun_set_vm_config",
        ];
        assert_eq!(log.borrow()[..4], first);
        assert_eq!(log.borrow().len(), 15);
    }

    #[test]
    fn an_after_create_refusal_stops_the_plan() {
        let inventory = refuse(
            Inventory,
            "\"/usr/local/lib/libkrunfw.5.dylib\" is not listed",
        );
        let outcome = run(&krun_plan(&fixture()), None, Err(inventory.clone()));
        assert_eq!(outcome.calls.len(), 2);
        assert_eq!(outcome.refusal, inventory);
    }

    #[test]
    fn a_negative_context_stops_everything() {
        for context in [-1, -12, i32::MIN] {
            let outcome = run(&krun_plan(&fixture()), Some((1, context)), Ok(()));
            assert_eq!(outcome.calls.len(), 2);
            assert_eq!(outcome.after_create_ran, 0);
            let detail = format!("krun_create_ctx returned {context}");
            assert_eq!(outcome.refusal, refuse(Krun, detail));
        }
    }

    #[test]
    fn init_log_failure_stops_before_create() {
        let outcome = run(&krun_plan(&fixture()), Some((0, -22)), Ok(()));
        assert_eq!(outcome.calls, [("krun_init_log", None)]);
        assert_eq!(outcome.refusal, refuse(Krun, "krun_init_log returned -22"));
    }

    #[test]
    fn each_nonzero_setter_stops_at_that_call() {
        for (at, name) in CALL_NAMES.into_iter().enumerate().take(13).skip(2) {
            for value in [-22, 1] {
                let outcome = run(&krun_plan(&fixture()), Some((at, value)), Ok(()));
                assert_eq!(outcome.calls.len(), at + 1, "{at}");
                let label = match at {
                    11 => format!("{name}(4090)"),
                    12 => format!("{name}(4091)"),
                    _ => name.to_owned(),
                };
                let detail = format!("{label} returned {value}");
                assert_eq!(outcome.refusal, refuse(Krun, detail));
            }
        }
    }

    #[test]
    fn start_enter_returning_is_always_a_refusal() {
        for value in [0, -22, 1] {
            let outcome = run(&krun_plan(&fixture()), Some((13, value)), Ok(()));
            assert_eq!(outcome.calls.len(), 14);
            let detail = format!("krun_start_enter returned {value}");
            assert_eq!(outcome.refusal, refuse(Krun, detail));
        }
    }

    #[test]
    fn a_plan_that_ends_without_start_refuses() {
        let plan = krun_plan(&fixture());
        let outcome = run(&plan[..13], None, Ok(()));
        assert_eq!(outcome.calls.len(), 13);
        let detail = "the plan ended without krun_start_enter";
        assert_eq!(outcome.refusal, refuse(Krun, detail));
    }
}
