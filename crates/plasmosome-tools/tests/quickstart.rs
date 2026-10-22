#![deny(clippy::exit)]

use std::ffi::OsStr;
use std::fs::Permissions;
use std::hash::{BuildHasher, RandomState};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use plasmosome_tools::quickstart::{
    BASE_RULE, Entry, Host, ID_OUTPUT_LIMIT, MOUNTINFO_LIMIT, System, check_base, noexec_in,
};

const LONGEST_BASE: usize = 100 - 1 - "/plasmosome-status-XXXXXXXX/membrane.uds".len();
const STRANGER: u32 = 4242;
const NOT_ROOT: &str = "run as a user other than root: root can write a 0555 directory";
#[cfg(target_os = "macos")]
const ILLEGAL_BYTE_SEQUENCE: i32 = 92;
#[cfg(not(target_os = "macos"))]
const ILLEGAL_BYTE_SEQUENCE: i32 = 84;

struct Fake {
    uid: u32,
    noexec: Option<PathBuf>,
    unknown: Option<&'static str>,
}

impl Host for Fake {
    fn effective_uid(&self) -> u32 {
        self.uid
    }

    fn lstat(&self, path: &Path) -> io::Result<Entry> {
        Entry::of(path)
    }

    fn noexec(&self, base: &Path) -> Result<bool, String> {
        match self.unknown {
            Some(why) => Err(why.to_owned()),
            None => Ok(self.noexec.as_deref() == Some(base)),
        }
    }
}

fn fake(uid: u32) -> Fake {
    Fake {
        uid,
        noexec: None,
        unknown: None,
    }
}

struct Base(PathBuf);

impl Drop for Base {
    fn drop(&mut self) {
        if self.0.is_dir() {
            let _ = std::fs::set_permissions(&self.0, Permissions::from_mode(0o700));
            let _ = std::fs::remove_dir_all(&self.0);
        } else {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

fn system() -> &'static System {
    static SYSTEM: OnceLock<System> = OnceLock::new();
    SYSTEM.get_or_init(|| System::new().expect("id -u gives the effective uid"))
}

fn euid() -> u32 {
    system().effective_uid()
}

fn private(path: &Path) -> bool {
    path.ancestors().all(|directory| {
        std::fs::symlink_metadata(directory).is_ok_and(|metadata| {
            metadata.file_type().is_dir()
                && (metadata.uid() == 0 || metadata.uid() == euid())
                && metadata.mode() & 0o022 == 0
        })
    })
}

fn root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let candidates = [
            Some(std::env::temp_dir().into_os_string()),
            std::env::var_os("XDG_RUNTIME_DIR"),
            std::env::var_os("HOME"),
        ];
        candidates
            .iter()
            .flatten()
            .filter(|candidate| !candidate.is_empty())
            .filter_map(|candidate| std::fs::canonicalize(candidate).ok())
            .find(|path| private(path) && path.as_os_str().len() <= LONGEST_BASE - 3)
            .unwrap_or_else(|| {
                panic!(
                    "none of {candidates:?} is a private directory under {} bytes",
                    LONGEST_BASE - 2
                )
            })
    })
}

fn letters(count: usize) -> String {
    let random = RandomState::new();
    (0..count)
        .map(|place| char::from(b'a' + (random.hash_one(place) % 26) as u8))
        .collect()
}

fn base_of(length: usize, mode: u32, directory: bool, ending: &str) -> Base {
    let size = length - root().as_os_str().len() - 1 - ending.len();
    loop {
        let path = root().join(format!("{}{ending}", letters(size)));
        let made = if directory {
            std::fs::create_dir(&path)
        } else {
            std::fs::File::create_new(&path).map(drop)
        };
        match made {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("cannot make {}: {error}", path.display()),
        }
        let base = Base(path);
        std::fs::set_permissions(&base.0, Permissions::from_mode(mode)).expect("chmod works");
        return base;
    }
}

fn longest() -> Base {
    base_of(LONGEST_BASE, 0o700, true, "")
}

