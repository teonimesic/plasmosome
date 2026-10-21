use std::fmt;
use std::fs::File;
use std::io::Read;
use std::os::fd::AsFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::nofollow::{
    DirIdentity, Kind, Reached, Walk, WalkRefusal, acl_grants_write, effective_uid,
};

/// The most bytes a trusted file may hold.
pub const MAX_TRUSTED_BYTES: usize = 1_048_576;

/// Which of spec 022 §3's checks a file failed. [`TrustCheck::as_str`] is the name the refusal's
/// `detail` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustCheck {
    Path,
    Ancestor,
    Parent,
    NotRegular,
    Owner,
    Links,
    Mode,
    Size,
}

impl TrustCheck {
    /// The check's name in spec 022: `path`, `ancestor`, `parent`, `not_regular`, `owner`,
    /// `links`, `mode` or `size`.
    pub fn as_str(self) -> &'static str {
        match self {
            TrustCheck::Path => "path",
            TrustCheck::Ancestor => "ancestor",
            TrustCheck::Parent => "parent",
            TrustCheck::NotRegular => "not_regular",
            TrustCheck::Owner => "owner",
            TrustCheck::Links => "links",
            TrustCheck::Mode => "mode",
            TrustCheck::Size => "size",
        }
    }
}

impl fmt::Display for TrustCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a file is not trusted: the file's path as given, the check it failed, and a short sentence
/// naming the component or the value found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustRefusal {
    pub path: PathBuf,
    pub check: TrustCheck,
    pub reason: String,
}

impl fmt::Display for TrustRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} fails the {} check: {}",
            self.path.display(),
            self.check,
            self.reason
        )
    }
}

impl std::error::Error for TrustRefusal {}

/// A file read under spec 022 §3 rules 1 to 4: the operator declarations file, or a declaration
/// it lists. It holds the bytes read through the descriptor that was checked, and the identity of
/// `/` and of every directory down to the file's parent.
#[derive(Debug)]
pub struct TrustedFile {
    path: PathBuf,
    bytes: Vec<u8>,
    chain: Vec<DirIdentity>,
}

impl TrustedFile {
    /// Walks `path` from "/" one component at a time without following a symlink, and refuses
    /// unless every directory above the file is owned by root or the effective UID, writable by
    /// neither group nor other and, on macOS, carries no ACL entry that lets another principal
    /// replace its entries; the file's own directory is owned by the effective UID; and the file,
    /// opened without following and without blocking, is a regular file the effective UID owns,
    /// with exactly one link, no write for group or other, no ACL entry granting write, and at
    /// most [`MAX_TRUSTED_BYTES`] bytes. `path` must be absolute and normal; on macOS that is
    /// `/private/var/...`, not `/var/...`. The file is read once, through the checked descriptor.
    pub fn open(path: &Path) -> Result<TrustedFile, TrustRefusal> {
        let walk = Walk::from_root(path).map_err(|refusal| refuse(path, refusal.into()))?;
        Self::from_walk(walk)
    }

    fn from_walk(walk: Walk) -> Result<TrustedFile, TrustRefusal> {
        let path = walk.path();
        let euid = effective_uid();
        let opened = walk
            .open_file(|reached| judge_file(reached, euid))
            .map_err(|fault| refuse(&path, fault))?;
        let file = File::from(opened.fd);
        let bytes = read_checked(&file, &path).map_err(|fault| refuse(&path, fault))?;
        Ok(TrustedFile {
            path,
            bytes,
            chain: opened.chain,
        })
    }

    /// The path the file was opened at.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file's bytes, read through the descriptor that passed the checks.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The device and inode of `/` and of every directory down to the file's parent, in order.
    pub fn chain(&self) -> &[DirIdentity] {
        &self.chain
    }
}

#[derive(Debug)]
enum Fault {
    Walk(WalkRefusal),
    Check(TrustCheck, String),
}

impl From<WalkRefusal> for Fault {
    fn from(refusal: WalkRefusal) -> Self {
        Fault::Walk(refusal)
    }
}

fn refuse(path: &Path, fault: Fault) -> TrustRefusal {
    let (check, reason) = match fault {
        Fault::Check(check, reason) => (check, reason),
        Fault::Walk(refusal) => (check_of(&refusal), refusal.to_string()),
    };
    TrustRefusal {
        path: path.to_path_buf(),
        check,
        reason,
    }
}

