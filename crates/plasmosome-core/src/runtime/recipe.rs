use super::line::{LineFault, MAX_RECORD_BYTES, read_line};
use super::path::{PathError, RecipePath};
use plasmosome_backend::digest::{Digest, DigestError};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::collections::hash_map::Entry;

const VCPUS_RANGE: &str = "an integer from 1 to 255";

const MEMORY_RANGE: &str = "an integer from 1 to 4294967295";

const RECIPE_FIELDS: [&str; 15] = [
    "version",
    "vcpus",
    "memory_mib",
    "kernel",
    "kernel_format",
    "initramfs",
    "root_image",
    "writable_root",
    "control_path",
    "data_path",
    "helper",
    "libraries",
    "host_policy",
    "guest_policy",
    "architecture",
];

const ARTIFACT_FIELDS: [&str; 2] = ["path", "sha256"];

const PER_CELL_FIELDS: [&str; 3] = ["writable_root", "control_path", "data_path"];

const TEMPLATE_FIELDS: [&str; 12] = [
    "version",
    "vcpus",
    "memory_mib",
    "kernel",
    "kernel_format",
    "initramfs",
    "root_image",
    "helper",
    "libraries",
    "host_policy",
    "guest_policy",
    "architecture",
];

static ABSENT: Value = Value::Null;

/// One pinned file of a recipe: its canonical absolute path and the SHA-256
/// its bytes must hash to. Nothing has opened or hashed the file yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    path: RecipePath,
    sha256: Digest,
}

impl Artifact {
    /// Reads an artifact from a JSON object with exactly the keys `path` and
    /// `sha256`. Every refusal names its key under `field`, such as
    /// `cell_runtime.membraned.path`; a value that is not an object refuses as
    /// `NotAnObject` naming `field` itself. `value` must come from
    /// [`strict_json::parse_value`](super::strict_json::parse_value), which refuses repeated keys.
    pub fn from_value(value: &Value, field: &str) -> Result<Artifact, RecipeRefusal> {
        let [path_value, sha256_value] = fields(value, ARTIFACT_FIELDS, field)?;
        let path = path(path_value, format!("{field}.path"))?;
        let field = format!("{field}.sha256");
        let sha256 = Digest::parse_hex(text(sha256_value, &field)?)
            .map_err(|fault| RecipeRefusal::BadDigest { field, fault })?;
        Ok(Artifact { path, sha256 })
    }

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
    const ALL: [KernelFormat; 6] = [
        KernelFormat::Raw,
        KernelFormat::Elf,
        KernelFormat::PeGz,
        KernelFormat::ImageBz2,
        KernelFormat::ImageGz,
        KernelFormat::ImageZstd,
    ];

    /// The value the helper passes to `krun_set_kernel`: 0 for `Raw` through 5
    /// for `ImageZstd`.
    pub fn krun_value(self) -> u32 {
        match self {
            KernelFormat::Raw => 0,
            KernelFormat::Elf => 1,
            KernelFormat::PeGz => 2,
            KernelFormat::ImageBz2 => 3,
            KernelFormat::ImageGz => 4,
            KernelFormat::ImageZstd => 5,
        }
    }

    /// The recipe spelling: `raw`, `elf`, `pe_gz`, `image_bz2`, `image_gz` or
    /// `image_zstd`.
    pub fn as_str(self) -> &'static str {
        match self {
            KernelFormat::Raw => "raw",
            KernelFormat::Elf => "elf",
            KernelFormat::PeGz => "pe_gz",
            KernelFormat::ImageBz2 => "image_bz2",
            KernelFormat::ImageGz => "image_gz",
            KernelFormat::ImageZstd => "image_zstd",
        }
    }
}

/// The CPU architecture a recipe's artifacts were built for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Architecture {
    Aarch64,
    X86_64,
}

impl Architecture {
    const ALL: [Architecture; 2] = [Architecture::Aarch64, Architecture::X86_64];

    /// The recipe spelling: `aarch64` or `x86_64`.
    pub fn as_str(self) -> &'static str {
        match self {
            Architecture::Aarch64 => "aarch64",
            Architecture::X86_64 => "x86_64",
        }
    }

    fn host() -> Option<Architecture> {
        if cfg!(target_arch = "aarch64") {
            Some(Architecture::Aarch64)
        } else if cfg!(target_arch = "x86_64") {
            Some(Architecture::X86_64)
        } else {
            None
        }
    }
}

/// What a host admits. A recipe asking for more refuses before launch.
///
/// Outside tests the only way to get one is [`PlatformLimits::host`], so a
/// caller cannot raise a limit above what the machine reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlatformLimits {
    architecture: Architecture,
    max_vcpus: u8,
    max_memory_mib: u32,
    max_socket_path_bytes: usize,
}

impl PlatformLimits {
    #[cfg(test)]
    pub(crate) const fn new(
        architecture: Architecture,
        max_vcpus: u8,
        max_memory_mib: u32,
        max_socket_path_bytes: usize,
    ) -> PlatformLimits {
        PlatformLimits {
            architecture,
            max_vcpus,
            max_memory_mib,
            max_socket_path_bytes,
        }
    }

    /// The only architecture a recipe may name.
    pub fn architecture(&self) -> Architecture {
        self.architecture
    }

    /// The most vCPUs a recipe may request.
    pub fn max_vcpus(&self) -> u8 {
        self.max_vcpus
    }

    /// The most guest memory, in MiB, a recipe may request.
    pub fn max_memory_mib(&self) -> u32 {
        self.max_memory_mib
    }

    /// The longest control or data socket path, in bytes, that fits
    /// `sockaddr_un.sun_path` with its terminating NUL.
    pub fn max_socket_path_bytes(&self) -> usize {
        self.max_socket_path_bytes
    }

    /// Reads this machine's limits: its architecture, its available
    /// parallelism capped at 255 vCPUs, its physical memory in MiB capped at
    /// `u32::MAX`, and the `sun_path` capacity less one byte.
    ///
    /// Refuses with `UnknownArchitecture` on an architecture a recipe cannot
    /// name, and with `Unavailable` when the system will not report its
    /// parallelism or memory.
    pub fn host() -> Result<PlatformLimits, HostLimitsError> {
        let architecture =
            Architecture::host().ok_or_else(|| HostLimitsError::UnknownArchitecture {
                found: std::env::consts::ARCH.to_owned(),
            })?;
        let parallelism =
            std::thread::available_parallelism().map_err(|_| HostLimitsError::Unavailable {
                limit: "available parallelism",
            })?;
        Ok(PlatformLimits {
            architecture,
            max_vcpus: capped_vcpus(parallelism.get()),
            max_memory_mib: physical_memory_mib()?,
            max_socket_path_bytes: array_length(|address: &libc::sockaddr_un| &address.sun_path)
                - 1,
        })
    }
}

#[allow(unsafe_code)]
fn physical_memory_mib() -> Result<u32, HostLimitsError> {
    let positive = |value: libc::c_long| u64::try_from(value).ok().filter(|value| *value > 0);
    // SAFETY: sysconf takes no pointer and has no memory precondition. A result of -1 or 0 is
    // refused below as Unavailable.
    let pages = positive(unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) });
    // SAFETY: sysconf takes no pointer and has no memory precondition. A result of -1 or 0 is
    // refused below as Unavailable.
    let page_bytes = positive(unsafe { libc::sysconf(libc::_SC_PAGESIZE) });
    let (Some(pages), Some(page_bytes)) = (pages, page_bytes) else {
        return Err(HostLimitsError::Unavailable {
            limit: "physical memory",
        });
    };
    Ok(capped_mib(
        u128::from(pages) * u128::from(page_bytes) / (1 << 20),
    ))
}

fn capped_vcpus(parallelism: usize) -> u8 {
    u8::try_from(parallelism).unwrap_or(u8::MAX)
}

fn capped_mib(mib: u128) -> u32 {
    u32::try_from(mib).unwrap_or(u32::MAX)
}

fn array_length<S, T, const N: usize>(_field: fn(&S) -> &[T; N]) -> usize {
    N
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
/// A value exists only after [`RuntimeRecipe::parse`],
/// [`RuntimeTemplate::complete`] or
/// [`SupervisorConfig::decode`](super::supervisor_config::SupervisorConfig::decode)
/// accepted every structural rule; there is no other constructor and no
/// setter.
/// Reading touches no file: whether the artifacts exist and hash to their
/// digests is for the caller to check before launch.
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
    /// A key appears twice in one object; `field` is its dotted name, such
    /// as `libraries[0].sha256`.
    DuplicateField { field: String },
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
    /// `architecture` is not `aarch64` or `x86_64`.
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
    /// `control_path` and `data_path` have the same spelling. Paths spelled
    /// differently may still name one file, for example on a case-insensitive
    /// file system.
    SocketPathsEqual,
    /// `field`, a socket path, is `bytes` long; `max` fit `sun_path`.
    SocketPathTooLong {
        field: &'static str,
        bytes: usize,
        max: usize,
    },
    /// One spelling fills two path roles; `first` comes before `second` in
    /// field order. Paths spelled differently may still name one file.
    PathReused { first: PathRole, second: PathRole },
}

/// Why [`PlatformLimits::host`] could not read this machine's limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostLimitsError {
    /// The host's own architecture, `found`, is neither `aarch64` nor `x86_64`.
    UnknownArchitecture { found: String },
    /// The host would not report `limit`.
    Unavailable { limit: &'static str },
}

