use super::digest::{Digest, DigestError};
use super::path::{PathError, RecipePath};

/// One pinned file of a recipe: its canonical absolute path and the SHA-256
/// its bytes must hash to. Nothing has opened or hashed the file yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    path: RecipePath,
    sha256: Digest,
}

impl Artifact {
    /// Where the file is. The path is canonical in spelling only.
    pub fn path(&self) -> &RecipePath {
        &self.path
    }

    /// The digest the file's bytes must have.
    pub fn sha256(&self) -> &Digest {
        &self.sha256
    }
}

/// How the helper loads the kernel: the six `KRUN_KERNEL_FORMAT_*` values of
/// the pinned libkrun header, in their header order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KernelFormat {
    Raw,
    Elf,
    PeGz,
    ImageBz2,
    ImageGz,
    ImageZstd,
}

impl KernelFormat {
    /// The value the helper passes to `krun_set_kernel`: 0 for `Raw` through 5
    /// for `ImageZstd`.
    pub fn krun_value(self) -> u32 {
        todo!()
    }

    /// The recipe spelling: `raw`, `elf`, `pe_gz`, `image_bz2`, `image_gz` or
    /// `image_zstd`.
    pub fn as_str(self) -> &'static str {
        todo!()
    }
}

/// The CPU architecture a recipe's artifacts were built for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Architecture {
    Aarch64,
    X86_64,
}

impl Architecture {
    /// The recipe spelling: `aarch64` or `x86_64`.
    pub fn as_str(self) -> &'static str {
        todo!()
    }

    /// The architecture this binary was built for, or `None` for any other.
    pub fn host() -> Option<Architecture> {
        todo!()
    }
}

/// What a host admits. A recipe asking for more refuses before launch.
///
/// Product code takes these from [`PlatformLimits::host`]; the fields are
/// public so tests can state limits that do not depend on the machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlatformLimits {
    /// The only architecture a recipe may name.
    pub architecture: Architecture,
    /// The most vCPUs a recipe may request.
    pub max_vcpus: u8,
    /// The most guest memory, in MiB, a recipe may request.
    pub max_memory_mib: u32,
    /// The longest control or data socket path, in bytes, that fits
    /// `sockaddr_un.sun_path` with its terminating NUL.
    pub max_socket_path_bytes: usize,
}

impl PlatformLimits {
    /// Reads this machine's limits: its architecture, its available
    /// parallelism capped at 255 vCPUs, its physical memory in MiB capped at
    /// `u32::MAX`, and the `sun_path` capacity less one byte.
    ///
    /// Refuses with `UnknownArchitecture` on an architecture a recipe cannot
    /// name, and with `HostLimitUnavailable` when the system will not report
    /// its parallelism or memory.
    pub fn host() -> Result<PlatformLimits, RecipeRefusal> {
        todo!()
    }
}

/// Which artifact of a recipe a value belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArtifactRole {
    Kernel,
    Initramfs,
    RootImage,
    Helper,
    /// The library at this index of `libraries`.
    Library(usize),
    HostPolicy,
    GuestPolicy,
}

/// Which recipe field holds a path.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PathRole {
    /// The `path` of this artifact.
    Artifact(ArtifactRole),
    WritableRoot,
    ControlPath,
    DataPath,
}

/// The deployment input that launches one hardware cell, as spec 001 §4.2
/// defines it.
///
/// A value exists only after [`RuntimeRecipe::parse`] accepted every
/// structural rule; there is no other constructor and no setter. Parsing
/// touches no file: whether the artifacts exist and hash to their digests is
/// for the caller to check before launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRecipe {
    vcpus: u8,
    memory_mib: u32,
    kernel: Artifact,
    kernel_format: KernelFormat,
    initramfs: Artifact,
    root_image: Artifact,
    writable_root: RecipePath,
    control_path: RecipePath,
    data_path: RecipePath,
    helper: Artifact,
    libraries: Vec<Artifact>,
    host_policy: Artifact,
    guest_policy: Artifact,
    architecture: Architecture,
}

