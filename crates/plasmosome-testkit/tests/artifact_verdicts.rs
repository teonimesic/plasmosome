#![cfg(target_os = "macos")]

use std::fs;
use std::os::macos::fs::MetadataExt as _;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use plasmosome_backend::Digest;
use plasmosome_core::artifact::{ArtifactFault, verify_artifact};
use plasmosome_core::nofollow::{Kind, Walk, WalkRefusal, effective_uid};
use plasmosome_core::runtime::recipe::{Artifact, ArtifactRole};
use plasmosome_core::runtime::strict_json;
use plasmosome_membrane::runtime::artifacts::ArtifactFault as MembraneFault;
use tempfile::TempDir;

const BYTES: &[u8] = b"a kernel, built for this table";
const FLAGS: [(&str, u32); 3] = [("nodump", 0x1), ("uchg", 0x2), ("uappnd", 0x4)];
const OTHER_USERS: &str = "/private/var/folders/zz";
const ROWS: u8 = 38;

type Verdict = Result<(), ArtifactFault>;
type Outcome = Result<Verdict, String>;

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
        let base = output("/usr/bin/getconf", &["DARWIN_USER_TEMP_DIR"]);
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
        walkable(&tree.root);
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
            add_acl(&path, acl);
        }
        set_flags(&path, flags);
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

fn walkable(path: &Path) {
    let walked =
        Walk::from_root(path).and_then(|walk| walk.open_directory(|_| Ok::<(), WalkRefusal>(())));
    if let Err(refusal) = walked {
        panic!("the walk from / refuses {}: {refusal}", path.display());
    }
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .unwrap_or_else(|error| panic!("chmod {mode:04o} {}: {error}", path.display()));
    let found = stat(path).permissions().mode() & 0o7777;
    assert_eq!(
        found,
        mode,
        "{} reads back mode {found:04o}, not {mode:04o}",
        path.display()
    );
}

fn set_flags(path: &Path, flags: &str) {
    if !flags.is_empty() {
        run("/usr/bin/chflags", &[flags], path);
    }
    let bit = |name: &str| {
        FLAGS
            .iter()
            .find(|(known, _)| *known == name)
            .unwrap_or_else(|| panic!("no bit for the flag {name}"))
            .1
    };
    let wanted = flags
        .split(',')
        .filter(|name| !name.is_empty())
        .fold(0, |all, name| all | bit(name));
    let known = FLAGS.iter().fold(0, |all, (_, bit)| all | bit);
    let found = stat(path).st_flags() & known;
    assert_eq!(
        found,
        wanted,
        "{} reads back flags {found:#x}, not {flags:?}",
        path.display()
    );
}

fn add_acl(path: &Path, rule: &str) {
    run("/bin/chmod", &["+a", rule], path);
    let listing = output("/bin/ls", &["-led", path.to_str().expect("UTF-8")]);
    let everyone = format!("group:{rule}");
    let found = listing
        .lines()
        .filter_map(|line| line.split_once(": ").map(|(_, entry)| entry))
        .any(|entry| entry == rule || entry == everyone);
    assert!(
        found,
        "{} reads back no ACL entry {rule:?}:\n{listing}",
        path.display()
    );
}

