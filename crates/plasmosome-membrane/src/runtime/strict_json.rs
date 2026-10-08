use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::cell::RefCell;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StrictJsonError {
    NotJson {
        line: usize,
        column: usize,
        reason: String,
    },
    DuplicateKey {
        path: String,
    },
}

impl std::fmt::Display for StrictJsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StrictJsonError::NotJson {
                line,
                column,
                reason,
            } => write!(
                f,
                "refused as JSON at line {line}, column {column}: {reason}"
            ),
            StrictJsonError::DuplicateKey { path } => {
                write!(f, "duplicate object key at {path:?}")
            }
        }
    }
}

impl std::error::Error for StrictJsonError {}

pub(crate) fn parse_value(bytes: &[u8]) -> Result<Value, StrictJsonError> {
    let duplicate = RefCell::new(None);
    let mut reader = serde_json::Deserializer::from_slice(bytes);
    let read = Strict {
        at: Location::Root,
        duplicate: &duplicate,
    }
    .deserialize(&mut reader)
    .and_then(|value| reader.end().map(|()| value));
    read.map_err(|error| match duplicate.into_inner() {
        Some(path) => StrictJsonError::DuplicateKey { path },
        None => StrictJsonError::NotJson {
            line: error.line(),
            column: error.column(),
            reason: reason_without_position(&error),
        },
    })
}

fn reason_without_position(error: &serde_json::Error) -> String {
    let shown = error.to_string();
    let position = format!(" at line {} column {}", error.line(), error.column());
    match shown.strip_suffix(&position) {
        Some(reason) => reason.to_owned(),
        None => shown,
    }
}

#[derive(Clone, Copy)]
enum Location<'a> {
    Root,
    Key(&'a Location<'a>, &'a str),
    Index(&'a Location<'a>, usize),
}

impl Location<'_> {
    fn pointer(&self) -> String {
        match self {
            Location::Root => String::new(),
            Location::Key(parent, key) => format!(
                "{}/{}",
                parent.pointer(),
                key.replace('~', "~0").replace('/', "~1")
            ),
            Location::Index(parent, index) => format!("{}/{index}", parent.pointer()),
        }
    }
}

struct Strict<'a> {
    at: Location<'a>,
    duplicate: &'a RefCell<Option<String>>,
}