fn refused(base: &Path, reason: &str, host: &dyn Host) {
    let refusal = check_base(base, host).expect_err("the base is refused");
    assert_eq!(
        refusal.to_string(),
        format!("quickstart: {reason}; {BASE_RULE}")
    );
    let empty = !base.is_dir()
        || std::fs::read_dir(base)
            .expect("the base can be listed")
            .next()
            .is_none();
    assert!(empty, "{} was changed", base.display());
}

fn passes(base: &Path, host: &dyn Host) {
    let made = check_base(base, host).expect("the base reaches the build");
    assert_eq!(made.parent(), Some(base));
    let name = made
        .file_name()
        .and_then(OsStr::to_str)
        .expect("a UTF-8 name");
    let tail = name
        .strip_prefix("plasmosome-status-")
        .expect("the quickstart's prefix");
    assert!(
        tail.len() == 8
            && tail
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
        "{name}"
    );
    let metadata = std::fs::symlink_metadata(&made).expect("the directory is made");
    assert!(
        metadata.is_dir() && metadata.mode() & 0o077 == 0,
        "{metadata:?}"
    );
    assert!(made.join("membrane.uds").as_os_str().len() < 100);
    std::fs::remove_dir(&made).expect("the private directory is removed");
    let left = std::fs::read_dir(base)
        .expect("the base can be listed")
        .count();
    assert_eq!(left, 0, "{} was changed", base.display());
}

fn too_long(base: &Path) -> String {
    format!(
        "{0} is too long: a socket path such as {0}/plasmosome-status-XXXXXXXX/membrane.uds must stay under 100 bytes",
        base.display()
    )
}

fn noexec(base: &Path) -> String {
    format!(
        "{} is on a noexec mount, and the build runs programs from it",
        base.display()
    )
}

#[test]
fn a_private_base_of_the_longest_length_reaches_the_build() {
    passes(&longest().0, system());
}

#[test]
fn a_base_one_byte_too_long_is_refused_in_one_line() {
    let base = base_of(LONGEST_BASE + 1, 0o700, true, "");
    refused(&base.0, &too_long(&base.0), system());
}

#[test]
fn a_base_one_byte_too_long_through_a_two_byte_character_is_refused() {
    let base = base_of(LONGEST_BASE + 1, 0o700, true, "\u{e9}");
    refused(&base.0, &too_long(&base.0), system());
}

#[test]
fn a_base_that_is_not_valid_utf8_is_refused() {
    let tail = letters(8);
    let mut name = b"q\xff".to_vec();
    name.extend(tail.bytes());
    let path = root().join(OsStr::from_bytes(&name));
    let _made = match std::fs::create_dir(&path) {
        Ok(()) => Some(Base(path.clone())),
        Err(error) if error.raw_os_error() == Some(ILLEGAL_BYTE_SEQUENCE) => None,
        Err(error) => panic!("cannot make the base: {error}"),
    };
    let shown = format!("{}/q\\xff{tail}", root().display());
    refused(
        &path,
        &format!("{shown} is not valid UTF-8, which the controller's JSON config cannot carry"),
        system(),
    );
}

#[test]
fn a_base_whose_owner_is_not_the_effective_uid_is_refused() {
    let base = longest();
    let chain: Vec<&Path> = base.0.ancestors().collect();
    let (first, owner) = chain
        .into_iter()
        .rev()
        .map(|directory| {
            let owner = std::fs::symlink_metadata(directory)
                .expect("lstat works")
                .uid();
            (directory, owner)
        })
        .find(|&(_, owner)| owner != 0 && owner != STRANGER)
        .expect("a directory on the way that root does not own");
    let reason = format!(
        "{} is owned by uid {owner}, not by you or root",
        first.display()
    );
    refused(&base.0, &reason, &fake(STRANGER));
}

#[test]
fn a_missing_base_is_refused() {
    let base = root().join(format!("missing-{}", letters(8)));
    let reason = format!(
        "cannot inspect {}: No such file or directory",
        base.display()
    );
    refused(&base, &reason, system());
}

#[test]
fn a_file_is_refused() {
    let base = base_of(LONGEST_BASE, 0o600, false, "");
    let reason = format!("{} is not a directory", base.0.display());
    refused(&base.0, &reason, system());
}

