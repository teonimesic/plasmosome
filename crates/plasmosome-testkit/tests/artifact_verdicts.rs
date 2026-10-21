#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use plasmosome_backend::Digest;
use plasmosome_core::artifact::{ArtifactFault, verify_artifact};
use plasmosome_core::nofollow::{Kind, Walk, WalkRefusal};
use plasmosome_core::runtime::recipe::{Artifact, ArtifactRole};
use plasmosome_core::runtime::strict_json;
use plasmosome_membrane::runtime::artifacts::ArtifactFault as MembraneFault;
use tempfile::TempDir;

const BYTES: &[u8] = b"a kernel, built for this table";

type Verdict = Result<(), ArtifactFault>;

struct Row {
    number: u8,
    what: String,
    artifact: Artifact,
    expected: Verdict,
}

struct Tree {
    root: PathBuf,
    _dir: TempDir,
}

impl Tree {
    fn new() -> Tree {
        let output = Command::new("/usr/bin/getconf")
            .arg("DARWIN_USER_TEMP_DIR")
            .output()
            .expect("getconf runs");
        assert!(
            output.status.success(),
            "getconf DARWIN_USER_TEMP_DIR failed"
        );
        let base = String::from_utf8(output.stdout).expect("the per-user temp directory is UTF-8");
        let base = fs::canonicalize(base.trim()).expect("the per-user temp directory resolves");
        let dir = tempfile::Builder::new()
            .prefix("verdicts")
            .tempdir_in(&base)
            .expect("a tree in the per-user temp directory");
        set_mode(dir.path(), 0o700);
        let tree = Tree {
            root: fs::canonicalize(dir.path()).expect("the tree resolves"),
            _dir: dir,
        };
        let walked = Walk::from_root(&tree.root)
            .and_then(|walk| walk.open_directory(|_| Ok::<(), WalkRefusal>(())));
        if let Err(refusal) = walked {
            panic!("the walk from / refuses the tree: {refusal}");
        }
        tree
    }

    fn join(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn dir(&self, relative: &str, mode: u32) -> PathBuf {
        let path = self.join(relative);
        fs::create_dir(&path).unwrap_or_else(|error| panic!("mkdir {}: {error}", path.display()));
        set_mode(&path, mode);
        path
    }

    fn file(&self, relative: &str, bytes: &[u8], mode: u32, acl: &str, flags: &str) -> PathBuf {
        let path = self.join(relative);
        fs::write(&path, bytes).unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
        set_mode(&path, mode);
        if !acl.is_empty() {
            run("/bin/chmod", &["+a", acl], &path);
        }
        if !flags.is_empty() {
            run("/usr/bin/chflags", &[flags], &path);
        }
        path
    }

    fn locked(&self, relative: &str) -> PathBuf {
        self.file(relative, BYTES, 0o444, "", "uchg")
    }

    fn row_file(&self, number: u8, bytes: &[u8], mode: u32, acl: &str, flags: &str) -> PathBuf {
        let dir = format!("r{number}");
        self.dir(&dir, 0o700);
        self.file(&format!("{dir}/kernel"), bytes, mode, acl, flags)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        quietly("/usr/bin/chflags", &["-R", "0"], &self.root);
        quietly("/bin/chmod", &["-R", "-N"], &self.root);
        open_directories(&self.root);
    }
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .unwrap_or_else(|error| panic!("chmod {mode:04o} {}: {error}", path.display()));
    let found = fs::symlink_metadata(path)
        .unwrap_or_else(|error| panic!("stat {}: {error}", path.display()))
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(
        found,
        mode,
        "{} reads back mode {found:04o}, not {mode:04o}",
        path.display()
    );
}

fn run(program: &str, args: &[&str], path: &Path) {
    let status = Command::new(program)
        .args(args)
        .arg(path)
        .status()
        .unwrap_or_else(|error| panic!("{program} runs: {error}"));
    assert!(
        status.success(),
        "{program} {args:?} {} failed",
        path.display()
    );
}

fn quietly(program: &str, args: &[&str], path: &Path) {
    let _ = Command::new(program)
        .args(args)
        .arg(path)
        .stderr(Stdio::null())
        .status();
}

fn open_directories(path: &Path) {
    if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
        return;
    }
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    for entry in fs::read_dir(path).into_iter().flatten().flatten() {
        open_directories(&entry.path());
    }
}

