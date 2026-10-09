use sha2::{Digest as _, Sha256};
use std::io::{ErrorKind, Read};

const READ_CHUNK: usize = 64 * 1024;

/// A SHA-256 digest: 32 bytes, written as 64 lowercase hexadecimal digits.
///
/// `Debug` and `Display` both print those 64 digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Digest([u8; 32]);

/// Why a text is not 64 lowercase hexadecimal digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestError {
    /// The text is `bytes` bytes long, not 64.
    WrongLength { bytes: usize },
    /// The byte at offset `at` is not one of `0-9` or `a-f`.
    NotLowercaseHex { at: usize },
}

impl Digest {
    /// Reads exactly 64 lowercase hexadecimal digits. Uppercase digits,
    /// any other byte, and any other length are refused with the first fault.
    pub fn parse_hex(text: &str) -> Result<Digest, DigestError> {
        let digits = text.as_bytes();
        if digits.len() != 64 {
            return Err(DigestError::WrongLength {
                bytes: digits.len(),
            });
        }
        let mut bytes = [0u8; 32];
        for (at, &digit) in digits.iter().enumerate() {
            let nibble = match digit {
                b'0'..=b'9' => digit - b'0',
                b'a'..=b'f' => digit - b'a' + 10,
                _ => return Err(DigestError::NotLowercaseHex { at }),
            };
            bytes[at / 2] |= if at % 2 == 0 { nibble << 4 } else { nibble };
        }
        Ok(Digest(bytes))
    }

    /// Hashes `bytes` with SHA-256.
    pub fn of(bytes: &[u8]) -> Digest {
        Digest(Sha256::digest(bytes).into())
    }

    /// Hashes everything `reader` yields until end of input, retrying
    /// interrupted reads. Any other read error is returned and no digest is
    /// produced.
    pub fn of_reader(mut reader: impl Read) -> std::io::Result<Digest> {
        let mut hasher = Sha256::new();
        let mut chunk = [0u8; READ_CHUNK];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => return Ok(Digest(hasher.finalize().into())),
                Ok(read) => hasher.update(&chunk[..read]),
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }

    /// The 64 lowercase hexadecimal digits.
    pub fn hex(&self) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut text = String::with_capacity(64);
        for byte in self.0 {
            text.push(char::from(DIGITS[usize::from(byte >> 4)]));
            text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
        }
        text
    }

    /// The 32 raw bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Display for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.hex())
    }
}

impl std::fmt::Debug for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.hex())
    }
}

impl std::fmt::Display for DigestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DigestError::WrongLength { bytes } => write!(
                f,
                "a SHA-256 digest is 64 lowercase hexadecimal digits, not {bytes} bytes"
            ),
            DigestError::NotLowercaseHex { at } => write!(
                f,
                "byte {at} of a SHA-256 digest is not a lowercase hexadecimal digit"
            ),
        }
    }
}

impl std::error::Error for DigestError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error, ErrorKind, Read};

    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const COUNTING: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn with_byte(text: &str, at: usize, replacement: &str) -> String {
        let mut changed = text.to_string();
        changed.replace_range(at..at + 1, replacement);
        changed
    }

    struct Trickle<'a> {
        bytes: &'a [u8],
        reads: usize,
    }

    impl Read for Trickle<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            if self.reads.is_multiple_of(97) {
                return Err(Error::from(ErrorKind::Interrupted));
            }
            let size = (self.reads % 4099 + 1)
                .min(buffer.len())
                .min(self.bytes.len());
            buffer[..size].copy_from_slice(&self.bytes[..size]);
            self.bytes = &self.bytes[size..];
            Ok(size)
        }
    }

    struct FailsAfter {
        served: bool,
    }

    impl Read for FailsAfter {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.served {
                return Err(Error::from(ErrorKind::PermissionDenied));
            }
            self.served = true;
            buffer[0] = b'a';
            Ok(1)
        }
    }

    #[test]
    fn lowercase_hex_round_trips_and_keeps_nibble_order() {
        let digest = Digest::parse_hex(COUNTING).expect("64 lowercase digits parse");
        assert_eq!(
            digest.as_bytes()[..8],
            [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]
        );
        assert_eq!(digest.hex(), COUNTING);
        assert_eq!(digest.to_string(), COUNTING);
        assert_eq!(format!("{digest:?}"), COUNTING);
    }

    #[test]
    fn a_length_other_than_sixty_four_bytes_refuses() {
        assert_eq!(
            Digest::parse_hex(&ABC[..63]),
            Err(DigestError::WrongLength { bytes: 63 })
        );
        assert_eq!(
            Digest::parse_hex(&format!("{ABC}0")),
            Err(DigestError::WrongLength { bytes: 65 })
        );
        assert_eq!(
            Digest::parse_hex(""),
            Err(DigestError::WrongLength { bytes: 0 })
        );
    }

    #[test]
    fn uppercase_digits_refuse_at_their_offset() {
        assert_eq!(
            Digest::parse_hex(&ABC.to_uppercase()),
            Err(DigestError::NotLowercaseHex { at: 0 })
        );
        assert_eq!(
            Digest::parse_hex(&with_byte(ABC, 40, "F")),
            Err(DigestError::NotLowercaseHex { at: 40 })
        );
    }

    #[test]
    fn non_hex_and_multibyte_text_refuses_by_byte_offset() {
        assert_eq!(
            Digest::parse_hex(&with_byte(ABC, 10, "g")),
            Err(DigestError::NotLowercaseHex { at: 10 })
        );
        assert_eq!(
            Digest::parse_hex(&format!("{}é", &ABC[..62])),
            Err(DigestError::NotLowercaseHex { at: 62 })
        );
        assert_eq!(
            Digest::parse_hex(&format!("{}é", &ABC[..63])),
            Err(DigestError::WrongLength { bytes: 65 })
        );
    }

    #[test]
    fn of_matches_the_fips_vectors() {
        assert_eq!(Digest::of(b"abc").hex(), ABC);
        assert_eq!(Digest::of(b"").hex(), EMPTY);
        assert_eq!(Digest::parse_hex(EMPTY), Ok(Digest::of(b"")));
    }

    #[test]
    fn of_reader_with_short_and_interrupted_reads_equals_of() {
        let bytes: Vec<u8> = (0..3 * 1024 * 1024u32)
            .map(|index| (index.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();
        let mut reader = Trickle {
            bytes: &bytes,
            reads: 0,
        };
        let streamed = Digest::of_reader(&mut reader).expect("a trickling reader is hashed");
        assert_eq!(streamed, Digest::of(&bytes));
        assert!(
            reader.reads > 97,
            "the reader was interrupted at least once"
        );
    }

    #[test]
    fn of_reader_returns_the_first_real_read_error() {
        let error = Digest::of_reader(FailsAfter { served: false })
            .expect_err("a failing read produces no digest");
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    }

    #[test]
    fn faults_describe_themselves() {
        assert_eq!(
            DigestError::WrongLength { bytes: 63 }.to_string(),
            "a SHA-256 digest is 64 lowercase hexadecimal digits, not 63 bytes"
        );
        assert_eq!(
            DigestError::NotLowercaseHex { at: 40 }.to_string(),
            "byte 40 of a SHA-256 digest is not a lowercase hexadecimal digit"
        );
    }
}
