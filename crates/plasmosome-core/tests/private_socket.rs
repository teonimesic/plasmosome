#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt, symlink};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use plasmosome_core::private_socket::{
    Accepted, PrivateDir, PrivateListener, PrivateSocketError, SocketEntry, check_peer_uid,
    check_private_path, effective_uid, peer_uid,
};
use tempfile::TempDir;

#[cfg(target_os = "macos")]
const SUN_PATH_MAX: usize = 103;
#[cfg(target_os = "linux")]
const SUN_PATH_MAX: usize = 107;

const NAME_ROOM: usize = 16;
const PATIENCE: Duration = Duration::from_secs(10);

struct ErrnoClobberingAllocator;

const CLOBBERED_ERRNO: i32 = libc::EDOM;

fn clobber_errno() {
    #[cfg(target_os = "macos")]
    unsafe {
        *libc::__error() = CLOBBERED_ERRNO
    };
    #[cfg(target_os = "linux")]
    unsafe {
        *libc::__errno_location() = CLOBBERED_ERRNO
    };
}

unsafe impl GlobalAlloc for ErrnoClobberingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        clobber_errno();
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        clobber_errno();
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(pointer, layout, size) };
        clobber_errno();
        moved
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        clobber_errno();
    }
}

#[global_allocator]
static ALLOCATOR: ErrnoClobberingAllocator = ErrnoClobberingAllocator;

#[cfg(target_os = "macos")]
fn root_bases() -> Vec<PathBuf> {
    let mut buffer = vec![0u8; libc::PATH_MAX as usize];
    let length = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    assert!(
        length > 1 && length <= buffer.len(),
        "confstr(_CS_DARWIN_USER_TEMP_DIR) returned {length}"
    );
    buffer.truncate(length - 1);
    let base = PathBuf::from(OsStr::from_bytes(&buffer));
    vec![
        base.canonicalize()
            .unwrap_or_else(|error| panic!("{} does not resolve: {error}", base.display())),
    ]
}

#[cfg(target_os = "macos")]
const ROOT_HINT: &str = "the integration tests make their roots under the per-user temp \
directory that confstr(_CS_DARWIN_USER_TEMP_DIR) reports; its ancestors must be owned by root \
or by this user, writable by neither group nor other, and carry no ACL entry that lets another \
principal replace entries";

#[cfg(target_os = "linux")]
fn root_bases() -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|base| !base.is_empty())
        .map(PathBuf::from)
        .into_iter()
        .collect();
    bases.push(PathBuf::from(env!("CARGO_TARGET_TMPDIR")));
    bases
}

#[cfg(target_os = "linux")]
const ROOT_HINT: &str = "the integration tests make their roots under XDG_RUNTIME_DIR when it \
is set, else under the target directory's tmp; set XDG_RUNTIME_DIR to a short path whose \
ancestors are owned by root or by this user and writable by neither group nor other";

fn private_root() -> TempDir {
    let mut refusals = Vec::new();
    for base in root_bases() {
        let root = match tempfile::Builder::new().prefix("ps").tempdir_in(&base) {
            Ok(root) => root,
            Err(error) => {
                refusals.push(format!("{}: {error}", base.display()));
                continue;
            }
        };
        set_mode(root.path(), 0o700);
        let length = root.path().as_os_str().len();
        match PrivateDir::open(root.path()) {
            Err(refusal) => refusals.push(format!("{}: {refusal}", base.display())),
            Ok(_) if length + 1 + NAME_ROOM > SUN_PATH_MAX => refusals.push(format!(
                "{}: a {length}-byte root leaves fewer than {NAME_ROOM} bytes for a socket name",
                base.display()
            )),
            Ok(_) => return root,
        }
    }
    panic!(
        "no base can hold a private test root:\n{}\n{ROOT_HINT}",
        refusals.join("\n")
    );
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
}

fn make_dir(path: &Path, mode: u32) {
    fs::create_dir(path).expect("mkdir");
    set_mode(path, mode);
}

fn private_dir(root: &TempDir) -> PrivateDir {
    PrivateDir::open(root.path()).expect("the test root is private")
}

fn bind(root: &TempDir, name: &str) -> PrivateListener {
    PrivateListener::bind(private_dir(root), name, effective_uid()).expect("bind")
}

