use std::fmt;

use serde::de::{DeserializeSeed, EnumAccess, VariantAccess, Visitor};
use serde::{Deserialize, Deserializer, forward_to_deserialize_any};

/// A deserializer that reads a struct, and each struct variant of an enum, only from a map.
///
/// A derived decoder of a struct with named fields also accepts a sequence and fills the fields
/// by position, and `deny_unknown_fields` does not stop it: `["/a", "/b"]` decodes into two path
/// fields without naming either, so two fields of one type can swap unnoticed. A derived shape
/// decoded through `ObjectOnly::new(deserializer)` refuses a sequence as an invalid type. Only
/// the value it wraps is affected: every field still decodes through its own type, so each
/// nested record must refuse a sequence itself. Wrap only a struct or an enum.
pub struct ObjectOnly<D>(D);

impl<D> ObjectOnly<D> {
    pub fn new(deserializer: D) -> ObjectOnly<D> {
        ObjectOnly(deserializer)
    }
}

impl<'de, D: Deserializer<'de>> Deserializer<'de> for ObjectOnly<D> {
    type Error = D::Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.0.deserialize_map(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_enum(name, variants, Variants(visitor))
    }

    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
        option unit unit_struct newtype_struct seq tuple tuple_struct map struct identifier
        ignored_any
    }
}

struct Variants<V>(V);

impl<'de, V: Visitor<'de>> Visitor<'de> for Variants<V> {
    type Value = V::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(formatter)
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<V::Value, A::Error> {
        self.0.visit_enum(Tag(data))
    }
}

struct Tag<A>(A);

impl<'de, A: EnumAccess<'de>> EnumAccess<'de> for Tag<A> {
    type Error = A::Error;
    type Variant = Content<A::Variant>;

    fn variant_seed<S: DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<(S::Value, Content<A::Variant>), A::Error> {
        let (tag, content) = self.0.variant_seed(seed)?;
        Ok((tag, Content(content)))
    }
}

struct Content<A>(A);

impl<'de, A: VariantAccess<'de>> VariantAccess<'de> for Content<A> {
    type Error = A::Error;

    fn unit_variant(self) -> Result<(), A::Error> {
        self.0.unit_variant()
    }

    fn newtype_variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<S::Value, A::Error> {
        self.0.newtype_variant_seed(seed)
    }

    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, A::Error> {
        self.0.tuple_variant(len, visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, A::Error> {
        self.0.newtype_variant_seed(Fields(visitor))
    }
}

struct Fields<V>(V);

impl<'de, V: Visitor<'de>> DeserializeSeed<'de> for Fields<V> {
    type Value = V::Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<V::Value, D::Error> {
        deserializer.deserialize_map(self.0)
    }
}

pub(crate) fn object<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(ObjectOnly::new(deserializer))
}

macro_rules! object_serde {
    ($($record:ident through $shape:ident),* $(,)?) => {$(
        impl serde::Serialize for $record {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                $shape::serialize(self, serializer)
            }
        }

        impl<'de> serde::Deserialize<'de> for $record {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                $shape::deserialize($crate::wire::ObjectOnly::new(deserializer))
            }
        }
    )*};
}

macro_rules! validated_object_serde {
    ($($record:ident through $shape:ident),* $(,)?) => {$(
        impl serde::Serialize for $record {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.validate()
                    .map_err(<S::Error as serde::ser::Error>::custom)?;
                $shape::serialize(self, serializer)
            }
        }

        impl<'de> serde::Deserialize<'de> for $record {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let record = $shape::deserialize($crate::wire::ObjectOnly::new(deserializer))?;
                record
                    .validate()
                    .map_err(<D::Error as serde::de::Error>::custom)?;
                Ok(record)
            }
        }
    )*};
}

pub(crate) use {object_serde, validated_object_serde};

#[cfg(test)]
mod tests {
    use std::fmt::Debug;
    use std::time::Duration;

