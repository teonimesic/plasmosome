pub(crate) const REGULAR: u32 = 0o100000;
pub(crate) const DIRECTORY: u32 = 0o040000;
pub(crate) const SYMLINK: u32 = 0o120000;

pub(crate) const POLICY: &[u8] = b"(guest policy)\n";
pub(crate) const INIT: &[u8] = b"\x7fELF the guest's PID1";

/// One newc entry as the builder writes it. `name` is the whole name area, NUL included, so a
/// test can leave the NUL out or put one early.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub(crate) name: Vec<u8>,
    pub(crate) mode: u32,
    pub(crate) nlink: u32,
    pub(crate) data: Vec<u8>,
    pub(crate) filesize: Option<u32>,
    pub(crate) check: u32,
    pub(crate) pad: u8,
    pub(crate) upper_hex: bool,
}

impl Entry {
    pub(crate) fn node(name: &str, mode: u32, data: &[u8]) -> Entry {
        let mut area = name.as_bytes().to_vec();
        area.push(0);
        Entry {
            name: area,
            mode,
            nlink: if mode & 0o170000 == DIRECTORY { 2 } else { 1 },
            data: data.to_vec(),
            filesize: None,
            check: 0,
            pad: 0,
            upper_hex: false,
        }
    }

    pub(crate) fn file(name: &str, permissions: u32, data: &[u8]) -> Entry {
        Entry::node(name, REGULAR | permissions, data)
    }

    pub(crate) fn dir(name: &str) -> Entry {
        Entry::node(name, DIRECTORY | 0o755, b"")
    }

    pub(crate) fn trailer() -> Entry {
        Entry {
            nlink: 1,
            ..Entry::node("TRAILER!!!", 0, b"")
        }
    }
}

/// The smallest archive that binds `policy`: `init`, `plasmosome/`, `plasmosome/guest-policy`
/// and the trailer.
pub(crate) fn bound(init: &[u8], policy: &[u8]) -> Vec<Entry> {
    vec![
        Entry::file("init", 0o755, init),
        Entry::dir("plasmosome"),
        Entry::file("plasmosome/guest-policy", 0o444, policy),
        Entry::trailer(),
    ]
}

pub(crate) fn standard() -> Vec<Entry> {
    bound(INIT, POLICY)
}

fn pad(out: &mut Vec<u8>, byte: u8) {
    while out.len() % 4 != 0 {
        out.push(byte);
    }
}

/// Writes `entries` in order as newc, padding each name and data area to 4 bytes with the
/// entry's `pad` byte. It adds no trailer: give one with [`Entry::trailer`].
pub(crate) fn encode(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let size = entry.filesize.unwrap_or(entry.data.len() as u32);
        let fields = [
            index as u32 + 1,
            entry.mode,
            501,
            20,
            entry.nlink,
            0,
            size,
            0,
            0,
            0,
            0,
            entry.name.len() as u32,
            entry.check,
        ];
        out.extend_from_slice(b"070701");
        for field in fields {
            let text = if entry.upper_hex {
                format!("{field:08X}")
            } else {
                format!("{field:08x}")
            };
            out.extend_from_slice(text.as_bytes());
        }
        out.extend_from_slice(&entry.name);
        pad(&mut out, entry.pad);
        out.extend_from_slice(&entry.data);
        pad(&mut out, entry.pad);
    }
    out
}

/// Where each entry's header starts in [`encode`]'s output.
pub(crate) fn offsets(entries: &[Entry]) -> Vec<u64> {
    (0..entries.len())
        .map(|index| encode(&entries[..index]).len() as u64)
        .collect()
}
