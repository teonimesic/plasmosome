use serde::{Deserialize, Serialize, Serializer};
use serde_json::value::RawValue;

use super::cut;
use crate::wire::{ObjectOnly, object_serde};

const REFUSED: i64 = 105;
const REFUSED_FROM: &str = "guest_request";
const REFUSED_TO: &str = "refused";

/// A request's `id`, kept as the exact JSON text the caller sent, so a reply
/// echoes it unchanged (spec 001 §1). Two ids are equal when their texts are
/// equal: `1` and `1.0` differ.
#[derive(Debug, Clone)]
pub struct RequestId(Box<RawValue>);

impl RequestId {
    /// The id `id`, written in decimal.
    pub fn number(id: u64) -> RequestId {
        RequestId(raw(id.to_string()))
    }

    /// The `null` id, under which a line whose own id cannot be trusted is
    /// answered.
    pub fn null() -> RequestId {
        RequestId(raw("null".to_owned()))
    }

    /// The id as an unsigned 64-bit integer, when its text is exactly that
    /// integer in decimal. `-0`, `1.0`, `1e0`, `"1"` and anything past
    /// `u64::MAX` give `None`.
    pub fn as_u64(&self) -> Option<u64> {
        self.0.get().parse().ok()
    }

    /// The id's JSON text, exactly as it was received or built.
    pub fn as_json(&self) -> &str {
        self.0.get()
    }
}

impl PartialEq for RequestId {
    fn eq(&self, other: &RequestId) -> bool {
        self.as_json() == other.as_json()
    }
}

impl Eq for RequestId {}

impl Serialize for RequestId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

fn raw(text: String) -> Box<RawValue> {
    RawValue::from_string(text).expect("a decimal number or null is one JSON value")
}

/// A protocol failure of spec 001 §1, which reuses the JSON-RPC reserve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolCode {
    /// -32700: the line is not UTF-8 or not one JSON value.
    ParseError,
    /// -32600: the line is JSON but not a request this lane can read.
    InvalidRequest,
    /// -32601: no method of that name on this lane.
    MethodNotFound,
    /// -32602: the method's params do not decode.
    InvalidParams,
    /// -32603: the responder failed, not the request.
    InternalError,
}

impl ProtocolCode {
    /// The code's integer on the wire.
    pub fn code(self) -> i64 {
        match self {
            ProtocolCode::ParseError => -32700,
            ProtocolCode::InvalidRequest => -32600,
            ProtocolCode::MethodNotFound => -32601,
            ProtocolCode::InvalidParams => -32602,
            ProtocolCode::InternalError => -32603,
        }
    }
}

/// The `detail` of a 105 refusal a bridge responder sends.
///
/// In JSON it is an object whose `kind` names the variant, for example
/// `{"kind": "rejected", "reason": "..."}`. Decoding refuses an array, an
/// unknown kind, and a missing, repeated or unknown field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    remote = "Self",
    tag = "kind",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RefusalDetail {
    /// The guest refused hello, observe, install, activate, drain, remove or
    /// shutdown (spec 001 §4.2). `reason` is for people: a caller never
    /// branches on it.
    Rejected { reason: String },
}

object_serde!(RefusalDetail through RefusalDetail);

/// The error object of a failed bridge request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    /// A protocol failure, written as exactly `{code, message}`.
    Protocol { code: ProtocolCode, message: String },
    /// A refusal, written as exactly
    /// `{code: 105, message, from: "guest_request", to: "refused", detail}`.
    Refused {
        message: String,
        detail: RefusalDetail,
    },
}

/// A request a lane's decoder accepted: the id its reply must echo and the
/// typed call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundRequest<C> {
    pub id: RequestId,
    pub call: C,
}

/// A request line the responder refuses: answer it with
/// `encode_failure(&id, &error)` and read the next line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRefusal {
    /// The request's own id, or `null` when the line held no id that can be
    /// trusted.
    pub id: RequestId,
    pub error: BridgeError,
}

#[derive(Serialize)]
struct RequestLine<'a> {
    id: u64,
    method: &'a str,
    params: &'a RawValue,
}

#[derive(Serialize)]
struct SuccessLine<'a, R> {
    id: &'a RequestId,
    result: &'a R,
}

#[derive(Serialize)]
struct FailureLine<'a, E> {
    id: &'a RequestId,
    error: E,
}