impl RuntimeRecipe {
    /// Reads one ndjson record: one line of strict JSON ending in a single
    /// newline, at most 1,048,576 bytes before that newline, with no carriage
    /// return. Every field is required, unknown and repeated keys refuse, and
    /// the recipe must fit `limits`. The line is checked before the size, so a
    /// buffer that does not end in a newline refuses as `NotOneLine` whatever
    /// its length: a reader that stops at a cap must report an oversized
    /// recipe itself.
    ///
    /// Rules are checked in a fixed order and the first broken one is
    /// returned: framing, JSON, the recipe's unknown then missing keys, each
    /// field's value on its own in spec order, then a final pass. That pass
    /// refuses zero vCPUs or memory first, then compares fields with each
    /// other and with `limits`. So `vcpus: 0` with an unknown `kernel_format`
    /// refuses as the unknown format. An artifact's own unknown and missing
    /// keys are checked when the value pass reaches its field.
    pub fn parse(record: &[u8], limits: &PlatformLimits) -> Result<RuntimeRecipe, RecipeRefusal> {
        let value = read_line(record).map_err(refused_line)?;
        RuntimeRecipe::from_value(&value, limits)
    }

    /// Reads a recipe from a JSON value, applying every rule of
    /// [`RuntimeRecipe::parse`] after its framing and JSON checks, in the same
    /// order. A recipe nested in a larger record is read this way. `value`
    /// must come from [`strict_json::parse_value`](super::strict_json::parse_value): a value read any other way
    /// may have dropped a repeated key, which this cannot see.
    pub(crate) fn from_value(
        value: &Value,
        limits: &PlatformLimits,
    ) -> Result<RuntimeRecipe, RecipeRefusal> {
        let [
            version,
            vcpus,
            memory_mib,
            kernel,
            kernel_format,
            initramfs,
            root_image,
            writable_root,
            control_path,
            data_path,
            helper,
            libraries,
            host_policy,
            guest_policy,
            architecture,
        ] = fields(value, RECIPE_FIELDS, "")?;
        check_version(version)?;
        let recipe = RuntimeRecipe {
            vcpus: whole(vcpus, "vcpus", VCPUS_RANGE)?,
            memory_mib: whole(memory_mib, "memory_mib", MEMORY_RANGE)?,
            kernel: Artifact::from_value(kernel, "kernel")?,
            kernel_format: spelled(
                kernel_format,
                "kernel_format",
                KernelFormat::ALL,
                KernelFormat::as_str,
                |found| RecipeRefusal::UnknownKernelFormat { found },
            )?,
            initramfs: Artifact::from_value(initramfs, "initramfs")?,
            root_image: Artifact::from_value(root_image, "root_image")?,
            writable_root: path(writable_root, "writable_root".to_owned())?,
            control_path: path(control_path, "control_path".to_owned())?,
            data_path: path(data_path, "data_path".to_owned())?,
            helper: Artifact::from_value(helper, "helper")?,
            libraries: library_list(libraries)?,
            host_policy: Artifact::from_value(host_policy, "host_policy")?,
            guest_policy: Artifact::from_value(guest_policy, "guest_policy")?,
            architecture: spelled(
                architecture,
                "architecture",
                Architecture::ALL,
                Architecture::as_str,
                |found| RecipeRefusal::UnknownArchitecture { found },
            )?,
        };
        recipe.admit(limits)?;
        Ok(recipe)
    }

    /// The recipe as one compact JSON line ending in one newline: keys in
    /// spec order, artifacts as `{"path":..,"sha256":..}`, strings escaped
    /// as `serde_json` escapes them.
    ///
    /// The layout is fixed so that a test runtime may find keys by fixed
    /// search; the pinned VMM helper parses the line as JSON in any key
    /// order. The in-repository helper accepts at most 1,048,577 bytes,
    /// including the newline, matching [`RuntimeRecipe::parse`]'s frame.
    /// A launcher must refuse a line longer than that before it starts the
    /// helper.
    pub fn to_ndjson(&self) -> Vec<u8> {
        let libraries: Vec<String> = self.libraries.iter().map(artifact_json).collect();
        format!(
            concat!(
                r#"{{"version":1,"vcpus":{},"memory_mib":{},"kernel":{},"kernel_format":{},"#,
                r#""initramfs":{},"root_image":{},"writable_root":{},"control_path":{},"#,
                r#""data_path":{},"helper":{},"libraries":[{}],"host_policy":{},"#,
                r#""guest_policy":{},"architecture":{}}}"#,
                "\n",
            ),
            self.vcpus,
            self.memory_mib,
            artifact_json(&self.kernel),
            quoted(self.kernel_format.as_str()),
            artifact_json(&self.initramfs),
            artifact_json(&self.root_image),
            quoted(self.writable_root.as_str()),
            quoted(self.control_path.as_str()),
            quoted(self.data_path.as_str()),
            artifact_json(&self.helper),
            libraries.join(","),
            artifact_json(&self.host_policy),
            artifact_json(&self.guest_policy),
            quoted(self.architecture.as_str()),
        )
        .into_bytes()
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
        self.leading_artifacts().chain(self.trailing_artifacts())
    }

    fn leading_artifacts(&self) -> impl Iterator<Item = (ArtifactRole, &Artifact)> {
        [
            (ArtifactRole::Kernel, &self.kernel),
            (ArtifactRole::Initramfs, &self.initramfs),
            (ArtifactRole::RootImage, &self.root_image),
        ]
        .into_iter()
    }

    fn trailing_artifacts(&self) -> impl Iterator<Item = (ArtifactRole, &Artifact)> {
        std::iter::once((ArtifactRole::Helper, &self.helper))
            .chain(
                self.libraries
                    .iter()
                    .enumerate()
                    .map(|(index, library)| (ArtifactRole::Library(index), library)),
            )
            .chain([
                (ArtifactRole::HostPolicy, &self.host_policy),
                (ArtifactRole::GuestPolicy, &self.guest_policy),
            ])
    }

    fn paths(&self) -> impl Iterator<Item = (PathRole, &RecipePath)> {
        self.leading_artifacts()
            .map(artifact_path)
            .chain([
                (PathRole::WritableRoot, &self.writable_root),
                (PathRole::ControlPath, &self.control_path),
                (PathRole::DataPath, &self.data_path),
            ])
            .chain(self.trailing_artifacts().map(artifact_path))
    }

    fn admit(&self, limits: &PlatformLimits) -> Result<(), RecipeRefusal> {
        if self.vcpus == 0 {
            return Err(RecipeRefusal::ZeroVcpus);
        }
        if self.memory_mib == 0 {
            return Err(RecipeRefusal::ZeroMemory);
        }
        if self.vcpus > limits.max_vcpus {
            return Err(RecipeRefusal::VcpusAboveLimit {
                requested: self.vcpus,
                limit: limits.max_vcpus,
            });
        }
        if self.memory_mib > limits.max_memory_mib {
            return Err(RecipeRefusal::MemoryAboveLimit {
                requested: self.memory_mib,
                limit: limits.max_memory_mib,
            });
        }
        if self.architecture == Architecture::X86_64 && self.kernel_format == KernelFormat::Raw {
            return Err(RecipeRefusal::UnadmittedBootPair);
        }
        if self.architecture != limits.architecture {
            return Err(RecipeRefusal::ArchitectureMismatch {
                recipe: self.architecture,
                host: limits.architecture,
            });
        }
        if self.control_path == self.data_path {
            return Err(RecipeRefusal::SocketPathsEqual);
        }
        for (field, path) in [
            ("control_path", &self.control_path),
            ("data_path", &self.data_path),
        ] {
            let bytes = path.as_str().len();
            if bytes > limits.max_socket_path_bytes {
                return Err(RecipeRefusal::SocketPathTooLong {
                    field,
                    bytes,
                    max: limits.max_socket_path_bytes,
                });
            }
        }
        match first_reuse(self.paths()) {
            Some((first, second)) => Err(RecipeRefusal::PathReused { first, second }),
            None => Ok(()),
        }
    }
}

/// Spec 023 §1's deployment template: a recipe object without the three
/// per-cell paths `writable_root`, `control_path` and `data_path`.
///
/// Reading a template checks only which keys it holds. The value of every key
/// is checked when [`RuntimeTemplate::complete`] makes a recipe from it, so a
/// caller that must refuse a bad template before any cell exists completes it
/// once with the paths it would use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTemplate {
    fields: Map<String, Value>,
}

/// Why a value is not a runtime template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateRefusal {
    /// The template carries `field`, one of the three paths the controller
    /// sets for each cell. Checked before any other key.
    PerCellField { field: &'static str },
    /// A recipe refusal, whose field names are relative to the template, such
    /// as `kernel.path`. [`RuntimeTemplate::from_value`] gives only key
    /// refusals here: the template is not a JSON object, or holds an unknown
    /// key, or lacks one of the other twelve recipe keys. A caller that wraps a
    /// refusal from [`RuntimeTemplate::complete`] in it carries any recipe
    /// refusal, value refusals included, and some of those come from the three
    /// per-cell paths rather than the template, such as `PathReused` naming
    /// `writable_root` or `SocketPathTooLong`.
    Recipe(RecipeRefusal),
}

impl RuntimeTemplate {
    /// Reads a template from a JSON object. Refuses a value that is not an
    /// object, then any of the three per-cell keys in that order, then an
    /// unknown key (the smallest by bytes), then a missing key in spec order.
    /// `value` must come from [`strict_json::parse_value`](super::strict_json::parse_value), which refuses
    /// repeated keys.
    pub fn from_value(value: &Value) -> Result<RuntimeTemplate, TemplateRefusal> {
        let Value::Object(entries) = value else {
            return Err(TemplateRefusal::Recipe(RecipeRefusal::NotAnObject {
                field: String::new(),
            }));
        };
        if let Some(field) = PER_CELL_FIELDS
            .into_iter()
            .find(|field| entries.contains_key(*field))
        {
            return Err(TemplateRefusal::PerCellField { field });
        }
        fields(value, TEMPLATE_FIELDS, "").map_err(TemplateRefusal::Recipe)?;
        Ok(RuntimeTemplate {
            fields: entries.clone(),
        })
    }

