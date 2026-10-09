use std::cmp::Ordering;
use std::fmt;

use serde::de::{Error as _, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer, forward_to_deserialize_any};

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

/// What a release is. Text forms are exactly `plasmid` and `genome`, and ordering compares
/// them, so `Genome` sorts before `Plasmid`.
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

impl Ord for ArtifactKind {
    fn cmp(&self, other: &ArtifactKind) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for ArtifactKind {
    fn partial_cmp(&self, other: &ArtifactKind) -> Option<Ordering> {
        Some(self.cmp(other))
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

/// Who endorses a release. Text forms are exactly `curated` and `user`, and ordering compares
/// them, so `Curated` sorts before `User`.
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

impl Ord for Population {
    fn cmp(&self, other: &Population) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for Population {
    fn partial_cmp(&self, other: &Population) -> Option<Ordering> {
        Some(self.cmp(other))
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

/// A release address without its digest: kind, population, publisher, name and version.
///
/// Ordering compares those fields in that order, matching spec 020's catalog order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReleaseKey {
    kind: ArtifactKind,
    population: Population,
    publisher: String,
    name: String,
    version: String,
}

impl ReleaseKey {
    /// Checks publisher and name against the identifier grammar and version against the
    /// version grammar, refusing the first field that fails. Never trims, folds case or
    /// substitutes.
    pub fn new(
        kind: ArtifactKind,
        population: Population,
        publisher: &str,
        name: &str,
        version: &str,
    ) -> Result<ReleaseKey, ArtifactRefError> {
        ReleaseKey::checked(
            kind,
            population,
            publisher.to_owned(),
            name.to_owned(),
            version.to_owned(),
        )
    }

    fn checked(
        kind: ArtifactKind,
        population: Population,
        publisher: String,
        name: String,
        version: String,
    ) -> Result<ReleaseKey, ArtifactRefError> {
        check_identifier("publisher", &publisher)?;
        check_identifier("name", &name)?;
        check_version(&version)?;
        Ok(ReleaseKey {
            kind,
            population,
            publisher,
            name,
            version,
        })
    }

    pub fn kind(&self) -> ArtifactKind {
        self.kind
    }

    pub fn population(&self) -> Population {
        self.population
    }

    pub fn publisher(&self) -> &str {
        &self.publisher
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    fn serialize_fields<S: SerializeStruct>(&self, record: &mut S) -> Result<(), S::Error> {
        record.serialize_field("kind", &self.kind)?;
        record.serialize_field("population", &self.population)?;
        record.serialize_field("publisher", &self.publisher)?;
        record.serialize_field("name", &self.name)?;
        record.serialize_field("version", &self.version)
    }
}

impl Serialize for ReleaseKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut record = serializer.serialize_struct("ReleaseKey", 5)?;
        self.serialize_fields(&mut record)?;
        record.end()
    }
}

#[derive(Deserialize)]
#[serde(rename = "ReleaseKey", deny_unknown_fields)]
struct ReleaseKeyWire {
    kind: ArtifactKind,
    population: Population,
    publisher: String,
    name: String,
    version: String,
}

impl<'de> Deserialize<'de> for ReleaseKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ReleaseKeyWire::deserialize(ObjectOnly(deserializer))?;
        ReleaseKey::checked(
            wire.kind,
            wire.population,
            wire.publisher,
            wire.name,
            wire.version,
        )
        .map_err(D::Error::custom)
    }
}

/// A release address pinned to its descriptor digest: exactly spec 020's six fields. The
/// registry ID belongs to the enclosing record, never to the reference.
///
/// Ordering compares the key, then the digest bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReleaseRef {
    key: ReleaseKey,
    digest: Digest,
}

impl ReleaseRef {
    /// Pairs a key with its digest. Both are already checked, so this cannot fail.
    pub fn new(key: ReleaseKey, digest: Digest) -> ReleaseRef {
        ReleaseRef { key, digest }
    }

    pub fn key(&self) -> &ReleaseKey {
        &self.key
    }

    pub fn digest(&self) -> &Digest {
        &self.digest
    }
}

impl Serialize for ReleaseRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut record = serializer.serialize_struct("ReleaseRef", 6)?;
        self.key.serialize_fields(&mut record)?;
        record.serialize_field("digest", &self.digest)?;
        record.end()
    }
}

