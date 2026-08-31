#![expect(
    clippy::result_large_err,
    reason = "a WireError carries the whole protocol table's structured fields by value; every error path in this module is a reply about to be written, not a hot loop"
)]

use std::collections::BTreeMap;
use std::io::{BufRead, Read, Write};
use std::panic::AssertUnwindSafe;
use std::time::Instant;

use serde_json::value::RawValue;
use serde_json::{Map, Value};

use crate::protocol::{
    CellStatusEntry, ControllerInfo, ErrorCode, InstanceState, Request, Response, StatusParams,
    StatusResult, WireError, null_id,
};
use crate::state::{CellRecord, ControllerState, InstanceName, PlasmidRecord};

/// The most bytes one request line may carry before its terminating newline.
///
/// A connection that sends more than this without a newline is answered
/// `-32600` once and then closed; nothing past the cap is read.
pub const MAX_LINE_BYTES: usize = 1 << 20;

/// What serves one verb.
///
/// Return the verb's result as serde data, or the error for the code the
/// protocol table names. A method the implementor does not serve is
/// `WireError::method_not_found`; params that do not parse are
/// `WireError::invalid_params`.
///
/// The connection loop owns `-32700` and `-32600`: only it sees the frame, so
/// only it may answer about one. An implementor must never return either; one
/// that does is answered `-32603` in its place and keeps serving. An
/// implementor that panics is answered `-32603`, and the connection ends.
pub trait Handler {
    fn handle(&mut self, method: &str, params: &Map<String, Value>) -> Result<Value, WireError>;
}

/// Serve one ndjson connection: read a request per line, write a reply per
/// line in the same order, flushing each one.
///
/// A line that fails to parse — including one that is not UTF-8 — is answered
/// and the conversation continues. A line longer than `MAX_LINE_BYTES` is
/// answered `-32600` under a `null` id and the connection then closes. A
/// handler panic is answered `-32603`, and the panic then resumes on this
/// thread whether or not that reply reached the client. Returns at end of
/// input, or with the first read or write failure.
pub fn serve_connection<R: BufRead, W: Write, H: Handler>(
    mut reader: R,
    mut writer: W,
    handler: &mut H,
) -> std::io::Result<()> {
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        buffer.clear();
        let taken = (&mut reader)
            .take(MAX_LINE_BYTES as u64 + 1)
            .read_until(b'\n', &mut buffer)?;
        if taken == 0 {
            return Ok(());
        }
        if !buffer.ends_with(b"\n") && buffer.len() > MAX_LINE_BYTES {
            write_reply(
                &mut writer,
                &Response::Failure {
                    id: null_id(),
                    error: WireError::line_too_long(MAX_LINE_BYTES),
                },
            )?;
            return Ok(());
        }
        if buffer.last() == Some(&b'\n') {
            buffer.pop();
        }
        if buffer.last() == Some(&b'\r') {
            buffer.pop();
        }
        let Ok(line) = std::str::from_utf8(&buffer) else {
            write_reply(
                &mut writer,
                &Response::Failure {
                    id: null_id(),
                    error: WireError::parse_error(),
                },
            )?;
            continue;
        };
        let answered = answer(line, handler);
        let written = write_reply(&mut writer, &answered.response);
        if let Some(payload) = answered.panic {
            std::panic::resume_unwind(payload);
        }
        written?;
    }
}