#[derive(Serialize)]
struct ProtocolObject<'a> {
    code: i64,
    message: &'a str,
}

#[derive(Serialize)]
struct RefusalObject<'a> {
    code: i64,
    message: &'a str,
    from: &'static str,
    to: &'static str,
    detail: &'a RefusalDetail,
}

#[derive(Deserialize)]
#[serde(
    deny_unknown_fields,
    expecting = "a request object with exactly id, method and params"
)]
struct Envelope {
    id: Box<RawValue>,
    method: String,
    params: Box<RawValue>,
}

/// The compact request line `{"id":id,"method":method,"params":{...}}`, with
/// its keys in that order and no newline: the writer adds one.
///
/// # Panics
///
/// When `params` fails to encode or encodes as anything but a JSON object.
/// Both are caller bugs: spec 001 makes params always an object.
pub fn encode_request<P: Serialize>(id: u64, method: &str, params: &P) -> Vec<u8> {
    let params = serde_json::value::to_raw_value(params).expect("request params encode as JSON");
    assert!(
        params.get().starts_with('{'),
        "request params must encode as a JSON object, not {}",
        cut(params.get())
    );
    encode(&RequestLine {
        id,
        method,
        params: &params,
    })
}

/// The compact success reply `{"id":id,"result":...}`, echoing `id`'s text
/// unchanged, with no newline.
///
/// # Panics
///
/// When `result` fails to encode, which is a caller bug.
pub fn encode_success<R: Serialize>(id: &RequestId, result: &R) -> Vec<u8> {
    encode(&SuccessLine { id, result })
}

/// The compact failure reply `{"id":id,"error":{...}}`, echoing `id`'s text
/// unchanged, with the error object exactly as [`BridgeError`] describes and
/// no newline.
pub fn encode_failure(id: &RequestId, error: &BridgeError) -> Vec<u8> {
    match error {
        BridgeError::Protocol { code, message } => encode(&FailureLine {
            id,
            error: ProtocolObject {
                code: code.code(),
                message,
            },
        }),
        BridgeError::Refused { message, detail } => encode(&FailureLine {
            id,
            error: RefusalObject {
                code: REFUSED,
                message,
                from: REFUSED_FROM,
                to: REFUSED_TO,
                detail,
            },
        }),
    }
}

/// Splits one request line, passed without its newline, into its id, its
/// method name and its params object, still as JSON text.
///
/// A line that is not UTF-8 or not exactly one JSON value is -32700 under a
/// `null` id. A JSON value that is not an object with exactly `id`, `method`
/// and `params`, each once and `method` a string, is -32600 under a `null`
/// id, since its id cannot be trusted. Params that are not a JSON object are
/// -32600 under the request's id. The id may be any JSON value and is kept
/// as its exact text. A refusal's message keeps at most 1,024 bytes of the
/// decoder's text. The caller's reader bounds the line's length.
pub fn split_request(line: &[u8]) -> Result<(RequestId, String, Box<RawValue>), RequestRefusal> {
    let Ok(text) = std::str::from_utf8(line) else {
        return Err(refused(
            RequestId::null(),
            ProtocolCode::ParseError,
            "the request line is not UTF-8",
        ));
    };
    if let Err(error) = serde_json::from_str::<&RawValue>(text) {
        let message = format!("the request line is not one JSON value: {error}");
        return Err(refused(
            RequestId::null(),
            ProtocolCode::ParseError,
            &message,
        ));
    }
    let mut reader = serde_json::Deserializer::from_str(text);
    let envelope = Envelope::deserialize(ObjectOnly::new(&mut reader)).map_err(|error| {
        let message = format!("the request is not exactly {{id, method, params}}: {error}");
        refused(RequestId::null(), ProtocolCode::InvalidRequest, &message)
    })?;
    let id = RequestId(envelope.id);
    if !envelope.params.get().starts_with('{') {
        let message = "the request params are not a JSON object";
        return Err(refused(id, ProtocolCode::InvalidRequest, message));
    }
    Ok((id, envelope.method, envelope.params))
}

fn refused(id: RequestId, code: ProtocolCode, message: &str) -> RequestRefusal {
    RequestRefusal {
        id,
        error: BridgeError::Protocol {
            code,
            message: cut(message),
        },
    }
}

fn encode(line: &impl Serialize) -> Vec<u8> {
    serde_json::to_vec(line).expect("a bridge line encodes as JSON")
}