fn stat(path: &Path) -> fs::Metadata {
    fs::symlink_metadata(path).unwrap_or_else(|error| panic!("stat {}: {error}", path.display()))
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

fn output(program: &str, args: &[&str]) -> String {
    let ran = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("{program} runs: {error}"));
    assert!(ran.status.success(), "{program} {args:?} failed");
    String::from_utf8(ran.stdout).unwrap_or_else(|_| panic!("{program} {args:?} prints UTF-8"))
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

fn root_owned_set_id_file() -> PathBuf {
    let sudo = PathBuf::from("/usr/bin/sudo");
    let facts = stat(&sudo);
    assert!(
        facts.is_file() && facts.st_uid() == 0 && facts.st_mode() & 0o7777 == 0o4511,
        "{} is not root's file of mode 4511 on this host, so the root-owned row cannot run",
        sudo.display()
    );
    sudo
}

fn another_users_directory() -> (PathBuf, u32) {
    let base = Path::new(OTHER_USERS);
    walkable(base);
    let euid = effective_uid();
    let mut entries: Vec<PathBuf> = fs::read_dir(base)
        .unwrap_or_else(|error| panic!("list {OTHER_USERS}: {error}"))
        .flatten()
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    entries
        .into_iter()
        .find_map(|path| {
            let facts = fs::symlink_metadata(&path).ok()?;
            let uid = facts.st_uid();
            (facts.is_dir() && uid != 0 && uid != euid).then_some((path, uid))
        })
        .unwrap_or_else(|| {
            panic!("no directory under {OTHER_USERS} belongs to another local user, so row 38 cannot run")
        })
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
    use WalkRefusal::{ForeignOwner, Missing, Replaceable, ReplaceableByAcl, Symlink};
    let allow = "everyone allow write";
    let files: [(u8, u32, &str, &str, Verdict); 16] = [
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
        (28, 0o4644, "", "uchg", Err(Writable { mode: 0o4644 })),
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
    set_flags(&linked, "uchg");
    add(27, "a second hard link", &linked, BYTES, Ok(()));

    for (number, right) in [
        (29, "append"),
        (30, "writeattr"),
        (31, "writeextattr"),
        (32, "writesecurity"),
        (33, "chown"),
        (34, "delete"),
    ] {
        let rule = format!("everyone allow {right}");
        let file = tree.row_file(number, BYTES, 0o444, &rule, "uchg");
        add(number, &rule, &file, BYTES, Err(WriteAcl));
    }
    let owner = output("/usr/bin/id", &["-un"]);
    let rule = format!("user:{} allow write", owner.trim());
    let named = tree.row_file(35, BYTES, 0o444, &rule, "uchg");
    add(
        35,
        "an ACL entry naming the file's owner",
        &named,
        BYTES,
        Err(WriteAcl),
    );
    let path = tree.dir("r36", 0o700);
    let below = tree.locked("r36/kernel");
    add_acl(&path, "everyone allow add_file");
    add(
        36,
        "a directory ACL allowing add_file",
        &below,
        BYTES,
        walk(ReplaceableByAcl { path }),
    );

    let sudo = root_owned_set_id_file();
    let mode = 0o4511;
    add(37, "root's sudo", &sudo, BYTES, Err(SetIdBits { mode }));
    let (path, uid) = another_users_directory();
    let below = path.join("kernel");
    add(
        38,
        "a directory of another local user",
        &below,
        BYTES,
        walk(ForeignOwner { path, uid }),
    );

    rows.sort_by_key(|row| row.number);
    (rows, listener)
}

fn by_controller(row: &Row) -> Outcome {
    match verify_artifact(&row.artifact) {
        Ok(()) => Ok(Ok(())),
        Err(refusal) if &refusal.path != row.artifact.path() => {
            Err(format!("the refusal names {}", refusal.path.as_str()))
        }
        Err(refusal) => Ok(Err(refusal.reason)),
    }
}

fn by_membrane(row: &Row) -> Outcome {
    let verified = plasmosome_membrane::runtime::artifacts::verify_artifact(
        ArtifactRole::Kernel,
        &row.artifact,
    );
    match verified {
        Ok(_) => Ok(Ok(())),
        Err(refusal) if refusal.path != row.artifact.path().as_path() => {
            Err(format!("the refusal names {}", refusal.path.display()))
        }
        Err(refusal) if refusal.role != ArtifactRole::Kernel => {
            Err(format!("the refusal names the role {:?}", refusal.role))
        }
        Err(refusal) => as_core(refusal.fault).map(Err),
    }
}

fn as_core(fault: MembraneFault) -> Result<ArtifactFault, String> {
    match fault {
        MembraneFault::Rule(fault) => Ok(fault),
        fault => Err(format!("only reconfirm gives {fault:?}")),
    }
}

#[test]
fn the_controller_and_the_membrane_give_every_file_the_verdict_of_its_row() {
    let tree = Tree::new();
    let (rows, listener) = table(&tree);
    let numbers: Vec<u8> = rows.iter().map(|row| row.number).collect();
    assert_eq!(numbers, (1..=ROWS).collect::<Vec<u8>>(), "the table's rows");
    let mut failures = Vec::new();
    for row in &rows {
        for (caller, outcome) in [
            ("controller", by_controller(row)),
            ("membrane", by_membrane(row)),
        ] {
            let wrong = match outcome {
                Ok(verdict) if verdict == row.expected => continue,
                Ok(verdict) => format!("{verdict:?}, expected {:?}", row.expected),
                Err(wrong) => wrong,
            };
            failures.push(format!(
                "row {} ({}), {caller}: {wrong}",
                row.number, row.what
            ));
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
