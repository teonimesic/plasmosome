use std::num::NonZeroU64;

use plasmosome_backend::digest::Digest;
use plasmosome_backend::guest_protocol::*;
use plasmosome_backend::paging::{MAX_FRAME_BYTES, ObservationPage};
use plasmosome_backend::universe::GrantId;
use proptest::prelude::*;
use serde_json::{Value, json};

fn split_reply(id: u64, line: &[u8]) -> Result<String, ReplyFault> {
    plasmosome_backend::guest_protocol::split_reply(id, line).map(|raw| raw.get().to_owned())
}

fn grant(n: u8) -> GrantId {
    serde_json::from_value(json!(format!("00112233-4455-4677-8899-aabbccddee{n:02x}"))).unwrap()
}

fn cases(n: u8) -> Vec<(ControlCall, ControlResult, Value)> {
    let boot = BootToken::from_bytes([n; 32]);
    let policy = Digest::parse_hex(&format!("{n:02x}").repeat(32)).unwrap();
    let grant = grant(n);
    let deadline_ms = NonZeroU64::new(17).unwrap();
    let selected = GrantParams { grant, deadline_ms };
    let operation = serde_json::from_value(json!({"WriteSessionFile":{
        "id":grant,"owner":{"cell":"cell-1","plugin":"plugin"},"path":"/session",
        "recipe":{"contents":[3,17],"mode":"read_only","guest_path":"/run/session"}
    }}))
    .unwrap();
    let install = InstallParams {
        operation,
        address: None,
        deadline_ms,
    };
    let remove = RemoveParams {
        inverse: install.operation.removal(),
        owner: install.operation.owner().clone(),
        force: true,
        deadline_ms,
    };
    vec![
        (
            ControlCall::Hello(HelloParams {
                lane: ControlLane::Normal,
            }),
            ControlResult::Hello(HelloResult { boot, policy }),
            json!({"boot":boot,"policy":policy}),
        ),
        (
            ControlCall::Observe(ObserveParams {
                deadline_ms,
                snapshot: 17,
                offset: 3,
            }),
            ControlResult::Observe(ObservationPage {
                snapshot: 17,
                offset: 3,
                total: 5,
                sha256: policy,
                bytes: vec![n, 17],
                complete: true,
            }),
            json!({"snapshot":17,"offset":3,"total":5,"sha256":policy,"bytes":[n,17],"complete":true}),
        ),
        (
            ControlCall::Install(install),
            ControlResult::Installed(InstalledResult {
                boot,
                grant,
                installed: Yes,
            }),
            json!({"boot":boot,"grant":grant,"installed":true}),
        ),
        (
            ControlCall::Activate(selected.clone()),
            ControlResult::Active(ActiveResult {
                boot,
                grant,
                active: Yes,
            }),
            json!({"boot":boot,"grant":grant,"active":true}),
        ),
        (
            ControlCall::Drain(selected),
            ControlResult::Drained(DrainedResult {
                boot,
                grant,
                drained: Yes,
            }),
            json!({"boot":boot,"grant":grant,"drained":true}),
        ),
        (
            ControlCall::Remove(remove),
            ControlResult::Removed(RemovedResult {
                boot,
                grant,
                guest_removed: Yes,
            }),
            json!({"boot":boot,"grant":grant,"guest_removed":true}),
        ),
        (
            ControlCall::Shutdown(ShutdownParams { deadline_ms }),
            ControlResult::Shutdown(ShutdownResult {
                boot,
                shutdown_requested: Yes,
            }),
            json!({"boot":boot,"shutdown_requested":true}),
        ),
    ]
}

fn reply(result: &Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"id":17,"result":result})).unwrap()
}

#[test]
fn yes_is_exactly_true() {
    assert_eq!(serde_json::from_str::<Yes>("true").unwrap(), Yes);
    assert_eq!(serde_json::to_string(&Yes).unwrap(), "true");
    for value in [
        "false",
        "1",
        "null",
        "\"true\"",
        "[]",
        "{}",
        "{\"Yes\":null}",
    ] {
        assert!(serde_json::from_str::<Yes>(value).is_err(), "{value}");
        assert!(serde_json::from_value::<Yes>(serde_json::from_str(value).unwrap()).is_err());
    }
}

#[test]
fn every_result_round_trips_without_replacing_values() {
    for n in [37, 93] {
        for (call, result, wire) in cases(n) {
            for id in [17, u64::MAX] {
                let encoded = encode_control_result(&RequestId::number(id), &result).unwrap();
                assert_eq!(
                    serde_json::from_slice::<Value>(&encoded).unwrap(),
                    json!({"id":id,"result":wire})
                );
                assert!(!encoded.contains(&b'\n'));
                assert_eq!(
                    decode_control_reply(
                        &call,
                        id,
                        Some(&BootToken::from_bytes([n; 32])),
                        &encoded
                    ),
                    Ok(result.clone())
                );
            }
        }
    }
}

