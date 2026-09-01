use crate::brokers::SetStatus;
use crate::readiness::NotReady;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The most bytes a request line may hold before its terminating newline.
pub const MAX_REQUEST_BYTES: usize = 1_048_576;

const STATUS_METHOD: &str = "membrane.status";
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const ACCEPT_POLL: Duration = Duration::from_millis(25);
const READ_TIMEOUT: Duration = Duration::from_millis(50);
const WRITE_TIMEOUT: Duration = Duration::from_millis(250);

/// Answers one request line, returning the response line to send back.
///
/// `line` is one ndjson request without its newline. `status` is asked exactly
/// once, and only when the line is a well-formed `membrane.status` request, so a
/// caller may put a live probe behind it. The returned string is a complete JSON
/// response and never carries a newline of its own.
///
/// A caller must not use the reply to decide whether the connection closes:
/// nothing in a response says that, and only an over-long line closes one.
pub fn respond(line: &str, status: &mut impl FnMut() -> SetStatus) -> String {
    let Ok(envelope) = serde_json::from_str::<Value>(line) else {
        return failure(Value::Null, PARSE_ERROR, "request line is not JSON");
    };
    let id = envelope.get("id").cloned().unwrap_or(Value::Null);
    let not_a_request = |reason| failure(id.clone(), INVALID_REQUEST, reason);
    let Some(fields) = envelope.as_object() else {
        return not_a_request("request is not an object");
    };
    let Some(method) = fields.get("method").and_then(Value::as_str) else {
        return not_a_request("request carries no string `method`");
    };
    if fields
        .get("params")
        .filter(|params| params.is_object())
        .is_none()
    {
        return not_a_request("request carries no object `params`");
    }
    if method != STATUS_METHOD {
        return failure(id, METHOD_NOT_FOUND, "no such method");
    }
    json!({"id": id, "result": wire_status(status())}).to_string()
}

fn failure(id: Value, code: i64, message: &str) -> String {
    json!({"id": id, "error": {"code": code, "message": message}}).to_string()
}

fn wire_status(status: SetStatus) -> Value {
    match status {
        SetStatus::Ready => json!({"ready": true, "state": "serving"}),
        SetStatus::NotReady { broker, reason } => {
            let mut result = json!({"ready": false, "state": "not_serving",
                                    "broker": broker, "reason": reason_name(&reason)});
            if let NotReady::Reported { state } = reason {
                result["broker_state"] = Value::String(state);
            }
            result
        }
        SetStatus::DeadlineSpent { unreached, asked } => {
            let mut spent = json!({"ready": false, "state": "deadline_spent",
                                   "unreached": unreached});
            if !asked.is_empty() {
                spent["asked"] = json!(asked);
            }
            spent
        }
        SetStatus::Empty => json!({"ready": false, "state": "empty"}),
    }
}

fn reason_name(reason: &NotReady) -> &'static str {
    match reason {
        NotReady::Unreachable { .. } => "unreachable",
        NotReady::TimedOut => "timed_out",
        NotReady::Malformed { .. } => "malformed",
        NotReady::Reported { .. } => "reported",
    }
}

/// Serves ndjson requests on `listener` until `shutdown` is set.
///
/// Connections are taken one at a time and each is answered in request order.
/// The flag is checked between accepts and between reads, and both halves of a
/// connection are bounded by a timeout, so neither an idle client nor one that
/// never reads its replies can hold the server open past shutdown. `status` is
/// called once per `membrane.status` request, never cached.
///
/// Returns `Ok` when the flag is set, and `Err` when the listener stopped being
/// usable — a caller that treats both alike reports a broken socket as a
/// requested shutdown.
pub fn serve(
    listener: UnixListener,
    shutdown: &AtomicBool,
    mut status: impl FnMut() -> SetStatus,
) -> Result<(), ListenerFailed> {
    listener.set_nonblocking(true).map_err(ListenerFailed)?;
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => converse(stream, shutdown, &mut status),
            Err(error) => match accept_outcome(&error) {
                Next::Poll => std::thread::sleep(ACCEPT_POLL),
                Next::Retry => {}
                Next::Fail => return Err(ListenerFailed(error)),
            },
        }
    }
    Ok(())
}