fn entry_of(path: &Path) -> SocketEntry {
    let metadata = fs::symlink_metadata(path).expect("stat");
    SocketEntry {
        path: path.to_path_buf(),
        dev: metadata.dev(),
        ino: metadata.ino(),
    }
}

fn accept_one(listener: &PrivateListener) -> Accepted {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(accepted) = listener.accept().expect("accept") {
            return accepted;
        }
        assert!(Instant::now() < deadline, "no connection arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn is_nonblocking(stream: &UnixStream) -> bool {
    let flags = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFL) };
    assert!(flags >= 0, "fcntl: {}", std::io::Error::last_os_error());
    flags & libc::O_NONBLOCK != 0
}

#[test]
fn open_refuses_the_system_temp_directories_and_the_root_directory() {
    #[cfg(target_os = "macos")]
    {
        assert_eq!(
            PrivateDir::open(Path::new("/tmp/cell")).unwrap_err(),
            PrivateSocketError::SymlinkInPath {
                at: PathBuf::from("/tmp")
            }
        );
        assert_eq!(
            PrivateDir::open(Path::new("/private/tmp/cell")).unwrap_err(),
            PrivateSocketError::Replaceable {
                at: PathBuf::from("/private/tmp"),
                mode: 0o1777
            }
        );
    }
    #[cfg(target_os = "linux")]
    assert_eq!(
        PrivateDir::open(Path::new("/tmp/cell")).unwrap_err(),
        PrivateSocketError::Replaceable {
            at: PathBuf::from("/tmp"),
            mode: 0o1777
        }
    );
    #[cfg(target_os = "macos")]
    let sticky = Path::new("/private/tmp");
    #[cfg(target_os = "linux")]
    let sticky = Path::new("/tmp");
    let held = tempfile::Builder::new()
        .prefix("ps")
        .tempdir_in(sticky)
        .expect("a directory in the sticky temp directory");
    set_mode(held.path(), 0o700);
    make_dir(&held.path().join("cell"), 0o700);
    assert_eq!(
        PrivateDir::open(&held.path().join("cell")).map(|dir| dir.path().to_path_buf()),
        Err(PrivateSocketError::Replaceable {
            at: sticky.to_path_buf(),
            mode: 0o1777
        }),
        "a private directory two levels under the sticky directory"
    );
    let expected = if effective_uid() == 0 {
        PrivateSocketError::NotPrivate {
            path: PathBuf::from("/"),
            mode: 0o755,
        }
    } else {
        PrivateSocketError::ForeignOwner {
            at: PathBuf::from("/"),
            uid: 0,
        }
    };
    assert_eq!(PrivateDir::open(Path::new("/")).unwrap_err(), expected);
}

#[test]
fn a_test_root_is_removed_even_when_its_test_panics() {
    let (sender, receiver) = mpsc::channel();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let root = private_root();
        sender
            .send(root.path().to_path_buf())
            .expect("the root path is sent");
        let locked = root.path().join("locked");
        make_dir(&locked, 0o700);
        make_dir(&locked.join("inner"), 0o700);
        set_mode(&locked, 0o000);
        #[cfg(target_os = "macos")]
        {
            let held = root.path().join("held");
            make_dir(&held, 0o700);
            add_acl(&held, "everyone deny delete");
        }
        panic!("deliberate panic: the root must still be removed");
    }));
    assert!(outcome.is_err(), "the closure panics");
    let path = receiver.recv().expect("the root path");
    assert!(
        fs::symlink_metadata(&path).is_err(),
        "{} was left behind",
        path.display()
    );
}

#[test]
fn open_names_the_component_even_when_allocation_overwrites_errno() {
    let root = private_root();
    let at = |name: &str| root.path().join(name);
    make_dir(&at("real"), 0o700);
    make_dir(&at("real/cell"), 0o700);
    symlink(at("real"), at("link")).expect("symlink");
    fs::write(at("file"), b"").expect("write");
    for (path, expected) in [
        (
            at("absent/cell"),
            PrivateSocketError::Missing { at: at("absent") },
        ),
        (
            at("link/cell"),
            PrivateSocketError::SymlinkInPath { at: at("link") },
        ),
        (
            at("file/cell"),
            PrivateSocketError::NotDirectory { at: at("file") },
        ),
    ] {
        assert_eq!(
            PrivateDir::open(&path).map(|dir| dir.path().to_path_buf()),
            Err(expected),
            "the allocator sets errno to {CLOBBERED_ERRNO} after every allocation"
        );
    }
}

