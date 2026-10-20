use plasmosome_backend::software::{Conflict, Holder, PinRefusal};
use plasmosome_core::{ErrorCode, WireError};
use serde_json::{Value, json};

fn wire(error: &WireError) -> Value {
    let value = serde_json::to_value(error).expect("a wire error serializes");
    let line = serde_json::to_string(error).expect("a wire error serializes");
    let back: WireError = serde_json::from_str(&line).expect("a wire error reads back");
    assert_eq!(&back, error, "{line}");
    value
}

#[test]
fn a_name_another_plasmid_holds_is_code_102_targeting_the_command_and_naming_the_plasmid() {
    let conflict = Conflict {
        target: "/usr/local/bin/gh".into(),
        holder: Holder::Plasmid("github-cli".into()),
    };
    let error = WireError::name_conflict(&conflict);
    assert_eq!(error.code(), ErrorCode::AlreadyExists);
    assert_eq!(
        wire(&error),
        json!({
            "code": 102,
            "message": "\"/usr/local/bin/gh\" is already provided by plasmid \"github-cli\"",
            "target": "/usr/local/bin/gh",
        })
    );
}

#[test]
fn a_name_the_image_resolves_is_code_102_targeting_the_command_and_naming_the_image() {
    let conflict = Conflict {
        target: "/usr/local/bin/git".into(),
        holder: Holder::Image,
    };
    assert_eq!(
        wire(&WireError::name_conflict(&conflict)),
        json!({
            "code": 102,
            "message": "\"/usr/local/bin/git\" would shadow a command the root image resolves on PATH",
            "target": "/usr/local/bin/git",
        })
    );
}

#[test]
fn every_pin_refusal_is_code_108_with_its_detail_and_path() {
    let refusals = [
        PinRefusal::Unreadable {
            path: "lib/data".into(),
        },
        PinRefusal::NotRegular {
            path: "pipe".into(),
        },
        PinRefusal::Link {
            path: "alias".into(),
        },
        PinRefusal::Escapes {
            path: "../outside".into(),
        },
        PinRefusal::DigestMismatch {
            path: "tool".into(),
        },
        PinRefusal::Missing {
            path: "lib/gone".into(),
        },
        PinRefusal::Unexpected {
            path: "extra".into(),
        },
        PinRefusal::CommandNotInTree { name: "gh".into() },
    ];
    for refusal in refusals {
        let error = WireError::pin_refusal(&refusal);
        assert_eq!(error.code(), ErrorCode::ManifestInvalid);
        assert_eq!(
            wire(&error),
            json!({
                "code": 108,
                "message": format!("the software tree cannot be pinned: {refusal}"),
                "detail": refusal.to_string(),
                "path": refusal.path(),
            }),
            "{refusal:?}"
        );
    }
}
