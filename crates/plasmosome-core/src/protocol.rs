use serde::de::{Error as _, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::value::RawValue;
use serde_json::{Map, Value};

use crate::state::{CellId, CellStatus, GenomeName, MockMode};

/// One control request as it arrives on the wire.
///
/// `id` is the token the client sent, kept as it arrived: an id is any JSON
/// value, and a number that does not fit an `f64` must still come back the way
/// it was written. `params` is never omitted: a verb that takes nothing is
/// still sent an empty object, and a line without it is an invalid request
/// rather than an empty one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: Box<RawValue>,
    pub method: String,
    pub params: Map<String, Value>,
}

impl PartialEq for Request {
    fn eq(&self, other: &Request) -> bool {
        self.id.get() == other.id.get()
            && self.method == other.method
            && self.params == other.params
    }
}

/// One control reply, carrying a result or an error and never both.
///
/// `id` is the requesting line's id, echoed unchanged. Reading a reply that
/// carries both a result and an error, or neither, fails: a client that
/// accepted one would be reading a protocol this one does not define.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
#[expect(
    clippy::large_enum_variant,
    reason = "boxing the error would change the variant's and Handler::handle's signatures and force a Box::new at the fifteen construction sites, for a type meant to read as the protocol table it mirrors"
)]
pub enum Response {
    Success { id: Box<RawValue>, result: Value },
    Failure { id: Box<RawValue>, error: WireError },
}

impl PartialEq for Response {
    fn eq(&self, other: &Response) -> bool {
        match (self, other) {
            (
                Response::Success { id, result },
                Response::Success {
                    id: other_id,
                    result: other_result,
                },
            ) => id.get() == other_id.get() && result == other_result,
            (
                Response::Failure { id, error },
                Response::Failure {
                    id: other_id,
                    error: other_error,
                },
            ) => id.get() == other_id.get() && error == other_error,
            _ => false,
        }
    }
}

impl<'de> Deserialize<'de> for Response {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Response, D::Error> {
        deserializer.deserialize_map(ResponseVisitor)
    }
}

struct ResponseVisitor;

impl<'de> Visitor<'de> for ResponseVisitor {
    type Value = Response;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a control reply: an id, and either a result or an error")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Response, A::Error> {
        let mut id: Option<Box<RawValue>> = None;
        let mut result: Option<Value> = None;
        let mut error: Option<WireError> = None;
        while let Some(key) = entries.next_key::<String>()? {
            match key.as_str() {
                "id" => {
                    if id.is_some() {
                        return Err(A::Error::duplicate_field("id"));
                    }
                    id = Some(entries.next_value()?);
                }
                "result" => {
                    if result.is_some() {
                        return Err(A::Error::duplicate_field("result"));
                    }
                    result = Some(entries.next_value()?);
                }
                "error" => {
                    if error.is_some() {
                        return Err(A::Error::duplicate_field("error"));
                    }
                    error = Some(entries.next_value()?);
                }
                _ => {
                    entries.next_value::<IgnoredAny>()?;
                }
            }
        }
        let id = id.ok_or_else(|| A::Error::missing_field("id"))?;
        match (result, error) {
            (Some(result), None) => Ok(Response::Success { id, result }),
            (None, Some(error)) => Ok(Response::Failure { id, error }),
            (Some(_), Some(_)) => Err(A::Error::custom(
                "a control reply carries a result or an error, this one carries both",
            )),
            (None, None) => Err(A::Error::custom(
                "a control reply carries a result or an error, this one carries neither",
            )),
        }
    }
}

/// The id a reply carries when the line it answers had no envelope to take one
/// from: the literal `null` token.
pub fn null_id() -> Box<RawValue> {
    RawValue::from_string("null".to_string()).expect("`null` is a JSON value")
}

/// The closed set of control protocol error codes: the four JSON-RPC reserve
/// codes and the application codes 100-110.
///
/// An integer outside the set does not deserialize. A client that reads a code
/// it does not know is reading a protocol it does not speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    AmbiguousTarget,
    UnknownTarget,
    AlreadyExists,
    UnresolvedRequirement,
    MockModeConflict,
    IllegalState,
    DrainTimeout,
    NotRunning,
    ManifestInvalid,
    WideningForbidden,
    AttestationRequired,
}

