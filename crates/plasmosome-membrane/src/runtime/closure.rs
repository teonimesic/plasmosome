use std::fmt;

#[cfg(target_os = "macos")]
use super::artifacts::Positioned;
use super::artifacts::VerifiedArtifact;
#[cfg(any(target_os = "macos", test))]
use super::path::RecipePath;
use super::recipe::ArtifactRole;

/// The most bytes [`audit_closure`] reads of one image. A larger file is refused as
/// [`ClosureFault::TooLarge`]; the read stops one byte past this bound.
pub const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;

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
        if cfg!(target_arch = "aarch64") {
            Some(CpuType::ARM64)
        } else if cfg!(target_arch = "x86_64") {
            Some(CpuType::X86_64)
        } else {
            None
        }
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct MachOImage {
    pub(crate) names: Vec<(LoadKind, String)>,
    pub(crate) rpaths: Vec<String>,
    pub(crate) has_dyld_environment: bool,
    pub(crate) dylinkers: Vec<String>,
    pub(crate) required_unknown: Vec<u32>,
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn read_macho(bytes: &[u8], cpu: CpuType) -> Result<MachOImage, ClosureFault> {
    const FAT_MAGIC: u32 = 0xcafe_babe;
    if bytes.len() >= 4 && be_word(bytes, 0) == FAT_MAGIC {
        let (start, end) = fat_member(bytes, cpu)?;
        read_thin(&bytes[start..end], start, cpu)
    } else {
        read_thin(bytes, 0, cpu)
    }
}

#[cfg(any(target_os = "macos", test))]
fn fat_member(bytes: &[u8], cpu: CpuType) -> Result<(usize, usize), ClosureFault> {
    const TABLE: usize = 8;
    const ENTRY: usize = 20;
    let malformed = |at: usize, what| ClosureFault::Malformed {
        offset: at as u64,
        what,
    };
    if bytes.len() < TABLE {
        return Err(malformed(4, "the fat header is cut short"));
    }
    let count = be_word(bytes, 4) as usize;
    if (bytes.len() - TABLE) / ENTRY < count {
        return Err(malformed(
            TABLE,
            "the fat table runs past the end of the file",
        ));
    }
    let mut found = None;
    for index in 0..count {
        let at = TABLE + ENTRY * index;
        let start = be_word(bytes, at + 8) as usize;
        let end = start
            .checked_add(be_word(bytes, at + 12) as usize)
            .filter(|end| *end <= bytes.len())
            .ok_or(malformed(at, "a fat member runs past the end of the file"))?;
        if be_word(bytes, at) == cpu.0 && found.replace((start, end)).is_some() {
            return Err(malformed(at, "two fat members are for the host CPU"));
        }
    }
    found.ok_or(ClosureFault::NoMemberForHost)
}

#[cfg(any(target_os = "macos", test))]
fn read_thin(member: &[u8], base: usize, cpu: CpuType) -> Result<MachOImage, ClosureFault> {
    const MH_MAGIC_64: u32 = 0xfeed_facf;
    const HEADER: usize = 32;
    let malformed = |at: usize, what| ClosureFault::Malformed {
        offset: (base + at) as u64,
        what,
    };
    if member.len() < 4 || le_word(member, 0) != MH_MAGIC_64 {
        return Err(ClosureFault::NotMachO);
    }
    if member.len() < HEADER {
        return Err(malformed(0, "the Mach-O header is cut short"));
    }
    if le_word(member, 4) != cpu.0 {
        return Err(ClosureFault::NoMemberForHost);
    }
    let count = le_word(member, 16);
    let commands = member
        .get(HEADER..HEADER + le_word(member, 20) as usize)
        .ok_or(malformed(20, "sizeofcmds runs past the end of the image"))?;
    let mut image = MachOImage::default();
    let mut at = 0;
    for _ in 0..count {
        let fault = |what| malformed(HEADER + at, what);
        let rest = &commands[at..];
        if rest.len() < 8 {
            return Err(fault(
                "ncmds counts more load commands than sizeofcmds holds",
            ));
        }
        let size = le_word(rest, 4) as usize;
        if size == 0 || !size.is_multiple_of(8) {
            return Err(fault("cmdsize is zero or not a multiple of 8"));
        }
        let command = rest
            .get(..size)
            .ok_or(fault("the load command runs past sizeofcmds"))?;
        read_command(command, &mut image).map_err(fault)?;
        at += size;
    }
    Ok(image)
}

#[cfg(any(target_os = "macos", test))]
fn read_command(command: &[u8], image: &mut MachOImage) -> Result<(), &'static str> {
    const DYLIB_FIELDS: usize = 24;
    const TEXT_FIELDS: usize = 12;
    match le_word(command, 0) {
        0x8000_001c => image.rpaths.push(name_in(command, TEXT_FIELDS)?),
        0xe => image.dylinkers.push(name_in(command, TEXT_FIELDS)?),
        0x27 => image.has_dyld_environment = true,
        cmd => {
            if let Some(kind) = load_kind(cmd) {
                image.names.push((kind, name_in(command, DYLIB_FIELDS)?));
            }
        }
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn load_kind(cmd: u32) -> Option<LoadKind> {
    match cmd {
        0xc => Some(LoadKind::Load),
        0x8000_0018 => Some(LoadKind::Weak),
        0x8000_001f => Some(LoadKind::Reexport),
        0x20 => Some(LoadKind::Lazy),
        0x8000_0023 => Some(LoadKind::Upward),
        _ => None,
    }
}

#[cfg(any(target_os = "macos", test))]
fn name_in(command: &[u8], fields: usize) -> Result<String, &'static str> {
    if command.len() < fields {
        return Err("the load command is shorter than its fixed fields");
    }
    let offset = le_word(command, 8) as usize;
    let text = command
        .get(offset..)
        .filter(|_| offset >= fields)
        .ok_or("the name offset is outside the load command's text")?;
    let end = text
        .iter()
        .position(|byte| *byte == 0)
        .ok_or("the name has no NUL")?;
    std::str::from_utf8(&text[..end])
        .map(str::to_owned)
        .map_err(|_| "the name is not UTF-8")
}

#[cfg(any(target_os = "macos", test))]
fn le_word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[cfg(any(target_os = "macos", test))]
fn be_word(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
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
    let position = |text: &str| listed.iter().position(|path| path.as_str() == text);
    let joined = if let Some(rest) = name.strip_prefix("@executable_path/") {
        join(executable_dir, rest)
    } else if let Some(rest) = name.strip_prefix("@loader_path/") {
        join(loader_dir, rest)
    } else if name.starts_with('/') {
        return match position(name) {
            Some(index) => Ok(Resolution::Listed(index)),
            None if is_system_name(name) => Ok(Resolution::System(name.to_owned())),
            None => Err(ClosureFault::UnlistedLibrary {
                name: name.to_owned(),
                resolved: None,
            }),
        };
    } else {
        return Err(ClosureFault::SearchedName {
            name: name.to_owned(),
        });
    };
    position(&joined)
        .map(Resolution::Listed)
        .ok_or(ClosureFault::UnlistedLibrary {
            name: name.to_owned(),
            resolved: Some(joined),
        })
}

#[cfg(any(target_os = "macos", test))]
fn join(directory: &str, rest: &str) -> String {
    let mut parts = Vec::new();
    for part in directory.split('/').chain(rest.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    format!("/{}", parts.join("/"))
}

#[cfg(any(target_os = "macos", test))]
fn directory(path: &RecipePath) -> &str {
    match path.as_str().rsplit_once('/') {
        Some((directory, _)) if !directory.is_empty() => directory,
        _ => "/",
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn is_system_name(name: &str) -> bool {
    name.is_ascii()
        && RecipePath::parse(name).is_ok()
        && (name.starts_with("/usr/lib/") || name.starts_with("/System/Library/"))
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VolumeFacts {
    pub(crate) mounted_at_root: bool,
    pub(crate) read_only: bool,
    pub(crate) seal: Option<bool>,
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn is_sealed_system_volume(facts: VolumeFacts) -> bool {
    let _ = facts;
    todo!()
}

#[cfg(target_os = "macos")]
pub(crate) fn volume_facts(path: &std::ffi::CStr) -> std::io::Result<VolumeFacts> {
    let _ = path;
    todo!()
}

#[cfg(target_os = "macos")]
pub(crate) fn on_sealed_volume(name: &str) -> Result<bool, ClosureFault> {
    use std::os::unix::ffi::OsStrExt;
    let mut nearest = std::path::Path::new(name);
    while let Err(error) = std::fs::symlink_metadata(nearest) {
        let absent = matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR));
        nearest = nearest
            .parent()
            .filter(|_| absent)
            .ok_or_else(|| io_fault("lstat", &error))?;
    }
    let nearest =
        std::ffi::CString::new(nearest.as_os_str().as_bytes()).map_err(|_| ClosureFault::Io {
            op: "statfs",
            errno: libc::EINVAL,
        })?;
    match on_read_only_root(&nearest) {
        Ok(sealed) => Ok(sealed),
        Err(error) if error.raw_os_error() == Some(libc::ENOENT) => Ok(false),
        Err(error) => Err(io_fault("statfs", &error)),
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn on_read_only_root(path: &std::ffi::CStr) -> std::io::Result<bool> {
    // SAFETY: libc::statfs is a plain C struct, for which all zeroes is a valid value.
    let mut facts: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is NUL-terminated, and statfs writes only the local `facts`.
    if unsafe { libc::statfs(path.as_ptr(), &mut facts) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mount = facts
        .f_mntonname
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| *byte as u8);
    Ok(mount.eq(*b"/") && facts.f_flags & libc::MNT_RDONLY as u32 != 0)
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuditedNames {
    pub(crate) resolved: Vec<(ArtifactRole, String, ArtifactRole)>,
    pub(crate) system_names: Vec<String>,
}

#[cfg(test)]
pub(crate) fn audit_images(
    cpu: CpuType,
    helper: (&RecipePath, &[u8]),
    libraries: &[(&RecipePath, &[u8])],
) -> Result<AuditedNames, ClosureRefusal> {
    audit_with(cpu, helper, libraries, |_| Ok(true))
}

#[cfg(any(target_os = "macos", test))]
fn audit_with(
    cpu: CpuType,
    helper: (&RecipePath, &[u8]),
    libraries: &[(&RecipePath, &[u8])],
    mut sealed: impl FnMut(&str) -> Result<bool, ClosureFault>,
) -> Result<AuditedNames, ClosureRefusal> {
    let listed: Vec<&RecipePath> = libraries.iter().map(|(path, _)| *path).collect();
    let executable_dir = directory(helper.0);
    let images = std::iter::once((ArtifactRole::Helper, helper)).chain(
        libraries
            .iter()
            .enumerate()
            .map(|(index, library)| (ArtifactRole::Library(index), *library)),
    );
    let mut audited = AuditedNames {
        resolved: Vec::new(),
        system_names: Vec::new(),
    };
    for (role, (path, bytes)) in images {
        let refuse = |fault| ClosureRefusal { image: role, fault };
        let image = read_macho(bytes, cpu).map_err(refuse)?;
        image_rules(&image).map_err(refuse)?;
        for (_, name) in &image.names {
            match resolve(name, directory(path), executable_dir, &listed).map_err(refuse)? {
                Resolution::Listed(index) => {
                    audited
                        .resolved
                        .push((role, name.clone(), ArtifactRole::Library(index)));
                }
                Resolution::System(name) if audited.system_names.contains(&name) => {}
                Resolution::System(name) => {
                    if !sealed(&name).map_err(refuse)? {
                        return Err(refuse(ClosureFault::UnlistedLibrary {
                            name,
                            resolved: None,
                        }));
                    }
                    audited.system_names.push(name);
                }
            }
        }
    }
    Ok(audited)
}

#[cfg(any(target_os = "macos", test))]
fn image_rules(image: &MachOImage) -> Result<(), ClosureFault> {
    if let Some(path) = image.rpaths.first() {
        return Err(ClosureFault::RpathPresent { path: path.clone() });
    }
    if image.has_dyld_environment {
        return Err(ClosureFault::DyldEnvironment);
    }
    match image.dylinkers.iter().find(|name| *name != "/usr/lib/dyld") {
        Some(name) => Err(ClosureFault::UnexpectedDylinker { name: name.clone() }),
        None => Ok(()),
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn read_bounded(source: impl std::io::Read, max: u64) -> Result<Vec<u8>, ClosureFault> {
    use std::io::Read;
    let mut bytes = Vec::new();
    source
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| io_fault("pread", &error))?;
    if bytes.len() as u64 > max {
        return Err(ClosureFault::TooLarge { max });
    }
    Ok(bytes)
}

#[cfg(any(target_os = "macos", test))]
fn io_fault(op: &'static str, error: &std::io::Error) -> ClosureFault {
    ClosureFault::Io {
        op,
        errno: error.raw_os_error().unwrap_or(libc::EIO),
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn sysctl_text(name: &'static std::ffi::CStr, label: &'static str) -> Result<String, ClosureFault> {
    let mut buffer = [0u8; 256];
    let mut size = buffer.len();
    // SAFETY: The name is a static NUL-terminated string. `buffer` and `size` are locals that
    // sysctlbyname may write, `size` holds the bytes `buffer` has, and no new value is passed.
    let read = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if read != 0 {
        let error = std::io::Error::last_os_error();
        return Err(ClosureFault::OsIdentity {
            name: label,
            errno: error.raw_os_error().unwrap_or(libc::EIO),
        });
    }
    std::ffi::CStr::from_bytes_until_nul(&buffer[..size.min(buffer.len())])
        .ok()
        .and_then(|text| text.to_str().ok())
        .map(str::to_owned)
        .ok_or(ClosureFault::OsIdentity {
            name: label,
            errno: libc::EILSEQ,
        })
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
    /// A `..` in a load name after a directory the name itself writes, as in
    /// `@loader_path/sub/../x`. dyld resolves `..` on the file system, so it goes through `sub`
    /// even when `sub` is a symlink, and no walk verified `sub`.
    DotDotAfterNamedDirectory { name: String },
    /// A load command of a kind this reader does not know, with the `LC_REQ_DYLD` bit
    /// (`0x8000_0000`) set: dyld must understand it to load the image, so it may load code.
    UnknownRequiredCommand { cmd: u32 },
}

impl fmt::Display for ClosureRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.image, self.fault)
    }
}

impl std::error::Error for ClosureRefusal {}

impl fmt::Display for ClosureFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClosureFault::Unsupported => {
                f.write_str("this host has no load-name audit; only macOS has one")
            }
            ClosureFault::TooLarge { max } => write!(f, "the image is longer than {max} bytes"),
            ClosureFault::Io { op, errno } => write!(f, "{op} failed with errno {errno}"),
            ClosureFault::NotMachO => {
                f.write_str("it is not a 64-bit little-endian Mach-O image or a fat file of them")
            }
            ClosureFault::NoMemberForHost => f.write_str("no image in it is for this host's CPU"),
            ClosureFault::Malformed { offset, what } => {
                write!(f, "it is malformed at byte {offset}: {what}")
            }
            ClosureFault::RpathPresent { path } => write!(f, "LC_RPATH {path} is a search path"),
            ClosureFault::DyldEnvironment => {
                f.write_str("LC_DYLD_ENVIRONMENT sets dyld variables from inside the image")
            }
            ClosureFault::UnexpectedDylinker { name } => {
                write!(f, "LC_LOAD_DYLINKER names {name}, not /usr/lib/dyld")
            }
            ClosureFault::SearchedName { name } => {
                write!(f, "{name:?} is a name dyld resolves by searching")
            }
            ClosureFault::UnlistedLibrary {
                name,
                resolved: Some(resolved),
            } => write!(
                f,
                "{name} resolves to {resolved}, which is not a listed library"
            ),
            ClosureFault::UnlistedLibrary {
                name,
                resolved: None,
            } => write!(
                f,
                "{name} is neither a listed library nor on the sealed system volume"
            ),
            ClosureFault::OsIdentity { name, errno } => {
                write!(f, "reading sysctl {name} failed with errno {errno}")
            }
            ClosureFault::DotDotAfterNamedDirectory { name } => {
                let _ = name;
                todo!()
            }
            ClosureFault::UnknownRequiredCommand { cmd } => {
                let _ = cmd;
                todo!()
            }
        }
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
    audit_on_host(helper, libraries)
}

#[cfg(target_os = "macos")]
fn audit_on_host(
    helper: &VerifiedArtifact,
    libraries: &[VerifiedArtifact],
) -> Result<ClosureEvidence, ClosureRefusal> {
    let refuse = |image| move |fault| ClosureRefusal { image, fault };
    let cpu = CpuType::host().ok_or(ClosureRefusal {
        image: ArtifactRole::Helper,
        fault: ClosureFault::Unsupported,
    })?;
    let read = |artifact: &VerifiedArtifact, image| {
        let source = Positioned {
            file: artifact.file(),
            offset: 0,
        };
        read_bounded(source, MAX_IMAGE_BYTES).map_err(refuse(image))
    };
    let helper_bytes = read(helper, ArtifactRole::Helper)?;
    let library_bytes = libraries
        .iter()
        .enumerate()
        .map(|(index, library)| read(library, ArtifactRole::Library(index)))
        .collect::<Result<Vec<_>, _>>()?;
    let inputs: Vec<(&RecipePath, &[u8])> = libraries
        .iter()
        .zip(&library_bytes)
        .map(|(library, bytes)| (library.path(), bytes.as_slice()))
        .collect();
    let audited = audit_with(
        cpu,
        (helper.path(), &helper_bytes),
        &inputs,
        on_sealed_volume,
    )?;
    let os = |name, label| sysctl_text(name, label).map_err(refuse(ArtifactRole::Helper));
    Ok(ClosureEvidence {
        resolved: audited.resolved,
        system_names: audited.system_names,
        os_build: os(c"kern.osversion", "kern.osversion")?,
        os_product_version: os(c"kern.osproductversion", "kern.osproductversion")?,
        loaded_images: ImageInventory::NotIndependentlyObserved,
        runtime_loads: RuntimeLoads::NotAudited,
    })
}

#[cfg(not(target_os = "macos"))]
fn audit_on_host(
    _helper: &VerifiedArtifact,
    _libraries: &[VerifiedArtifact],
) -> Result<ClosureEvidence, ClosureRefusal> {
    Err(ClosureRefusal {
        image: ArtifactRole::Helper,
        fault: ClosureFault::Unsupported,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ArtifactRole::{Helper, Library};
    use ClosureFault::*;
    use proptest::prelude::*;

    const MH_EXECUTE: u32 = 0x2;
    const MH_DYLIB: u32 = 0x6;
    const LC_LOAD_DYLIB: u32 = 0xc;
    const LC_ID_DYLIB: u32 = 0xd;
    const LC_LOAD_DYLINKER: u32 = 0xe;
    const LC_LOAD_WEAK_DYLIB: u32 = 0x8000_0018;
    const LC_RPATH: u32 = 0x8000_001c;
    const LC_REEXPORT_DYLIB: u32 = 0x8000_001f;
    const LC_LAZY_LOAD_DYLIB: u32 = 0x20;
    const LC_LOAD_UPWARD_DYLIB: u32 = 0x8000_0023;
    const LC_DYLD_ENVIRONMENT: u32 = 0x27;
    const ARM64: u32 = 0x0100_000c;
    const X86_64: u32 = 0x0100_0007;
    const LOAD_KINDS: [(u32, LoadKind); 5] = [
        (LC_LOAD_DYLIB, LoadKind::Load),
        (LC_LOAD_WEAK_DYLIB, LoadKind::Weak),
        (LC_REEXPORT_DYLIB, LoadKind::Reexport),
        (LC_LAZY_LOAD_DYLIB, LoadKind::Lazy),
        (LC_LOAD_UPWARD_DYLIB, LoadKind::Upward),
    ];

    const HELPER: &str = "/b/bin/krun-helper";
    const LIBKRUN: &str = "/b/lib/libkrun.1.19.4.dylib";
    const KRUN_NAME: &str = "@executable_path/../lib/libkrun.1.19.4.dylib";
    const LIBSYSTEM: &str = "/usr/lib/libSystem.B.dylib";
    const HYPERVISOR: &str =
        "/System/Library/Frameworks/Hypervisor.framework/Versions/A/Hypervisor";

    fn le(fields: &[u32]) -> Vec<u8> {
        fields
            .iter()
            .flat_map(|field| field.to_le_bytes())
            .collect()
    }

    fn command(header: &[u32], name: &str) -> Vec<u8> {
        let mut bytes = le(header);
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        bytes.resize(bytes.len().next_multiple_of(8), 0);
        let size = u32::try_from(bytes.len()).expect("a small command");
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        bytes
    }

    fn dylib(cmd: u32, name: &str) -> Vec<u8> {
        command(&[cmd, 0, 24, 2, 0x1_0000, 0x1_0000], name)
    }

    fn text(cmd: u32, text: &str) -> Vec<u8> {
        command(&[cmd, 0, 12], text)
    }

    fn load(name: &str) -> Vec<u8> {
        dylib(LC_LOAD_DYLIB, name)
    }

    fn image(cpu: u32, filetype: u32, commands: &[Vec<u8>]) -> Vec<u8> {
        let body = commands.concat();
        let count = |n: usize| u32::try_from(n).expect("small");
        let mut bytes = le(&[
            0xfeed_facf,
            cpu,
            0,
            filetype,
            count(commands.len()),
            count(body.len()),
            0,
            0,
        ]);
        bytes.extend_from_slice(&body);
        bytes.extend_from_slice(&[0x5a; 16]);
        bytes
    }

    fn executable(commands: &[Vec<u8>]) -> Vec<u8> {
        let mut all = vec![text(LC_LOAD_DYLINKER, "/usr/lib/dyld")];
        all.extend_from_slice(commands);
        image(ARM64, MH_EXECUTE, &all)
    }

    fn library(commands: &[Vec<u8>]) -> Vec<u8> {
        image(ARM64, MH_DYLIB, commands)
    }

    fn fat(members: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let small = |n: usize| u32::try_from(n).expect("small");
        let mut table = vec![0xcafe_babe, small(members.len())];
        let mut body = Vec::new();
        let start = (8 + 20 * members.len()).next_multiple_of(16);
        for (cpu, member) in members {
            body.resize(body.len().next_multiple_of(16), 0);
            table.extend([*cpu, 0, small(start + body.len()), small(member.len()), 4]);
            body.extend_from_slice(member);
        }
        let mut bytes: Vec<u8> = table.iter().flat_map(|field| field.to_be_bytes()).collect();
        bytes.resize(start, 0);
        bytes.extend_from_slice(&body);
        bytes
    }

    fn put(bytes: &[u8], at: usize, value: u32) -> Vec<u8> {
        let mut bytes = bytes.to_vec();
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        bytes
    }

    fn path(text: &str) -> RecipePath {
        RecipePath::parse(text).unwrap_or_else(|error| panic!("{text}: {error:?}"))
    }

    fn read(bytes: &[u8]) -> Result<MachOImage, ClosureFault> {
        read_macho(bytes, CpuType::ARM64)
    }

    fn names(bytes: &[u8], cpu: CpuType) -> Vec<(LoadKind, String)> {
        read_macho(bytes, cpu).expect("the image reads").names
    }

    fn loads(list: &[&str]) -> Vec<(LoadKind, String)> {
        list.iter()
            .map(|name| (LoadKind::Load, (*name).to_owned()))
            .collect()
    }

    fn resolve_in(
        name: &str,
        loader_dir: &str,
        listed: &[&str],
    ) -> Result<Resolution, ClosureFault> {
        let listed: Vec<RecipePath> = listed.iter().map(|text| path(text)).collect();
        resolve(
            name,
            loader_dir,
            "/b/bin",
            &listed.iter().collect::<Vec<_>>(),
        )
    }

    fn unlisted(name: &str, resolved: Option<&str>) -> ClosureFault {
        UnlistedLibrary {
            name: name.to_owned(),
            resolved: resolved.map(str::to_owned),
        }
    }

    const BIN: &str = "/b/bin";

    fn refused(helper: &[u8], libraries: &[(&str, Vec<u8>)]) -> (ArtifactRole, ClosureFault) {
        audit(helper, libraries).expect_err("the audit refuses")
    }

    fn audit(
        helper: &[u8],
        libraries: &[(&str, Vec<u8>)],
    ) -> Result<AuditedNames, (ArtifactRole, ClosureFault)> {
        let helper_path = path(HELPER);
        let paths: Vec<RecipePath> = libraries.iter().map(|(text, _)| path(text)).collect();
        let inputs: Vec<(&RecipePath, &[u8])> = paths
            .iter()
            .zip(libraries)
            .map(|(path, (_, bytes))| (path, bytes.as_slice()))
            .collect();
        audit_images(CpuType::ARM64, (&helper_path, helper), &inputs)
            .map_err(|refusal| (refusal.image, refusal.fault))
    }

    fn qualified_helper() -> Vec<u8> {
        executable(&[
            load(KRUN_NAME),
            load("/usr/lib/libiconv.2.dylib"),
            load(LIBSYSTEM),
        ])
    }

    fn libkrun_with(extra: &[Vec<u8>]) -> Vec<u8> {
        let mut commands = vec![
            dylib(LC_ID_DYLIB, KRUN_NAME),
            load(HYPERVISOR),
            load("/usr/lib/libiconv.2.dylib"),
            load(LIBSYSTEM),
        ];
        commands.extend_from_slice(extra);
        library(&commands)
    }

    #[test]
    fn the_host_cpu_is_the_build_target() {
        let expected = match std::env::consts::ARCH {
            "aarch64" => Some(CpuType::ARM64),
            "x86_64" => Some(CpuType::X86_64),
            _ => None,
        };
        assert_eq!(CpuType::host(), expected);
    }

    #[test]
    fn a_thin_image_yields_its_load_names_and_dynamic_linker() {
        let image = read(&executable(&[load(LIBSYSTEM)])).expect("it reads");

        assert_eq!(image.names, loads(&[LIBSYSTEM]));
        assert_eq!(image.dylinkers, ["/usr/lib/dyld"]);
        assert!(image.rpaths.is_empty() && !image.has_dyld_environment);
    }

    #[test]
    fn a_fat_file_yields_the_member_for_the_cpu_asked_not_the_first() {
        let x86 = image(X86_64, MH_EXECUTE, &[load("/x86/only.dylib")]);
        let bytes = fat(&[(X86_64, x86), (ARM64, executable(&[load(LIBSYSTEM)]))]);

        assert_eq!(names(&bytes, CpuType::ARM64), loads(&[LIBSYSTEM]));
        assert_eq!(names(&bytes, CpuType::X86_64), loads(&["/x86/only.dylib"]));
    }

    #[test]
    fn a_file_with_no_image_or_two_images_for_the_host_cpu_refuses() {
        let x86 = image(X86_64, MH_EXECUTE, &[load(LIBSYSTEM)]);
        let arm = executable(&[load(LIBSYSTEM)]);

        assert_eq!(read(&fat(&[(X86_64, x86.clone())])), Err(NoMemberForHost));
        assert_eq!(read(&fat(&[])), Err(NoMemberForHost));
        assert_eq!(read(&x86), Err(NoMemberForHost));
        assert!(matches!(
            read(&fat(&[(ARM64, arm.clone()), (ARM64, arm)])),
            Err(Malformed { offset: 28, .. })
        ));
    }

    #[test]
    fn other_formats_are_not_macho() {
        let fat64 = [
            &0xcafe_babf_u32.to_be_bytes()[..],
            &fat(&[(ARM64, executable(&[]))])[4..],
        ]
        .concat();
        let swapped = [&0xfeed_facf_u32.to_be_bytes()[..], &executable(&[])[4..]].concat();
        let rows: [&[u8]; 7] = [
            b"",
            b"\xcf\xfa\xed",
            b"#!/bin/sh\n",
            b"\x7fELF\x02\x01\x01\x00",
            &put(&executable(&[]), 0, 0xfeed_face),
            &swapped,
            &fat64,
        ];
        for bytes in rows {
            assert_eq!(read(bytes), Err(NotMachO), "{bytes:02x?}");
        }
    }

    #[test]
    fn every_load_kind_is_collected_in_order_and_the_install_name_is_not() {
        let mut commands = vec![dylib(LC_ID_DYLIB, "/b/lib/self.dylib")];
        commands.extend(
            LOAD_KINDS
                .iter()
                .map(|(cmd, _)| dylib(*cmd, &format!("/usr/lib/{cmd:x}.dylib"))),
        );
        let expected: Vec<_> = LOAD_KINDS
            .iter()
            .map(|(cmd, kind)| (*kind, format!("/usr/lib/{cmd:x}.dylib")))
            .collect();

        assert_eq!(names(&library(&commands), CpuType::ARM64), expected);
    }

    #[test]
    fn an_unlisted_library_named_by_any_load_kind_refuses() {
        for (cmd, _) in LOAD_KINDS {
            let helper = executable(&[dylib(cmd, "/opt/homebrew/lib/libz.dylib")]);
            assert_eq!(
                refused(&helper, &[]),
                (Helper, unlisted("/opt/homebrew/lib/libz.dylib", None)),
                "{cmd:#x}"
            );
        }
    }

    #[test]
    fn an_rpath_or_dyld_environment_refuses_in_the_helper_and_in_a_library_with_clean_names() {
        let rpath = |path: &str| RpathPresent {
            path: path.to_owned(),
        };
        let environment = text(LC_DYLD_ENVIRONMENT, "DYLD_INSERT_LIBRARIES=/tmp/x.dylib");
        let llvm = "/opt/homebrew/opt/llvm/lib";
        let with_library =
            |extra: Vec<u8>| refused(&qualified_helper(), &[(LIBKRUN, libkrun_with(&[extra]))]);

        assert_eq!(
            refused(&executable(&[load(LIBSYSTEM), text(LC_RPATH, "/b/r")]), &[]),
            (Helper, rpath("/b/r"))
        );
        assert_eq!(
            with_library(text(LC_RPATH, llvm)),
            (Library(0), rpath(llvm))
        );
        assert_eq!(
            refused(&executable(std::slice::from_ref(&environment)), &[]),
            (Helper, DyldEnvironment)
        );
        assert_eq!(with_library(environment), (Library(0), DyldEnvironment));
        assert_eq!(with_library(text(LC_RPATH, "")), (Library(0), rpath("")));
    }

    #[test]
    fn an_unknown_load_command_refuses_only_when_dyld_requires_it() {
        const REQUIRED: u32 = 0x8000_0099;
        let required = executable(&[load(LIBSYSTEM), text(REQUIRED, "x")]);
        let optional = executable(&[load(LIBSYSTEM), text(0x99, "x")]);
        let unknown = UnknownRequiredCommand { cmd: REQUIRED };

        assert_eq!(
            read(&required).expect("it reads").required_unknown,
            [REQUIRED]
        );
        assert_eq!(refused(&required, &[]), (Helper, unknown.clone()));
        assert_eq!(
            refused(
                &qualified_helper(),
                &[(LIBKRUN, libkrun_with(&[text(REQUIRED, "x")]))]
            ),
            (Library(0), unknown)
        );
        assert!(
            read(&optional)
                .expect("it reads")
                .required_unknown
                .is_empty()
        );
        assert!(audit(&optional, &[]).is_ok());
        for known in [0x8000_0022, 0x8000_0028, 0x8000_0033, 0x8000_0034] {
            let image = executable(&[load(LIBSYSTEM), text(known, "x")]);
            assert!(audit(&image, &[]).is_ok(), "{known:#x}");
        }
    }

    #[test]
    fn rpath_names_bare_names_and_unknown_at_names_refuse_as_searched() {
        let searched = |name: &str| SearchedName {
            name: name.to_owned(),
        };
        for name in [
            "@rpath/libx.dylib",
            "libx.dylib",
            "lib/x.dylib",
            "",
            "@executable_pathx/x",
            "@loader_path",
            "@x/y",
        ] {
            assert_eq!(
                resolve_in(name, "/b/bin", &[LIBKRUN]),
                Err(searched(name)),
                "{name:?}"
            );
        }
        let helper = executable(&[load("@rpath/libkrun.1.19.4.dylib")]);
        assert_eq!(
            refused(&helper, &[]),
            (Helper, searched("@rpath/libkrun.1.19.4.dylib"))
        );
    }

    #[test]
    fn a_loader_relative_name_resolves_lexically_to_exactly_one_listed_library() {
        let listed = |name, loader_dir, listed| resolve_in(name, loader_dir, listed);
        let joined = |name: &str, loader_dir| match resolve_in(name, loader_dir, &[]) {
            Err(UnlistedLibrary { resolved, .. }) => resolved.expect("a joined path"),
            other => panic!("{name}: {other:?}"),
        };
        let dotted = "@executable_path/./../lib//libkrun.1.19.4.dylib";

        assert_eq!(
            listed(KRUN_NAME, BIN, &[LIBKRUN]),
            Ok(Resolution::Listed(0))
        );
        assert_eq!(
            listed(KRUN_NAME, BIN, &["/b/lib/x", LIBKRUN]),
            Ok(Resolution::Listed(1))
        );
        assert_eq!(listed(dotted, BIN, &[LIBKRUN]), Ok(Resolution::Listed(0)));
        assert_eq!(
            listed("@loader_path/../lib/d", BIN, &["/b/lib/d"]),
            Ok(Resolution::Listed(0))
        );
        assert_eq!(
            listed("@executable_path/../lib/d", "/c/lib", &["/b/lib/d"]),
            Ok(Resolution::Listed(0))
        );
        assert_eq!(joined(KRUN_NAME, BIN), LIBKRUN);
        assert_eq!(joined("@executable_path/../../../x.dylib", BIN), "/x.dylib");
        assert_eq!(joined("@executable_path/", BIN), BIN);
        assert_eq!(
            joined("@loader_path/libextra.dylib", "/b/lib"),
            "/b/lib/libextra.dylib"
        );
    }

    #[test]
    fn an_absolute_listed_path_is_listed_before_the_system_rule_and_compared_exactly() {
        let dotted = "/b/lib/./libkrun.1.19.4.dylib";

        assert_eq!(
            resolve_in(LIBKRUN, BIN, &[LIBKRUN]),
            Ok(Resolution::Listed(0))
        );
        assert_eq!(
            resolve_in(LIBSYSTEM, BIN, &[LIBSYSTEM]),
            Ok(Resolution::Listed(0))
        );
        assert_eq!(
            resolve_in(dotted, BIN, &[LIBKRUN]),
            Err(unlisted(dotted, None))
        );
    }

    const SYSTEM: [&str; 5] = [
        LIBSYSTEM,
        "/usr/lib/swift/libswiftCore.dylib",
        HYPERVISOR,
        "/usr/lib/cronx.dylib",
        "/System/Library/User Template/x.dylib",
    ];

    const NOT_SYSTEM: [&str; 15] = [
        "/usr/libx/e.dylib",
        "/usr/lib/./e.dylib",
        "/usr//lib/e.dylib",
        "/usr/lib/",
        "/usr/lib",
        "/System/Library",
        "/opt/homebrew/lib/libz.dylib",
        "/usr/local/lib/x.dylib",
        "/opt/homebrew/opt/llvm/lib/libkrunfw.5.dylib",
        "/USR/LIB/libSystem.B.dylib",
        "/System/library/Frameworks/x",
        "/usr/lib/libe\u{301}.dylib",
        "/usr/lib/lib\u{e9}.dylib",
        "/tmp/e.dylib",
        "/usr/lib/e.dylib\u{0}",
    ];

    #[test]
    fn only_a_clean_ascii_name_under_the_two_system_prefixes_is_system() {
        for name in SYSTEM {
            assert!(is_system_name(name), "{name}");
            assert_eq!(
                resolve_in(name, "/b/bin", &[LIBKRUN]),
                Ok(Resolution::System(name.to_owned()))
            );
        }
        for name in NOT_SYSTEM {
            assert!(!is_system_name(name), "{name:?}");
            assert_eq!(
                resolve_in(name, "/b/bin", &[LIBKRUN]),
                Err(unlisted(name, None)),
                "{name:?}"
            );
        }
        assert!(!is_system_name(KRUN_NAME));
    }

    #[test]
    fn a_dot_dot_after_a_directory_the_name_writes_refuses_and_a_leading_run_does_not() {
        let climbs = |name: &str| DotDotAfterNamedDirectory {
            name: name.to_owned(),
        };
        for (name, loader_dir, listed) in [
            (
                "@executable_path/sub/../libt.dylib",
                BIN,
                "/b/bin/libt.dylib",
            ),
            (
                "@executable_path/../sub/../lib/libt.dylib",
                BIN,
                "/b/lib/libt.dylib",
            ),
            ("@executable_path/../../x/../y", BIN, "/y"),
            ("@loader_path/a/../b.dylib", "/b/lib", "/b/lib/b.dylib"),
            ("@loader_path/./a/./../b.dylib", "/b/lib", "/b/lib/b.dylib"),
            ("/b/lib/../lib/libkrun.1.19.4.dylib", BIN, LIBKRUN),
            ("/usr/lib/swift/../libSystem.B.dylib", BIN, LIBSYSTEM),
            ("/usr/lib/../../tmp/e.dylib", BIN, LIBKRUN),
        ] {
            assert_eq!(
                resolve_in(name, loader_dir, &[listed]),
                Err(climbs(name)),
                "{name}"
            );
        }
        for name in [
            "/usr/lib/swift/../libSystem.B.dylib",
            "/usr/lib/../../tmp/e.dylib",
        ] {
            assert!(!is_system_name(name), "{name}");
        }
        for name in [
            KRUN_NAME,
            "@executable_path/../../b/lib/libkrun.1.19.4.dylib",
            "@executable_path/./../lib//libkrun.1.19.4.dylib",
        ] {
            assert_eq!(
                resolve_in(name, BIN, &[LIBKRUN]),
                Ok(Resolution::Listed(0)),
                "{name}"
            );
        }
        let helper = executable(&[load("@executable_path/sub/../libt.dylib")]);
        assert_eq!(
            refused(&helper, &[("/b/bin/libt.dylib", library(&[]))]),
            (Helper, climbs("@executable_path/sub/../libt.dylib"))
        );
    }

    #[test]
    fn the_qualified_shape_without_its_rpath_passes_and_records_each_resolution_once() {
        let helper = executable(&[
            load(KRUN_NAME),
            load(LIBSYSTEM),
            dylib(LC_LOAD_WEAK_DYLIB, LIBSYSTEM),
        ]);
        let audited = audit(&helper, &[(LIBKRUN, libkrun_with(&[]))]).expect("it passes");

        assert_eq!(
            audited.resolved,
            [(Helper, KRUN_NAME.to_owned(), Library(0))]
        );
        assert_eq!(
            audited.system_names,
            [LIBSYSTEM, HYPERVISOR, "/usr/lib/libiconv.2.dylib"]
        );
    }

    #[test]
    fn a_listed_librarys_own_unlisted_name_refuses_with_that_library_as_the_image() {
        let libkrun = libkrun_with(&[load("@loader_path/libextra.dylib")]);
        let fault = unlisted("@loader_path/libextra.dylib", Some("/b/lib/libextra.dylib"));

        assert_eq!(
            refused(&qualified_helper(), &[(LIBKRUN, libkrun)]),
            (Library(0), fault)
        );
    }

    #[test]
    fn library_to_library_names_resolve_against_the_helpers_and_their_own_directories() {
        let helper = executable(&[load("@executable_path/../lib/sub/a.dylib")]);
        let a = library(&[
            load("@executable_path/../lib/b.dylib"),
            load("@loader_path/../c.dylib"),
        ]);
        let libraries = [
            ("/b/lib/sub/a.dylib", a),
            ("/b/lib/b.dylib", library(&[])),
            ("/b/lib/c.dylib", library(&[])),
            ("/b/lib/unused.dylib", library(&[])),
        ];

        let audited = audit(&helper, &libraries).expect("it passes");

        assert_eq!(
            audited.resolved,
            [
                (
                    Helper,
                    "@executable_path/../lib/sub/a.dylib".to_owned(),
                    Library(0)
                ),
                (
                    Library(0),
                    "@executable_path/../lib/b.dylib".to_owned(),
                    Library(1)
                ),
                (Library(0), "@loader_path/../c.dylib".to_owned(), Library(2)),
            ]
        );
    }

    #[test]
    fn a_foreign_dynamic_linker_or_unreadable_image_refuses_with_its_role() {
        let foreign = image(ARM64, MH_EXECUTE, &[text(LC_LOAD_DYLINKER, "/tmp/dyld")]);
        let dylinker = |name: &str| UnexpectedDylinker {
            name: name.to_owned(),
        };
        let in_library = library(&[text(LC_LOAD_DYLINKER, "/usr/lib/dyld2")]);

        assert_eq!(refused(&foreign, &[]), (Helper, dylinker("/tmp/dyld")));
        assert_eq!(
            refused(&qualified_helper(), &[(LIBKRUN, in_library)]),
            (Library(0), dylinker("/usr/lib/dyld2"))
        );
        assert_eq!(
            refused(&qualified_helper(), &[(LIBKRUN, b"nope".to_vec())]),
            (Library(0), NotMachO)
        );
        assert_eq!(refused(b"", &[]), (Helper, NotMachO));
    }

    #[test]
    fn a_cut_or_inconsistent_header_load_command_or_name_refuses_malformed_where_it_breaks() {
        let one = executable(&[]);
        let named = library(&[load("/usr/lib/libSystem.B.dyl")]);
        let mut no_nul = named.clone();
        no_nul[56..88]
            .iter_mut()
            .filter(|byte| **byte == 0)
            .for_each(|byte| *byte = b'x');
        let mut not_utf8 = named.clone();
        not_utf8[57] = 0xff;
        let rows: [(&str, Vec<u8>, u64); 12] = [
            ("header cut short", one[..20].to_vec(), 0),
            ("ncmds past sizeofcmds", put(&one, 16, 3), 64),
            ("sizeofcmds past the file", put(&one, 20, 4096), 20),
            ("cmdsize 0", put(&one, 36, 0), 32),
            ("cmdsize not a multiple of 8", put(&one, 36, 20), 32),
            ("cmdsize past sizeofcmds", put(&one, 36, 40), 32),
            ("name offset past its command", put(&named, 40, 4096), 32),
            (
                "name offset inside the dylib header",
                put(&named, 40, 8),
                32,
            ),
            ("name without NUL", no_nul, 32),
            ("name not UTF-8", not_utf8, 32),
            (
                "dylib command too short",
                library(&[text(LC_LOAD_DYLIB, "/x")]),
                32,
            ),
            (
                "rpath command too short",
                put(&library(&[text(LC_RPATH, "")]), 36, 8),
                32,
            ),
        ];
        for (what, bytes, offset) in rows {
            assert!(
                matches!(read(&bytes), Err(Malformed { offset: at, .. }) if at == offset),
                "{what}: {:?}",
                read(&bytes)
            );
        }
    }

    #[test]
    fn a_fat_table_or_member_past_the_end_refuses_malformed_at_the_table() {
        let good = fat(&[(ARM64, executable(&[]))]);
        let be = |bytes: &[u8], at: usize, value: u32| {
            let mut bytes = bytes.to_vec();
            bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
            bytes
        };
        for bytes in [
            be(&good, 4, 1_000_000),
            be(&good, 20, 1_000_000),
            be(&be(&good, 16, u32::MAX), 20, u32::MAX),
            good[..12].to_vec(),
        ] {
            assert!(
                matches!(read(&bytes), Err(Malformed { offset: 8, .. })),
                "{:?}",
                read(&bytes)
            );
        }
    }

    proptest! {
        #[test]
        fn no_mutated_or_random_image_panics_the_reader_or_the_audit(
            seed in proptest::collection::vec(any::<u8>(), 0..256),
            flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 0..16),
            cut in any::<usize>(),
        ) {
            let _ = read(&seed);
            let thin = qualified_helper();
            let both = fat(&[(X86_64, thin.clone()), (ARM64, libkrun_with(&[text(LC_RPATH, "/r")]))]);
            for mut bytes in [thin, both] {
                for (at, value) in &flips {
                    let index = at % bytes.len();
                    bytes[index] = *value;
                }
                let _ = read(&bytes[..cut % (bytes.len() + 1)]);
                let _ = audit(&bytes, &[(LIBKRUN, bytes.clone())]);
            }
        }
    }

    #[test]
    fn the_read_stops_one_byte_past_the_bound_and_reports_a_failed_read() {
        struct Failing;
        impl std::io::Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from_raw_os_error(libc::EIO))
            }
        }

        assert_eq!(read_bounded(&[7u8; 16][..], 16), Ok(vec![7u8; 16]));
        assert_eq!(read_bounded(&[7u8; 17][..], 16), Err(TooLarge { max: 16 }));
        assert_eq!(
            read_bounded(std::io::repeat(1), 16),
            Err(TooLarge { max: 16 })
        );
        assert_eq!(
            read_bounded(Failing, 16),
            Err(Io {
                op: "pread",
                errno: libc::EIO
            })
        );
    }

    #[test]
    fn a_refusal_displays_its_image_and_rule() {
        let show = |image, fault| ClosureRefusal { image, fault }.to_string();

        assert_eq!(
            show(
                Library(0),
                RpathPresent {
                    path: "/l".to_owned()
                }
            ),
            "libraries[0]: LC_RPATH /l is a search path"
        );
        assert_eq!(
            show(Helper, unlisted("@loader_path/x", Some("/b/bin/x"))),
            "helper: @loader_path/x resolves to /b/bin/x, which is not a listed library"
        );
        assert_eq!(
            show(Helper, unlisted("/o/x", None)),
            "helper: /o/x is neither a listed library nor on the sealed system volume"
        );
        let name = || "/n".to_owned();
        for fault in [
            Unsupported,
            TooLarge { max: 1 },
            Io {
                op: "pread",
                errno: 5,
            },
            NotMachO,
            NoMemberForHost,
            Malformed {
                offset: 3,
                what: "header",
            },
            DyldEnvironment,
            UnexpectedDylinker { name: name() },
            SearchedName { name: name() },
            OsIdentity {
                name: "kern.osversion",
                errno: 2,
            },
            DotDotAfterNamedDirectory { name: name() },
            UnknownRequiredCommand { cmd: 0x8000_0099 },
        ] {
            assert!(
                show(Helper, fault.clone()).len() > "helper: ".len(),
                "{fault:?}"
            );
        }
    }

    #[test]
    fn a_volume_is_sealed_only_when_mounted_at_root_read_only_and_reporting_the_seal() {
        let sealed = VolumeFacts {
            mounted_at_root: true,
            read_only: true,
            seal: Some(true),
        };
        for mounted_at_root in [false, true] {
            for read_only in [false, true] {
                for seal in [None, Some(false), Some(true)] {
                    let facts = VolumeFacts {
                        mounted_at_root,
                        read_only,
                        seal,
                    };
                    assert_eq!(is_sealed_system_volume(facts), facts == sealed, "{facts:?}");
                }
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_macos_the_audit_refuses_unsupported() {
        let helper =
            crate::runtime::artifacts::verified_for_test(Helper, std::path::Path::new("/dev/null"));

        assert_eq!(
            audit_closure(&helper, &[]),
            Err(ClosureRefusal {
                image: Helper,
                fault: Unsupported
            })
        );
    }

    #[cfg(target_os = "macos")]
    mod darwin {
        use super::*;
        use crate::runtime::artifacts::verify_artifact;
        use crate::runtime::digest::Digest;
        use crate::runtime::recipe::Artifact;
        use crate::runtime::strict_json;
        use crate::runtime::test_fixtures::tree::ArtifactTree;
        use std::path::Path;
        use std::process::Command;

        fn cc(args: &[&Path]) {
            let status = Command::new("cc").args(args).status().expect("cc runs");
            assert!(status.success(), "cc {args:?}: {status}");
        }

        fn verified(role: ArtifactRole, path: &Path) -> VerifiedArtifact {
            let pin = Digest::of(&std::fs::read(path).expect("read"));
            let text =
                serde_json::json!({"path": path.to_str().expect("UTF-8"), "sha256": pin.hex()});
            let value = strict_json::parse_value(text.to_string().as_bytes()).expect("JSON");
            let artifact = Artifact::from_value(&value, "helper").expect("an artifact");
            verify_artifact(role, &artifact).unwrap_or_else(|refusal| panic!("{refusal}"))
        }

        fn bundle() -> (ArtifactTree, VerifiedArtifact, VerifiedArtifact) {
            let work = tempfile::tempdir().expect("a build directory");
            let file = |name: &str, text: &str| {
                let path = work.path().join(name);
                std::fs::write(&path, text).expect("write");
                path
            };
            let source = file("t.c", "int t_value(void) { return 7; }\n");
            let main = file(
                "main.c",
                "int t_value(void);\nint main(void) { return t_value(); }\n",
            );
            let (dylib, program) = (work.path().join("libt.dylib"), work.path().join("prog"));
            let install = Path::new("@executable_path/../lib/libt.dylib");
            cc(&[
                Path::new("-dynamiclib"),
                Path::new("-install_name"),
                install,
                Path::new("-o"),
                &dylib,
                &source,
            ]);
            cc(&[Path::new("-o"), &program, &main, &dylib]);
            let tree = ArtifactTree::new();
            tree.dir("bin", 0o755);
            tree.dir("lib", 0o755);
            let helper = tree.immutable_file("bin/prog", &std::fs::read(&program).expect("read"));
            let library =
                tree.immutable_file("lib/libt.dylib", &std::fs::read(&dylib).expect("read"));
            let (helper, library) = (verified(Helper, &helper), verified(Library(0), &library));
            (tree, helper, library)
        }

        fn sysctl(name: &str) -> String {
            let output = Command::new("/usr/sbin/sysctl")
                .args(["-n", name])
                .output()
                .expect("sysctl runs");
            String::from_utf8(output.stdout)
                .expect("UTF-8")
                .trim_end()
                .to_owned()
        }

        #[test]
        fn a_toolchain_built_program_and_dylib_in_the_bundle_shape_pass_and_fail_unlisted() {
            let (tree, helper, library) = bundle();

            let evidence = audit_closure(&helper, &[library]).expect("the closure passes");

            assert_eq!(
                evidence.resolved,
                [(
                    Helper,
                    "@executable_path/../lib/libt.dylib".to_owned(),
                    Library(0)
                )]
            );
            assert!(
                evidence.system_names.contains(&LIBSYSTEM.to_owned()),
                "{evidence:?}"
            );
            assert_eq!(evidence.os_build, sysctl("kern.osversion"));
            assert_eq!(evidence.os_product_version, sysctl("kern.osproductversion"));
            assert_eq!(
                evidence.loaded_images,
                ImageInventory::NotIndependentlyObserved
            );
            assert_eq!(evidence.runtime_loads, RuntimeLoads::NotAudited);
            let resolved = tree.join("lib/libt.dylib");
            let fault = unlisted("@executable_path/../lib/libt.dylib", resolved.to_str());
            assert_eq!(
                audit_closure(&helper, &[]),
                Err(ClosureRefusal {
                    image: Helper,
                    fault
                })
            );
        }

        #[test]
        fn a_system_name_is_on_the_sealed_volume_only_if_its_nearest_existing_path_is() {
            for name in [LIBSYSTEM, HYPERVISOR, "/usr/lib/no-such-dir/no-such.dylib"] {
                assert_eq!(on_sealed_volume(name), Ok(true), "{name}");
            }
            for name in [
                "/System/Library/Caches/x.dylib",
                "/System/Library/Speech/x.dylib",
                "/usr/lib/cron/x.dylib",
                "/usr/lib/cron",
                "/usr/lib/ruby/site_ruby/x.bundle",
                "/System/Library/PrivateFrameworks/MobileDevice.framework/MobileDevice",
                "/System/Library/Templates/Data/private/etc/localtime/x.dylib",
                "/System/Library/PrivateFrameworks/XPCServices/x.dylib",
            ] {
                assert_eq!(on_sealed_volume(name), Ok(false), "{name}");
            }
        }

        #[test]
        fn a_system_name_that_leaves_the_sealed_volume_refuses_in_the_helper_or_a_library() {
            let tree = ArtifactTree::new();
            tree.dir("bin", 0o755);
            tree.dir("lib", 0o755);
            let cron = || dylib(LC_LOAD_WEAK_DYLIB, "/usr/lib/cron/libx.dylib");
            let bytes = executable(&[load(LIBSYSTEM), cron()]);
            let helper = verified(Helper, &tree.immutable_file("bin/helper", &bytes));
            let bytes = executable(&[load(LIBSYSTEM), load("@executable_path/../lib/libx.dylib")]);
            let clean = verified(Helper, &tree.immutable_file("bin/clean", &bytes));
            let bytes = library(&[load(LIBSYSTEM), cron()]);
            let shim = verified(Library(0), &tree.immutable_file("lib/libx.dylib", &bytes));
            let refusal = |image| {
                Err(ClosureRefusal {
                    image,
                    fault: unlisted("/usr/lib/cron/libx.dylib", None),
                })
            };

            assert_eq!(audit_closure(&helper, &[]), refusal(Helper));
            assert_eq!(audit_closure(&clean, &[shim]), refusal(Library(0)));
        }

        #[test]
        fn the_root_volume_reports_the_seal_and_the_data_preboot_and_dev_volumes_do_not() {
            let facts = |path: &std::ffi::CStr| {
                volume_facts(path).unwrap_or_else(|error| panic!("{path:?}: {error}"))
            };
            let volume = |mounted_at_root, read_only, seal| VolumeFacts {
                mounted_at_root,
                read_only,
                seal,
            };

            assert_eq!(facts(c"/"), volume(true, true, Some(true)));
            assert_eq!(facts(c"/usr/lib"), volume(true, true, Some(true)));
            for path in [
                c"/System/Volumes/Data",
                c"/System/Volumes/Preboot",
                c"/System/Volumes/Preboot/Cryptexes/OS",
            ] {
                assert_eq!(facts(path), volume(false, false, Some(false)), "{path:?}");
            }
            assert_eq!(facts(c"/dev"), volume(false, false, None));
        }
    }
}
