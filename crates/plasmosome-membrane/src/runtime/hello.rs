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
pub struct BootToken([u8; 32]);

/// Why a text is not a boot token. It holds the same fault a digest's text
/// can have, but describes it as a boot token's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootTokenError(pub DigestError);

impl BootToken {
    /// Reads exactly 64 lowercase hexadecimal digits. Uppercase digits, any
    /// other byte and any other length are refused with the first fault.
    pub fn parse_hex(text: &str) -> Result<BootToken, BootTokenError> {
        Digest::parse_hex(text)
            .map(|digest| BootToken(*digest.as_bytes()))
            .map_err(BootTokenError)
    }

    /// The 64 lowercase hexadecimal digits.
    pub fn hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

impl std::fmt::Debug for BootToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.hex())
    }
}

impl std::fmt::Display for BootTokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            DigestError::WrongLength { bytes } => write!(
                f,
                "a boot token is 64 lowercase hexadecimal digits, not {bytes} bytes"
            ),
            DigestError::NotLowercaseHex { at } => write!(
                f,
                "byte {at} of a boot token is not a lowercase hexadecimal digit"
            ),
        }
    }
}

impl std::error::Error for BootTokenError {}

/// Why a hello reply was refused.
///
/// `field` and `path` are JSON pointers into the reply: `""` is the whole
/// reply and `"/result/boot"` its boot token. Text the guest chose is cut to
/// at most 1024 bytes, on a character boundary and never inside a pointer's
/// `~` escape.
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
    /// `None` when the error has no `code` that is a signed 64-bit integer;
    /// `message` is empty when it has no string `message`; `detail` is the
    /// compact JSON of its `detail`, when it has one.
    GuestError {
        code: Option<i64>,
        message: String,
        detail: Option<String>,
    },
    /// `boot` is not 64 lowercase hexadecimal digits.
    BadBoot { fault: BootTokenError },
    /// `policy` is not 64 lowercase hexadecimal digits.
    BadPolicy { fault: DigestError },
    /// The guest verified the policy `found` instead of the expected one.
    PolicyMismatch { expected: Digest, found: Digest },
    /// The withdrawal lane returned the boot token `found` instead of the
    /// normal lane's.
    BootChanged {
        expected: BootToken,
        found: BootToken,
    },
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
                code,
                message,
                detail,
            } => {
                match code {
                    Some(code) => write!(f, "the guest refused hello with code {code}")?,
                    None => write!(
                        f,
                        "the guest refused hello with no code that is a signed 64-bit integer"
                    )?,
                }
                write!(f, ": {message:?}")?;
                match detail {
                    Some(detail) => write!(f, ", detail {detail}"),
                    None => Ok(()),
                }
            }
            HelloRefusal::BadBoot { fault } => {
                write!(f, "the hello boot token is refused: {fault}")
            }
            HelloRefusal::BadPolicy { fault } => write!(f, "the hello policy is refused: {fault}"),
            HelloRefusal::PolicyMismatch { expected, found } => write!(
                f,
                "the guest verified policy {found}, not the expected {expected}"
            ),
            HelloRefusal::BootChanged { expected, found } => write!(
                f,
                "the withdrawal lane's boot token is {}, not the normal lane's {}",
                found.hex(),
                expected.hex()
            ),
        }
    }
}

impl std::error::Error for HelloRefusal {}

/// Judges the normal lane's hello reply and returns its boot token, which
/// [`judge_withdrawal`] needs.
///
/// `frame` is one line without its newline; the caller's reader bounds its
/// size. The reply must be exactly `{"id": 1, "result": {"boot", "policy"}}`
/// with a 64-digit lowercase hexadecimal boot token and a policy equal to
/// `expected_policy`. An error object refuses as
/// [`HelloRefusal::GuestError`] whatever its id, because protocol failures
/// answer under a `null` id.
pub fn judge_normal(frame: &[u8], expected_policy: &Digest) -> Result<BootToken, HelloRefusal> {
    judge(ControlLane::Normal, frame, expected_policy)
}

