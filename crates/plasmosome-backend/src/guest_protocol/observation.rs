use std::collections::BTreeSet;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::BootToken;
use crate::GrantId;
use crate::digest::Digest;
use crate::wire::{ObjectOnly, object_serde};

/// The guest's complete structural account. All fields are required on the wire.
/// Decoding validates it; callers changing public fields must call validate before use.
/// Host association, policy and authority checks belong to the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct GuestObservation {
    pub boot: BootToken,
    pub policy: Digest,
    pub fs_mounts: Vec<GuestProjection>,
    pub fs_handles: Vec<GuestProjection>,
    pub socket_listeners: Vec<GuestProjection>,
    pub socket_streams: Vec<SocketStreamProjection>,
    pub network: GuestNetworkObservation,
    pub software: SoftwareObservation,
}

impl Serialize for GuestObservation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Self::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for GuestObservation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let observation = Self::deserialize(ObjectOnly::new(deserializer))?;
        observation.validate().map_err(D::Error::custom)?;
        Ok(observation)
    }
}

/// The six required network inventories. An empty array reports observed absence.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct GuestNetworkObservation {
    pub dns_listeners: Vec<GuestProjection>,
    pub tuns: Vec<GuestProjection>,
    pub routes: Vec<GuestProjection>,
    pub proxy_rules: Vec<GuestProjection>,
    pub tcp_flows: Vec<GuestProjection>,
    pub udp_flows: Vec<GuestProjection>,
}

/// One physical object, identified within its kind and boot, with every actual binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct GuestProjection {
    pub id: String,
    pub bindings: Vec<GuestBinding>,
}

/// The grant and inspected admission of one object's binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct GuestBinding {
    pub grant: GrantId,
    pub admission: GuestAdmission,
}

/// Exactly the lowercase admission name in JSON; object forms refuse.
/// Revoked Mount bindings remain inventoried while workload references remain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GuestAdmission {
    Staged,
    Active,
    Draining,
    Closed,
    Revoked,
}

impl<'de> Deserialize<'de> for GuestAdmission {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match String::deserialize(deserializer)?.as_str() {
            "staged" => Ok(Self::Staged),
            "active" => Ok(Self::Active),
            "draining" => Ok(Self::Draining),
            "closed" => Ok(Self::Closed),
            "revoked" => Ok(Self::Revoked),
            other => Err(D::Error::unknown_variant(
                other,
                &["staged", "active", "draining", "closed", "revoked"],
            )),
        }
    }
}

/// An accepted data stream and its still-present original guest endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct SocketStreamProjection {
    pub id: String,
    pub grant: GrantId,
    pub ends: Vec<SocketStreamEnd>,
}

/// One still-present endpoint; its identity is unique across all streams.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct SocketStreamEnd {
    pub side: StreamSide,
    pub id: String,
}

/// Exactly "accepted" or "relay" in JSON; object forms refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamSide {
    Accepted,
    Relay,
}

impl<'de> Deserialize<'de> for StreamSide {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match String::deserialize(deserializer)?.as_str() {
            "accepted" => Ok(Self::Accepted),
            "relay" => Ok(Self::Relay),
            other => Err(D::Error::unknown_variant(other, &["accepted", "relay"])),
        }
    }
}

/// Software trees and informational references, both required even when empty.
/// Trees accept only [] while spec 027 reserves their journal record.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct SoftwareObservation {
    pub trees: Vec<SoftwareTree>,
    pub held: Vec<TreeReference>,
}

/// No tree record exists until spec 027's Q1 is answered. Every wire value refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoftwareTree {}

impl Serialize for SoftwareTree {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        match *self {}
    }
}

impl<'de> Deserialize<'de> for SoftwareTree {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(D::Error::custom(
            "spec 027 reserves SoftwareTree until Q1; trees must be empty",
        ))
    }
}

/// One scanned reference into detached software. exec_id is required, and may be null.
/// These rows are informational and never grant bindings or residue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct TreeReference {
    pub path: String,
    pub pid: u64,
    pub comm: String,
    #[serde(deserialize_with = "required_exec_id")]
    pub exec_id: Option<String>,
}

fn required_exec_id<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::deserialize(deserializer)
}

object_serde!(
    GuestNetworkObservation through GuestNetworkObservation,
    GuestProjection through GuestProjection,
    GuestBinding through GuestBinding,
    SocketStreamProjection through SocketStreamProjection,
    SocketStreamEnd through SocketStreamEnd,
    SoftwareObservation through SoftwareObservation,
    TreeReference through TreeReference,
);

/// The containing inventory fixes an object's kind; IDs may repeat across kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectionKind {
    FsMounts,
    FsHandles,
    SocketListeners,
    SocketStreams,
    DnsListeners,
    Tuns,
    Routes,
    ProxyRules,
    TcpFlows,
    UdpFlows,
}

/// A borrowed projection, preserving a stream's different binding representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionRef<'a> {
    Object(&'a GuestProjection),
    Stream(&'a SocketStreamProjection),
}

impl ProjectionRef<'_> {
    /// The object's original identity, without interpreting it as a capability address.
    pub fn id(&self) -> &str {
        match self {
            Self::Object(object) => &object.id,
            Self::Stream(stream) => &stream.id,
        }
    }
}