fn check_of(refusal: &WalkRefusal) -> TrustCheck {
    match refusal {
        WalkRefusal::NotNormal { .. }
        | WalkRefusal::Symlink { .. }
        | WalkRefusal::Missing { .. }
        | WalkRefusal::Io { .. } => TrustCheck::Path,
        WalkRefusal::NotDirectory { .. }
        | WalkRefusal::ForeignOwner { .. }
        | WalkRefusal::Replaceable { .. }
        | WalkRefusal::ReplaceableByAcl { .. }
        | WalkRefusal::OwnershipIgnored { .. } => TrustCheck::Ancestor,
    }
}

fn judge_file(reached: &Reached<'_>, euid: u32) -> Result<(), Fault> {
    let path = reached.path.display();
    if let Some(parent) = reached.parent
        && parent.uid != euid
    {
        let holder = reached.path.parent().unwrap_or(reached.path);
        return Err(Fault::Check(
            TrustCheck::Parent,
            format!(
                "{} is owned by uid {}, not the controller's uid {euid}",
                holder.display(),
                parent.uid
            ),
        ));
    }
    let facts = reached.facts;
    if facts.kind != Kind::Regular {
        return Err(Fault::Check(
            TrustCheck::NotRegular,
            format!("{path} is {}, not a regular file", kind_name(facts.kind)),
        ));
    }
    if facts.uid != euid {
        return Err(Fault::Check(
            TrustCheck::Owner,
            format!(
                "{path} is owned by uid {}, not the controller's uid {euid}",
                facts.uid
            ),
        ));
    }
    Ok(())
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Directory => "a directory",
        Kind::Regular => "a regular file",
        Kind::Socket => "a socket",
        Kind::Symlink => "a symlink",
        Kind::Other => "a FIFO, a device or another special file",
    }
}