#[test]
fn observe_preserves_every_page_value() {
    let (call, _, _) = cases(37).remove(1);
    let sha256 = Digest::parse_hex(&"5d".repeat(32)).unwrap();
    for (snapshot, offset, total, complete) in [(43, 7, 11, false), (61, 9, 11, true)] {
        let result = ControlResult::Observe(ObservationPage {
            snapshot,
            offset,
            total,
            sha256,
            bytes: vec![7, 29],
            complete,
        });
        let encoded = encode_control_result(&RequestId::number(17), &result).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&encoded).unwrap(),
            json!({"id":17,"result":{"snapshot":snapshot,"offset":offset,"total":total,
                "sha256":sha256,"bytes":[7,29],"complete":complete}})
        );
        assert_eq!(decode_control_reply(&call, 17, None, &encoded), Ok(result));
    }
}

#[test]
fn reply_results_are_closed() {
    for (call, _, wire) in cases(37) {
        let refuses = |bytes: Vec<u8>| {
            assert!(
                matches!(
                    decode_control_reply(&call, 17, None, &bytes),
                    Err(ReplyFault::Result { .. })
                ),
                "{:?}: {}",
                call.method(),
                String::from_utf8_lossy(&bytes)
            )
        };
        let fields = wire.as_object().unwrap();
        for key in fields.keys() {
            let mut missing = wire.clone();
            missing.as_object_mut().unwrap().remove(key);
            refuses(reply(&missing));
            let mut null = wire.clone();
            null[key] = Value::Null;
            refuses(reply(&null));
            let repeated = format!(
                "{{\"id\":17,\"result\":{{{},\"{key}\":{}}}}}",
                &wire.to_string()[1..wire.to_string().len() - 1],
                fields[key]
            );
            refuses(repeated.into_bytes());
        }
        let mut extra = wire.clone();
        extra["extra"] = json!(1);
        refuses(reply(&extra));
        for value in [
            Value::Null,
            json!([]),
            json!({"Hello":wire}),
            Value::Array(fields.values().cloned().collect()),
        ] {
            refuses(reply(&value));
        }
        for flag in [
            "installed",
            "active",
            "drained",
            "guest_removed",
            "shutdown_requested",
        ] {
            if fields.contains_key(flag) {
                let mut false_flag = wire.clone();
                false_flag[flag] = json!(false);
                refuses(reply(&false_flag));
            }
        }
    }
}

#[test]
fn result_records_refuse_positional_arrays_directly() {
    macro_rules! check {
        ($ty:ty, $wire:expr) => {{
            let wire = $wire;
            assert!(serde_json::from_value::<$ty>(wire.clone()).is_ok());
            let array = Value::Array(
                [
                    "boot",
                    "grant",
                    "policy",
                    "installed",
                    "active",
                    "drained",
                    "guest_removed",
                    "shutdown_requested",
                ]
                .into_iter()
                .filter_map(|key| wire.get(key).cloned())
                .collect(),
            );
            assert!(serde_json::from_value::<$ty>(array).is_err());
        }};
    }
    let rows = cases(37);
    check!(HelloResult, rows[0].2.clone());
    check!(InstalledResult, rows[2].2.clone());
    check!(ActiveResult, rows[3].2.clone());
    check!(DrainedResult, rows[4].2.clone());
    check!(RemovedResult, rows[5].2.clone());
    check!(ShutdownResult, rows[6].2.clone());
}

#[test]
fn reply_checks_echoed_grant_and_boot() {
    for (call, result, mut wire) in cases(37) {
        if matches!(result, ControlResult::Observe(_)) {
            continue;
        }
        let boot = BootToken::from_bytes([37; 32]);
        let mut other_boots = vec![BootToken::from_bytes([93; 32])];
        for position in [0, 16, 31] {
            let mut bytes = *boot.as_bytes();
            bytes[position] = 93;
            other_boots.push(BootToken::from_bytes(bytes));
        }
        for other in other_boots {
            wire["boot"] = json!(other);
            assert_eq!(
                decode_control_reply(&call, 17, Some(&boot), &reply(&wire)),
                Err(ReplyFault::BootChanged {
                    expected: boot,
                    found: other
                })
            );
            assert!(decode_control_reply(&call, 17, None, &reply(&wire)).is_ok());
        }
        wire["boot"] = json!(boot);
        if let Some(expected) = call.grant() {
            for found in [
                grant(93),
                serde_json::from_value(json!("10112233-4455-4677-8899-aabbccddee25")).unwrap(),
                serde_json::from_value(json!("00112233-4455-4677-8899-aabbccddee26")).unwrap(),
            ] {
                wire["grant"] = json!(found);
                assert_eq!(
                    decode_control_reply(&call, 17, Some(&boot), &reply(&wire)),
                    Err(ReplyFault::GrantMismatch { expected, found })
                );
            }
        }
    }
}

