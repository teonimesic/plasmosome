use super::digest::{Digest, DigestError};
use super::strict_json::{StrictJsonError, parse_value};
use serde_json::{Map, Value};

const GUEST_TEXT_LIMIT: usize = 1024;

/// One of the two connections the guest shim opens to the host's 4090
/// control port. The shim opens the normal lane first and the withdrawal lane
/// after the normal lane's hello.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControlLane {
    /// The lane that carries every 4090 verb.
    Normal,
    /// The prioritized lane that admits only hello, observe and remove.
    Withdrawal,
}

/// The hello request line the host writes on `lane`, ending in one newline.
///
/// It carries exactly `id`, `method` and `params: {lane}`. The guest shim
/// exits on any other field, so send these bytes unchanged.
pub fn hello_request(lane: ControlLane) -> &'static [u8] {
    match lane {
        ControlLane::Normal => {
            b"{\"id\":1,\"method\":\"hello\",\"params\":{\"lane\":\"normal\"}}\n"
        }
        ControlLane::Withdrawal => {
            b"{\"id\":2,\"method\":\"hello\",\"params\":{\"lane\":\"withdrawal\"}}\n"
        }
    }
}

/// The request id that `hello_request(lane)` carries and its reply must
/// echo: 1 on the normal lane, 2 on the withdrawal lane.
pub fn request_id(lane: ControlLane) -> u64 {
    match lane {
        ControlLane::Normal => 1,
        ControlLane::Withdrawal => 2,
    }
}

/// The guest's boot token: 32 random bytes the shim draws once per boot and
/// returns on both lanes as 64 lowercase hexadecimal digits.
///
/// `Debug` prints those 64 digits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BootToken(Digest);

impl BootToken {
    /// Reads exactly 64 lowercase hexadecimal digits. Uppercase digits, any
    /// other byte and any other length are refused with the first fault.
    pub fn parse_hex(text: &str) -> Result<BootToken, DigestError> {
        Digest::parse_hex(text).map(BootToken)
    }

    /// The 64 lowercase hexadecimal digits.
    pub fn hex(&self) -> String {
        self.0.hex()
    }
}

impl std::fmt::Debug for BootToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.hex())
    }
}

/// A hello reply that passed every check in [`judge_reply`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HelloAnswer {
    /// The lane the reply arrived on.
    pub lane: ControlLane,
    /// The guest's boot token.
    pub boot: BootToken,
    /// The SHA-256 of the guest policy the shim verified; equal to the
    /// expected one.
    pub policy: Digest,
}

/// Why [`judge_reply`] refused a hello reply.
///
/// `field` and `path` are JSON pointers into the reply: `""` is the whole
/// reply and `"/result/boot"` its boot token. Text the guest chose is cut to
/// 1024 bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HelloRefusal {
    /// The frame is not UTF-8.
    NotUtf8,
    /// The frame is not exactly one JSON value.
    NotJson { detail: String },
    /// An object in the frame repeats the key at `path`.
    DuplicateKey { path: String },
    /// The value at `field` is not a JSON object.
    NotAnObject { field: String },
    /// The record holds a field hello does not define.
    UnknownField { field: String },
    /// The record lacks a field hello requires.
    MissingField { field: String },
    /// The value at `field` is not a string.
    WrongType { field: String },
    /// The reply answers `found`, written as JSON, instead of the lane's
    /// integer request id.
    WrongId { expected: u64, found: String },
    /// The guest answered with an error object, under any id. `code` is
    /// `None` when the error has no integer `code`; `message` is empty when
    /// it has no string `message`.
    GuestError { code: Option<i64>, message: String },
    /// `boot` is not 64 lowercase hexadecimal digits.
    BadBoot { fault: DigestError },
    /// `policy` is not 64 lowercase hexadecimal digits.
    BadPolicy { fault: DigestError },
    /// The guest verified a policy other than the expected one.
    PolicyMismatch { expected: Digest, reported: Digest },
    /// This lane's boot token differs from the one an earlier hello returned.
    BootChanged { first: BootToken, second: BootToken },
}