#[cfg(target_os = "macos")]
fn add_acl(path: &Path, rule: &str) {
    let status = std::process::Command::new("/bin/chmod")
        .arg("+a")
        .arg(rule)
        .arg(path)
        .status()
        .expect("chmod +a runs");
    assert!(status.success(), "chmod +a {rule:?} failed");
}

#[cfg(target_os = "macos")]
fn clear_acl(path: &Path) {
    let status = std::process::Command::new("/bin/chmod")
        .arg("-N")
        .arg(path)
        .status()
        .expect("chmod -N runs");
    assert!(status.success(), "chmod -N failed");
}

#[cfg(target_os = "macos")]
#[test]
fn a_private_directory_with_an_acl_entry_is_refused() {
    let root = private_root();
    let cell = root.path().join("cell");
    make_dir(&cell, 0o700);
    add_acl(&cell, "everyone allow list");
    let opened = PrivateDir::open(&cell);
    clear_acl(&cell);
    assert_eq!(
        opened.unwrap_err(),
        PrivateSocketError::AclPresent { path: cell }
    );
}

#[cfg(target_os = "macos")]
#[test]
fn an_ancestor_acl_that_denies_or_only_allows_reading_does_not_refuse() {
    let root = private_root();
    let home = root.path().join("home");
    make_dir(&home, 0o755);
    let cell = home.join("cell");
    make_dir(&cell, 0o700);
    for rule in [
        "everyone deny delete",
        "everyone deny add_file,add_subdirectory,delete_child,writesecurity,chown",
        "everyone allow list,search,readattr,readextattr,readsecurity",
    ] {
        add_acl(&home, rule);
        let opened = PrivateDir::open(&cell);
        clear_acl(&home);
        assert_eq!(
            opened
                .unwrap_or_else(|refusal| panic!("{rule:?} refused: {refusal}"))
                .path(),
            cell
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn an_ancestor_acl_that_allows_replacing_entries_is_refused() {
    let root = private_root();
    let ancestor = root.path().join("ancestor");
    make_dir(&ancestor, 0o755);
    let cell = ancestor.join("cell");
    make_dir(&cell, 0o700);
    let user = std::process::Command::new("/usr/bin/id")
        .arg("-un")
        .output()
        .expect("id runs");
    let user = String::from_utf8(user.stdout).expect("utf-8");
    for rule in [
        "everyone allow add_file,add_subdirectory,delete_child".to_string(),
        "everyone allow add_file".to_string(),
        "everyone allow add_subdirectory".to_string(),
        "everyone allow delete_child".to_string(),
        "everyone allow delete".to_string(),
        "everyone allow writesecurity".to_string(),
        "everyone allow chown".to_string(),
        "everyone allow list,search,add_file,directory_inherit".to_string(),
        "everyone allow add_file,only_inherit,file_inherit".to_string(),
        format!("user:{} allow add_file,delete_child", user.trim()),
    ] {
        add_acl(&ancestor, &rule);
        let opened = PrivateDir::open(&cell);
        clear_acl(&ancestor);
        assert_eq!(
            opened.map(|dir| dir.path().to_path_buf()),
            Err(PrivateSocketError::ReplaceableByAcl {
                at: ancestor.clone()
            }),
            "{rule:?}"
        );
    }
    add_acl(&ancestor, "everyone allow add_file");
    add_acl(&ancestor, "everyone deny delete");
    let opened = PrivateDir::open(&cell);
    clear_acl(&ancestor);
    assert_eq!(
        opened.map(|dir| dir.path().to_path_buf()),
        Err(PrivateSocketError::ReplaceableByAcl {
            at: ancestor.clone()
        }),
        "an allow entry after a deny entry"
    );
    let middle = ancestor.join("middle");
    make_dir(&middle, 0o755);
    make_dir(&middle.join("cell"), 0o700);
    add_acl(&ancestor, "everyone allow add_subdirectory,delete_child");
    let opened = PrivateDir::open(&middle.join("cell"));
    clear_acl(&ancestor);
    assert_eq!(
        opened.map(|dir| dir.path().to_path_buf()),
        Err(PrivateSocketError::ReplaceableByAcl { at: ancestor }),
        "an allow entry on a grandparent"
    );
}

#[cfg(target_os = "linux")]
fn posix_acl(mask: u16) -> Vec<u8> {
    let undefined = u32::MAX;
    let entries: [(u16, u16, u32); 5] = [
        (0x01, 0o7, undefined),
        (0x02, 0o5, 65534),
        (0x04, 0o0, undefined),
        (0x10, mask, undefined),
        (0x20, 0o0, undefined),
    ];
    let mut bytes = 2u32.to_le_bytes().to_vec();
    for (tag, perm, id) in entries {
        bytes.extend(tag.to_le_bytes());
        bytes.extend(perm.to_le_bytes());
        bytes.extend(id.to_le_bytes());
    }
    bytes
}

#[cfg(target_os = "linux")]
fn set_xattr(path: &Path, name: &std::ffi::CStr, value: &[u8]) -> Result<(), i32> {
    let dir = fs::File::open(path).expect("open the directory");
    let outcome = unsafe {
        libc::fsetxattr(
            dir.as_raw_fd(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    if outcome == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error()
            .raw_os_error()
            .expect("an errno"))
    }
}

#[cfg(target_os = "linux")]
const ACL_OPT_OUT: &str = "PLASMOSOME_ACL_TESTS_UNSUPPORTED";

#[cfg(target_os = "linux")]
fn assert_acl_refuses(attribute: &std::ffi::CStr, mask: u16) {
    let root = private_root();
    let cell = root.path().join("cell");
    make_dir(&cell, 0o700);
    match set_xattr(&cell, attribute, &posix_acl(mask)) {
        Err(errno) if errno == libc::EOPNOTSUPP => {
            assert!(
                std::env::var_os(ACL_OPT_OUT).is_some_and(|value| value == "1"),
                "the filesystem under {} refused {attribute:?} with ENOTSUP, so this ACL test \
                 cannot run; run the tests on a filesystem with POSIX ACLs or set {ACL_OPT_OUT}=1 \
                 to skip it",
                root.path().display()
            );
            println!("skipped: the filesystem refused {attribute:?} and {ACL_OPT_OUT}=1 is set");
            return;
        }
        other => other.expect("the ACL is written"),
    }
    assert_eq!(
        fs::metadata(&cell).expect("stat").mode() & 0o7777,
        0o700,
        "the ACL leaves the mode private, so only the ACL rule can refuse"
    );
    assert_eq!(
        PrivateDir::open(&cell).unwrap_err(),
        PrivateSocketError::AclPresent { path: cell }
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_private_directory_with_an_access_acl_is_refused() {
    assert_acl_refuses(c"system.posix_acl_access", 0o0);
}

#[cfg(target_os = "linux")]
#[test]
fn a_private_directory_with_a_default_acl_is_refused() {
    assert_acl_refuses(c"system.posix_acl_default", 0o5);
}

#[test]
fn bind_reports_the_entry_it_created() {
    let root = private_root();
    let listener = bind(&root, "sock");
    let path = root.path().join("sock");
    assert_eq!(listener.entry(), &entry_of(&path));
    let metadata = fs::symlink_metadata(&path).expect("stat");
    assert!(metadata.file_type().is_socket());
    assert_eq!(metadata.uid(), effective_uid());
    assert_eq!(metadata.mode() & 0o7777, 0o600);
    UnixStream::connect(&path).expect("a client connects");
}

#[test]
fn bind_refuses_an_existing_entry_and_leaves_it() {
    let root = private_root();
    let at = |name: &str| root.path().join(name);
    fs::write(at("file"), b"contents").expect("write");
    symlink(at("absent"), at("dangling")).expect("symlink");
    fs::write(at("victim"), b"victim").expect("write");
    symlink(at("victim"), at("pointer")).expect("symlink");
    drop(UnixListener::bind(at("stale")).expect("bind a stale socket"));
    let stale = entry_of(&at("stale"));
    for name in ["file", "dangling", "pointer", "stale"] {
        assert_eq!(
            PrivateListener::bind(private_dir(&root), name, effective_uid()).unwrap_err(),
            PrivateSocketError::AddressInUse { path: at(name) }
        );
    }
    assert_eq!(fs::read(at("file")).expect("read"), b"contents");
    assert_eq!(
        fs::read_link(at("dangling")).expect("readlink"),
        at("absent")
    );
    assert_eq!(
        fs::read_link(at("pointer")).expect("readlink"),
        at("victim")
    );
    assert_eq!(fs::read(at("victim")).expect("read"), b"victim");
    assert!(
        fs::symlink_metadata(at("stale"))
            .expect("stat")
            .file_type()
            .is_socket()
    );
    assert_eq!(entry_of(&at("stale")), stale);
}

#[test]
fn bind_refuses_a_name_with_a_slash_or_dots() {
    let root = private_root();
    for name in ["", ".", "..", "a/b", "/sock", "nul\0byte"] {
        assert_eq!(
            PrivateListener::bind(private_dir(&root), name, effective_uid()).unwrap_err(),
            PrivateSocketError::BadName {
                name: name.to_string()
            }
        );
    }
    assert_eq!(fs::read_dir(root.path()).expect("readdir").count(), 0);
}

#[test]
fn bind_refuses_a_directory_that_stopped_being_private_after_open() {
    let root = private_root();
    let cell = root.path().join("cell");
    make_dir(&cell, 0o700);
    let dir = PrivateDir::open(&cell).expect("a private directory opens");
    set_mode(&cell, 0o750);
    assert_eq!(
        PrivateListener::bind(dir, "sock", effective_uid()).map(|_| ()),
        Err(PrivateSocketError::NotPrivate {
            path: cell.clone(),
            mode: 0o750
        })
    );
    assert_eq!(fs::read_dir(&cell).expect("readdir").count(), 0);
    set_mode(&cell, 0o700);
    let listener = PrivateListener::bind(
        PrivateDir::open(&cell).expect("a private directory opens"),
        "sock",
        effective_uid(),
    )
    .expect("a later bind finds the name free");
    assert_eq!(listener.entry().path, cell.join("sock"));
}

#[test]
fn bind_refuses_a_path_longer_than_sun_path() {
    let root = private_root();
    let used = root.path().as_os_str().len() + 1;
    let room = SUN_PATH_MAX
        .checked_sub(used)
        .filter(|room| *room > 0)
        .unwrap_or_else(|| {
            panic!(
                "{} leaves no room for a name in {SUN_PATH_MAX} bytes",
                root.path().display()
            )
        });
    let longest = "s".repeat(room);
    let listener = bind(&root, &longest);
    assert_eq!(listener.entry().path.as_os_str().len(), SUN_PATH_MAX);
    drop(listener);
    let too_long = "s".repeat(room + 1);
    assert_eq!(
        PrivateListener::bind(private_dir(&root), &too_long, effective_uid()).unwrap_err(),
        PrivateSocketError::PathTooLong {
            path: root.path().join(&too_long),
            max: SUN_PATH_MAX
        }
    );
    assert_eq!(fs::read_dir(root.path()).expect("readdir").count(), 0);
}

#[test]
fn drop_removes_only_the_bound_socket() {
    let root = private_root();
    let path = root.path().join("sock");

    drop(bind(&root, "sock"));
    assert!(
        fs::symlink_metadata(&path).is_err(),
        "the bound socket stays"
    );

    let listener = bind(&root, "sock");
    fs::remove_file(&path).expect("unlink");
    fs::write(&path, b"replacement").expect("write");
    drop(listener);
    assert_eq!(fs::read(&path).expect("read"), b"replacement");
    fs::remove_file(&path).expect("unlink");

    let listener = bind(&root, "sock");
    fs::remove_file(&path).expect("unlink");
    drop(UnixListener::bind(&path).expect("bind another socket"));
    let other = entry_of(&path);
    drop(listener);
    assert_eq!(entry_of(&path), other);
    fs::remove_file(&path).expect("unlink");

    let listener = bind(&root, "sock");
    let moved = root.path().join("moved");
    fs::rename(&path, &moved).expect("rename");
    drop(listener);
    assert!(
        fs::symlink_metadata(&moved)
            .expect("the renamed socket stays")
            .file_type()
            .is_socket()
    );
    assert!(fs::symlink_metadata(&path).is_err());
}

#[test]
fn drop_removes_the_socket_through_the_held_directory_after_a_rename() {
    let root = private_root();
    let run = root.path().join("run");
    make_dir(&run, 0o700);
    let listener = PrivateListener::bind(
        PrivateDir::open(&run).expect("a private directory opens"),
        "sock",
        effective_uid(),
    )
    .expect("bind");
    let moved = root.path().join("moved");
    fs::rename(&run, &moved).expect("rename");
    make_dir(&run, 0o700);
    drop(UnixListener::bind(run.join("sock")).expect("bind a replacement socket"));
    let replacement = entry_of(&run.join("sock"));
    drop(listener);
    assert_eq!(
        entry_of(&run.join("sock")),
        replacement,
        "the socket now at the bound path was removed"
    );
    assert!(
        fs::symlink_metadata(moved.join("sock")).is_err(),
        "the bound socket stayed in the directory that was held"
    );
}

#[test]
fn accept_returns_a_trusted_stream_for_a_same_uid_client() {
    let root = private_root();
    let listener = bind(&root, "sock");
    assert!(listener.accept().expect("accept").is_none());
    let mut client = UnixStream::connect(&listener.entry().path).expect("connect");
    client.write_all(b"ping\n").expect("send");
    let Accepted::Trusted(server) = accept_one(&listener) else {
        panic!("a same-UID client was refused");
    };
    assert!(!is_nonblocking(&server), "the trusted stream blocks");
    server.set_read_timeout(Some(PATIENCE)).expect("timeout");
    let mut request = String::new();
    BufReader::new(&server)
        .read_line(&mut request)
        .expect("the request is still unread after accept");
    assert_eq!(request, "ping\n");
    (&server).write_all(b"pong\n").expect("reply");
    let mut reply = String::new();
    BufReader::new(&client).read_line(&mut reply).expect("read");
    assert_eq!(reply, "pong\n");
}

#[test]
fn accept_refuses_a_peer_whose_uid_is_not_trusted_before_reading() {
    let root = private_root();
    let euid = effective_uid();
    let trusted = euid.wrapping_add(1);
    let listener = PrivateListener::bind(private_dir(&root), "sock", trusted).expect("bind");
    let mut client = UnixStream::connect(&listener.entry().path).expect("connect");
    client.set_read_timeout(Some(PATIENCE)).expect("timeout");
    client
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"plasmosome.recovery\"}\n")
        .expect("send");
    match accept_one(&listener) {
        Accepted::Refused(refusal) => assert_eq!(
            refusal,
            PrivateSocketError::PeerMismatch {
                trusted,
                found: euid
            }
        ),
        Accepted::Trusted(_) => panic!("a peer with an untrusted UID was accepted"),
    }
    let mut reply = Vec::new();
    let end = client.read_to_end(&mut reply);
    if let Err(error) = &end {
        assert!(
            !matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ),
            "the refused stream was left open"
        );
    }
    assert!(reply.is_empty(), "the refused client got a reply");
    #[cfg(target_os = "macos")]
    assert_eq!(end.expect("EOF"), 0);
    #[cfg(target_os = "linux")]
    assert_eq!(
        end.expect_err("the server closed with the request unread")
            .kind(),
        std::io::ErrorKind::ConnectionReset
    );
}

fn set_blocking(fd: RawFd) {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    assert!(flags >= 0, "fcntl: {}", std::io::Error::last_os_error());
    let outcome = unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) };
    assert!(outcome == 0, "fcntl: {}", std::io::Error::last_os_error());
}

#[test]
fn accept_judges_a_silent_peer_without_waiting_to_read_from_it() {
    let root = private_root();
    let euid = effective_uid();
    let trusted = euid.wrapping_add(1);
    let listener = PrivateListener::bind(private_dir(&root), "sock", trusted).expect("bind");
    set_blocking(listener.as_raw_fd());
    let client = UnixStream::connect(&listener.entry().path).expect("connect");
    let (finished, done) = mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        let waited = done.recv_timeout(PATIENCE).is_err();
        drop(client);
        waited
    });
    let accepted = listener.accept().expect("accept");
    let _ = finished.send(());
    assert!(
        !watchdog.join().expect("the watchdog finishes"),
        "accept blocked until the silent client closed: it read before judging the peer"
    );
    assert!(
        matches!(
            accepted,
            Some(Accepted::Refused(PrivateSocketError::PeerMismatch { trusted: t, found }))
                if t == trusted && found == euid
        ),
        "{accepted:?}"
    );
}

