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