impl std::fmt::Display for HelloRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HelloRefusal::NotUtf8 => f.write_str("the hello reply is not UTF-8"),
            HelloRefusal::NotJson { detail } => {
                write!(f, "the hello reply is not one JSON value: {detail}")
            }
            HelloRefusal::DuplicateKey { path } => {
                write!(f, "the hello reply repeats the key at {path:?}")
            }
            HelloRefusal::NotAnObject { field } => {
                write!(f, "the hello reply value at {field:?} is not a JSON object")
            }
            HelloRefusal::UnknownField { field } => write!(
                f,
                "the hello reply has a field hello does not define at {field:?}"
            ),
            HelloRefusal::MissingField { field } => {
                write!(f, "the hello reply has no {field:?}")
            }
            HelloRefusal::WrongType { field } => {
                write!(f, "the hello reply value at {field:?} is not a string")
            }
            HelloRefusal::WrongId { expected, found } => {
                write!(f, "the hello reply answers id {found}, not {expected}")
            }
            HelloRefusal::GuestError {
                code: Some(code),
                message,
            } => write!(f, "the guest refused hello with code {code}: {message:?}"),
            HelloRefusal::GuestError {
                code: None,
                message,
            } => write!(
                f,
                "the guest refused hello without an integer code: {message:?}"
            ),
            HelloRefusal::BadBoot {
                fault: DigestError::WrongLength { bytes },
            } => write!(
                f,
                "the hello boot token is {bytes} bytes long, not 64 lowercase hexadecimal digits"
            ),
            HelloRefusal::BadBoot {
                fault: DigestError::NotLowercaseHex { at },
            } => write!(
                f,
                "byte {at} of the hello boot token is not a lowercase hexadecimal digit"
            ),
            HelloRefusal::BadPolicy { fault } => write!(f, "the hello policy is refused: {fault}"),
            HelloRefusal::PolicyMismatch { expected, reported } => write!(
                f,
                "the guest verified policy {reported}, not the expected {expected}"
            ),
            HelloRefusal::BootChanged { first, second } => write!(
                f,
                "the guest boot token changed from {} to {}",
                first.hex(),
                second.hex()
            ),
        }
    }
}

impl std::error::Error for HelloRefusal {}

/// Judges one hello reply read from `lane`.
///
/// `frame` is one line without its newline; the caller's reader bounds its
/// size. The reply must be exactly `{"id", "result": {"boot", "policy"}}`
/// with the lane's request id, a 64-digit lowercase hexadecimal boot token
/// and a policy equal to `expected_policy`. Pass the boot token of the
/// normal lane's answer as `earlier` when judging the withdrawal lane, and
/// of the original association when judging a reconnection; the boot must
/// then equal it. An error object refuses as [`HelloRefusal::GuestError`]
/// whatever its id, because protocol failures answer under a `null` id.
pub fn judge_reply(
    lane: ControlLane,
    frame: &[u8],
    expected_policy: &Digest,
    earlier: Option<&BootToken>,
) -> Result<HelloAnswer, HelloRefusal> {
    if std::str::from_utf8(frame).is_err() {
        return Err(HelloRefusal::NotUtf8);
    }
    let reply = parse_value(frame).map_err(|fault| match fault {
        StrictJsonError::DuplicateKey { path } => HelloRefusal::DuplicateKey { path: cut(&path) },
        StrictJsonError::NotJson { .. } => HelloRefusal::NotJson {
            detail: fault.to_string(),
        },
    })?;
    let reply = object(&reply, "")?;
    if let Some(error) = reply.get("error") {
        return Err(HelloRefusal::GuestError {
            code: error.get("code").and_then(Value::as_i64),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .map(cut)
                .unwrap_or_default(),
        });
    }
    closed(reply, "", &["id", "result"])?;
    let expected = request_id(lane);
    if reply["id"].as_u64() != Some(expected) {
        return Err(HelloRefusal::WrongId {
            expected,
            found: cut(&reply["id"].to_string()),
        });
    }
    let result = object(&reply["result"], "/result")?;
    closed(result, "/result", &["boot", "policy"])?;
    let boot = BootToken::parse_hex(text(result, "/result", "boot")?)
        .map_err(|fault| HelloRefusal::BadBoot { fault })?;
    let policy = Digest::parse_hex(text(result, "/result", "policy")?)
        .map_err(|fault| HelloRefusal::BadPolicy { fault })?;
    if policy != *expected_policy {
        return Err(HelloRefusal::PolicyMismatch {
            expected: *expected_policy,
            reported: policy,
        });
    }
    if let Some(&first) = earlier
        && first != boot
    {
        return Err(HelloRefusal::BootChanged {
            first,
            second: boot,
        });
    }
    Ok(HelloAnswer { lane, boot, policy })
}

