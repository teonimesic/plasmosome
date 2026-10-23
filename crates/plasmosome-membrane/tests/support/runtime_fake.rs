use std::ffi::OsString;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

mod target_cache {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/target_cache.rs"
    ));
}

pub(crate) mod owned_wait {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/owned_wait.rs"
    ));
}

use target_cache::{Fixture, cache_root, declared_target};

#[derive(Clone, Hash)]
enum Argument {
    Literal(OsString),
    Source,
    Output,
    Library,
}

#[derive(Clone, Hash)]
struct Invocation {
    source: PathBuf,
    arguments: Vec<Argument>,
}

#[derive(Clone, Hash)]
struct BuildRecipe {
    compiler: PathBuf,
    identity: Vec<u8>,
    helper: Invocation,
    library: Option<Invocation>,
}

impl BuildRecipe {
    fn host(source: PathBuf) -> Self {
        let compiler = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|directory| directory.join("cc"))
            .find(|path| path.is_file())
            .and_then(|path| fs::canonicalize(path).ok())
            .expect("the host C compiler has a resolved path");
        let version = Command::new(&compiler).arg("--version").output().unwrap();
        assert!(
            version.status.success(),
            "the C compiler reports its identity"
        );
        let mut identity = version.stdout;
        identity.extend(version.stderr);
        let common = || {
            ["-std=c11", "-Wall", "-Wextra", "-Werror"]
                .map(|argument| Argument::Literal(argument.into()))
                .to_vec()
        };
        let mut helper = Invocation {
            source,
            arguments: common(),
        };
        let library = if cfg!(target_os = "macos") {
            helper
                .arguments
                .push(Argument::Literal("-DPLASMOSOME_FIXTURE_LIBRARY".into()));
            helper.arguments.push(Argument::Library);
            let mut arguments = common();
            arguments.extend(
                [
                    "-dynamiclib",
                    "-install_name",
                    "@executable_path/../lib/libplasmosome-fake.dylib",
                ]
                .map(|argument| Argument::Literal(argument.into())),
            );
            arguments.extend([
                Argument::Source,
                Argument::Literal("-o".into()),
                Argument::Output,
            ]);
            Some(Invocation {
                source: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/support/runtime_fake_library.c"),
                arguments,
            })
        } else {
            None
        };
        helper.arguments.extend([
            Argument::Source,
            Argument::Literal("-o".into()),
            Argument::Output,
        ]);
        Self {
            compiler,
            identity,
            helper,
            library,
        }
    }

    fn key(&self) -> String {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        for invocation in std::iter::once(&self.helper).chain(self.library.iter()) {
            fs::read(&invocation.source)
                .expect("the C source is readable")
                .hash(&mut hasher);
        }
        format!("{:016x}", hasher.finish())
    }

    fn command(&self, invocation: &Invocation, output: &Path, library: &Path) -> Command {
        let mut command = Command::new(&self.compiler);
        for argument in &invocation.arguments {
            command.arg(match argument {
                Argument::Literal(text) => text.as_os_str(),
                Argument::Source => invocation.source.as_os_str(),
                Argument::Output => output.as_os_str(),
                Argument::Library => library.as_os_str(),
            });
        }
        command
    }
}

#[cfg(not(target_os = "macos"))]
const FAKE_MODES: &[&str] = &["record"];
#[cfg(target_os = "macos")]
const FAKE_MODES: &[&str] = &[
    "record",
    "shim",
    "shim-unconfined",
    "shim-worker",
    "shim-bad-policy",
    "exit-early",
    "silent",
];

static FAKE_DIRECTORY: Fixture = Fixture::new(build_runtime_fake);

fn build_runtime_fake() -> PathBuf {
    let executable = std::env::current_exe()
        .and_then(fs::canonicalize)
        .expect("the running test executable has a canonical path");
    let root = cache_root(
        &executable,
        "plasmosome-runtime-fake",
        "the runtime fake",
        declared_target().as_deref(),
    )
    .unwrap_or_else(|message| panic!("{message}"));
    compile_runtime_fake(&root)
}

/// Takes the name of a mode of `tests/support/runtime_fake.c` and returns the
/// path of the runtime fake under that name, since the fake picks its mode
/// from the base name of `argv[0]`.
///
/// The fake is compiled with the host C compiler once per source into
/// `plasmosome-runtime-fake/<key>/bin/runtime-fake` inside the Cargo target
/// directory, as `supervision_fixture` is, and each mode is a hard link to
/// it. Each test process runs it once with no mode before the first path is
/// returned, so macOS spends the first-exec cost of a freshly linked binary
/// where no test deadline is running. Panics on a mode the fake lacks, and,
/// when setup fails, on every call with the first failure's message.
/// Callers must not remove or replace the returned file.
pub fn runtime_fake(mode: &str) -> PathBuf {
    assert!(
        FAKE_MODES.contains(&mode),
        "{mode} is a mode of runtime_fake.c"
    );
    FAKE_DIRECTORY.path().join(mode)
}

