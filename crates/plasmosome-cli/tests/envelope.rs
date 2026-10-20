use std::io::{self, Write};

use plasmosome_cli::json::{Json, scan};
use plasmosome_cli::{
    ClientCode, ClientError, EmitError, Envelope, ErrorBody, Form, Instance, KernelError, Stream,
    emit,
};

const SOCKET: &str = "/Users/me/.plasmosome/instances/work/control.uds";

const RESULT: &str = r#"{"name":"work","state":"running","ready":true,"controller":{"uptime_ms":9142,"ledger_generation":0},"cells":[],"big":18446744073709551616,"unknown":{"n":1.50}}"#;

const KERNEL_ERROR: &str = r#"{"code":101,"message":"no plasmid named x","target":"plasmosome other","fix":"run plasmid list","field":"network.hosts"}"#;

fn character(code_point: u32) -> char {
    char::from_u32(code_point).expect("a test names a valid code point")
}

fn parsed(text: &str) -> Json {
    scan(text.as_bytes()).expect("the fixture is valid JSON")
}

fn instance(name: Option<&str>) -> Option<Instance> {
    Some(Instance {
        name: name.map(str::to_string),
        socket: SOCKET.to_string(),
    })
}

fn success(instance: Option<Instance>) -> Envelope {
    Envelope {
        command: "plasmosome.status".to_string(),
        instance,
        outcome: Ok(parsed(RESULT)),
    }
}

fn kernel_failure(error: &str) -> Envelope {
    Envelope {
        command: "plasmosome.status".to_string(),
        instance: instance(Some("other")),
        outcome: Err(ErrorBody::Kernel(
            KernelError::new(parsed(error)).expect("the fixture is an object without domain"),
        )),
    }
}

fn client_failure(code: ClientCode, message: &str, fields: Vec<(&str, Json)>) -> Envelope {
    Envelope {
        command: "invalid".to_string(),
        instance: None,
        outcome: Err(ErrorBody::Client(ClientError {
            code,
            message: message.to_string(),
            fields: fields
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        })),
    }
}

fn emitted(envelope: &Envelope, form: Form) -> (String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    emit(envelope, form, &mut stdout, &mut stderr).expect("writing to memory succeeds");
    (
        String::from_utf8(stdout).expect("stdout is UTF-8"),
        String::from_utf8(stderr).expect("stderr is UTF-8"),
    )
}

