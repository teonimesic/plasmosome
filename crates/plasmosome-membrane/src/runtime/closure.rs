use std::fmt;

use super::artifacts::VerifiedArtifact;
#[cfg(any(target_os = "macos", test))]
use super::path::RecipePath;
use super::recipe::ArtifactRole;

/// The most bytes [`audit_closure`] reads of one image. A larger file is refused as
/// [`ClosureFault::TooLarge`]; the read stops one byte past this bound.
pub const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;

/// Which load command names a library. dyld loads a library of each kind when it is present, so
/// every kind is audited.
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoadKind {
    Load,
    Weak,
    Reexport,
    Lazy,
    Upward,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CpuType(pub(crate) u32);

#[cfg(any(target_os = "macos", test))]
impl CpuType {
    pub(crate) const ARM64: CpuType = CpuType(0x0100_000c);
    pub(crate) const X86_64: CpuType = CpuType(0x0100_0007);

    pub(crate) fn host() -> Option<CpuType> {
        todo!()
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct MachOImage {
    pub(crate) names: Vec<(LoadKind, String)>,
    pub(crate) rpaths: Vec<String>,
    pub(crate) has_dyld_environment: bool,
    pub(crate) dylinkers: Vec<String>,
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn read_macho(bytes: &[u8], cpu: CpuType) -> Result<MachOImage, ClosureFault> {
    let _ = (bytes, cpu);
    todo!()
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolution {
    Listed(usize),
    System(String),
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn resolve(
    name: &str,
    loader_dir: &str,
    executable_dir: &str,
    listed: &[&RecipePath],
) -> Result<Resolution, ClosureFault> {
    let _ = (name, loader_dir, executable_dir, listed);
    todo!()
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn is_system_boundary(name: &str) -> bool {
    let _ = name;
    todo!()
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuditedNames {
    pub(crate) resolved: Vec<(ArtifactRole, String, ArtifactRole)>,
    pub(crate) system_names: Vec<String>,
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn audit_images(
    cpu: CpuType,
    helper: (&RecipePath, &[u8]),
    libraries: &[(&RecipePath, &[u8])],
) -> Result<AuditedNames, ClosureRefusal> {
    let _ = (cpu, helper, libraries);
    todo!()
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn read_bounded(source: impl std::io::Read, max: u64) -> Result<Vec<u8>, ClosureFault> {
    let _ = (source, max);
    todo!()
}

/// What the static audit found: which listed library each load name resolved to, which names it
/// relied on the Darwin system boundary for, and the OS build that names that boundary.
///
/// It covers load commands only. `loaded_images` and `runtime_loads` say what it does not show;
/// a reader must not take a pass as spec 001 §4.2's inspection of the images the running helper
/// actually loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureEvidence {
    /// One entry per load name that resolved to a listed library: the image holding the load
    /// command, the name as written, and the library it resolved to. In audit order: the helper,
    /// then each library in recipe order.
    pub resolved: Vec<(ArtifactRole, String, ArtifactRole)>,
    /// Each load name relied on as part of the sealed system volume or shared cache, once, in the
    /// order first met.
    pub system_names: Vec<String>,
    /// `kern.osversion`, such as `25G83`: the build of the sealed system volume those names come
    /// from.
    pub os_build: String,
    /// `kern.osproductversion`, such as `26.6.2`.
    pub os_product_version: String,
    /// Always `NotIndependentlyObserved`: this audit reads files before exec and never sees the
    /// images the helper loads.
    pub loaded_images: ImageInventory,
    /// Always `NotAudited`: names an image's code passes to `dlopen` are not load commands.
    pub runtime_loads: RuntimeLoads,
}

/// Whether anyone inspected the images the running helper loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageInventory {
    /// Nobody did. Spec 001 §4.2 asks the supervisor to inspect the actual loaded image set
    /// before guest execution; this audit does not, and owner gate O-5 on `plasmosome-qep` decides
    /// how it can be done.
    NotIndependentlyObserved,
}

/// Whether the libraries an image loads by name at run time were audited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeLoads {
    /// They were not. A name the code passes to `dlopen` is in no load command, so this audit
    /// never sees it, and dyld searches for a leaf name. The pinned libkrun's `krun_create_ctx`
    /// loads `libkrunfw.5.dylib` that way on every context, whether or not an external kernel is
    /// set.
    NotAudited,
}

/// Why the closure was refused: the image whose bytes or load names broke a rule, and the rule.
/// A fault that belongs to no one image, `Unsupported` or `OsIdentity`, names the helper, whose
/// launch it refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureRefusal {
    pub image: ArtifactRole,
    pub fault: ClosureFault,
}

/// The first rule an image broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClosureFault {
    /// This host has no closure audit: only macOS does.
    Unsupported,
    /// The file is longer than `max` bytes.
    TooLarge { max: u64 },
    /// Reading the verified file failed.
    Io { op: &'static str, errno: i32 },
    /// The file is not a 64-bit little-endian Mach-O image or a fat file of them.
    NotMachO,
    /// No image in the file is for this host's CPU.
    NoMemberForHost,
    /// The header or a load command is cut short or inconsistent at `offset` bytes into the file.
    Malformed { offset: u64, what: &'static str },
    /// An `LC_RPATH` entry: a search path dyld consults for `@rpath` names and for any leaf name
    /// passed to `dlopen`.
    RpathPresent { path: String },
    /// An `LC_DYLD_ENVIRONMENT` command, which sets dyld variables from inside the image.
    DyldEnvironment,
    /// An `LC_LOAD_DYLINKER` other than `/usr/lib/dyld`.
    UnexpectedDylinker { name: String },
    /// A name dyld resolves by searching: an `@rpath` name, a name with no leading `/` or `@`,
    /// or an `@` name other than `@executable_path/` and `@loader_path/`.
    SearchedName { name: String },
    /// A name that is neither a listed library nor the sealed system boundary. `resolved` is the
    /// path a loader-relative name joins to, and `None` for an absolute name.
    UnlistedLibrary {
        name: String,
        resolved: Option<String>,
    },
    /// The OS identity could not be read: the sysctl `name` failed with `errno`.
    OsIdentity { name: &'static str, errno: i32 },
}

impl fmt::Display for ClosureRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::error::Error for ClosureRefusal {}

impl fmt::Display for ClosureFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!()
    }
}

/// Audits, before exec, every load name in `helper` and in each of `libraries`: each must
/// resolve, with no search, to one of `libraries` or to the sealed system boundary, under
/// `/usr/lib/` or `/System/Library/`. Pass the verified helper and the verified libraries in
/// recipe order, so `libraries[i]` has the role `Library(i)`.
///
/// It reads each file through its verified descriptor, at most [`MAX_IMAGE_BYTES`], and refuses
/// any `LC_RPATH`, any `LC_DYLD_ENVIRONMENT` and a dynamic linker other than `/usr/lib/dyld`. A
/// listed library that nothing names is allowed. On success the evidence records each resolution
/// and the OS build. Off macOS it refuses `Unsupported`.
///
/// It does not show which images the helper loads; see [`ClosureEvidence`].
pub fn audit_closure(
    helper: &VerifiedArtifact,
    libraries: &[VerifiedArtifact],
) -> Result<ClosureEvidence, ClosureRefusal> {
    let _ = (helper, libraries);
    todo!()
}
