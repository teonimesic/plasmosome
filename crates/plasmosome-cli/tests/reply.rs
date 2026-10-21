use std::collections::VecDeque;
use std::io::{self, BufRead, Read};

use plasmosome_cli::json::{Json, MAX_REPLY_BYTES, Number, scan, write_compact};
use plasmosome_cli::reply::{
    ReadFailure, Reply, judge_reply, kernel_exit, read_reply, request_line, unanswered,
};
use plasmosome_cli::{ClientCode, ClientError, Envelope, ErrorBody, Form, KernelError, emit};

const LF: u8 = b'\n';

const CHUNK: usize = 65_521;

fn parsed(text: &str) -> Json {
    scan(text.as_bytes()).expect("the fixture is valid JSON")
}

fn text(value: &str) -> Json {
    Json::String(value.to_string())
}

fn number(value: &str) -> Json {
    Json::Number(Number::new(value.to_string()).expect("the fixture is a JSON number"))
}

fn result_of(line: &str) -> Json {
    match judge_reply(line.as_bytes()) {
        Ok(Reply::Result(result)) => result,
        other => panic!("{line} should be a result, got {other:?}"),
    }
}

fn kernel_of(line: &str) -> KernelError {
    match judge_reply(line.as_bytes()) {
        Ok(Reply::Kernel(error)) => error,
        other => panic!("{line} should be a kernel error, got {other:?}"),
    }
}

fn bad_reply_detail(line: &[u8]) -> String {
    let error = match judge_reply(line) {
        Err(error) => error,
        Ok(reply) => panic!(
            "{:?} should be a bad reply, got {reply:?}",
            String::from_utf8_lossy(line)
        ),
    };
    assert_eq!(error.code(), ClientCode::BadReply);
    assert_eq!(error.code().exit().code(), 70);
    match error.fields() {
        [(key, Json::String(detail))] if key == "detail" => detail.clone(),
        other => panic!("a bad reply carries one string detail, got {other:?}"),
    }
}

fn emitted_stderr(error: ErrorBody) -> serde_json::Value {
    let envelope = Envelope {
        command: "plasmosome.status".to_string(),
        instance: None,
        outcome: Err(error),
    };
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    emit(&envelope, Form::Json, &mut stdout, &mut stderr).expect("vectors accept every write");
    assert!(stdout.is_empty());
    serde_json::from_slice(&stderr).expect("the envelope is JSON")
}

fn read_through_the_buffer(reader: &mut impl BufRead, out: &mut [u8]) -> io::Result<usize> {
    let available = reader.fill_buf()?;
    let count = available.len().min(out.len());
    out[..count].copy_from_slice(&available[..count]);
    reader.consume(count);
    Ok(count)
}

enum Step {
    Bytes(Vec<u8>),
    Fail(io::Error),
}

struct Script {
    steps: VecDeque<Step>,
    taken: usize,
}

impl Script {
    fn new(steps: Vec<Step>) -> Script {
        Script {
            steps: steps.into(),
            taken: 0,
        }
    }

    fn left(&self) -> Vec<u8> {
        let mut left = Vec::new();
        for (index, step) in self.steps.iter().enumerate() {
            if let Step::Bytes(bytes) = step {
                let start = if index == 0 { self.taken } else { 0 };
                left.extend_from_slice(&bytes[start..]);
            }
        }
        left
    }
}

impl Read for Script {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        read_through_the_buffer(self, out)
    }
}

impl BufRead for Script {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if matches!(self.steps.front(), Some(Step::Fail(_)))
            && let Some(Step::Fail(error)) = self.steps.pop_front()
        {
            return Err(error);
        }
        match self.steps.front() {
            Some(Step::Bytes(bytes)) => Ok(&bytes[self.taken..]),
            _ => Ok(&[]),
        }
    }

    fn consume(&mut self, count: usize) {
        self.taken += count;
        if let Some(Step::Bytes(bytes)) = self.steps.front()
            && self.taken == bytes.len()
        {
            self.steps.pop_front();
            self.taken = 0;
        }
    }
}

struct Chunked {
    bytes: Vec<u8>,
    at: usize,
}