/// What the accept loop does with an error the listener returned.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    Poll,
    Retry,
    Fail,
}

/// Why `serve` stopped when it did not stop because it was asked to.
#[derive(Debug)]
pub struct ListenerFailed(pub std::io::Error);

pub fn accept_outcome(error: &std::io::Error) -> Next {
    match error.kind() {
        ErrorKind::WouldBlock => Next::Poll,
        ErrorKind::Interrupted => Next::Retry,
        _ => Next::Fail,
    }
}

enum Request {
    Line(String),
    NotUtf8,
    TooLong,
    Ended,
}

fn converse(stream: UnixStream, shutdown: &AtomicBool, status: &mut impl FnMut() -> SetStatus) {
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(READ_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
    {
        return;
    }
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    loop {
        let reply = match read_request(&mut reader, shutdown) {
            Request::Line(line) => respond(&line, status),
            Request::NotUtf8 => failure(Value::Null, PARSE_ERROR, "request line is not UTF-8"),
            Request::TooLong => {
                let refusal = failure(Value::Null, INVALID_REQUEST, "request line is too long");
                let _ = writeln!(writer, "{refusal}");
                let _ = writer.flush();
                return;
            }
            Request::Ended => return,
        };
        if writeln!(writer, "{reply}").is_err() || writer.flush().is_err() {
            return;
        }
    }
}

fn read_request(reader: &mut BufReader<UnixStream>, shutdown: &AtomicBool) -> Request {
    let mut line: Vec<u8> = Vec::new();
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return Request::Ended;
        }
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(error) if waiting(&error) => continue,
            Err(_) => return Request::Ended,
        };
        if available.is_empty() {
            return Request::Ended;
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let taken = newline.unwrap_or(available.len());
        let consumed = newline.map_or(available.len(), |at| at + 1);
        line.extend_from_slice(&available[..taken]);
        reader.consume(consumed);
        if line.len() > MAX_REQUEST_BYTES {
            return Request::TooLong;
        }
        if newline.is_some() {
            return match String::from_utf8(line) {
                Ok(text) => Request::Line(text),
                Err(_) => Request::NotUtf8,
            };
        }
    }
}

