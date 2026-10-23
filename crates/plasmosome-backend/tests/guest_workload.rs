use plasmosome_backend::guest_protocol::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::fmt::Debug;

fn boot() -> BootToken {
    BootToken::from_bytes([37; 32])
}
fn exec() -> ExecId {
    ExecId::new(11).unwrap()
}
fn rows() -> [(&'static str, Value, Value); 4] {
    let stream = json!({"bytes":[7],"total":1,"truncated":false,"closed":false});
    [
        (
            "exec",
            json!({"argv":["true"],"request_key":null,"deadline_ms":700000}),
            json!({"boot":boot(),"exec_id":"e-11","state":"running"}),
        ),
        (
            "exec_status",
            json!({"exec_id":"e-11","wait":true,"deadline_ms":17}),
            json!({"boot":boot(),"exec_id":"e-11","state":"exited","exit_code":255,"duration_ms":0}),
        ),
        (
            "exec_output",
            json!({"exec_id":"e-11","deadline_ms":17}),
            json!({"boot":boot(),"exec_id":"e-11","stdout":stream,"stderr":stream}),
        ),
        (
            "workload_stop",
            json!({"deadline_ms":17}),
            json!({"boot":boot(),"stopped":true}),
        ),
    ]
}
fn request(name: &str, params: &Value) -> Result<InboundRequest<ControlCall>, RequestRefusal> {
    decode_control_request(
        LaneState::Assigned(ControlLane::Normal),
        &encode_request(17, name, params).unwrap(),
    )
}
fn reply(call: &ControlCall, value: &Value) -> Result<ControlResult, ReplyFault> {
    decode_control_reply(
        call,
        17,
        Some(&boot()),
        &encode_success(&RequestId::number(17), value).unwrap(),
    )
}
fn closed<T: DeserializeOwned + Serialize + Debug + PartialEq>(value: Value) {
    let parsed: T = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), value);
    for key in value.as_object().unwrap().keys() {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<T>(missing.clone()).is_err(),
            "{missing}"
        );
        let text = value.to_string();
        let repeated = format!("{{\"{key}\":{},{}", value[key], &text[1..]);
        assert!(serde_json::from_str::<T>(&repeated).is_err(), "{repeated}");
    }
    let mut extra = value.clone();
    extra["unknown"] = json!(null);
    for bad in [
        extra,
        json!(value.as_object().unwrap().values().collect::<Vec<_>>()),
        json!(null),
    ] {
        assert!(serde_json::from_value::<T>(bad.clone()).is_err(), "{bad}");
        assert!(
            serde_json::from_str::<T>(&bad.to_string()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn exec_id_accepts_only_the_canonical_form() {
    assert_eq!(ExecId::new(0), None);
    for n in [1, 11, u64::MAX] {
        let text = format!("e-{n}");
        assert_eq!(ExecId::parse(&text).unwrap().number(), n);
        assert_eq!(
            serde_json::to_value(ExecId::new(n).unwrap()).unwrap(),
            json!(text)
        );
        assert_eq!(
            serde_json::from_value::<ExecId>(json!(text))
                .unwrap()
                .number(),
            n
        );
    }
    for text in [
        "x",
        "e-",
        "e-0",
        "e-01",
        "e-+1",
        "e--1",
        "e-1x",
        "e-18446744073709551616",
        "e-١",
        " e-1",
        "e-1\n",
    ] {
        assert_eq!(ExecId::parse(text), Err(ExecIdFault));
        assert!(serde_json::from_value::<ExecId>(json!(text)).is_err());
    }
    assert!(serde_json::from_value::<ExecId>(json!(1)).is_err());
}

#[test]
fn request_key_accepts_only_the_allowed_characters() {
    for text in ["a".to_owned(), "AZaz09._-".to_owned(), "z".repeat(64)] {
        let key = RequestKey::parse(&text).unwrap();
        assert_eq!(key.as_str(), text);
        assert_eq!(serde_json::to_value(&key).unwrap(), json!(text));
        assert_eq!(
            serde_json::from_value::<RequestKey>(json!(text)).unwrap(),
            key
        );
    }
    for text in [
        "".to_owned(),
        "a".repeat(65),
        "a b".to_owned(),
        "é".to_owned(),
        "a/".to_owned(),
        "a\0".to_owned(),
    ] {
        assert_eq!(RequestKey::parse(&text), Err(RequestKeyFault));
        assert!(serde_json::from_value::<RequestKey>(json!(text)).is_err());
    }
    assert!(serde_json::from_value::<RequestKey>(json!({})).is_err());
}

#[test]
fn exec_state_decodes_only_from_a_string() {
    for (state, name) in [
        (ExecState::Running, "running"),
        (ExecState::Exited, "exited"),
        (ExecState::Signaled, "signaled"),
    ] {
        assert_eq!(serde_json::to_value(state).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<ExecState>(json!(name)).unwrap(),
            state
        );
        for bad in [
            json!({name:null}),
            json!([name]),
            json!(name.to_uppercase()),
            json!("other"),
            json!(null),
            json!(0),
        ] {
            assert!(serde_json::from_value::<ExecState>(bad).is_err());
        }
    }
}

#[test]
fn signal_refuses_zero_and_over_64() {
    for n in [1, 9, 64] {
        let signal = Signal::new(n).unwrap();
        assert_eq!(signal.get(), n);
        assert_eq!(serde_json::to_value(signal).unwrap(), json!(n));
        assert_eq!(serde_json::from_value::<Signal>(json!(n)).unwrap(), signal);
    }
    for n in [0, 65, 255] {
        assert!(Signal::new(n).is_none());
    }
    for bad in [
        json!(0),
        json!(65),
        json!(256),
        json!(-1),
        json!(1.5),
        json!("9"),
    ] {
        assert!(serde_json::from_value::<Signal>(bad).is_err());
    }
}

#[test]
fn stream_refuses_inconsistent_truncation_and_oversize() {
    for total in [0, 1, 65535, 65536, 65537, u64::MAX] {
        let bytes = vec![7; total.min(65536) as usize];
        for closed_flag in [false, true] {
            let stream = Stream::new(bytes.clone(), total, closed_flag).unwrap();
            assert_eq!(
                (
                    stream.bytes(),
                    stream.total(),
                    stream.truncated(),
                    stream.closed()
                ),
                (bytes.as_ref(), total, total > 65536, closed_flag)
            );
            let mut value = serde_json::to_value(&stream).unwrap();
            closed::<Stream>(value.clone());
            value["truncated"] = json!(!stream.truncated());
            assert!(serde_json::from_value::<Stream>(value).is_err());
        }
    }
    for (bytes, total, fault) in [
        (vec![0; 65537], 65537, StreamFault::TooLong),
        (vec![0; 10], 100, StreamFault::ByteCount),
        (vec![0], 0, StreamFault::ByteCount),
    ] {
        assert_eq!(Stream::new(bytes.clone(), total, true), Err(fault));
        assert!(
            serde_json::from_value::<Stream>(
                json!({"bytes":bytes,"total":total,"truncated":true,"closed":true})
            )
            .is_err()
        );
        assert!(fault.to_string().starts_with("invalid stream:"));
    }
}

#[test]
fn exec_status_round_trips_each_shape() {
    let statuses = [
        ExecStatus::Running {
            boot: boot(),
            exec_id: exec(),
        },
        ExecStatus::Exited {
            boot: boot(),
            exec_id: exec(),
            exit_code: 0,
            duration_ms: u64::MAX,
        },
        ExecStatus::Signaled {
            boot: boot(),
            exec_id: exec(),
            signal: Signal::new(9).unwrap(),
            duration_ms: 40,
        },
    ];
    for status in statuses {
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(
            serde_json::from_value::<ExecStatus>(value.clone()).unwrap(),
            status
        );
        assert_eq!(status.boot(), boot());
        assert_eq!(status.exec_id(), exec());
        closed::<ExecStatus>(value);
    }
}

#[test]
fn exec_status_refuses_another_shapes_key() {
    for (state, fields, foreign) in [
        ("running", json!({}), "exit_code"),
        ("exited", json!({"exit_code":0,"duration_ms":0}), "signal"),
        ("signaled", json!({"signal":9,"duration_ms":0}), "exit_code"),
    ] {
        let mut value = json!({"boot":boot(),"exec_id":exec(),"state":state});
        value
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        for extra in [json!(1), json!(null)] {
            value[foreign] = extra;
            assert!(
                serde_json::from_value::<ExecStatus>(value.clone()).is_err(),
                "{value}"
            );
        }
    }
    for (field, bad) in [
        ("exit_code", json!(256)),
        ("exit_code", json!(-1)),
        ("duration_ms", json!(null)),
        ("state", json!({"exited":null})),
    ] {
        let mut value = rows()[1].2.clone();
        value[field] = bad;
        assert!(serde_json::from_value::<ExecStatus>(value).is_err());
    }
}

#[test]
fn exec_request_requires_request_key_but_accepts_null() {
    closed::<ExecParams>(rows()[0].1.clone());
    for key in [json!(null), json!("retry-1")] {
        let mut params = rows()[0].1.clone();
        params["request_key"] = key.clone();
        let call = request("exec", &params).unwrap().call;
        let encoded: Value =
            serde_json::from_slice(&encode_control_request(17, &call).unwrap()).unwrap();
        assert_eq!(encoded["params"]["request_key"], key);
    }
}

#[test]
fn exec_request_decodes_a_bad_argv_for_pid1_to_refuse() {
    for (argv, fault) in [
        (json!([]), ArgvFault::Empty),
        (json!([""]), ArgvFault::EmptyProgram),
        (json!(["a\0"]), ArgvFault::Nul { index: 0 }),
        (json!(["a", "b\0"]), ArgvFault::Nul { index: 1 }),
    ] {
        let mut params = rows()[0].1.clone();
        params["argv"] = argv;
        let ControlCall::Exec(params) = request("exec", &params).unwrap().call else {
            panic!()
        };
        assert_eq!(check_argv(&params.argv), Err(fault));
    }
    assert_eq!(check_argv(&["true".into(), "".into()]), Ok(()));
}

#[test]
fn requests_and_replies_are_closed_and_round_trip() {
    closed::<ExecStatusParams>(rows()[1].1.clone());
    closed::<ExecOutputParams>(rows()[2].1.clone());
    closed::<WorkloadStopParams>(rows()[3].1.clone());
    closed::<ExecResult>(rows()[0].2.clone());
    closed::<ExecOutputResult>(rows()[2].2.clone());
    closed::<WorkloadStopResult>(rows()[3].2.clone());
    for (name, params, result) in rows() {
        let call = request(name, &params).unwrap().call;
        assert_eq!(call.grant(), None);
        assert_eq!(call.method().name(), name);
        let encoded = encode_control_request(17, &call).unwrap();
        assert_eq!(
            decode_control_request(LaneState::Assigned(ControlLane::Normal), &encoded)
                .unwrap()
                .call,
            call
        );
        let parsed = reply(&call, &result).unwrap();
        let encoded = encode_control_result(&RequestId::number(17), &parsed).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&encoded).unwrap()["result"],
            result
        );
    }
    for state in ["running", "exited", "signaled"] {
        let mut value = rows()[0].2.clone();
        value["state"] = json!(state);
        closed::<ExecResult>(value);
    }
}

#[test]
fn reply_missing_or_unknown_field_refuses() {
    for (name, params, result) in rows() {
        let call = request(name, &params).unwrap().call;
        for key in result.as_object().unwrap().keys() {
            let mut missing = result.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(matches!(
                reply(&call, &missing),
                Err(ReplyFault::Result { .. })
            ));
        }
        let mut extra = result.clone();
        extra["unknown"] = json!(true);
        assert!(matches!(
            reply(&call, &extra),
            Err(ReplyFault::Result { .. })
        ));
    }
}

#[test]
fn zero_deadline_and_non_array_argv_are_invalid_params() {
    for (name, mut params, _) in rows() {
        params["deadline_ms"] = json!(0);
        assert!(matches!(
            request(name, &params).unwrap_err().error,
            BridgeError::Protocol {
                code: ProtocolCode::InvalidParams,
                ..
            }
        ));
    }
    for argv in [json!("true"), json!(null), json!([1]), json!({})] {
        let mut params = rows()[0].1.clone();
        params["argv"] = argv;
        assert!(matches!(
            request("exec", &params).unwrap_err().error,
            BridgeError::Protocol {
                code: ProtocolCode::InvalidParams,
                ..
            }
        ));
    }
}

#[test]
fn workload_verbs_are_refused_on_the_withdrawal_lane() {
    for (name, params, _) in rows() {
        let line = encode_request(17, name, &params).unwrap();
        for state in [
            LaneState::AwaitingHello,
            LaneState::Assigned(ControlLane::Withdrawal),
        ] {
            assert!(matches!(
                decode_control_request(state, &line).unwrap_err().error,
                BridgeError::Protocol {
                    code: ProtocolCode::MethodNotFound,
                    ..
                }
            ));
        }
    }
}

#[test]
fn a_reply_with_another_boot_is_boot_changed() {
    for (name, params, mut result) in rows() {
        let call = request(name, &params).unwrap().call;
        result["boot"] = json!(BootToken::from_bytes([38; 32]));
        assert_eq!(
            reply(&call, &result),
            Err(ReplyFault::BootChanged {
                expected: boot(),
                found: BootToken::from_bytes([38; 32])
            })
        );
    }
}

#[test]
fn an_exec_status_reply_for_another_exec_is_exec_mismatch() {
    for (name, params, mut result) in rows()
        .into_iter()
        .filter(|r| ["exec_status", "exec_output"].contains(&r.0))
    {
        let call = request(name, &params).unwrap().call;
        result["exec_id"] = json!("e-12");
        assert_eq!(
            reply(&call, &result),
            Err(ReplyFault::ExecMismatch {
                expected: exec(),
                found: ExecId::new(12).unwrap()
            })
        );
    }
}

#[test]
fn each_detail_encodes_exactly_as_spec_024_writes_it() {
    for (detail, literal) in [
        (RefusalDetail::InvalidArgv, r#"{"kind":"invalid_argv"}"#),
        (RefusalDetail::ExecLimit, r#"{"kind":"exec_limit"}"#),
        (
            RefusalDetail::SpawnFailed { errno: 11 },
            r#"{"kind":"spawn_failed","errno":11}"#,
        ),
        (RefusalDetail::Busy, r#"{"kind":"busy"}"#),
        (RefusalDetail::KeyConflict, r#"{"kind":"key_conflict"}"#),
        (RefusalDetail::Stopped, r#"{"kind":"stopped"}"#),
        (
            RefusalDetail::WorkloadRemains { count: 3 },
            r#"{"kind":"workload_remains","count":3}"#,
        ),
        (
            RefusalDetail::NotRetained {
                exec_id: "e-11".into(),
                what: NotRetainedWhat::Exec,
            },
            r#"{"kind":"not_retained","exec_id":"e-11","what":"exec"}"#,
        ),
    ] {
        assert_eq!(serde_json::to_string(&detail).unwrap(), literal);
        closed::<RefusalDetail>(serde_json::from_str(literal).unwrap());
        let error = BridgeError::Refused {
            message: "refused".into(),
            detail,
        };
        let bytes = encode_failure(&RequestId::number(17), &error);
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap(),
            json!({"id":17,"error":{"code":105,"message":"refused","from":"guest_request","to":"refused","detail":serde_json::from_str::<Value>(literal).unwrap()}})
        );
        assert_eq!(
            split_reply(17, &bytes).unwrap_err(),
            ReplyFault::Refused(error)
        );
    }
    for bad in [
        json!({"kind":"no_reply"}),
        json!({"kind":"spawn_failed","errno":2147483648u64}),
        json!({"kind":"workload_remains","count":-1}),
    ] {
        assert!(serde_json::from_value::<RefusalDetail>(bad).is_err());
    }
}

#[test]
fn not_retained_what_is_exact_and_required() {
    for what in ["exec", "output"] {
        let value = json!({"kind":"not_retained","exec_id":"e-11","what":what});
        closed::<RefusalDetail>(value);
    }
    for what in [
        json!("other"),
        json!("Exec"),
        json!({"exec":null}),
        json!(["output"]),
        json!(null),
    ] {
        assert!(
            serde_json::from_value::<RefusalDetail>(
                json!({"kind":"not_retained","exec_id":"e-11","what":what})
            )
            .is_err()
        );
    }
}