#[derive(Deserialize)]
#[serde(rename = "ReleaseRef", deny_unknown_fields)]
struct ReleaseRefWire {
    kind: ArtifactKind,
    population: Population,
    publisher: String,
    name: String,
    version: String,
    digest: Digest,
}

impl<'de> Deserialize<'de> for ReleaseRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ReleaseRefWire::deserialize(ObjectOnly(deserializer))?;
        let key = ReleaseKey::checked(
            wire.kind,
            wire.population,
            wire.publisher,
            wire.name,
            wire.version,
        )
        .map_err(D::Error::custom)?;
        Ok(ReleaseRef::new(key, wire.digest))
    }
}

struct ObjectOnly<D>(D);

impl<'de, D: Deserializer<'de>> Deserializer<'de> for ObjectOnly<D> {
    type Error = D::Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.0.deserialize_map(visitor)
    }

    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
        option unit unit_struct newtype_struct seq tuple tuple_struct map struct enum identifier
        ignored_any
    }
}

/// Checks one publisher or name, and is also usable for a client alias: `[a-z0-9]+(-[a-z0-9]+)*`
/// within 64 bytes. `field` names the record field in the error.
pub fn check_identifier(field: &'static str, text: &str) -> Result<(), ArtifactRefError> {
    let segments_valid = text.split('-').all(|segment| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    });
    if text.len() <= 64 && segments_valid {
        Ok(())
    } else {
        Err(ArtifactRefError::Identifier {
            field,
            text: text.to_owned(),
        })
    }
}

/// Checks one exact version: `[A-Za-z0-9][A-Za-z0-9._-]*` within 128 bytes. The text is never
/// interpreted, so `latest` is accepted as an ordinary version string.
pub fn check_version(text: &str) -> Result<(), ArtifactRefError> {
    let starts_alphanumeric = text
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric());
    let rest_valid = text
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if text.len() <= 128 && starts_alphanumeric && rest_valid {
        Ok(())
    } else {
        Err(ArtifactRefError::Version {
            text: text.to_owned(),
        })
    }
}

/// Why a text is not a spec 020 reference value. Every variant keeps the refused text
/// unchanged, and `Display` starts with the field's name and the quoted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactRefError {
    /// Not the lower-case hyphenated text of a non-nil UUID.
    RegistryId { text: String },
    /// Not `sha256:` followed by 64 lower-case hex digits.
    Digest { text: String },
    /// A publisher or name, as `field` says, outside `[a-z0-9]+(-[a-z0-9]+)*` or over 64 bytes.
    Identifier { field: &'static str, text: String },
    /// Outside `[A-Za-z0-9][A-Za-z0-9._-]*` or over 128 bytes.
    Version { text: String },
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
            ArtifactRefError::Identifier { field, text } => write!(
                formatter,
                "{field} {text:?} does not match [a-z0-9]+(-[a-z0-9]+)* within 64 bytes"
            ),
            ArtifactRefError::Version { text } => write!(
                formatter,
                "version {text:?} does not match [A-Za-z0-9][A-Za-z0-9._-]* within 128 bytes"
            ),
        }
    }
}

