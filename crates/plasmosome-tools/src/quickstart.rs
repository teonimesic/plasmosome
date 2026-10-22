use std::ffi::OsStr;
use std::fmt;
use std::fs::{DirBuilder, File};
use std::hash::{BuildHasher, RandomState};
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The rule a refused base breaks. Every refusal line ends with it.
pub const BASE_RULE: &str = "choose a base you can write whose directories are each owned by you or by root and writable by neither group nor other, such as a 0700 directory under your home, and name it in PLASMOSOME_QUICKSTART_DIR";

/// The longest socket path the quickstart makes, relative to the base.
pub const LONGEST_SOCKET: &str = "plasmosome-status-XXXXXXXX/membrane.uds";

/// A Unix socket path must be shorter than this many bytes.
pub const SOCKET_LIMIT: usize = 100;

/// The start of the private directory's name. Eight characters follow it.
pub const PRIVATE_PREFIX: &str = "plasmosome-status-";

/// The most bytes read from a mountinfo file. A longer file is refused.
pub const MOUNTINFO_LIMIT: u64 = 16 * 1024 * 1024;

/// The most bytes read from `id -u`. Longer output is refused.
pub const ID_OUTPUT_LIMIT: u64 = 64;

const MOUNTINFO: &str = "/proc/self/mountinfo";
const NAME_CHARACTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789_";
const NAME_LENGTH: usize = 8;
const NAME_ATTEMPTS: u64 = 100;

/// A base the quickstart will not use. It displays as the one line the quickstart prints,
/// `quickstart: <reason>; <BASE_RULE>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    reason: String,
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "quickstart: {}; {BASE_RULE}", self.reason)
    }
}

/// What `lstat` says of one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// Whether the path is a directory. A link to one is not.
    pub directory: bool,
    /// The owner's uid.
    pub uid: u32,
    /// The permission bits with the set-id and sticky bits, `st_mode & 0o7777`.
    pub mode: u32,
}

impl Entry {
    /// Reads `path` without following a link.
    pub fn of(path: &Path) -> io::Result<Entry> {
        let metadata = std::fs::symlink_metadata(path)?;
        Ok(Entry {
            directory: metadata.file_type().is_dir(),
            uid: metadata.uid(),
            mode: metadata.mode() & 0o7777,
        })
    }
}

/// What [`check_base`] asks of the host. [`System`] asks the operating system.
pub trait Host {
    /// The process's effective uid.
    fn effective_uid(&self) -> u32;
    /// `path`'s entry, read without following a link.
    fn lstat(&self, path: &Path) -> io::Result<Entry>;
    /// Whether `base`, a real path, is on a mount that does not let programs run. `Err` says why
    /// that cannot be told.
    fn noexec(&self, base: &Path) -> Result<bool, String>;
}

/// The host this process runs on.
#[derive(Debug)]
pub struct System {
    uid: u32,
}

impl System {
    /// Reads the effective uid once, from `id -u`, which prints it. `Err` says why it could not.
    pub fn new() -> Result<System, String> {
        System::reading(Command::new("id").arg("-u"))
    }

    /// Reads the effective uid from what `command` prints, which must be what `id -u` prints: one
    /// line of ASCII digits, and an exit status of 0. Output over [`ID_OUTPUT_LIMIT`] is refused.
    pub fn reading(command: &mut Command) -> Result<System, String> {
        let failed = |why: String| format!("cannot read the effective uid: {why}");
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| failed(format!("cannot run id -u: {error}")))?;
        let mut output = Vec::new();
        let read = child
            .stdout
            .take()
            .expect("stdout is piped")
            .take(ID_OUTPUT_LIMIT + 1)
            .read_to_end(&mut output);
        let status = child
            .wait()
            .map_err(|error| failed(format!("cannot wait for id -u: {error}")))?;
        read.map_err(|error| failed(format!("cannot read id -u: {error}")))?;
        if output.len() as u64 > ID_OUTPUT_LIMIT {
            return Err(failed(format!(
                "id -u printed over {ID_OUTPUT_LIMIT} bytes"
            )));
        }
        if !status.success() {
            return Err(failed(format!("id -u exited with {status}")));
        }
        output
            .strip_suffix(b"\n")
            .filter(|digits| digits.iter().all(u8::is_ascii_digit))
            .and_then(|digits| std::str::from_utf8(digits).ok()?.parse().ok())
            .map(|uid| System { uid })
            .ok_or_else(|| {
                failed(format!(
                    "id -u printed {:?}, not a uid",
                    String::from_utf8_lossy(&output)
                ))
            })
    }
}