#[test]
fn reply_envelope_rules() {
    assert_eq!(split_reply(17, &[255]), Err(ReplyFault::NotUtf8));
    for text in ["{", "{} {}", "{\"id\":17,\"result\":01}"] {
        assert!(matches!(
            split_reply(17, text.as_bytes()),
            Err(ReplyFault::NotJson { .. })
        ));
    }
    for text in [
        "null",
        "[]",
        "{}",
        "{\"id\":17}",
        "{\"result\":{}}",
        "{\"id\":17,\"result\":{},\"error\":null}",
        "{\"id\":17,\"result\":{},\"x\":0}",
        "{\"id\":17,\"id\":17,\"result\":{}}",
        "{\"id\":17,\"result\":null,\"result\":{}}",
        "{\"id\":17,\"error\":null,\"error\":{}}",
        "{\"id\":17,\"error\":null}",
    ] {
        assert!(
            matches!(
                split_reply(17, text.as_bytes()),
                Err(ReplyFault::Envelope { .. })
            ),
            "{text}"
        );
    }
    assert_eq!(
        split_reply(17, b"{\"id\":17,\"result\":null}")
            .unwrap()
            .as_str(),
        "null"
    );
    for id in [
        "18",
        "\"17\"",
        "17.0",
        "17e0",
        "-1",
        "null",
        "{}",
        "18446744073709551616",
    ] {
        let line = format!("{{\"id\":{id},\"result\":{{}}}}");
        assert_eq!(
            split_reply(17, line.as_bytes()),
            Err(ReplyFault::WrongId {
                expected: 17,
                found: id.into()
            })
        );
    }
    for code in [
        ProtocolCode::ParseError,
        ProtocolCode::InvalidRequest,
        ProtocolCode::MethodNotFound,
        ProtocolCode::InvalidParams,
        ProtocolCode::InternalError,
    ] {
        let error = BridgeError::Protocol {
            code,
            message: "a failure".into(),
        };
        assert_eq!(
            split_reply(17, &encode_failure(&RequestId::number(17), &error)),
            Err(ReplyFault::Refused(error.clone()))
        );
        assert_eq!(
            split_reply(17, &encode_failure(&RequestId::null(), &error)),
            Err(ReplyFault::Unattributed(error.clone()))
        );
        assert!(matches!(
            split_reply(17, &encode_failure(&RequestId::number(18), &error)),
            Err(ReplyFault::WrongId { .. })
        ));
    }
    let error = BridgeError::Refused {
        message: "denied".into(),
        detail: RefusalDetail::Rejected {
            reason: "busy".into(),
        },
    };
    assert_eq!(
        split_reply(17, &encode_failure(&RequestId::number(17), &error)),
        Err(ReplyFault::Refused(error.clone()))
    );
    assert_eq!(
        split_reply(17, &encode_failure(&RequestId::null(), &error)),
        Err(ReplyFault::Unattributed(error))
    );
}

#[test]
fn error_objects_are_exactly_protocol_or_refusal() {
    let protocol = json!({"code":-32602,"message":"bad"});
    let refused = json!({"code":105,"message":"no","from":"guest_request","to":"refused","detail":{"kind":"rejected","reason":"busy"}});
    let rejects = |error: &Value| {
        assert!(
            matches!(
                split_reply(
                    17,
                    &serde_json::to_vec(&json!({"id":17,"error":error})).unwrap()
                ),
                Err(ReplyFault::Envelope { .. })
            ),
            "{error}"
        )
    };
    for valid in [&protocol, &refused] {
        for key in valid.as_object().unwrap().keys() {
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(key);
            rejects(&missing);
            let mut null = valid.clone();
            null[key] = Value::Null;
            rejects(&null);
            let repeated = format!(
                "{{\"id\":17,\"error\":{{{},\"{key}\":{}}}}}",
                &valid.to_string()[1..valid.to_string().len() - 1],
                valid[key]
            );
            assert!(matches!(
                split_reply(17, repeated.as_bytes()),
                Err(ReplyFault::Envelope { .. })
            ));
        }
        let mut extra = valid.clone();
        extra["extra"] = json!(1);
        rejects(&extra);
    }
    for key in ["from", "to", "detail"] {
        for value in [Value::Null, refused[key].clone()] {
            let mut extra = protocol.clone();
            extra[key] = value;
            rejects(&extra);
        }
    }
    for (key, value) in [
        ("code", json!(106)),
        ("code", json!(105.0)),
        ("from", json!("guest")),
        ("to", json!("failed")),
        ("detail", json!([])),
        ("detail", json!({"kind":"other","reason":"r"})),
        ("detail", json!({"kind":"rejected"})),
        ("detail", json!({"kind":"rejected","reason":"r","extra":0})),
    ] {
        let mut bad = refused.clone();
        bad[key] = value;
        rejects(&bad);
    }
    rejects(&json!([105,"no","guest_request","refused",{"kind":"rejected","reason":"r"}]));
    rejects(&json!({"code":0,"message":"bad"}));
}

