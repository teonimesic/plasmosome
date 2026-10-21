use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::os::unix::fs::FileExt;

use super::artifacts::VerifiedArtifact;
use super::digest::Digest;
use super::recipe::ArtifactRole;

/// The largest initramfs [`verify_initramfs`] reads: 256 MiB. A larger one is refused, never cut.
pub const MAX_INITRAMFS_BYTES: u64 = 256 << 20;

/// What [`verify_initramfs`] found in an archive that binds the guest policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitramfsBinding {
    /// The size of the `init` entry's data.
    pub init_size: u64,
    /// The SHA-256 of the `init` entry's data.
    pub init_digest: Digest,
    /// The SHA-256 of the `plasmosome/guest-policy` entry's data, equal to the guest policy's.
    pub policy_digest: Digest,
    /// How many entries come before the trailer.
    pub entries: usize,
}

/// Why an initramfs does not bind the guest policy as spec 001 §4.2 asks. An `offset` counts bytes
/// from the start of the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitramfsRefusal {
    /// An artifact with this role was passed where the initramfs or the guest policy belongs.
    WrongRole { role: ArtifactRole },
    /// The archive holds `bytes` bytes, more than [`MAX_INITRAMFS_BYTES`]. When it grew after
    /// verification, `bytes` counts only what was read: the bound plus one.
    TooLarge { bytes: u64 },
    /// The entry at `offset` does not start with the newc magic `070701`. Compressed archives,
    /// the other cpio formats and zero bytes between entries are refused here.
    NotNewc { offset: u64 },
    /// A header field of the entry at `offset` is not 8 hexadecimal digits, or is a `filesize`
    /// other than 0 on a directory or the trailer, which the kernel would skip without reading.
    BadHeaderField { offset: u64, field: &'static str },
    /// The entry at `offset` has a check field other than 0.
    NonzeroCheck { offset: u64 },
    /// The name of the entry at `offset` is not one plain relative spelling, for `reason`.
    BadName { offset: u64, reason: &'static str },
    /// A second entry has this name.
    DuplicateEntry { name: String },
    /// This entry is a symbolic link.
    SymlinkEntry { name: String },
    /// This entry is neither a regular file, a directory nor a symlink; `mode` is its mode field.
    UnsupportedType { name: String, mode: u32 },
    /// This regular entry has an `nlink` other than 1, so the kernel may make it a hard link.
    HardLink { name: String, nlink: u32 },
    /// No earlier directory entry is this entry's parent.
    MissingParent { name: String },
    /// The part of the archive that starts at `offset` runs past its end.
    Truncated { offset: u64 },
    /// The padding byte at `offset` is not zero.
    NonzeroPadding { offset: u64 },
    /// The byte at `offset` comes after the trailer and is not zero. The kernel would unpack
    /// what follows as another archive.
    DataAfterTrailer { offset: u64 },
    /// The archive ends without a `TRAILER!!!` entry.
    MissingTrailer,
    /// No regular entry is named `init`.
    MissingInit,
    /// The `init` entry has no execute bit; `mode` is its mode field.
    InitNotExecutable { mode: u32 },
    /// The `init` entry is empty.
    InitEmpty,
    /// No regular entry is named `plasmosome/guest-policy`.
    MissingGuestPolicy,
    /// The `plasmosome/guest-policy` entry hashes to `actual`, not the verified `expected`.
    GuestPolicyMismatch { expected: Digest, actual: Digest },
    /// A read of the initramfs through its descriptor failed with this errno.
    Io { op: &'static str, errno: i32 },
}

impl fmt::Display for InitramfsRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use InitramfsRefusal::*;
        match self {
            WrongRole { role } => write!(
                f,
                "the {role} was passed where the initramfs or the guest policy belongs"
            ),
            TooLarge { bytes } => write!(
                f,
                "the initramfs holds {bytes} bytes, more than {MAX_INITRAMFS_BYTES}"
            ),
            NotNewc { offset } => write!(f, "the entry at byte {offset} is not newc 070701"),
            BadHeaderField { offset, field } => write!(
                f,
                "the {field} field of the entry at byte {offset} is not 8 hex digits, or is a \
                 body on a directory or the trailer"
            ),
            NonzeroCheck { offset } => {
                write!(f, "the entry at byte {offset} has a nonzero check field")
            }
            BadName { offset, reason } => {
                write!(f, "the name of the entry at byte {offset} {reason}")
            }
            DuplicateEntry { name } => write!(f, "{name} appears twice"),
            SymlinkEntry { name } => write!(f, "{name} is a symlink"),
            UnsupportedType { name, mode } => write!(
                f,
                "{name} has mode {mode:06o}, neither a regular file nor a directory"
            ),
            HardLink { name, nlink } => write!(f, "{name} has nlink {nlink}, not 1"),
            MissingParent { name } => write!(f, "{name} has no earlier parent directory"),
            Truncated { offset } => write!(f, "the part at byte {offset} runs past the end"),
            NonzeroPadding { offset } => write!(f, "padding byte {offset} is not zero"),
            DataAfterTrailer { offset } => {
                write!(f, "byte {offset}, after the trailer, is not zero")
            }
            MissingTrailer => f.write_str("the archive ends with no TRAILER!!! entry"),
            MissingInit => f.write_str("the archive has no regular init entry"),
            InitNotExecutable { mode } => write!(f, "init has mode {mode:06o}, no execute bit"),
            InitEmpty => f.write_str("init is empty"),
            MissingGuestPolicy => {
                f.write_str("the archive has no regular plasmosome/guest-policy entry")
            }
            GuestPolicyMismatch { expected, actual } => write!(
                f,
                "plasmosome/guest-policy hashes to {actual}, not the verified {expected}"
            ),
            Io { op, errno } => write!(f, "{op} failed with errno {errno}"),
        }
    }
}