impl<'de> DeserializeSeed<'de> for Strict<'_> {
    type Value = Value;

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Strict<'_> {
    type Value = Value;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("a number that is not finite"))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<Value, A::Error> {
        let mut read = Vec::new();
        while let Some(item) = items.next_element_seed(Strict {
            at: Location::Index(&self.at, read.len()),
            duplicate: self.duplicate,
        })? {
            read.push(item);
        }
        Ok(Value::Array(read))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Value, A::Error> {
        let mut read = Map::new();
        while let Some(key) = entries.next_key::<String>()? {
            let at = Location::Key(&self.at, &key);
            if read.contains_key(&key) {
                self.duplicate.replace(Some(at.pointer()));
                return Err(de::Error::custom("duplicate object key"));
            }
            let value = entries.next_value_seed(Strict {
                at,
                duplicate: self.duplicate,
            })?;
            read.insert(key, value);
        }
        Ok(Value::Object(read))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn duplicate_at(text: &str) -> String {
        match parse_value(text.as_bytes()) {
            Err(StrictJsonError::DuplicateKey { path }) => path,
            other => panic!("{text} refuses as a duplicate key, got {other:?}"),
        }
    }

    fn assert_refused_as_serde_json_refuses(text: &str) {
        let expected = serde_json::from_str::<serde_json::Value>(text)
            .expect_err("serde_json refuses the same text");
        match parse_value(text.as_bytes()) {
            Err(StrictJsonError::NotJson {
                line,
                column,
                reason,
            }) => {
                assert_eq!(
                    (line, column),
                    (expected.line(), expected.column()),
                    "{text:?}"
                );
                assert_eq!(
                    format!("{reason} at line {line} column {column}"),
                    expected.to_string(),
                    "{text:?} keeps serde_json's reason"
                );
            }
            other => panic!("{text:?} is not JSON, got {other:?}"),
        }
    }

    fn not_json(line: usize, column: usize, reason: &str) -> StrictJsonError {
        StrictJsonError::NotJson {
            line,
            column,
            reason: reason.to_string(),
        }
    }

    #[test]
    fn a_repeated_top_level_key_refuses_naming_it() {
        assert_eq!(duplicate_at(r#"{"a":1,"a":2}"#), "/a");
    }

    #[test]
    fn a_key_repeated_after_another_key_refuses() {
        assert_eq!(
            duplicate_at(r#"{"kernel":1,"vcpus":2,"kernel":3}"#),
            "/kernel"
        );
    }

    #[test]
    fn a_repeated_key_in_a_nested_object_refuses_with_its_path() {
        assert_eq!(duplicate_at(r#"{"k":{"s":1,"s":1}}"#), "/k/s");
    }

    #[test]
    fn a_repeated_key_inside_an_array_element_refuses_with_its_index() {
        assert_eq!(duplicate_at(r#"{"l":[{"p":1},{"p":1,"p":2}]}"#), "/l/1/p");
    }

    #[test]
    fn sibling_objects_may_reuse_a_key() {
        assert_eq!(
            parse_value(br#"{"a":{"x":1},"b":{"x":1}}"#),
            Ok(json!({"a": {"x": 1}, "b": {"x": 1}}))
        );
    }

    #[test]
    fn a_key_spelled_with_an_escape_is_the_same_key() {
        assert_eq!(duplicate_at(r#"{"a":1,"\u0061":2}"#), "/a");
    }

    #[test]
    fn text_after_the_single_value_refuses() {
        for text in ["{} {}", "{}x"] {
            assert_refused_as_serde_json_refuses(text);
        }
        assert_eq!(
            parse_value(b"{} {}"),
            Err(not_json(1, 4, "trailing characters"))
        );
        assert_eq!(parse_value(b"{} \n"), Ok(json!({})));
    }

    #[test]
    fn every_value_kind_reads_as_serde_json_reads_it() {
        let text = r#"{"t":true,"f":false,"n":null,"u":18446744073709551615,"i":-9223372036854775808,"x":-0.5,"e":1e300,"s":"\u00e9\n","a":[[],{},[1,"2",[3]]],"o":{"":{"k":"v"}}}"#;
        assert_eq!(
            parse_value(text.as_bytes()),
            Ok(serde_json::from_str::<serde_json::Value>(text).expect("serde_json reads it"))
        );
    }

    #[test]
    fn a_key_holding_a_slash_or_a_tilde_is_escaped_in_the_path() {
        assert_eq!(duplicate_at(r#"{"a/b~c":{"x":1,"x":2}}"#), "/a~1b~0c/x");
    }

    #[test]
    fn malformed_text_reports_the_position_serde_json_reports() {
        for text in [
            "",
            "{",
            "{\n  \"a\": tru }",
            "[1,]",
            "{\"a\" 1}",
            "\"\u{0}\"",
        ] {
            assert_refused_as_serde_json_refuses(text);
        }
    }

    #[test]
    fn a_single_value_refused_for_its_content_keeps_the_reason() {
        assert_refused_as_serde_json_refuses("[1e400]");
        assert_eq!(
            parse_value(b"[1e400]"),
            Err(not_json(1, 6, "number out of range"))
        );
        assert_eq!(
            parse_value(b"[1e400]")
                .expect_err("an infinite number refuses")
                .to_string(),
            "refused as JSON at line 1, column 6: number out of range"
        );
    }

    #[test]
    fn bytes_that_are_not_utf8_refuse() {
        assert!(matches!(
            parse_value(b"{\"a\":\"\xff\"}"),
            Err(StrictJsonError::NotJson { line: 1, .. })
        ));
    }

    #[test]
    fn nesting_past_the_recursion_limit_refuses_instead_of_overflowing() {
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        assert_refused_as_serde_json_refuses(&deep);
        assert_eq!(
            parse_value(deep.as_bytes()),
            Err(not_json(1, 128, "recursion limit exceeded"))
        );
    }

    #[test]
    fn faults_describe_themselves() {
        assert_eq!(
            not_json(3, 7, "trailing characters").to_string(),
            "refused as JSON at line 3, column 7: trailing characters"
        );
        assert_eq!(
            StrictJsonError::DuplicateKey {
                path: "/l/1/p".to_string()
            }
            .to_string(),
            "duplicate object key at \"/l/1/p\""
        );
    }

    #[test]
    fn a_duplicate_path_is_quoted_and_escaped_when_shown() {
        let shown = |text: &str| {
            parse_value(text.as_bytes())
                .expect_err("a repeated key refuses")
                .to_string()
        };
        assert_eq!(
            shown(r#"{"a\nb":1,"a\nb":2}"#),
            r#"duplicate object key at "/a\nb""#
        );
        assert_eq!(
            shown(r#"{"\u001b[31mX":1,"\u001b[31mX":2}"#),
            r#"duplicate object key at "/\u{1b}[31mX""#
        );
        assert_eq!(shown(r#"{"":1,"":2}"#), r#"duplicate object key at "/""#);
        assert_eq!(
            shown(r#"{"a":{"":1,"":2}}"#),
            r#"duplicate object key at "/a/""#
        );
    }
}