fn object<'a>(value: &'a Value, at: &str) -> Result<&'a Map<String, Value>, HelloRefusal> {
    value.as_object().ok_or_else(|| HelloRefusal::NotAnObject {
        field: at.to_owned(),
    })
}

fn closed(record: &Map<String, Value>, at: &str, fields: &[&str]) -> Result<(), HelloRefusal> {
    if let Some(unknown) = record.keys().find(|key| !fields.contains(&key.as_str())) {
        return Err(HelloRefusal::UnknownField {
            field: cut(&pointer(at, unknown)),
        });
    }
    match fields.iter().find(|field| !record.contains_key(**field)) {
        Some(missing) => Err(HelloRefusal::MissingField {
            field: pointer(at, missing),
        }),
        None => Ok(()),
    }
}

fn text<'a>(record: &'a Map<String, Value>, at: &str, key: &str) -> Result<&'a str, HelloRefusal> {
    record[key].as_str().ok_or_else(|| HelloRefusal::WrongType {
        field: pointer(at, key),
    })
}

fn pointer(at: &str, key: &str) -> String {
    format!("{at}/{}", key.replace('~', "~0").replace('/', "~1"))
}

fn cut(text: &str) -> String {
    text[..text.floor_char_boundary(GUEST_TEXT_LIMIT)].to_owned()
}

#[cfg(test)]
mod tests {
    use super::super::strict_json::parse_value;
    use super::*;
    use proptest::prelude::*;
    use serde_json::{Value, json};

    const BOOT: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    const OTHER_BOOT: &str = "ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100";

    fn guest_policy() -> Digest {
        Digest::of(b"guest policy bytes")
    }

    fn host_policy() -> Digest {
        Digest::of(b"host policy bytes")
    }

    fn boot(text: &str) -> BootToken {
        BootToken::parse_hex(text).expect("a 64-digit lowercase token parses")
    }

