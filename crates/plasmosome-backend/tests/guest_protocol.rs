use plasmosome_backend::DigestError;
use plasmosome_backend::guest_protocol::{
    BootToken, BootTokenError, BridgeError, ControlLane, EncodeError, ProtocolCode, RefusalDetail,
    RequestId, RequestRefusal, encode_failure, encode_request, encode_success, split_request,
};
use proptest::prelude::*;
use serde::de::DeserializeOwned;
use serde::{Serialize, Serializer};
use serde_json::value::RawValue;
use serde_json::{Value, json};

const BOOT: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn decoded<T: DeserializeOwned>(text: &str) -> [Result<T, String>; 2] {
    let value: Value = serde_json::from_str(text).expect("the case is JSON");
    let via_value = serde_json::from_value::<T>(value).map_err(|error| error.to_string());
    [
        serde_json::from_str::<T>(text).map_err(|error| error.to_string()),
        via_value,
    ]
}

fn refusals<T: DeserializeOwned + std::fmt::Debug>(text: &str) -> Vec<String> {
    let outcomes = decoded::<T>(text).into_iter();
    outcomes.map(|outcome| outcome.expect_err(text)).collect()
}

fn values(stream: &str) -> Vec<String> {
    let values = serde_json::Deserializer::from_str(stream).into_iter::<Value>();
    values
        .map(|value| value.expect("a JSON stream").to_string())
        .collect()
}

fn split(line: impl AsRef<[u8]>) -> Result<[String; 3], (String, i64)> {
    match split_request(line.as_ref()) {
        Ok((id, method, params)) => Ok([id.as_json().into(), method, params.get().into()]),
        Err(RequestRefusal {
            id,
            error: BridgeError::Protocol { code, .. },
        }) => Err((id.as_json().to_owned(), code.code())),
        Err(other) => panic!("split_request refuses only with protocol codes: {other:?}"),
    }
}

fn refusal_of(line: impl AsRef<[u8]>) -> (String, i64) {
    let line = line.as_ref();
    split(line).expect_err(&String::from_utf8_lossy(line))
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("an encoder writes UTF-8")
}

#[test]
fn control_lane_wire_names_are_exactly_normal_and_withdrawal() {
    for (lane, name) in [
        (ControlLane::Normal, "normal"),
        (ControlLane::Withdrawal, "withdrawal"),
    ] {
        assert_eq!(
            serde_json::to_value(lane).expect("a lane encodes"),
            json!(name)
        );
        assert_eq!(decoded(&json!(name).to_string()), [Ok(lane), Ok(lane)]);
    }
    let others = r#""Normal" "WITHDRAWAL" "withdraw" "" 0 true null {"normal":null} ["normal"]"#;
    for other in values(others) {
        refusals::<ControlLane>(&other);
    }
}

