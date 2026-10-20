#[cfg(test)]
mod tests {
    use super::ConfigRecordRefusal::*;
    use super::*;
    use crate::runtime::cell_id::{MAX_CELL_NUMBER, format_cell_id};
    use crate::runtime::fixtures::{LIMITS, artifact, valid_recipe};
    use crate::runtime::path::PathError;
    use crate::runtime::recipe::RecipeRefusal;
    use crate::runtime::strict_json;
    use proptest::prelude::*;
    use serde_json::{Value, json};

    const BOUND: usize = 1_048_576;

    fn recipe_from(value: &Value) -> RuntimeRecipe {
        RuntimeRecipe::parse(format!("{value}\n").as_bytes(), &LIMITS)
            .unwrap_or_else(|refusal| panic!("the fixture recipe parses, got {refusal:?}"))
    }

    fn root(text: &str) -> RecipePath {
        RecipePath::parse(text).unwrap_or_else(|fault| panic!("{text:?} parses, got {fault:?}"))
    }

    fn config() -> SupervisorConfig {
        SupervisorConfig {
            cell: format_cell_id(7),
            instance_root: root("/var/plasmosome"),
            runtime_recipe: recipe_from(&valid_recipe()),
            boot_deadline_ms: 60_000,
        }
    }

    fn encoded(config: &SupervisorConfig) -> Vec<u8> {
        config
            .encode()
            .unwrap_or_else(|refusal| panic!("{config:?} encodes, got {refusal:?}"))
    }

    fn record(edits: &[(&str, Option<Value>)]) -> Vec<u8> {
        let mut record =
            strict_json::parse_value(&encoded(&config())).expect("an encoded record is JSON");
        for (pointer, replacement) in edits {
            let (parent, key) = pointer.rsplit_once('/').expect("a JSON pointer");
            let fields = record
                .pointer_mut(parent)
                .and_then(Value::as_object_mut)
                .expect("the pointer names a key of an object");
            match replacement {
                Some(value) => fields.insert(key.to_owned(), value.clone()),
                None => fields.remove(key),
            };
        }
        format!("{record}\n").into_bytes()
    }

    fn set(pointer: &str, value: Value) -> (&str, Option<Value>) {
        (pointer, Some(value))
    }

    fn with(pointer: &str, value: Value) -> Vec<u8> {
        record(&[set(pointer, value)])
    }

    fn without(pointer: &str) -> Vec<u8> {
        record(&[(pointer, None)])
    }

    fn decoded(record: &[u8]) -> SupervisorConfig {
        SupervisorConfig::decode(record, &LIMITS).unwrap_or_else(|refusal| {
            panic!("{} refused: {refusal:?}", String::from_utf8_lossy(record))
        })
    }

    fn names_shown_by(refusal: &ConfigRecordRefusal) -> Vec<String> {
        match refusal {
            UnknownField { field } | DuplicateKey { field } => vec![format!("{field:?}")],
            MissingField { field } => vec![format!("`{field}`")],
            BadCell => vec!["`cell`".to_owned()],
            BadInstanceRoot { .. } => vec!["`instance_root`".to_owned()],
            Recipe(_) => vec!["`runtime_recipe`".to_owned()],
            BadBootDeadline => vec!["`boot_deadline_ms`".to_owned()],
            TooLarge { bytes } => vec![bytes.to_string()],
            NotOneLine | NotUtf8 | NotJson { .. } | NotAnObject => Vec::new(),
        }
    }

    fn refused(record: &[u8]) -> ConfigRecordRefusal {
        match SupervisorConfig::decode(record, &LIMITS) {
            Err(refusal) => {
                let shown = refusal.to_string();
                assert!(!shown.contains(['\n', '\r']), "{shown:?} is one line");
                for name in names_shown_by(&refusal) {
                    assert!(shown.contains(&name), "{shown:?} names {name}");
                }
                refusal
            }
            Ok(config) => panic!("{} decoded: {config:?}", String::from_utf8_lossy(record)),
        }
    }

    fn library_paths(count: usize, bytes: usize) -> Vec<usize> {
        let (each, extra) = (bytes / count, bytes % count);
        (0..count)
            .map(|index| each + usize::from(index < extra))
            .collect()
    }

    fn recipe_with_line_bytes(target: usize) -> RuntimeRecipe {
        let mut value = valid_recipe();
        value["libraries"] = json!([]);
        let base = recipe_from(&value).to_ndjson().len() - 1;
        let room = target + 1 - base;
        let count = room.div_ceil(1_088) + 1;
        let libraries: Vec<Value> = library_paths(count, room - 88 * count)
            .into_iter()
            .enumerate()
            .map(|(index, bytes)| {
                let head = format!("/opt/cell/lib/{index:06}/");
                artifact(&format!("{head}{}", "a".repeat(bytes - head.len())), 'e')
            })
            .collect();
        value["libraries"] = Value::Array(libraries);
        let recipe = recipe_from(&value);
        assert_eq!(
            recipe.to_ndjson().len() - 1,
            target,
            "the recipe line length"
        );
        recipe
    }

