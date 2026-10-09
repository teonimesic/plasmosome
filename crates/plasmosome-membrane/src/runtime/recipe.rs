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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::{Value, json};

    const LIMITS: PlatformLimits = PlatformLimits {
        architecture: Architecture::Aarch64,
        max_vcpus: 8,
        max_memory_mib: 16_384,
        max_socket_path_bytes: 103,
    };

    const X86_64_LIMITS: PlatformLimits = PlatformLimits {
        architecture: Architecture::X86_64,
        ..LIMITS
    };

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

    fn digest(digit: char) -> String {
        digit.to_string().repeat(64)
    }

    fn artifact(path: &str, digit: char) -> Value {
        json!({"path": path, "sha256": digest(digit)})
    }

    fn valid() -> Value {
        json!({
            "version": 1,
            "vcpus": 2,
            "memory_mib": 2048,
            "kernel": artifact("/opt/cell/kernel", '1'),
            "kernel_format": "raw",
            "initramfs": artifact("/opt/cell/initramfs", '2'),
            "root_image": artifact("/opt/cell/root.raw", '3'),
            "writable_root": "/var/cell/c1/root.raw",
            "control_path": "/var/cell/c1/control.sock",
            "data_path": "/var/cell/c1/data.sock",
            "helper": artifact("/opt/cell/krun-helper", '4'),
            "libraries": [artifact("/opt/cell/lib/libkrun.1.dylib", '5')],
            "host_policy": artifact("/opt/cell/host.sb", '6'),
            "guest_policy": artifact("/opt/cell/guest.json", '7'),
            "architecture": "aarch64",
        })
    }

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

    fn with(pointer: &str, value: Value) -> Vec<u8> {
        record(&[(pointer, Some(value))])
    }

    fn without(pointer: &str) -> Vec<u8> {
        record(&[(pointer, None)])
    }

    fn parsed(record: &[u8]) -> RuntimeRecipe {
        RuntimeRecipe::parse(record, &LIMITS).unwrap_or_else(|refusal| {
            panic!("{} refused: {refusal:?}", String::from_utf8_lossy(record))
        })
    }

    fn refused_under(record: &[u8], limits: &PlatformLimits) -> RecipeRefusal {
        match RuntimeRecipe::parse(record, limits) {
            Err(refusal) => refusal,
            Ok(recipe) => panic!("{} parsed: {recipe:?}", String::from_utf8_lossy(record)),
        }
    }

    fn refused(record: &[u8]) -> RecipeRefusal {
        refused_under(record, &LIMITS)
    }

    fn wrong_type(field: &str, expected: &'static str) -> RecipeRefusal {
        RecipeRefusal::WrongType {
            field: field.to_owned(),
            expected,
        }
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
        assert_eq!(
            [
                recipe.writable_root().as_str(),
                recipe.control_path().as_str(),
                recipe.data_path().as_str(),
            ],
            [
                "/var/cell/c1/root.raw",
                "/var/cell/c1/control.sock",
                "/var/cell/c1/data.sock",
            ]
        );
        let listed: Vec<(ArtifactRole, &str, String)> = recipe
            .artifacts()
            .map(|(role, artifact)| (role, artifact.path().as_str(), artifact.sha256().hex()))
            .collect();
        assert_eq!(
            listed,
            [
                (ArtifactRole::Kernel, "/opt/cell/kernel", digest('1')),
                (ArtifactRole::Initramfs, "/opt/cell/initramfs", digest('2')),
                (ArtifactRole::RootImage, "/opt/cell/root.raw", digest('3')),
                (ArtifactRole::Helper, "/opt/cell/krun-helper", digest('4')),
                (
                    ArtifactRole::Library(0),
                    "/opt/cell/lib/libkrun.1.dylib",
                    digest('5')
                ),
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
        let golden = GOLDEN.to_owned();
        assert_eq!(
            String::from_utf8(parsed(&record(&[])).to_ndjson()).expect("the encoding is UTF-8"),
            golden
        );
    }

    #[test]
    fn an_encoded_recipe_parses_back_to_itself() {
        let three = json!([
            artifact("/opt/cell/lib/a.dylib", 'a'),
            artifact("/opt/cell/lib/b.dylib", 'b'),
            artifact("/opt/cell/lib/c.dylib", 'c'),
        ]);
        let escaped = [
            (
                "/writable_root",
                Some(json!("/var/cell/a \"quoted\" name/back\\slash")),
            ),
            (
                "/control_path",
                Some(json!("/var/cell/caf\u{e9}/\t\x07.sock")),
            ),
            (
                "/libraries/0/path",
                Some(json!("/opt/cell/\u{2603} lib/\r\n")),
            ),
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
        proptest::collection::vec(
            any::<char>().prop_filter("a component holds no / or NUL", |c| {
                !matches!(c, '/' | '\0')
            }),
            1..6,
        )
        .prop_map(String::from_iter)
        .prop_filter("a component is not . or ..", |name| {
            name != "." && name != ".."
        })
    }

    proptest! {
        #[test]
        fn any_admitted_recipe_survives_its_encoding(
            vcpus in 1..=LIMITS.max_vcpus,
            memory_mib in 1..=LIMITS.max_memory_mib,
            names in proptest::collection::vec(component(), 0..5),
            name in component(),
        ) {
            let libraries: Vec<Value> = names
                .iter()
                .enumerate()
                .map(|(index, name)| artifact(&format!("/lib/{index}/{name}"), 'e'))
                .collect();
            let record = record(&[
                ("/vcpus", Some(json!(vcpus))),
                ("/memory_mib", Some(json!(memory_mib))),
                ("/libraries", Some(Value::Array(libraries))),
                ("/writable_root", Some(json!(format!("/w/{name}")))),
                ("/data_path", Some(json!(format!("/d/{name}")))),
            ]);
            let recipe = parsed(&record);
            prop_assert_eq!(parsed(&recipe.to_ndjson()), recipe);
        }
    }

    #[test]
    fn a_record_that_is_not_exactly_one_line_refuses() {
        let line = record(&[]);
        let body = &line[..line.len() - 1];
        let carriage_return_in_a_string = {
            let mut text = with("/writable_root", json!("/var/cell/X"));
            let at = text
                .iter()
                .position(|byte| *byte == b'X')
                .expect("X is in the record");
            text[at] = b'\r';
            text
        };
        for case in [
            body.to_vec(),
            [line.as_slice(), &line].concat(),
            [line.as_slice(), b"\n"].concat(),
            [b"{\n".as_slice(), &line[1..]].concat(),
            [body, b"\r\n"].concat(),
            [b"\r".as_slice(), &line].concat(),
            [b"{\r".as_slice(), &line[1..]].concat(),
            carriage_return_in_a_string,
            Vec::new(),
        ] {
            assert_eq!(
                refused(&case),
                RecipeRefusal::NotOneLine,
                "{:?}",
                String::from_utf8_lossy(&case)
            );
        }
    }

    #[test]
    fn a_record_that_is_not_utf8_refuses() {
        let mut text = with("/writable_root", json!("/var/cell/X"));
        let at = text
            .iter()
            .position(|byte| *byte == b'X')
            .expect("X is in the record");
        text[at] = 0xff;
        assert_eq!(refused(&text), RecipeRefusal::NotUtf8);
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
        assert_eq!(
            refused(&padded(1_048_577)),
            RecipeRefusal::TooLarge { bytes: 1_048_577 }
        );
    }

    #[test]
    fn framing_is_checked_line_then_size_then_encoding() {
        let mut oversized = vec![b' '; 1_048_577];
        oversized[0] = 0xff;
        assert_eq!(refused(&oversized), RecipeRefusal::NotOneLine);
        oversized.push(b'\n');
        assert_eq!(
            refused(&oversized),
            RecipeRefusal::TooLarge { bytes: 1_048_577 }
        );
    }

    #[test]
    fn a_record_that_is_not_one_json_value_refuses() {
        for text in [&b"\n"[..], b"{\"version\":1,\n", b"{} {}\n", b"[1e400]\n"] {
            assert!(
                matches!(refused(text), RecipeRefusal::NotJson { .. }),
                "{:?}",
                String::from_utf8_lossy(text)
            );
        }
        assert_eq!(
            refused(b"{} {}\n"),
            RecipeRefusal::NotJson {
                detail: "trailing characters at line 1, column 4".to_owned()
            }
        );
    }

    #[test]
    fn each_missing_field_refuses_by_name() {
        for field in FIELDS {
            assert_eq!(
                refused(&without(&format!("/{field}"))),
                RecipeRefusal::MissingField {
                    field: field.to_owned()
                }
            );
        }
        for (pointer, name) in ARTIFACTS {
            for key in ["path", "sha256"] {
                assert_eq!(
                    refused(&without(&format!("{pointer}/{key}"))),
                    RecipeRefusal::MissingField {
                        field: format!("{name}.{key}")
                    }
                );
            }
        }
    }

    #[test]
    fn unknown_keys_refuse_before_missing_ones_and_missing_ones_in_spec_order() {
        assert_eq!(
            refused(&record(&[("/architecture", None), ("/vcpus", None)])),
            RecipeRefusal::MissingField {
                field: "vcpus".to_owned()
            }
        );
        assert_eq!(
            refused(&record(&[("/vcpus", None), ("/extra", Some(json!(1)))])),
            RecipeRefusal::UnknownField {
                field: "extra".to_owned()
            }
        );
        assert_eq!(
            refused(&record(&[
                ("/zeta", Some(json!(1))),
                ("/alpha", Some(json!(1)))
            ])),
            RecipeRefusal::UnknownField {
                field: "alpha".to_owned()
            }
        );
        assert_eq!(
            refused(&with(
                "/libraries",
                json!([
                    artifact("/opt/cell/lib/a.dylib", 'a'),
                    artifact("/opt/cell/lib/b.dylib", 'b'),
                    {"path": "/opt/cell/lib/c.dylib"},
                ])
            )),
            RecipeRefusal::MissingField {
                field: "libraries[2].sha256".to_owned()
            }
        );
    }

    #[test]
    fn an_unknown_key_refuses_at_any_depth_with_its_dotted_name() {
        for (pointer, field) in [
            ("/extra", "extra"),
            ("/kernel/size", "kernel.size"),
            ("/libraries/0/size", "libraries[0].size"),
        ] {
            assert_eq!(
                refused(&with(pointer, json!(1))),
                RecipeRefusal::UnknownField {
                    field: field.to_owned()
                }
            );
        }
    }

    #[test]
    fn a_value_of_the_wrong_shape_refuses_naming_its_field() {
        assert_eq!(
            refused(b"[]\n"),
            RecipeRefusal::NotAnObject {
                field: String::new()
            }
        );
        assert_eq!(
            refused(&with("/kernel", json!("/opt/cell/kernel"))),
            RecipeRefusal::NotAnObject {
                field: "kernel".to_owned()
            }
        );
        assert_eq!(
            refused(&with("/libraries", json!(["/opt/cell/lib/a.dylib"]))),
            RecipeRefusal::NotAnObject {
                field: "libraries[0]".to_owned()
            }
        );
        assert_eq!(
            refused(&with("/libraries", json!({}))),
            wrong_type("libraries", "an array")
        );
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
        assert_eq!(
            refused(kernel_twice.as_bytes()),
            RecipeRefusal::DuplicateField {
                path: "/kernel".to_owned()
            }
        );
        let helper = r#""helper":{"#;
        assert!(text.contains(helper));
        let sha256_twice = text.replacen(
            helper,
            &format!(r#"{helper}"sha256":"{}","#, digest('4')),
            1,
        );
        assert_eq!(
            refused(sha256_twice.as_bytes()),
            RecipeRefusal::DuplicateField {
                path: "/helper/sha256".to_owned()
            }
        );
    }

    #[test]
    fn version_is_the_integer_one() {
        for (found, refusal) in [(2, 2), (0, 0), (-1, -1)] {
            assert_eq!(
                refused(&with("/version", json!(found))),
                RecipeRefusal::UnsupportedVersion { found: refusal }
            );
        }
        assert_eq!(
            refused(&with("/version", json!(u64::MAX))),
            RecipeRefusal::UnsupportedVersion {
                found: i128::from(u64::MAX)
            }
        );
        for wrong in [json!(1.0), json!("1"), json!(null), json!(true)] {
            assert_eq!(
                refused(&with("/version", wrong)),
                wrong_type("version", "the integer 1")
            );
        }
    }

    #[test]
    fn vcpus_and_memory_are_positive_and_within_the_host_limits() {
        let vcpus_type = wrong_type("vcpus", "an integer from 0 to 255");
        let memory_type = wrong_type("memory_mib", "an integer from 0 to 4294967295");
        assert_eq!(refused(&with("/vcpus", json!(0))), RecipeRefusal::ZeroVcpus);
        for wrong in [json!(256), json!(-1), json!(2.0), json!("2")] {
            assert_eq!(refused(&with("/vcpus", wrong)), vcpus_type);
        }
        assert_eq!(
            refused(&with("/vcpus", json!(9))),
            RecipeRefusal::VcpusAboveLimit {
                requested: 9,
                limit: 8
            }
        );
        assert_eq!(parsed(&with("/vcpus", json!(8))).vcpus(), 8);
        assert_eq!(
            refused(&with("/memory_mib", json!(0))),
            RecipeRefusal::ZeroMemory
        );
        for wrong in [json!(4_294_967_296_u64), json!(-1), json!(2048.0)] {
            assert_eq!(refused(&with("/memory_mib", wrong)), memory_type);
        }
        assert_eq!(
            refused(&with("/memory_mib", json!(16_385))),
            RecipeRefusal::MemoryAboveLimit {
                requested: 16_385,
                limit: 16_384
            }
        );
        assert_eq!(
            parsed(&with("/memory_mib", json!(16_384))).memory_mib(),
            16_384
        );
        let roomy = PlatformLimits {
            max_vcpus: u8::MAX,
            max_memory_mib: u32::MAX,
            ..LIMITS
        };
        let largest = record(&[
            ("/vcpus", Some(json!(255))),
            ("/memory_mib", Some(json!(u32::MAX))),
        ]);
        let recipe = RuntimeRecipe::parse(&largest, &roomy).expect("the largest values parse");
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
        for unknown in ["RAW", "bzImage", ""] {
            assert_eq!(
                refused(&with("/kernel_format", json!(unknown))),
                RecipeRefusal::UnknownKernelFormat {
                    found: unknown.to_owned()
                }
            );
        }
    }

    #[test]
    fn x86_64_with_raw_refuses_on_either_host_before_the_architecture_compare() {
        let x86_64 = |format: &str| {
            record(&[
                ("/architecture", Some(json!("x86_64"))),
                ("/kernel_format", Some(json!(format))),
            ])
        };
        for limits in [LIMITS, X86_64_LIMITS] {
            assert_eq!(
                refused_under(&x86_64("raw"), &limits),
                RecipeRefusal::UnadmittedBootPair
            );
        }
        assert_eq!(
            refused(&x86_64("elf")),
            RecipeRefusal::ArchitectureMismatch {
                recipe: Architecture::X86_64,
                host: Architecture::Aarch64
            }
        );
        assert_eq!(
            refused_under(&record(&[]), &X86_64_LIMITS),
            RecipeRefusal::ArchitectureMismatch {
                recipe: Architecture::Aarch64,
                host: Architecture::X86_64
            }
        );
        let recipe =
            RuntimeRecipe::parse(&x86_64("elf"), &X86_64_LIMITS).expect("x86_64 elf parses");
        assert_eq!(recipe.architecture().as_str(), "x86_64");
        for unknown in ["arm64", "AARCH64", ""] {
            assert_eq!(
                refused(&with("/architecture", json!(unknown))),
                RecipeRefusal::UnknownArchitecture {
                    found: unknown.to_owned()
                }
            );
        }
    }

    #[test]
    fn a_bad_digest_refuses_in_every_artifact_by_name() {
        for (pointer, name) in ARTIFACTS {
            let sha256 = format!("{pointer}/sha256");
            let field = format!("{name}.sha256");
            assert_eq!(
                refused(&with(&sha256, json!("A".repeat(64)))),
                RecipeRefusal::BadDigest {
                    field: field.clone(),
                    fault: DigestError::NotLowercaseHex { at: 0 }
                }
            );
            assert_eq!(
                refused(&with(&sha256, json!("a".repeat(63)))),
                RecipeRefusal::BadDigest {
                    field,
                    fault: DigestError::WrongLength { bytes: 63 }
                }
            );
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
                assert_eq!(
                    refused(&with(pointer, json!(text))),
                    RecipeRefusal::BadPath {
                        field: name.to_owned(),
                        fault
                    }
                );
            }
        }
    }

    #[test]
    fn socket_paths_are_distinct_and_fit_the_host_sun_path() {
        assert_eq!(
            refused(&with("/data_path", json!("/var/cell/c1/control.sock"))),
            RecipeRefusal::SocketPathsEqual
        );
        for (pointer, field) in [
            ("/control_path", "control_path"),
            ("/data_path", "data_path"),
        ] {
            parsed(&with(pointer, path_of_length(103)));
            assert_eq!(
                refused(&with(pointer, path_of_length(104))),
                RecipeRefusal::SocketPathTooLong {
                    field,
                    bytes: 104,
                    max: 103
                }
            );
        }
        let both_long = record(&[
            ("/control_path", Some(path_of_length(105))),
            ("/data_path", Some(path_of_length(104))),
        ]);
        assert_eq!(
            refused(&both_long),
            RecipeRefusal::SocketPathTooLong {
                field: "control_path",
                bytes: 105,
                max: 103
            }
        );
    }

    #[test]
    fn one_path_in_two_roles_refuses_naming_both() {
        use ArtifactRole::*;
        for (pointer, path, first, second) in [
            (
                "/writable_root",
                "/opt/cell/root.raw",
                PathRole::Artifact(RootImage),
                PathRole::WritableRoot,
            ),
            (
                "/libraries/0/path",
                "/opt/cell/krun-helper",
                PathRole::Artifact(Helper),
                PathRole::Artifact(Library(0)),
            ),
            (
                "/data_path",
                "/opt/cell/host.sb",
                PathRole::DataPath,
                PathRole::Artifact(HostPolicy),
            ),
            (
                "/guest_policy/path",
                "/opt/cell/kernel",
                PathRole::Artifact(Kernel),
                PathRole::Artifact(GuestPolicy),
            ),
            (
                "/initramfs/path",
                "/var/cell/c1/control.sock",
                PathRole::Artifact(Initramfs),
                PathRole::ControlPath,
            ),
        ] {
            assert_eq!(
                refused(&with(pointer, json!(path))),
                RecipeRefusal::PathReused { first, second }
            );
        }
        let twin_libraries = json!([
            artifact("/opt/cell/lib/a.dylib", 'a'),
            artifact("/opt/cell/lib/a.dylib", 'b'),
        ]);
        assert_eq!(
            refused(&with("/libraries", twin_libraries)),
            RecipeRefusal::PathReused {
                first: PathRole::Artifact(Library(0)),
                second: PathRole::Artifact(Library(1))
            }
        );
    }

    #[test]
    fn of_several_reused_paths_the_first_pair_in_field_order_is_reported() {
        let two_reuses = record(&[
            ("/root_image/path", Some(json!("/opt/cell/initramfs"))),
            ("/guest_policy/path", Some(json!("/opt/cell/kernel"))),
        ]);
        assert_eq!(
            refused(&two_reuses),
            RecipeRefusal::PathReused {
                first: PathRole::Artifact(ArtifactRole::Kernel),
                second: PathRole::Artifact(ArtifactRole::GuestPolicy)
            }
        );
    }

    #[test]
    fn value_rules_come_before_the_cross_field_rules_in_a_fixed_order() {
        let cases = [
            (
                record(&[
                    ("/vcpus", Some(json!(0))),
                    ("/kernel_format", Some(json!("bzImage"))),
                ]),
                RecipeRefusal::UnknownKernelFormat {
                    found: "bzImage".to_owned(),
                },
            ),
            (
                record(&[("/vcpus", Some(json!(0))), ("/memory_mib", Some(json!(0)))]),
                RecipeRefusal::ZeroVcpus,
            ),
            (
                record(&[("/memory_mib", Some(json!(0))), ("/vcpus", Some(json!(9)))]),
                RecipeRefusal::ZeroMemory,
            ),
            (
                record(&[
                    ("/vcpus", Some(json!(9))),
                    ("/memory_mib", Some(json!(16_385))),
                ]),
                RecipeRefusal::VcpusAboveLimit {
                    requested: 9,
                    limit: 8,
                },
            ),
            (
                record(&[
                    ("/memory_mib", Some(json!(16_385))),
                    ("/architecture", Some(json!("x86_64"))),
                ]),
                RecipeRefusal::MemoryAboveLimit {
                    requested: 16_385,
                    limit: 16_384,
                },
            ),
            (
                record(&[
                    ("/architecture", Some(json!("x86_64"))),
                    ("/kernel_format", Some(json!("elf"))),
                    ("/data_path", Some(json!("/var/cell/c1/control.sock"))),
                ]),
                RecipeRefusal::ArchitectureMismatch {
                    recipe: Architecture::X86_64,
                    host: Architecture::Aarch64,
                },
            ),
            (
                record(&[
                    ("/control_path", Some(path_of_length(104))),
                    ("/data_path", Some(path_of_length(104))),
                ]),
                RecipeRefusal::SocketPathsEqual,
            ),
            (
                record(&[
                    ("/control_path", Some(path_of_length(104))),
                    ("/writable_root", Some(json!("/opt/cell/root.raw"))),
                ]),
                RecipeRefusal::SocketPathTooLong {
                    field: "control_path",
                    bytes: 104,
                    max: 103,
                },
            ),
        ];
        for (record, refusal) in cases {
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
        assert_eq!(Architecture::host(), Some(limits.architecture));
        assert_eq!(limits.architecture.as_str(), std::env::consts::ARCH);
        assert!(limits.max_vcpus >= 1, "{limits:?}");
        assert!(limits.max_memory_mib >= 1, "{limits:?}");
        let sun_path = if cfg!(target_os = "macos") { 103 } else { 107 };
        assert_eq!(limits.max_socket_path_bytes, sun_path);
    }

    #[test]
    fn roles_are_shown_as_recipe_field_names() {
        use ArtifactRole::*;
        let shown: Vec<String> = [
            PathRole::Artifact(Kernel),
            PathRole::Artifact(Initramfs),
            PathRole::Artifact(RootImage),
            PathRole::WritableRoot,
            PathRole::ControlPath,
            PathRole::DataPath,
            PathRole::Artifact(Helper),
            PathRole::Artifact(Library(3)),
            PathRole::Artifact(HostPolicy),
            PathRole::Artifact(GuestPolicy),
        ]
        .iter()
        .map(PathRole::to_string)
        .collect();
        assert_eq!(
            shown,
            [
                "kernel.path",
                "initramfs.path",
                "root_image.path",
                "writable_root",
                "control_path",
                "data_path",
                "helper.path",
                "libraries[3].path",
                "host_policy.path",
                "guest_policy.path",
            ]
        );
    }

    #[test]
    fn refusals_describe_themselves() {
        let field = |name: &str| name.to_owned();
        for (refusal, text) in [
            (
                RecipeRefusal::TooLarge { bytes: 1_048_577 },
                "the recipe is 1048577 bytes before its newline; at most 1048576 are allowed",
            ),
            (
                RecipeRefusal::NotOneLine,
                "the recipe is not one line ending in a single newline with no carriage return",
            ),
            (RecipeRefusal::NotUtf8, "the recipe is not UTF-8"),
            (
                RecipeRefusal::NotJson {
                    detail: field("trailing characters at line 1, column 4"),
                },
                "the recipe is not JSON: trailing characters at line 1, column 4",
            ),
            (
                RecipeRefusal::NotAnObject { field: field("") },
                "the recipe is not a JSON object",
            ),
            (
                RecipeRefusal::NotAnObject {
                    field: field("libraries[2]"),
                },
                "`libraries[2]` must be a JSON object",
            ),
            (
                RecipeRefusal::DuplicateField {
                    path: field("/helper/sha256"),
                },
                r#"the recipe repeats the key at "/helper/sha256""#,
            ),
            (
                RecipeRefusal::UnknownField {
                    field: field("kernel.si\nze"),
                },
                r#"the recipe carries an unknown field "kernel.si\nze""#,
            ),
            (
                RecipeRefusal::MissingField {
                    field: field("libraries[2].sha256"),
                },
                "the recipe is missing `libraries[2].sha256`",
            ),
            (
                wrong_type("vcpus", "an integer from 0 to 255"),
                "`vcpus` must be an integer from 0 to 255",
            ),
            (
                RecipeRefusal::UnsupportedVersion { found: 2 },
                "recipe version 2 is not supported; only version 1 is",
            ),
            (
                RecipeRefusal::BadDigest {
                    field: field("kernel.sha256"),
                    fault: DigestError::WrongLength { bytes: 63 },
                },
                "`kernel.sha256` is refused: a SHA-256 digest is 64 lowercase hexadecimal digits, not 63 bytes",
            ),
            (
                RecipeRefusal::BadPath {
                    field: field("data_path"),
                    fault: PathError::NotAbsolute,
                },
                "`data_path` is refused: the path does not start with /",
            ),
            (
                RecipeRefusal::UnknownKernelFormat {
                    found: field("bzImage"),
                },
                r#"`kernel_format` "bzImage" is not one of raw, elf, pe_gz, image_bz2, image_gz or image_zstd"#,
            ),
            (
                RecipeRefusal::UnknownArchitecture {
                    found: field("riscv64"),
                },
                r#"architecture "riscv64" is neither aarch64 nor x86_64"#,
            ),
            (RecipeRefusal::ZeroVcpus, "`vcpus` must be at least 1"),
            (RecipeRefusal::ZeroMemory, "`memory_mib` must be at least 1"),
            (
                RecipeRefusal::VcpusAboveLimit {
                    requested: 9,
                    limit: 8,
                },
                "`vcpus` is 9; this host admits at most 8",
            ),
            (
                RecipeRefusal::MemoryAboveLimit {
                    requested: 16_385,
                    limit: 16_384,
                },
                "`memory_mib` is 16385; this host admits at most 16384",
            ),
            (
                RecipeRefusal::UnadmittedBootPair,
                "kernel format raw is not admitted for x86_64: the pinned libkrun boots it without the initramfs and command line",
            ),
            (
                RecipeRefusal::ArchitectureMismatch {
                    recipe: Architecture::X86_64,
                    host: Architecture::Aarch64,
                },
                "the recipe is built for x86_64 but this host is aarch64",
            ),
            (
                RecipeRefusal::SocketPathsEqual,
                "`control_path` and `data_path` are the same path",
            ),
            (
                RecipeRefusal::SocketPathTooLong {
                    field: "control_path",
                    bytes: 104,
                    max: 103,
                },
                "`control_path` is 104 bytes; a socket path on this host holds at most 103",
            ),
            (
                RecipeRefusal::PathReused {
                    first: PathRole::Artifact(ArtifactRole::Helper),
                    second: PathRole::Artifact(ArtifactRole::Library(0)),
                },
                "`helper.path` and `libraries[0].path` are the same path",
            ),
            (
                RecipeRefusal::HostLimitUnavailable {
                    limit: "physical memory",
                },
                "this host did not report its physical memory",
            ),
        ] {
            assert_eq!(refusal.to_string(), text);
        }
    }
}
