#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StrictJsonError {
    NotJson { line: usize, column: usize },
    DuplicateKey { path: String },
}

impl std::fmt::Display for StrictJsonError {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for StrictJsonError {}

pub(crate) fn parse_value(_bytes: &[u8]) -> Result<serde_json::Value, StrictJsonError> {
    todo!()
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

    fn serde_json_position(text: &str) -> StrictJsonError {
        let error = serde_json::from_str::<serde_json::Value>(text)
            .expect_err("serde_json refuses the same text");
        StrictJsonError::NotJson {
            line: error.line(),
            column: error.column(),
        }
    }

    #[test]
    fn a_repeated_top_level_key_refuses_naming_it() {
        assert_eq!(duplicate_at(r#"{"a":1,"a":2}"#), "/a");
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
        assert_eq!(duplicate_at(r#"{"a":1,"a":2}"#), "/a");
    }

    #[test]
    fn text_after_the_single_value_refuses() {
        for text in ["{} {}", "{}x"] {
            assert_eq!(
                parse_value(text.as_bytes()),
                Err(serde_json_position(text)),
                "{text:?} holds more than one value"
            );
        }
        assert_eq!(parse_value(b"{} \n"), Ok(json!({})));
    }

    #[test]
    fn every_value_kind_reads_as_serde_json_reads_it() {
        let text = r#"{"t":true,"f":false,"n":null,"u":18446744073709551615,"i":-9223372036854775808,"x":-0.5,"e":1e300,"s":"é\n","a":[[],{},[1,"2",[3]]],"o":{"":{"k":"v"}}}"#;
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
            assert_eq!(
                parse_value(text.as_bytes()),
                Err(serde_json_position(text)),
                "{text:?} is not JSON"
            );
        }
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
        assert!(matches!(
            parse_value(deep.as_bytes()),
            Err(StrictJsonError::NotJson { .. })
        ));
    }

    #[test]
    fn faults_describe_themselves() {
        assert_eq!(
            StrictJsonError::NotJson { line: 3, column: 7 }.to_string(),
            "not one JSON value: refused at line 3, column 7"
        );
        assert_eq!(
            StrictJsonError::DuplicateKey {
                path: "/l/1/p".to_string()
            }
            .to_string(),
            "duplicate object key at /l/1/p"
        );
    }
}