fn waiting(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::readiness::NotReady;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::thread;

    const STATUS_REQUEST: &str = r#"{"id":1,"method":"membrane.status","params":{}}"#;
    const PATIENCE: Duration = Duration::from_secs(10);

    fn answer(line: &str, status: SetStatus) -> Value {
        let mut once = Some(status);
        let mut supply = move || once.take().expect("the status is asked once per request");
        let reply = respond(line, &mut supply);
        serde_json::from_str(&reply)
            .unwrap_or_else(|error| panic!("the reply to {line:?} is JSON, got {reply:?}: {error}"))
    }

    fn result_of(status: SetStatus) -> Value {
        let reply = answer(STATUS_REQUEST, status);
        reply
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("a status reply carries a result, got {reply}"))
    }

    fn error_of(line: &str) -> Value {
        let reply = answer(line, SetStatus::Ready);
        reply
            .get("error")
            .cloned()
            .unwrap_or_else(|| panic!("the reply to {line:?} carries an error, got {reply}"))
    }

    struct Serving {
        socket: PathBuf,
        shutdown: Arc<AtomicBool>,
        finished: mpsc::Receiver<Result<(), ListenerFailed>>,
        handle: Option<thread::JoinHandle<()>>,
        _dir: tempfile::TempDir,
    }

    impl Serving {
        fn client(&self) -> BufReader<UnixStream> {
            let stream = UnixStream::connect(&self.socket)
                .unwrap_or_else(|error| panic!("a client reaches the served socket: {error}"));
            stream
                .set_read_timeout(Some(PATIENCE))
                .expect("the test client bounds its own reads");
            BufReader::new(stream)
        }
    }

    impl Drop for Serving {
        fn drop(&mut self) {
            self.shutdown.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn serving(status: impl FnMut() -> SetStatus + Send + 'static) -> Serving {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("c.uds");
        let listener = UnixListener::bind(&socket).expect("the test binds its control socket");
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&shutdown);
        let (done, finished) = mpsc::channel();
        let handle = thread::spawn(move || {
            let outcome = serve(listener, &flag, status);
            let _ = done.send(outcome);
        });
        Serving {
            socket,
            shutdown,
            finished,
            handle: Some(handle),
            _dir: dir,
        }
    }

    fn ask(client: &mut BufReader<UnixStream>, line: &str) -> Value {
        let mut stream = client.get_ref().try_clone().expect("clone for writing");
        writeln!(stream, "{line}").expect("the request reaches the server");
        stream.flush().expect("the request is flushed");
        read_reply(client)
    }

    fn read_reply(client: &mut BufReader<UnixStream>) -> Value {
        let mut reply = String::new();
        let read = client
            .read_line(&mut reply)
            .expect("the server answers rather than failing the read");
        assert_ne!(
            read, 0,
            "the server answered rather than closing the socket"
        );
        serde_json::from_str(&reply)
            .unwrap_or_else(|error| panic!("the server answers JSON, got {reply:?}: {error}"))
    }

    fn padded_request(total: usize) -> String {
        let prefix = r#"{"id":1,"method":"membrane.status","params":{"pad":""#;
        let suffix = r#""}}"#;
        let padding = total - prefix.len() - suffix.len();
        format!("{prefix}{}{suffix}", "x".repeat(padding))
    }

    #[test]
    fn a_status_request_is_answered_with_the_sets_readiness() {
        let reply = answer(
            r#"{"id":"abc","method":"membrane.status","params":{}}"#,
            SetStatus::Ready,
        );
        assert_eq!(reply.get("id"), Some(&json!("abc")));
        assert_eq!(
            reply.get("result"),
            Some(&json!({"ready": true, "state": "serving"}))
        );
        assert_eq!(reply.get("error"), None, "a success carries no error key");
    }

    #[test]
    fn each_set_status_maps_to_its_wire_shape() {
        assert_eq!(
            result_of(SetStatus::Ready),
            json!({"ready": true, "state": "serving"})
        );
        assert_eq!(
            result_of(SetStatus::NotReady {
                broker: "egressd".to_string(),
                reason: NotReady::Unreachable {
                    path: PathBuf::from("/absent")
                },
            }),
            json!({"ready": false, "state": "not_serving", "broker": "egressd",
                   "reason": "unreachable"})
        );
        assert_eq!(
            result_of(SetStatus::NotReady {
                broker: "egressd".to_string(),
                reason: NotReady::TimedOut,
            }),
            json!({"ready": false, "state": "not_serving", "broker": "egressd",
                   "reason": "timed_out"})
        );
        assert_eq!(
            result_of(SetStatus::NotReady {
                broker: "dnsd".to_string(),
                reason: NotReady::Malformed {
                    line: "42".to_string()
                },
            }),
            json!({"ready": false, "state": "not_serving", "broker": "dnsd",
                   "reason": "malformed"})
        );
        assert_eq!(
            result_of(SetStatus::NotReady {
                broker: "dnsd".to_string(),
                reason: NotReady::Reported {
                    state: "starting".to_string()
                },
            }),
            json!({"ready": false, "state": "not_serving", "broker": "dnsd",
                   "reason": "reported", "broker_state": "starting"})
        );
        assert_eq!(
            result_of(SetStatus::DeadlineSpent {
                unreached: "dnsd".to_string(),
                asked: vec!["egressd".to_string()],
            }),
            json!({"ready": false, "state": "deadline_spent", "unreached": "dnsd",
                   "asked": ["egressd"]})
        );
        assert_eq!(
            result_of(SetStatus::Empty),
            json!({"ready": false, "state": "empty"})
        );
    }

    #[test]
    fn a_line_that_is_not_json_is_answered_32700() {
        let reply = answer("this is not json", SetStatus::Ready);
        assert_eq!(reply.get("id"), Some(&Value::Null));
        assert_eq!(
            reply.pointer("/error/code"),
            Some(&json!(PARSE_ERROR)),
            "got {reply}"
        );
        assert_eq!(reply.get("result"), None, "a failure carries no result key");
    }

    #[test]
    fn an_envelope_that_is_not_a_request_is_answered_32600() {
        for line in [
            "[1,2,3]",
            "7",
            r#"{"id":5,"params":{}}"#,
            r#"{"id":5,"method":7,"params":{}}"#,
            r#"{"id":5,"method":"membrane.status"}"#,
            r#"{"id":5,"method":"membrane.status","params":7}"#,
        ] {
            assert_eq!(
                error_of(line).get("code"),
                Some(&json!(INVALID_REQUEST)),
                "{line} is not a request"
            );
        }
        assert_eq!(
            answer("[1,2,3]", SetStatus::Ready).get("id"),
            Some(&Value::Null)
        );
        assert_eq!(
            answer(r#"{"id":5,"params":{}}"#, SetStatus::Ready).get("id"),
            Some(&json!(5)),
            "an id that is there is echoed even when the rest is not a request"
        );
    }

    #[test]
    fn an_unknown_method_is_answered_32601() {
        let reply = answer(
            r#"{"id":[1,{"deep":true}],"method":"membrane.nope","params":{}}"#,
            SetStatus::Ready,
        );
        assert_eq!(
            reply.pointer("/error/code"),
            Some(&json!(METHOD_NOT_FOUND)),
            "got {reply}"
        );
        assert_eq!(
            reply.get("id"),
            Some(&json!([1, {"deep": true}])),
            "any JSON id is echoed verbatim"
        );
    }

    #[test]
    fn a_connection_carries_many_requests_in_order_and_survives_a_parse_error() {
        let served = serving(|| SetStatus::Empty);
        let mut client = served.client();
        assert_eq!(
            ask(&mut client, "not json at all").pointer("/error/code"),
            Some(&json!(PARSE_ERROR))
        );
        assert_eq!(
            ask(&mut client, STATUS_REQUEST).pointer("/result/state"),
            Some(&json!("empty")),
            "the conversation continues after a parse error"
        );
        drop(client);

        let mut second = served.client();
        assert_eq!(
            ask(&mut second, STATUS_REQUEST).pointer("/result/state"),
            Some(&json!("empty")),
            "a second connection is served once the first closes"
        );
    }

    #[test]
    fn a_request_line_at_the_cap_is_answered_and_one_byte_over_is_refused_and_closed() {
        let served = serving(|| SetStatus::Ready);

        let mut at_cap = served.client();
        let request = padded_request(MAX_REQUEST_BYTES);
        assert_eq!(request.len(), MAX_REQUEST_BYTES);
        assert_eq!(
            ask(&mut at_cap, &request).pointer("/result/ready"),
            Some(&json!(true)),
            "a line exactly at the cap is served"
        );
        drop(at_cap);

        let mut over_cap = served.client();
        let too_long = padded_request(MAX_REQUEST_BYTES + 1);
        assert_eq!(too_long.len(), MAX_REQUEST_BYTES + 1);
        let mut stream = over_cap.get_ref().try_clone().expect("clone for writing");
        let _ = writeln!(stream, "{too_long}");
        let _ = stream.flush();
        let reply = read_reply(&mut over_cap);
        assert_eq!(reply.get("id"), Some(&Value::Null));
        assert_eq!(reply.pointer("/error/code"), Some(&json!(INVALID_REQUEST)));
        let mut after = String::new();
        assert_eq!(
            over_cap.read_line(&mut after).ok(),
            Some(0),
            "the connection closes after an over-long line, got {after:?}"
        );
    }

    #[test]
    fn shutdown_stops_serve_even_with_an_idle_connection_open() {
        let served = serving(|| SetStatus::Ready);
        let mut client = served.client();
        assert_eq!(
            ask(&mut client, STATUS_REQUEST).pointer("/result/ready"),
            Some(&json!(true)),
            "the server is inside the connection before the flag is set"
        );

        served.shutdown.store(true, Ordering::Relaxed);
        assert!(
            served
                .finished
                .recv_timeout(PATIENCE)
                .expect("serve returns once the flag is set, with the client still connected")
                .is_ok(),
            "a shutdown that was asked for reports success",
        );
    }

    #[test]
    fn shutdown_stops_serve_even_with_a_client_that_never_reads_its_replies() {
        let served = serving(|| SetStatus::Ready);
        let client = served.client();
        let mut writer = client
            .get_ref()
            .try_clone()
            .expect("the test clones its own writing half");
        writer
            .set_write_timeout(Some(Duration::from_millis(200)))
            .expect("the test bounds its own writes so a wedged server cannot hang it");
        for _ in 0..20_000 {
            if writeln!(writer, "{STATUS_REQUEST}").is_err() {
                break;
            }
        }
        let _ = writer.flush();

        served.shutdown.store(true, Ordering::Relaxed);
        assert!(
            served
                .finished
                .recv_timeout(PATIENCE)
                .expect(
                    "serve returns once the flag is set even against a client that never reads; \
                     a write with no timeout blocks forever and turns the covered SIGTERM \
                     teardown into the uncovered SIGKILL residue path",
                )
                .is_ok(),
            "a shutdown that was asked for reports success",
        );
    }

    #[test]
    fn a_line_that_is_not_utf8_is_answered_32700_rather_than_silently_repaired() {
        let served = serving(|| SetStatus::Ready);
        let mut client = served.client();
        let mut writer = client
            .get_ref()
            .try_clone()
            .expect("the test clones its own writing half");
        let mut line: Vec<u8> = br#"{"id":1,"method":"membrane.status","params":{"pad":""#.to_vec();
        line.push(0xFF);
        line.extend_from_slice(br#""}}"#);
        line.push(b'\n');
        writer.write_all(&line).expect("the test sends its bytes");
        writer.flush().expect("the test flushes");

        let mut reply = String::new();
        client
            .read_line(&mut reply)
            .expect("a reply comes back for a line that is not UTF-8");
        let value: Value = serde_json::from_str(&reply)
            .unwrap_or_else(|error| panic!("the reply is JSON, got {reply:?}: {error}"));
        assert_eq!(
            value.pointer("/error/code"),
            Some(&json!(PARSE_ERROR)),
            "spec 001 answers a line that is not UTF-8 with a parse error; repairing the bytes \
             into something that happens to parse serves a request nobody sent, got {value}",
        );
    }

    #[test]
    fn an_accept_error_is_classified_by_what_it_means_for_the_loop() {
        assert_eq!(
            accept_outcome(&std::io::Error::from(ErrorKind::WouldBlock)),
            Next::Poll,
            "a nonblocking listener with nothing pending is the idle case, not a failure",
        );
        assert_eq!(
            accept_outcome(&std::io::Error::from(ErrorKind::Interrupted)),
            Next::Retry,
            "a signal interrupting accept is retried, not a failure",
        );
        assert_eq!(
            accept_outcome(&std::io::Error::from(ErrorKind::Other)),
            Next::Fail,
            "any other accept error is a listener that stopped working",
        );
    }

    #[test]
    fn a_requested_shutdown_reports_success_and_a_broken_listener_would_not() {
        let served = serving(|| SetStatus::Ready);
        let mut client = served.client();
        assert_eq!(
            ask(&mut client, STATUS_REQUEST).pointer("/result/ready"),
            Some(&json!(true)),
            "the server is serving before the flag is set"
        );
        served.shutdown.store(true, Ordering::Relaxed);
        assert!(
            served
                .finished
                .recv_timeout(PATIENCE)
                .expect("serve returns once the flag is set")
                .is_ok(),
            "a shutdown that was asked for reports success; only that makes a listener failure \
             distinguishable from it",
        );
    }

    #[test]
    fn a_deadline_spent_before_anything_was_asked_omits_the_empty_list() {
        assert_eq!(
            result_of(SetStatus::DeadlineSpent {
                unreached: "dnsd".to_string(),
                asked: Vec::new(),
            }),
            json!({"ready": false, "state": "deadline_spent", "unreached": "dnsd"}),
            "spec 001 never sends a field with nothing in it",
        );
    }
}