fn pin(path: &Path, bytes: &[u8]) -> Artifact {
    let text = serde_json::json!({
        "path": path.to_str().expect("test paths are UTF-8"),
        "sha256": Digest::of(bytes).hex(),
    })
    .to_string();
    let value = strict_json::parse_value(text.as_bytes()).expect("the pin is JSON");
    Artifact::from_value(&value, "kernel").expect("the pin is a valid artifact")
}

fn table(tree: &Tree) -> (Vec<Row>, UnixListener) {
    use ArtifactFault::{NotImmutable, NotRegular, SetIdBits, Writable, WriteAcl};
    use WalkRefusal::{Missing, Replaceable, Symlink};
    let allow = "everyone allow write";
    let files: [(u8, u32, &str, &str, Verdict); 15] = [
        (1, 0o444, "", "uchg", Ok(())),
        (2, 0o555, "", "uchg", Ok(())),
        (3, 0o400, "", "uchg", Ok(())),
        (4, 0o1555, "", "uchg", Ok(())),
        (5, 0o444, "", "uchg,nodump", Ok(())),
        (7, 0o444, "everyone deny write", "uchg", Ok(())),
        (9, 0o444, "", "", Err(NotImmutable)),
        (10, 0o444, "", "uappnd", Err(NotImmutable)),
        (11, 0o644, "", "uchg", Err(Writable { mode: 0o644 })),
        (12, 0o464, "", "uchg", Err(Writable { mode: 0o464 })),
        (13, 0o446, "", "uchg", Err(Writable { mode: 0o446 })),
        (14, 0o4555, "", "uchg", Err(SetIdBits { mode: 0o4555 })),
        (15, 0o2555, "", "uchg", Err(SetIdBits { mode: 0o2555 })),
        (16, 0o444, allow, "uchg", Err(WriteAcl)),
        (17, 0o444, allow, "", Err(WriteAcl)),
    ];
    let mut rows: Vec<Row> = files
        .into_iter()
        .map(|(number, mode, acl, flags, expected)| Row {
            number,
            what: format!("mode {mode:04o}, ACL {acl:?}, flags {flags:?}"),
            artifact: pin(&tree.row_file(number, BYTES, mode, acl, flags), BYTES),
            expected,
        })
        .collect();
    let mut add = |number, what: &str, path: &Path, pinned: &[u8], expected| {
        rows.push(Row {
            number,
            what: what.to_owned(),
            artifact: pin(path, pinned),
            expected,
        });
    };
    let walk = |refusal| Err(ArtifactFault::Walk(refusal));

    let empty = tree.row_file(6, b"", 0o444, "", "uchg");
    add(6, "an empty file", &empty, b"", Ok(()));
    let other = tree.row_file(8, BYTES, 0o444, "", "uchg");
    let found = Digest::of(BYTES);
    add(
        8,
        "other bytes pinned",
        &other,
        b"other",
        Err(ArtifactFault::Digest { found }),
    );

    tree.dir("r18", 0o700);
    let directory = tree.dir("r18/kernel", 0o555);
    let kind = Kind::Directory;
    add(
        18,
        "a directory",
        &directory,
        BYTES,
        Err(NotRegular { kind }),
    );
    tree.dir("r19", 0o700);
    let socket = tree.join("r19/kernel");
    let listener = UnixListener::bind(&socket).expect("a socket binds in the tree");
    let kind = Kind::Socket;
    add(
        19,
        "a bound socket",
        &socket,
        BYTES,
        Err(NotRegular { kind }),
    );

    tree.dir("r20", 0o700);
    let path = tree.join("r20/kernel");
    symlink(tree.locked("r20/target"), &path).expect("symlink");
    add(
        20,
        "a symlink at the name",
        &path,
        BYTES,
        walk(Symlink { path: path.clone() }),
    );
    tree.dir("r21", 0o700);
    tree.dir("r21/real", 0o700);
    tree.locked("r21/real/kernel");
    let path = tree.join("r21/link");
    symlink(tree.join("r21/real"), &path).expect("symlink");
    let through = path.join("kernel");
    add(
        21,
        "a symlinked directory",
        &through,
        BYTES,
        walk(Symlink { path }),
    );

    tree.dir("r22", 0o700);
    let path = tree.join("r22/kernel");
    add(
        22,
        "a missing name",
        &path,
        BYTES,
        walk(Missing { path: path.clone() }),
    );
    tree.dir("r23", 0o700);
    let path = tree.join("r23/absent");
    let below = path.join("kernel");
    add(
        23,
        "a missing directory",
        &below,
        BYTES,
        walk(Missing { path }),
    );

    for (number, mode) in [(24, 0o775), (25, 0o1777)] {
        let path = tree.dir(&format!("r{number}"), 0o700);
        let file = tree.locked(&format!("r{number}/kernel"));
        set_mode(&path, mode);
        add(
            number,
            "a replaceable directory",
            &file,
            BYTES,
            walk(Replaceable { path, mode }),
        );
    }

    let canonical = tree.row_file(26, BYTES, 0o444, "", "uchg");
    let alias = canonical
        .strip_prefix("/private")
        .map(|rest| Path::new("/").join(rest))
        .expect("the per-user temp directory is under /private/var");
    let path = PathBuf::from("/var");
    add(26, "the /var alias", &alias, BYTES, walk(Symlink { path }));

    tree.dir("r27", 0o700);
    tree.dir("r27/other", 0o700);
    let linked = tree.join("r27/kernel");
    fs::write(&linked, BYTES).expect("write");
    fs::hard_link(&linked, tree.join("r27/other/kernel")).expect("a second hard link");
    set_mode(&linked, 0o444);
    run("/usr/bin/chflags", &["uchg"], &linked);
    add(27, "a second hard link", &linked, BYTES, Ok(()));

    rows.sort_by_key(|row| row.number);
    (rows, listener)
}