impl std::error::Error for InitramfsRefusal {}

/// Proves that the verified `initramfs` binds the verified `guest_policy`, as spec 001 §4.2 asks
/// before launch.
///
/// `initramfs` must have the role `Initramfs` and `guest_policy` the role `GuestPolicy`, or it
/// refuses `WrongRole`. An initramfs over [`MAX_INITRAMFS_BYTES`] is refused before any read.
/// Otherwise it reads the archive from offset 0 to its end with positioned reads through
/// `initramfs.file()`, never by path, through a limit of the bound plus one byte. The archive
/// must be uncompressed newc whose entries are plain relative names, regular files with one link
/// or directories, each name once with its parent before it, ending in `TRAILER!!!` and then only
/// zero bytes. It must hold a regular, executable, non-empty `init`, a `plasmosome` directory and
/// a regular `plasmosome/guest-policy` whose bytes hash to `guest_policy.digest()`.
///
/// It reads the bytes the descriptor holds now and does not hash them again. A write through a
/// shared mapping made before the file was locked can change them without moving its size or
/// times, as [`VerifiedArtifact::reconfirm`] says. Call `reconfirm` after this, just before the
/// launch that opens the initramfs by path.
pub fn verify_initramfs(
    initramfs: &VerifiedArtifact,
    guest_policy: &VerifiedArtifact,
) -> Result<InitramfsBinding, InitramfsRefusal> {
    for (artifact, role) in [
        (initramfs, ArtifactRole::Initramfs),
        (guest_policy, ArtifactRole::GuestPolicy),
    ] {
        if artifact.role() != role {
            return Err(InitramfsRefusal::WrongRole {
                role: artifact.role(),
            });
        }
    }
    let bytes = read_bounded(initramfs.file(), initramfs.size(), MAX_INITRAMFS_BYTES)?;
    audit_newc(&bytes, guest_policy.digest())
}

const MAGIC: &[u8; 6] = b"070701";
const HEADER_BYTES: usize = 110;
const PATH_MAX: usize = 4096;
const TRAILER: &str = "TRAILER!!!";
const POLICY_NAME: &str = "plasmosome/guest-policy";
const FIELDS: [&str; 13] = [
    "ino",
    "mode",
    "uid",
    "gid",
    "nlink",
    "mtime",
    "filesize",
    "devmajor",
    "devminor",
    "rdevmajor",
    "rdevminor",
    "namesize",
    "check",
];
const TYPE_BITS: u32 = 0o170000;
const REGULAR: u32 = 0o100000;
const DIRECTORY: u32 = 0o040000;
const SYMLINK: u32 = 0o120000;

struct Entry<'a> {
    at: usize,
    mode: u32,
    nlink: u32,
    name: &'a str,
    data: &'a [u8],
    end: usize,
}

pub(crate) fn audit_newc(
    bytes: &[u8],
    expected_policy: &Digest,
) -> Result<InitramfsBinding, InitramfsRefusal> {
    let mut seen: HashMap<&str, bool> = HashMap::new();
    let (mut init, mut policy) = (None, None);
    let mut at = 0;
    loop {
        let entry = read_entry(bytes, at)?;
        if entry.name == TRAILER {
            if !entry.data.is_empty() {
                return Err(bad_field(entry.at, "filesize"));
            }
            let after = bytes.get(entry.end..).unwrap_or_default();
            return match after.iter().position(|byte| *byte != 0) {
                Some(index) => Err(InitramfsRefusal::DataAfterTrailer {
                    offset: (entry.end + index) as u64,
                }),
                None => bind(init, policy, expected_policy, seen.len()),
            };
        }
        judge(&entry, &seen)?;
        let regular = entry.mode & TYPE_BITS == REGULAR;
        match entry.name {
            "init" if regular => init = Some((entry.mode, entry.data)),
            POLICY_NAME if regular => policy = Some(entry.data),
            _ => {}
        }
        seen.insert(entry.name, !regular);
        at = entry.end;
    }
}