    fn reply(id: u64, boot: &str, policy: &str) -> String {
        format!(r#"{{"id":{id},"result":{{"boot":"{boot}","policy":"{policy}"}}}}"#)
    }

    fn valid(lane: ControlLane) -> String {
        reply(request_id(lane), BOOT, &guest_policy().hex())
    }

    fn judge(lane: ControlLane, frame: &str) -> Result<HelloAnswer, HelloRefusal> {
        judge_reply(lane, frame.as_bytes(), &guest_policy(), None)
    }

    fn refusal(lane: ControlLane, frame: &str) -> HelloRefusal {
        judge(lane, frame).expect_err(frame)
    }

    fn owned(text: &str) -> String {
        text.to_string()
    }

    fn keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn the_hello_requests_are_exactly_the_two_qualified_lines() {
        assert_eq!(
            hello_request(ControlLane::Normal),
            b"{\"id\":1,\"method\":\"hello\",\"params\":{\"lane\":\"normal\"}}\n"
        );
        assert_eq!(
            hello_request(ControlLane::Withdrawal),
            b"{\"id\":2,\"method\":\"hello\",\"params\":{\"lane\":\"withdrawal\"}}\n"
        );
    }

    #[test]
    fn each_request_is_one_line_of_only_id_method_and_params_lane() {
        for (lane, name) in [
            (ControlLane::Normal, "normal"),
            (ControlLane::Withdrawal, "withdrawal"),
        ] {
            let line = hello_request(lane);
            let (body, newline) = line.split_at(line.len() - 1);
            assert_eq!(newline, b"\n");
            assert!(!body.contains(&b'\n'), "{lane:?} is one line");
            let request = parse_value(body).expect("the request is strict JSON");
            assert_eq!(keys(&request), ["id", "method", "params"]);
            assert_eq!(keys(&request["params"]), ["lane"]);
            assert_eq!(
                request,
                json!({"id": request_id(lane), "method": "hello", "params": {"lane": name}})
            );
        }
        assert_eq!(request_id(ControlLane::Normal), 1);
        assert_eq!(request_id(ControlLane::Withdrawal), 2);
    }

    #[test]
    fn a_valid_reply_yields_its_lane_boot_and_policy() {
        assert_eq!(
            judge(ControlLane::Normal, &valid(ControlLane::Normal)),
            Ok(HelloAnswer {
                lane: ControlLane::Normal,
                boot: boot(BOOT),
                policy: guest_policy(),
            })
        );
        assert_eq!(
            judge_reply(
                ControlLane::Withdrawal,
                valid(ControlLane::Withdrawal).as_bytes(),
                &guest_policy(),
                Some(&boot(BOOT)),
            ),
            Ok(HelloAnswer {
                lane: ControlLane::Withdrawal,
                boot: boot(BOOT),
                policy: guest_policy(),
            })
        );
    }

    #[test]
    fn a_reply_under_any_id_but_the_lanes_integer_refuses() {
        let policy = guest_policy().hex();
        assert_eq!(
            refusal(ControlLane::Normal, &reply(2, BOOT, &policy)),
            HelloRefusal::WrongId {
                expected: 1,
                found: owned("2")
            }
        );
        assert_eq!(
            refusal(ControlLane::Withdrawal, &reply(1, BOOT, &policy)),
            HelloRefusal::WrongId {
                expected: 2,
                found: owned("1")
            }
        );
        for (id, found) in [
            (json!("1"), "\"1\""),
            (json!(1.0), "1.0"),
            (json!(-1), "-1"),
            (Value::Null, "null"),
        ] {
            let frame = json!({"id": id, "result": {"boot": BOOT, "policy": policy}});
            assert_eq!(
                refusal(ControlLane::Normal, &frame.to_string()),
                HelloRefusal::WrongId {
                    expected: 1,
                    found: owned(found)
                }
            );
        }
    }

    #[test]
    fn a_reply_without_an_id_refuses_as_missing() {
        let frame = json!({"result": {"boot": BOOT, "policy": guest_policy().hex()}});
        assert_eq!(
            refusal(ControlLane::Normal, &frame.to_string()),
            HelloRefusal::MissingField {
                field: owned("/id")
            }
        );
    }

    #[test]
    fn a_field_hello_does_not_define_refuses() {
        let policy = guest_policy().hex();
        let top = json!({"id": 1, "result": {"boot": BOOT, "policy": policy}, "deadline_ms": 5});
        assert_eq!(
            refusal(ControlLane::Normal, &top.to_string()),
            HelloRefusal::UnknownField {
                field: owned("/deadline_ms")
            }
        );
        let inner = json!({"id": 1, "result": {"boot": BOOT, "policy": policy, "grant": "g"}});
        assert_eq!(
            refusal(ControlLane::Normal, &inner.to_string()),
            HelloRefusal::UnknownField {
                field: owned("/result/grant")
            }
        );
        let slashed = json!({"id": 1, "result": {"boot": BOOT, "policy": policy, "a/b~": 0}});
        assert_eq!(
            refusal(ControlLane::Normal, &slashed.to_string()),
            HelloRefusal::UnknownField {
                field: owned("/result/a~1b~0")
            }
        );
    }

    #[test]
    fn a_field_hello_requires_refuses_when_absent() {
        let policy = guest_policy().hex();
        for (frame, missing) in [
            (json!({"id": 1}), "/result"),
            (
                json!({"id": 1, "result": {"policy": policy}}),
                "/result/boot",
            ),
            (json!({"id": 1, "result": {"boot": BOOT}}), "/result/policy"),
        ] {
            assert_eq!(
                refusal(ControlLane::Normal, &frame.to_string()),
                HelloRefusal::MissingField {
                    field: owned(missing)
                }
            );
        }
    }

    #[test]
    fn a_value_of_the_wrong_shape_refuses_naming_where() {
        let policy = guest_policy().hex();
        for (frame, expected) in [
            (json!([1]), HelloRefusal::NotAnObject { field: owned("") }),
            (
                json!({"id": 1, "result": "ok"}),
                HelloRefusal::NotAnObject {
                    field: owned("/result"),
                },
            ),
            (
                json!({"id": 1, "result": {"boot": 7, "policy": policy}}),
                HelloRefusal::WrongType {
                    field: owned("/result/boot"),
                },
            ),
            (
                json!({"id": 1, "result": {"boot": BOOT, "policy": null}}),
                HelloRefusal::WrongType {
                    field: owned("/result/policy"),
                },
            ),
        ] {
            assert_eq!(refusal(ControlLane::Normal, &frame.to_string()), expected);
        }
    }

    #[test]
    fn a_repeated_key_refuses_even_with_the_right_value_last() {
        let frame = format!(
            r#"{{"id":1,"result":{{"boot":"{BOOT}","policy":"{}","policy":"{}"}}}}"#,
            host_policy().hex(),
            guest_policy().hex()
        );
        assert_eq!(
            refusal(ControlLane::Normal, &frame),
            HelloRefusal::DuplicateKey {
                path: owned("/result/policy")
            }
        );
        assert_eq!(
            refusal(ControlLane::Normal, r#"{"id":2,"id":1}"#),
            HelloRefusal::DuplicateKey { path: owned("/id") }
        );
    }

    #[test]
    fn a_boot_token_not_64_lowercase_hex_digits_refuses() {
        let policy = guest_policy().hex();
        for (token, fault) in [
            (BOOT.to_uppercase(), DigestError::NotLowercaseHex { at: 20 }),
            (
                BOOT[..63].to_string(),
                DigestError::WrongLength { bytes: 63 },
            ),
            (format!("{BOOT}0"), DigestError::WrongLength { bytes: 65 }),
            (String::new(), DigestError::WrongLength { bytes: 0 }),
        ] {
            assert_eq!(
                refusal(ControlLane::Normal, &reply(1, &token, &policy)),
                HelloRefusal::BadBoot { fault },
                "{token:?}"
            );
        }
    }

    #[test]
    fn a_policy_other_than_the_expected_refuses_naming_both() {
        let reported = Digest::of(b"some other policy");
        assert_eq!(
            refusal(ControlLane::Normal, &reply(1, BOOT, &reported.hex())),
            HelloRefusal::PolicyMismatch {
                expected: guest_policy(),
                reported,
            }
        );
        assert_eq!(
            refusal(ControlLane::Normal, &reply(1, BOOT, &host_policy().hex())),
            HelloRefusal::PolicyMismatch {
                expected: guest_policy(),
                reported: host_policy(),
            }
        );
    }

    #[test]
    fn a_policy_not_64_lowercase_hex_digits_refuses() {
        let upper = guest_policy().hex().to_uppercase();
        let first_letter = upper
            .bytes()
            .position(|digit| digit.is_ascii_uppercase())
            .expect("a SHA-256 in hex has a letter");
        assert_eq!(
            refusal(ControlLane::Normal, &reply(1, BOOT, &upper)),
            HelloRefusal::BadPolicy {
                fault: DigestError::NotLowercaseHex { at: first_letter }
            }
        );
        assert_eq!(
            refusal(ControlLane::Normal, &reply(1, BOOT, "")),
            HelloRefusal::BadPolicy {
                fault: DigestError::WrongLength { bytes: 0 }
            }
        );
    }

    #[test]
    fn a_withdrawal_boot_other_than_the_normal_boot_refuses() {
        let normal = judge(ControlLane::Normal, &valid(ControlLane::Normal))
            .expect("the normal lane answers");
        let changed = reply(2, OTHER_BOOT, &guest_policy().hex());
        assert_eq!(
            judge_reply(
                ControlLane::Withdrawal,
                changed.as_bytes(),
                &guest_policy(),
                Some(&normal.boot),
            ),
            Err(HelloRefusal::BootChanged {
                first: boot(BOOT),
                second: boot(OTHER_BOOT),
            })
        );
    }

    #[test]
    fn an_error_reply_refuses_with_its_code_whatever_the_id() {
        assert_eq!(
            refusal(
                ControlLane::Normal,
                r#"{"id":1,"error":{"code":105,"message":"refused"}}"#
            ),
            HelloRefusal::GuestError {
                code: Some(105),
                message: owned("refused")
            }
        );
        assert_eq!(
            refusal(
                ControlLane::Normal,
                r#"{"id":null,"error":{"code":-32600,"message":"x"}}"#
            ),
            HelloRefusal::GuestError {
                code: Some(-32600),
                message: owned("x")
            }
        );
        let both = format!(
            r#"{{"id":1,"result":{{"boot":"{BOOT}","policy":"{}"}},"error":{{"code":105,"message":"m"}}}}"#,
            guest_policy().hex()
        );
        assert_eq!(
            refusal(ControlLane::Normal, &both),
            HelloRefusal::GuestError {
                code: Some(105),
                message: owned("m")
            }
        );
    }

    #[test]
    fn an_error_without_an_integer_code_or_a_string_message_still_refuses() {
        for error in [
            json!({"code": 1.5, "message": 7}),
            json!({"message": null}),
            json!("refused"),
            Value::Null,
        ] {
            let frame = json!({"id": 1, "error": error});
            assert_eq!(
                refusal(ControlLane::Normal, &frame.to_string()),
                HelloRefusal::GuestError {
                    code: None,
                    message: String::new()
                },
                "{frame}"
            );
        }
    }

    #[test]
    fn text_the_guest_chose_is_cut_to_1024_bytes() {
        let long = "k".repeat(2000);
        let message = json!({"id": 1, "error": {"code": 105, "message": long}});
        assert_eq!(
            refusal(ControlLane::Normal, &message.to_string()),
            HelloRefusal::GuestError {
                code: Some(105),
                message: "k".repeat(1024)
            }
        );
        let key = json!({"id": 1, "result": {}, &long: 0});
        assert_eq!(
            refusal(ControlLane::Normal, &key.to_string()),
            HelloRefusal::UnknownField {
                field: format!("/{}", &long[..1023])
            }
        );
        let id = json!({"id": long, "result": {}});
        assert_eq!(
            refusal(ControlLane::Normal, &id.to_string()),
            HelloRefusal::WrongId {
                expected: 1,
                found: format!("\"{}", &long[..1023])
            }
        );
        let repeated = format!(r#"{{"{long}":0,"{long}":0}}"#);
        assert_eq!(
            refusal(ControlLane::Normal, &repeated),
            HelloRefusal::DuplicateKey {
                path: format!("/{}", &long[..1023])
            }
        );
    }

    #[test]
    fn bytes_that_are_not_utf8_refuse() {
        let mut frame = valid(ControlLane::Normal).into_bytes();
        frame[30] = 0xff;
        assert_eq!(
            judge_reply(ControlLane::Normal, &frame, &guest_policy(), None),
            Err(HelloRefusal::NotUtf8)
        );
    }

    #[test]
    fn text_after_the_reply_refuses() {
        assert_eq!(
            refusal(ControlLane::Normal, "{}x"),
            HelloRefusal::NotJson {
                detail: owned("refused as JSON at line 1, column 3: trailing characters")
            }
        );
        let twice = format!("{0}{0}", valid(ControlLane::Normal));
        assert!(matches!(
            refusal(ControlLane::Normal, &twice),
            HelloRefusal::NotJson { .. }
        ));
    }

    #[test]
    fn a_boot_token_round_trips_and_prints_its_digits() {
        let token = boot(BOOT);
        assert_eq!(token.hex(), BOOT);
        assert_eq!(format!("{token:?}"), BOOT);
        assert_ne!(token, boot(OTHER_BOOT));
    }

    #[test]
    fn refusals_describe_themselves() {
        for (refusal, shown) in [
            (HelloRefusal::NotUtf8, "the hello reply is not UTF-8".to_string()),
            (
                HelloRefusal::NotJson {
                    detail: owned("d"),
                },
                "the hello reply is not one JSON value: d".to_string(),
            ),
            (
                HelloRefusal::DuplicateKey {
                    path: owned("/id"),
                },
                "the hello reply repeats the key at \"/id\"".to_string(),
            ),
            (
                HelloRefusal::NotAnObject {
                    field: owned("/result"),
                },
                "the hello reply value at \"/result\" is not a JSON object".to_string(),
            ),
            (
                HelloRefusal::UnknownField {
                    field: owned("/x"),
                },
                "the hello reply has a field hello does not define at \"/x\"".to_string(),
            ),
            (
                HelloRefusal::MissingField {
                    field: owned("/id"),
                },
                "the hello reply has no \"/id\"".to_string(),
            ),
            (
                HelloRefusal::WrongType {
                    field: owned("/result/boot"),
                },
                "the hello reply value at \"/result/boot\" is not a string".to_string(),
            ),
            (
                HelloRefusal::WrongId {
                    expected: 1,
                    found: owned("\"1\""),
                },
                "the hello reply answers id \"1\", not 1".to_string(),
            ),
            (
                HelloRefusal::GuestError {
                    code: Some(105),
                    message: owned("no"),
                },
                "the guest refused hello with code 105: \"no\"".to_string(),
            ),
            (
                HelloRefusal::GuestError {
                    code: None,
                    message: owned("no"),
                },
                "the guest refused hello without an integer code: \"no\"".to_string(),
            ),
            (
                HelloRefusal::BadBoot {
                    fault: DigestError::WrongLength { bytes: 3 },
                },
                "the hello boot token is 3 bytes long, not 64 lowercase hexadecimal digits"
                    .to_string(),
            ),
            (
                HelloRefusal::BadBoot {
                    fault: DigestError::NotLowercaseHex { at: 9 },
                },
                "byte 9 of the hello boot token is not a lowercase hexadecimal digit".to_string(),
            ),
            (
                HelloRefusal::BadPolicy {
                    fault: DigestError::WrongLength { bytes: 3 },
                },
                "the hello policy is refused: a SHA-256 digest is 64 lowercase hexadecimal digits, not 3 bytes"
                    .to_string(),
            ),
            (
                HelloRefusal::PolicyMismatch {
                    expected: guest_policy(),
                    reported: host_policy(),
                },
                format!(
                    "the guest verified policy {}, not the expected {}",
                    host_policy(),
                    guest_policy()
                ),
            ),
            (
                HelloRefusal::BootChanged {
                    first: boot(BOOT),
                    second: boot(OTHER_BOOT),
                },
                format!("the guest boot token changed from {BOOT} to {OTHER_BOOT}"),
            ),
        ] {
            assert_eq!(refusal.to_string(), shown);
        }
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(
            frame in proptest::collection::vec(any::<u8>(), 0..256),
            withdrawal in any::<bool>(),
        ) {
            let lane = if withdrawal { ControlLane::Withdrawal } else { ControlLane::Normal };
            let _ = judge_reply(lane, &frame, &guest_policy(), Some(&boot(BOOT)));
        }

        #[test]
        fn a_guest_message_is_cut_on_a_character_boundary(message in "\\PC{0,700}") {
            let frame = json!({"id": null, "error": {"code": 105, "message": message}});
            let HelloRefusal::GuestError { message: kept, .. } =
                refusal(ControlLane::Normal, &frame.to_string())
            else {
                panic!("an error reply refuses as a guest error");
            };
            prop_assert!(message.starts_with(&kept));
            prop_assert!(kept.len() <= 1024);
            prop_assert!(message.len() <= 1024 || kept.len() > 1020);
            prop_assert!(message.len() > 1024 || kept == message);
        }
    }
}