/// Judges the withdrawal lane's hello reply by the same rules under id 2,
/// and refuses it unless its boot token equals `normal`, the token
/// [`judge_normal`] returned.
pub fn judge_withdrawal(
    frame: &[u8],
    expected_policy: &Digest,
    normal: &BootToken,
) -> Result<(), HelloRefusal> {
    let found = judge(ControlLane::Withdrawal, frame, expected_policy)?;
    if found != *normal {
        return Err(HelloRefusal::BootChanged {
            expected: *normal,
            found,
        });
    }
    Ok(())
}

fn judge(
    lane: ControlLane,
    frame: &[u8],
    expected_policy: &Digest,
) -> Result<BootToken, HelloRefusal> {
    if std::str::from_utf8(frame).is_err() {
        return Err(HelloRefusal::NotUtf8);
    }
    let reply = parse_value(frame).map_err(|fault| match fault {
        StrictJsonError::DuplicateKey { path } => HelloRefusal::DuplicateKey {
            path: cut_pointer(&path),
        },
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
            detail: error.get("detail").map(|detail| cut(&detail.to_string())),
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
    let found = Digest::parse_hex(text(result, "/result", "policy")?)
        .map_err(|fault| HelloRefusal::BadPolicy { fault })?;
    if found != *expected_policy {
        return Err(HelloRefusal::PolicyMismatch {
            expected: *expected_policy,
            found,
        });
    }
    Ok(boot)
}

fn object<'a>(value: &'a Value, at: &str) -> Result<&'a Map<String, Value>, HelloRefusal> {
    value.as_object().ok_or_else(|| HelloRefusal::NotAnObject {
        field: at.to_owned(),
    })
}