    /// Makes one cell's recipe: the template with the three per-cell paths
    /// added, read under `limits` by every rule [`RuntimeRecipe::parse`]
    /// applies after its framing and JSON checks. Every
    /// value rule and every cross-field rule applies to the result, so a
    /// refusal here may come from the template's own values.
    pub fn complete(
        &self,
        writable_root: &RecipePath,
        control_path: &RecipePath,
        data_path: &RecipePath,
        limits: &PlatformLimits,
    ) -> Result<RuntimeRecipe, RecipeRefusal> {
        let mut fields = self.fields.clone();
        for (key, path) in PER_CELL_FIELDS
            .into_iter()
            .zip([writable_root, control_path, data_path])
        {
            fields.insert(key.to_owned(), Value::from(path.as_str()));
        }
        RuntimeRecipe::from_value(&Value::Object(fields), limits)
    }
}

fn artifact_path((role, artifact): (ArtifactRole, &Artifact)) -> (PathRole, &RecipePath) {
    (PathRole::Artifact(role), &artifact.path)
}

fn refused_line(fault: LineFault) -> RecipeRefusal {
    match fault {
        LineFault::NotOneLine => RecipeRefusal::NotOneLine,
        LineFault::TooLarge { bytes } => RecipeRefusal::TooLarge { bytes },
        LineFault::NotUtf8 => RecipeRefusal::NotUtf8,
        LineFault::NotJson { detail } => RecipeRefusal::NotJson { detail },
        LineFault::DuplicateKey { field } => RecipeRefusal::DuplicateField { field },
    }
}

fn fields<'a, const N: usize>(
    value: &'a Value,
    names: [&str; N],
    at: &str,
) -> Result<[&'a Value; N], RecipeRefusal> {
    let Value::Object(entries) = value else {
        return Err(RecipeRefusal::NotAnObject {
            field: at.to_owned(),
        });
    };
    if let Some(unknown) = entries
        .keys()
        .filter(|key| !names.contains(&key.as_str()))
        .min()
    {
        return Err(RecipeRefusal::UnknownField {
            field: dotted(at, unknown),
        });
    }
    let mut found = [&ABSENT; N];
    for (slot, name) in found.iter_mut().zip(names) {
        *slot = entries
            .get(name)
            .ok_or_else(|| RecipeRefusal::MissingField {
                field: dotted(at, name),
            })?;
    }
    Ok(found)
}

fn dotted(at: &str, key: &str) -> String {
    if at.is_empty() {
        key.to_owned()
    } else {
        format!("{at}.{key}")
    }
}

fn wrong_type(field: &str, expected: &'static str) -> RecipeRefusal {
    RecipeRefusal::WrongType {
        field: field.to_owned(),
        expected,
    }
}

fn check_version(value: &Value) -> Result<(), RecipeRefusal> {
    let found = value
        .as_u64()
        .map(i128::from)
        .or_else(|| value.as_i64().map(i128::from));
    match found {
        Some(1) => Ok(()),
        Some(found) => Err(RecipeRefusal::UnsupportedVersion { found }),
        None => Err(wrong_type("version", "the integer 1")),
    }
}

fn whole<T: TryFrom<u64>>(
    value: &Value,
    field: &str,
    expected: &'static str,
) -> Result<T, RecipeRefusal> {
    value
        .as_u64()
        .and_then(|number| T::try_from(number).ok())
        .ok_or_else(|| wrong_type(field, expected))
}

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, RecipeRefusal> {
    value.as_str().ok_or_else(|| wrong_type(field, "a string"))
}

fn spelled<T: Copy, const N: usize>(
    value: &Value,
    field: &str,
    all: [T; N],
    spelling: fn(T) -> &'static str,
    refuse: fn(String) -> RecipeRefusal,
) -> Result<T, RecipeRefusal> {
    let found = text(value, field)?;
    all.into_iter()
        .find(|candidate| spelling(*candidate) == found)
        .ok_or_else(|| refuse(found.to_owned()))
}

fn path(value: &Value, field: String) -> Result<RecipePath, RecipeRefusal> {
    RecipePath::parse(text(value, &field)?).map_err(|fault| RecipeRefusal::BadPath { field, fault })
}

fn library_list(value: &Value) -> Result<Vec<Artifact>, RecipeRefusal> {
    let Value::Array(entries) = value else {
        return Err(wrong_type("libraries", "an array"));
    };
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| Artifact::from_value(entry, &format!("libraries[{index}]")))
        .collect()
}

fn first_reuse<'a>(
    paths: impl Iterator<Item = (PathRole, &'a RecipePath)>,
) -> Option<(PathRole, PathRole)> {
    let mut seen: HashMap<&str, (usize, PathRole)> = HashMap::new();
    let mut first: Option<((usize, usize), (PathRole, PathRole))> = None;
    for (index, (role, path)) in paths.enumerate() {
        match seen.entry(path.as_str()) {
            Entry::Vacant(slot) => {
                slot.insert((index, role));
            }
            Entry::Occupied(held) => {
                let (earlier, earlier_role) = *held.get();
                if first.is_none_or(|(at, _)| (earlier, index) < at) {
                    first = Some(((earlier, index), (earlier_role, role)));
                }
            }
        }
    }
    first.map(|(_, pair)| pair)
}

fn quoted(text: &str) -> String {
    Value::from(text).to_string()
}

fn artifact_json(artifact: &Artifact) -> String {
    format!(
        r#"{{"path":{},"sha256":{}}}"#,
        quoted(artifact.path.as_str()),
        quoted(&artifact.sha256.hex())
    )
}

impl std::fmt::Display for ArtifactRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArtifactRole::Kernel => f.write_str("kernel"),
            ArtifactRole::Initramfs => f.write_str("initramfs"),
            ArtifactRole::RootImage => f.write_str("root_image"),
            ArtifactRole::Helper => f.write_str("helper"),
            ArtifactRole::Library(index) => write!(f, "libraries[{index}]"),
            ArtifactRole::HostPolicy => f.write_str("host_policy"),
            ArtifactRole::GuestPolicy => f.write_str("guest_policy"),
        }
    }
}

impl std::fmt::Display for PathRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathRole::Artifact(role) => write!(f, "{role}.path"),
            PathRole::WritableRoot => f.write_str("writable_root"),
            PathRole::ControlPath => f.write_str("control_path"),
            PathRole::DataPath => f.write_str("data_path"),
        }
    }
}

impl std::fmt::Display for RecipeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecipeRefusal::TooLarge { bytes } => write!(
                f,
                "the recipe is {bytes} bytes before its newline; at most {MAX_RECORD_BYTES} are allowed"
            ),
            RecipeRefusal::NotOneLine => f.write_str(
                "the recipe is not one line ending in a single newline with no carriage return",
            ),
            RecipeRefusal::NotUtf8 => f.write_str("the recipe is not UTF-8"),
            RecipeRefusal::NotJson { detail } => write!(f, "the recipe is not JSON: {detail}"),
            RecipeRefusal::NotAnObject { field } if field.is_empty() => {
                f.write_str("the recipe is not a JSON object")
            }
            RecipeRefusal::NotAnObject { field } => write!(f, "`{field}` must be a JSON object"),
            RecipeRefusal::DuplicateField { field } => {
                write!(f, "the recipe repeats the field {field:?}")
            }
            RecipeRefusal::UnknownField { field } => {
                write!(f, "the recipe carries an unknown field {field:?}")
            }
            RecipeRefusal::MissingField { field } => write!(f, "the recipe is missing `{field}`"),
            RecipeRefusal::WrongType { field, expected } => write!(f, "`{field}` must be {expected}"),
            RecipeRefusal::UnsupportedVersion { found } => write!(
                f,
                "recipe version {found} is not supported; only version 1 is"
            ),
            RecipeRefusal::BadDigest { field, fault } => write!(f, "`{field}` is refused: {fault}"),
            RecipeRefusal::BadPath { field, fault } => write!(f, "`{field}` is refused: {fault}"),
            RecipeRefusal::UnknownKernelFormat { found } => write!(
                f,
                "`kernel_format` {found:?} is not one of raw, elf, pe_gz, image_bz2, image_gz or image_zstd"
            ),
            RecipeRefusal::UnknownArchitecture { found } => {
                write!(f, "architecture {found:?} is neither aarch64 nor x86_64")
            }
            RecipeRefusal::ZeroVcpus => write!(f, "`vcpus` must be {VCPUS_RANGE}"),
            RecipeRefusal::ZeroMemory => write!(f, "`memory_mib` must be {MEMORY_RANGE}"),
            RecipeRefusal::VcpusAboveLimit { requested, limit } => write!(
                f,
                "`vcpus` is {requested}; this host admits at most {limit}"
            ),
            RecipeRefusal::MemoryAboveLimit { requested, limit } => write!(
                f,
                "`memory_mib` is {requested}; this host admits at most {limit}"
            ),
            RecipeRefusal::UnadmittedBootPair => f.write_str(
                "kernel format raw is not admitted for x86_64: the pinned libkrun boots it without the initramfs and command line",
            ),
            RecipeRefusal::ArchitectureMismatch { recipe, host } => write!(
                f,
                "the recipe is built for {} but this host is {}",
                recipe.as_str(),
                host.as_str()
            ),
            RecipeRefusal::SocketPathsEqual => {
                f.write_str("`control_path` and `data_path` are the same path")
            }
            RecipeRefusal::SocketPathTooLong { field, bytes, max } => write!(
                f,
                "`{field}` is {bytes} bytes; a socket path on this host holds at most {max}"
            ),
            RecipeRefusal::PathReused { first, second } => {
                write!(f, "`{first}` and `{second}` are the same path")
            }
        }
    }
}

impl std::error::Error for RecipeRefusal {}