impl Host for System {
    fn effective_uid(&self) -> u32 {
        self.uid
    }

    fn lstat(&self, path: &Path) -> io::Result<Entry> {
        Entry::of(path)
    }

    fn noexec(&self, base: &Path) -> Result<bool, String> {
        if cfg!(target_os = "linux") {
            noexec_in(Path::new(MOUNTINFO), base)
        } else {
            Ok(false)
        }
    }
}

/// Checks `base` with the README quickstart's checks and refusal lines, in its order except that
/// the write check comes last, then makes the private directory the quickstart keeps its files in.
///
/// `base` must be absolute, with no link in it, as `std::fs::canonicalize` returns it. The checks,
/// in order: its path is UTF-8; `/`, each directory below it and `base` itself is a directory, not
/// a link, owned by root or by `host`'s effective uid, and writable by neither group nor other;
/// `base` is not on a noexec mount; and `base` joined to [`LONGEST_SOCKET`] is shorter than
/// [`SOCKET_LIMIT`] bytes. Last, it makes a directory in `base` named [`PRIVATE_PREFIX`] and
/// eight characters, with mode 0700. Making it is the check that the process can create entries
/// in `base`.
///
/// Returns that directory, which the caller removes. A refusal names the first check that failed
/// and makes nothing.
pub fn check_base(base: &Path, host: &dyn Host) -> Result<PathBuf, Refusal> {
    let bytes = base.as_os_str().as_bytes();
    if std::str::from_utf8(bytes).is_err() {
        return refuse(format!(
            "{} is not valid UTF-8, which the controller's JSON config cannot carry",
            escaped(bytes)
        ));
    }
    let chain: Vec<&Path> = base.ancestors().collect();
    for directory in chain.into_iter().rev() {
        let shown = directory.display();
        let entry = match host.lstat(directory) {
            Ok(entry) => entry,
            Err(error) => return refuse(format!("cannot inspect {shown}: {}", strerror(&error))),
        };
        if !entry.directory {
            return refuse(format!("{shown} is not a directory"));
        }
        if entry.uid != 0 && entry.uid != host.effective_uid() {
            return refuse(format!(
                "{shown} is owned by uid {}, not by you or root",
                entry.uid
            ));
        }
        if entry.mode & 0o022 != 0 {
            return refuse(format!(
                "{shown} has mode {:04o}: group or other can replace entries in it",
                entry.mode
            ));
        }
    }
    let shown = base.display();
    match host.noexec(base) {
        Ok(false) => {}
        Ok(true) => {
            return refuse(format!(
                "{shown} is on a noexec mount, and the build runs programs from it"
            ));
        }
        Err(why) => {
            return refuse(format!(
                "cannot tell whether {shown} is on a noexec mount: {why}"
            ));
        }
    }
    let longest = base.join(LONGEST_SOCKET);
    if longest.as_os_str().len() >= SOCKET_LIMIT {
        return refuse(format!(
            "{shown} is too long: a socket path such as {} must stay under {SOCKET_LIMIT} bytes",
            longest.display()
        ));
    }
    make_private(base)
}