/// Why a record is not a recipe this host may launch. Each variant names the
/// first rule the record broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipeRefusal {
    /// The record holds `bytes` bytes before its newline; at most 1,048,576
    /// are allowed.
    TooLarge { bytes: usize },
    /// The record does not end in one newline, holds another newline, or holds
    /// a carriage return.
    NotOneLine,
    /// The record is not UTF-8.
    NotUtf8,
    /// The record is not one JSON value.
    NotJson { detail: String },
    /// `field` is not a JSON object; an empty `field` is the record itself.
    NotAnObject { field: String },
    /// An object key appears twice, at this JSON pointer.
    DuplicateField { path: String },
    /// `field` is not part of a recipe.
    UnknownField { field: String },
    /// `field` is required and absent.
    MissingField { field: String },
    /// `field` holds a value of the wrong JSON type or range.
    WrongType {
        field: String,
        expected: &'static str,
    },
    /// `version` is an integer other than 1.
    UnsupportedVersion { found: i128 },
    /// `field` is not 64 lowercase hexadecimal digits.
    BadDigest { field: String, fault: DigestError },
    /// `field` is not a canonical absolute path.
    BadPath { field: String, fault: PathError },
    /// `kernel_format` is not one of the six known spellings.
    UnknownKernelFormat { found: String },
    /// `architecture` is not `aarch64` or `x86_64`; from
    /// [`PlatformLimits::host`], the host's own architecture.
    UnknownArchitecture { found: String },
    /// `vcpus` is zero.
    ZeroVcpus,
    /// `memory_mib` is zero.
    ZeroMemory,
    /// `vcpus` exceeds the host's limit.
    VcpusAboveLimit { requested: u8, limit: u8 },
    /// `memory_mib` exceeds the host's limit.
    MemoryAboveLimit { requested: u32, limit: u32 },
    /// An `x86_64` recipe names kernel format `raw`, which the pinned libkrun
    /// boots without the initramfs and command line.
    UnadmittedBootPair,
    /// The recipe's architecture is not the host's.
    ArchitectureMismatch {
        recipe: Architecture,
        host: Architecture,
    },
    /// `control_path` and `data_path` are the same path.
    SocketPathsEqual,
    /// `field`, a socket path, is `bytes` long; `max` fit `sun_path`.
    SocketPathTooLong {
        field: &'static str,
        bytes: usize,
        max: usize,
    },
    /// One path fills two roles; `first` comes before `second` in field order.
    PathReused { first: PathRole, second: PathRole },
    /// The host would not report `limit`.
    HostLimitUnavailable { limit: &'static str },
}

impl RuntimeRecipe {
    /// Reads one ndjson record: one line of strict JSON ending in a single
    /// newline, at most 1,048,576 bytes before that newline, with no carriage
    /// return. Every field is required, unknown and repeated keys refuse, and
    /// the recipe must fit `limits`.
    ///
    /// Rules are checked in a fixed order and the first broken one is
    /// returned: framing, JSON, unknown then missing keys, field values in
    /// spec order, then the rules that compare fields with each other and
    /// with `limits`.
    pub fn parse(record: &[u8], limits: &PlatformLimits) -> Result<RuntimeRecipe, RecipeRefusal> {
        let _ = (record, limits);
        todo!()
    }

    /// The recipe as one compact JSON line ending in one newline: keys in
    /// spec order, artifacts as `{"path":..,"sha256":..}`, strings escaped
    /// as `serde_json` escapes them.
    ///
    /// The layout is fixed. The VMM helper and test runtimes read exactly
    /// this form, so a change to it is a protocol change.
    pub fn to_ndjson(&self) -> Vec<u8> {
        todo!()
    }

    /// Guest vCPUs, at least 1.
    pub fn vcpus(&self) -> u8 {
        self.vcpus
    }

    /// Guest memory in MiB, at least 1.
    pub fn memory_mib(&self) -> u32 {
        self.memory_mib
    }

    /// The guest kernel.
    pub fn kernel(&self) -> &Artifact {
        &self.kernel
    }

    /// How the helper loads the kernel.
    pub fn kernel_format(&self) -> KernelFormat {
        self.kernel_format
    }

    /// The protected initramfs.
    pub fn initramfs(&self) -> &Artifact {
        &self.initramfs
    }

    /// The immutable RAW root image the writable copy is made from.
    pub fn root_image(&self) -> &Artifact {
        &self.root_image
    }

    /// Where the per-cell writable copy of the root image goes.
    pub fn writable_root(&self) -> &RecipePath {
        &self.writable_root
    }

    /// The control socket path.
    pub fn control_path(&self) -> &RecipePath {
        &self.control_path
    }

    /// The data socket path.
    pub fn data_path(&self) -> &RecipePath {
        &self.data_path
    }

    /// The VMM helper executable.
    pub fn helper(&self) -> &Artifact {
        &self.helper
    }

    /// The helper's library closure, in recipe order; may be empty.
    pub fn libraries(&self) -> &[Artifact] {
        &self.libraries
    }

    /// The host sandbox policy.
    pub fn host_policy(&self) -> &Artifact {
        &self.host_policy
    }

    /// The guest policy the handshake compares by digest.
    pub fn guest_policy(&self) -> &Artifact {
        &self.guest_policy
    }

    /// The architecture the artifacts were built for; the host's.
    pub fn architecture(&self) -> Architecture {
        self.architecture
    }

    /// Every artifact with its role, in field order: kernel, initramfs,
    /// root image, helper, each library, host policy, guest policy.
    pub fn artifacts(&self) -> impl Iterator<Item = (ArtifactRole, &Artifact)> {
        std::iter::empty()
    }
}

impl std::fmt::Display for ArtifactRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::fmt::Display for PathRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::fmt::Display for RecipeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::error::Error for RecipeRefusal {}
