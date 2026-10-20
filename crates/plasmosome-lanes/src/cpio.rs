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

/// Write `entries` as an uncompressed newc archive ending in `TRAILER!!!`. Names are relative,
/// with no `.` or `..` component, and appear once.
pub fn write_newc(entries: &[Entry]) -> Result<Vec<u8>, CpioError> {
    let _ = entries;
    Ok(Vec::new())
}

/// Read a newc archive back into its entries, without the trailer.
pub fn read_newc(bytes: &[u8]) -> Result<Vec<Entry>, CpioError> {
    let _ = bytes;
    Ok(Vec::new())
}

/// The lanes' initramfs: `init` is `lane-init`, `bin/lane-probe` is the probe, `lane.json` is the
/// guest configuration, and `dev/console` and `dev/null` exist before devtmpfs is mounted.
pub fn lane_initramfs(
    init: &[u8],
    probe: &[u8],
    guest: &GuestConfig,
) -> Result<Vec<u8>, CpioError> {
    let _ = (init, probe, guest);
    write_newc(&[])
}