fn bind(
    init: Option<(u32, &[u8])>,
    policy: Option<&[u8]>,
    expected: &Digest,
    entries: usize,
) -> Result<InitramfsBinding, InitramfsRefusal> {
    let (mode, init) = init.ok_or(InitramfsRefusal::MissingInit)?;
    if mode & 0o111 == 0 {
        return Err(InitramfsRefusal::InitNotExecutable { mode });
    }
    if init.is_empty() {
        return Err(InitramfsRefusal::InitEmpty);
    }
    let actual = Digest::of(policy.ok_or(InitramfsRefusal::MissingGuestPolicy)?);
    if actual != *expected {
        return Err(InitramfsRefusal::GuestPolicyMismatch {
            expected: *expected,
            actual,
        });
    }
    Ok(InitramfsBinding {
        init_size: init.len() as u64,
        init_digest: Digest::of(init),
        policy_digest: actual,
        entries,
    })
}

fn read_entry(bytes: &[u8], at: usize) -> Result<Entry<'_>, InitramfsRefusal> {
    let offset = at as u64;
    let rest = bytes.get(at..).unwrap_or_default();
    if rest.is_empty() {
        return Err(InitramfsRefusal::MissingTrailer);
    }
    match rest.get(..MAGIC.len()) {
        Some(magic) if magic == MAGIC => {}
        None if MAGIC.starts_with(rest) => return Err(InitramfsRefusal::Truncated { offset }),
        _ => return Err(InitramfsRefusal::NotNewc { offset }),
    }
    let header = take(bytes, at, HEADER_BYTES)?;
    let mut fields = [0; 13];
    for (index, field) in fields.iter_mut().enumerate() {
        let start = MAGIC.len() + index * 8;
        *field = header
            .get(start..start + 8)
            .and_then(hex_field)
            .ok_or_else(|| bad_field(at, FIELDS[index]))?;
    }
    let [
        _,
        mode,
        _,
        _,
        nlink,
        _,
        filesize,
        _,
        _,
        _,
        _,
        namesize,
        check,
    ] = fields;
    if check != 0 {
        return Err(InitramfsRefusal::NonzeroCheck { offset });
    }
    let bad_name = |reason| InitramfsRefusal::BadName { offset, reason };
    let namesize = namesize as usize;
    if namesize < 2 {
        return Err(bad_name("is empty"));
    }
    if namesize > PATH_MAX {
        return Err(bad_name("is longer than PATH_MAX"));
    }
    let name_at = at + HEADER_BYTES;
    let Some((0, name)) = take(bytes, name_at, namesize)?.split_last() else {
        return Err(bad_name("does not end in NUL"));
    };
    if name.contains(&0) {
        return Err(bad_name("holds a NUL before its end"));
    }
    let name = std::str::from_utf8(name).map_err(|_| bad_name("is not UTF-8"))?;
    let data_at = padded(bytes, name_at + namesize)?;
    let data = take(bytes, data_at, filesize as usize)?;
    let end = padded(bytes, data_at + data.len())?;
    Ok(Entry {
        at,
        mode,
        nlink,
        name,
        data,
        end,
    })
}

fn judge(entry: &Entry<'_>, seen: &HashMap<&str, bool>) -> Result<(), InitramfsRefusal> {
    if let Some(reason) = name_fault(entry.name) {
        let offset = entry.at as u64;
        return Err(InitramfsRefusal::BadName { offset, reason });
    }
    let name = entry.name.to_string();
    match entry.mode & TYPE_BITS {
        REGULAR if entry.nlink != 1 => {
            let nlink = entry.nlink;
            return Err(InitramfsRefusal::HardLink { name, nlink });
        }
        DIRECTORY if !entry.data.is_empty() => return Err(bad_field(entry.at, "filesize")),
        REGULAR | DIRECTORY => {}
        SYMLINK => return Err(InitramfsRefusal::SymlinkEntry { name }),
        _ => {
            let mode = entry.mode;
            return Err(InitramfsRefusal::UnsupportedType { name, mode });
        }
    }
    if let Some((parent, _)) = entry.name.rsplit_once('/')
        && seen.get(parent) != Some(&true)
    {
        return Err(InitramfsRefusal::MissingParent { name });
    }
    if seen.contains_key(entry.name) {
        return Err(InitramfsRefusal::DuplicateEntry { name });
    }
    Ok(())
}

