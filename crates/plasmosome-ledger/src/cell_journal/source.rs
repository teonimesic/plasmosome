use std::fmt;

use plasmosome_backend::PluginId;
use serde::de::{Error as _, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use super::{CellJournalFaultKind, JournalRuleError};
use crate::artifact::{ArtifactKind, RegistryId, ReleaseRef};

/// Where an attachment came from. `Local` is the unregistered selection and asserts no registry
/// provenance. `Registry` names the registry, the imported graph's root and this plugin's
/// release in it.
///
/// It encodes as `{"kind":"local"}` or `{"kind":"registry","registry_id":..,"root":..,
/// "member":..}`. Decoding accepts only those two objects, each key once. It refuses a member
/// that is not a plasmid, and never defaults to `Local`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[expect(
    clippy::large_enum_variant,
    reason = "desired-state records reuse the registry form's three fields as plain values, and an attachment holds one source, so boxing them saves nothing that matters"
)]
pub enum AttachmentSource {
    Local,
    Registry {
        registry_id: RegistryId,
        root: ReleaseRef,
        member: ReleaseRef,
    },
}

impl AttachmentSource {
    /// Checks that a registry member is a plasmid named `plugin`, the change's plugin. A local
    /// source always passes. A refusal is `InvalidSource`. This checks no graph membership.
    pub fn validate_member(&self, plugin: &PluginId) -> Result<(), JournalRuleError> {
        match self {
            AttachmentSource::Registry { member, .. }
                if member.key().kind() != ArtifactKind::Plasmid
                    || member.key().name() != plugin.as_str() =>
            {
                Err(JournalRuleError::new(
                    CellJournalFaultKind::InvalidSource,
                    format!(
                        "member {:?} is not the plasmid {:?}",
                        member.key().name(),
                        plugin.as_str()
                    ),
                ))
            }
            _ => Ok(()),
        }
    }
}

const KEYS: &[&str] = &["kind", "registry_id", "root", "member"];

impl<'de> Deserialize<'de> for AttachmentSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(SourceVisitor)
    }
}

struct SourceVisitor;

impl<'de> Visitor<'de> for SourceVisitor {
    type Value = AttachmentSource;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an attachment source object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<AttachmentSource, A::Error> {
        let mut kind: Option<String> = None;
        let mut registry_id: Option<RegistryId> = None;
        let mut root: Option<ReleaseRef> = None;
        let mut member: Option<ReleaseRef> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "kind" if kind.is_none() => kind = Some(map.next_value()?),
                "registry_id" if registry_id.is_none() => registry_id = Some(map.next_value()?),
                "root" if root.is_none() => root = Some(map.next_value()?),
                "member" if member.is_none() => member = Some(map.next_value()?),
                known if KEYS.contains(&known) => {
                    return Err(A::Error::custom(format_args!("duplicate field `{known}`")));
                }
                other => return Err(A::Error::unknown_field(other, KEYS)),
            }
        }
        match kind.as_deref() {
            None => Err(A::Error::missing_field("kind")),
            Some("local") if registry_id.is_none() && root.is_none() && member.is_none() => {
                Ok(AttachmentSource::Local)
            }
            Some("local") => Err(A::Error::custom("a local source has no other field")),
            Some("registry") => {
                let member = member.ok_or_else(|| A::Error::missing_field("member"))?;
                if member.key().kind() != ArtifactKind::Plasmid {
                    return Err(A::Error::custom("a registry member must be a plasmid"));
                }
                Ok(AttachmentSource::Registry {
                    registry_id: registry_id
                        .ok_or_else(|| A::Error::missing_field("registry_id"))?,
                    root: root.ok_or_else(|| A::Error::missing_field("root"))?,
                    member,
                })
            }
            Some(other) => Err(A::Error::unknown_variant(other, &["local", "registry"])),
        }
    }
}