fn by_controller(row: &Row) -> Verdict {
    verify_artifact(&row.artifact).map_err(|refusal| {
        assert_eq!(
            &refusal.path,
            row.artifact.path(),
            "row {}, controller: the refusal names another path",
            row.number
        );
        refusal.reason
    })
}

fn by_membrane(row: &Row) -> Verdict {
    plasmosome_membrane::runtime::artifacts::verify_artifact(ArtifactRole::Kernel, &row.artifact)
        .map(drop)
        .map_err(|refusal| as_core(refusal.fault, row.artifact.path().as_path()))
}

fn as_core(fault: MembraneFault, path: &Path) -> ArtifactFault {
    match fault {
        MembraneFault::Walk(refusal) => ArtifactFault::Walk(refusal),
        MembraneFault::NotRegular { kind } => ArtifactFault::NotRegular { kind },
        MembraneFault::ForeignOwner { uid } => ArtifactFault::ForeignOwner { uid },
        MembraneFault::Writable { mode } => ArtifactFault::Writable { mode },
        MembraneFault::SetIdBits { mode } => ArtifactFault::SetIdBits { mode },
        MembraneFault::WriteAcl => ArtifactFault::WriteAcl,
        MembraneFault::NotImmutable => ArtifactFault::NotImmutable,
        MembraneFault::ImmutabilityUnverifiable => ArtifactFault::ImmutabilityUnverifiable,
        MembraneFault::ChangedWhileHashing => ArtifactFault::ChangedWhileHashing,
        MembraneFault::DigestMismatch { actual, .. } => ArtifactFault::Digest { found: actual },
        MembraneFault::Io { op, errno } => ArtifactFault::Walk(WalkRefusal::Io {
            op,
            path: path.to_path_buf(),
            errno,
        }),
        fault @ (MembraneFault::ChangedSinceVerification | MembraneFault::Replaced { .. }) => {
            panic!("only reconfirm gives {fault:?}")
        }
    }
}

#[test]
fn the_controller_and_the_membrane_give_every_file_the_verdict_of_its_row() {
    let tree = Tree::new();
    let (rows, listener) = table(&tree);
    let numbers: Vec<u8> = rows.iter().map(|row| row.number).collect();
    assert_eq!(numbers, (1..=27).collect::<Vec<u8>>(), "the table's rows");
    let mut failures = Vec::new();
    for row in &rows {
        for (caller, verdict) in [
            ("controller", by_controller(row)),
            ("membrane", by_membrane(row)),
        ] {
            if verdict != row.expected {
                failures.push(format!(
                    "row {} ({}), {caller}: {verdict:?}, expected {:?}",
                    row.number, row.what, row.expected
                ));
            }
        }
    }
    let root = tree.root.clone();
    drop(listener);
    drop(tree);
    assert!(
        failures.is_empty(),
        "{} verdicts differ from their rows:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        fs::symlink_metadata(&root).is_err(),
        "{} is left behind",
        root.display()
    );
}