fn name_fault(name: &str) -> Option<&'static str> {
    let mut parts = name.split('/');
    if name.starts_with('/') {
        Some("is absolute")
    } else if name.ends_with('/') {
        Some("ends in /")
    } else if parts.clone().any(str::is_empty) {
        Some("has an empty part")
    } else if parts.any(|part| part == "." || part == "..") {
        Some("has a . or .. part")
    } else {
        None
    }
}

fn hex_field(text: &[u8]) -> Option<u32> {
    text.iter().try_fold(0, |value, byte| {
        Some((value << 4) | char::from(*byte).to_digit(16)?)
    })
}

fn bad_field(at: usize, field: &'static str) -> InitramfsRefusal {
    InitramfsRefusal::BadHeaderField {
        offset: at as u64,
        field,
    }
}

fn take(bytes: &[u8], at: usize, length: usize) -> Result<&[u8], InitramfsRefusal> {
    at.checked_add(length)
        .and_then(|end| bytes.get(at..end))
        .ok_or(InitramfsRefusal::Truncated { offset: at as u64 })
}

fn padded(bytes: &[u8], at: usize) -> Result<usize, InitramfsRefusal> {
    let end = at.next_multiple_of(4);
    match take(bytes, at, end - at)?
        .iter()
        .position(|byte| *byte != 0)
    {
        Some(index) => Err(InitramfsRefusal::NonzeroPadding {
            offset: (at + index) as u64,
        }),
        None => Ok(end),
    }
}

fn read_bounded(file: &File, size: u64, bound: u64) -> Result<Vec<u8>, InitramfsRefusal> {
    if size > bound {
        return Err(InitramfsRefusal::TooLarge { bytes: size });
    }
    let mut bytes = Vec::with_capacity(usize::try_from(size).map_or(0, |size| size + 1));
    Positioned { file, offset: 0 }
        .take(bound.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| InitramfsRefusal::Io {
            op: "pread",
            errno: error.raw_os_error().unwrap_or(libc::EIO),
        })?;
    match bytes.len() as u64 {
        read if read > bound => Err(InitramfsRefusal::TooLarge { bytes: read }),
        _ => Ok(bytes),
    }
}

struct Positioned<'a> {
    file: &'a File,
    offset: u64,
}

