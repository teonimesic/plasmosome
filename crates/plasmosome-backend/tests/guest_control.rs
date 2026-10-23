use std::{fmt::Debug, net::Ipv4Addr, num::NonZeroU64};

use plasmosome_backend::guest_protocol::*;
use plasmosome_backend::universe::{GrantId, UniverseOp};
use proptest::prelude::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const NORMAL: LaneState = LaneState::Assigned(ControlLane::Normal);
const WITHDRAWAL: LaneState = LaneState::Assigned(ControlLane::Withdrawal);
const GRANT: &str = "00112233-4455-4677-8899-aabbccddeeff";

fn operations() -> Vec<UniverseOp> {
    let bodies = [
        (
            "WriteSessionFile",
            json!({"path":"/session", "recipe":{"contents":[0,255],"mode":"read_only","guest_path":"/run/session"}}),
        ),
        (
            "BindUds",
            json!({"path":"/socket", "recipe":{"upstream":"/upstream","guest_path":"/guest"}}),
        ),
        (
            "SetProxyMap",
            json!({"host":"example.test","route":"route", "recipe":{"transport":"tcp","destination":"example.test","port":443,"allow_private":false}}),
        ),
        (
            "SpawnBroker",
            json!({"name":"broker", "launch":{"command":["/bin/broker"],"control_socket":"/control","data_socket":"/data"}}),
        ),
        (
            "AddMount",
            json!({"source":"/source","target":"/target", "recipe":{"access":"read_only"}}),
        ),
    ];
    bodies
        .into_iter()
        .map(|(name, mut body)| {
            body["id"] = json!(GRANT);
            body["owner"] = json!({"cell":"cell-1","plugin":"plugin"});
            serde_json::from_value(json!({name:body})).expect("complete grant fixture")
        })
        .collect()
}

fn calls() -> Vec<ControlCall> {
    let op = operations().remove(0);
    let deadline_ms = NonZeroU64::new(5).unwrap();
    let selected = GrantParams {
        grant: op.id(),
        deadline_ms,
    };
    let mut calls = vec![
        ControlCall::Hello(HelloParams {
            lane: ControlLane::Normal,
        }),
        ControlCall::Observe(ObserveParams {
            deadline_ms,
            snapshot: 0,
            offset: 0,
        }),
        ControlCall::Activate(selected.clone()),
        ControlCall::Drain(selected),
        ControlCall::Remove(RemoveParams {
            inverse: op.removal(),
            owner: op.owner().clone(),
            force: false,
            deadline_ms,
        }),
        ControlCall::Shutdown(ShutdownParams { deadline_ms }),
    ];
    calls.extend(operations().into_iter().map(|operation| {
        ControlCall::Install(InstallParams {
            address: matches!(operation, UniverseOp::SetProxyMap { .. })
                .then(|| ProxyAddress::new(ProxyAddress::FIRST).unwrap()),
            operation,
            deadline_ms,
        })
    }));
    calls
}

fn params(call: &ControlCall) -> Value {
    let encoded = encode_control_request(1, call).unwrap();
    serde_json::from_str::<Value>(std::str::from_utf8(&encoded).unwrap()).unwrap()["params"].clone()
}

fn request(id: &str, method: &str, params: impl std::fmt::Display) -> String {
    format!(
        "{{\"id\":{id},\"method\":{},\"params\":{params}}}",
        json!(method)
    )
}

fn refusal(state: LaneState, line: &str, code: ProtocolCode, id: &str) {
    let refused = decode_control_request(state, line.as_bytes()).expect_err(line);
    assert_eq!(refused.id.as_json(), id, "{line}");
    let BridgeError::Protocol {
        code: found,
        message,
    } = refused.error
    else {
        panic!("protocol refusal")
    };
    assert_eq!(found, code, "{line}");
    assert!(!message.is_empty());
    assert!(message.len() <= 1024);
}

#[test]
fn proxy_address_range() {
    for text in [
        "198.18.0.1",
        "198.18.255.255",
        "198.19.0.0",
        "198.19.255.254",
    ] {
        let ip: Ipv4Addr = text.parse().unwrap();
        let address = ProxyAddress::new(ip).unwrap();
        assert_eq!(address.get(), ip);
        assert_eq!(serde_json::to_value(address).unwrap(), json!(text));
        assert_eq!(
            serde_json::from_value::<ProxyAddress>(json!(text)).unwrap(),
            address
        );
    }
    for text in [
        "198.18.0.0",
        "198.19.255.255",
        "198.20.0.0",
        "10.0.0.1",
        "255.255.255.255",
    ] {
        let address = text.parse().unwrap();
        let fault = AddressFault::OutOfRange { address };
        assert_eq!(ProxyAddress::new(address), Err(fault.clone()));
        assert!(fault.to_string().contains(text));
        assert!(serde_json::from_value::<ProxyAddress>(json!(text)).is_err());
    }
    for text in [
        "198.018.0.1",
        "198.18.0.01",
        " 198.18.0.1",
        "198.18.0.1\n",
        "::ffff:198.18.0.1",
        "",
        "198.18.1",
    ] {
        let error = serde_json::from_value::<ProxyAddress>(json!(text))
            .unwrap_err()
            .to_string();
        assert!(error.contains("not canonical IPv4"), "{text}: {error}");
    }
    for value in [Value::Null, json!(123), json!([198, 18, 0, 1]), json!({})] {
        assert!(serde_json::from_value::<ProxyAddress>(value).is_err());
    }
}