#[test]
fn the_envelope_shape() {
    let named = format!(
        r#"{{"schema":1,"command":"plasmosome.status","instance":{{"name":"work","socket":"{SOCKET}"}},"result":{RESULT}}}"#
    );
    assert_eq!(
        emitted(&success(instance(Some("work"))), Form::Json),
        (format!("{named}\n"), String::new())
    );

    let unnamed = format!(
        r#"{{"schema":1,"command":"plasmosome.status","instance":{{"socket":"{SOCKET}"}},"result":{RESULT}}}"#
    );
    assert_eq!(
        emitted(&success(instance(None)), Form::Json),
        (format!("{unnamed}\n"), String::new())
    );

    let no_socket = format!(r#"{{"schema":1,"command":"plasmosome.status","result":{RESULT}}}"#);
    assert_eq!(
        emitted(&success(None), Form::Json),
        (format!("{no_socket}\n"), String::new())
    );
}

#[test]
fn a_kernel_error_keeps_every_field_after_its_domain() {
    let expected = format!(
        r#"{{"schema":1,"command":"plasmosome.status","instance":{{"name":"other","socket":"{SOCKET}"}},"error":{{"domain":"kernel","code":101,"message":"no plasmid named x","target":"plasmosome other","fix":"run plasmid list","field":"network.hosts"}}}}"#
    );
    assert_eq!(
        emitted(&kernel_failure(KERNEL_ERROR), Form::Json),
        (String::new(), format!("{expected}\n"))
    );
}

#[test]
fn a_client_error_writes_its_code_and_fields() {
    let envelope = client_failure(
        ClientCode::InvalidArgument,
        "--deadline-ms takes an integer",
        vec![("argument", Json::String("--deadline-ms".to_string()))],
    );
    let expected = r#"{"schema":1,"command":"invalid","error":{"domain":"client","code":"invalid_argument","message":"--deadline-ms takes an integer","argument":"--deadline-ms"}}"#;
    assert_eq!(
        emitted(&envelope, Form::Json),
        (String::new(), format!("{expected}\n"))
    );
}

#[test]
fn a_kernel_error_is_an_object_without_domain() {
    let array = parsed("[1]");
    assert_eq!(KernelError::new(parsed("[1]")), Err(array));
    let with_domain = parsed(r#"{"code":101,"message":"m","domain":"x"}"#);
    assert_eq!(
        KernelError::new(parsed(r#"{"code":101,"message":"m","domain":"x"}"#)),
        Err(with_domain)
    );
    let error = KernelError::new(parsed(r#"{"code":101,"message":"m"}"#)).expect("an object");
    assert_eq!(
        error.members(),
        &[
            ("code".to_string(), Json::Number("101".to_string())),
            ("message".to_string(), Json::String("m".to_string())),
        ]
    );
}

#[test]
fn the_human_form_of_a_failure() {
    let error = format!(
        r#"{{"code":101,"message":"no plasmid{}x","fix":"run{}it","target":"t"}}"#,
        character(0x1b),
        character(0x202e)
    );
    assert_eq!(
        emitted(&kernel_failure(&error), Form::Human),
        (
            String::new(),
            "no plasmid<U+001B>x\nfix: run<U+202E>it\n(kernel 101)\n".to_string()
        )
    );

    let not_served = client_failure(
        ClientCode::NotServed,
        "plasmosome cell list is not served yet",
        vec![("method", Json::String("cell.list".to_string()))],
    );
    assert_eq!(
        emitted(&not_served, Form::Human),
        (
            String::new(),
            "plasmosome cell list is not served yet\n(client not_served)\n".to_string()
        )
    );

    let unsafe_socket = client_failure(
        ClientCode::UnsafeSocket,
        "the instance directory is open to others",
        vec![
            ("path", Json::String("/x".to_string())),
            ("rule", Json::String("mode".to_string())),
            (
                "fix",
                Json::String(format!("chmod 700 /x{}", character(0x9b))),
            ),
        ],
    );
    assert_eq!(
        emitted(&unsafe_socket, Form::Human),
        (
            String::new(),
            "the instance directory is open to others\nfix: chmod 700 /x<U+009B>\n(client unsafe_socket)\n"
                .to_string()
        )
    );
}

#[test]
fn the_human_form_of_a_success() {
    let envelope = Envelope {
        command: "plasmosome.status".to_string(),
        instance: Some(Instance {
            name: Some(format!("work{}", character(0x7f))),
            socket: "/s".to_string(),
        }),
        outcome: Ok(parsed(r#"{"ready":true,"cells":[]}"#)),
    };
    let expected = [
        "{",
        "  \"schema\": 1,",
        "  \"command\": \"plasmosome.status\",",
        "  \"instance\": {",
        "    \"name\": \"work<U+007F>\",",
        "    \"socket\": \"/s\"",
        "  },",
        "  \"result\": {",
        "    \"ready\": true,",
        "    \"cells\": []",
        "  }",
        "}",
        "",
    ]
    .join("\n");
    assert_eq!(emitted(&envelope, Form::Human), (expected, String::new()));
}

#[test]
fn the_json_form_is_one_line_on_each_stream() {
    let text = format!(r#"{{"a":"line{}break"}}"#, "\n".escape_default());
    let (stdout, _) = emitted(
        &Envelope {
            command: "plasmosome.status".to_string(),
            instance: None,
            outcome: Ok(parsed(&text)),
        },
        Form::Json,
    );
    assert_eq!(stdout.matches('\n').count(), 1);
    assert!(stdout.ends_with('\n'));
    let (_, stderr) = emitted(&kernel_failure(KERNEL_ERROR), Form::Json);
    assert_eq!(stderr.matches('\n').count(), 1);
    assert!(stderr.ends_with('\n'));
}

struct Broken;

impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

fn failed_stream(result: Result<(), EmitError>) -> Stream {
    let error = result.expect_err("the stream refuses every write");
    assert_eq!(error.error.kind(), io::ErrorKind::BrokenPipe);
    error.stream
}

#[test]
fn a_failed_write_names_its_stream() {
    let mut sink = Vec::new();
    assert_eq!(
        failed_stream(emit(&success(None), Form::Json, &mut Broken, &mut sink)),
        Stream::Stdout
    );
    assert_eq!(
        failed_stream(emit(
            &kernel_failure(KERNEL_ERROR),
            Form::Json,
            &mut sink,
            &mut Broken
        )),
        Stream::Stderr
    );
    assert!(sink.is_empty());
    emit(&success(None), Form::Json, &mut sink, &mut Broken)
        .expect("a success never touches stderr");
    emit(
        &kernel_failure(KERNEL_ERROR),
        Form::Human,
        &mut Broken,
        &mut sink,
    )
    .expect("a failure never touches stdout");
}