/// Whether the mount that holds `base` has `noexec` among its per-mount options, read from
/// `mountinfo`, a file in the format of `/proc/self/mountinfo`.
///
/// The mount is the one whose mount point is the longest whole-component prefix of `base`; of two
/// on the same point, the later line. `Err` says why it cannot tell: the file cannot be read or is
/// over [`MOUNTINFO_LIMIT`], a line has fewer than six fields, or no mount holds `base`.
pub fn noexec_in(mountinfo: &Path, base: &Path) -> Result<bool, String> {
    let shown = mountinfo.display();
    let mut bytes = Vec::new();
    File::open(mountinfo)
        .and_then(|file| file.take(MOUNTINFO_LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("cannot read {shown}: {}", strerror(&error)))?;
    if bytes.len() as u64 > MOUNTINFO_LIMIT {
        return Err(format!("{shown} is over {MOUNTINFO_LIMIT} bytes"));
    }
    let mut holder: Option<(usize, bool)> = None;
    for line in bytes.split(|&byte| byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&[u8]> = line.split(|&byte| byte == b' ').collect();
        let [_, _, _, _, point, options, ..] = fields[..] else {
            return Err(format!("{shown} has a line with fewer than six fields"));
        };
        let point = unescaped(point);
        let point = Path::new(OsStr::from_bytes(&point));
        if !base.starts_with(point) {
            continue;
        }
        let depth = point.components().count();
        if holder.is_none_or(|(deepest, _)| depth >= deepest) {
            let noexec = options
                .split(|&byte| byte == b',')
                .any(|option| option == b"noexec");
            holder = Some((depth, noexec));
        }
    }
    holder
        .map(|(_, noexec)| noexec)
        .ok_or_else(|| format!("no mount in {shown} holds it"))
}

fn make_private(base: &Path) -> Result<PathBuf, Refusal> {
    let shown = base.display();
    let random = RandomState::new();
    for attempt in 0..NAME_ATTEMPTS {
        let path = base.join(private_name(random.hash_one(attempt)));
        match DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
                ) =>
            {
                return refuse(format!("you cannot create entries in {shown}"));
            }
            Err(error) => {
                return refuse(format!(
                    "cannot create a private directory in {shown}: {}",
                    strerror(&error)
                ));
            }
        }
    }
    refuse(format!(
        "cannot create a private directory in {shown}: {NAME_ATTEMPTS} names were taken"
    ))
}

fn private_name(mut random: u64) -> String {
    let mut name = String::from(PRIVATE_PREFIX);
    let count = NAME_CHARACTERS.len() as u64;
    for _ in 0..NAME_LENGTH {
        name.push(char::from(NAME_CHARACTERS[(random % count) as usize]));
        random /= count;
    }
    name
}

fn unescaped(field: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(field.len());
    let mut rest = field;
    while let Some((&first, tail)) = rest.split_first() {
        if let [
            high @ b'0'..=b'3',
            middle @ b'0'..=b'7',
            low @ b'0'..=b'7',
            after @ ..,
        ] = tail
            && first == b'\\'
        {
            bytes.push((high - b'0') * 64 + (middle - b'0') * 8 + (low - b'0'));
            rest = after;
        } else {
            bytes.push(first);
            rest = tail;
        }
    }
    bytes
}

fn escaped(bytes: &[u8]) -> String {
    let mut shown = String::new();
    for chunk in bytes.utf8_chunks() {
        shown.push_str(chunk.valid());
        for byte in chunk.invalid() {
            shown.push_str(&format!("\\x{byte:02x}"));
        }
    }
    shown
}

fn strerror(error: &io::Error) -> String {
    let text = error.to_string();
    match error.raw_os_error() {
        Some(code) => text
            .strip_suffix(&format!(" (os error {code})"))
            .unwrap_or(&text)
            .to_owned(),
        None => text,
    }
}

fn refuse<T>(reason: String) -> Result<T, Refusal> {
    Err(Refusal { reason })
}