    fn config_with_record_bytes(bytes: usize) -> SupervisorConfig {
        let base = config();
        let overhead = encoded(&base).len() - base.runtime_recipe.to_ndjson().len();
        SupervisorConfig {
            runtime_recipe: recipe_with_line_bytes(bytes - overhead),
            ..base
        }
    }

    fn with_byte_in_instance_root(byte: u8) -> Vec<u8> {
        let mut text = with("/instance_root", json!("/var/X"));
        let at = text
            .iter()
            .position(|held| *held == b'X')
            .expect("X is in the record");
        text[at] = byte;
        text
    }

    #[test]
    fn the_record_is_one_line_with_its_keys_in_spec_order() {
        let config = config();
        let line = config.runtime_recipe.to_ndjson();
        let nested = std::str::from_utf8(&line[..line.len() - 1]).expect("UTF-8");
        let expected = format!(
            "{{\"cell\":\"cell-7\",\"instance_root\":\"/var/plasmosome\",\"runtime_recipe\":{nested},\"boot_deadline_ms\":60000}}\n"
        );
        assert_eq!(
            String::from_utf8(encoded(&config)).expect("UTF-8"),
            expected
        );
        assert_eq!(decoded(&encoded(&config)), config);
    }

    fn component() -> impl Strategy<Value = String> {
        let character = prop_oneof![
            Just('"'),
            Just('\\'),
            Just('\n'),
            Just('\r'),
            Just('\u{e9}'),
            Just('\u{2603}'),
            any::<char>(),
        ]
        .prop_filter("no / or NUL", |c| !matches!(c, '/' | '\0'));
        proptest::collection::vec(character, 1..6)
            .prop_map(String::from_iter)
            .prop_filter("not . or ..", |name| name != "." && name != "..")
    }

    proptest! {
        #[test]
        fn record_round_trips(
            number in 1..=MAX_CELL_NUMBER,
            names in proptest::collection::vec(component(), 1..5),
            boot_deadline_ms in 1..=u64::MAX,
            libraries in 0..4_usize,
        ) {
            let instance_root = root(&format!("/{}", names.join("/")));
            let cell = format_cell_id(number);
            let mut recipe = valid_recipe();
            recipe["writable_root"] = json!(format!("{}/cells/{cell}/runtime/root.img", instance_root.as_str()));
            recipe["libraries"] = (0..libraries)
                .map(|index| artifact(&format!("/opt/cell/lib/{index}.dylib"), 'e'))
                .collect();
            let config = SupervisorConfig {
                cell,
                instance_root,
                runtime_recipe: recipe_from(&recipe),
                boot_deadline_ms,
            };
            let record = encoded(&config);
            prop_assert_eq!(record.iter().filter(|byte| **byte == b'\n').count(), 1);
            prop_assert_eq!(SupervisorConfig::decode(&record, &LIMITS), Ok(config));
        }
    }

    #[test]
    fn a_record_that_is_not_exactly_one_line_refuses() {
        let line = encoded(&config());
        let body = &line[..line.len() - 1];
        for case in [
            body.to_vec(),
            [line.as_slice(), &line].concat(),
            [line.as_slice(), b"x"].concat(),
            [line.as_slice(), b"\n"].concat(),
            [b"{\n".as_slice(), &line[1..]].concat(),
            [body, b"\r\n"].concat(),
            [b"\r".as_slice(), &line].concat(),
            with_byte_in_instance_root(b'\r'),
            Vec::new(),
        ] {
            let shown = String::from_utf8_lossy(&case).into_owned();
            assert_eq!(refused(&case), NotOneLine, "{shown:?}");
        }
    }

    #[test]
    fn the_bound_counts_the_bytes_before_the_newline() {
        let padded = |length: usize| {
            let mut text = encoded(&config());
            text.pop();
            text.resize(length, b' ');
            text.push(b'\n');
            text
        };
        assert_eq!(decoded(&padded(BOUND)), config());
        assert_eq!(refused(&padded(BOUND + 1)), TooLarge { bytes: BOUND + 1 });
    }

    #[test]
    fn framing_is_checked_line_then_size_then_encoding() {
        let mut oversized = vec![b' '; BOUND + 1];
        oversized[0] = 0xff;
        assert_eq!(refused(&oversized), NotOneLine);
        oversized.push(b'\n');
        assert_eq!(refused(&oversized), TooLarge { bytes: BOUND + 1 });
        oversized[0] = b'\r';
        assert_eq!(refused(&oversized), NotOneLine);
    }

