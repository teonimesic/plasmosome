use std::fmt::Debug;

use plasmosome_backend::{GrantId, Handle, UniverseClass};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

const CLASSES: [(UniverseClass, &str); 5] = [
    (UniverseClass::SessionFile, "SessionFile"),
    (UniverseClass::UdsPath, "UdsPath"),
    (UniverseClass::ProxyMap, "ProxyMap"),
    (UniverseClass::BrokerPid, "BrokerPid"),
    (UniverseClass::Mount, "Mount"),
];

fn class_forms<T: DeserializeOwned + Serialize + PartialEq + Debug>(
    expected: T,
    pointer: &str,
    name: &str,
) {
    let mut encoded = serde_json::to_value(&expected).unwrap();
    assert_eq!(encoded.pointer(pointer).unwrap(), &json!(name));
    assert_eq!(
        serde_json::from_value::<T>(encoded.clone()).unwrap(),
        expected
    );
    assert_eq!(
        serde_json::from_str::<T>(&encoded.to_string()).unwrap(),
        expected
    );
    *encoded.pointer_mut(pointer).unwrap() = json!({name: null});
    for error in [
        serde_json::from_value::<T>(encoded.clone()).unwrap_err(),
        serde_json::from_str::<T>(&encoded.to_string()).unwrap_err(),
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

#[test]
fn standalone_classes_accept_strings_and_refuse_objects() {
    for (class, name) in CLASSES {
        class_forms(class, "", name);
    }
}

#[test]
fn handle_classes_accept_strings_and_refuse_objects() {
    let id = GrantId::new();
    for (class, name) in CLASSES {
        class_forms(Handle { class, id }, "/class", name);
    }
}

#[test]
fn class_names_and_non_string_shapes_are_closed() {
    for (_, name) in CLASSES {
        for refused in [json!(name.to_lowercase()), json!(format!("{name} "))] {
            for error in [
                serde_json::from_value::<UniverseClass>(refused.clone()).unwrap_err(),
                serde_json::from_str::<UniverseClass>(&refused.to_string()).unwrap_err(),
            ] {
                assert!(error.is_data());
                assert!(error.to_string().contains("unknown variant"), "{error}");
            }
        }
    }
    for refused in [
        json!(""),
        json!("unknown"),
        json!("mount"),
        json!("session-file"),
        Value::Null,
        json!(true),
        json!(0),
        json!(1.5),
        json!([]),
        json!(["Mount"]),
        json!({}),
        json!({"Mount": {}}),
        json!({"Mount": 0}),
    ] {
        assert!(
            serde_json::from_value::<UniverseClass>(refused.clone()).is_err(),
            "{refused}"
        );
        assert!(
            serde_json::from_str::<UniverseClass>(&refused.to_string()).is_err(),
            "{refused}"
        );
    }
}