#[test]
fn a_boot_token_is_exactly_64_lowercase_hex_digits_in_text_and_in_json() {
    let token = BootToken::parse_hex(BOOT).expect("64 lowercase digits parse");
    let bytes: [u8; 32] = std::array::from_fn(|at| (at as u8).wrapping_mul(37));
    assert_eq!(
        (token.hex(), format!("{token:?}")),
        (BOOT.into(), BOOT.into())
    );
    assert_eq!(BootToken::from_bytes(*token.as_bytes()), token);
    assert_eq!(BootToken::from_bytes(bytes).as_bytes(), &bytes);
    let other = BootToken::from_bytes(bytes);
    assert_eq!(BootToken::parse_hex(&other.hex()), Ok(other));
    assert_ne!(other, token);
    assert_eq!(
        serde_json::to_value(token).expect("a token encodes"),
        json!(BOOT)
    );
    assert_eq!(decoded(&json!(BOOT).to_string()), [Ok(token), Ok(token)]);
    let wrong = |bytes| DigestError::WrongLength { bytes };
    for (digits, fault) in [
        (BOOT.to_uppercase(), DigestError::NotLowercaseHex { at: 20 }),
        (BOOT[..63].to_owned(), wrong(63)),
        (format!("{BOOT}0"), wrong(65)),
        (format!(" {BOOT}"), wrong(65)),
        (format!("{BOOT}\n"), wrong(65)),
        (String::new(), wrong(0)),
    ] {
        let fault = BootTokenError(fault);
        assert_eq!(BootToken::parse_hex(&digits), Err(fault), "{digits:?}");
        for error in refusals::<BootToken>(&json!(digits).to_string()) {
            assert!(error.contains(&fault.to_string()), "{error}");
            assert!(!error.contains("SHA-256"), "{error}");
        }
    }
    for other in values(&format!(r#"7 null ["{BOOT}"] {{"boot":"{BOOT}"}}"#)) {
        refusals::<BootToken>(&other);
    }
    assert_eq!(
        BootTokenError(wrong(3)).to_string(),
        "a boot token is 64 lowercase hexadecimal digits, not 3 bytes"
    );
    assert_eq!(
        BootTokenError(DigestError::NotLowercaseHex { at: 9 }).to_string(),
        "byte 9 of a boot token is not a lowercase hexadecimal digit"
    );
}

#[test]
fn a_request_line_splits_into_its_id_method_and_params_text() {
    let line = r#"{"id":7,"method":"activate","params":{"grant":"g","deadline_ms":5}}"#;
    let params = r#"{"grant":"g","deadline_ms":5}"#;
    assert_eq!(
        split(line),
        Ok(["7".into(), "activate".into(), params.into()])
    );
    let spaced = r#" {"params" : {} , "method":"" ,"id":null} "#;
    assert_eq!(
        split(spaced),
        Ok(["null".into(), String::new(), "{}".into()])
    );
}

#[test]
fn a_line_that_is_not_utf8_or_not_one_json_value_is_a_parse_error_under_a_null_id() {
    let mut not_utf8 = br#"{"id":1,"method":"hello","params":{}}"#.to_vec();
    not_utf8[20] = 0xff;
    let lines = r#"{
{"id":1,"method":"hello","params":{}}x
{"id":1,"method":"hello","params":{}}{"id":2}
{"id":1,"method":"hello","params":{},}
nul"#;
    for line in lines
        .lines()
        .map(str::as_bytes)
        .chain([&not_utf8[..], b"", b"   "])
    {
        let shown = String::from_utf8_lossy(line);
        assert_eq!(refusal_of(line), ("null".into(), -32700), "{shown:?}");
    }
}

#[test]
fn json_that_is_not_exactly_id_method_params_is_an_invalid_request_under_a_null_id() {
    let lines = r#"{"id":1,"method":"hello","params":{},"jsonrpc":"2.0"}
{"id":1,"method":"hello"}
{"method":"hello","params":{}}
{"id":1,"params":{}}
{"id":1,"id":2,"method":"hello","params":{}}
{"id":1,"method":"hello","method":"observe","params":{}}
{"id":1,"method":"hello","params":{},"params":{}}
{"id":1,"method":7,"params":{}}
{"id":1,"method":null,"params":{}}
[1,"hello",{}]
[{"id":1,"method":"hello","params":{}}]
"hello"
1
null
{}"#;
    for line in lines.lines() {
        assert_eq!(refusal_of(line), ("null".into(), -32600), "{line}");
    }
}

#[test]
fn params_that_are_not_an_object_are_an_invalid_request_under_the_request_id() {
    for (id, params) in [
        ("4", "[]"),
        (r#""x""#, r#"["lane"]"#),
        ("[1]", r#""normal""#),
    ]
    .into_iter()
    .chain([("4", "null"), ("4", "0")])
    {
        let line = format!(r#"{{"id":{id},"method":"hello","params":{params}}}"#);
        assert_eq!(refusal_of(&line), (id.into(), -32600), "{line}");
    }
}

#[test]
fn a_request_id_is_echoed_with_its_exact_json_text() {
    let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
    let ids = r#"7|"abc"|{"k":[1]}|{"k": [1] }|"\u0041"|1.0|1e0|-1|18446744073709551616|null|true"#;
    let error = BridgeError::Protocol {
        code: ProtocolCode::MethodNotFound,
        message: "m".to_owned(),
    };
    for id in ids.split('|').chain([deep.as_str()]) {
        let line = format!(r#"{{"id": {id} ,"method":"hello","params":{{}}}}"#);
        let (received, _, _) = split_request(line.as_bytes()).expect("any JSON id is accepted");
        assert_eq!(received.as_json(), id);
        let success =
            text(encode_success(&received, &json!({"ok": true})).expect("a result encodes"));
        assert_eq!(success, format!(r#"{{"id":{id},"result":{{"ok":true}}}}"#));
        let failure = text(encode_failure(&received, &error));
        assert_eq!(
            failure,
            format!(r#"{{"id":{id},"error":{{"code":-32601,"message":"m"}}}}"#)
        );
        let refused = format!(r#"{{"id": {id} ,"method":"hello","params":[]}}"#);
        assert_eq!(refusal_of(refused), (id.to_owned(), -32600));
    }
}

#[test]
fn request_ids_compare_by_text_and_read_as_u64_only_in_canonical_decimal() {
    let id = |text: &str| {
        let line = format!(r#"{{"id":{text},"method":"m","params":{{}}}}"#);
        split_request(line.as_bytes()).expect("a valid line").0
    };
    assert_eq!(
        (RequestId::number(7), RequestId::null()),
        (id("7"), id("null"))
    );
    assert_eq!(
        (RequestId::number(7).as_json(), RequestId::null().as_json()),
        ("7", "null")
    );
    assert_ne!(id("1"), id("1.0"));
    assert_ne!(id(r#"{"k":1}"#), id(r#"{"k": 1}"#));
    for (text, number) in [("0", 0), ("7", 7), ("18446744073709551615", u64::MAX)] {
        assert_eq!(id(text).as_u64(), Some(number));
        assert_eq!(RequestId::number(number).as_u64(), Some(number));
    }
    for text in r#"-0 -1 1.0 1e0 "7" 18446744073709551616 null [7]"#.split(' ') {
        assert_eq!(id(text).as_u64(), None, "{text}");
    }
}

#[test]
fn encode_request_writes_id_method_and_params_in_that_order() {
    let hello = encode_request(1, "hello", &json!({"lane": ControlLane::Normal}))
        .expect("hello params encode");
    assert_eq!(
        text(hello),
        r#"{"id":1,"method":"hello","params":{"lane":"normal"}}"#
    );
    let observe =
        text(encode_request(u64::MAX, "observe", &json!({})).expect("empty params encode"));
    assert_eq!(
        observe,
        r#"{"id":18446744073709551615,"method":"observe","params":{}}"#
    );
    let drain = encode_request(9, "drain", &json!({"grant": "g", "deadline_ms": 3}))
        .expect("drain params encode");
    let params = r#"{"deadline_ms":3,"grant":"g"}"#;
    assert_eq!(
        split(drain),
        Ok(["9".into(), "drain".into(), params.into()])
    );
}

#[test]
fn encode_request_returns_a_bounded_error_for_params_that_are_not_an_object() {
    for value in [
        json!(["lane"]),
        json!("normal"),
        json!(null),
        json!(7),
        json!(true),
    ] {
        let found = value.to_string();
        let error = encode_request(1, "hello", &value).expect_err("params must be an object");
        assert_eq!(
            error,
            EncodeError::ParamsNotObject {
                found: found.clone()
            }
        );
        assert_eq!(
            error.to_string(),
            format!("request params must encode as a JSON object, not {found}")
        );
        let erased: &dyn std::error::Error = &error;
        assert!(erased.source().is_none());
    }
    let value = json!(format!("{}{}", "k".repeat(1022), "😀".repeat(40)));
    let error = encode_request(1, "hello", &value).expect_err("a string is not an object");
    assert_eq!(
        error,
        EncodeError::ParamsNotObject {
            found: format!("\"{}", "k".repeat(1022))
        }
    );
}

struct Failing;

impl Serialize for Failing {
    fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("the value refused serialization"))
    }
}

#[test]
fn both_encoders_return_the_callers_serialization_error() {
    #[derive(Serialize)]
    struct Record {
        nested: Failing,
    }
    let record = Record { nested: Failing };
    for error in [
        encode_request(1, "install", &record),
        encode_request(1, "install", &Failing),
        encode_success(&RequestId::number(1), &record),
        encode_success(&RequestId::number(1), &Failing),
    ] {
        let error = error.expect_err("the serializer failed");
        assert_eq!(
            error,
            EncodeError::Serialize {
                message: "the value refused serialization".into()
            }
        );
        assert_eq!(
            error.to_string(),
            "bridge value failed to encode: the value refused serialization"
        );
    }
}

#[test]
fn both_encoders_compact_lf_and_cr_in_raw_json_at_every_depth() {
    #[derive(Serialize)]
    struct Record<'a> {
        nested: [&'a RawValue; 1],
    }
    for newline in ["\n", "\r", "\r\n"] {
        let raw = RawValue::from_string(format!("{{{newline}\t\"lane\" : \"normal\" {newline}}}"))
            .expect("formatted JSON");
        let nested = Record { nested: [&raw] };
        for (request, success, compact) in [
            (
                encode_request(1, "hello", &raw),
                encode_success(&RequestId::number(1), &raw),
                r#"{"lane":"normal"}"#,
            ),
            (
                encode_request(1, "hello", &nested),
                encode_success(&RequestId::number(1), &nested),
                r#"{"nested":[{"lane":"normal"}]}"#,
            ),
        ] {
            let request = text(request.expect("object params encode"));
            let success = text(success.expect("a result encodes"));
            assert_eq!(
                request,
                format!(r#"{{"id":1,"method":"hello","params":{compact}}}"#)
            );
            assert_eq!(success, format!(r#"{{"id":1,"result":{compact}}}"#));
            for line in [request, success] {
                assert!(!line.contains(['\n', '\r', '\t', ' ']));
                serde_json::from_str::<Value>(&line).expect("one complete JSON value");
            }
        }
    }
}

#[test]
fn both_encoders_keep_string_whitespace_escapes_and_exact_id_text() {
    let contents = r#"{"text":"a b\n\r\t\" c\\ d\\\" e😀　","tail":"ends\\"}"#;
    let raw = RawValue::from_string(format!("{{\n\"payload\" : {contents}\r\n}}"))
        .expect("string escapes are valid JSON");
    let compact = format!(r#"{{"payload":{contents}}}"#);
    let id_text = r#"{"key": ["a b\n"] }"#;
    let incoming = format!(r#"{{"id":{id_text},"method":"m","params":{{}}}}"#);
    let (id, _, _) = split_request(incoming.as_bytes()).expect("a compound id is valid");
    let request = text(encode_request(1, "a b\n", &raw).expect("params encode"));
    let success = text(encode_success(&id, &raw).expect("a result encodes"));
    assert_eq!(
        request,
        format!(r#"{{"id":1,"method":"a b\n","params":{compact}}}"#)
    );
    assert_eq!(success, format!(r#"{{"id":{id_text},"result":{compact}}}"#));
    for line in [request, success] {
        assert!(!line.contains(['\n', '\r']));
        let value: Value = serde_json::from_str(&line).expect("a complete JSON value");
        let payload = value
            .get("params")
            .or_else(|| value.get("result"))
            .expect("a payload");
        assert_eq!(
            payload,
            &serde_json::from_str::<Value>(&compact).expect("expected JSON")
        );
    }
    for result in [json!(null), json!(["a b\n"]), json!(7)] {
        let line = encode_success(&id, &result).expect("results need not be objects");
        let value: Value = serde_json::from_slice(&line).expect("a complete reply");
        assert_eq!(value["result"], result);
    }
}

#[test]
fn encode_failure_escapes_line_breaks_in_all_error_strings() {
    let message = "a b\n\r\t\"\\".to_owned();
    for error in [
        BridgeError::Protocol {
            code: ProtocolCode::InvalidRequest,
            message: message.clone(),
        },
        BridgeError::Refused {
            message: message.clone(),
            detail: RefusalDetail::Rejected {
                reason: message.clone(),
            },
        },
    ] {
        let line = text(encode_failure(&RequestId::number(1), &error));
        assert!(!line.contains(['\n', '\r', '\t']));
        let value: Value = serde_json::from_str(&line).expect("one complete failure reply");
        assert_eq!(value["error"]["message"], message);
        if matches!(error, BridgeError::Refused { .. }) {
            assert_eq!(value["error"]["detail"]["reason"], message);
        }
    }
}

#[test]
fn a_protocol_error_is_exactly_code_and_message_and_a_refusal_exactly_its_six_fields() {
    for (code, number) in [
        (ProtocolCode::ParseError, -32700),
        (ProtocolCode::InvalidRequest, -32600),
        (ProtocolCode::MethodNotFound, -32601),
        (ProtocolCode::InvalidParams, -32602),
        (ProtocolCode::InternalError, -32603),
    ] {
        assert_eq!(code.code(), number);
        let message = "bad \"line\"".to_owned();
        let line = text(encode_failure(
            &RequestId::null(),
            &BridgeError::Protocol { code, message },
        ));
        let error = format!(r#"{{"code":{number},"message":"bad \"line\""}}"#);
        assert_eq!(line, format!(r#"{{"id":null,"error":{error}}}"#));
    }
    let detail = RefusalDetail::Rejected { reason: "r".into() };
    let refusal = BridgeError::Refused {
        message: "m".into(),
        detail,
    };
    assert_eq!(
        text(encode_failure(&RequestId::number(3), &refusal)),
        r#"{"id":3,"error":{"code":105,"message":"m","from":"guest_request","to":"refused","detail":{"kind":"rejected","reason":"r"}}}"#
    );
}

#[test]
fn a_refusal_detail_decodes_only_from_its_closed_object() {
    let rejected = RefusalDetail::Rejected { reason: "r".into() };
    let encoded = serde_json::to_value(&rejected).expect("a detail encodes");
    assert_eq!(encoded, json!({"kind": "rejected", "reason": "r"}));
    let reordered = r#"{"reason":"r","kind":"rejected"}"#;
    assert_eq!(decoded(reordered), [Ok(rejected.clone()), Ok(rejected)]);
    for (detail, expected) in [
        (r#"["rejected","r"]"#, "invalid type: sequence"),
        (r#"{"kind":"rejected"}"#, "missing field `reason`"),
        (r#"{"reason":"r"}"#, "missing field `kind`"),
        (
            r#"{"kind":"rejected","reason":"r","stage":"x"}"#,
            "unknown field `stage`",
        ),
        (
            r#"{"kind":"no_active_grant","reason":"r"}"#,
            "unknown variant",
        ),
        (r#"{"kind":"rejected","reason":7}"#, "invalid type: integer"),
        (r#""rejected""#, "invalid type: string"),
    ] {
        for error in refusals::<RefusalDetail>(detail) {
            assert!(error.contains(expected), "{detail}: {error}");
        }
    }
    let repeated = r#"{"kind":"rejected","reason":"a","reason":"r"}"#;
    let error = serde_json::from_str::<RefusalDetail>(repeated).expect_err(repeated);
    assert!(
        error.to_string().contains("duplicate field `reason`"),
        "{error}"
    );
}

#[test]
fn a_refusal_message_keeps_at_most_1024_bytes_of_the_line_on_a_character_boundary() {
    for key in [
        "k".repeat(2000),
        format!("{}{}", "k".repeat(959), "😀".repeat(40)),
    ] {
        let line = json!({"id": 1, "method": "hello", "params": {}, &key: 0}).to_string();
        let Err(RequestRefusal {
            error: BridgeError::Protocol { code, message },
            ..
        }) = split_request(line.as_bytes())
        else {
            panic!("an unknown key refuses");
        };
        assert_eq!(code, ProtocolCode::InvalidRequest);
        assert!((1021..=1024).contains(&message.len()), "{}", message.len());
        let whole =
            format!("the request is not exactly {{id, method, params}}: unknown field `{key}`");
        assert!(whole.starts_with(&message), "{message}");
        let prefix = "the request is not exactly {id, method, params}: unknown field `";
        assert_eq!(prefix.len(), 64);
        let expected = if key.contains('😀') {
            assert_eq!(prefix.len() + 959, 1023);
            assert_eq!('😀'.len_utf8(), 4);
            format!("{prefix}{}", "k".repeat(959))
        } else {
            format!("{prefix}{}", "k".repeat(960))
        };
        assert_eq!(message, expected);
    }
}

fn request_like() -> impl Strategy<Value = String> {
    let key = prop_oneof![
        prop::sample::select(vec!["id", "method", "params"]).prop_map(str::to_owned),
        "\\PC{0,6}",
    ];
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<i64>().prop_map(Value::from),
        "\\PC{0,8}".prop_map(Value::from),
    ];
    let value = leaf.prop_recursive(3, 12, 3, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..3).prop_map(Value::from),
            prop::collection::btree_map("\\PC{0,4}", inner, 0..3)
                .prop_map(|entries| Value::Object(entries.into_iter().collect())),
        ]
    });
    let fields = prop::collection::vec((key, value), 0..5);
    fields.prop_map(|fields| {
        let fields: Vec<String> = fields
            .iter()
            .map(|(key, value)| format!("{}:{value}", json!(key)))
            .collect();
        format!("{{{}}}", fields.join(","))
    })
}

proptest! {
    #[test]
    fn split_request_never_panics_and_keeps_what_it_accepts(
        bytes in prop::collection::vec(any::<u8>(), 0..64),
        object in request_like(),
    ) {
        for line in [bytes, object.into_bytes()] {
            match split_request(&line) {
                Ok((id, _, params)) => {
                    let text = String::from_utf8(line.clone()).expect("an accepted line is UTF-8");
                    prop_assert!(text.contains(id.as_json()));
                    prop_assert!(params.get().starts_with('{'), "params {}", params.get());
                }
                Err(RequestRefusal { id, error: BridgeError::Protocol { code, message } }) => {
                    prop_assert!(message.len() <= 1024);
                    let parse = code == ProtocolCode::ParseError;
                    prop_assert!(parse || code == ProtocolCode::InvalidRequest, "{code:?}");
                    prop_assert!(!parse || id == RequestId::null(), "{id:?}");
                }
                Err(other) => prop_assert!(false, "{other:?}"),
            }
        }
    }
}