#[test]
fn a_base_group_can_write_is_refused() {
    let base = base_of(LONGEST_BASE, 0o770, true, "");
    let reason = format!(
        "{} has mode 0770: group or other can replace entries in it",
        base.0.display()
    );
    refused(&base.0, &reason, system());
}

#[test]
fn a_base_without_write_permission_is_refused() {
    assert_ne!(euid(), 0, "{NOT_ROOT}");
    let base = base_of(LONGEST_BASE, 0o555, true, "");
    let reason = format!("you cannot create entries in {}", base.0.display());
    refused(&base.0, &reason, system());
}

#[test]
fn a_base_without_search_permission_is_refused() {
    assert_ne!(euid(), 0, "{NOT_ROOT}");
    let base = base_of(LONGEST_BASE, 0o600, true, "");
    let reason = format!("you cannot create entries in {}", base.0.display());
    refused(&base.0, &reason, system());
}

#[test]
fn a_base_on_a_noexec_mount_is_refused() {
    let base = longest();
    let host = Fake {
        noexec: Some(base.0.clone()),
        ..fake(euid())
    };
    refused(&base.0, &noexec(&base.0), &host);
}

#[test]
#[ignore = "needs a prepared noexec mount named by QUICKSTART_NOEXEC_BASE"]
fn a_base_on_a_real_noexec_mount_is_refused() {
    let named = std::env::var_os("QUICKSTART_NOEXEC_BASE")
        .expect("QUICKSTART_NOEXEC_BASE names an empty private base on a noexec mount");
    let base = std::fs::canonicalize(named).expect("the noexec base exists");
    refused(&base, &noexec(&base), system());
}

#[test]
fn a_base_only_the_effective_uid_can_write_reaches_the_build() {
    let base = longest();
    let owner = std::fs::symlink_metadata(&base.0)
        .expect("lstat works")
        .uid();
    assert_eq!(
        euid(),
        owner,
        "id -u gives the effective uid, which owns what the process makes"
    );
    passes(&base.0, system());
}

#[test]
fn the_refusal_line_ends_with_the_readmes_rule() {
    let base = root().join(format!("missing-{}", letters(8)));
    let refusal = check_base(&base, system()).expect_err("a missing base is refused");
    assert_eq!(
        refusal.to_string(),
        format!(
            "quickstart: cannot inspect {}: No such file or directory; choose a base you can write whose directories are each owned by you or by root and writable by neither group nor other, such as a 0700 directory under your home, and name it in PLASMOSOME_QUICKSTART_DIR",
            base.display()
        )
    );
}

#[test]
fn a_link_to_a_private_directory_is_not_a_directory() {
    let target = longest();
    let link = base_of(LONGEST_BASE, 0o700, true, "");
    std::fs::remove_dir(&link.0).expect("the placeholder is removed");
    std::os::unix::fs::symlink(&target.0, &link.0).expect("the link is made");
    let reason = format!("{} is not a directory", link.0.display());
    refused(&link.0, &reason, system());
}

#[test]
fn a_directory_above_the_base_is_checked_before_the_base_and_its_length() {
    let parent = base_of(root().as_os_str().len() + 3, 0o770, true, "");
    let base = parent.0.join(letters(LONGEST_BASE));
    let reason = format!(
        "{} has mode 0770: group or other can replace entries in it",
        parent.0.display()
    );
    refused(&base, &reason, system());
}

#[test]
fn a_sticky_mode_is_shown_in_four_digits() {
    let base = base_of(LONGEST_BASE, 0o1777, true, "");
    let reason = format!(
        "{} has mode 1777: group or other can replace entries in it",
        base.0.display()
    );
    refused(&base.0, &reason, system());
}

#[test]
fn the_noexec_answer_comes_before_the_length() {
    let base = base_of(LONGEST_BASE + 1, 0o700, true, "");
    let host = Fake {
        noexec: Some(base.0.clone()),
        ..fake(euid())
    };
    refused(&base.0, &noexec(&base.0), &host);
}

#[test]
fn the_length_comes_before_the_write_check() {
    assert_ne!(euid(), 0, "{NOT_ROOT}");
    let base = base_of(LONGEST_BASE + 1, 0o555, true, "");
    refused(&base.0, &too_long(&base.0), system());
}