fn compile_runtime_fake(root: &Path) -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/runtime_fake.c");
    let recipe = BuildRecipe::host(source);
    let key = root.join(recipe.key());
    let bin = key.join("bin");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&bin)
        .unwrap_or_else(|error| {
            panic!(
                "the runtime fake cache directory {} is created, readable only by its owner: {error}",
                key.display()
            )
        });
    compile_library(&recipe, &key);
    let fake = bin.join("runtime-fake");
    if !fake.exists() {
        publish_fake(&recipe, &fake);
    }
    for mode in FAKE_MODES {
        if let Err(error) = fs::hard_link(&fake, bin.join(mode))
            && error.kind() != ErrorKind::AlreadyExists
        {
            panic!("the runtime fake is linked as mode {mode}: {error}");
        }
    }
    let mut child = Command::new(&fake).spawn().unwrap_or_else(|error| {
        panic!(
            "the compiled runtime fake {} runs: {error}; if it is stale, run `cargo clean`",
            fake.display()
        )
    });
    let usage = owned_wait::exit_within(&mut child, owned_wait::SETUP_BOUND)
        .expect("the cached fake's state is readable")
        .expect("the cached fake exits within the setup bound");
    assert_eq!(
        usage.code(),
        Some(64),
        "the compiled runtime fake {} refuses a name that is no mode; if it is stale, run `cargo clean`",
        fake.display()
    );
    bin
}

fn publish_fake(recipe: &BuildRecipe, fake: &Path) {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let unique = fake.with_extension(format!(
        "{}-{}.partial",
        std::process::id(),
        CALLS.fetch_add(1, Ordering::Relaxed)
    ));
    let library = fake
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("lib/libplasmosome-fake.dylib");
    let compiled = recipe.command(&recipe.helper, &unique, &library).status();
    let published = match &compiled {
        Ok(status) if status.success() => fs::hard_link(&unique, fake),
        _ => Ok(()),
    };
    if let Err(error) = fs::remove_file(&unique)
        && error.kind() != ErrorKind::NotFound
    {
        panic!(
            "the runtime fake's temporary name {} is removed: {error}",
            unique.display()
        );
    }
    let status = compiled.expect("the host C compiler starts for the runtime fake");
    assert!(
        status.success(),
        "the runtime fake compiles from {}: {status}",
        recipe.helper.source.display()
    );
    if let Err(error) = published
        && error.kind() != ErrorKind::AlreadyExists
    {
        panic!(
            "the compiled runtime fake is published at {}: {error}",
            fake.display()
        );
    }
}

fn compile_library(recipe: &BuildRecipe, key: &Path) {
    let Some(invocation) = &recipe.library else {
        return;
    };
    let lib = key.join("lib");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&lib)
        .or_else(|error| {
            if error.kind() == ErrorKind::AlreadyExists {
                Ok(())
            } else {
                Err(error)
            }
        })
        .unwrap();
    let library = lib.join("libplasmosome-fake.dylib");
    if library.exists() {
        return;
    }
    let temporary = tempfile::NamedTempFile::new_in(&lib).unwrap();
    let status = recipe
        .command(invocation, temporary.path(), &library)
        .status()
        .unwrap();
    assert!(status.success(), "the fixture library compiles: {status}");
    if let Err(error) = fs::hard_link(temporary.path(), &library) {
        assert_eq!(error.kind(), ErrorKind::AlreadyExists, "{error}");
    }
}

#[test]
fn cache_key_and_invocation_include_linker_and_library_arguments() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/runtime_fake.c");
    let recipe = BuildRecipe::host(source);
    let key = recipe.key();
    let mut changed = recipe.clone();
    changed
        .helper
        .arguments
        .push(Argument::Literal("-Wl,-rpath,/tmp".into()));
    assert_ne!(
        changed.key(),
        key,
        "a linker change invalidates the cached helper"
    );
    let command = changed.command(&changed.helper, Path::new("output"), Path::new("library"));
    assert!(
        command
            .get_args()
            .any(|argument| argument == "-Wl,-rpath,/tmp")
    );
    changed = recipe.clone();
    if let Some(library) = &mut changed.library {
        library
            .arguments
            .push(Argument::Literal("-Wl,-dead_strip".into()));
        assert_ne!(
            changed.key(),
            key,
            "a library argument alone invalidates the cache"
        );
    }
    let mut changed = recipe.clone();
    changed.identity.push(b'!');
    assert_ne!(
        changed.key(),
        key,
        "a compiler version change invalidates the cache"
    );
    changed = recipe.clone();
    changed.compiler.push("other");
    assert_ne!(
        changed.key(),
        key,
        "a compiler path change invalidates the cache"
    );
}

#[test]
fn either_c_sources_bytes_invalidate_the_build_recipe() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("helper.c");
    fs::write(&source, b"first helper").unwrap();
    let mut recipe = BuildRecipe::host(source.clone());
    let key = recipe.key();
    fs::write(source, b"changed helper").unwrap();
    assert_ne!(recipe.key(), key);
    if let Some(library) = &mut recipe.library {
        library.source = directory.path().join("library.c");
        fs::write(&library.source, b"first library").unwrap();
        let key = recipe.key();
        fs::write(directory.path().join("library.c"), b"changed library").unwrap();
        assert_ne!(recipe.key(), key);
    }
}