impl std::fmt::Display for HostLimitsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostLimitsError::UnknownArchitecture { found } => write!(
                f,
                "this host's architecture {found:?} is neither aarch64 nor x86_64"
            ),
            HostLimitsError::Unavailable { limit } => {
                write!(f, "this host did not report its {limit}")
            }
        }
    }
}

impl std::error::Error for HostLimitsError {}

impl std::fmt::Display for TemplateRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateRefusal::PerCellField { field } => write!(
                f,
                "the template carries `{field}`, which the controller sets for each cell"
            ),
            TemplateRefusal::Recipe(refusal) => refusal.fmt(f),
        }
    }
}

impl std::error::Error for TemplateRefusal {}

#[cfg(test)]
mod tests {
    use super::RecipeRefusal::*;
    use super::*;
    use crate::runtime::fixtures::{LIMITS, artifact, digest, valid_recipe as valid};
    use crate::runtime::strict_json;
    use proptest::prelude::*;
    use serde_json::{Value, json};

    const X86_64_LIMITS: PlatformLimits = PlatformLimits::new(Architecture::X86_64, 8, 16_384, 103);

    const ROOMY: PlatformLimits =
        PlatformLimits::new(Architecture::Aarch64, u8::MAX, u32::MAX, 103);

    const FIELDS: [&str; 15] = [
        "version",
        "vcpus",
        "memory_mib",
        "kernel",
        "kernel_format",
        "initramfs",
        "root_image",
        "writable_root",
        "control_path",
        "data_path",
        "helper",
        "libraries",
        "host_policy",
        "guest_policy",
        "architecture",
    ];

    const ARTIFACTS: [(&str, &str); 7] = [
        ("/kernel", "kernel"),
        ("/initramfs", "initramfs"),
        ("/root_image", "root_image"),
        ("/helper", "helper"),
        ("/libraries/0", "libraries[0]"),
        ("/host_policy", "host_policy"),
        ("/guest_policy", "guest_policy"),
    ];

    const PATHS: [(&str, &str); 10] = [
        ("/kernel/path", "kernel.path"),
        ("/initramfs/path", "initramfs.path"),
        ("/root_image/path", "root_image.path"),
        ("/writable_root", "writable_root"),
        ("/control_path", "control_path"),
        ("/data_path", "data_path"),
        ("/helper/path", "helper.path"),
        ("/libraries/0/path", "libraries[0].path"),
        ("/host_policy/path", "host_policy.path"),
        ("/guest_policy/path", "guest_policy.path"),
    ];

    const GOLDEN: &str = concat!(
        r#"{"version":1,"vcpus":2,"memory_mib":2048,"#,
        r#""kernel":{"path":"/opt/cell/kernel","sha256":"1111111111111111111111111111111111111111111111111111111111111111"},"#,
        r#""kernel_format":"raw","#,
        r#""initramfs":{"path":"/opt/cell/initramfs","sha256":"2222222222222222222222222222222222222222222222222222222222222222"},"#,
        r#""root_image":{"path":"/opt/cell/root.raw","sha256":"3333333333333333333333333333333333333333333333333333333333333333"},"#,
        r#""writable_root":"/var/cell/c1/root.raw","#,
        r#""control_path":"/var/cell/c1/control.sock","#,
        r#""data_path":"/var/cell/c1/data.sock","#,
        r#""helper":{"path":"/opt/cell/krun-helper","sha256":"4444444444444444444444444444444444444444444444444444444444444444"},"#,
        r#""libraries":[{"path":"/opt/cell/lib/libkrun.1.dylib","sha256":"5555555555555555555555555555555555555555555555555555555555555555"}],"#,
        r#""host_policy":{"path":"/opt/cell/host.sb","sha256":"6666666666666666666666666666666666666666666666666666666666666666"},"#,
        r#""guest_policy":{"path":"/opt/cell/guest.json","sha256":"7777777777777777777777777777777777777777777777777777777777777777"},"#,
        r#""architecture":"aarch64"}"#,
        "\n",
    );

    fn record(edits: &[(&str, Option<Value>)]) -> Vec<u8> {
        let mut recipe = valid();
        for (pointer, replacement) in edits {
            let (parent, key) = pointer.rsplit_once('/').expect("a JSON pointer");
            let fields = recipe
                .pointer_mut(parent)
                .and_then(Value::as_object_mut)
                .expect("the pointer names a key of an object");
            match replacement {
                Some(value) => fields.insert(key.to_owned(), value.clone()),
                None => fields.remove(key),
            };
        }
        format!("{recipe}\n").into_bytes()
    }

    fn set(pointer: &str, value: Value) -> (&str, Option<Value>) {
        (pointer, Some(value))
    }

    fn with(pointer: &str, value: Value) -> Vec<u8> {
        record(&[set(pointer, value)])
    }

    fn without(pointer: &str) -> Vec<u8> {
        record(&[(pointer, None)])
    }

    fn body_value(record: &[u8]) -> Option<Value> {
        let body = record.strip_suffix(b"\n")?;
        strict_json::parse_value(body).ok()
    }

    fn parse_checked(
        record: &[u8],
        limits: &PlatformLimits,
    ) -> Result<RuntimeRecipe, RecipeRefusal> {
        let parsed = RuntimeRecipe::parse(record, limits);
        let framing = matches!(parsed, Err(NotOneLine | TooLarge { .. } | NotUtf8));
        if let (false, Some(value)) = (framing, body_value(record)) {
            assert_eq!(
                RuntimeRecipe::from_value(&value, limits),
                parsed,
                "from_value and parse disagree on {}",
                String::from_utf8_lossy(record)
            );
        }
        parsed
    }

    fn parsed_under(record: &[u8], limits: &PlatformLimits) -> RuntimeRecipe {
        parse_checked(record, limits).unwrap_or_else(|refusal| {
            panic!("{} refused: {refusal:?}", String::from_utf8_lossy(record))
        })
    }

    fn parsed(record: &[u8]) -> RuntimeRecipe {
        parsed_under(record, &LIMITS)
    }

    fn refused_under(record: &[u8], limits: &PlatformLimits) -> RecipeRefusal {
        match parse_checked(record, limits) {
            Err(refusal) => {
                let shown = refusal.to_string();
                assert!(!shown.contains(['\n', '\r']), "{shown:?} is one line");
                for name in names_shown_by(&refusal) {
                    assert!(shown.contains(&name), "{shown:?} names {name}");
                }
                refusal
            }
            Ok(recipe) => panic!("{} parsed: {recipe:?}", String::from_utf8_lossy(record)),
        }
    }

    fn names_shown_by(refusal: &RecipeRefusal) -> Vec<String> {
        match refusal {
            UnknownField { field } | DuplicateField { field } => vec![format!("{field:?}")],
            NotAnObject { field } if field.is_empty() => Vec::new(),
            NotAnObject { field }
            | MissingField { field }
            | WrongType { field, .. }
            | BadDigest { field, .. }
            | BadPath { field, .. } => vec![format!("`{field}`")],
            SocketPathTooLong { field, .. } => vec![format!("`{field}`")],
            PathReused { first, second } => vec![format!("`{first}`"), format!("`{second}`")],
            UnsupportedVersion { found } => vec![format!("version {found}")],
            UnknownKernelFormat { found } => vec![format!("`kernel_format` {found:?}")],
            UnknownArchitecture { found } => vec![format!("architecture {found:?}")],
            ZeroVcpus => vec!["`vcpus`".to_owned()],
            ZeroMemory => vec!["`memory_mib`".to_owned()],
            VcpusAboveLimit { requested, limit } => {
                vec![
                    format!("`vcpus` is {requested}"),
                    format!("at most {limit}"),
                ]
            }
            MemoryAboveLimit { requested, limit } => {
                vec![
                    format!("`memory_mib` is {requested}"),
                    format!("at most {limit}"),
                ]
            }
            UnadmittedBootPair => vec!["kernel format raw".to_owned(), "x86_64".to_owned()],
            ArchitectureMismatch { recipe, host } => vec![
                format!("built for {}", recipe.as_str()),
                format!("this host is {}", host.as_str()),
            ],
            SocketPathsEqual => vec!["`control_path`".to_owned(), "`data_path`".to_owned()],
            TooLarge { .. } | NotOneLine | NotUtf8 | NotJson { .. } => Vec::new(),
        }
    }

    fn refused(record: &[u8]) -> RecipeRefusal {
        refused_under(record, &LIMITS)
    }

    fn wrong_type(field: &str, expected: &'static str) -> RecipeRefusal {
        let field = field.to_owned();
        WrongType { field, expected }
    }

    fn missing(field: &str) -> RecipeRefusal {
        let field = field.to_owned();
        MissingField { field }
    }

    fn unknown(field: &str) -> RecipeRefusal {
        let field = field.to_owned();
        UnknownField { field }
    }

    fn not_an_object(field: &str) -> RecipeRefusal {
        let field = field.to_owned();
        NotAnObject { field }
    }

    fn duplicate(field: &str) -> RecipeRefusal {
        let field = field.to_owned();
        DuplicateField { field }
    }

    fn path_of_length(bytes: usize) -> Value {
        json!(format!(
            "/var/cell/{}",
            "s".repeat(bytes - "/var/cell/".len())
        ))
    }

