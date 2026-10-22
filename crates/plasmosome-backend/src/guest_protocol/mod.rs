use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::digest::{Digest, DigestError};

mod envelope;

pub use envelope::{
    BridgeError, InboundRequest, ProtocolCode, RefusalDetail, RequestId, RequestRefusal,
    encode_failure, encode_request, encode_success, split_request,
};

const GUEST_TEXT_BYTES: usize = 1024;

/// One of the two connections the guest shim opens to the host's 4090
/// control port. The shim opens the normal lane first and the withdrawal lane
/// after the normal lane's hello.
///
/// In JSON it is exactly the string `"normal"` or `"withdrawal"`. Decoding
/// refuses every other value, the map form `{"normal": null}` included.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlLane {
    /// The lane that carries every 4090 verb.
    Normal,
    /// The prioritized lane that admits only hello, observe and remove.
    Withdrawal,
}

const LANES: &[&str] = &["normal", "withdrawal"];

impl<'de> Deserialize<'de> for ControlLane {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ControlLane, D::Error> {
        match String::deserialize(deserializer)?.as_str() {
            "normal" => Ok(ControlLane::Normal),
            "withdrawal" => Ok(ControlLane::Withdrawal),
            other => Err(D::Error::unknown_variant(other, LANES)),
        }
    }
}

/// The guest's boot token: 32 random bytes the shim draws once per boot and
/// returns on both lanes as 64 lowercase hexadecimal digits.
///
/// `Debug` prints those 64 digits, and in JSON it is that string. Decoding
/// accepts exactly what [`BootToken::parse_hex`] accepts.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BootToken([u8; 32]);

/// Why a text is not a boot token. It holds the same fault a digest's text
/// can have, but describes it as a boot token's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootTokenError(pub DigestError);

impl BootToken {
    /// The token whose 32 bytes are `bytes`. The guest calls it once per boot
    /// with bytes from the kernel's initialized random source.
    pub fn from_bytes(bytes: [u8; 32]) -> BootToken {
        BootToken(bytes)
    }

    /// Reads exactly 64 lowercase hexadecimal digits. Uppercase digits, any
    /// other byte and any other length are refused with the first fault.
    pub fn parse_hex(text: &str) -> Result<BootToken, BootTokenError> {
        Digest::parse_hex(text)
            .map(|digest| BootToken(*digest.as_bytes()))
            .map_err(BootTokenError)
    }

    /// The 64 lowercase hexadecimal digits.
    pub fn hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// The 32 raw bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for BootToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

impl Serialize for BootToken {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for BootToken {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<BootToken, D::Error> {
        BootToken::parse_hex(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl fmt::Display for BootTokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            DigestError::WrongLength { bytes } => write!(
                f,
                "a boot token is 64 lowercase hexadecimal digits, not {bytes} bytes"
            ),
            DigestError::NotLowercaseHex { at } => write!(
                f,
                "byte {at} of a boot token is not a lowercase hexadecimal digit"
            ),
        }
    }
}

impl std::error::Error for BootTokenError {}

fn cut(text: &str) -> String {
    text[..text.floor_char_boundary(GUEST_TEXT_BYTES)].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cut_keeps_at_most_1024_bytes_and_ends_on_a_character_boundary() {
        assert_eq!(cut("short"), "short");
        let exact = "a".repeat(1024);
        assert_eq!(cut(&exact), exact);
        assert_eq!(cut(&format!("{exact}b")), exact);
        let wide = format!("{}é", "a".repeat(1023));
        assert_eq!(cut(&wide), "a".repeat(1023));
        let fits = format!("{}é", "a".repeat(1022));
        assert_eq!(cut(&fits), fits);
    }
}
