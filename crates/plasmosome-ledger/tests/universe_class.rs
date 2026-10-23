use plasmosome_backend::{GrantId, Handle, UniverseClass};
use plasmosome_ledger::{Effect, InverseVia, Ledger, LogRecord};
use serde_json::json;

fn effects() -> impl Iterator<Item = Effect> {
    [
        UniverseClass::SessionFile,
        UniverseClass::UdsPath,
        UniverseClass::ProxyMap,
        UniverseClass::BrokerPid,
        UniverseClass::Mount,
    ]
    .into_iter()
    .map(|class| {
        Effect::exact(
            "holding",
            InverseVia::Backend(Handle {
                class,
                id: GrantId::new(),
            }),
        )
    })
}

const CLASS: &str = "/reversibility/Exact/via/Backend/class";

#[test]
fn effect_classes_accept_strings_and_refuse_objects() {
    for effect in effects() {
        let mut encoded = serde_json::to_value(&effect).unwrap();
        assert_eq!(
            serde_json::from_value::<Effect>(encoded.clone()).unwrap(),
            effect
        );
        assert_eq!(
            serde_json::from_str::<Effect>(&encoded.to_string()).unwrap(),
            effect
        );
        let name = encoded
            .pointer(CLASS)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        *encoded.pointer_mut(CLASS).unwrap() = json!({name: null});
        for error in [
            serde_json::from_value::<Effect>(encoded.clone()).unwrap_err(),
            serde_json::from_str::<Effect>(&encoded.to_string()).unwrap_err(),
        ] {
            assert!(error.is_data(), "{encoded}: {error}");
            assert!(
                error
                    .to_string()
                    .contains("expected a UniverseClass string"),
                "{error}"
            );
        }
    }
}

#[test]
fn log_reader_classes_accept_strings_and_refuse_objects_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.ndjson");
    for effect in effects() {
        let record = LogRecord {
            format: 3,
            plugin: "plugin".into(),
            effect: effect.clone(),
        };
        let mut encoded = serde_json::to_value(record).unwrap();
        let expected = [effect];
        let pointer = format!("/effect{CLASS}");
        let name = encoded
            .pointer(&pointer)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        for ending in ["\n", ""] {
            let valid = format!("{encoded}{ending}");
            std::fs::write(&path, &valid).unwrap();
            assert_eq!(Ledger::open_file(&path).unwrap().effects(), &expected);
            assert_eq!(std::fs::read(&path).unwrap(), valid.as_bytes());
            let mut refused = encoded.clone();
            *refused.pointer_mut(&pointer).unwrap() = json!({(name.as_str()): null});
            let bytes = format!("{refused}{ending}");
            std::fs::write(&path, &bytes).unwrap();
            let error = Ledger::open_file(&path).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("line 1"), "{error}");
            assert!(
                error
                    .to_string()
                    .contains("expected a UniverseClass string"),
                "{error}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes.as_bytes());
        }
        *encoded.pointer_mut(&pointer).unwrap() = json!({name: null});
        for error in [
            serde_json::from_value::<LogRecord>(encoded.clone()).unwrap_err(),
            serde_json::from_str::<LogRecord>(&encoded.to_string()).unwrap_err(),
        ] {
            assert!(error.is_data(), "{error}");
        }
    }
}