impl Read for Chunked {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        read_through_the_buffer(self, out)
    }
}

impl BufRead for Chunked {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let end = self.bytes.len().min(self.at + CHUNK);
        Ok(&self.bytes[self.at..end])
    }

    fn consume(&mut self, count: usize) {
        self.at += count;
    }
}

struct Endless {
    chunk: Vec<u8>,
    taken: usize,
    gives_out_after: usize,
}

impl Read for Endless {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        read_through_the_buffer(self, out)
    }
}

impl BufRead for Endless {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let left = self.gives_out_after - self.taken;
        Ok(&self.chunk[..left.min(self.chunk.len())])
    }

    fn consume(&mut self, count: usize) {
        self.taken += count;
    }
}

fn line_of(length: usize) -> Vec<u8> {
    let mut bytes = vec![b'x'; length];
    bytes.push(LF);
    bytes
}

#[test]
fn a_result_passes_through() {
    let result = r#"{"name":"work","state":"running","extra":{"nested":[1,2]},"big":18446744073709551616,"n":1.50}"#;
    let line = format!(r#"{{"id":1,"result":{result}}}"#);
    let passed = result_of(&line);
    assert_eq!(write_compact(&passed), result);
    assert_eq!(passed, parsed(result));

    let envelope = Envelope {
        command: "plasmosome.status".to_string(),
        instance: None,
        outcome: Ok(passed),
    };
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    emit(&envelope, Form::Json, &mut stdout, &mut stderr).expect("vectors accept every write");
    let printed = String::from_utf8(stdout).expect("the envelope is UTF-8");
    assert_eq!(
        printed,
        format!(r#"{{"schema":1,"command":"plasmosome.status","result":{result}}}"#) + "\n"
    );
    assert!(stderr.is_empty());
}

#[test]
fn members_may_come_in_any_order_and_unknown_ones_are_ignored() {
    assert_eq!(
        result_of(r#"{"result":{"a":1},"jsonrpc":"2.0","id":1}"#),
        parsed(r#"{"a":1}"#)
    );
    assert_eq!(
        result_of(" \t{ \"id\" : 1 , \"result\" : { } }\r"),
        parsed("{}")
    );
}

#[test]
fn a_kernel_error_keeps_every_field() {
    let error =
        r#"{"code":101,"message":"m","target":"plasmosome x","fix":"f","field":"network.hosts"}"#;
    let kernel = kernel_of(&format!(r#"{{"id":1,"error":{error}}}"#));
    assert_eq!(kernel_exit(&kernel).code(), 2);
    let fixture = parsed(error);
    let Json::Object(members) = &fixture else {
        panic!("the fixture is an object");
    };
    assert_eq!(kernel.members(), members.as_slice());

    let mut expected: serde_json::Value = serde_json::from_str(error).expect("the fixture is JSON");
    expected["domain"] = serde_json::Value::from("kernel");
    assert_eq!(emitted_stderr(ErrorBody::Kernel(kernel))["error"], expected);
}

#[test]
fn kernel_codes_exit_by_the_table() {
    let exit_for = |id: &str, error: &str| {
        kernel_exit(&kernel_of(&format!(r#"{{"id":{id},"error":{error}}}"#))).code()
    };
    for code in 100..=110 {
        let expected = if code == 107 { 1 } else { 2 };
        assert_eq!(
            exit_for("1", &format!(r#"{{"code":{code},"message":"m"}}"#)),
            expected,
            "code {code}"
        );
    }
    for to in [r#""running""#, r#"["unknown"]"#, "null", r#""Unknown""#] {
        assert_eq!(
            exit_for(
                "1",
                &format!(r#"{{"code":105,"message":"m","from":"x","to":{to}}}"#)
            ),
            2,
            "105 with to {to}"
        );
    }
    assert_eq!(
        exit_for(
            "1",
            r#"{"code":105,"message":"m","from":"running","to":"unknown"}"#
        ),
        3
    );
    for code in ["101", "999"] {
        assert_eq!(
            exit_for(
                "1",
                &format!(r#"{{"code":{code},"message":"m","to":"unknown"}}"#)
            ),
            2,
            "code {code} with to unknown"
        );
    }
    for code in ["-32601", "-32602", "999", "-1", "0", "99999999999999999999"] {
        assert_eq!(
            exit_for("1", &format!(r#"{{"code":{code},"message":"m"}}"#)),
            2,
            "code {code}"
        );
    }
    assert_eq!(exit_for("1", r#"{"code":-32603,"message":"m"}"#), 70);
    for code in ["-32700", "-32600"] {
        for id in ["null", "1"] {
            assert_eq!(
                exit_for(id, &format!(r#"{{"code":{code},"message":"m"}}"#)),
                70,
                "code {code} under id {id}"
            );
        }
    }
}

#[test]
fn a_protocol_error_under_a_null_id_is_relayed_as_the_kernels() {
    let kernel = kernel_of(r#"{"id":null,"error":{"code":-32700,"message":"parse error"}}"#);
    let printed = emitted_stderr(ErrorBody::Kernel(kernel));
    assert_eq!(
        printed["error"],
        serde_json::json!({"domain": "kernel", "code": -32700, "message": "parse error"})
    );
}

#[test]
fn a_kernel_exit_reads_the_code_it_was_given() {
    let without_code = KernelError::new(parsed(r#"{"message":"m"}"#))
        .expect("the fixture is an object without domain");
    assert_eq!(kernel_exit(&without_code).code(), 2);
    let text_code = KernelError::new(parsed(r#"{"code":"107","message":"m"}"#))
        .expect("the fixture is an object without domain");
    assert_eq!(kernel_exit(&text_code).code(), 2);
}

#[test]
fn every_bad_reply() {
    let cases: [(&[u8], &str); 25] = [
        (b"not json", "expected a value at byte 0"),
        (b"", "the text ends inside a value at byte 0"),
        (&[b'{', 0xff, b'}'], "the text is not UTF-8 at byte 1"),
        (b"[1]", "the reply is not an object"),
        (b"1", "the reply is not an object"),
        (
            br#"{"id":1,"result":{},"error":{"code":101,"message":"m"}}"#,
            "the reply has both result and error",
        ),
        (br#"{"id":1}"#, "the reply has neither result nor error"),
        (br#"{"id":2,"result":{}}"#, "the reply's id is not 1"),
        (br#"{"id":-1,"result":{}}"#, "the reply's id is not 1"),
        (br#"{"id":"1","result":{}}"#, "the reply's id is not 1"),
        (br#"{"id":1.0,"result":{}}"#, "the reply's id is not 1"),
        (br#"{"result":{}}"#, "the reply's id is not 1"),
        (br#"{"id":null,"result":{}}"#, "the reply's id is not 1"),
        (
            br#"{"id":null,"error":{"code":101,"message":"m"}}"#,
            "the reply's id is not 1",
        ),
        (
            br#"{"id":null,"error":{"code":-32603,"message":"m"}}"#,
            "the reply's id is not 1",
        ),
        (
            br#"{"id":2,"error":{"code":-32700,"message":"m"}}"#,
            "the reply's id is not 1",
        ),
        (
            br#"{"error":{"code":-32600,"message":"m"}}"#,
            "the reply's id is not 1",
        ),
        (br#"{"id":1,"result":[]}"#, "the result is not an object"),
        (br#"{"id":1,"result":null}"#, "the result is not an object"),
        (br#"{"id":1,"error":"boom"}"#, "the error is not an object"),
        (
            br#"{"id":1,"error":{"code":"101","message":"m"}}"#,
            "the error's code is not an integer",
        ),
        (
            br#"{"id":1,"error":{"code":101}}"#,
            "the error's message is not a string",
        ),
        (
            br#"{"id":1,"error":{"code":101,"message":"m","domain":"client"}}"#,
            "the error already has a domain key",
        ),
        (
            br#"{"id":1,"result":{},"result":{}}"#,
            "the key /result repeats at byte 20",
        ),
        (
            br#"{"id":1,"result":{"a":{"b":1,"b":2}}}"#,
            "the key /result/a/b repeats at byte 29",
        ),
    ];
    for (line, detail) in cases {
        assert_eq!(
            bad_reply_detail(line),
            detail,
            "{:?}",
            String::from_utf8_lossy(line)
        );
    }
}

#[test]
fn a_code_must_be_an_integer_and_a_message_a_string() {
    for code in ["101.0", "1e2", "1E2", "true", "null", "{}"] {
        let line = format!(r#"{{"id":1,"error":{{"code":{code},"message":"m"}}}}"#);
        assert_eq!(
            bad_reply_detail(line.as_bytes()),
            "the error's code is not an integer",
            "code {code}"
        );
    }
    for message in ["5", "null", r#"["m"]"#] {
        let line = format!(r#"{{"id":1,"error":{{"code":101,"message":{message}}}}}"#);
        assert_eq!(
            bad_reply_detail(line.as_bytes()),
            "the error's message is not a string",
            "message {message}"
        );
    }
    for domain in [r#""kernel""#, "null", "1"] {
        let line = format!(r#"{{"id":1,"error":{{"code":101,"message":"m","domain":{domain}}}}}"#);
        assert_eq!(
            bad_reply_detail(line.as_bytes()),
            "the error already has a domain key",
            "domain {domain}"
        );
    }
}

#[test]
fn a_bad_reply_is_written_as_a_client_error_with_its_detail() {
    let error: ClientError = judge_reply(b"[1]").expect_err("an array is not a reply");
    let printed = emitted_stderr(ErrorBody::Client(error));
    assert_eq!(printed["error"]["domain"], "client");
    assert_eq!(printed["error"]["code"], "bad_reply");
    assert_eq!(printed["error"]["detail"], "the reply is not an object");
    assert!(printed["error"]["message"].is_string());
}

#[test]
fn a_reply_line_is_read_up_to_its_newline() {
    let mut reader = Script::new(vec![
        Step::Bytes(br#"{"id":1,"#.to_vec()),
        Step::Bytes(br#""result":{}}"#.to_vec()),
        Step::Bytes(b"\nrest".to_vec()),
    ]);
    let line = read_reply(&mut reader).expect("the line ends with a newline");
    assert_eq!(line, br#"{"id":1,"result":{}}"#);
    assert_eq!(
        result_of(std::str::from_utf8(&line).expect("UTF-8")),
        parsed("{}")
    );
}

#[test]
fn bytes_after_the_newline_are_not_read() {
    let mut input: &[u8] = b"{\"id\":1,\"result\":{}}\n{\"id\":1,\"error\":not json}\nmore";
    let line = read_reply(&mut input).expect("the line ends with a newline");
    assert_eq!(line, br#"{"id":1,"result":{}}"#);
    assert_eq!(input, b"{\"id\":1,\"error\":not json}\nmore");
    assert!(matches!(judge_reply(&line), Ok(Reply::Result(_))));

    let mut reader = Script::new(vec![
        Step::Bytes(b"{\"id\":1,\"result\":{}}\n".to_vec()),
        Step::Fail(io::Error::other("a read after the newline")),
        Step::Bytes(b"tail".to_vec()),
    ]);
    assert_eq!(
        read_reply(&mut reader).expect("the line ends with a newline"),
        br#"{"id":1,"result":{}}"#
    );
    assert!(matches!(reader.steps.front(), Some(Step::Fail(_))));
    assert_eq!(reader.left(), b"tail");
}

#[test]
fn an_empty_line_is_a_line() {
    let mut input: &[u8] = b"\nmore";
    assert_eq!(read_reply(&mut input), Ok(Vec::new()));
    assert_eq!(input, b"more");
}

#[test]
fn end_of_input_before_a_newline_is_closed() {
    let mut empty: &[u8] = b"";
    assert_eq!(read_reply(&mut empty), Err(ReadFailure::Closed));
    let mut partial: &[u8] = br#"{"id":1,"result":{}}"#;
    assert_eq!(read_reply(&mut partial), Err(ReadFailure::Closed));

    let mut the_bound_then_the_end = Chunked {
        bytes: vec![b'x'; MAX_REPLY_BYTES],
        at: 0,
    };
    assert_eq!(
        read_reply(&mut the_bound_then_the_end),
        Err(ReadFailure::Closed)
    );
    let mut one_byte_more_then_the_end = Chunked {
        bytes: vec![b'x'; MAX_REPLY_BYTES + 1],
        at: 0,
    };
    assert_eq!(
        read_reply(&mut one_byte_more_then_the_end),
        Err(ReadFailure::TooLarge)
    );
}

#[test]
fn read_errors_are_classified() {
    let cases = [
        (io::ErrorKind::WouldBlock, ReadFailure::TimedOut),
        (io::ErrorKind::TimedOut, ReadFailure::TimedOut),
        (
            io::ErrorKind::ConnectionReset,
            ReadFailure::Transport { errno: None },
        ),
    ];
    for (kind, expected) in cases {
        let mut reader = Script::new(vec![
            Step::Bytes(br#"{"id":"#.to_vec()),
            Step::Fail(io::Error::from(kind)),
        ]);
        assert_eq!(read_reply(&mut reader), Err(expected), "{kind:?}");
    }
    let mut reader = Script::new(vec![Step::Fail(io::Error::from_raw_os_error(54))]);
    assert_eq!(
        read_reply(&mut reader),
        Err(ReadFailure::Transport { errno: Some(54) })
    );
}

#[test]
fn an_interrupted_read_is_retried() {
    let mut reader = Script::new(vec![
        Step::Fail(io::Error::from(io::ErrorKind::Interrupted)),
        Step::Bytes(b"{\"id\":1,\"result\":{}}\n".to_vec()),
    ]);
    assert_eq!(
        read_reply(&mut reader),
        Ok(br#"{"id":1,"result":{}}"#.to_vec())
    );

    let mut mid_line = Script::new(vec![
        Step::Bytes(br#"{"id":1,"#.to_vec()),
        Step::Fail(io::Error::from(io::ErrorKind::Interrupted)),
        Step::Bytes(b"\"result\":{}}\n".to_vec()),
    ]);
    assert_eq!(
        read_reply(&mut mid_line),
        Ok(br#"{"id":1,"result":{}}"#.to_vec())
    );
}

#[test]
fn a_line_at_the_bound_is_read_and_one_byte_more_is_too_large() {
    let mut at_the_bound = Chunked {
        bytes: line_of(MAX_REPLY_BYTES),
        at: 0,
    };
    let line = read_reply(&mut at_the_bound).expect("a line of the bound's length is read");
    assert_eq!(line.len(), MAX_REPLY_BYTES);
    assert_eq!(at_the_bound.at, MAX_REPLY_BYTES + 1);

    let mut past_the_bound = Chunked {
        bytes: line_of(MAX_REPLY_BYTES + 1),
        at: 0,
    };
    assert_eq!(read_reply(&mut past_the_bound), Err(ReadFailure::TooLarge));
    assert!(past_the_bound.at <= MAX_REPLY_BYTES + 1);

    let mut whole: &[u8] = &line_of(MAX_REPLY_BYTES + 1);
    assert_eq!(read_reply(&mut whole), Err(ReadFailure::TooLarge));
}

#[test]
fn the_bound_is_checked_while_reading() {
    let mut endless = Endless {
        chunk: vec![b'x'; CHUNK],
        taken: 0,
        gives_out_after: 2 * MAX_REPLY_BYTES + 1,
    };
    assert_eq!(read_reply(&mut endless), Err(ReadFailure::TooLarge));
    assert!(
        endless.taken <= MAX_REPLY_BYTES + 1,
        "took {} bytes of an endless line",
        endless.taken
    );
}

#[test]
fn unanswered_by_state_change() {
    let max_bytes = number(&MAX_REPLY_BYTES.to_string());
    let cases = [
        (
            ReadFailure::Closed,
            false,
            "no_reply",
            "cause",
            text("closed"),
            1,
        ),
        (
            ReadFailure::TimedOut,
            false,
            "no_reply",
            "cause",
            text("deadline"),
            1,
        ),
        (
            ReadFailure::Transport { errno: Some(54) },
            false,
            "no_reply",
            "cause",
            text("transport"),
            1,
        ),
        (
            ReadFailure::TooLarge,
            false,
            "reply_too_large",
            "max_bytes",
            max_bytes,
            2,
        ),
        (
            ReadFailure::Closed,
            true,
            "outcome_unknown",
            "cause",
            text("closed"),
            3,
        ),
        (
            ReadFailure::TimedOut,
            true,
            "outcome_unknown",
            "cause",
            text("deadline"),
            3,
        ),
        (
            ReadFailure::Transport { errno: None },
            true,
            "outcome_unknown",
            "cause",
            text("transport"),
            3,
        ),
        (
            ReadFailure::TooLarge,
            true,
            "outcome_unknown",
            "cause",
            text("reply_too_large"),
            3,
        ),
    ];
    for (failure, changes_state, code, key, value, exit) in cases {
        let error = unanswered(failure, changes_state);
        let label = format!("{failure:?} with changes_state {changes_state}");
        assert_eq!(error.code().as_str(), code, "{label}");
        assert_eq!(error.code().exit().code(), exit, "{label}");
        assert_eq!(
            error.fields(),
            [(key.to_string(), value)].as_slice(),
            "{label}"
        );
        assert!(!error.message().is_empty(), "{label}");
    }
}

#[test]
fn a_transport_failure_names_its_errno_in_the_message() {
    let named = unanswered(ReadFailure::Transport { errno: Some(54) }, false);
    assert!(named.message().contains("errno 54"), "{}", named.message());
    let unnamed = unanswered(ReadFailure::Transport { errno: None }, true);
    assert!(
        !unnamed.message().contains("errno"),
        "{}",
        unnamed.message()
    );
}

#[test]
fn the_request_line() {
    assert_eq!(
        request_line("plasmosome.status", Vec::new()),
        Ok("{\"id\":1,\"method\":\"plasmosome.status\",\"params\":{}}\n".to_string())
    );
    assert_eq!(
        request_line(
            "plasmosome.status",
            vec![("name".to_string(), text("work"))]
        ),
        Ok(
            "{\"id\":1,\"method\":\"plasmosome.status\",\"params\":{\"name\":\"work\"}}\n"
                .to_string()
        )
    );
}

#[test]
fn a_request_line_escapes_controls_and_is_one_line() {
    let escape = char::from_u32(0x1b).expect("U+001B is a character");
    let line = request_line(
        "cell.exec",
        vec![(
            "argv".to_string(),
            Json::Array(vec![text(&format!("a{escape}\nb"))]),
        )],
    )
    .expect("the params repeat no key");
    assert!(line.ends_with('\n'));
    assert_eq!(line.matches('\n').count(), 1);
    assert!(!line.contains(escape));
    let parsed_back: serde_json::Value =
        serde_json::from_str(line.trim_end()).expect("the request line is JSON");
    assert_eq!(parsed_back["params"]["argv"][0], format!("a{escape}\nb"));
}

#[test]
fn a_request_line_escapes_its_method() {
    let backslash = char::from_u32(0x5c).expect("U+005C is a character");
    let escape = char::from_u32(0x1b).expect("U+001B is a character");
    let newline = char::from_u32(0x0a).expect("U+000A is a character");
    let method = format!("a\"b{backslash}c{escape}d{newline}e");
    let line = request_line(&method, Vec::new()).expect("no params repeat a key");
    let written =
        format!("\"a{backslash}\"b{backslash}{backslash}c{backslash}u001bd{backslash}u000ae\"");
    assert_eq!(
        line,
        format!("{{\"id\":1,\"method\":{written},\"params\":{{}}}}{newline}")
    );
    let parsed_back: serde_json::Value =
        serde_json::from_str(line.trim_end()).expect("the request line is JSON");
    assert_eq!(parsed_back["method"], method);
}

#[test]
fn a_request_line_refuses_a_repeated_key() {
    let repeated = vec![
        ("name".to_string(), text("a")),
        ("name".to_string(), text("b")),
    ];
    assert_eq!(
        request_line("plasmosome.status", repeated),
        Err(vec![
            ("name".to_string(), text("a")),
            ("name".to_string(), text("b")),
        ])
    );
    let nested = vec![(
        "x".to_string(),
        Json::Object(vec![
            ("k".to_string(), Json::Null),
            ("k".to_string(), Json::Null),
        ]),
    )];
    assert!(request_line("plasmosome.status", nested).is_err());
}