#[test]
fn a_base_whose_mount_cannot_be_read_is_refused() {
    let base = longest();
    let host = Fake {
        unknown: Some("cannot read /proc/self/mountinfo: Permission denied"),
        ..fake(euid())
    };
    let reason = format!(
        "cannot tell whether {} is on a noexec mount: cannot read /proc/self/mountinfo: Permission denied",
        base.0.display()
    );
    refused(&base.0, &reason, &host);
}

#[test]
fn a_base_that_is_not_utf8_shows_each_bad_byte_and_keeps_the_good_ones() {
    let base = Path::new(OsStr::from_bytes(b"/n\xc3\xa9/\xff\xe2\x82x\xed\xa0\x80"));
    let refusal = check_base(base, &fake(euid())).expect_err("the base is refused");
    assert_eq!(
        refusal.to_string(),
        format!(
            "quickstart: /n\u{e9}/\\xff\\xe2\\x82x\\xed\\xa0\\x80 is not valid UTF-8, which the controller's JSON config cannot carry; {BASE_RULE}"
        )
    );
}

#[test]
fn the_uid_is_read_from_one_line_of_digits_and_nothing_else() {
    let read = |script: &str| {
        System::reading(Command::new("/bin/sh").args(["-c", script]))
            .map(|host| host.effective_uid())
    };
    assert_eq!(read("echo 4242"), Ok(4242));
    let longest = format!("printf '%0{}d\\n' 7", ID_OUTPUT_LIMIT - 1);
    assert_eq!(read(&longest), Ok(7));
    let over = format!("printf '%0{}d' 0", ID_OUTPUT_LIMIT + 1);
    for (script, why) in [
        (
            "echo 7; exit 3",
            "id -u exited with exit status: 3".to_owned(),
        ),
        (
            over.as_str(),
            format!("id -u printed over {ID_OUTPUT_LIMIT} bytes"),
        ),
        ("echo", r#"id -u printed "\n", not a uid"#.to_owned()),
        ("printf 42", r#"id -u printed "42", not a uid"#.to_owned()),
        ("echo +42", r#"id -u printed "+42\n", not a uid"#.to_owned()),
        (
            "echo 4294967296",
            r#"id -u printed "4294967296\n", not a uid"#.to_owned(),
        ),
        (
            "echo 1; echo 2",
            r#"id -u printed "1\n2\n", not a uid"#.to_owned(),
        ),
    ] {
        assert_eq!(
            read(script),
            Err(format!("cannot read the effective uid: {why}")),
            "{script}"
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn on_linux_the_system_host_reads_noexec_from_its_mounts() {
    assert_eq!(system().noexec(Path::new("/proc")), Ok(true));
    assert_eq!(system().noexec(root()), Ok(false));
}

#[test]
#[cfg(not(target_os = "linux"))]
fn off_linux_the_system_host_finds_no_noexec_mount() {
    assert_eq!(system().noexec(Path::new("/dev")), Ok(false));
}

fn mountinfo(lines: &[&str]) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("a scratch file is made");
    for line in lines {
        writeln!(file, "{line}").expect("the line is written");
    }
    file
}

fn mount(point: &str, options: &str) -> String {
    format!("36 25 0:32 / {point} {options} shared:12 - ext4 /dev/sda1 rw,errors=continue")
}

fn noexec_of(lines: &[&str], base: &str) -> Result<bool, String> {
    noexec_in(mountinfo(lines).path(), Path::new(base))
}

#[test]
fn the_longest_mount_point_holding_the_base_wins() {
    let root = mount("/", "rw,relatime");
    let home = mount("/home", "rw,noexec");
    assert_eq!(noexec_of(&[&root, &home], "/home/u/q"), Ok(true));
    assert_eq!(noexec_of(&[&home, &root], "/home/u/q"), Ok(true));
    assert_eq!(noexec_of(&[&root, &home], "/var/q"), Ok(false));
    let open = mount("/home/u", "rw");
    assert_eq!(noexec_of(&[&root, &home, &open], "/home/u/q"), Ok(false));
}

#[test]
fn a_mount_point_holds_only_whole_components() {
    let root = mount("/", "rw");
    let home = mount("/home", "rw,noexec");
    assert_eq!(noexec_of(&[&root, &home], "/homeq/x"), Ok(false));
    assert_eq!(noexec_of(&[&root, &home], "/home"), Ok(true));
}

#[test]
fn escapes_in_a_mount_point_are_read() {
    let root = mount("/", "rw");
    let spaced = mount(r"/mnt/a\040b\011c\134d", "rw,noexec");
    assert_eq!(noexec_of(&[&root, &spaced], "/mnt/a b\tc\\d/q"), Ok(true));
    assert_eq!(
        noexec_of(&[&root, &spaced], r"/mnt/a\040b\011c\134d/q"),
        Ok(false)
    );
    let kept = mount(r"/mnt/e\9f\04", "rw,noexec");
    assert_eq!(noexec_of(&[&root, &kept], r"/mnt/e\9f\04/q"), Ok(true));
}

#[test]
fn a_bind_mount_is_matched_by_its_mount_point() {
    let root = mount("/", "rw");
    let bind = "40 25 8:1 /srv/data /home/u/work rw,nosuid,noexec shared:12 - ext4 /dev/sda1 rw";
    assert_eq!(noexec_of(&[&root, bind], "/home/u/work/q"), Ok(true));
    assert_eq!(noexec_of(&[&root, bind], "/srv/data/q"), Ok(false));
}

#[test]
fn noexec_is_found_among_other_options_and_only_per_mount() {
    let root = mount("/", "rw");
    let many = mount("/a", "rw,nosuid,nodev,noexec,relatime");
    assert_eq!(noexec_of(&[&root, &many], "/a/q"), Ok(true));
    let near = mount("/b", "rw,noexecs,exec");
    assert_eq!(noexec_of(&[&root, &near], "/b/q"), Ok(false));
    let super_only = "41 25 0:40 / /c rw,nosuid shared:3 - tmpfs tmpfs rw,noexec";
    assert_eq!(noexec_of(&[&root, super_only], "/c/q"), Ok(false));
}

#[test]
fn of_two_mounts_on_one_point_the_later_wins() {
    let root = mount("/", "rw");
    let under = mount("/home", "rw,noexec");
    let over = mount("/home", "rw");
    assert_eq!(noexec_of(&[&root, &under, &over], "/home/q"), Ok(false));
    assert_eq!(noexec_of(&[&root, &over, &under], "/home/q"), Ok(true));
}

#[test]
fn an_unreadable_mountinfo_refuses() {
    let made = tempfile::tempdir().expect("a scratch directory is made");
    let missing = made.path().join("mountinfo");
    assert_eq!(
        noexec_in(&missing, Path::new("/q")),
        Err(format!(
            "cannot read {}: No such file or directory",
            missing.display()
        ))
    );
}

#[test]
fn a_mountinfo_is_read_up_to_its_bound_and_refused_past_it() {
    let root = mount("/", "rw,noexec");
    let fixed = "1 1 0:1 / / x\n".len() + root.len() + 1;
    let padded = format!(
        "1 1 0:1 / /{} x",
        "p".repeat(MOUNTINFO_LIMIT as usize - fixed)
    );
    let file = mountinfo(&[&padded, &root]);
    let size = std::fs::metadata(file.path()).expect("stat works").len();
    assert_eq!(size, MOUNTINFO_LIMIT);
    assert_eq!(noexec_in(file.path(), Path::new("/q")), Ok(true));
    writeln!(file.as_file()).expect("one more byte is written");
    assert_eq!(
        noexec_in(file.path(), Path::new("/q")),
        Err(format!("{} is over 16777216 bytes", file.path().display()))
    );
}

#[test]
fn a_line_with_fewer_than_six_fields_refuses() {
    let file = mountinfo(&[&mount("/", "rw"), "36 25 0:32 / /home"]);
    assert_eq!(
        noexec_in(file.path(), Path::new("/q")),
        Err(format!(
            "{} has a line with fewer than six fields",
            file.path().display()
        ))
    );
}

#[test]
fn a_base_no_mount_holds_refuses() {
    let file = mountinfo(&[&mount("/home", "rw")]);
    assert_eq!(
        noexec_in(file.path(), Path::new("/var/q")),
        Err(format!("no mount in {} holds it", file.path().display()))
    );
}
