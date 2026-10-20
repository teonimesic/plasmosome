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
fn a_name_two_commands_of_one_tree_share_is_code_102_targeting_the_later_command() {
    let conflict = Conflict {
        target: "/usr/local/bin/foo".into(),
        holder: Holder::Command("Foo".into()),
    };
    assert_eq!(
        wire(&WireError::name_conflict(&conflict)),
        json!({
            "code": 102,
            "message": "\"/usr/local/bin/foo\" is the same name as command \"Foo\" of the same tree on a volume that ignores case",
            "target": "/usr/local/bin/foo",
        })
    );
}

#[test]
fn every_pin_refusal_is_code_108_naming_the_plasmid_with_its_tree_path() {
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
        PinRefusal::InvalidPath {
            path: "../outside".into(),
        },
        PinRefusal::Collides {
            path: "Tool".into(),
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
        let error = WireError::pin_refusal("github-cli", &refusal);
        assert_eq!(error.code(), ErrorCode::ManifestInvalid);
        let detail = format!("plasmid github-cli: {refusal}");
        assert_eq!(
            wire(&error),
            json!({
                "code": 108,
                "message": format!("the software tree cannot be pinned: {detail}"),
                "detail": detail,
                "path": refusal.path(),
            }),
            "{refusal:?}"
        );
    }
}

#[test]
fn a_pin_refusal_quotes_a_plasmid_id_as_a_manifest_refusal_does() {
    let refusal = PinRefusal::DigestMismatch {
        path: "tool".into(),
    };
    let value = wire(&WireError::pin_refusal("my tools", &refusal));
    assert_eq!(
        value["detail"],
        json!("plasmid \"my tools\": \"tool\" does not match the digest the package lists for it"),
        "{value}"
    );
}
