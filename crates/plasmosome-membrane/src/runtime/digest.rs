/// A SHA-256 digest: 32 bytes, written as 64 lowercase hexadecimal digits.
///
/// `Debug` and `Display` both print those 64 digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Digest([u8; 32]);

/// Why a text is not 64 lowercase hexadecimal digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestFault {
    /// The text is `bytes` bytes long, not 64.
    WrongLength { bytes: usize },
    /// The byte at offset `at` is not one of `0-9` or `a-f`.
    NotLowercaseHex { at: usize },
}

impl Digest {
    /// Reads exactly 64 lowercase hexadecimal digits. Uppercase digits,
    /// any other byte, and any other length are refused with the first fault.
    pub fn parse_hex(_text: &str) -> Result<Digest, DigestFault> {
        todo!()
    }

    /// Hashes `bytes` with SHA-256.
    pub fn of(_bytes: &[u8]) -> Digest {
        todo!()
    }

    /// Hashes everything `reader` yields until end of input, retrying
    /// interrupted reads. Any other read error is returned and no digest is
    /// produced.
    pub fn of_reader(_reader: impl std::io::Read) -> std::io::Result<Digest> {
        todo!()
    }

    /// The 64 lowercase hexadecimal digits.
    pub fn hex(&self) -> String {
        todo!()
    }

    /// The 32 raw bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Display for Digest {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::fmt::Debug for Digest {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::fmt::Display for DigestFault {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for DigestFault {}