    #[test]
    fn a_record_that_is_not_utf8_refuses() {
        assert_eq!(refused(&with_byte_in_instance_root(0xff)), NotUtf8);
    }

    #[test]
    fn a_record_that_is_not_one_json_value_refuses() {
        for text in [
            &b"\n"[..],
            b"{\"cell\":\"cell-7\",\n",
            b"{} {}\n",
            b"[1e400]\n",
        ] {
            let shown = String::from_utf8_lossy(text).into_owned();
            assert!(matches!(refused(text), NotJson { .. }), "{shown:?}");
        }
        let detail = "trailing characters at line 1, column 4".to_owned();
        assert_eq!(refused(b"{} {}\n"), NotJson { detail });
    }

    #[test]
    fn a_repeated_key_refuses_at_the_top_and_inside_the_recipe() {
        let text = String::from_utf8(encoded(&config())).expect("UTF-8");
        let cell_twice = text.replacen('{', r#"{"cell":"cell-8","#, 1);
        let field = "cell".to_owned();
        assert_eq!(refused(cell_twice.as_bytes()), DuplicateKey { field });
        for (opening, repeated, field) in [
            (
                r#""runtime_recipe":{"#,
                r#""vcpus":3,"#,
                "runtime_recipe.vcpus",
            ),
            (
                r#""kernel":{"#,
                r#""path":"/opt/cell/other","#,
                "runtime_recipe.kernel.path",
            ),
        ] {
            assert!(text.contains(opening), "{opening}");
            let twice = text.replacen(opening, &format!("{opening}{repeated}"), 1);
            let field = field.to_owned();
            assert_eq!(refused(twice.as_bytes()), DuplicateKey { field });
        }
    }

    #[test]
    fn the_record_holds_exactly_its_four_keys() {
        for text in [&b"[]\n"[..], b"\"cell-7\"\n", b"null\n"] {
            assert_eq!(refused(text), NotAnObject);
        }
        let unknown = |field: &str| UnknownField {
            field: field.to_owned(),
        };
        assert_eq!(refused(&with("/extra", json!(1))), unknown("extra"));
        let two_unknown = record(&[set("/zeta", json!(1)), set("/alpha", json!(1))]);
        assert_eq!(refused(&two_unknown), unknown("alpha"));
        let missing_and_unknown = record(&[("/cell", None), set("/extra", json!(1))]);
        assert_eq!(refused(&missing_and_unknown), unknown("extra"));
        for field in [
            "cell",
            "instance_root",
            "runtime_recipe",
            "boot_deadline_ms",
        ] {
            let refusal = refused(&without(&format!("/{field}")));
            assert_eq!(refusal, MissingField { field });
        }
        let two_missing = record(&[("/boot_deadline_ms", None), ("/instance_root", None)]);
        let field = "instance_root";
        assert_eq!(refused(&two_missing), MissingField { field });
    }

    #[test]
    fn the_cell_is_an_id_spec_023_allocates() {
        for cell in [
            json!("cell-0"),
            json!("cell-01"),
            json!("cell-1/x"),
            json!("cell-12345678901"),
            json!("Cell-1"),
            json!(7),
            json!(null),
        ] {
            assert_eq!(refused(&with("/cell", cell.clone())), BadCell, "{cell}");
        }
        let largest = with("/cell", json!("cell-9999999999"));
        assert_eq!(decoded(&largest).cell, format_cell_id(MAX_CELL_NUMBER));
    }

    #[test]
    fn the_instance_root_is_a_canonical_absolute_path() {
        for (value, fault) in [
            (json!("relative/root"), Some(PathError::NotAbsolute)),
            (json!("/var//plasmosome"), Some(PathError::EmptyComponent)),
            (json!("/var/plasmosome/"), Some(PathError::TrailingSlash)),
            (json!("/"), Some(PathError::Root)),
            (json!(5), None),
        ] {
            let refusal = refused(&with("/instance_root", value.clone()));
            assert_eq!(refusal, BadInstanceRoot { fault }, "{value}");
        }
    }

    #[test]
    fn the_runtime_recipe_is_read_by_the_recipe_rules() {
        for (edit, refusal) in [
            (
                set("/runtime_recipe/vcpus", json!(9)),
                RecipeRefusal::VcpusAboveLimit {
                    requested: 9,
                    limit: 8,
                },
            ),
            (
                set("/runtime_recipe", json!("{}")),
                RecipeRefusal::NotAnObject {
                    field: String::new(),
                },
            ),
            (
                ("/runtime_recipe/kernel", None),
                RecipeRefusal::MissingField {
                    field: "kernel".to_owned(),
                },
            ),
            (
                set(
                    "/runtime_recipe/data_path",
                    json!("/var/cell/c1/control.sock"),
                ),
                RecipeRefusal::SocketPathsEqual,
            ),
        ] {
            assert_eq!(refused(&record(&[edit])), Recipe(refusal));
        }
    }

    #[test]
    fn the_boot_deadline_is_a_positive_integer() {
        for value in [
            json!(0),
            json!(-1),
            json!(1.5),
            json!(1.0),
            json!("5"),
            json!(null),
            json!(18_446_744_073_709_551_616.0),
        ] {
            let refusal = refused(&with("/boot_deadline_ms", value.clone()));
            assert_eq!(refusal, BadBootDeadline, "{value}");
        }
        for deadline in [1, u64::MAX] {
            let config = decoded(&with("/boot_deadline_ms", json!(deadline)));
            assert_eq!(config.boot_deadline_ms, deadline);
        }
        let text = String::from_utf8(encoded(&config())).expect("UTF-8");
        let above = text.replace("60000}", "18446744073709551616}");
        assert_eq!(refused(above.as_bytes()), BadBootDeadline);
    }

    #[test]
    fn values_are_checked_in_key_order() {
        let bad_cell = set("/cell", json!("cell-0"));
        let bad_root = set("/instance_root", json!("relative"));
        let bad_recipe = set("/runtime_recipe/vcpus", json!(0));
        let bad_deadline = set("/boot_deadline_ms", json!(0));
        let cases = [
            (
                vec![bad_deadline.clone(), bad_root.clone(), bad_cell],
                BadCell,
            ),
            (
                vec![bad_deadline.clone(), bad_recipe.clone(), bad_root],
                BadInstanceRoot {
                    fault: Some(PathError::NotAbsolute),
                },
            ),
            (
                vec![bad_deadline, bad_recipe],
                Recipe(RecipeRefusal::ZeroVcpus),
            ),
        ];
        for (edits, refusal) in cases {
            assert_eq!(refused(&record(&edits)), refusal);
        }
    }

    #[test]
    fn encode_refuses_an_oversized_record() {
        let largest = config_with_record_bytes(BOUND);
        let line = encoded(&largest);
        assert_eq!(line.len(), BOUND + 1);
        assert_eq!(decoded(&line), largest);
        let over = config_with_record_bytes(BOUND + 1);
        assert_eq!(over.encode(), Err(TooLarge { bytes: BOUND + 1 }));
        let biggest_recipe = SupervisorConfig {
            runtime_recipe: recipe_with_line_bytes(BOUND),
            ..config()
        };
        assert!(matches!(
            biggest_recipe.encode(),
            Err(TooLarge { bytes }) if bytes > BOUND
        ));
    }

    #[test]
    fn encode_refuses_a_record_decode_would_refuse() {
        for cell in ["cell-0", "cell-01", "cell-1/x", "x", ""] {
            let config = SupervisorConfig {
                cell: CellId::from(cell),
                ..config()
            };
            assert_eq!(config.encode(), Err(BadCell), "{cell:?}");
        }
        let config = SupervisorConfig {
            boot_deadline_ms: 0,
            ..config()
        };
        assert_eq!(config.encode(), Err(BadBootDeadline));
    }

    #[test]
    fn refusals_describe_themselves() {
        for (refusal, text) in [
            (
                NotOneLine,
                "the config record is not one line ending in a single newline with no carriage return",
            ),
            (
                TooLarge { bytes: 1_048_577 },
                "the config record is 1048577 bytes before its newline; at most 1048576 are allowed",
            ),
            (NotUtf8, "the config record is not UTF-8"),
            (
                NotJson {
                    detail: "trailing characters at line 1, column 4".to_owned(),
                },
                "the config record is not JSON: trailing characters at line 1, column 4",
            ),
            (
                DuplicateKey {
                    field: "runtime_recipe.vcpus".to_owned(),
                },
                r#"the config record repeats the field "runtime_recipe.vcpus""#,
            ),
            (NotAnObject, "the config record is not a JSON object"),
            (
                UnknownField {
                    field: "ex\ntra".to_owned(),
                },
                r#"the config record carries an unknown field "ex\ntra""#,
            ),
            (
                MissingField { field: "cell" },
                "the config record is missing `cell`",
            ),
            (
                BadCell,
                "`cell` must be cell-<n>, with n from 1 to 9999999999 and no leading zero",
            ),
            (
                BadInstanceRoot { fault: None },
                "`instance_root` must be a string",
            ),
            (
                BadInstanceRoot {
                    fault: Some(PathError::NotAbsolute),
                },
                "`instance_root` is refused: the path does not start with /",
            ),
            (
                Recipe(RecipeRefusal::ZeroVcpus),
                "`runtime_recipe` is refused: `vcpus` must be an integer from 1 to 255",
            ),
            (
                BadBootDeadline,
                "`boot_deadline_ms` must be an integer from 1 to 18446744073709551615",
            ),
        ] {
            assert_eq!(refusal.to_string(), text);
        }
    }
}