impl std::error::Error for ArtifactRefError {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use proptest::prelude::*;
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
        assert_eq!(
            serde_json::from_value::<RegistryId>(json!(UUID)).unwrap(),
            id
        );
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
                error
                    .to_string()
                    .starts_with(&format!("registry_id {text:?} ")),
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
            assert_eq!(
                serde_json::from_value::<ArtifactKind>(json!(text)).unwrap(),
                kind
            );
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(text));
        }
        for (text, population) in [("curated", Population::Curated), ("user", Population::User)] {
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
                error
                    .to_string()
                    .starts_with(&format!("population {text:?} ")),
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

    #[test]
    fn identifiers_follow_the_publisher_and_name_grammar() {
        let longest = format!("{}-b", "a".repeat(62));
        assert_eq!(longest.len(), 64);
        for text in ["a", "a1", "0", "github-pr", "a-b-c", longest.as_str()] {
            assert_eq!(check_identifier("publisher", text), Ok(()), "{text:?}");
        }
        let too_long = format!("{longest}c");
        for text in [
            "",
            "-a",
            "a-",
            "a--b",
            "A",
            "a_b",
            "a.b",
            "a b",
            "\u{e9}",
            too_long.as_str(),
        ] {
            assert_eq!(
                check_identifier("name", text),
                Err(ArtifactRefError::Identifier {
                    field: "name",
                    text: text.to_string(),
                }),
                "{text:?}"
            );
        }
    }

    #[test]
    fn versions_follow_the_exact_version_grammar() {
        let longest = format!("1.a_B-{}", "z".repeat(122));
        assert_eq!(longest.len(), 128);
        for text in [
            "1",
            "1.2.0",
            "RC-1",
            "v1_2",
            "latest",
            "Z",
            longest.as_str(),
        ] {
            assert_eq!(check_version(text), Ok(()), "{text:?}");
        }
        let too_long = format!("{longest}z");
        for text in [
            "",
            ".1",
            "-1",
            "_1",
            "1.2.0+build",
            "1 2",
            "latest ",
            " 1",
            "1\n",
            "1/2",
            "\u{e9}",
            too_long.as_str(),
        ] {
            assert_eq!(
                check_version(text),
                Err(ArtifactRefError::Version {
                    text: text.to_string(),
                }),
                "{text:?}"
            );
        }
    }

    #[test]
    fn release_key_new_checks_each_field_and_keeps_its_text() {
        let key = ReleaseKey::new(
            ArtifactKind::Genome,
            Population::User,
            "acme",
            "researcher",
            "latest",
        )
        .unwrap();
        assert_eq!(key.kind(), ArtifactKind::Genome);
        assert_eq!(key.population(), Population::User);
        assert_eq!(key.publisher(), "acme");
        assert_eq!(key.name(), "researcher");
        assert_eq!(key.version(), "latest");
        let refused = |publisher, name, version| {
            ReleaseKey::new(
                ArtifactKind::Plasmid,
                Population::Curated,
                publisher,
                name,
                version,
            )
            .unwrap_err()
        };
        assert_eq!(
            refused("Acme", "researcher", "1"),
            ArtifactRefError::Identifier {
                field: "publisher",
                text: "Acme".to_string(),
            }
        );
        assert_eq!(
            refused("acme", "re_searcher", "1"),
            ArtifactRefError::Identifier {
                field: "name",
                text: "re_searcher".to_string(),
            }
        );
        assert_eq!(
            refused("acme", "researcher", "1+2"),
            ArtifactRefError::Version {
                text: "1+2".to_string(),
            }
        );
        assert_eq!(
            refused("acme", "researcher", "1+2").to_string(),
            "version \"1+2\" does not match [A-Za-z0-9][A-Za-z0-9._-]* within 128 bytes"
        );
        assert_eq!(
            refused("Acme", "researcher", "1").to_string(),
            "publisher \"Acme\" does not match [a-z0-9]+(-[a-z0-9]+)* within 64 bytes"
        );
    }

    const SIX: [&str; 6] = [
        "kind",
        "population",
        "publisher",
        "name",
        "version",
        "digest",
    ];

    fn release_json() -> Value {
        json!({
            "kind": "plasmid",
            "population": "curated",
            "publisher": "plasmosome",
            "name": "github-pr",
            "version": "1.2.0",
            "digest": digest_text(),
        })
    }

    fn key_json() -> Value {
        let mut key = release_json();
        key.as_object_mut().unwrap().remove("digest");
        key
    }

    fn refusal<T: serde::de::DeserializeOwned + fmt::Debug>(text: &str) -> String {
        serde_json::from_str::<T>(text).unwrap_err().to_string()
    }

    #[test]
    fn release_ref_round_trips_its_six_fields() {
        let text = release_json().to_string();
        let release: ReleaseRef = serde_json::from_str(&text).unwrap();
        let key = release.key();
        assert_eq!(key.kind(), ArtifactKind::Plasmid);
        assert_eq!(key.population(), Population::Curated);
        assert_eq!(key.publisher(), "plasmosome");
        assert_eq!(key.name(), "github-pr");
        assert_eq!(key.version(), "1.2.0");
        assert_eq!(release.digest(), &Digest::parse(&digest_text()).unwrap());
        assert_eq!(
            serde_json::to_string(&release).unwrap(),
            format!(
                "{{\"kind\":\"plasmid\",\"population\":\"curated\",\"publisher\":\"plasmosome\",\
                 \"name\":\"github-pr\",\"version\":\"1.2.0\",\"digest\":\"{}\"}}",
                digest_text()
            )
        );
        assert_eq!(serde_json::to_value(&release).unwrap(), release_json());
        let alone: ReleaseKey = serde_json::from_str(&key_json().to_string()).unwrap();
        assert_eq!(&alone, key);
        assert_eq!(
            serde_json::to_string(&alone).unwrap(),
            "{\"kind\":\"plasmid\",\"population\":\"curated\",\"publisher\":\"plasmosome\",\
             \"name\":\"github-pr\",\"version\":\"1.2.0\"}"
        );
        assert_eq!(ReleaseRef::new(alone, *release.digest()), release);
    }

    #[test]
    fn release_ref_refuses_missing_unknown_null_and_duplicate_fields() {
        let records: [(&str, Value, &[&str], fn(&str) -> String); 2] = [
            ("ReleaseRef", release_json(), &SIX, refusal::<ReleaseRef>),
            ("ReleaseKey", key_json(), &SIX[..5], refusal::<ReleaseKey>),
        ];
        for (record, value, fields, refuse) in records {
            let mut cases = Vec::new();
            for field in fields {
                let mut missing = value.clone();
                missing.as_object_mut().unwrap().remove(*field);
                cases.push((missing.to_string(), format!("missing field `{field}`")));
                let mut null = value.clone();
                null[*field] = Value::Null;
                cases.push((null.to_string(), "invalid type: null".to_string()));
            }
            let mut unknown = value.clone();
            unknown["registry_id"] = json!(UUID);
            cases.push((
                unknown.to_string(),
                "unknown field `registry_id`".to_string(),
            ));
            let text = value.to_string();
            let duplicated = text.replacen(
                "\"name\":\"github-pr\"",
                "\"name\":\"github-pr\",\"name\":\"github-pr\"",
                1,
            );
            assert_ne!(duplicated, text);
            cases.push((duplicated, "duplicate field `name`".to_string()));
            let positional: Vec<Value> = fields.iter().map(|field| value[*field].clone()).collect();
            cases.push((
                Value::Array(positional).to_string(),
                "invalid type: sequence".to_string(),
            ));
            for (text, reason) in cases {
                let error = refuse(&text);
                assert!(error.contains(&reason), "{record} {text}: {error}");
            }
        }
        let mut with_digest = key_json();
        with_digest["digest"] = json!(digest_text());
        assert!(refusal::<ReleaseKey>(&with_digest.to_string()).contains("unknown field `digest`"));
    }

    #[test]
    fn release_ref_refuses_each_field_grammar_violation() {
        for (field, bad) in [
            ("kind", "plugin"),
            ("population", "users"),
            ("publisher", "Plasmosome"),
            ("name", "github_pr"),
            ("version", "1.2.0+build"),
            ("digest", "sha256:abc"),
        ] {
            let mut value = release_json();
            value[field] = json!(bad);
            let error = refusal::<ReleaseRef>(&value.to_string());
            assert!(
                error.starts_with(&format!("{field} {bad:?} ")),
                "{field}: {error}"
            );
            if field != "digest" {
                let mut key = key_json();
                key[field] = json!(bad);
                let error = refusal::<ReleaseKey>(&key.to_string());
                assert!(
                    error.starts_with(&format!("{field} {bad:?} ")),
                    "{field}: {error}"
                );
            }
        }
    }

    #[test]
    fn release_ref_parses_from_the_spec_020_toml_inline_table() {
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Member {
            release: ReleaseRef,
            mock: String,
        }

        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Genome {
            id: String,
            version: String,
            description: String,
            plasmids: BTreeMap<String, Member>,
        }

        let expected: ReleaseRef = serde_json::from_value(release_json()).unwrap();
        let header = "id = \"researcher\"\nversion = \"1.0.0\"\n\
                      description = \"Work on pull requests with recorded GitHub responses.\"\n";
        let six = format!(
            "kind = \"plasmid\", population = \"curated\", publisher = \"plasmosome\", \
             name = \"github-pr\", version = \"1.2.0\", digest = \"{}\"",
            digest_text()
        );
        let inline =
            format!("{header}\n[plasmids.github-pr]\nrelease = {{ {six} }}\nmock = \"simulate\"\n");
        let nested = format!(
            "{header}\n[plasmids.github-pr]\nmock = \"simulate\"\n\n\
             [plasmids.github-pr.release]\n{}\n",
            six.replace(", ", "\n")
        );
        for text in [inline, nested] {
            let genome: Genome = toml::from_str(&text).unwrap();
            assert_eq!(
                (genome.id.as_str(), genome.version.as_str()),
                ("researcher", "1.0.0")
            );
            assert!(genome.description.starts_with("Work on pull requests"));
            let member = &genome.plasmids["github-pr"];
            assert_eq!(member.release, expected);
            assert_eq!(member.mock, "simulate");
        }
        let member = |release: &str| {
            format!(
                "{header}\n[plasmids.github-pr]\nrelease = {{ {release} }}\nmock = \"simulate\"\n"
            )
        };
        assert!(toml::from_str::<Genome>(&member(&six)).is_ok());
        for (release, reason) in [
            (
                six.split(", digest").next().unwrap().to_string(),
                "missing field `digest`",
            ),
            (
                six.replace("kind = \"plasmid\"", "kind = { plasmid = {} }"),
                "invalid type: map",
            ),
        ] {
            let error = toml::from_str::<Genome>(&member(&release)).unwrap_err();
            assert!(error.to_string().contains(reason), "{error}");
        }
    }

    #[test]
    fn release_keys_sort_like_the_catalog() {
        use ArtifactKind::{Genome, Plasmid};
        use Population::{Curated, User};
        let key = |kind, population, publisher, name, version| {
            ReleaseKey::new(kind, population, publisher, name, version).unwrap()
        };
        let catalog = vec![
            key(Genome, Curated, "z", "z", "9"),
            key(Genome, User, "a", "a", "1"),
            key(Plasmid, Curated, "a", "a", "1"),
            key(Plasmid, Curated, "a", "a", "1.10.0"),
            key(Plasmid, Curated, "a", "a", "1.9.0"),
            key(Plasmid, Curated, "a", "a", "B"),
            key(Plasmid, Curated, "a", "a", "a"),
            key(Plasmid, Curated, "a", "a-b", "1"),
            key(Plasmid, Curated, "a", "a1", "1"),
            key(Plasmid, Curated, "a", "b", "0"),
            key(Plasmid, Curated, "b", "a", "0"),
            key(Plasmid, User, "a", "a", "0"),
        ];
        let mut sorted = catalog.clone();
        sorted.reverse();
        sorted.sort();
        assert_eq!(sorted, catalog);
        let low = Digest::from_sha256([0; 32]);
        let high = Digest::from_sha256([0xff; 32]);
        assert!(low < high && low.to_string() < high.to_string());
        let first = catalog[0].clone();
        let mut references = vec![
            ReleaseRef::new(catalog[1].clone(), low),
            ReleaseRef::new(first.clone(), high),
            ReleaseRef::new(first.clone(), low),
        ];
        references.sort();
        assert_eq!(
            references,
            vec![
                ReleaseRef::new(first.clone(), low),
                ReleaseRef::new(first, high),
                ReleaseRef::new(catalog[1].clone(), low),
            ]
        );
    }

    proptest! {
        #[test]
        fn generated_identifiers_round_trip(
            identifier in "[a-z0-9]{1,8}(-[a-z0-9]{1,8}){0,3}",
        ) {
            prop_assert_eq!(check_identifier("name", &identifier), Ok(()));
            let key = ReleaseKey::new(
                ArtifactKind::Plasmid,
                Population::User,
                &identifier,
                &identifier,
                "1.0.0",
            )
            .unwrap();
            let text = serde_json::to_string(&key).unwrap();
            prop_assert_eq!(serde_json::from_str::<ReleaseKey>(&text).unwrap(), key);
        }

        #[test]
        fn one_upper_case_byte_is_refused(
            identifier in "[a-z0-9]{1,8}(-[a-z0-9]{1,8}){0,3}",
            at in any::<prop::sample::Index>(),
        ) {
            let mut bytes = identifier.into_bytes();
            let at = at.index(bytes.len());
            bytes[at] = b'A';
            let text = String::from_utf8(bytes).unwrap();
            prop_assert!(check_identifier("publisher", &text).is_err(), "{:?}", text);
        }
    }
}