impl ErrorCode {
    /// The integer this code travels as.
    pub fn as_i64(self) -> i64 {
        match self {
            ErrorCode::ParseError => -32700,
            ErrorCode::InvalidRequest => -32600,
            ErrorCode::MethodNotFound => -32601,
            ErrorCode::InvalidParams => -32602,
            ErrorCode::AmbiguousTarget => 100,
            ErrorCode::UnknownTarget => 101,
            ErrorCode::AlreadyExists => 102,
            ErrorCode::UnresolvedRequirement => 103,
            ErrorCode::MockModeConflict => 104,
            ErrorCode::IllegalState => 105,
            ErrorCode::DrainTimeout => 106,
            ErrorCode::NotRunning => 107,
            ErrorCode::ManifestInvalid => 108,
            ErrorCode::WideningForbidden => 109,
            ErrorCode::AttestationRequired => 110,
        }
    }

    /// The code an integer names, or `None` when the integer is outside the set.
    pub fn from_i64(code: i64) -> Option<ErrorCode> {
        match code {
            -32700 => Some(ErrorCode::ParseError),
            -32600 => Some(ErrorCode::InvalidRequest),
            -32601 => Some(ErrorCode::MethodNotFound),
            -32602 => Some(ErrorCode::InvalidParams),
            100 => Some(ErrorCode::AmbiguousTarget),
            101 => Some(ErrorCode::UnknownTarget),
            102 => Some(ErrorCode::AlreadyExists),
            103 => Some(ErrorCode::UnresolvedRequirement),
            104 => Some(ErrorCode::MockModeConflict),
            105 => Some(ErrorCode::IllegalState),
            106 => Some(ErrorCode::DrainTimeout),
            107 => Some(ErrorCode::NotRunning),
            108 => Some(ErrorCode::ManifestInvalid),
            109 => Some(ErrorCode::WideningForbidden),
            110 => Some(ErrorCode::AttestationRequired),
            _ => None,
        }
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(self.as_i64())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ErrorCode, D::Error> {
        let code = i64::deserialize(deserializer)?;
        ErrorCode::from_i64(code).ok_or_else(|| {
            serde::de::Error::custom(format!("{code} is not a control protocol error code"))
        })
    }
}

/// A structured control error: a closed `code`, a human `message`, and the
/// fields the protocol table names for that code.
///
/// Build one through the constructor for its code — that is the only way to
/// build one, so a code cannot travel without the fields it owes. Branch on
/// `code` and the fields; the message is prose and may change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireError {
    code: ErrorCode,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    candidates: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plasmid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    modes: Option<Vec<MockMode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plasmids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolutions: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deadline_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    verb: Option<String>,
}

impl WireError {
    fn bare(code: ErrorCode, message: String) -> WireError {
        WireError {
            code,
            message,
            candidates: None,
            target: None,
            capability: None,
            plasmid: None,
            node: None,
            modes: None,
            plasmids: None,
            resolutions: None,
            from: None,
            to: None,
            handle: None,
            deadline_ms: None,
            detail: None,
            path: None,
            verb: None,
        }
    }

    /// The code this error carries. Branch on it, never on the message.
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// Code 100: the request named a target that several objects answer to.
    pub fn ambiguous_target(candidates: Vec<String>) -> WireError {
        let mut error = WireError::bare(
            ErrorCode::AmbiguousTarget,
            format!(
                "the target is ambiguous: {} candidates match",
                candidates.len()
            ),
        );
        error.candidates = Some(candidates);
        error
    }

    /// Code 101: the request named a target that does not exist.
    pub fn unknown_target(target: String) -> WireError {
        let mut error = WireError::bare(
            ErrorCode::UnknownTarget,
            format!("`{target}` is not a target this controller knows"),
        );
        error.target = Some(target);
        error
    }

    /// Code 102: the target the request would create is already there.
    pub fn already_exists(target: String) -> WireError {
        let mut error = WireError::bare(
            ErrorCode::AlreadyExists,
            format!("`{target}` already exists"),
        );
        error.target = Some(target);
        error
    }