#[test]
fn reply_faults_cut_guest_text_on_a_character_boundary() {
    let long = format!("{}😀tail", "x".repeat(1023));
    let prefix = "x".repeat(1023);
    let error = BridgeError::Refused {
        message: long.clone(),
        detail: RefusalDetail::Rejected {
            reason: long.clone(),
        },
    };
    let bounded = BridgeError::Refused {
        message: prefix.clone(),
        detail: RefusalDetail::Rejected {
            reason: prefix.clone(),
        },
    };
    assert_eq!(
        split_reply(17, &encode_failure(&RequestId::number(17), &error)),
        Err(ReplyFault::Refused(bounded))
    );
    let error = BridgeError::Protocol {
        code: ProtocolCode::InternalError,
        message: long.clone(),
    };
    assert_eq!(
        split_reply(17, &encode_failure(&RequestId::null(), &error)),
        Err(ReplyFault::Unattributed(BridgeError::Protocol {
            code: ProtocolCode::InternalError,
            message: prefix
        }))
    );
    let id = format!("\"{}😀tail\"", "x".repeat(1022));
    assert_eq!(
        split_reply(17, format!("{{\"id\":{id},\"result\":null}}").as_bytes()),
        Err(ReplyFault::WrongId {
            expected: 17,
            found: format!("\"{}", "x".repeat(1022))
        })
    );
    let bad = format!("{{\"id\":17,\"result\":null,{}:0}}", json!(long));
    let Err(ReplyFault::Envelope { detail }) = split_reply(17, bad.as_bytes()) else {
        panic!("envelope fault")
    };
    assert_eq!(detail.len(), 1024);
    let (call, _, mut wire) = cases(37).remove(0);
    wire[long] = json!(0);
    let Err(ReplyFault::Result { detail }) = decode_control_reply(&call, 17, None, &reply(&wire))
    else {
        panic!("result fault")
    };
    assert_eq!(detail.len(), 1024);
}

#[test]
fn reply_depth_is_bounded_and_the_reader_owns_the_frame_bound() {
    for containers in [127, 128, 129] {
        let line = format!(
            "{{\"id\":17,\"result\":{}0{}}}",
            "[".repeat(containers - 1),
            "]".repeat(containers - 1)
        );
        let decoded = split_reply(17, line.as_bytes());
        if containers <= 128 {
            assert_eq!(
                decoded,
                Ok(format!(
                    "{}0{}",
                    "[".repeat(containers - 1),
                    "]".repeat(containers - 1)
                )),
                "{containers} containers: {line}"
            );
        } else {
            assert_eq!(
                decoded,
                Err(ReplyFault::NotJson {
                    detail: "recursion limit exceeded".into()
                }),
                "{containers} containers: {line}"
            );
        }
    }
    let text = "[{}] \\\"".repeat(200);
    assert!(split_reply(17, &reply(&json!(text))).is_ok());
    let line = reply(&json!("x".repeat(MAX_FRAME_BYTES)));
    assert!(line.len() > MAX_FRAME_BYTES);
    assert!(split_reply(17, &line).is_ok());
    let (call, _, _) = cases(37).remove(1);
    let wire = json!({"snapshot":0,"offset":0,"total":0,"sha256":"25".repeat(32),"bytes":[],"complete":false});
    let ControlResult::Observe(page) =
        decode_control_reply(&call, 17, None, &reply(&wire)).unwrap()
    else {
        panic!("page")
    };
    assert!(page.check().is_err());
}

proptest! {
    #[test]
    fn decoders_never_panic(data in prop::collection::vec(any::<u8>(), 0..4096)) {
        for (call,_,_) in cases(37) {
            let _ = decode_control_reply(&call,17,Some(&BootToken::from_bytes([37;32])),&data);
        }
    }
}