fn poll_readable(listener: &PrivateListener) -> bool {
    let mut descriptor = libc::pollfd {
        fd: listener.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let ready = unsafe { libc::poll(&mut descriptor, 1, 0) };
    assert!(ready >= 0, "poll: {}", std::io::Error::last_os_error());
    ready == 1 && descriptor.revents & libc::POLLIN != 0
}

#[test]
fn the_listening_descriptor_polls_readable_while_a_client_waits() {
    let root = private_root();
    let listener = bind(&root, "sock");
    assert!(!poll_readable(&listener), "nothing is pending yet");
    let _client = UnixStream::connect(&listener.entry().path).expect("connect");
    assert!(
        poll_readable(&listener),
        "the pending client shows as readable"
    );
    assert!(matches!(accept_one(&listener), Accepted::Trusted(_)));
    assert!(!poll_readable(&listener), "the client was taken");
}

#[test]
fn check_peer_uid_on_a_client_stream_compares_the_server_uid() {
    let root = private_root();
    let listener = bind(&root, "sock");
    let client = UnixStream::connect(&listener.entry().path).expect("connect");
    let euid = effective_uid();
    assert_eq!(peer_uid(&client).expect("peer uid"), euid);
    assert_eq!(check_peer_uid(&client, euid), Ok(()));
    assert_eq!(
        check_peer_uid(&client, euid.wrapping_add(1)),
        Err(PrivateSocketError::PeerMismatch {
            trusted: euid.wrapping_add(1),
            found: euid
        })
    );
}

#[test]
fn socket_entry_validates_type_owner_and_mode_without_following() {
    let root = private_root();
    let listener = bind(&root, "sock");
    let dir = private_dir(&root);
    let euid = effective_uid();
    let path = root.path().join("sock");
    assert_eq!(dir.socket_entry("sock", euid), Ok(listener.entry().clone()));
    assert_eq!(
        dir.socket_entry("sock", euid.wrapping_add(1)),
        Err(PrivateSocketError::SocketOwner {
            path: path.clone(),
            uid: euid
        })
    );
    symlink(&path, root.path().join("link")).expect("symlink");
    assert_eq!(
        dir.socket_entry("link", euid),
        Err(PrivateSocketError::NotASocket {
            path: root.path().join("link")
        })
    );
    fs::write(root.path().join("file"), b"").expect("write");
    set_mode(&root.path().join("file"), 0o600);
    assert_eq!(
        dir.socket_entry("file", euid),
        Err(PrivateSocketError::NotASocket {
            path: root.path().join("file")
        })
    );
    assert_eq!(
        dir.socket_entry("absent", euid),
        Err(PrivateSocketError::Missing {
            at: root.path().join("absent")
        })
    );
    assert_eq!(
        dir.socket_entry("../sock", euid),
        Err(PrivateSocketError::BadName {
            name: "../sock".to_string()
        })
    );
    set_mode(&path, 0o660);
    assert_eq!(
        dir.socket_entry("sock", euid),
        Err(PrivateSocketError::SocketMode { path, mode: 0o660 })
    );
}

#[test]
fn check_private_path_checks_the_parent_and_the_entry() {
    let root = private_root();
    let listener = bind(&root, "sock");
    let path = listener.entry().path.clone();
    let euid = effective_uid();
    assert_eq!(
        check_private_path(&path, euid),
        Ok(listener.entry().clone())
    );
    assert_eq!(
        check_private_path(&path, euid.wrapping_add(1)),
        Err(PrivateSocketError::SocketOwner {
            path: path.clone(),
            uid: euid
        })
    );
    assert_eq!(
        check_private_path(Path::new("relative/sock"), euid),
        Err(PrivateSocketError::NotAbsolute {
            path: PathBuf::from("relative/sock")
        })
    );
    assert_eq!(
        check_private_path(Path::new("/"), euid),
        Err(PrivateSocketError::NotAbsolute {
            path: PathBuf::from("/")
        })
    );
    assert_eq!(
        check_private_path(Path::new(OsStr::from_bytes(b"/x/\xffsock")), euid),
        Err(PrivateSocketError::BadName {
            name: "\u{fffd}sock".to_string()
        })
    );
    set_mode(root.path(), 0o755);
    let refused = check_private_path(&path, euid);
    set_mode(root.path(), 0o700);
    assert_eq!(
        refused,
        Err(PrivateSocketError::NotPrivate {
            path: root.path().to_path_buf(),
            mode: 0o755
        })
    );
}

#[cfg(target_os = "macos")]
const CONNECT_AND_SEND: &str = r#"use Socket;
socket(my $s, PF_UNIX, SOCK_STREAM, 0) or do { print STDERR "socket errno=", $!+0, "\n"; exit 3 };
connect($s, pack_sockaddr_un($ARGV[0])) or do { print STDERR "connect errno=", $!+0, "\n"; exit 2 };
syswrite($s, $ARGV[1]);
my $reply = '';
while (sysread($s, my $chunk, 4096)) { $reply .= $chunk }
print $reply;
exit 0;"#;

#[cfg(target_os = "macos")]
fn run_fixture_client(
    prefix: &[&str],
    socket: &Path,
    mut serve: impl FnMut(),
) -> std::process::Output {
    use std::process::{Command, Stdio};

    let (program, arguments) = prefix.split_first().expect("a nonempty command prefix");
    let mut client = Command::new(program)
        .args(arguments)
        .args(["/usr/bin/perl", "-e", CONNECT_AND_SEND])
        .arg(socket)
        .arg("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"plasmosome.recovery\"}\n")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the fixture runs perl");
    let deadline = Instant::now() + Duration::from_secs(20);
    while client.try_wait().expect("wait").is_none() {
        serve();
        if Instant::now() > deadline {
            let _ = client.kill();
            panic!("the fixture client did not finish");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    serve();
    client.wait_with_output().expect("collect")
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "needs a distinct-UID fixture: set PLASMOSOME_OTHER_UID_PREFIX (owner decision O-8)"]
fn a_different_uid_client_cannot_reach_the_socket() {
    let Ok(prefix) = std::env::var("PLASMOSOME_OTHER_UID_PREFIX") else {
        panic!("unproved: no distinct-UID fixture (O-8)");
    };
    let prefix: Vec<&str> = prefix.split_whitespace().collect();
    let (program, arguments) = prefix.split_first().expect("a nonempty command prefix");
    let identity = std::process::Command::new(program)
        .args(arguments)
        .args(["/usr/bin/id", "-u"])
        .output()
        .expect("the fixture runs id");
    assert!(identity.status.success(), "the fixture cannot run id");
    let other: u32 = String::from_utf8(identity.stdout)
        .expect("utf-8")
        .trim()
        .parse()
        .expect("a numeric uid");
    let euid = effective_uid();
    assert_ne!(other, euid, "the fixture runs as this test's own UID");

    let temp = &root_bases()[0];
    let base = temp
        .parent()
        .expect("the per-user temp directory has a parent");
    let root = tempfile::Builder::new()
        .prefix("ps")
        .tempdir_in(base)
        .expect("a root other UIDs can search");
    set_mode(root.path(), 0o755);
    let open = root.path().join("open");
    make_dir(&open, 0o755);
    let control = UnixListener::bind(open.join("sock")).expect("bind the control socket");
    control.set_nonblocking(true).expect("nonblocking");
    set_mode(&open.join("sock"), 0o777);
    let private = root.path().join("private");
    make_dir(&private, 0o700);
    let listener = PrivateListener::bind(
        PrivateDir::open(&private).expect("a private directory opens"),
        "sock",
        euid,
    )
    .expect("bind");

    let reached = run_fixture_client(&prefix, &open.join("sock"), || {
        while let Ok((stream, _)) = control.accept() {
            drop(stream);
        }
    });
    assert!(
        reached.status.success(),
        "unproved: the fixture UID cannot reach the test root (O-8): {}",
        String::from_utf8_lossy(&reached.stderr)
    );

    let mut outcomes = Vec::new();
    let output = run_fixture_client(&prefix, &listener.entry().path, || {
        while let Some(accepted) = listener.accept().expect("accept") {
            outcomes.push(accepted);
        }
    });
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.stdout.is_empty(), "the other UID received a reply");
    if other == 0 {
        assert!(
            output.status.success(),
            "root's client stopped before the socket: {stderr}"
        );
        assert!(
            matches!(
                &outcomes[..],
                [Accepted::Refused(PrivateSocketError::PeerMismatch { trusted, found: 0 })]
                    if *trusted == euid
            ),
            "root's connection was not refused by peer UID: {outcomes:?}"
        );
        println!("proved: uid 0 reached the socket and was refused by peer UID (O-8)");
    } else {
        assert_eq!(
            output.status.code(),
            Some(2),
            "uid {other}'s client did not fail at connect: {stderr}"
        );
        assert_eq!(
            stderr.trim(),
            format!("connect errno={}", libc::EACCES),
            "uid {other}'s connect failed for another reason"
        );
        assert!(outcomes.is_empty(), "uid {other} reached accept");
        println!(
            "proved: uid {other} reached a 0777 socket beside the private directory and was \
             refused at connect with EACCES inside it (O-8)"
        );
    }
}