/// The first structural fault. Host-dependent checks are outside this validator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestObservationFault {
    EmptyId {
        kind: ProjectionKind,
    },
    EmptyBindings {
        kind: ProjectionKind,
        id: String,
    },
    DuplicateObject {
        kind: ProjectionKind,
        id: String,
    },
    DuplicateBinding {
        kind: ProjectionKind,
        id: String,
        grant: GrantId,
    },
    StreamEndCount {
        id: String,
        count: usize,
    },
    DuplicateStreamSide {
        id: String,
        side: StreamSide,
    },
    EmptyStreamEndId {
        id: String,
    },
    DuplicateStreamEnd {
        end: String,
    },
}

impl fmt::Display for GuestObservationFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid guest observation: {self:?}")
    }
}

impl std::error::Error for GuestObservationFault {}

impl GuestObservation {
    /// An account with every required inventory present and empty.
    pub fn empty(boot: BootToken, policy: Digest) -> Self {
        Self {
            boot,
            policy,
            fs_mounts: Vec::new(),
            fs_handles: Vec::new(),
            socket_listeners: Vec::new(),
            socket_streams: Vec::new(),
            network: GuestNetworkObservation::default(),
            software: SoftwareObservation::default(),
        }
    }

    /// Every projection in all ten kinds. Ordinary inventories follow field order,
    /// then streams follow their array order. Software rows are not projections.
    pub fn projections(&self) -> impl Iterator<Item = (ProjectionKind, ProjectionRef<'_>)> {
        use ProjectionKind::*;
        [
            (FsMounts, &self.fs_mounts),
            (FsHandles, &self.fs_handles),
            (SocketListeners, &self.socket_listeners),
            (DnsListeners, &self.network.dns_listeners),
            (Tuns, &self.network.tuns),
            (Routes, &self.network.routes),
            (ProxyRules, &self.network.proxy_rules),
            (TcpFlows, &self.network.tcp_flows),
            (UdpFlows, &self.network.udp_flows),
        ]
        .into_iter()
        .flat_map(|(kind, objects)| {
            objects
                .iter()
                .map(move |p| (kind, ProjectionRef::Object(p)))
        })
        .chain(
            self.socket_streams
                .iter()
                .map(|p| (SocketStreams, ProjectionRef::Stream(p))),
        )
    }

    /// Every (kind, projection ID, grant) binding, including each stream's grant.
    /// Repeated grants on different objects remain separate rows. Software is excluded.
    pub fn grants(&self) -> impl Iterator<Item = (ProjectionKind, &str, GrantId)> {
        self.projections().flat_map(|(kind, projection)| {
            let (id, bindings, stream) = match projection {
                ProjectionRef::Object(p) => (p.id.as_str(), p.bindings.as_slice(), None),
                ProjectionRef::Stream(p) => (p.id.as_str(), &[][..], Some(p.grant)),
            };
            bindings
                .iter()
                .map(move |b| (kind, id, b.grant))
                .chain(stream.into_iter().map(move |grant| (kind, id, grant)))
        })
    }

    /// Refuses empty IDs or bindings, repeated objects within a kind, repeated grants
    /// on an object, and invalid stream ends. A surviving single stream end is valid.
    /// Does not check boot, policy, host authority or whether a grant belongs to this cell.
    pub fn validate(&self) -> Result<(), GuestObservationFault> {
        use GuestObservationFault::*;
        let mut objects = BTreeSet::new();
        let mut ends = BTreeSet::new();
        for (kind, projection) in self.projections() {
            let id = projection.id();
            if id.is_empty() {
                return Err(EmptyId { kind });
            }
            if !objects.insert((kind, id.to_owned())) {
                return Err(DuplicateObject {
                    kind,
                    id: id.to_owned(),
                });
            }
            match projection {
                ProjectionRef::Object(object) => {
                    if object.bindings.is_empty() {
                        return Err(EmptyBindings {
                            kind,
                            id: id.to_owned(),
                        });
                    }
                    let mut grants = BTreeSet::new();
                    for binding in &object.bindings {
                        if !grants.insert(binding.grant) {
                            return Err(DuplicateBinding {
                                kind,
                                id: id.to_owned(),
                                grant: binding.grant,
                            });
                        }
                    }
                }
                ProjectionRef::Stream(stream) => {
                    if stream.ends.is_empty() || stream.ends.len() > 2 {
                        return Err(StreamEndCount {
                            id: id.to_owned(),
                            count: stream.ends.len(),
                        });
                    }
                    let mut sides = BTreeSet::new();
                    for end in &stream.ends {
                        if !sides.insert(end.side) {
                            return Err(DuplicateStreamSide {
                                id: id.to_owned(),
                                side: end.side,
                            });
                        }
                        if end.id.is_empty() {
                            return Err(EmptyStreamEndId { id: id.to_owned() });
                        }
                        if !ends.insert(&end.id) {
                            return Err(DuplicateStreamEnd {
                                end: end.id.clone(),
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