#[test]
fn method_names_are_exact_strings() {
    assert_eq!(
        ControlMethod::ALL.map(ControlMethod::name),
        [
            "hello", "observe", "install", "activate", "drain", "remove", "shutdown"
        ]
    );
    for method in ControlMethod::ALL {
        let name = method.name();
        assert_eq!(ControlMethod::from_name(name), Some(method));
        assert_eq!(serde_json::to_value(method).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<ControlMethod>(json!(name)).unwrap(),
            method
        );
        for value in [
            json!({name:null}),
            json!([name]),
            json!(name.to_uppercase()),
            json!(format!(" {name}")),
            Value::Null,
            json!(0),
        ] {
            assert!(serde_json::from_value::<ControlMethod>(value).is_err());
        }
    }
}

#[test]
fn unknown_method_is_method_not_found() {
    for name in [
        "unknown",
        "Hello",
        "hello ",
        "",
        "exec",
        "exec_status",
        "exec_output",
        "workload_stop",
    ] {
        assert_eq!(ControlMethod::from_name(name), None);
        for state in [LaneState::AwaitingHello, NORMAL, WITHDRAWAL] {
            refusal(
                state,
                &request(r#"{"k": [1.00]}"#, name, "{}"),
                ProtocolCode::MethodNotFound,
                r#"{"k": [1.00]}"#,
            );
        }
    }
}

#[test]
fn awaiting_hello_admits_only_hello() {
    for call in calls() {
        let encoded = encode_control_request(1, &call).unwrap();
        if call.method() == ControlMethod::Hello {
            assert!(decode_control_request(LaneState::AwaitingHello, &encoded).is_ok());
        } else {
            refusal(
                LaneState::AwaitingHello,
                std::str::from_utf8(&encoded).unwrap(),
                ProtocolCode::MethodNotFound,
                "1",
            );
        }
    }
}

#[test]
fn withdrawal_lane_admits_only_hello_observe_remove() {
    for mut call in calls() {
        if let ControlCall::Hello(hello) = &mut call {
            hello.lane = ControlLane::Withdrawal;
        }
        let method = call.method();
        let admitted = matches!(
            method,
            ControlMethod::Hello | ControlMethod::Observe | ControlMethod::Remove
        );
        assert_eq!(method.admitted_on(ControlLane::Withdrawal), admitted);
        assert!(method.admitted_on(ControlLane::Normal));
        let encoded = encode_control_request(1, &call).unwrap();
        if admitted {
            assert_eq!(
                decode_control_request(WITHDRAWAL, &encoded).unwrap().call,
                call
            );
        } else {
            refusal(
                WITHDRAWAL,
                std::str::from_utf8(&encoded).unwrap(),
                ProtocolCode::MethodNotFound,
                "1",
            );
        }
    }
}

#[test]
fn hello_lane_must_match_the_assigned_lane() {
    for (state, same, other) in [
        (NORMAL, "normal", "withdrawal"),
        (WITHDRAWAL, "withdrawal", "normal"),
    ] {
        let line = request("null", "hello", json!({"lane":same}));
        assert!(decode_control_request(state, line.as_bytes()).is_ok());
        refusal(
            state,
            &request("null", "hello", json!({"lane":other})),
            ProtocolCode::InvalidParams,
            "null",
        );
    }
    refusal(
        NORMAL,
        &request("7", "hello", json!({"lane":{"normal":null}})),
        ProtocolCode::InvalidParams,
        "7",
    );
}

#[test]
fn observe_requires_a_u64_id() {
    let body = json!({"deadline_ms":1,"snapshot":0,"offset":0});
    for id in [
        r#""1""#,
        "-1",
        "-0",
        "1.0",
        "1e0",
        "18446744073709551616",
        "null",
        "true",
        "[]",
        "{}",
    ] {
        refusal(
            NORMAL,
            &request(id, "observe", &body),
            ProtocolCode::InvalidRequest,
            id,
        );
    }
    for id in ["0", "1", "18446744073709551615"] {
        assert_eq!(
            decode_control_request(NORMAL, request(id, "observe", &body).as_bytes())
                .unwrap()
                .id
                .as_json(),
            id
        );
    }
}

fn refuses_both<T: DeserializeOwned + Debug>(value: Value) {
    assert!(
        serde_json::from_str::<T>(&value.to_string()).is_err(),
        "{value}"
    );
    assert!(
        serde_json::from_value::<T>(value.clone()).is_err(),
        "{value}"
    );
}

#[test]
fn params_are_closed_and_required() {
    for call in calls() {
        let method = call.method().name();
        let body = params(&call);
        let mut extra = body.clone();
        extra["extra"] = json!(0);
        refusal(
            NORMAL,
            &request("7", method, extra),
            ProtocolCode::InvalidParams,
            "7",
        );
        for (key, value) in body.as_object().unwrap() {
            let mut missing = body.clone();
            missing.as_object_mut().unwrap().remove(key);
            refusal(
                NORMAL,
                &request("7", method, missing),
                ProtocolCode::InvalidParams,
                "7",
            );
            let repeated = format!("{{{}:{value},{}", json!(key), &body.to_string()[1..]);
            refusal(
                NORMAL,
                &request("7", method, repeated),
                ProtocolCode::InvalidParams,
                "7",
            );
            if key != "address" {
                let mut null = body.clone();
                null[key] = Value::Null;
                refusal(
                    NORMAL,
                    &request("7", method, null),
                    ProtocolCode::InvalidParams,
                    "7",
                );
            }
        }
        if body.get("deadline_ms").is_some() {
            for bad in [json!(0), json!(-1), json!(1.0), json!("1")] {
                let mut changed = body.clone();
                changed["deadline_ms"] = bad;
                refusal(
                    NORMAL,
                    &request("7", method, changed),
                    ProtocolCode::InvalidParams,
                    "7",
                );
            }
            let overflow = body
                .to_string()
                .replace("\"deadline_ms\":5", "\"deadline_ms\":18446744073709551616");
            refusal(
                NORMAL,
                &request("7", method, overflow),
                ProtocolCode::InvalidParams,
                "7",
            );
        }
        let positional = Value::Array(match &call {
            ControlCall::Hello(_) => vec![body["lane"].clone()],
            ControlCall::Observe(_) => vec![
                body["deadline_ms"].clone(),
                body["snapshot"].clone(),
                body["offset"].clone(),
            ],
            ControlCall::Install(_) => vec![
                body["operation"].clone(),
                body["address"].clone(),
                body["deadline_ms"].clone(),
            ],
            ControlCall::Activate(_) | ControlCall::Drain(_) => {
                vec![body["grant"].clone(), body["deadline_ms"].clone()]
            }
            ControlCall::Remove(_) => vec![
                body["inverse"].clone(),
                body["owner"].clone(),
                body["force"].clone(),
                body["deadline_ms"].clone(),
            ],
            ControlCall::Shutdown(_) => vec![body["deadline_ms"].clone()],
        });
        for value in [positional, Value::Null, json!(1), json!("params")] {
            match call {
                ControlCall::Hello(_) => refuses_both::<HelloParams>(value),
                ControlCall::Observe(_) => refuses_both::<ObserveParams>(value),
                ControlCall::Install(_) => refuses_both::<InstallParams>(value),
                ControlCall::Activate(_) | ControlCall::Drain(_) => {
                    refuses_both::<GrantParams>(value)
                }
                ControlCall::Remove(_) => refuses_both::<RemoveParams>(value),
                ControlCall::Shutdown(_) => refuses_both::<ShutdownParams>(value),
            }
        }
    }
}

#[test]
fn install_address_rules() {
    for call in calls()
        .into_iter()
        .filter(|call| matches!(call, ControlCall::Install(_)))
    {
        let body = params(&call);
        assert!(serde_json::from_value::<InstallParams>(body.clone()).is_ok());
        let mut wrong = body.clone();
        wrong["address"] = if body["address"].is_null() {
            json!("198.18.0.1")
        } else {
            Value::Null
        };
        refusal(
            NORMAL,
            &request("7", "install", wrong),
            ProtocolCode::InvalidParams,
            "7",
        );
        let mut missing = body.clone();
        missing.as_object_mut().unwrap().remove("address");
        refusal(
            NORMAL,
            &request("7", "install", missing),
            ProtocolCode::InvalidParams,
            "7",
        );
    }
}

#[test]
fn install_refuses_an_inverse() {
    for operation in operations() {
        let body = json!({"operation":operation.removal(),"address":null,"deadline_ms":1});
        refusal(
            NORMAL,
            &request("7", "install", body),
            ProtocolCode::InvalidParams,
            "7",
        );
    }
}

#[test]
fn every_call_round_trips() {
    let grant: GrantId = serde_json::from_value(json!(GRANT)).unwrap();
    for call in calls() {
        let encoded = encode_control_request(u64::MAX, &call).unwrap();
        assert!(!encoded.iter().any(|b| matches!(b, b'\n' | b'\r' | b'\t')));
        let decoded = decode_control_request(NORMAL, &encoded).unwrap();
        assert_eq!(decoded.id, RequestId::number(u64::MAX));
        assert_eq!(decoded.call, call);
        let selected = !matches!(
            call,
            ControlCall::Hello(_) | ControlCall::Observe(_) | ControlCall::Shutdown(_)
        );
        assert_eq!(call.grant(), selected.then_some(grant));
    }
}

#[test]
fn observe_preserves_nondefault_paging_on_both_lanes() {
    for snapshot in [1, u64::MAX] {
        for offset in [1, u64::MAX] {
            let call = ControlCall::Observe(ObserveParams {
                deadline_ms: NonZeroU64::new(5).unwrap(),
                snapshot,
                offset,
            });
            let encoded = encode_control_request(1, &call).unwrap();
            let body: Value = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(body["params"]["snapshot"], json!(snapshot));
            assert_eq!(body["params"]["offset"], json!(offset));
            for state in [NORMAL, WITHDRAWAL] {
                assert_eq!(decode_control_request(state, &encoded).unwrap().call, call);
            }
        }
    }
}

#[test]
fn remove_preserves_force_on_both_lanes() {
    for force in [false, true] {
        for mut call in calls() {
            let ControlCall::Remove(remove) = &mut call else {
                continue;
            };
            remove.force = force;
            assert_eq!(params(&call)["force"], json!(force));
            let encoded = encode_control_request(1, &call).unwrap();
            for state in [NORMAL, WITHDRAWAL] {
                assert_eq!(decode_control_request(state, &encoded).unwrap().call, call);
            }
        }
    }
}

#[test]
fn install_preserves_the_assigned_proxy_address() {
    let operation = operations()
        .into_iter()
        .find(|op| matches!(op, UniverseOp::SetProxyMap { .. }))
        .unwrap();
    for ip in [
        ProxyAddress::FIRST,
        Ipv4Addr::new(198, 18, 0, 2),
        ProxyAddress::LAST,
    ] {
        let call = ControlCall::Install(InstallParams {
            operation: operation.clone(),
            address: Some(ProxyAddress::new(ip).unwrap()),
            deadline_ms: NonZeroU64::new(5).unwrap(),
        });
        let encoded = encode_control_request(1, &call).unwrap();
        let body: Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(body["params"]["address"], json!(ip.to_string()));
        assert_eq!(decode_control_request(NORMAL, &encoded).unwrap().call, call);
    }
}

#[test]
fn encoding_keeps_strings_and_propagates_operation_errors() {
    let mut operation = operations().remove(0);
    let UniverseOp::WriteSessionFile { owner, path, .. } = &mut operation else {
        unreachable!()
    };
    owner.plugin = "spaces  \t\n\r\"\\é".into();
    *path = "/a b".into();
    let mut call = ControlCall::Install(InstallParams {
        operation,
        address: None,
        deadline_ms: NonZeroU64::new(1).unwrap(),
    });
    let encoded = encode_control_request(1, &call).unwrap();
    assert_eq!(decode_control_request(NORMAL, &encoded).unwrap().call, call);
    let ControlCall::Install(params) = &mut call else {
        unreachable!()
    };
    let UniverseOp::WriteSessionFile { path, .. } = &mut params.operation else {
        unreachable!()
    };
    *path = "relative".into();
    let source = serde_json::to_string(&params.operation)
        .unwrap_err()
        .to_string();
    assert_eq!(
        encode_control_request(1, &call),
        Err(EncodeError::Serialize { message: source })
    );
}

#[test]
fn typed_refusals_keep_exact_ids_and_bound_guest_text() {
    for id in ["1.00", r#"{"k": [1, 2]}"#, r#""a\\b""#] {
        refusal(
            NORMAL,
            &request(id, "shutdown", json!({"extra":0})),
            ProtocolCode::InvalidParams,
            id,
        );
    }
    let extra = format!("{}😀", "a".repeat(1100));
    refusal(
        NORMAL,
        &request("1", "hello", json!({extra:0})),
        ProtocolCode::InvalidParams,
        "1",
    );
    refusal(NORMAL, "{", ProtocolCode::ParseError, "null");
    refusal(
        NORMAL,
        r#"{"id":1,"method":"hello","params":[]}"#,
        ProtocolCode::InvalidRequest,
        "1",
    );
}

proptest! {
    #[test]
    fn decoder_never_panics(bytes in prop::collection::vec(any::<u8>(),0..4096)) {
        for state in [LaneState::AwaitingHello,NORMAL,WITHDRAWAL] {
            let _ = decode_control_request(state,&bytes);
        }
    }
}