fn read_checked(file: &File, path: &Path) -> Result<Vec<u8>, Fault> {
    let shown = path.display();
    let metadata = file
        .metadata()
        .map_err(|error| io_fault("fstat", path, &error))?;
    if metadata.nlink() != 1 {
        return Err(Fault::Check(
            TrustCheck::Links,
            format!(
                "{shown} has {} links; a trusted file has exactly one",
                metadata.nlink()
            ),
        ));
    }
    let mode = metadata.mode() & 0o7777;
    if mode & 0o022 != 0 {
        return Err(Fault::Check(
            TrustCheck::Mode,
            format!("{shown} has mode {mode:04o}: group or other can write it"),
        ));
    }
    if acl_grants_write(file.as_fd()).map_err(|error| io_fault("acl", path, &error))? {
        return Err(Fault::Check(
            TrustCheck::Mode,
            format!("{shown} carries an ACL allow entry that grants write"),
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_TRUSTED_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io_fault("read", path, &error))?;
    if bytes.len() > MAX_TRUSTED_BYTES {
        return Err(Fault::Check(
            TrustCheck::Size,
            format!("{shown} holds more than {MAX_TRUSTED_BYTES} bytes"),
        ));
    }
    Ok(bytes)
}

fn io_fault(op: &'static str, path: &Path, error: &std::io::Error) -> Fault {
    Fault::Walk(WalkRefusal::Io {
        op,
        path: path.to_path_buf(),
        errno: error.raw_os_error().unwrap_or(libc::EIO),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nofollow::fixtures::{
        TestRoot, facts, identity_of, make_dir, make_fifo, set_mode, within_a_second, write_file,
    };
    use crate::nofollow::{Facts, Kind, Reached, effective_uid};
    use std::fs;
    use std::os::fd::OwnedFd;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};

    const LIMIT: usize = 1_048_576;

    fn open_in(root: &TestRoot, relative: &str) -> Result<TrustedFile, TrustRefusal> {
        TrustedFile::from_walk(root.walk(relative))
    }

    fn refused(outcome: Result<TrustedFile, TrustRefusal>) -> TrustRefusal {
        match outcome {
            Ok(file) => panic!("{} opened", file.path().display()),
            Err(refusal) => refusal,
        }
    }

    fn check_of(outcome: Result<TrustedFile, TrustRefusal>) -> (PathBuf, TrustCheck) {
        let refusal = refused(outcome);
        (refusal.path, refusal.check)
    }

    #[test]
    fn opens_a_0600_file_in_a_0700_directory_with_its_bytes_and_directory_identities() {
        let root = TestRoot::new();
        make_dir(&root.join("dir"), 0o700);
        write_file(&root.join("dir/operator.json"), b"{\"version\": 1}", 0o600);
        let file = open_in(&root, "dir/operator.json").expect("a trusted file opens");
        assert_eq!(file.path(), root.join("dir/operator.json"));
        assert_eq!(file.bytes(), b"{\"version\": 1}");
        assert_eq!(
            file.chain(),
            [identity_of(root.path()), identity_of(&root.join("dir"))]
        );
    }

    #[test]
    fn open_walks_from_the_root_and_returns_every_directory_identity_down_to_the_parent() {
        let root = TestRoot::walkable_from_root();
        write_file(&root.join("operator.json"), b"{\"version\": 1}", 0o600);
        let file = TrustedFile::open(&root.join("operator.json")).expect("a trusted file opens");
        assert_eq!(file.path(), root.join("operator.json"));
        assert_eq!(file.bytes(), b"{\"version\": 1}");
        let mut at = PathBuf::from("/");
        let mut expected = vec![identity_of(&at)];
        for component in root.path().components().skip(1) {
            at.push(component);
            expected.push(identity_of(&at));
        }
        assert_eq!(file.chain(), expected);
    }

    #[test]
    fn the_bytes_come_from_the_checked_descriptor_not_the_path_text() {
        let root = TestRoot::new();
        make_dir(&root.join("a"), 0o700);
        make_dir(&root.join("b"), 0o700);
        write_file(&root.join("a/f.json"), b"good", 0o600);
        write_file(&root.join("b/f.json"), b"bad!", 0o600);
        let start = OwnedFd::from(fs::File::open(root.join("a")).expect("a opens"));
        let walk = Walk::from_dir(start, &root.join("b"), Path::new("f.json"))
            .expect("a normal relative path");
        let file = TrustedFile::from_walk(walk).expect("the file in a opens");
        assert_eq!(file.path(), root.join("b/f.json"));
        assert_eq!(file.bytes(), b"good");
    }

    #[test]
    fn a_group_or_other_writable_start_is_ancestor() {
        let root = TestRoot::new();
        write_file(&root.join("operator.json"), b"{}", 0o600);
        for mode in [0o770, 0o707] {
            set_mode(root.path(), mode);
            let refusal = refused(open_in(&root, "operator.json"));
            assert_eq!(
                (refusal.path.clone(), refusal.check),
                (root.join("operator.json"), TrustCheck::Ancestor)
            );
            assert!(
                refusal
                    .reason
                    .contains(&format!("{} has mode {mode:04o}", root.path().display())),
                "{}",
                refusal.reason
            );
        }
        set_mode(root.path(), 0o700);
    }

    #[test]
    fn a_final_symlink_to_a_valid_file_is_path() {
        let root = TestRoot::new();
        write_file(&root.join("real.json"), b"{}", 0o600);
        symlink("real.json", root.join("link.json")).expect("symlink");
        let refusal = refused(open_in(&root, "link.json"));
        assert_eq!(
            (refusal.path.clone(), refusal.check),
            (root.join("link.json"), TrustCheck::Path)
        );
        assert!(
            refusal.reason.contains("link.json is a symlink"),
            "{}",
            refusal.reason
        );
    }

    #[test]
    fn a_symlinked_directory_above_a_valid_file_is_path() {
        let root = TestRoot::new();
        make_dir(&root.join("real"), 0o700);
        write_file(&root.join("real/operator.json"), b"{}", 0o600);
        symlink("real", root.join("alias")).expect("symlink");
        let refusal = refused(open_in(&root, "alias/operator.json"));
        assert_eq!(
            (refusal.path.clone(), refusal.check),
            (root.join("alias/operator.json"), TrustCheck::Path)
        );
        assert!(
            refusal
                .reason
                .contains(&format!("{} is a symlink", root.join("alias").display())),
            "{}",
            refusal.reason
        );
    }

    #[test]
    fn a_file_group_or_other_can_write_is_mode() {
        let root = TestRoot::new();
        for (name, mode) in [("group.json", 0o620), ("other.json", 0o602)] {
            write_file(&root.join(name), b"{}", mode);
            assert_eq!(
                check_of(open_in(&root, name)),
                (root.join(name), TrustCheck::Mode),
                "{mode:o}"
            );
        }
    }

    #[test]
    fn a_file_with_a_second_hard_link_is_links() {
        let root = TestRoot::new();
        write_file(&root.join("one.json"), b"{}", 0o600);
        fs::hard_link(root.join("one.json"), root.join("two.json")).expect("link");
        for name in ["one.json", "two.json"] {
            assert_eq!(
                check_of(open_in(&root, name)),
                (root.join(name), TrustCheck::Links)
            );
        }
    }

    #[test]
    fn a_group_writable_parent_or_grandparent_is_ancestor() {
        let root = TestRoot::new();
        make_dir(&root.join("a"), 0o700);
        make_dir(&root.join("a/b"), 0o700);
        write_file(&root.join("a/b/operator.json"), b"{}", 0o600);
        set_mode(&root.join("a/b"), 0o770);
        assert_eq!(
            check_of(open_in(&root, "a/b/operator.json")),
            (root.join("a/b/operator.json"), TrustCheck::Ancestor)
        );
        set_mode(&root.join("a/b"), 0o700);
        set_mode(&root.join("a"), 0o707);
        let refusal = refused(open_in(&root, "a/b/operator.json"));
        assert_eq!(refusal.check, TrustCheck::Ancestor);
        assert!(
            refusal
                .reason
                .contains(&format!("{} has mode 0707", root.join("a").display())),
            "{}",
            refusal.reason
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_file_with_an_everyone_allow_write_acl_is_mode() {
        let root = TestRoot::new();
        write_file(&root.join("operator.json"), b"{}", 0o600);
        crate::nofollow::fixtures::add_acl(&root.join("operator.json"), "everyone allow write");
        assert_eq!(
            check_of(open_in(&root, "operator.json")),
            (root.join("operator.json"), TrustCheck::Mode)
        );
    }

    #[test]
    fn a_fifo_is_not_regular_within_a_second() {
        let root = TestRoot::new();
        let fifo = root.join("operator.json");
        make_fifo(&fifo);
        let walk = root.walk("operator.json");
        let outcome = within_a_second(&fifo, move || {
            TrustedFile::from_walk(walk).map(|file| file.bytes().len())
        });
        assert_eq!(
            outcome.map(|outcome| outcome.map_err(|refusal| (refusal.path, refusal.check))),
            Some(Err((fifo, TrustCheck::NotRegular)))
        );
    }

    #[test]
    fn a_directory_or_a_socket_is_not_regular() {
        let root = TestRoot::new();
        make_dir(&root.join("dir.json"), 0o700);
        let _socket = UnixListener::bind(root.join("sock.json")).expect("bind");
        for name in ["dir.json", "sock.json"] {
            assert_eq!(
                check_of(open_in(&root, name)),
                (root.join(name), TrustCheck::NotRegular)
            );
        }
    }

    #[test]
    fn one_byte_over_a_mebibyte_is_size_and_exactly_a_mebibyte_opens() {
        let root = TestRoot::new();
        write_file(&root.join("exact.json"), &vec![b' '; LIMIT], 0o600);
        write_file(&root.join("over.json"), &vec![b' '; LIMIT + 1], 0o600);
        assert_eq!(
            open_in(&root, "exact.json").map(|file| file.bytes().len()),
            Ok(LIMIT)
        );
        assert_eq!(
            check_of(open_in(&root, "over.json")),
            (root.join("over.json"), TrustCheck::Size)
        );
    }

    #[test]
    fn etc_hosts_is_parent_because_root_owns_its_directory() {
        if effective_uid() == 0 {
            println!("skipped: root owns /etc, so it passes the parent check");
            return;
        }
        #[cfg(target_os = "macos")]
        let hosts = Path::new("/private/etc/hosts");
        #[cfg(target_os = "linux")]
        let hosts = Path::new("/etc/hosts");
        assert_eq!(
            check_of(TrustedFile::open(hosts)),
            (hosts.to_path_buf(), TrustCheck::Parent)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn etc_hosts_through_the_etc_symlink_is_path() {
        assert_eq!(
            check_of(TrustedFile::open(Path::new("/etc/hosts"))),
            (PathBuf::from("/etc/hosts"), TrustCheck::Path)
        );
    }

    #[test]
    fn a_missing_file_or_a_relative_path_is_path() {
        let root = TestRoot::new();
        assert_eq!(
            check_of(open_in(&root, "absent.json")),
            (root.join("absent.json"), TrustCheck::Path)
        );
        assert_eq!(
            check_of(TrustedFile::open(Path::new("operator.json"))),
            (PathBuf::from("operator.json"), TrustCheck::Path)
        );
    }

    #[test]
    fn a_file_the_controller_cannot_read_is_path() {
        if effective_uid() == 0 {
            println!("skipped: root reads every file");
            return;
        }
        let root = TestRoot::new();
        write_file(&root.join("operator.json"), b"{}", 0o200);
        let refusal = refused(open_in(&root, "operator.json"));
        assert_eq!(refusal.check, TrustCheck::Path);
        assert!(refusal.reason.contains("errno 13"), "{}", refusal.reason);
    }

    #[test]
    fn the_file_judge_checks_the_parent_then_the_type_then_the_owner() {
        let euid = 501;
        let path = Path::new("/r/operator.json");
        let judged = |file: Facts, parent: Facts| {
            judge_file(
                &Reached {
                    path,
                    facts: file,
                    parent: Some(parent),
                },
                euid,
            )
        };
        let own_dir = facts(Kind::Directory, euid, 0o700);
        assert!(matches!(
            judged(facts(Kind::Regular, euid, 0o600), own_dir),
            Ok(())
        ));
        assert!(matches!(
            judged(facts(Kind::Regular, 4242, 0o600), own_dir),
            Err(Fault::Check(TrustCheck::Owner, _))
        ));
        assert!(matches!(
            judged(facts(Kind::Regular, 0, 0o600), own_dir),
            Err(Fault::Check(TrustCheck::Owner, _))
        ));
        assert!(matches!(
            judged(facts(Kind::Other, 4242, 0o600), own_dir),
            Err(Fault::Check(TrustCheck::NotRegular, _))
        ));
        assert!(matches!(
            judged(
                facts(Kind::Other, 4242, 0o600),
                facts(Kind::Directory, 0, 0o755)
            ),
            Err(Fault::Check(TrustCheck::Parent, _))
        ));
    }

    #[test]
    fn the_root_directory_is_not_regular() {
        assert_eq!(
            check_of(TrustedFile::open(Path::new("/"))),
            (PathBuf::from("/"), TrustCheck::NotRegular)
        );
    }

    #[test]
    fn a_not_regular_reason_names_what_was_found() {
        for (kind, name) in [
            (Kind::Directory, "a directory"),
            (Kind::Regular, "a regular file"),
            (Kind::Socket, "a socket"),
            (Kind::Symlink, "a symlink"),
            (Kind::Other, "a FIFO, a device or another special file"),
        ] {
            assert_eq!(kind_name(kind), name);
        }
    }

    #[test]
    fn checks_carry_the_names_spec_022_gives_them() {
        let cases = [
            (TrustCheck::Path, "path"),
            (TrustCheck::Ancestor, "ancestor"),
            (TrustCheck::Parent, "parent"),
            (TrustCheck::NotRegular, "not_regular"),
            (TrustCheck::Owner, "owner"),
            (TrustCheck::Links, "links"),
            (TrustCheck::Mode, "mode"),
            (TrustCheck::Size, "size"),
        ];
        for (check, name) in cases {
            assert_eq!(check.as_str(), name);
            assert_eq!(check.to_string(), name);
        }
    }

    #[test]
    fn a_refusal_names_the_file_the_check_and_the_reason() {
        let refusal = TrustRefusal {
            path: PathBuf::from("/r/operator.json"),
            check: TrustCheck::Links,
            reason: "/r/operator.json has 2 links; a trusted file has exactly one".to_string(),
        };
        assert_eq!(
            refusal.to_string(),
            "/r/operator.json fails the links check: /r/operator.json has 2 links; a trusted \
             file has exactly one"
        );
    }
}