fn closed(record: &Map<String, Value>, at: &str, fields: &[&str]) -> Result<(), HelloRefusal> {
    if let Some(unknown) = record.keys().find(|key| !fields.contains(&key.as_str())) {
        return Err(HelloRefusal::UnknownField {
            field: cut_pointer(&pointer(at, unknown)),
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

fn cut_pointer(pointer: &str) -> String {
    let kept = cut(pointer);
    match kept.strip_suffix('~') {
        Some(whole_escapes) => whole_escapes.to_owned(),
        None => kept,
    }
}

#[cfg(test)]
mod tests {
    use super::super::strict_json::parse_value;
    use super::*;
    use proptest::prelude::*;
    use serde_json::{Value, json};

    const BOOT: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    const OTHER_BOOT: &str = "ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100";
    const LANES: [ControlLane; 2] = [ControlLane::Normal, ControlLane::Withdrawal];

    fn guest_policy() -> Digest {
        Digest::of(b"guest policy bytes")
    }

    fn host_policy() -> Digest {
        Digest::of(b"host policy bytes")
    }

    fn boot(text: &str) -> BootToken {
        BootToken::parse_hex(text).expect("a 64-digit lowercase token parses")
    }

    fn judge_on(lane: ControlLane, frame: &str) -> Result<BootToken, HelloRefusal> {
        match lane {
            ControlLane::Normal => judge_normal(frame.as_bytes(), &guest_policy()),
            ControlLane::Withdrawal => {
                judge_withdrawal(frame.as_bytes(), &guest_policy(), &boot(BOOT))
                    .map(|()| boot(BOOT))
            }
        }
    }

    fn refusal(lane: ControlLane, frame: &str) -> HelloRefusal {
        judge_on(lane, frame).expect_err(frame)
    }

    fn reply(lane: ControlLane, boot: &str, policy: &str) -> String {
        json!({"id": request_id(lane), "result": {"boot": boot, "policy": policy}}).to_string()
    }

    fn owned(text: &str) -> String {
        text.to_string()
    }

    fn guest_text(refusal: HelloRefusal) -> String {
        match refusal {
            HelloRefusal::GuestError {
                message,
                detail: None,
                ..
            } => message,
            HelloRefusal::GuestError {
                detail: Some(detail),
                ..
            } => detail,
            HelloRefusal::UnknownField { field }
            | HelloRefusal::DuplicateKey { path: field }
            | HelloRefusal::WrongId { found: field, .. } => field,
            other => panic!("{other:?} carries no guest text"),
        }
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
    fn no_request_carries_a_field_or_a_line_break_the_shim_would_die_on() {
        for (lane, name) in [
            (ControlLane::Normal, "normal"),
            (ControlLane::Withdrawal, "withdrawal"),
        ] {
            let line = hello_request(lane);
            let (body, newline) = line.split_at(line.len() - 1);
            assert_eq!(newline, b"\n");
            assert!(!body.contains(&b'\n'), "{lane:?} is one line");
            assert_eq!(
                parse_value(body),
                Ok(json!({"id": request_id(lane), "method": "hello", "params": {"lane": name}}))
            );
        }
        assert_eq!(request_id(ControlLane::Normal), 1);
        assert_eq!(request_id(ControlLane::Withdrawal), 2);
    }

    #[test]
    fn a_valid_reply_yields_the_boot_token_on_either_lane() {
        let policy = guest_policy().hex();
        let normal = reply(ControlLane::Normal, BOOT, &policy);
        assert_eq!(
            judge_normal(normal.as_bytes(), &guest_policy()),
            Ok(boot(BOOT))
        );
        let withdrawal = reply(ControlLane::Withdrawal, BOOT, &policy);
        assert_eq!(
            judge_withdrawal(withdrawal.as_bytes(), &guest_policy(), &boot(BOOT)),
            Ok(())
        );
    }

    #[test]
    fn a_reply_under_any_id_but_the_lanes_integer_refuses() {
        let policy = guest_policy().hex();
        for (lane, other) in [
            (ControlLane::Normal, ControlLane::Withdrawal),
            (ControlLane::Withdrawal, ControlLane::Normal),
        ] {
            let id = request_id(lane);
            for (sent, found) in [
                (json!(request_id(other)), request_id(other).to_string()),
                (json!(id.to_string()), format!("\"{id}\"")),
                (json!(id as f64), format!("{id}.0")),
                (json!(-1), owned("-1")),
                (Value::Null, owned("null")),
            ] {
                let frame = json!({"id": sent, "result": {"boot": BOOT, "policy": policy}});
                assert_eq!(
                    refusal(lane, &frame.to_string()),
                    HelloRefusal::WrongId {
                        expected: id,
                        found
                    }
                );
            }
        }
    }

    #[test]
    fn a_field_hello_does_not_define_refuses_even_when_it_resembles_one() {
        let policy = guest_policy().hex();
        for lane in LANES {
            for (key, field) in [
                ("deadline_ms", "/deadline_ms"),
                ("ids", "/ids"),
                ("results", "/results"),
                ("a/b~", "/a~1b~0"),
            ] {
                let mut frame =
                    json!({"id": request_id(lane), "result": {"boot": BOOT, "policy": policy}});
                frame[key] = json!(0);
                assert_eq!(
                    refusal(lane, &frame.to_string()),
                    HelloRefusal::UnknownField {
                        field: owned(field)
                    }
                );
            }
            for (key, field) in [
                ("grant", "/result/grant"),
                ("boots", "/result/boots"),
                ("policy2", "/result/policy2"),
            ] {
                let mut frame =
                    json!({"id": request_id(lane), "result": {"boot": BOOT, "policy": policy}});
                frame["result"][key] = json!(0);
                assert_eq!(
                    refusal(lane, &frame.to_string()),
                    HelloRefusal::UnknownField {
                        field: owned(field)
                    }
                );
            }
        }
    }

    #[test]
    fn a_field_hello_requires_refuses_when_absent() {
        let policy = guest_policy().hex();
        for lane in LANES {
            let id = request_id(lane);
            for (frame, missing) in [
                (json!({"result": {"boot": BOOT, "policy": policy}}), "/id"),
                (json!({"id": id}), "/result"),
                (
                    json!({"id": id, "result": {"policy": policy}}),
                    "/result/boot",
                ),
                (
                    json!({"id": id, "result": {"boot": BOOT}}),
                    "/result/policy",
                ),
            ] {
                assert_eq!(
                    refusal(lane, &frame.to_string()),
                    HelloRefusal::MissingField {
                        field: owned(missing)
                    }
                );
            }
        }
    }

    #[test]
    fn a_value_of_the_wrong_shape_refuses_naming_where() {
        let policy = guest_policy().hex();
        for lane in LANES {
            let id = request_id(lane);
            for (frame, expected) in [
                (json!([1]), HelloRefusal::NotAnObject { field: owned("") }),
                (
                    json!({"id": id, "result": "ok"}),
                    HelloRefusal::NotAnObject {
                        field: owned("/result"),
                    },
                ),
                (
                    json!({"id": id, "result": {"boot": 7, "policy": policy}}),
                    HelloRefusal::WrongType {
                        field: owned("/result/boot"),
                    },
                ),
                (
                    json!({"id": id, "result": {"boot": BOOT, "policy": null}}),
                    HelloRefusal::WrongType {
                        field: owned("/result/policy"),
                    },
                ),
            ] {
                assert_eq!(refusal(lane, &frame.to_string()), expected);
            }
        }
    }

    #[test]
    fn a_repeated_key_refuses_even_with_the_right_value_last() {
        for lane in LANES {
            let id = request_id(lane);
            let frame = format!(
                r#"{{"id":{id},"result":{{"boot":"{BOOT}","policy":"{}","policy":"{}"}}}}"#,
                host_policy().hex(),
                guest_policy().hex()
            );
            assert_eq!(
                refusal(lane, &frame),
                HelloRefusal::DuplicateKey {
                    path: owned("/result/policy")
                }
            );
            assert_eq!(
                refusal(lane, &format!(r#"{{"id":7,"id":{id}}}"#)),
                HelloRefusal::DuplicateKey { path: owned("/id") }
            );
        }
    }

    #[test]
    fn a_boot_token_not_64_lowercase_hex_digits_refuses() {
        let policy = guest_policy().hex();
        for lane in LANES {
            for (token, fault) in [
                (BOOT.to_uppercase(), DigestError::NotLowercaseHex { at: 20 }),
                (
                    BOOT[..63].to_string(),
                    DigestError::WrongLength { bytes: 63 },
                ),
                (format!("{BOOT}0"), DigestError::WrongLength { bytes: 65 }),
                (format!(" {BOOT}"), DigestError::WrongLength { bytes: 65 }),
                (format!("{BOOT}\n"), DigestError::WrongLength { bytes: 65 }),
                (String::new(), DigestError::WrongLength { bytes: 0 }),
            ] {
                assert_eq!(
                    refusal(lane, &reply(lane, &token, &policy)),
                    HelloRefusal::BadBoot {
                        fault: BootTokenError(fault)
                    },
                    "{token:?}"
                );
            }
        }
    }

    #[test]
    fn a_policy_other_than_the_expected_refuses_naming_both() {
        for lane in LANES {
            for found in [Digest::of(b"some other policy"), host_policy()] {
                assert_eq!(
                    refusal(lane, &reply(lane, BOOT, &found.hex())),
                    HelloRefusal::PolicyMismatch {
                        expected: guest_policy(),
                        found,
                    }
                );
            }
        }
    }

    #[test]
    fn a_policy_not_64_lowercase_hex_digits_refuses() {
        let policy = guest_policy().hex();
        let upper = policy.to_uppercase();
        let first_letter = upper
            .bytes()
            .position(|digit| digit.is_ascii_uppercase())
            .expect("a SHA-256 in hex has a letter");
        for lane in LANES {
            for (text, fault) in [
                (
                    upper.clone(),
                    DigestError::NotLowercaseHex { at: first_letter },
                ),
                (
                    format!("{policy}\n"),
                    DigestError::WrongLength { bytes: 65 },
                ),
                (format!(" {policy}"), DigestError::WrongLength { bytes: 65 }),
                (String::new(), DigestError::WrongLength { bytes: 0 }),
            ] {
                assert_eq!(
                    refusal(lane, &reply(lane, BOOT, &text)),
                    HelloRefusal::BadPolicy { fault },
                    "{text:?}"
                );
            }
        }
    }

    #[test]
    fn a_withdrawal_boot_other_than_the_normal_boot_refuses() {
        let policy = guest_policy().hex();
        let normal = judge_normal(
            reply(ControlLane::Normal, BOOT, &policy).as_bytes(),
            &guest_policy(),
        )
        .expect("the normal lane answers");
        let changed = reply(ControlLane::Withdrawal, OTHER_BOOT, &policy);
        assert_eq!(
            judge_withdrawal(changed.as_bytes(), &guest_policy(), &normal),
            Err(HelloRefusal::BootChanged {
                expected: boot(BOOT),
                found: boot(OTHER_BOOT),
            })
        );
    }

    #[test]
    fn an_error_reply_refuses_with_its_code_and_detail_whatever_the_id() {
        let refused = json!({
            "code": 105, "message": "refused", "from": "guest_request", "to": "refused",
            "detail": {"stage": "policy_effect"}
        });
        for lane in LANES {
            for id in [json!(request_id(lane)), Value::Null, json!(7), json!("x")] {
                assert_eq!(
                    refusal(lane, &json!({"id": id, "error": refused}).to_string()),
                    HelloRefusal::GuestError {
                        code: Some(105),
                        message: owned("refused"),
                        detail: Some(owned(r#"{"stage":"policy_effect"}"#)),
                    },
                    "{id}"
                );
            }
            let protocol = json!({"id": null, "error": {"code": -32600, "message": "x"}});
            assert_eq!(
                refusal(lane, &protocol.to_string()),
                HelloRefusal::GuestError {
                    code: Some(-32600),
                    message: owned("x"),
                    detail: None,
                }
            );
            let both = json!({
                "id": request_id(lane),
                "result": {"boot": BOOT, "policy": guest_policy().hex()},
                "error": {"code": 105, "message": "m"}
            });
            assert!(matches!(
                refusal(lane, &both.to_string()),
                HelloRefusal::GuestError {
                    code: Some(105),
                    ..
                }
            ));
        }
    }

    #[test]
    fn an_error_without_a_signed_64_bit_code_or_a_string_message_still_refuses() {
        for lane in LANES {
            for error in [
                json!({"code": 1.5, "message": 7}),
                json!({"code": 9_223_372_036_854_775_808_u64}),
                json!({"message": null}),
                json!("refused"),
                Value::Null,
            ] {
                let frame = json!({"id": 1, "error": error});
                assert_eq!(
                    refusal(lane, &frame.to_string()),
                    HelloRefusal::GuestError {
                        code: None,
                        message: String::new(),
                        detail: None,
                    },
                    "{frame}"
                );
            }
        }
    }

    #[test]
    fn text_the_guest_chose_is_cut_to_1024_bytes_without_splitting_an_escape() {
        let long = "k".repeat(2000);
        let tildes = "~".repeat(600);
        let escapes = format!("/{}", "~0".repeat(511));
        for lane in LANES {
            let id = request_id(lane);
            for (frame, kept) in [
                (
                    json!({"id": id, "error": {"code": 105, "message": long}}).to_string(),
                    long[..1024].to_string(),
                ),
                (
                    json!({"id": id, "error": {"detail": long}}).to_string(),
                    format!("\"{}", &long[..1023]),
                ),
                (
                    json!({"id": id, "result": {}, &long: 0}).to_string(),
                    format!("/{}", &long[..1023]),
                ),
                (
                    json!({"id": long, "result": {}}).to_string(),
                    format!("\"{}", &long[..1023]),
                ),
                (
                    format!(r#"{{"{long}":0,"{long}":0}}"#),
                    format!("/{}", &long[..1023]),
                ),
                (
                    json!({"id": id, "result": {}, &tildes: 0}).to_string(),
                    escapes.clone(),
                ),
                (format!(r#"{{"{tildes}":0,"{tildes}":0}}"#), escapes.clone()),
            ] {
                assert_eq!(guest_text(refusal(lane, &frame)), kept, "{lane:?}");
            }
        }
    }

    #[test]
    fn bytes_that_are_not_utf8_refuse() {
        for lane in LANES {
            let mut frame = reply(lane, BOOT, &guest_policy().hex()).into_bytes();
            frame[30] = 0xff;
            let judged = match lane {
                ControlLane::Normal => judge_normal(&frame, &guest_policy()).map(|_| ()),
                ControlLane::Withdrawal => judge_withdrawal(&frame, &guest_policy(), &boot(BOOT)),
            };
            assert_eq!(judged, Err(HelloRefusal::NotUtf8));
        }
    }

    #[test]
    fn text_after_the_reply_refuses() {
        for lane in LANES {
            assert_eq!(
                refusal(lane, "{}x"),
                HelloRefusal::NotJson {
                    detail: owned("refused as JSON at line 1, column 3: trailing characters")
                }
            );
            let valid = reply(lane, BOOT, &guest_policy().hex());
            assert!(matches!(
                refusal(lane, &format!("{valid}{valid}")),
                HelloRefusal::NotJson { .. }
            ));
        }
    }

    #[test]
    fn a_boot_token_round_trips_and_prints_its_digits() {
        let token = boot(BOOT);
        assert_eq!(token.hex(), BOOT);
        assert_eq!(format!("{token:?}"), BOOT);
        assert_ne!(token, boot(OTHER_BOOT));
    }

    #[test]
    fn refusal_messages_put_expected_before_found_and_never_call_a_token_a_digest() {
        assert_eq!(
            BootToken::parse_hex("abc")
                .expect_err("three digits refuse")
                .to_string(),
            "a boot token is 64 lowercase hexadecimal digits, not 3 bytes"
        );
        for (refusal, shown) in [
            (
                HelloRefusal::WrongId {
                    expected: 1,
                    found: owned("\"1\""),
                },
                owned("the hello reply answers id \"1\", not 1"),
            ),
            (
                HelloRefusal::PolicyMismatch {
                    expected: guest_policy(),
                    found: host_policy(),
                },
                format!(
                    "the guest verified policy {}, not the expected {}",
                    host_policy(),
                    guest_policy()
                ),
            ),
            (
                HelloRefusal::BootChanged {
                    expected: boot(BOOT),
                    found: boot(OTHER_BOOT),
                },
                format!(
                    "the withdrawal lane's boot token is {OTHER_BOOT}, not the normal lane's {BOOT}"
                ),
            ),
            (
                HelloRefusal::BadBoot {
                    fault: BootTokenError(DigestError::NotLowercaseHex { at: 9 }),
                },
                owned(
                    "the hello boot token is refused: byte 9 of a boot token is not a lowercase hexadecimal digit",
                ),
            ),
            (
                HelloRefusal::GuestError {
                    code: Some(105),
                    message: owned("refused"),
                    detail: Some(owned(r#"{"stage":"policy_effect"}"#)),
                },
                owned(
                    r#"the guest refused hello with code 105: "refused", detail {"stage":"policy_effect"}"#,
                ),
            ),
            (
                HelloRefusal::GuestError {
                    code: None,
                    message: owned("no"),
                    detail: None,
                },
                owned(
                    "the guest refused hello with no code that is a signed 64-bit integer: \"no\"",
                ),
            ),
        ] {
            assert_eq!(refusal.to_string(), shown);
        }
    }

    fn long_text() -> impl Strategy<Value = String> {
        (
            1000usize..1030,
            prop::sample::select(vec!["é", "漢", "😀"]),
            1usize..8,
        )
            .prop_map(|(ascii, tail, count)| format!("{}{}", "a".repeat(ascii), tail.repeat(count)))
    }

    fn guest_key() -> impl Strategy<Value = String> {
        prop_oneof![
            prop::sample::select(vec![
                "id", "result", "error", "boot", "policy", "code", "message", "detail"
            ])
            .prop_map(str::to_owned),
            "\\PC{0,8}",
            long_text(),
        ]
    }

    fn guest_value() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            prop::sample::select(vec![
                Value::Null,
                json!(1),
                json!(2),
                json!(105),
                json!(1.5),
                json!(u64::MAX),
                json!(BOOT),
                json!(guest_policy().hex()),
            ]),
            any::<i64>().prop_map(Value::from),
            "\\PC{0,8}".prop_map(Value::from),
            long_text().prop_map(Value::from),
        ];
        leaf.prop_recursive(3, 16, 4, |inner| {
            prop::collection::vec((guest_key(), inner), 0..4)
                .prop_map(|entries| Value::Object(entries.into_iter().collect()))
        })
    }

    fn reply_like() -> impl Strategy<Value = Value> {
        let field = |valid: Value| {
            prop_oneof![
                2 => Just(Some(valid)),
                1 => guest_value().prop_map(Some),
                1 => Just(None),
            ]
        };
        let extra = || prop::collection::vec((guest_key(), guest_value()), 0..2);
        (
            prop_oneof![Just(json!(1)), Just(json!(2)), guest_value()],
            field(json!(BOOT)),
            field(json!(guest_policy().hex())),
            prop_oneof![3 => Just(None), 1 => guest_value().prop_map(Some)],
            extra(),
            extra(),
        )
            .prop_map(|(id, boot, policy, error, top, inner)| {
                let mut result: Map<String, Value> = inner.into_iter().collect();
                result.extend(boot.map(|boot| (owned("boot"), boot)));
                result.extend(policy.map(|policy| (owned("policy"), policy)));
                let mut reply: Map<String, Value> = top.into_iter().collect();
                reply.insert(owned("id"), id);
                reply.insert(owned("result"), Value::Object(result));
                reply.extend(error.map(|error| (owned("error"), error)));
                Value::Object(reply)
            })
    }

    proptest! {
        #[test]
        fn no_guest_bytes_or_json_panic_either_judge(
            bytes in proptest::collection::vec(any::<u8>(), 0..64),
            reply in reply_like(),
            other in guest_value(),
        ) {
            for frame in [bytes, reply.to_string().into_bytes(), other.to_string().into_bytes()] {
                let _ = judge_normal(&frame, &guest_policy());
                let _ = judge_withdrawal(&frame, &guest_policy(), &boot(BOOT));
            }
        }

        #[test]
        fn guest_text_is_cut_on_a_character_boundary_wherever_a_refusal_carries_it(
            text in long_text(),
        ) {
            let policy = guest_policy().hex();
            for lane in LANES {
                let id = request_id(lane);
                for (frame, whole) in [
                    (json!({"id": id, "error": {"message": &text}}).to_string(), text.clone()),
                    (json!({"id": id, "error": {"detail": &text}}).to_string(), format!("\"{text}\"")),
                    (json!({"id": id, "result": {}, &text: 0}).to_string(), format!("/{text}")),
                    (
                        json!({"id": id, "result": {"boot": BOOT, "policy": policy, &text: 0}}).to_string(),
                        format!("/result/{text}"),
                    ),
                    (json!({"id": &text, "result": {}}).to_string(), format!("\"{text}\"")),
                    (format!(r#"{{"{text}":0,"{text}":0}}"#), format!("/{text}")),
                ] {
                    let kept = guest_text(refusal(lane, &frame));
                    prop_assert!(whole.starts_with(&kept));
                    prop_assert!(kept.len() <= 1024);
                    prop_assert!(whole.len() <= 1024 || kept.len() > 1020);
                    prop_assert!(whole.len() > 1024 || kept == whole);
                }
            }
        }
    }
}