impl Read for Positioned<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.file.read_at(buffer, self.offset)?;
        self.offset += read as u64;
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_fixtures::newc::{
        Entry, INIT, POLICY, REGULAR, SYMLINK, encode, offsets, standard,
    };
    use InitramfsRefusal::*;
    use proptest::prelude::*;

    fn audit_bytes(bytes: &[u8]) -> Result<InitramfsBinding, InitramfsRefusal> {
        audit_newc(bytes, &Digest::of(POLICY))
    }

    fn audit(entries: &[Entry]) -> Result<InitramfsBinding, InitramfsRefusal> {
        audit_bytes(&encode(entries))
    }

    fn binding(entries: usize) -> InitramfsBinding {
        InitramfsBinding {
            init_size: INIT.len() as u64,
            init_digest: Digest::of(INIT),
            policy_digest: Digest::of(POLICY),
            entries,
        }
    }

    fn with(index: usize, change: impl FnOnce(&mut Entry)) -> Vec<Entry> {
        let mut entries = standard();
        change(&mut entries[index]);
        entries
    }

    fn plus(entry: Entry) -> Vec<Entry> {
        let mut entries = standard();
        entries.insert(3, entry);
        entries
    }

    fn named(name: &[u8]) -> Entry {
        Entry {
            name: name.to_vec(),
            ..Entry::file("x", 0o644, b"x")
        }
    }

    fn data_start(entries: &[Entry], index: usize) -> u64 {
        (offsets(entries)[index] + 110 + entries[index].name.len() as u64).next_multiple_of(4)
    }

    fn name(text: &str) -> String {
        text.to_string()
    }

    #[test]
    fn a_minimal_archive_binds_init_and_the_policy() {
        assert_eq!(audit(&standard()), Ok(binding(3)));
    }

    #[test]
    fn zero_bytes_after_the_trailer_pass_as_in_the_qualified_bundle() {
        for zeros in [1, 3, 4, 88] {
            let mut bytes = encode(&standard());
            bytes.resize(bytes.len() + zeros, 0);
            assert_eq!(audit_bytes(&bytes), Ok(binding(3)), "{zeros} zero bytes");
        }
    }

    #[test]
    fn a_second_archive_or_any_nonzero_byte_after_the_trailer_refuses_data_after_trailer() {
        let first = encode(&standard());
        let second = encode(&[
            Entry::file("init", 0o755, b"another init"),
            Entry::trailer(),
        ]);
        let end = first.len() as u64;
        for (tail, offset) in [
            ([&[0u8; 4][..], &second].concat(), end + 4),
            (second.clone(), end),
            (vec![0, 0, 7], end + 2),
        ] {
            let bytes = [first.as_slice(), &tail].concat();
            assert_eq!(audit_bytes(&bytes), Err(DataAfterTrailer { offset }));
        }
    }

    #[test]
    fn a_second_entry_with_a_name_already_seen_refuses_duplicate_entry() {
        for (entry, duplicate) in [
            (Entry::file("init", 0o755, b"a later init"), "init"),
            (Entry::dir("plasmosome"), "plasmosome"),
            (
                Entry::file("plasmosome/guest-policy", 0o444, POLICY),
                "plasmosome/guest-policy",
            ),
        ] {
            assert_eq!(
                audit(&plus(entry)),
                Err(DuplicateEntry {
                    name: name(duplicate)
                })
            );
        }
    }

    #[test]
    fn a_symlinked_init_or_a_symlinked_policy_directory_refuses_symlink_entry() {
        let link = |index: usize, target: &[u8]| {
            with(index, |entry| {
                entry.mode = SYMLINK | 0o777;
                entry.data = target.to_vec();
            })
        };
        assert_eq!(
            audit(&link(0, b"/bin/sh")),
            Err(SymlinkEntry { name: name("init") })
        );
        assert_eq!(
            audit(&link(1, b"/tmp")),
            Err(SymlinkEntry {
                name: name("plasmosome")
            })
        );
    }

    #[test]
    fn a_name_that_is_not_one_plain_relative_spelling_refuses_bad_name() {
        let long = [vec![b'a'; 4096], vec![0]].concat();
        let rows: [(&[u8], &str); 14] = [
            (b"/init\0", "is absolute"),
            (b"/\0", "is absolute"),
            (b"../init\0", "has a . or .. part"),
            (b"./init\0", "has a . or .. part"),
            (b".\0", "has a . or .. part"),
            (b"plasmosome/../init\0", "has a . or .. part"),
            (b"plasmosome//guest-policy\0", "has an empty part"),
            (b"init/\0", "ends in /"),
            (b"\0", "is empty"),
            (b"", "is empty"),
            (b"init\0x\0", "holds a NUL before its end"),
            (b"init", "does not end in NUL"),
            (b"\xff\0", "is not UTF-8"),
            (&long, "is longer than PATH_MAX"),
        ];
        for (area, reason) in rows {
            let entries = plus(named(area));
            let offset = offsets(&entries)[3];
            assert_eq!(
                audit(&entries),
                Err(BadName { offset, reason }),
                "{:?}",
                String::from_utf8_lossy(area)
            );
        }
        let longest = [vec![b'a'; 4095], vec![0]].concat();
        assert_eq!(audit(&plus(named(&longest))), Ok(binding(4)));
    }

    #[test]
    fn a_policy_entry_one_byte_off_refuses_with_both_digests() {
        let mut changed = POLICY.to_vec();
        changed[0] ^= 1;
        let entries = with(2, |entry| entry.data = changed.clone());
        assert_eq!(
            audit(&entries),
            Err(GuestPolicyMismatch {
                expected: Digest::of(POLICY),
                actual: Digest::of(&changed),
            })
        );
    }

    #[test]
    fn a_missing_init_policy_or_parent_refuses_with_its_own_variant() {
        let without = |index: usize| {
            let mut entries = standard();
            entries.remove(index);
            entries
        };
        let mut reordered = standard();
        reordered.swap(1, 2);
        let policy = || MissingParent {
            name: name("plasmosome/guest-policy"),
        };
        let rows = [
            (without(0), MissingInit),
            (with(0, |entry| *entry = Entry::dir("init")), MissingInit),
            (without(2), MissingGuestPolicy),
            (
                with(2, |entry| *entry = Entry::dir("plasmosome/guest-policy")),
                MissingGuestPolicy,
            ),
            (without(1), policy()),
            (
                with(1, |entry| *entry = Entry::file("plasmosome", 0o644, b"x")),
                policy(),
            ),
            (reordered, policy()),
            (
                plus(Entry::file("plasmosome/a/b", 0o644, b"x")),
                MissingParent {
                    name: name("plasmosome/a/b"),
                },
            ),
        ];
        for (entries, refusal) in rows {
            assert_eq!(audit(&entries), Err(refusal));
        }
    }

    #[test]
    fn a_regular_entry_with_other_than_one_link_refuses_hard_link_and_a_directory_may_have_any() {
        for (index, nlink, entry) in [
            (0, 2, "init"),
            (0, 0, "init"),
            (2, 2, "plasmosome/guest-policy"),
        ] {
            assert_eq!(
                audit(&with(index, |found| found.nlink = nlink)),
                Err(HardLink {
                    name: name(entry),
                    nlink
                })
            );
        }
        for nlink in [0, 1, 3, 7] {
            let entries = with(1, |entry| entry.nlink = nlink);
            assert_eq!(audit(&entries), Ok(binding(3)), "nlink {nlink}");
        }
    }

    #[test]
    fn a_device_fifo_socket_or_unknown_type_refuses_unsupported_type() {
        for mode in [
            0o020644, 0o060644, 0o010644, 0o140644, 0o030644, 0o000644, 0o170644,
        ] {
            let entry = Entry::node("node", mode, b"");
            assert_eq!(
                audit(&plus(entry)),
                Err(UnsupportedType {
                    name: name("node"),
                    mode
                }),
                "{mode:o}"
            );
        }
    }

    #[test]
    fn compressed_bytes_other_cpio_magics_and_stray_zero_bytes_refuse_not_newc() {
        let archive = encode(&standard());
        let second = offsets(&standard())[1] as usize;
        let magic = |at: usize, text: &[u8; 6]| {
            let mut bytes = archive.clone();
            bytes[at..at + 6].copy_from_slice(text);
            bytes
        };
        let spliced =
            |at: usize, inserted: &[u8]| [&archive[..at], inserted, &archive[at..]].concat();
        let rows = [
            (spliced(0, &[0x1f, 0x8b, 0x08, 0x00]), 0),
            (spliced(0, &[0x28, 0xb5, 0x2f, 0xfd]), 0),
            (vec![0x1f, 0x8b], 0),
            (magic(0, b"070702"), 0),
            (magic(0, b"070707"), 0),
            (magic(second, b"070702"), second),
            (spliced(0, &[0; 4]), 0),
            (spliced(second, &[0; 4]), second),
        ];
        for (bytes, offset) in rows {
            let offset = offset as u64;
            assert_eq!(audit_bytes(&bytes), Err(NotNewc { offset }));
        }
    }

    #[test]
    fn an_archive_cut_inside_a_header_name_padding_or_data_refuses_truncated() {
        let entries = standard();
        let archive = encode(&entries);
        let at = offsets(&entries);
        let cut = |end: u64| archive[..end as usize].to_vec();
        let name_end = at[0] + 110 + 5;
        let rows = [
            (b"0707".to_vec(), 0),
            (cut(50), 0),
            (cut(at[1] + 60), at[1]),
            (cut(at[2] + 115), at[2] + 110),
            (cut(name_end), name_end),
            (cut(data_start(&entries, 0) + 4), data_start(&entries, 0)),
            (
                cut(data_start(&entries, 2) + POLICY.len() as u64),
                data_start(&entries, 2) + POLICY.len() as u64,
            ),
        ];
        for (bytes, offset) in rows {
            assert_eq!(audit_bytes(&bytes), Err(Truncated { offset }));
        }
        let past = with(2, |entry| entry.filesize = Some(1 << 20));
        assert_eq!(
            audit(&past),
            Err(Truncated {
                offset: data_start(&past, 2)
            })
        );
    }

    proptest! {
        #[test]
        fn flipped_cut_or_extended_archives_never_panic(
            flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 0..8),
            cut in prop::option::of(any::<prop::sample::Index>()),
            extra in prop::collection::vec(any::<u8>(), 0..64),
        ) {
            let mut bytes = encode(&standard());
            for (index, byte) in flips {
                let at = index.index(bytes.len());
                bytes[at] = byte;
            }
            if let Some(cut) = cut {
                bytes.truncate(cut.index(bytes.len()));
            }
            bytes.extend(extra);
            let _ = audit_bytes(&bytes);
        }
    }

    #[test]
    fn nonzero_padding_a_nonzero_check_or_a_field_that_is_not_hex_refuses() {
        let padded = plus(Entry {
            pad: 1,
            ..Entry::file("a", 0o644, b"abc")
        });
        let rows = [
            (
                with(0, |entry| entry.pad = 1),
                NonzeroPadding { offset: 115 },
            ),
            (
                padded.clone(),
                NonzeroPadding {
                    offset: data_start(&padded, 3) + 3,
                },
            ),
            (
                with(1, |entry| entry.check = 1),
                NonzeroCheck {
                    offset: offsets(&standard())[1],
                },
            ),
        ];
        for (entries, refusal) in rows {
            assert_eq!(audit(&entries), Err(refusal));
        }
        let archive = encode(&standard());
        let policy_at = offsets(&standard())[2] as usize;
        let fields = "ino mode uid gid nlink mtime filesize devmajor devminor rdevmajor rdevminor";
        let fields = fields.split(' ').chain(["namesize", "check"]);
        for (index, field) in fields.enumerate() {
            for (bad, entry) in [(b'g', 0), (b'+', policy_at), (b' ', policy_at)] {
                let mut bytes = archive.clone();
                bytes[entry + 6 + index * 8] = bad;
                let offset = entry as u64;
                assert_eq!(
                    audit_bytes(&bytes),
                    Err(BadHeaderField { offset, field }),
                    "{field} {bad}"
                );
            }
        }
    }

    #[test]
    fn upper_case_hex_passes_and_a_directory_or_trailer_with_a_body_refuses() {
        let mut upper = standard();
        for entry in &mut upper {
            entry.upper_hex = true;
        }
        assert_eq!(audit(&upper), Ok(binding(3)));
        let at = offsets(&standard());
        for index in [1, 3] {
            let entries = with(index, |entry| entry.data = b"body".to_vec());
            assert_eq!(
                audit(&entries),
                Err(BadHeaderField {
                    offset: at[index],
                    field: "filesize"
                })
            );
        }
    }

    #[test]
    fn init_needs_an_execute_bit_and_bytes() {
        let mode = REGULAR | 0o644;
        assert_eq!(
            audit(&with(0, |entry| entry.mode = mode)),
            Err(InitNotExecutable { mode })
        );
        assert_eq!(audit(&with(0, |entry| entry.data.clear())), Err(InitEmpty));
        for execute in [0o100, 0o010, 0o001] {
            let entries = with(0, |entry| entry.mode = REGULAR | 0o444 | execute);
            assert_eq!(audit(&entries), Ok(binding(3)), "{execute:o}");
        }
    }

    #[test]
    fn an_archive_without_its_trailer_refuses_missing_trailer() {
        assert_eq!(audit(&standard()[..3]), Err(MissingTrailer));
        assert_eq!(audit_bytes(b""), Err(MissingTrailer));
    }

    #[test]
    fn the_read_refuses_past_the_bound_before_or_while_reading_and_names_a_failed_pread() {
        use std::os::unix::fs::FileExt;
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("initramfs");
        std::fs::write(&path, b"0123456789").expect("write");
        let file = File::open(&path).expect("open");
        let directory = File::open(dir.path()).expect("a directory opens for reading");
        assert_eq!(read_bounded(&file, 10, 10), Ok(b"0123456789".to_vec()));
        assert_eq!(read_bounded(&file, 9, 9), Err(TooLarge { bytes: 10 }));
        assert_eq!(
            read_bounded(&directory, 11, 10),
            Err(TooLarge { bytes: 11 })
        );
        let errno = directory
            .read_at(&mut [0], 0)
            .expect_err("a directory cannot be read")
            .raw_os_error()
            .expect("an errno");
        assert_eq!(
            read_bounded(&directory, 0, 10),
            Err(Io { op: "pread", errno })
        );
    }

    #[test]
    fn each_refusal_says_what_was_found() {
        let (expected, actual) = (Digest::of(b"a"), Digest::of(b"b"));
        let mismatch = format!("{actual}, not the verified {expected}");
        let rows = [
            (
                WrongRole {
                    role: ArtifactRole::Kernel,
                },
                "the kernel was passed",
            ),
            (TooLarge { bytes: 7 }, "7 bytes, more than"),
            (NotNewc { offset: 8 }, "at byte 8 is not newc"),
            (
                BadHeaderField {
                    offset: 9,
                    field: "mode",
                },
                "mode field of the entry at byte 9",
            ),
            (
                NonzeroCheck { offset: 10 },
                "entry at byte 10 has a nonzero check",
            ),
            (
                BadName {
                    offset: 11,
                    reason: "is empty",
                },
                "name of the entry at byte 11 is empty",
            ),
            (DuplicateEntry { name: name("init") }, "init appears twice"),
            (SymlinkEntry { name: name("init") }, "init is a symlink"),
            (
                UnsupportedType {
                    name: name("n"),
                    mode: 0o20644,
                },
                "n has mode 020644",
            ),
            (
                HardLink {
                    name: name("init"),
                    nlink: 2,
                },
                "init has nlink 2",
            ),
            (
                MissingParent { name: name("a/b") },
                "a/b has no earlier parent",
            ),
            (Truncated { offset: 12 }, "at byte 12 runs past the end"),
            (NonzeroPadding { offset: 13 }, "padding byte 13 is not zero"),
            (
                DataAfterTrailer { offset: 14 },
                "byte 14, after the trailer",
            ),
            (MissingTrailer, "no TRAILER!!!"),
            (MissingInit, "no regular init"),
            (InitNotExecutable { mode: 0o100644 }, "init has mode 100644"),
            (InitEmpty, "init is empty"),
            (MissingGuestPolicy, "no regular plasmosome/guest-policy"),
            (GuestPolicyMismatch { expected, actual }, &mismatch),
            (
                Io {
                    op: "pread",
                    errno: 5,
                },
                "pread failed with errno 5",
            ),
        ];
        for (refusal, part) in rows {
            let shown = refusal.to_string();
            assert!(shown.contains(part), "{shown:?} names {part}");
        }
    }

    #[cfg(target_os = "macos")]
    mod macos {
        use super::*;
        use crate::runtime::artifacts::verify_artifact;
        use crate::runtime::recipe::Artifact;
        use crate::runtime::strict_json;
        use crate::runtime::test_fixtures::tree::{ArtifactTree, set_flags, set_mode};
        use std::os::unix::fs::FileExt;
        use std::path::Path;

        fn verified(role: ArtifactRole, path: &Path, bytes: &[u8]) -> VerifiedArtifact {
            let text = serde_json::json!({
                "path": path.to_str().expect("test paths are UTF-8"),
                "sha256": Digest::of(bytes).hex(),
            })
            .to_string();
            let value = strict_json::parse_value(text.as_bytes()).expect("the artifact is JSON");
            let artifact =
                Artifact::from_value(&value, "initramfs").expect("the artifact is valid");
            verify_artifact(role, &artifact).expect("the artifact verifies")
        }

        #[test]
        fn verified_artifacts_bind_and_a_swapped_path_still_reads_the_verified_archive() {
            let tree = ArtifactTree::new();
            let archive = encode(&standard());
            let path = tree.immutable_file("initramfs", &archive);
            let policy_path = tree.immutable_file("guest-policy", POLICY);
            let initramfs = verified(ArtifactRole::Initramfs, &path, &archive);
            let policy = verified(ArtifactRole::GuestPolicy, &policy_path, POLICY);
            assert_eq!(verify_initramfs(&initramfs, &policy), Ok(binding(3)));
            set_flags(&path, 0);
            std::fs::rename(&path, tree.join("initramfs.old")).expect("rename");
            let other = encode(&with(2, |entry| entry.data = b"another policy".to_vec()));
            tree.immutable_file("initramfs", &other);
            assert_eq!(verify_initramfs(&initramfs, &policy), Ok(binding(3)));
        }

        #[test]
        fn a_role_mix_up_refuses_wrong_role() {
            let tree = ArtifactTree::new();
            let archive = encode(&standard());
            let kernel_path = tree.immutable_file("kernel", &archive);
            let path = tree.immutable_file("initramfs", &archive);
            let policy_path = tree.immutable_file("guest-policy", POLICY);
            let kernel = verified(ArtifactRole::Kernel, &kernel_path, &archive);
            let initramfs = verified(ArtifactRole::Initramfs, &path, &archive);
            let policy = verified(ArtifactRole::GuestPolicy, &policy_path, POLICY);
            for (first, second, role) in [
                (&kernel, &policy, ArtifactRole::Kernel),
                (&policy, &policy, ArtifactRole::GuestPolicy),
                (&initramfs, &initramfs, ArtifactRole::Initramfs),
                (&initramfs, &kernel, ArtifactRole::Kernel),
            ] {
                assert_eq!(verify_initramfs(first, second), Err(WrongRole { role }));
            }
        }

        #[test]
        fn bytes_a_held_writer_appends_after_verification_refuse_data_after_trailer() {
            let tree = ArtifactTree::new();
            let archive = encode(&standard());
            let path = tree.file("initramfs", &archive, 0o644);
            let writer = std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("a writer opened before the lock");
            set_mode(&path, 0o444);
            set_flags(&path, libc::UF_IMMUTABLE);
            let policy_path = tree.immutable_file("guest-policy", POLICY);
            let initramfs = verified(ArtifactRole::Initramfs, &path, &archive);
            let policy = verified(ArtifactRole::GuestPolicy, &policy_path, POLICY);
            let second = encode(&[
                Entry::file("init", 0o755, b"another init"),
                Entry::trailer(),
            ]);
            writer
                .write_all_at(&second, archive.len() as u64)
                .expect("the held writer still writes under uchg");
            assert_eq!(
                verify_initramfs(&initramfs, &policy),
                Err(DataAfterTrailer {
                    offset: archive.len() as u64
                })
            );
        }
    }
}
