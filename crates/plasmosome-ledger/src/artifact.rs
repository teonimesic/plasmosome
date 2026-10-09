use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A registry's identity: a non-nil UUID written in canonical lower-case hyphenated form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegistryId(uuid::Uuid);

impl RegistryId {
    /// Accepts only the 36-character lower-case hyphenated form of a non-nil UUID. Upper case,
    /// braces, a `urn:uuid:` prefix and the 32-digit form are refused, although
    /// `Uuid::parse_str` reads them.
    pub fn parse(text: &str) -> Result<RegistryId, ArtifactRefError> {
        match uuid::Uuid::parse_str(text) {
            Ok(uuid) if !uuid.is_nil() && uuid.hyphenated().to_string() == text => {
                Ok(RegistryId(uuid))
            }
            _ => Err(ArtifactRefError::RegistryId {
                text: text.to_owned(),
            }),
        }
    }

    /// The UUID this ID holds.
    pub fn as_uuid(&self) -> &uuid::Uuid {
        &self.0
    }
}

/// Writes the canonical lower-case hyphenated text.
impl fmt::Display for RegistryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.hyphenated(), formatter)
    }
}

impl Serialize for RegistryId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for RegistryId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        RegistryId::parse(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// What a release is. Text forms are exactly `plasmid` and `genome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtifactKind {
    Plasmid,
    Genome,
}

impl ArtifactKind {
    fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::Plasmid => "plasmid",
            ArtifactKind::Genome => "genome",
        }
    }
}

impl Serialize for ArtifactKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ArtifactKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        match text.as_str() {
            "plasmid" => Ok(ArtifactKind::Plasmid),
            "genome" => Ok(ArtifactKind::Genome),
            _ => Err(D::Error::custom(format_args!(
                "kind {text:?} is not plasmid or genome"
            ))),
        }
    }
}

/// Who endorses a release. Text forms are exactly `curated` and `user`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Population {
    Curated,
    User,
}

impl Population {
    fn as_str(self) -> &'static str {
        match self {
            Population::Curated => "curated",
            Population::User => "user",
        }
    }
}

impl Serialize for Population {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Population {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        match text.as_str() {
            "curated" => Ok(Population::Curated),
            "user" => Ok(Population::User),
            _ => Err(D::Error::custom(format_args!(
                "population {text:?} is not curated or user"
            ))),
        }
    }
}

/// A SHA-256 content digest written as `sha256:` and 64 lower-case hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest([u8; 32]);

impl Digest {
    /// Accepts exactly `sha256:` followed by 64 digits in `0-9a-f`. Any other prefix, case,
    /// length or byte is refused; nothing is trimmed or lower-cased.
    pub fn parse(text: &str) -> Result<Digest, ArtifactRefError> {
        let refused = || ArtifactRefError::Digest {
            text: text.to_owned(),
        };
        let digits = text.strip_prefix("sha256:").ok_or_else(refused)?.as_bytes();
        if digits.len() != 64 {
            return Err(refused());
        }
        let mut bytes = [0; 32];
        for (byte, pair) in bytes.iter_mut().zip(digits.chunks_exact(2)) {
            let high = nibble(pair[0]).ok_or_else(refused)?;
            let low = nibble(pair[1]).ok_or_else(refused)?;
            *byte = high << 4 | low;
        }
        Ok(Digest(bytes))
    }

    /// Wraps 32 bytes a caller already hashed with SHA-256. This crate never hashes.
    pub fn from_sha256(bytes: [u8; 32]) -> Digest {
        Digest(bytes)
    }

    /// The 32 digest bytes.
    pub fn sha256(&self) -> &[u8; 32] {
        &self.0
    }
}

fn nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    }
}

/// Writes `sha256:` followed by 64 lower-case hex digits.
impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sha256:")?;
        self.0
            .iter()
            .try_for_each(|byte| write!(formatter, "{byte:02x}"))
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Digest::parse(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// Why a text is not a spec 020 reference value. `field` names the record field, and every
/// variant keeps the refused text unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactRefError {
    RegistryId { text: String },
    Digest { text: String },
}

impl fmt::Display for ArtifactRefError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArtifactRefError::RegistryId { text } => write!(
                formatter,
                "registry_id {text:?} is not a non-nil UUID in lower-case hyphenated form"
            ),
            ArtifactRefError::Digest { text } => write!(
                formatter,
                "digest {text:?} is not sha256: followed by 64 lower-case hex digits"
            ),
        }
    }
}

