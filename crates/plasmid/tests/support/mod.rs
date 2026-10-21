use std::path::Path;
use std::process::Output;

use serde_json::{Map, Value};

pub fn text(stream: &[u8]) -> String {
    String::from_utf8(stream.to_vec()).expect("the stream is UTF-8")
}

pub fn entries(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .expect("the directory is readable")
        .map(|entry| {
            entry
                .expect("a directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn keys(object: &Map<String, Value>) -> Vec<&str> {
    object.keys().map(String::as_str).collect()
}

pub fn envelope(output: &Output) -> (String, Map<String, Value>) {
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = text(&output.stderr);
    assert!(
        stderr.ends_with('\n') && stderr.matches('\n').count() == 1,
        "stderr is one line: {stderr:?}"
    );
    let value: Value = serde_json::from_str(&stderr)
        .unwrap_or_else(|error| panic!("stderr is one JSON envelope: {error}: {stderr:?}"));
    let Value::Object(mut envelope) = value else {
        panic!("the envelope is an object: {stderr:?}");
    };
    let mut seen = keys(&envelope);
    seen.sort_unstable();
    assert_eq!(seen, ["command", "error", "schema"], "{stderr:?}");
    assert_eq!(envelope["schema"], 1, "{stderr:?}");
    let Some(Value::String(command)) = envelope.remove("command") else {
        panic!("command is a string: {stderr:?}");
    };
    let Some(Value::Object(error)) = envelope.remove("error") else {
        panic!("error is an object: {stderr:?}");
    };
    assert_eq!(error["domain"], "client", "{stderr:?}");
    assert!(error["message"].is_string(), "{stderr:?}");
    (command, error)
}

pub fn usage_error(output: &Output) -> Map<String, Value> {
    assert_eq!(output.status.code(), Some(64), "{output:?}");
    let (command, error) = envelope(output);
    assert_eq!(command, "invalid", "{output:?}");
    assert_eq!(error["code"], "invalid_argument", "{output:?}");
    for key in keys(&error) {
        assert!(
            ["domain", "code", "message", "argument"].contains(&key),
            "{key} in {error:?}"
        );
    }
    error
}