    #[test]
    fn the_valid_recipe_parses_with_every_field_in_its_accessor() {
        let recipe = parsed(&record(&[]));
        assert_eq!((recipe.vcpus(), recipe.memory_mib()), (2, 2048));
        assert_eq!(recipe.kernel_format(), KernelFormat::Raw);
        assert_eq!(recipe.architecture(), Architecture::Aarch64);
        assert_eq!(recipe.writable_root().as_str(), "/var/cell/c1/root.raw");
        assert_eq!(recipe.control_path().as_str(), "/var/cell/c1/control.sock");
        assert_eq!(recipe.data_path().as_str(), "/var/cell/c1/data.sock");
        let listed: Vec<(ArtifactRole, &str, String)> = recipe
            .artifacts()
            .map(|(role, artifact)| (role, artifact.path().as_str(), artifact.sha256().hex()))
            .collect();
        let library = "/opt/cell/lib/libkrun.1.dylib";
        assert_eq!(
            listed,
            [
                (ArtifactRole::Kernel, "/opt/cell/kernel", digest('1')),
                (ArtifactRole::Initramfs, "/opt/cell/initramfs", digest('2')),
                (ArtifactRole::RootImage, "/opt/cell/root.raw", digest('3')),
                (ArtifactRole::Helper, "/opt/cell/krun-helper", digest('4')),
                (ArtifactRole::Library(0), library, digest('5')),
                (ArtifactRole::HostPolicy, "/opt/cell/host.sb", digest('6')),
                (
                    ArtifactRole::GuestPolicy,
                    "/opt/cell/guest.json",
                    digest('7')
                ),
            ]
        );
        let named = [
            recipe.kernel(),
            recipe.initramfs(),
            recipe.root_image(),
            recipe.helper(),
            &recipe.libraries()[0],
            recipe.host_policy(),
            recipe.guest_policy(),
        ];
        let in_order: Vec<&Artifact> = recipe.artifacts().map(|(_, artifact)| artifact).collect();
        assert_eq!(named.to_vec(), in_order);
        assert_eq!(recipe.libraries().len(), 1);
    }

