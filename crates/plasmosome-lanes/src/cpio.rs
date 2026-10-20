use std::fmt;

use crate::config::GuestConfig;

/// File type bits of a regular file.
pub const S_IFREG: u32 = 0o100000;
/// File type bits of a directory.
pub const S_IFDIR: u32 = 0o040000;
/// File type bits of a character device.
pub const S_IFCHR: u32 = 0o020000;

/// One member of a newc archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev_major: u32,
    pub rdev_minor: u32,
    pub data: Vec<u8>,
}

impl Entry {
    /// A regular file owned by root, with permission bits `perm`.
    pub fn file(name: &str, perm: u32, data: Vec<u8>) -> Entry {
        Entry {
            name: name.to_string(),
            mode: S_IFREG | perm,
            uid: 0,
            gid: 0,
            rdev_major: 0,
            rdev_minor: 0,
            data,
        }
    }

    /// A directory owned by root, with permission bits `perm`.
    pub fn dir(name: &str, perm: u32) -> Entry {
        Entry {
            mode: S_IFDIR | perm,
            ..Entry::file(name, 0, Vec::new())
        }
    }

    /// A character device node owned by root.
    pub fn char_device(name: &str, perm: u32, major: u32, minor: u32) -> Entry {
        Entry {
            mode: S_IFCHR | perm,
            rdev_major: major,
            rdev_minor: minor,
            ..Entry::file(name, 0, Vec::new())
        }
    }
}

/// Why an archive could not be written or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpioError(pub String);

impl fmt::Display for CpioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CpioError {}

const MAGIC: &[u8; 6] = b"070701";
const HEADER: usize = 110;
const TRAILER: &str = "TRAILER!!!";

fn padding(length: usize) -> usize {
    (4 - length % 4) % 4
}

fn check_name(name: &str) -> Result<(), CpioError> {
    let plain = name
        .split('/')
        .all(|part| !part.is_empty() && part != "." && part != "..");
    if !plain || name.contains('\0') || name == TRAILER {
        return Err(CpioError(format!(
            "`{name}` is not a relative name of plain parts: no `.`, `..`, empty part or trailing `/`"
        )));
    }
    Ok(())
}

fn push_header(out: &mut Vec<u8>, fields: [u32; 13]) {
    out.extend_from_slice(MAGIC);
    for field in fields {
        out.extend_from_slice(format!("{field:08x}").as_bytes());
    }
}

fn push_member(out: &mut Vec<u8>, ino: u32, entry: &Entry, nlink: u32) -> Result<(), CpioError> {
    let too_big = |what: &str| CpioError(format!("`{}`: {what} does not fit newc", entry.name));
    let size = u32::try_from(entry.data.len()).map_err(|_| too_big("its size"))?;
    let name_size = u32::try_from(entry.name.len() + 1).map_err(|_| too_big("its name"))?;
    push_header(
        out,
        [
            ino,
            entry.mode,
            entry.uid,
            entry.gid,
            nlink,
            0,
            size,
            0,
            0,
            entry.rdev_major,
            entry.rdev_minor,
            name_size,
            0,
        ],
    );
    out.extend_from_slice(entry.name.as_bytes());
    out.push(0);
    out.resize(out.len() + padding(HEADER + entry.name.len() + 1), 0);
    out.extend_from_slice(&entry.data);
    out.resize(out.len() + padding(entry.data.len()), 0);
    Ok(())
}

/// Write `entries` as an uncompressed newc archive ending in `TRAILER!!!`. A name is relative and
/// made of parts joined by single `/`s, with no `.`, `..` or empty part and no trailing `/`, so
/// each guest path has one spelling. Each name appears once.
pub fn write_newc(entries: &[Entry]) -> Result<Vec<u8>, CpioError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        check_name(&entry.name)?;
        if !seen.insert(entry.name.as_str()) {
            return Err(CpioError(format!(
                "`{}` appears more than once",
                entry.name
            )));
        }
        let nlink = if entry.mode & 0o170000 == S_IFDIR {
            2
        } else {
            1
        };
        let ino = u32::try_from(index + 1)
            .map_err(|_| CpioError("too many entries for newc".to_string()))?;
        push_member(&mut out, ino, entry, nlink)?;
    }
    let trailer = Entry {
        mode: 0,
        ..Entry::file(TRAILER, 0, Vec::new())
    };
    push_member(&mut out, 0, &trailer, 1)?;
    Ok(out)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], CpioError> {
        let end = self
            .at
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| CpioError(format!("the archive ends early at byte {}", self.at)))?;
        let taken = &self.bytes[self.at..end];
        self.at = end;
        Ok(taken)
    }

    fn field(&mut self) -> Result<u32, CpioError> {
        let text = std::str::from_utf8(self.take(8)?)
            .map_err(|_| CpioError("a header field is not ASCII".to_string()))?;
        u32::from_str_radix(text, 16)
            .map_err(|_| CpioError(format!("header field `{text}` is not hexadecimal")))
    }
}

/// Read a newc archive back into its entries, without the trailer.
pub fn read_newc(bytes: &[u8]) -> Result<Vec<Entry>, CpioError> {
    let mut cursor = Cursor { bytes, at: 0 };
    let mut entries = Vec::new();
    loop {
        if cursor.take(6)? != MAGIC {
            return Err(CpioError(format!(
                "no newc magic at byte {}",
                cursor.at - 6
            )));
        }
        let mut fields = [0u32; 13];
        for field in &mut fields {
            *field = cursor.field()?;
        }
        let [
            _,
            mode,
            uid,
            gid,
            _,
            _,
            size,
            _,
            _,
            rdev_major,
            rdev_minor,
            name_size,
            _,
        ] = fields;
        let name_bytes = cursor.take(name_size as usize)?;
        let name = std::str::from_utf8(name_bytes.strip_suffix(&[0]).unwrap_or(name_bytes))
            .map_err(|_| CpioError("a name is not UTF-8".to_string()))?
            .to_string();
        cursor.take(padding(HEADER + name_size as usize))?;
        if name == TRAILER {
            return Ok(entries);
        }
        let data = cursor.take(size as usize)?.to_vec();
        cursor.take(padding(size as usize))?;
        entries.push(Entry {
            name,
            mode,
            uid,
            gid,
            rdev_major,
            rdev_minor,
            data,
        });
    }
}

/// The lanes' initramfs: `init` is `lane-init`, `bin/lane-probe` is the probe, `lane.json` is the
/// guest configuration, and `dev/console` and `dev/null` exist before devtmpfs is mounted.
pub fn lane_initramfs(
    init: &[u8],
    probe: &[u8],
    guest: &GuestConfig,
) -> Result<Vec<u8>, CpioError> {
    let configuration = guest
        .to_json()
        .map_err(|error| CpioError(format!("lane.json: {error}")))?;
    write_newc(&[
        Entry::dir("dev", 0o755),
        Entry::char_device("dev/console", 0o600, 5, 1),
        Entry::char_device("dev/null", 0o666, 1, 3),
        Entry::dir("proc", 0o555),
        Entry::dir("sys", 0o555),
        Entry::dir("run", 0o755),
        Entry::dir("bin", 0o755),
        Entry::file("bin/lane-probe", 0o755, probe.to_vec()),
        Entry::file("lane.json", 0o644, configuration.into_bytes()),
        Entry::file("init", 0o755, init.to_vec()),
    ])
}