    use serde::de::DeserializeOwned;
    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};

    use super::ObjectOnly;
    use crate::{
        BrokerLaunch, Capability, CellId, CellOwner, Diff, DrainSpec, FileAccess, Grant, GrantId,
        GrantKind, Handle, LedgerEntry, MountRecipe, OsObject, OsState, PluginId, ProxyRecipe,
        ProxyTransport, ResidueReport, SessionFileRecipe, UdsRecipe, UniverseClass, UniverseOp,
        UniverseRemoval,
    };

    fn owner() -> CellOwner {
        CellOwner {
            cell: CellId::from("cell-1"),
            plugin: PluginId::from("github-pr"),
        }
    }

    fn mount() -> Capability {
        Capability::Mount {
            source: "/srv/repo".to_string(),
            target: "/workspace".to_string(),
        }
    }

    fn object() -> OsObject {
        OsObject {
            id: GrantId::new(),
            owner: owner(),
            capability: mount(),
        }
    }

    fn operations() -> Vec<(UniverseOp, &'static str, &'static [&'static str])> {
        let id = GrantId::new;
        vec![
            (
                UniverseOp::WriteSessionFile {
                    id: id(),
                    path: "/skills/pr.md".to_string(),
                    owner: owner(),
                },
                "/WriteSessionFile",
                &["id", "path", "owner"],
            ),
            (
                UniverseOp::BindUds {
                    id: id(),
                    path: "/run/plasmosome/egressd.uds".to_string(),
                    owner: owner(),
                },
                "/BindUds",
                &["id", "path", "owner"],
            ),
            (
                UniverseOp::SetProxyMap {
                    id: id(),
                    host: "api.github.com".to_string(),
                    route: "splice".to_string(),
                    owner: owner(),
                },
                "/SetProxyMap",
                &["id", "host", "route", "owner"],
            ),
            (
                UniverseOp::SpawnBroker {
                    id: id(),
                    pid: 4242,
                    name: "egressd".to_string(),
                    owner: owner(),
                },
                "/SpawnBroker",
                &["id", "pid", "name", "owner"],
            ),
            (
                UniverseOp::AddMount {
                    id: id(),
                    source: "/srv/repo".to_string(),
                    target: "/workspace".to_string(),
                    owner: owner(),
                },
                "/AddMount",
                &["id", "source", "target", "owner"],
            ),
        ]
    }

    fn positional(encoded: &Value, at: &str, fields: &[&str]) -> Value {
        let mut array = encoded.clone();
        let record = array
            .pointer_mut(at)
            .expect("the record sits at its pointer");
        assert_eq!(
            record.as_object().map(serde_json::Map::len),
            Some(fields.len()),
            "{record} must hold exactly the fields {fields:?}"
        );
        *record = Value::Array(fields.iter().map(|field| record[*field].clone()).collect());
        array
    }

    fn sequence_accepted<T>(record: &str, value: &T, at: &str, fields: &[&str]) -> Vec<String>
    where
        T: Serialize + DeserializeOwned + PartialEq + Debug,
    {
        let encoded = serde_json::to_value(value).expect("a valid record encodes");
        assert_eq!(
            &serde_json::from_value::<T>(encoded.clone()).expect("its object decodes"),
            value
        );
        let array = positional(&encoded, at, fields);
        let mut accepted = Vec::new();
        let through_value = serde_json::from_value::<T>(array.clone()).map(drop);
        let through_text = serde_json::from_str::<T>(&array.to_string()).map(drop);
        for (path, outcome) in [("value", through_value), ("text", through_text)] {
            match outcome {
                Err(error)
                    if error.is_data() && error.to_string().contains("invalid type: sequence") => {}
                other => accepted.push(format!("{record} at {at:?} through {path}: {other:?}")),
            }
        }
        accepted
    }

    #[test]
    fn every_backend_record_decodes_from_an_object_and_refuses_a_positional_array() {
        let entry = LedgerEntry {
            handle: Handle {
                class: UniverseClass::Mount,
                id: GrantId::new(),
            },
            owner: owner(),
            capability: mount(),
            kind: GrantKind::Hot,
        };
        let mut state = OsState::new();
        state.insert(object()).unwrap();
        let mut accepted = Vec::new();
        accepted.extend(sequence_accepted(
            "Handle",
            &entry.handle,
            "",
            &["class", "id"],
        ));
        accepted.extend(sequence_accepted(
            "LedgerEntry",
            &entry,
            "",
            &["handle", "owner", "capability", "kind"],
        ));
        accepted.extend(sequence_accepted(
            "Grant",
            &Grant {
                owner: owner(),
                capability: mount(),
                kind: GrantKind::Hot,
            },
            "",
            &["owner", "capability", "kind"],
        ));
        let drain = DrainSpec::graceful(Duration::from_millis(5));
        accepted.extend(sequence_accepted(
            "DrainSpec",
            &drain,
            "",
            &["deadline", "policy"],
        ));
        accepted.extend(sequence_accepted(
            "DrainSpec.deadline",
            &drain,
            "/deadline",
            &["secs", "nanos"],
        ));
        accepted.extend(sequence_accepted(
            "CellOwner",
            &owner(),
            "",
            &["cell", "plugin"],
        ));
        for (capability, at, fields) in [
            (
                Capability::SessionFile {
                    path: "/skills/pr.md".to_string(),
                },
                "/SessionFile",
                &["path"][..],
            ),
            (
                Capability::UdsSocket {
                    path: "/run/plasmosome/egressd.uds".to_string(),
                },
                "/UdsSocket",
                &["path"][..],
            ),
            (
                Capability::ProxyMap {
                    host: "api.github.com".to_string(),
                    route: "splice".to_string(),
                },
                "/ProxyMap",
                &["host", "route"][..],
            ),
            (
                Capability::Broker {
                    pid: 4242,
                    name: "egressd".to_string(),
                },
                "/Broker",
                &["pid", "name"][..],
            ),
            (mount(), "/Mount", &["source", "target"][..]),
        ] {
            accepted.extend(sequence_accepted("Capability", &capability, at, fields));
        }
        for (operation, at, fields) in operations() {
            accepted.extend(sequence_accepted("UniverseOp", &operation, at, fields));
        }
        accepted.extend(sequence_accepted(
            "OsObject",
            &object(),
            "",
            &["id", "owner", "capability"],
        ));
        accepted.extend(sequence_accepted("OsState", &state, "", &["objects"]));
        accepted.extend(sequence_accepted(
            "UniverseRemoval",
            &UniverseRemoval {
                id: GrantId::new(),
                capability: mount(),
            },
            "",
            &["id", "capability"],
        ));
        let diff = Diff::between(&OsState::new(), &state);
        accepted.extend(sequence_accepted("Diff", &diff, "", &["added", "removed"]));
        accepted.extend(sequence_accepted(
            "ResidueReport",
            &ResidueReport::from_diff(diff, vec!["an operator assertion".to_string()]),
            "/Residue",
            &["leaked", "lost", "assertions"],
        ));
        accepted.extend(sequence_accepted(
            "SessionFileRecipe",
            &SessionFileRecipe {
                contents: b"seed".to_vec(),
                mode: FileAccess::ReadOnly,
                guest_path: "/etc/plasmosome/seed".to_string(),
            },
            "",
            &["contents", "mode", "guest_path"],
        ));
        accepted.extend(sequence_accepted(
            "UdsRecipe",
            &UdsRecipe {
                upstream: "/run/plasmosome/upstream.sock".to_string(),
                guest_path: "/run/agent.sock".to_string(),
            },
            "",
            &["upstream", "guest_path"],
        ));
        accepted.extend(sequence_accepted(
            "ProxyRecipe",
            &ProxyRecipe {
                transport: ProxyTransport::Tcp,
                destination: "api.github.com".to_string(),
                port: 443,
                allow_private: false,
            },
            "",
            &["transport", "destination", "port", "allow_private"],
        ));
        accepted.extend(sequence_accepted(
            "MountRecipe",
            &MountRecipe {
                access: FileAccess::ReadWrite,
            },
            "",
            &["access"],
        ));
        accepted.extend(sequence_accepted(
            "BrokerLaunch",
            &BrokerLaunch {
                command: vec!["/usr/libexec/egressd".to_string()],
                control_socket: "/run/plasmosome/egressd.control".to_string(),
                data_socket: "/run/plasmosome/egressd.data".to_string(),
            },
            "",
            &["command", "control_socket", "data_socket"],
        ));
        assert!(
            accepted.is_empty(),
            "these positional arrays decoded:\n{}",
            accepted.join("\n")
        );
    }

    #[test]
    fn a_launch_array_cannot_swap_its_two_endpoints() {
        let swapped = json!([
            ["/usr/libexec/egressd"],
            "/run/plasmosome/egressd.data",
            "/run/plasmosome/egressd.control"
        ]);
        let error = serde_json::from_value::<BrokerLaunch>(swapped).unwrap_err();
        assert!(
            error.to_string().contains("invalid type: sequence"),
            "{error}"
        );
    }

    #[test]
    fn unit_newtype_and_tuple_variants_decode_through_object_only_unchanged() {
        #[derive(Clone, Debug, PartialEq, Deserialize)]
        enum Shape {
            Unit,
            Newtype(u8),
            Tuple(u8, u8),
        }
        let cases = [
            (json!("Unit"), Shape::Unit),
            (json!({ "Newtype": 1 }), Shape::Newtype(1)),
            (json!({ "Tuple": [1, 2] }), Shape::Tuple(1, 2)),
        ];
        for (value, expected) in cases {
            let text = value.to_string();
            let mut reader = serde_json::Deserializer::from_str(&text);
            let through_text = Shape::deserialize(ObjectOnly::new(&mut reader));
            let through_value = Shape::deserialize(ObjectOnly::new(value));
            assert_eq!(
                through_text.map_err(|error| error.to_string()),
                Ok(expected.clone()),
                "{text}"
            );
            assert_eq!(
                through_value.map_err(|error| error.to_string()),
                Ok(expected),
                "{text}"
            );
        }
    }
}