    /// Code 103: a plasmid requires a capability nothing in the set provides.
    pub fn unresolved_requirement(capability: String, plasmid: String) -> WireError {
        let mut error = WireError::bare(
            ErrorCode::UnresolvedRequirement,
            format!("`{plasmid}` requires `{capability}`, which nothing provides"),
        );
        error.capability = Some(capability);
        error.plasmid = Some(plasmid);
        error
    }

    /// Code 104: one node is declared at two mock modes, with no winner.
    pub fn mock_mode_conflict(
        node: String,
        modes: Vec<MockMode>,
        plasmids: Vec<String>,
        resolutions: Vec<String>,
    ) -> WireError {
        let mut error = WireError::bare(
            ErrorCode::MockModeConflict,
            format!(
                "`{node}` is declared at {} mock modes by {} plasmids",
                modes.len(),
                plasmids.len()
            ),
        );
        error.node = Some(node);
        error.modes = Some(modes);
        error.plasmids = Some(plasmids);
        error.resolutions = Some(resolutions);
        error
    }

    /// Code 105: the lifecycle forbids the transition the request asked for.
    pub fn illegal_state(from: String, to: String) -> WireError {
        let mut error = WireError::bare(
            ErrorCode::IllegalState,
            format!("`{from}` cannot become `{to}`"),
        );
        error.from = Some(from);
        error.to = Some(to);
        error
    }

    /// Code 106: a drain ran past its deadline.
    pub fn drain_timeout(handle: String, deadline_ms: u64) -> WireError {
        let mut error = WireError::bare(
            ErrorCode::DrainTimeout,
            format!("`{handle}` did not drain within {deadline_ms}ms"),
        );
        error.handle = Some(handle);
        error.deadline_ms = Some(deadline_ms);
        error
    }

    /// Code 107: the named instance is not up, and no verb here starts one.
    pub fn not_running(target: String) -> WireError {
        let mut error =
            WireError::bare(ErrorCode::NotRunning, format!("`{target}` is not running"));
        error.target = Some(target);
        error
    }

    /// Code 108: a manifest failed the frozen grammar.
    pub fn manifest_invalid(detail: String, path: String) -> WireError {
        let mut error = WireError::bare(
            ErrorCode::ManifestInvalid,
            format!("`{path}` is not a valid manifest: {detail}"),
        );
        error.detail = Some(detail);
        error.path = Some(path);
        error
    }

    /// Code 109: the request would widen an existing grant.
    pub fn widening_forbidden(plasmid: String) -> WireError {
        let mut error = WireError::bare(
            ErrorCode::WideningForbidden,
            format!("`{plasmid}` may not widen a grant it already holds"),
        );
        error.plasmid = Some(plasmid);
        error
    }

    /// Code 110: the verb needs a host-side attestation the request did not carry.
    pub fn attestation_required(verb: String) -> WireError {
        let mut error = WireError::bare(
            ErrorCode::AttestationRequired,
            format!("`{verb}` needs a host-side attestation"),
        );
        error.verb = Some(verb);
        error
    }

    /// Reserve code -32700: the line was not JSON.
    pub fn parse_error() -> WireError {
        WireError::bare(ErrorCode::ParseError, "the line is not JSON".to_string())
    }

    /// Reserve code -32600: the line was JSON but not a control envelope.
    pub fn invalid_request(message: String) -> WireError {
        WireError::bare(ErrorCode::InvalidRequest, message)
    }

    /// Reserve code -32601: no served verb answers to this method.
    pub fn method_not_found(method: &str) -> WireError {
        WireError::bare(
            ErrorCode::MethodNotFound,
            format!("`{method}` is not a method this controller serves"),
        )
    }

    /// Reserve code -32602: the method is served, its params did not parse.
    pub fn invalid_params(message: String) -> WireError {
        WireError::bare(ErrorCode::InvalidParams, message)
    }
}

/// What a named instance is doing, as reported by whoever probed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstanceState {
    Running,
    Stopped,
    Unreachable,
}

/// The result of `plasmosome.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusResult {
    pub name: String,
    pub state: InstanceState,
    pub ready: bool,
    pub controller: ControllerInfo,
    pub cells: Vec<CellStatusEntry>,
}

/// The controller's own numbers inside a status result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerInfo {
    pub uptime_ms: u64,
    pub ledger_generation: u64,
}