impl std::error::Error for ArtifactRefError {}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    const UUID: &str = "0b0c5d3e-6c5f-4b1a-9d7e-2f4a8b1c3d5e";
    const HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn digest_text() -> String {
        format!("sha256:{HEX}")
    }

    #[test]
    fn registry_id_accepts_only_canonical_lowercase_non_nil() {
        let id = RegistryId::parse(UUID).unwrap();
        assert_eq!(id.to_string(), UUID);
        assert_eq!(id.as_uuid().hyphenated().to_string(), UUID);
        assert_eq!(serde_json::to_value(id).unwrap(), json!(UUID));
        assert_eq!(serde_json::from_value::<RegistryId>(json!(UUID)).unwrap(), id);
        let refused = [
            UUID.to_uppercase(),
            format!("{{{UUID}}}"),
            format!("urn:uuid:{UUID}"),
            UUID.replace('-', ""),
            "00000000-0000-0000-0000-000000000000".to_string(),
            String::new(),
            UUID[1..].to_string(),
            format!("{UUID}0"),
        ];
        for text in refused {
            assert_eq!(
                RegistryId::parse(&text),
                Err(ArtifactRefError::RegistryId { text: text.clone() }),
                "{text:?}"
            );
            let error = serde_json::from_value::<RegistryId>(json!(text)).unwrap_err();
            assert!(
                error.to_string().starts_with(&format!("registry_id {text:?} ")),
                "{error}"
            );
        }
    }

    #[test]
    fn digest_accepts_only_sha256_prefix_and_64_lowercase_hex() {
        let digest = Digest::parse(&digest_text()).unwrap();
        assert_eq!(
            digest.sha256().to_vec(),
            [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef].repeat(4)
        );
        assert_eq!(digest.to_string(), digest_text());
        assert_eq!(Digest::from_sha256(*digest.sha256()), digest);
        assert_eq!(serde_json::to_value(digest).unwrap(), json!(digest_text()));
        assert_eq!(
            serde_json::from_value::<Digest>(json!(digest_text())).unwrap(),
            digest
        );
        let refused = [
            format!("SHA256:{HEX}"),
            format!("sha-256:{HEX}"),
            format!("sha256{HEX}"),
            format!(":{HEX}"),
            HEX.to_string(),
            "sha256:".to_string(),
            format!("sha256:{}", &HEX[1..]),
            format!("sha256:{HEX}0"),
            format!("sha256:{}A", &HEX[..63]),
            format!("sha256:{}g", &HEX[..63]),
            format!(" sha256:{HEX}"),
            format!("sha256:{HEX}\n"),
        ];
        for text in refused {
            assert_eq!(
                Digest::parse(&text),
                Err(ArtifactRefError::Digest { text: text.clone() }),
                "{text:?}"
            );
            let error = serde_json::from_value::<Digest>(json!(text)).unwrap_err();
            assert!(
                error.to_string().starts_with(&format!("digest {text:?} ")),
                "{error}"
            );
        }
    }

    #[test]
    fn kind_and_population_vocabularies_are_closed() {
        for (text, kind) in [
            ("plasmid", ArtifactKind::Plasmid),
            ("genome", ArtifactKind::Genome),
        ] {
            assert_eq!(serde_json::from_value::<ArtifactKind>(json!(text)).unwrap(), kind);
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(text));
        }
        for (text, population) in [
            ("curated", Population::Curated),
            ("user", Population::User),
        ] {
            assert_eq!(
                serde_json::from_value::<Population>(json!(text)).unwrap(),
                population
            );
            assert_eq!(serde_json::to_value(population).unwrap(), json!(text));
        }
        for text in ["Plasmid", "plugin", "genomes", "curated", "user", ""] {
            let error = serde_json::from_value::<ArtifactKind>(json!(text)).unwrap_err();
            assert!(
                error.to_string().starts_with(&format!("kind {text:?} ")),
                "{error}"
            );
        }
        for text in ["Curated", "curated ", "users", "plasmid", "genome", ""] {
            let error = serde_json::from_value::<Population>(json!(text)).unwrap_err();
            assert!(
                error.to_string().starts_with(&format!("population {text:?} ")),
                "{error}"
            );
        }
        for value in [
            json!({ "plasmid": null }),
            json!({ "curated": null }),
            Value::Null,
            json!(["plasmid"]),
        ] {
            assert!(
                serde_json::from_str::<ArtifactKind>(&value.to_string()).is_err()
                    && serde_json::from_str::<Population>(&value.to_string()).is_err(),
                "{value}"
            );
        }
    }
}