fn write_reply<W: Write>(writer: &mut W, response: &Response) -> std::io::Result<()> {
    let encoded = serde_json::to_string(response).map_err(std::io::Error::other)?;
    writer.write_all(encoded.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

struct Answered {
    response: Response,
    panic: Option<Box<dyn std::any::Any + Send>>,
}

impl Answered {
    fn reply(response: Response) -> Answered {
        Answered {
            response,
            panic: None,
        }
    }
}

fn guard_loop_owned_codes(error: WireError) -> WireError {
    match error.code() {
        ErrorCode::ParseError | ErrorCode::InvalidRequest => WireError::internal(),
        _ => error,
    }
}

fn answer<H: Handler>(line: &str, handler: &mut H) -> Answered {
    if serde_json::from_str::<&RawValue>(line).is_err() {
        return Answered::reply(Response::Failure {
            id: null_id(),
            error: WireError::parse_error(),
        });
    }
    let Ok(fields) = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(line) else {
        return Answered::reply(Response::Failure {
            id: null_id(),
            error: WireError::invalid_request(
                "a control request is a JSON object with `id`, `method` and `params`".to_string(),
            ),
        });
    };
    let id = fields.get("id").cloned().unwrap_or_else(null_id);
    let request = match serde_json::from_str::<Request>(line) {
        Ok(request) => request,
        Err(error) => {
            return Answered::reply(Response::Failure {
                id,
                error: WireError::invalid_request(error.to_string()),
            });
        }
    };
    match std::panic::catch_unwind(AssertUnwindSafe(|| {
        handler.handle(&request.method, &request.params)
    })) {
        Ok(Ok(result)) => Answered::reply(Response::Success {
            id: request.id,
            result,
        }),
        Ok(Err(error)) => Answered::reply(Response::Failure {
            id: request.id,
            error: guard_loop_owned_codes(error),
        }),
        Err(payload) => Answered {
            response: Response::Failure {
                id: request.id,
                error: WireError::internal(),
            },
            panic: Some(payload),
        },
    }
}

const STATUS_METHOD: &str = "plasmosome.status";

/// The controller of one named instance, answering `plasmosome.status`.
pub struct Controller {
    name: InstanceName,
    state: ControllerState,
    started: Instant,
    ledger_generation: u64,
}

impl Controller {
    /// Build the controller for `name`. `ledger_generation` is the generation
    /// the caller's ledger is at; this type does not track one.
    pub fn new(name: InstanceName, state: ControllerState, ledger_generation: u64) -> Controller {
        Controller {
            name,
            state,
            started: Instant::now(),
            ledger_generation,
        }
    }

    fn status(&self) -> StatusResult {
        let cells = self
            .state
            .instances
            .iter()
            .find(|instance| instance.name == self.name)
            .map(|instance| instance.cells.iter().map(cell_entry).collect())
            .unwrap_or_default();
        StatusResult {
            name: self.name.to_string(),
            state: InstanceState::Running,
            ready: true,
            controller: ControllerInfo {
                uptime_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
                ledger_generation: self.ledger_generation,
            },
            cells,
        }
    }
}

fn cell_entry(cell: &CellRecord) -> CellStatusEntry {
    CellStatusEntry {
        id: cell.id.clone(),
        genome: cell.genome.clone(),
        state: cell.status,
        plasmids: cell
            .plasmids
            .iter()
            .map(PlasmidRecord::list_label)
            .collect(),
    }
}

impl Handler for Controller {
    fn handle(&mut self, method: &str, params: &Map<String, Value>) -> Result<Value, WireError> {
        if method != STATUS_METHOD {
            return Err(WireError::method_not_found(method));
        }
        let params = serde_json::from_value::<StatusParams>(Value::Object(params.clone()))
            .map_err(|error| WireError::invalid_params(error.to_string()))?;
        if let Some(name) = params.name {
            let name = InstanceName::parse(&name)
                .map_err(|error| WireError::invalid_params(error.to_string()))?;
            if name != self.name {
                return Err(WireError::unknown_target(format!("plasmosome {name}")));
            }
        }
        Ok(serde_json::to_value(self.status())
            .expect("a status result is serde data with string keys"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ErrorCode;
    use crate::state::{CellId, CellStatus, GenomeName, InstanceRecord, MockMode};
    use std::io::{BufReader, Read};
    use std::net::Shutdown;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::panic::AssertUnwindSafe;
    use std::thread;

    struct Echo;

    impl Handler for Echo {
        fn handle(
            &mut self,
            method: &str,
            params: &Map<String, Value>,
        ) -> Result<Value, WireError> {
            if method != "echo" {
                return Err(WireError::method_not_found(method));
            }
            Ok(Value::Object(params.clone()))
        }
    }

    struct Panicking;

    impl Handler for Panicking {
        fn handle(
            &mut self,
            method: &str,
            params: &Map<String, Value>,
        ) -> Result<Value, WireError> {
            if method == "boom" {
                panic!("the handler for `boom` fails while answering");
            }
            Ok(Value::Object(params.clone()))
        }
    }

    struct BrokenPipe;

    impl Write for BrokenPipe {
        fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "the client hung up",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Lying;

    impl Handler for Lying {
        fn handle(
            &mut self,
            method: &str,
            params: &Map<String, Value>,
        ) -> Result<Value, WireError> {
            if method == "lie" {
                return Err(WireError::parse_error());
            }
            Ok(Value::Object(params.clone()))
        }
    }

    fn reply_lines_of<H: Handler>(script: &[u8], expected: usize, handler: &mut H) -> Vec<String> {
        let mut written: Vec<u8> = Vec::new();
        serve_connection(script, &mut written, handler)
            .expect("the loop serves the scripted lines to a writer that cannot fail");
        let replies = String::from_utf8(written).expect("every reply is utf-8");
        let lines: Vec<String> = replies.lines().map(str::to_string).collect();
        assert_eq!(
            lines.len(),
            expected,
            "the script asked for {expected} replies and the loop wrote {}: {lines:?}",
            lines.len()
        );
        lines
    }

    fn reply_lines<H: Handler>(script: &str, expected: usize, handler: &mut H) -> Vec<String> {
        reply_lines_of(script.as_bytes(), expected, handler)
    }

    fn parse_replies(lines: &[String]) -> Vec<Value> {
        lines
            .iter()
            .map(|line| {
                serde_json::from_str::<Value>(line)
                    .unwrap_or_else(|error| panic!("reply `{line}` is not one JSON line: {error}"))
            })
            .collect()
    }

    fn converse_of<H: Handler>(script: &[u8], expected: usize, handler: &mut H) -> Vec<Value> {
        parse_replies(&reply_lines_of(script, expected, handler))
    }

    fn converse<H: Handler>(script: &str, expected: usize, handler: &mut H) -> Vec<Value> {
        converse_of(script.as_bytes(), expected, handler)
    }

    fn echoed(fields: &str) -> Value {
        serde_json::from_str::<Value>(fields).expect("the expected echo parses")
    }

    fn id_token(line: &str) -> String {
        let fields = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(line)
            .unwrap_or_else(|error| panic!("reply `{line}` is not a JSON object: {error}"));
        fields
            .get("id")
            .unwrap_or_else(|| panic!("reply `{line}` carries no id"))
            .get()
            .to_string()
    }

    fn code_of(reply: &Value) -> i64 {
        reply
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_i64)
            .unwrap_or_else(|| panic!("reply {reply} carries no error code"))
    }

    fn controller() -> Controller {
        Controller::new(
            InstanceName::from("work"),
            ControllerState {
                instances: vec![InstanceRecord {
                    name: InstanceName::from("work"),
                    cells: vec![
                        CellRecord {
                            id: CellId::from("cell-1"),
                            genome: Some(GenomeName::from("researcher")),
                            status: CellStatus::Ready,
                            plasmids: vec![
                                PlasmidRecord {
                                    plasmid: "github-pr".to_string(),
                                    mock: MockMode::Simulate,
                                },
                                PlasmidRecord {
                                    plasmid: "model-provider".to_string(),
                                    mock: MockMode::Passthrough,
                                },
                            ],
                        },
                        CellRecord {
                            id: CellId::from("cell-2"),
                            genome: None,
                            status: CellStatus::Draining,
                            plasmids: Vec::new(),
                        },
                    ],
                }],
            },
            4,
        )
    }

    #[test]
    fn a_line_that_is_not_json_gets_parse_error_with_null_id() {
        let replies = converse("this is not json\n", 1, &mut Echo);
        assert_eq!(
            code_of(&replies[0]),
            ErrorCode::ParseError.as_i64(),
            "an unparseable line: {}",
            replies[0]
        );
        assert_eq!(
            replies[0].get("id"),
            Some(&Value::Null),
            "an unparseable line has no id to echo: {}",
            replies[0]
        );
    }

    #[test]
    fn a_json_line_that_is_not_the_envelope_is_invalid_request() {
        let script = concat!(
            "{\"method\":\"echo\",\"params\":{}}\n",
            "{\"id\":1,\"params\":{}}\n",
            "{\"id\":2,\"method\":\"echo\"}\n",
            "{\"id\":3,\"method\":\"echo\",\"params\":[]}\n",
            "[1,\"echo\",{}]\n",
        );
        let replies = converse(script, 5, &mut Echo);
        for (reply, missing) in replies.iter().zip([
            "an envelope with no id",
            "an envelope with no method",
            "an envelope with no params",
            "an envelope whose params are not an object",
            "a JSON array, which is not an envelope this protocol defines",
        ]) {
            assert_eq!(
                code_of(reply),
                ErrorCode::InvalidRequest.as_i64(),
                "{missing}: {reply}"
            );
        }
        assert_eq!(
            replies
                .iter()
                .map(|reply| reply.get("id").cloned().unwrap_or(Value::Null))
                .collect::<Vec<Value>>(),
            vec![
                Value::Null,
                Value::from(1),
                Value::from(2),
                Value::from(3),
                Value::Null,
            ],
            "an invalid request echoes the id when the object carried one: {replies:?}"
        );
    }

    #[test]
    fn an_unknown_method_is_method_not_found() {
        let replies = converse(
            "{\"id\":1,\"method\":\"plasmosome.fly\",\"params\":{}}\n",
            1,
            &mut Echo,
        );
        assert_eq!(
            code_of(&replies[0]),
            ErrorCode::MethodNotFound.as_i64(),
            "a method no handler serves: {}",
            replies[0]
        );
    }

    #[test]
    fn status_params_that_do_not_parse_are_invalid_params() {
        let replies = converse(
            "{\"id\":1,\"method\":\"plasmosome.status\",\"params\":{\"name\":42}}\n",
            1,
            &mut controller(),
        );
        assert_eq!(
            code_of(&replies[0]),
            ErrorCode::InvalidParams.as_i64(),
            "a served method with params that do not parse is not a missing method: {}",
            replies[0]
        );
    }

    #[test]
    fn every_reply_echoes_the_request_id_verbatim() {
        let script = concat!(
            "{\"id\":\"abc\",\"method\":\"echo\",\"params\":{}}\n",
            "{\"id\":{\"trace\":\"x-9\"},\"method\":\"echo\",\"params\":{}}\n",
        );
        let replies = converse(script, 2, &mut Echo);
        assert_eq!(
            replies
                .iter()
                .map(|reply| reply.get("id").cloned().unwrap_or(Value::Null))
                .collect::<Vec<Value>>(),
            vec![
                Value::from("abc"),
                serde_json::from_str::<Value>("{\"trace\":\"x-9\"}").expect("an object id parses"),
            ],
            "the ids that came back: {replies:?}"
        );
    }

    #[test]
    fn replies_come_back_in_request_order() {
        let script = concat!(
            "{\"id\":1,\"method\":\"echo\",\"params\":{}}\n",
            "not json\n",
            "{\"id\":3,\"method\":\"nope\",\"params\":{}}\n",
            "{\"id\":4,\"method\":\"echo\",\"params\":{}}\n",
        );
        let replies = converse(script, 4, &mut Echo);
        assert_eq!(
            replies
                .iter()
                .map(|reply| reply.get("id").cloned().unwrap_or(Value::Null))
                .collect::<Vec<Value>>(),
            vec![Value::from(1), Value::Null, Value::from(3), Value::from(4)],
            "replies arrive in request order: {replies:?}"
        );
    }

    #[test]
    fn the_loop_survives_a_bad_line_and_keeps_serving() {
        let script = concat!(
            "{ not json at all\n",
            "{\"id\":2,\"method\":\"echo\",\"params\":{\"still\":\"here\"}}\n",
        );
        let replies = converse(script, 2, &mut Echo);
        assert_eq!(
            replies[1].get("result"),
            Some(&serde_json::from_str::<Value>("{\"still\":\"here\"}").expect("the echo parses")),
            "the line after a bad one is served: {}",
            replies[1]
        );
    }

    #[test]
    fn status_reports_the_instance_its_cells_and_their_mock_labels() {
        let replies = converse(
            "{\"id\":3,\"method\":\"plasmosome.status\",\"params\":{\"name\":\"work\"}}\n",
            1,
            &mut controller(),
        );
        let result = replies[0]
            .get("result")
            .unwrap_or_else(|| panic!("status answered with no result: {}", replies[0]));
        assert_eq!(
            result.get("name").and_then(Value::as_str),
            Some("work"),
            "the instance status names: {result}"
        );
        assert_eq!(
            result.get("state").and_then(Value::as_str),
            Some("running"),
            "the instance state: {result}"
        );
        assert_eq!(
            result.get("ready").and_then(Value::as_bool),
            Some(true),
            "the readiness of a controller that just answered: {result}"
        );
        assert_eq!(
            result
                .get("controller")
                .and_then(|controller| controller.get("ledger_generation"))
                .and_then(Value::as_u64),
            Some(4),
            "the ledger generation the controller was built with: {result}"
        );
        assert!(
            result
                .get("controller")
                .and_then(|controller| controller.get("uptime_ms"))
                .and_then(Value::as_u64)
                .is_some(),
            "the controller reports an uptime: {result}"
        );
        let cells = result
            .get("cells")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("status answered without a cell list: {result}"));
        assert_eq!(cells.len(), 2, "the cells of `work`: {result}");
        assert_eq!(
            cells[0].get("plasmids"),
            Some(
                &serde_json::from_str::<Value>(
                    "[\"github-pr [mock:simulate]\", \"model-provider [real]\"]"
                )
                .expect("the expected labels parse")
            ),
            "the D2 mock labels of cell-1: {}",
            cells[0]
        );
        assert_eq!(
            cells[1].get("state").and_then(Value::as_str),
            Some("draining"),
            "the state of cell-2: {}",
            cells[1]
        );
    }

    #[test]
    fn status_for_a_name_this_controller_is_not_is_unknown_target() {
        let replies = converse(
            "{\"id\":3,\"method\":\"plasmosome.status\",\"params\":{\"name\":\"other\"}}\n",
            1,
            &mut controller(),
        );
        assert_eq!(
            code_of(&replies[0]),
            ErrorCode::UnknownTarget.as_i64(),
            "a controller resolves the name it was asked for, it never guesses: {}",
            replies[0]
        );
        assert_eq!(
            replies[0]
                .get("error")
                .and_then(|error| error.get("target"))
                .and_then(Value::as_str),
            Some("plasmosome other"),
            "the target the request asked for: {}",
            replies[0]
        );
        assert_eq!(
            replies[0].get("result"),
            None,
            "an unknown target is never answered with this controller's own status: {}",
            replies[0]
        );
    }

    #[test]
    fn an_id_a_json_number_cannot_hold_comes_back_unchanged() {
        let ids = [
            "1e400",
            "123456789012345678901234567890",
            "1e2",
            "1.0000000000000000000000001",
            "18446744073709551615",
            "\"abc\"",
            "{\"trace\":\"x-9\"}",
            "[1,2,3]",
        ];
        let script: String = ids
            .iter()
            .map(|id| format!("{{\"id\":{id},\"method\":\"echo\",\"params\":{{}}}}\n"))
            .collect();
        let lines = reply_lines(&script, ids.len(), &mut Echo);
        assert_eq!(
            lines
                .iter()
                .map(|line| id_token(line))
                .collect::<Vec<String>>(),
            ids.iter()
                .map(|id| (*id).to_string())
                .collect::<Vec<String>>(),
            "every reply carries back the id token its request sent: {lines:?}"
        );
    }

    #[test]
    fn a_status_name_that_is_not_an_instance_name_is_invalid_params() {
        for name in ["../..", "", "work/../other", ".."] {
            let script = format!(
                "{{\"id\":1,\"method\":\"plasmosome.status\",\"params\":{{\"name\":\"{name}\"}}}}\n"
            );
            let replies = converse(&script, 1, &mut controller());
            assert_eq!(
                code_of(&replies[0]),
                ErrorCode::InvalidParams.as_i64(),
                "`{name}` is not an instance name, and a later verb resolves this name into a path: {}",
                replies[0]
            );
        }
    }

    #[test]
    fn a_line_that_is_not_utf8_is_answered_parse_error_and_the_loop_keeps_serving() {
        let mut script: Vec<u8> = Vec::new();
        script.extend_from_slice(b"{\"id\":1,\"method\":\"echo\",\"params\":{}}\n");
        script.extend_from_slice(&[0xFF, 0xFE, 0xFD]);
        script.push(b'\n');
        script
            .extend_from_slice(b"{\"id\":3,\"method\":\"echo\",\"params\":{\"still\":\"here\"}}\n");
        let replies = converse_of(&script, 3, &mut Echo);
        assert_eq!(
            code_of(&replies[1]),
            ErrorCode::ParseError.as_i64(),
            "JSON is UTF-8, so bytes that are not text are not JSON: {}",
            replies[1]
        );
        assert_eq!(
            replies[1].get("id"),
            Some(&Value::Null),
            "an id read out of bytes the loop refused to trust is not an id: {}",
            replies[1]
        );
        assert_eq!(
            replies[2].get("result"),
            Some(&echoed("{\"still\":\"here\"}")),
            "the request after a non-text line is served: {}",
            replies[2]
        );
    }

    #[test]
    fn a_line_past_the_cap_is_refused_and_the_connection_ends() {
        let mut script: Vec<u8> = vec![b'a'; MAX_LINE_BYTES + 1];
        script.push(b'\n');
        script.extend_from_slice(b"{\"id\":2,\"method\":\"echo\",\"params\":{}}\n");
        let replies = converse_of(&script, 1, &mut Echo);
        assert_eq!(
            code_of(&replies[0]),
            ErrorCode::InvalidRequest.as_i64(),
            "a line past the cap is refused, not parsed: {}",
            replies[0]
        );
        assert_eq!(
            replies[0].get("id"),
            Some(&Value::Null),
            "the loop never read an envelope to take an id from: {}",
            replies[0]
        );
    }

    #[test]
    fn a_line_exactly_at_the_cap_is_served() {
        let request = "{\"id\":1,\"method\":\"echo\",\"params\":{}}";
        let mut line = request.to_string();
        line.push_str(&" ".repeat(MAX_LINE_BYTES - request.len()));
        assert_eq!(
            line.len(),
            MAX_LINE_BYTES,
            "the test line is exactly the cap, or it proves nothing about the boundary"
        );
        let mut terminated = line.clone().into_bytes();
        terminated.push(b'\n');
        let replies = converse_of(&terminated, 1, &mut Echo);
        assert_eq!(
            replies[0].get("result"),
            Some(&echoed("{}")),
            "a line of exactly the cap fits under it: {}",
            replies[0]
        );

        let replies = converse_of(line.as_bytes(), 1, &mut Echo);
        assert_eq!(
            replies[0].get("result"),
            Some(&echoed("{}")),
            "the cap is a maximum, not a threshold, at end of input as well: {}",
            replies[0]
        );
    }

    #[test]
    fn a_final_line_without_a_newline_is_still_answered() {
        let script = concat!(
            "{\"id\":1,\"method\":\"echo\",\"params\":{}}\n",
            "{\"id\":2,\"method\":\"echo\",\"params\":{\"last\":true}}",
        );
        let replies = converse(script, 2, &mut Echo);
        assert_eq!(
            replies[1].get("result"),
            Some(&echoed("{\"last\":true}")),
            "a client that hangs up without a trailing newline is still answered: {}",
            replies[1]
        );
    }

    #[test]
    fn a_panicking_handler_is_answered_internal_before_the_panic_resumes() {
        let script = concat!(
            "{\"id\":1,\"method\":\"boom\",\"params\":{}}\n",
            "{\"id\":2,\"method\":\"echo\",\"params\":{}}\n",
        );
        let mut written: Vec<u8> = Vec::new();
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            serve_connection(script.as_bytes(), &mut written, &mut Panicking)
        }));
        assert!(
            outcome.is_err(),
            "the panic resumes, so the process above the connection still sees it"
        );
        let replies = String::from_utf8(written).expect("every reply is utf-8");
        let lines: Vec<String> = replies.lines().map(str::to_string).collect();
        assert_eq!(
            lines.len(),
            1,
            "the panicking request is answered and nothing after it is served: {lines:?}"
        );
        let reply = parse_replies(&lines).remove(0);
        assert_eq!(
            code_of(&reply),
            ErrorCode::Internal.as_i64(),
            "a handler that failed while answering is the controller failing: {reply}"
        );
        assert_eq!(
            reply.get("id"),
            Some(&Value::from(1)),
            "the answer carries the id of the request that caused it: {reply}"
        );
    }

    #[test]
    fn a_panic_resumes_even_when_the_reply_cannot_be_written() {
        let script = "{\"id\":1,\"method\":\"boom\",\"params\":{}}\n";
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            serve_connection(script.as_bytes(), BrokenPipe, &mut Panicking)
        }));
        assert!(
            outcome.is_err(),
            "a client that hung up before its answer landed must not cost the process above the panic"
        );
    }

    #[test]
    fn a_crlf_line_is_served_and_its_carriage_return_counts_against_the_cap() {
        let replies = converse(
            "{\"id\":1,\"method\":\"echo\",\"params\":{\"crlf\":true}}\r\n",
            1,
            &mut Echo,
        );
        assert_eq!(
            replies[0].get("result"),
            Some(&echoed("{\"crlf\":true}")),
            "a client that ends its lines with CRLF is served: {}",
            replies[0]
        );

        let request = "{\"id\":1,\"method\":\"echo\",\"params\":{}}";
        let mut line = request.to_string();
        line.push_str(&" ".repeat(MAX_LINE_BYTES - request.len()));
        line.push('\r');
        assert_eq!(
            line.len(),
            MAX_LINE_BYTES + 1,
            "this line is one byte past the cap, and that byte is the carriage return"
        );
        let mut script = line.into_bytes();
        script.push(b'\n');
        let replies = converse_of(&script, 1, &mut Echo);
        assert_eq!(
            code_of(&replies[0]),
            ErrorCode::InvalidRequest.as_i64(),
            "the cap counts every byte before the newline, the carriage return included: {}",
            replies[0]
        );
    }

    #[test]
    fn a_panicking_handler_still_answers_the_client_over_a_real_socket() {
        let directory = tempfile::tempdir().expect("the test owns a temporary directory");
        let socket = directory.path().join("control.uds");
        let listener =
            UnixListener::bind(&socket).expect("the controller binds its control socket");
        let server = thread::spawn(move || {
            let (stream, _) = listener
                .accept()
                .expect("the controller accepts its one client");
            let reader = BufReader::new(
                stream
                    .try_clone()
                    .expect("the connection clones for reading"),
            );
            serve_connection(reader, stream, &mut Panicking)
        });

        let mut client = UnixStream::connect(&socket).expect("the client reaches the socket");
        client
            .write_all(b"{\"id\":1,\"method\":\"boom\",\"params\":{}}\n")
            .expect("the client writes its request line");
        client.flush().expect("the client flushes its request");
        let mut answered = String::new();
        BufReader::new(client.try_clone().expect("the client clones for reading"))
            .read_to_string(&mut answered)
            .expect("the client reads until the controller hangs up");

        let lines: Vec<String> = answered.lines().map(str::to_string).collect();
        assert_eq!(
            lines.len(),
            1,
            "the client reads a whole reply line and then end of input: {lines:?}"
        );
        let reply = parse_replies(&lines).remove(0);
        assert_eq!(
            code_of(&reply),
            ErrorCode::Internal.as_i64(),
            "the client is answered rather than left with a dead socket: {reply}"
        );
        assert!(
            server.join().is_err(),
            "the panic resumes on the serving thread, so whoever spawned it still owns the failure"
        );
    }

    #[test]
    fn a_handler_returning_a_loop_owned_code_is_answered_internal_and_the_loop_keeps_serving() {
        let script = concat!(
            "{\"id\":1,\"method\":\"lie\",\"params\":{}}\n",
            "{\"id\":2,\"method\":\"echo\",\"params\":{\"still\":\"here\"}}\n",
        );
        let replies = converse(script, 2, &mut Lying);
        assert_eq!(
            code_of(&replies[0]),
            ErrorCode::Internal.as_i64(),
            "only the loop saw the frame, so only the loop may answer about it: {}",
            replies[0]
        );
        assert_eq!(
            replies[0].get("id"),
            Some(&Value::from(1)),
            "the replacement answers the request that was asked: {}",
            replies[0]
        );
        assert_eq!(
            replies[1].get("result"),
            Some(&echoed("{\"still\":\"here\"}")),
            "a handler that returned normally is wrong about the protocol, not broken: {}",
            replies[1]
        );
    }

    #[test]
    fn a_real_socket_conversation_answers_line_per_line_and_ends_at_eof() {
        let directory = tempfile::tempdir().expect("the test owns a temporary directory");
        let socket = directory.path().join("control.uds");
        let listener =
            UnixListener::bind(&socket).expect("the controller binds its control socket");
        let server = thread::spawn(move || {
            let (stream, _) = listener
                .accept()
                .expect("the controller accepts its one client");
            let reader = BufReader::new(
                stream
                    .try_clone()
                    .expect("the connection clones for reading"),
            );
            serve_connection(reader, stream, &mut controller())
        });

        let mut client = UnixStream::connect(&socket).expect("the client reaches the socket");
        client
            .write_all(
                concat!(
                    "{\"id\":1,\"method\":\"plasmosome.status\",\"params\":{}}\n",
                    "{\"id\":2,\"method\":\"plasmosome.fly\",\"params\":{}}\n",
                )
                .as_bytes(),
            )
            .expect("the client writes both request lines");
        client.flush().expect("the client flushes its requests");
        client
            .shutdown(Shutdown::Write)
            .expect("the client hangs up its writing half");
        let mut answered = String::new();
        BufReader::new(client.try_clone().expect("the client clones for reading"))
            .read_to_string(&mut answered)
            .expect("the client reads until the controller hangs up");

        let replies: Vec<Value> = answered
            .lines()
            .map(|line| {
                serde_json::from_str::<Value>(line)
                    .unwrap_or_else(|error| panic!("reply `{line}` is not one JSON line: {error}"))
            })
            .collect();
        assert_eq!(replies.len(), 2, "two requests, two replies: {replies:?}");
        assert_eq!(
            replies[0]
                .get("result")
                .and_then(|result| result.get("name"))
                .and_then(Value::as_str),
            Some("work"),
            "the status answer over a real socket: {}",
            replies[0]
        );
        assert_eq!(
            code_of(&replies[1]),
            ErrorCode::MethodNotFound.as_i64(),
            "the unserved verb over a real socket: {}",
            replies[1]
        );
        server
            .join()
            .expect("the serving thread finishes")
            .expect("the client hanging up ends the loop without error");
    }
}