    #[test]
    fn the_encoding_is_one_compact_line_with_keys_in_spec_order() {
        let encoded = parsed(&record(&[])).to_ndjson();
        assert_eq!(String::from_utf8(encoded).expect("UTF-8"), GOLDEN);
        let one = format!(
            r#""libraries":[{{"path":"/opt/cell/lib/libkrun.1.dylib","sha256":"{}"}}]"#,
            digest('5')
        );
        assert!(GOLDEN.contains(&one));
        let two = json!([
            artifact("/opt/cell/lib/a.dylib", 'a'),
            artifact("/opt/cell/lib/b.dylib", 'b'),
        ]);
        let two_listed = format!(
            r#"[{{"path":"/opt/cell/lib/a.dylib","sha256":"{}"}},{{"path":"/opt/cell/lib/b.dylib","sha256":"{}"}}]"#,
            digest('a'),
            digest('b')
        );
        for (libraries, listed) in [(json!([]), "[]".to_owned()), (two, two_listed)] {
            let encoded = parsed(&with("/libraries", libraries)).to_ndjson();
            let expected = GOLDEN.replace(&one, &format!(r#""libraries":{listed}"#));
            assert_eq!(String::from_utf8(encoded).expect("UTF-8"), expected);
        }
    }

    #[test]
    fn an_encoded_recipe_parses_back_to_itself() {
        let three = json!([
            artifact("/opt/cell/lib/a.dylib", 'a'),
            artifact("/opt/cell/lib/b.dylib", 'b'),
            artifact("/opt/cell/lib/c.dylib", 'c'),
        ]);
        let escaped = [
            set(
                "/writable_root",
                json!("/var/cell/a \"quoted\" name/back\\slash"),
            ),
            set("/control_path", json!("/var/cell/caf\u{e9}/\t\x07.sock")),
            set("/libraries/0/path", json!("/opt/cell/\u{2603} lib/\r\n")),
        ];
        for record in [
            record(&[]),
            with("/libraries", json!([])),
            with("/libraries", three),
            record(&escaped),
        ] {
            let recipe = parsed(&record);
            let encoded = recipe.to_ndjson();
            assert_eq!(
                parsed(&encoded),
                recipe,
                "{}",
                String::from_utf8_lossy(&encoded)
            );
        }
    }

    fn component() -> impl Strategy<Value = String> {
        let character = any::<char>().prop_filter("no / or NUL", |c| !matches!(c, '/' | '\0'));
        proptest::collection::vec(character, 1..6)
            .prop_map(String::from_iter)
            .prop_filter("not . or ..", |name| name != "." && name != "..")
    }

    proptest! {
        #[test]
        fn any_admitted_recipe_survives_its_encoding(
            vcpus in 1..=ROOMY.max_vcpus(),
            memory_mib in 1..=ROOMY.max_memory_mib(),
            names in proptest::collection::vec(component(), 0..5),
            name in component(),
        ) {
            let libraries: Vec<Value> = names
                .iter()
                .enumerate()
                .map(|(index, name)| artifact(&format!("/lib/{index}/{name}"), 'e'))
                .collect();
            let record = record(&[
                set("/vcpus", json!(vcpus)),
                set("/memory_mib", json!(memory_mib)),
                set("/libraries", Value::Array(libraries)),
                set("/writable_root", json!(format!("/w/{name}"))),
                set("/data_path", json!(format!("/d/{name}"))),
            ]);
            let recipe = parsed_under(&record, &ROOMY);
            prop_assert_eq!(parsed_under(&recipe.to_ndjson(), &ROOMY), recipe);
        }
    }

    fn with_byte_in_writable_root(byte: u8) -> Vec<u8> {
        let mut text = with("/writable_root", json!("/var/cell/X"));
        let at = text
            .iter()
            .position(|held| *held == b'X')
            .expect("X is in the record");
        text[at] = byte;
        text
    }

    #[test]
    fn a_record_that_is_not_exactly_one_line_refuses() {
        let line = record(&[]);
        let body = &line[..line.len() - 1];
        for case in [
            body.to_vec(),
            [line.as_slice(), &line].concat(),
            [line.as_slice(), b"\n"].concat(),
            [b"{\n".as_slice(), &line[1..]].concat(),
            [body, b"\r\n"].concat(),
            [b"\r".as_slice(), &line].concat(),
            [b"{\r".as_slice(), &line[1..]].concat(),
            with_byte_in_writable_root(b'\r'),
            Vec::new(),
        ] {
            let shown = String::from_utf8_lossy(&case).into_owned();
            assert_eq!(refused(&case), NotOneLine, "{shown:?}");
        }
    }

    #[test]
    fn a_record_that_is_not_utf8_refuses() {
        assert_eq!(refused(&with_byte_in_writable_root(0xff)), NotUtf8);
    }

    #[test]
    fn the_frame_bound_counts_the_bytes_before_the_newline() {
        let padded = |length: usize| {
            let mut text = record(&[]);
            text.pop();
            text.resize(length, b' ');
            text.push(b'\n');
            text
        };
        assert_eq!(parsed(&padded(1_048_576)), parsed(&record(&[])));
        assert_eq!(refused(&padded(1_048_577)), TooLarge { bytes: 1_048_577 });
    }

    #[test]
    fn framing_is_checked_line_then_size_then_encoding() {
        for length in [0, 1_048_576, 1_048_577, 1_048_578, 1_048_579, 8 * 1_048_576] {
            assert_eq!(refused(&vec![b' '; length]), NotOneLine, "{length} bytes");
        }
        let mut oversized = vec![b' '; 1_048_577];
        oversized[0] = 0xff;
        assert_eq!(refused(&oversized), NotOneLine);
        oversized.push(b'\n');
        assert_eq!(refused(&oversized), TooLarge { bytes: 1_048_577 });
        oversized[0] = b'\r';
        assert_eq!(refused(&oversized), NotOneLine);
    }

    #[test]
    fn a_record_that_is_not_one_json_value_refuses() {
        for text in [&b"\n"[..], b"{\"version\":1,\n", b"{} {}\n", b"[1e400]\n"] {
            let shown = String::from_utf8_lossy(text).into_owned();
            assert!(matches!(refused(text), NotJson { .. }), "{shown:?}");
        }
        let detail = "trailing characters at line 1, column 4".to_owned();
        assert_eq!(refused(b"{} {}\n"), NotJson { detail });
    }

    #[test]
    fn each_missing_field_refuses_by_name() {
        for field in FIELDS {
            assert_eq!(refused(&without(&format!("/{field}"))), missing(field));
        }
        for (pointer, name) in ARTIFACTS {
            for key in ["path", "sha256"] {
                let refusal = refused(&without(&format!("{pointer}/{key}")));
                assert_eq!(refusal, missing(&format!("{name}.{key}")));
            }
        }
    }

    #[test]
    fn unknown_keys_refuse_before_missing_ones_and_missing_ones_in_spec_order() {
        let both_missing = record(&[("/architecture", None), ("/vcpus", None)]);
        assert_eq!(refused(&both_missing), missing("vcpus"));
        let missing_and_unknown = record(&[("/vcpus", None), set("/extra", json!(1))]);
        assert_eq!(refused(&missing_and_unknown), unknown("extra"));
        let two_unknown = record(&[set("/zeta", json!(1)), set("/alpha", json!(1))]);
        assert_eq!(refused(&two_unknown), unknown("alpha"));
        let third_without_digest = json!([
            artifact("/opt/cell/lib/a.dylib", 'a'),
            artifact("/opt/cell/lib/b.dylib", 'b'),
            {"path": "/opt/cell/lib/c.dylib"},
        ]);
        let refusal = refused(&with("/libraries", third_without_digest));
        assert_eq!(refusal, missing("libraries[2].sha256"));
    }

    #[test]
    fn an_unknown_key_refuses_at_any_depth_with_its_dotted_name() {
        for (pointer, field) in [
            ("/extra", "extra"),
            ("/kernel/size", "kernel.size"),
            ("/libraries/0/size", "libraries[0].size"),
        ] {
            assert_eq!(refused(&with(pointer, json!(1))), unknown(field));
        }
    }

    #[test]
    fn a_value_of_the_wrong_shape_refuses_naming_its_field() {
        assert_eq!(refused(b"[]\n"), not_an_object(""));
        let kernel_as_text = with("/kernel", json!("/opt/cell/kernel"));
        assert_eq!(refused(&kernel_as_text), not_an_object("kernel"));
        let library_as_text = with("/libraries", json!(["/opt/cell/lib/a.dylib"]));
        assert_eq!(refused(&library_as_text), not_an_object("libraries[0]"));
        for libraries in [json!({}), json!(null)] {
            let refusal = refused(&with("/libraries", libraries));
            assert_eq!(refusal, wrong_type("libraries", "an array"));
        }
        for (pointer, field) in [
            ("/writable_root", "writable_root"),
            ("/kernel_format", "kernel_format"),
            ("/architecture", "architecture"),
            ("/helper/path", "helper.path"),
            ("/helper/sha256", "helper.sha256"),
        ] {
            assert_eq!(
                refused(&with(pointer, json!(1))),
                wrong_type(field, "a string")
            );
        }
    }

    #[test]
    fn a_repeated_key_refuses_wherever_it_appears() {
        let text = String::from_utf8(record(&[])).expect("the record is UTF-8");
        let kernel_twice = text.replacen('{', r#"{"kernel":0,"#, 1);
        assert_eq!(refused(kernel_twice.as_bytes()), duplicate("kernel"));
        for (opening, field) in [
            (r#""helper":{"#, "helper.sha256"),
            (r#""libraries":[{"#, "libraries[0].sha256"),
        ] {
            assert!(text.contains(opening));
            let repeated = format!(r#"{opening}"sha256":"{}","#, digest('4'));
            let sha256_twice = text.replacen(opening, &repeated, 1);
            assert_eq!(refused(sha256_twice.as_bytes()), duplicate(field));
        }
    }

    #[test]
    fn a_refusal_that_quotes_recipe_text_stays_on_one_line() {
        let text = String::from_utf8(record(&[])).expect("the record is UTF-8");
        let twice = text.replacen('{', r#"{"a\rb":0,"a\rb":1,"#, 1);
        let (format, architecture) = ("raw\n".to_owned(), "x86\n64".to_owned());
        for (record, refusal) in [
            (with("/ex\ntra", json!(1)), unknown("ex\ntra")),
            (with("/kernel/si\nze", json!(1)), unknown("kernel.si\nze")),
            (twice.into_bytes(), duplicate("a\rb")),
            (
                with("/kernel_format", json!(format)),
                UnknownKernelFormat { found: format },
            ),
            (
                with("/architecture", json!(architecture)),
                UnknownArchitecture {
                    found: architecture,
                },
            ),
        ] {
            assert_eq!(refused(&record), refusal);
            let shown = refusal.to_string();
            assert!(!shown.contains(['\n', '\r']), "{shown:?}");
        }
    }

    #[test]
    fn version_is_the_integer_one() {
        for found in [2, 0, -1] {
            let refusal = UnsupportedVersion {
                found: i128::from(found),
            };
            assert_eq!(refused(&with("/version", json!(found))), refusal);
        }
        let found = i128::from(u64::MAX);
        let refusal = refused(&with("/version", json!(u64::MAX)));
        assert_eq!(refusal, UnsupportedVersion { found });
        for wrong in [json!(1.0), json!("1"), json!(null), json!(true)] {
            let refusal = refused(&with("/version", wrong));
            assert_eq!(refusal, wrong_type("version", "the integer 1"));
        }
    }

    #[test]
    fn vcpus_and_memory_are_positive_and_within_the_host_limits() {
        let vcpus_type = wrong_type("vcpus", "an integer from 1 to 255");
        let memory_type = wrong_type("memory_mib", "an integer from 1 to 4294967295");
        let vcpus_text = "`vcpus` must be an integer from 1 to 255";
        assert_eq!(vcpus_type.to_string(), vcpus_text);
        assert_eq!(ZeroVcpus.to_string(), vcpus_text);
        let memory_text = "`memory_mib` must be an integer from 1 to 4294967295";
        assert_eq!(memory_type.to_string(), memory_text);
        assert_eq!(ZeroMemory.to_string(), memory_text);
        assert_eq!(refused(&with("/vcpus", json!(0))), ZeroVcpus);
        for wrong in [json!(256), json!(-1), json!(2.0), json!("2")] {
            assert_eq!(refused(&with("/vcpus", wrong)), vcpus_type);
        }
        let refusal = refused(&with("/vcpus", json!(9)));
        assert_eq!(
            refusal,
            VcpusAboveLimit {
                requested: 9,
                limit: 8
            }
        );
        assert_eq!(parsed(&with("/vcpus", json!(8))).vcpus(), 8);
        assert_eq!(refused(&with("/memory_mib", json!(0))), ZeroMemory);
        for wrong in [json!(4_294_967_296_u64), json!(-1), json!(2048.0)] {
            assert_eq!(refused(&with("/memory_mib", wrong)), memory_type);
        }
        let (requested, limit) = (16_385, 16_384);
        let refusal = refused(&with("/memory_mib", json!(requested)));
        assert_eq!(refusal, MemoryAboveLimit { requested, limit });
        assert_eq!(
            parsed(&with("/memory_mib", json!(limit))).memory_mib(),
            limit
        );
        let largest = record(&[
            set("/vcpus", json!(255)),
            set("/memory_mib", json!(u32::MAX)),
        ]);
        let recipe = parsed_under(&largest, &ROOMY);
        assert_eq!((recipe.vcpus(), recipe.memory_mib()), (255, u32::MAX));
    }

    #[test]
    fn every_kernel_format_maps_to_its_krun_value_in_spec_order() {
        for (spelling, krun) in [
            ("raw", 0),
            ("elf", 1),
            ("pe_gz", 2),
            ("image_bz2", 3),
            ("image_gz", 4),
            ("image_zstd", 5),
        ] {
            let format = parsed(&with("/kernel_format", json!(spelling))).kernel_format();
            assert_eq!((format.krun_value(), format.as_str()), (krun, spelling));
        }
        for found in ["RAW", "bzImage", ""] {
            let refusal = refused(&with("/kernel_format", json!(found)));
            let found = found.to_owned();
            assert_eq!(refusal, UnknownKernelFormat { found });
        }
    }

    #[test]
    fn x86_64_with_raw_refuses_on_either_host_before_the_architecture_compare() {
        let x86_64 = |format: &str| {
            record(&[
                set("/architecture", json!("x86_64")),
                set("/kernel_format", json!(format)),
            ])
        };
        for limits in [LIMITS, X86_64_LIMITS] {
            assert_eq!(refused_under(&x86_64("raw"), &limits), UnadmittedBootPair);
        }
        let (aarch64, x86) = (Architecture::Aarch64, Architecture::X86_64);
        let refusal = refused(&x86_64("elf"));
        assert_eq!(
            refusal,
            ArchitectureMismatch {
                recipe: x86,
                host: aarch64
            }
        );
        let refusal = refused_under(&record(&[]), &X86_64_LIMITS);
        assert_eq!(
            refusal,
            ArchitectureMismatch {
                recipe: aarch64,
                host: x86
            }
        );
        let recipe = RuntimeRecipe::parse(&x86_64("elf"), &X86_64_LIMITS).expect("x86_64 elf");
        assert_eq!(recipe.architecture().as_str(), "x86_64");
        for found in ["arm64", "AARCH64", ""] {
            let refusal = refused(&with("/architecture", json!(found)));
            let found = found.to_owned();
            assert_eq!(refusal, UnknownArchitecture { found });
        }
    }

    #[test]
    fn a_bad_digest_refuses_in_every_artifact_by_name() {
        for (pointer, name) in ARTIFACTS {
            let sha256 = format!("{pointer}/sha256");
            let field = format!("{name}.sha256");
            for (text, fault) in [
                ("A".repeat(64), DigestError::NotLowercaseHex { at: 0 }),
                ("a".repeat(63), DigestError::WrongLength { bytes: 63 }),
            ] {
                let field = field.clone();
                assert_eq!(
                    refused(&with(&sha256, json!(text))),
                    BadDigest { field, fault }
                );
            }
        }
    }

    #[test]
    fn a_bad_path_refuses_in_every_path_field_by_name() {
        for (pointer, name) in PATHS {
            for (text, fault) in [
                ("relative/path", PathError::NotAbsolute),
                ("/var//cell", PathError::EmptyComponent),
                ("/var/ce\0ll", PathError::Nul),
            ] {
                let field = name.to_owned();
                assert_eq!(
                    refused(&with(pointer, json!(text))),
                    BadPath { field, fault }
                );
            }
        }
    }

    #[test]
    fn socket_paths_are_distinct_and_fit_the_host_sun_path() {
        let same = with("/data_path", json!("/var/cell/c1/control.sock"));
        assert_eq!(refused(&same), SocketPathsEqual);
        for (pointer, field) in [
            ("/control_path", "control_path"),
            ("/data_path", "data_path"),
        ] {
            parsed(&with(pointer, path_of_length(103)));
            let refusal = refused(&with(pointer, path_of_length(104)));
            assert_eq!(
                refusal,
                SocketPathTooLong {
                    field,
                    bytes: 104,
                    max: 103
                }
            );
        }
        let accented =
            |count: usize, tail: &str| json!(format!("/var/cell/{}{tail}", "\u{e9}".repeat(count)));
        parsed(&with("/control_path", accented(46, "s")));
        let refusal = refused(&with("/control_path", accented(47, "")));
        let field = "control_path";
        assert_eq!(
            refusal,
            SocketPathTooLong {
                field,
                bytes: 104,
                max: 103
            }
        );
        let both_long = record(&[
            set("/control_path", path_of_length(105)),
            set("/data_path", path_of_length(104)),
        ]);
        let field = "control_path";
        assert_eq!(
            refused(&both_long),
            SocketPathTooLong {
                field,
                bytes: 105,
                max: 103
            }
        );
    }

    #[test]
    fn one_path_in_two_roles_refuses_naming_both() {
        use ArtifactRole::*;
        use PathRole::{Artifact as Of, ControlPath, DataPath, WritableRoot};
        for (pointer, path, first, second) in [
            (
                "/writable_root",
                "/opt/cell/root.raw",
                Of(RootImage),
                WritableRoot,
            ),
            (
                "/libraries/0/path",
                "/opt/cell/krun-helper",
                Of(Helper),
                Of(Library(0)),
            ),
            ("/data_path", "/opt/cell/host.sb", DataPath, Of(HostPolicy)),
            (
                "/guest_policy/path",
                "/opt/cell/kernel",
                Of(Kernel),
                Of(GuestPolicy),
            ),
            (
                "/initramfs/path",
                "/var/cell/c1/control.sock",
                Of(Initramfs),
                ControlPath,
            ),
        ] {
            assert_eq!(
                refused(&with(pointer, json!(path))),
                PathReused { first, second }
            );
        }
        let twins = json!([
            artifact("/opt/cell/lib/a.dylib", 'a'),
            artifact("/opt/cell/lib/a.dylib", 'b'),
        ]);
        let (first, second) = (Of(Library(0)), Of(Library(1)));
        assert_eq!(
            refused(&with("/libraries", twins)),
            PathReused { first, second }
        );
    }

    #[test]
    fn of_several_reused_paths_the_first_pair_in_field_order_is_reported() {
        let two_reuses = record(&[
            set("/root_image/path", json!("/opt/cell/initramfs")),
            set("/guest_policy/path", json!("/opt/cell/kernel")),
        ]);
        let first = PathRole::Artifact(ArtifactRole::Kernel);
        let second = PathRole::Artifact(ArtifactRole::GuestPolicy);
        assert_eq!(refused(&two_reuses), PathReused { first, second });
    }

    #[test]
    fn value_rules_come_before_the_cross_field_rules_in_a_fixed_order() {
        let (zero, over) = (json!(0), json!(9));
        let found = "bzImage".to_owned();
        let control = json!("/var/cell/c1/control.sock");
        let long = path_of_length(104);
        let cases = [
            (
                vec![
                    set("/vcpus", zero.clone()),
                    set("/kernel_format", json!(found)),
                ],
                UnknownKernelFormat { found },
            ),
            (
                vec![
                    set("/vcpus", zero.clone()),
                    set("/memory_mib", zero.clone()),
                ],
                ZeroVcpus,
            ),
            (
                vec![set("/memory_mib", zero), set("/vcpus", over.clone())],
                ZeroMemory,
            ),
            (
                vec![set("/vcpus", over), set("/memory_mib", json!(16_385))],
                VcpusAboveLimit {
                    requested: 9,
                    limit: 8,
                },
            ),
            (
                vec![
                    set("/memory_mib", json!(16_385)),
                    set("/architecture", json!("x86_64")),
                ],
                MemoryAboveLimit {
                    requested: 16_385,
                    limit: 16_384,
                },
            ),
            (
                vec![
                    set("/architecture", json!("x86_64")),
                    set("/kernel_format", json!("elf")),
                    set("/data_path", control),
                ],
                ArchitectureMismatch {
                    recipe: Architecture::X86_64,
                    host: Architecture::Aarch64,
                },
            ),
            (
                vec![
                    set("/control_path", long.clone()),
                    set("/data_path", long.clone()),
                ],
                SocketPathsEqual,
            ),
            (
                vec![
                    set("/control_path", long),
                    set("/writable_root", json!("/opt/cell/root.raw")),
                ],
                SocketPathTooLong {
                    field: "control_path",
                    bytes: 104,
                    max: 103,
                },
            ),
        ];
        for (edits, refusal) in cases {
            let record = record(&edits);
            assert_eq!(
                refused(&record),
                refusal,
                "{}",
                String::from_utf8_lossy(&record)
            );
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn the_host_limits_come_from_this_machine() {
        let limits = PlatformLimits::host().expect("this host reports its limits");
        assert_eq!(limits.architecture().as_str(), std::env::consts::ARCH);
        let parallelism = std::thread::available_parallelism().expect("a CPU count");
        assert_eq!(usize::from(limits.max_vcpus()), parallelism.get().min(255));
        let installed_mib = u32::try_from(installed_memory_bytes() >> 20).unwrap_or(u32::MAX);
        assert_eq!(limits.max_memory_mib(), installed_mib);
        let sun_path = if cfg!(target_os = "macos") { 103 } else { 107 };
        assert_eq!(limits.max_socket_path_bytes(), sun_path);
    }

    #[cfg(target_os = "macos")]
    fn installed_memory_bytes() -> u64 {
        let sysctl = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .expect("sysctl runs");
        let text = String::from_utf8(sysctl.stdout).expect("sysctl prints UTF-8");
        text.trim().parse().expect("hw.memsize is a byte count")
    }

    #[cfg(target_os = "linux")]
    fn installed_memory_bytes() -> u64 {
        let info = std::fs::read_to_string("/proc/meminfo").expect("/proc/meminfo reads");
        let total = info
            .lines()
            .find_map(|line| line.strip_prefix("MemTotal:"))
            .and_then(|value| value.trim().strip_suffix("kB"))
            .expect("/proc/meminfo has MemTotal in kB");
        total.trim().parse::<u64>().expect("MemTotal is a number") * 1024
    }

    #[test]
    fn host_readings_saturate_at_the_largest_value_a_recipe_can_hold() {
        assert_eq!(
            [254, 255, 256, 1_000].map(capped_vcpus),
            [254, 255, 255, 255]
        );
        let most = u128::from(u32::MAX);
        assert_eq!(
            [most - 1, most, most + 1, 1 << 40].map(capped_mib),
            [u32::MAX - 1, u32::MAX, u32::MAX, u32::MAX]
        );
    }

    #[test]
    fn host_limit_errors_say_what_the_host_did_not_report() {
        let unknown = HostLimitsError::UnknownArchitecture {
            found: "riscv64".to_owned(),
        };
        let unavailable = HostLimitsError::Unavailable {
            limit: "physical memory",
        };
        assert_eq!(
            [unknown.to_string(), unavailable.to_string()],
            [
                "this host's architecture \"riscv64\" is neither aarch64 nor x86_64",
                "this host did not report its physical memory",
            ]
        );
    }

    #[test]
    fn limits_report_what_they_were_built_with() {
        let limits = PlatformLimits::new(Architecture::X86_64, 3, 5, 7);
        assert_eq!(limits.architecture(), Architecture::X86_64);
        assert_eq!(
            (
                limits.max_vcpus(),
                limits.max_memory_mib(),
                limits.max_socket_path_bytes()
            ),
            (3, 5, 7)
        );
    }

    #[test]
    fn refusals_describe_themselves() {
        for (refusal, text) in [
            (not_an_object(""), "the recipe is not a JSON object"),
            (
                unknown("kernel.si\nze"),
                r#"the recipe carries an unknown field "kernel.si\nze""#,
            ),
            (
                missing("libraries[2].sha256"),
                "the recipe is missing `libraries[2].sha256`",
            ),
            (
                SocketPathTooLong {
                    field: "control_path",
                    bytes: 104,
                    max: 103,
                },
                "`control_path` is 104 bytes; a socket path on this host holds at most 103",
            ),
            (
                ArchitectureMismatch {
                    recipe: Architecture::X86_64,
                    host: Architecture::Aarch64,
                },
                "the recipe is built for x86_64 but this host is aarch64",
            ),
        ] {
            assert_eq!(refusal.to_string(), text);
        }
    }

    #[test]
    fn a_reused_path_is_shown_with_both_recipe_field_names() {
        use ArtifactRole::*;
        use PathRole::{Artifact as Of, ControlPath, DataPath, WritableRoot};
        let pairs = [
            (Of(Kernel), Of(Initramfs)),
            (Of(RootImage), WritableRoot),
            (ControlPath, DataPath),
            (Of(Helper), Of(Library(3))),
            (Of(HostPolicy), Of(GuestPolicy)),
        ];
        let shown = pairs.map(|(first, second)| PathReused { first, second }.to_string());
        assert_eq!(
            shown,
            [
                "`kernel.path` and `initramfs.path` are the same path",
                "`root_image.path` and `writable_root` are the same path",
                "`control_path` and `data_path` are the same path",
                "`helper.path` and `libraries[3].path` are the same path",
                "`host_policy.path` and `guest_policy.path` are the same path",
            ]
        );
    }

    #[test]
    fn from_value_matches_parse() {
        let control = json!("/var/cell/c1/control.sock");
        let records = [
            record(&[]),
            with("/libraries", json!([])),
            with("/vcpus", json!(9)),
            with("/vcpus", json!(0)),
            with("/memory_mib", json!(16_385)),
            with("/version", json!(2)),
            with("/kernel_format", json!("bzImage")),
            with("/architecture", json!("x86_64")),
            with("/data_path", control),
            with("/control_path", path_of_length(104)),
            with("/writable_root", json!("/opt/cell/root.raw")),
            with("/helper/sha256", json!("A".repeat(64))),
            with("/kernel/path", json!("relative/path")),
            with("/libraries/0/size", json!(1)),
            with("/extra", json!(1)),
            without("/guest_policy"),
            with("/libraries", json!({})),
            with("/kernel", json!("/opt/cell/kernel")),
            b"[]\n".to_vec(),
        ];
        for record in records {
            let shown = String::from_utf8_lossy(&record).into_owned();
            let value = body_value(&record).expect("the record is one strict JSON value");
            assert_eq!(
                RuntimeRecipe::from_value(&value, &LIMITS),
                RuntimeRecipe::parse(&record, &LIMITS),
                "{shown}"
            );
        }
        let recipe = parsed(&record(&[]));
        let value = body_value(&recipe.to_ndjson()).expect("the encoding is strict JSON");
        assert_eq!(RuntimeRecipe::from_value(&value, &LIMITS), Ok(recipe));
    }

    #[test]
    fn from_value_sees_no_framing() {
        let padded = format!("{}{}", valid(), " ".repeat(1_048_577));
        let value = strict_json::parse_value(padded.as_bytes()).expect("strict JSON");
        assert_eq!(
            RuntimeRecipe::from_value(&value, &LIMITS),
            Ok(parsed(&record(&[])))
        );
    }

    fn artifact_refused(value: &Value, field: &str) -> RecipeRefusal {
        match Artifact::from_value(value, field) {
            Err(refusal) => {
                let shown = refusal.to_string();
                assert!(!shown.contains(['\n', '\r']), "{shown:?} is one line");
                for name in names_shown_by(&refusal) {
                    assert!(shown.contains(&name), "{shown:?} names {name}");
                }
                refusal
            }
            Ok(read) => panic!("{value} read as {read:?}"),
        }
    }

    #[test]
    fn artifact_from_value_reads_exactly_path_and_sha256_naming_the_given_field() {
        let field = "cell_runtime.membraned";
        let membraned = "/usr/local/lib/plasmosome/membraned";
        let read = Artifact::from_value(&artifact(membraned, '8'), field)
            .expect("an object with exactly path and sha256 reads");
        assert_eq!(read.path().as_str(), membraned);
        assert_eq!(read.sha256().hex(), digest('8'));
        let sha256 = digest('8');
        let cases = [
            (
                json!({"path": membraned}),
                missing("cell_runtime.membraned.sha256"),
            ),
            (
                json!({"sha256": sha256}),
                missing("cell_runtime.membraned.path"),
            ),
            (
                json!({"path": membraned, "sha256": sha256, "size": 1}),
                unknown("cell_runtime.membraned.size"),
            ),
            (
                json!({"path": "relative/membraned", "sha256": sha256}),
                BadPath {
                    field: "cell_runtime.membraned.path".to_owned(),
                    fault: PathError::NotAbsolute,
                },
            ),
            (
                json!({"path": membraned, "sha256": "A".repeat(64)}),
                BadDigest {
                    field: "cell_runtime.membraned.sha256".to_owned(),
                    fault: DigestError::NotLowercaseHex { at: 0 },
                },
            ),
            (
                json!({"path": 1, "sha256": sha256}),
                wrong_type("cell_runtime.membraned.path", "a string"),
            ),
            (json!(membraned), not_an_object("cell_runtime.membraned")),
        ];
        for (value, refusal) in cases {
            assert_eq!(artifact_refused(&value, field), refusal, "{value}");
        }
        let elsewhere = json!({"path": "relative", "sha256": digest('8')});
        assert_eq!(
            artifact_refused(&elsewhere, "helper"),
            BadPath {
                field: "helper.path".to_owned(),
                fault: PathError::NotAbsolute,
            }
        );
    }

    const PER_CELL: [&str; 3] = ["writable_root", "control_path", "data_path"];

    const CONTROL: &str = "/var/cell/c1/control.sock";

    const DATA: &str = "/var/cell/c1/data.sock";

    fn template_with(edits: &[(&str, Option<Value>)]) -> Value {
        let mut template = valid();
        let fields = template
            .as_object_mut()
            .expect("the valid recipe is an object");
        for key in PER_CELL {
            fields.remove(key);
        }
        for (key, replacement) in edits {
            match replacement {
                Some(value) => fields.insert((*key).to_owned(), value.clone()),
                None => fields.remove(*key),
            };
        }
        template
    }

    fn template_read(value: &Value) -> RuntimeTemplate {
        RuntimeTemplate::from_value(value)
            .unwrap_or_else(|refusal| panic!("{value} refused as a template: {refusal:?}"))
    }

    fn template_refused(value: &Value) -> TemplateRefusal {
        match RuntimeTemplate::from_value(value) {
            Err(refusal) => {
                let shown = refusal.to_string();
                assert!(!shown.contains(['\n', '\r']), "{shown:?} is one line");
                let names = match &refusal {
                    TemplateRefusal::PerCellField { field } => vec![format!("`{field}`")],
                    TemplateRefusal::Recipe(inner) => names_shown_by(inner),
                };
                for name in names {
                    assert!(shown.contains(&name), "{shown:?} names {name}");
                }
                refusal
            }
            Ok(template) => panic!("{value} read as a template: {template:?}"),
        }
    }

    fn per_cell(field: &'static str) -> TemplateRefusal {
        TemplateRefusal::PerCellField { field }
    }

    fn recipe_path(text: &str) -> RecipePath {
        RecipePath::parse(text).unwrap_or_else(|fault| panic!("{text:?} parses, got {fault:?}"))
    }

    fn completed(
        template: &RuntimeTemplate,
        writable_root: &str,
        control_path: &str,
        data_path: &str,
    ) -> Result<RuntimeRecipe, RecipeRefusal> {
        template.complete(
            &recipe_path(writable_root),
            &recipe_path(control_path),
            &recipe_path(data_path),
            &LIMITS,
        )
    }

    #[test]
    fn template_refuses_each_per_cell_key() {
        for field in PER_CELL {
            let path = json!("/var/cell/c1/elsewhere");
            let carrying = template_with(&[(field, Some(path.clone()))]);
            assert_eq!(template_refused(&carrying), per_cell(field));
            let also_missing = template_with(&[(field, Some(path.clone())), ("vcpus", None)]);
            assert_eq!(template_refused(&also_missing), per_cell(field));
            let also_unknown = template_with(&[(field, Some(path)), ("extra", Some(json!(1)))]);
            assert_eq!(template_refused(&also_unknown), per_cell(field));
            let not_a_path = template_with(&[(field, Some(json!(null)))]);
            assert_eq!(template_refused(&not_a_path), per_cell(field));
        }
        let two = template_with(&[
            ("data_path", Some(json!("/var/cell/d"))),
            ("control_path", Some(json!("/var/cell/c"))),
        ]);
        assert_eq!(template_refused(&two), per_cell("control_path"));
        assert_eq!(template_refused(&valid()), per_cell("writable_root"));
    }

    #[test]
    fn template_refuses_a_missing_or_unknown_key() {
        let kept: Vec<&str> = FIELDS
            .into_iter()
            .filter(|field| !PER_CELL.contains(field))
            .collect();
        assert_eq!(kept.len(), 12);
        for field in kept {
            let refusal = template_refused(&template_with(&[(field, None)]));
            assert_eq!(refusal, TemplateRefusal::Recipe(missing(field)), "{field}");
        }
        let extra = template_with(&[("extra", Some(json!(1)))]);
        assert_eq!(
            template_refused(&extra),
            TemplateRefusal::Recipe(unknown("extra"))
        );
        let missing_and_unknown = template_with(&[("vcpus", None), ("extra", Some(json!(1)))]);
        assert_eq!(
            template_refused(&missing_and_unknown),
            TemplateRefusal::Recipe(unknown("extra"))
        );
        let two_missing = template_with(&[("architecture", None), ("memory_mib", None)]);
        assert_eq!(
            template_refused(&two_missing),
            TemplateRefusal::Recipe(missing("memory_mib"))
        );
        for value in [json!([]), json!("template"), json!(null)] {
            assert_eq!(
                template_refused(&value),
                TemplateRefusal::Recipe(not_an_object("")),
                "{value}"
            );
        }
    }

    #[test]
    fn complete_applies_admit() {
        let template = template_read(&template_with(&[]));
        assert_eq!(
            completed(&template, "/var/cell/c1/root.raw", CONTROL, DATA),
            Ok(parsed(&record(&[])))
        );
        let over = template_read(&template_with(&[("vcpus", Some(json!(9)))]));
        assert_eq!(
            completed(&over, "/var/cell/c1/root.raw", CONTROL, DATA),
            Err(VcpusAboveLimit {
                requested: 9,
                limit: 8
            })
        );
        assert_eq!(
            completed(&template, "/var/cell/c1/root.raw", CONTROL, CONTROL),
            Err(SocketPathsEqual)
        );
        let long = path_of_length(104);
        let long = long.as_str().expect("a path is a string");
        assert_eq!(
            completed(&template, "/var/cell/c1/root.raw", long, DATA),
            Err(SocketPathTooLong {
                field: "control_path",
                bytes: 104,
                max: 103
            })
        );
        assert_eq!(
            completed(&template, "/opt/cell/root.raw", CONTROL, DATA),
            Err(PathReused {
                first: PathRole::Artifact(ArtifactRole::RootImage),
                second: PathRole::WritableRoot
            })
        );
    }

    #[test]
    fn complete_applies_every_value_rule_the_template_deferred() {
        for (key, value, refusal) in [
            ("vcpus", json!("2"), wrong_type("vcpus", VCPUS_RANGE)),
            ("version", json!(2), UnsupportedVersion { found: 2 }),
            (
                "kernel_format",
                json!("bzImage"),
                UnknownKernelFormat {
                    found: "bzImage".to_owned(),
                },
            ),
            (
                "helper",
                json!({"path": "/opt/cell/krun-helper"}),
                missing("helper.sha256"),
            ),
        ] {
            let template = template_read(&template_with(&[(key, Some(value))]));
            assert_eq!(
                completed(&template, "/var/cell/c1/root.raw", CONTROL, DATA),
                Err(refusal),
                "{key}"
            );
        }
    }

    #[test]
    fn template_refusals_describe_themselves() {
        assert_eq!(
            per_cell("control_path").to_string(),
            "the template carries `control_path`, which the controller sets for each cell"
        );
        assert_eq!(
            TemplateRefusal::Recipe(missing("vcpus")).to_string(),
            "the recipe is missing `vcpus`"
        );
    }
}