/// One cell inside a status result.
///
/// `plasmids` carries the mock-mode labels, one per attached plasmid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellStatusEntry {
    pub id: CellId,
    pub genome: Option<GenomeName>,
    pub state: CellStatus,
    pub plasmids: Vec<String>,
}

/// The params `plasmosome.status` accepts. An absent `name` means the instance
/// the request reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusParams {
    pub name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PlasmidRecord;

    fn raw_id(token: &str) -> Box<RawValue> {
        RawValue::from_string(token.to_string()).expect("the test id is a JSON value")
    }

    fn wire_fields(error: &WireError) -> Vec<String> {
        let value = serde_json::to_value(error).expect("a wire error serializes");
        let Value::Object(map) = value else {
            panic!("a wire error serializes as a JSON object, got {error:?}");
        };
        let mut names: Vec<String> = map.keys().cloned().collect();
        names.sort();
        names
    }

    fn expect_code_and_fields(error: WireError, code: i64, fields: &[&str]) {
        let value = serde_json::to_value(&error).expect("a wire error serializes");
        assert_eq!(
            value.get("code"),
            Some(&Value::from(code)),
            "the code on the wire for {value}"
        );
        let mut expected: Vec<String> = ["code", "message"]
            .iter()
            .chain(fields.iter())
            .map(|name| (*name).to_string())
            .collect();
        expected.sort();
        assert_eq!(
            wire_fields(&error),
            expected,
            "the structured fields code {code} carries, on the wire as {value}"
        );
    }

    #[test]
    fn every_application_error_serializes_its_code_and_structured_fields() {
        expect_code_and_fields(
            WireError::ambiguous_target(vec!["cell-1".to_string(), "cell-2".to_string()]),
            100,
            &["candidates"],
        );
        expect_code_and_fields(
            WireError::unknown_target("plasmosome work".to_string()),
            101,
            &["target"],
        );
        expect_code_and_fields(
            WireError::already_exists("plasmosome work".to_string()),
            102,
            &["target"],
        );
        expect_code_and_fields(
            WireError::unresolved_requirement("net.http".to_string(), "github-pr".to_string()),
            103,
            &["capability", "plasmid"],
        );
        expect_code_and_fields(
            WireError::mock_mode_conflict(
                "mock-github".to_string(),
                vec![MockMode::Simulate, MockMode::Passthrough],
                vec!["github-pr".to_string(), "mock-github".to_string()],
                vec!["force_simulate".to_string()],
            ),
            104,
            &["node", "modes", "plasmids", "resolutions"],
        );
        expect_code_and_fields(
            WireError::illegal_state("draining".to_string(), "ready".to_string()),
            105,
            &["from", "to"],
        );
        expect_code_and_fields(
            WireError::drain_timeout("handle-7".to_string(), 5000),
            106,
            &["handle", "deadline_ms"],
        );
        expect_code_and_fields(
            WireError::not_running("plasmosome work".to_string()),
            107,
            &["target"],
        );
        expect_code_and_fields(
            WireError::manifest_invalid(
                "delivery must be non-empty".to_string(),
                "plasmids/github-pr.toml".to_string(),
            ),
            108,
            &["detail", "path"],
        );
        expect_code_and_fields(
            WireError::widening_forbidden("github-pr".to_string()),
            109,
            &["plasmid"],
        );
        expect_code_and_fields(
            WireError::attestation_required("cell.exec".to_string()),
            110,
            &["verb"],
        );
    }

    #[test]
    fn an_unknown_error_code_does_not_deserialize() {
        for code in ["111", "99", "0", "-32603"] {
            let outcome = serde_json::from_str::<ErrorCode>(code);
            assert!(
                outcome.is_err(),
                "code {code} is outside the closed table and must not deserialize, got {outcome:?}"
            );
        }
        let smuggled = "{\"code\":111,\"message\":\"invented\"}";
        assert!(
            serde_json::from_str::<WireError>(smuggled).is_err(),
            "a wire error carrying an invented code must not deserialize: {smuggled}"
        );
        assert_eq!(
            serde_json::from_str::<ErrorCode>("101").expect("101 is in the table"),
            ErrorCode::UnknownTarget
        );
    }

    #[test]
    fn a_response_carries_result_or_error_never_both() {
        let success = Response::Success {
            id: raw_id("7"),
            result: serde_json::from_str("{\"ready\":true}").expect("the result parses"),
        };
        let encoded = serde_json::to_value(&success).expect("a success response serializes");
        assert_eq!(
            encoded,
            serde_json::from_str::<Value>("{\"id\":7,\"result\":{\"ready\":true}}")
                .expect("the expected success shape parses"),
            "the success envelope on the wire"
        );
        assert_eq!(
            serde_json::from_value::<Response>(encoded).expect("a success response round-trips"),
            success
        );

        let failure = Response::Failure {
            id: raw_id("7"),
            error: WireError::unknown_target("plasmosome work".to_string()),
        };
        let encoded = serde_json::to_value(&failure).expect("a failure response serializes");
        assert_eq!(
            encoded.get("result"),
            None,
            "a failure envelope carries no result: {encoded}"
        );
        assert_eq!(
            encoded
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_i64),
            Some(101),
            "the failure envelope's code: {encoded}"
        );
        assert_eq!(
            serde_json::from_value::<Response>(encoded).expect("a failure response round-trips"),
            failure
        );

        let both =
            "{\"id\":7,\"result\":{\"ready\":true},\"error\":{\"code\":101,\"message\":\"gone\"}}";
        assert!(
            serde_json::from_str::<Response>(both).is_err(),
            "a reply carrying a result and an error at once is not a reply this protocol defines: {both}"
        );
        let neither = "{\"id\":7}";
        assert!(
            serde_json::from_str::<Response>(neither).is_err(),
            "a reply carrying neither a result nor an error is not a reply this protocol defines: {neither}"
        );
        let only_result = "{\"id\":7,\"result\":{\"ready\":true}}";
        assert!(
            serde_json::from_str::<Response>(only_result).is_ok(),
            "a reply carrying only a result reads as a success: {only_result}"
        );
        let only_error = "{\"id\":7,\"error\":{\"code\":101,\"message\":\"gone\"}}";
        assert!(
            serde_json::from_str::<Response>(only_error).is_ok(),
            "a reply carrying only an error reads as a failure: {only_error}"
        );
    }

    #[test]
    fn the_status_result_serializes_the_frozen_shape() {
        let labels = |plasmids: &[PlasmidRecord]| -> Vec<String> {
            plasmids.iter().map(PlasmidRecord::list_label).collect()
        };
        let result = StatusResult {
            name: "work".to_string(),
            state: InstanceState::Running,
            ready: true,
            controller: ControllerInfo {
                uptime_ms: 9142,
                ledger_generation: 4,
            },
            cells: vec![
                CellStatusEntry {
                    id: CellId::from("cell-1"),
                    genome: Some(GenomeName::from("researcher")),
                    state: CellStatus::Ready,
                    plasmids: labels(&[
                        PlasmidRecord {
                            plasmid: "github-pr".to_string(),
                            mock: MockMode::Simulate,
                        },
                        PlasmidRecord {
                            plasmid: "model-provider".to_string(),
                            mock: MockMode::Passthrough,
                        },
                    ]),
                },
                CellStatusEntry {
                    id: CellId::from("cell-2"),
                    genome: Some(GenomeName::from("researcher")),
                    state: CellStatus::Draining,
                    plasmids: Vec::new(),
                },
            ],
        };
        let frozen = serde_json::from_str::<Value>(
            "{\"name\": \"work\", \"state\": \"running\", \"ready\": true,
              \"controller\": {\"uptime_ms\": 9142, \"ledger_generation\": 4},
              \"cells\": [
                {\"id\": \"cell-1\", \"genome\": \"researcher\", \"state\": \"ready\",
                 \"plasmids\": [\"github-pr [mock:simulate]\", \"model-provider [real]\"]},
                {\"id\": \"cell-2\", \"genome\": \"researcher\", \"state\": \"draining\",
                 \"plasmids\": []}
              ]}",
        )
        .expect("the frozen status shape parses");
        let encoded = serde_json::to_value(&result).expect("a status result serializes");
        assert_eq!(encoded, frozen, "the status result on the wire");
        assert_eq!(
            serde_json::from_value::<StatusResult>(encoded).expect("a status result round-trips"),
            result
        );
    }
}
