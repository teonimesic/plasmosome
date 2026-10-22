use std::fs;

use plasmosome_cli::json::{
    Json, MAX_REPLY_BYTES, Number, Reason, ScanError, scan, write_compact, write_human,
};

const BACKSLASH: char = '\\';

const SPEC_025: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/specs/025-plasmosome-command-line.md"
);

const TERMINAL_CONTROLS_PARAGRAPH: &str = "**Terminal controls are escaped in both forms.**";

fn escaped(hex: &str) -> String {
    format!("{BACKSLASH}u{hex}")
}

fn character(code_point: u32) -> char {
    char::from_u32(code_point).expect("a test names a valid code point")
}

fn terminal_controls() -> Vec<char> {
    [
        (0x00, 0x1f),
        (0x7f, 0x7f),
        (0x80, 0x9f),
        (0x061c, 0x061c),
        (0x200e, 0x200f),
        (0x202a, 0x202e),
        (0x2066, 0x2069),
    ]
    .into_iter()
    .flat_map(|(first, last)| first..=last)
    .map(character)
    .collect()
}

fn spec_025_terminal_controls() -> Vec<u32> {
    let spec = fs::read_to_string(SPEC_025).expect("spec 025 is readable beside the crate");
    let paragraph = spec
        .lines()
        .skip_while(|line| !line.starts_with(TERMINAL_CONTROLS_PARAGRAPH))
        .take_while(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let list = paragraph
        .split_once("The terminal controls are ")
        .and_then(|(_, rest)| rest.split_once('.'))
        .map(|(list, _)| list)
        .unwrap_or_else(|| panic!("spec 025's list of terminal controls was not found"));
    let mut points = Vec::new();
    let mut range_open = false;
    for piece in list.split("U+").skip(1) {
        let rest =
            piece.trim_start_matches(|c: char| c.is_ascii_digit() || ('A'..='F').contains(&c));
        let digits = &piece[..piece.len() - rest.len()];
        if !(4..=6).contains(&digits.len()) {
            range_open = false;
            continue;
        }
        let point = u32::from_str_radix(digits, 16).expect("upper-case hex digits");
        if range_open {
            let first = points.pop().expect("a range starts at a code point");
            points.extend(first..=point);
        } else {
            points.push(point);
        }
        range_open = rest.trim() == "to";
    }
    points.sort_unstable();
    points
}

fn number(text: &str) -> Json {
    Json::Number(Number::new(text.to_string()).expect("the test names valid number text"))
}

fn string(text: &str) -> Json {
    Json::String(text.to_string())
}

fn object(members: Vec<(&str, Json)>) -> Json {
    Json::Object(
        members
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn refused(input: &[u8]) -> ScanError {
    match scan(input) {
        Ok(value) => panic!(
            "{:?} scanned as {value:?}; it breaks the grammar and must be refused",
            String::from_utf8_lossy(input)
        ),
        Err(error) => error,
    }
}

#[test]
fn every_terminal_control_is_escaped() {
    let controls = terminal_controls();
    assert_eq!(
        controls.len(),
        77,
        "spec 025 section 3 names 77 code points"
    );
    for &control in &controls {
        let key = format!("k{control}k");
        let text = format!("v{control}v");
        let written = write_compact(&Json::Object(vec![(
            key.clone(),
            Json::String(text.clone()),
        )]));
        let accepted = [
            format!("{:04x}", u32::from(control)),
            format!("{:04X}", u32::from(control)),
        ]
        .map(|hex| {
            let six = escaped(&hex);
            format!("{{\"k{six}k\":\"v{six}v\"}}")
        });
        assert!(
            accepted.contains(&written),
            "U+{:04X} is written as JSON's six-character escape in a key and in a value, not as {written:?}",
            u32::from(control)
        );
        assert!(
            !written.chars().any(|c| controls.contains(&c)),
            "U+{:04X} reached the output raw",
            u32::from(control)
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&written).expect("the compact form is valid JSON");
        let mut expected = serde_json::Map::new();
        expected.insert(key, serde_json::Value::String(text));
        assert_eq!(
            parsed,
            serde_json::Value::Object(expected),
            "U+{:04X} parses back to the same key and value",
            u32::from(control)
        );
    }
}

#[test]
fn other_characters_pass_through_raw() {
    for code_point in [
        0x20, 0x2f, 0x7e, 0xa0, 0xff, 0x061b, 0x061d, 0x200d, 0x2010, 0x2028, 0x2029, 0x202f,
        0x2065, 0x206a, 0xfeff, 0x1f600,
    ] {
        let raw = character(code_point);
        assert_eq!(
            write_compact(&Json::String(raw.to_string())),
            format!("\"{raw}\""),
            "U+{code_point:04X} is not a terminal control and is written as UTF-8"
        );
    }
}

#[test]
fn every_character_is_escaped_exactly_when_spec_025_lists_it() {
    let listed = spec_025_terminal_controls();
    let tested: Vec<u32> = terminal_controls().into_iter().map(u32::from).collect();
    assert_eq!(
        listed, tested,
        "the test's list of terminal controls is spec 025's list"
    );
    for character in (0..=0x10ffff).filter_map(char::from_u32) {
        if character == '"' || character == BACKSLASH {
            continue;
        }
        let point = u32::from(character);
        let value = Json::String(character.to_string());
        let compact = write_compact(&value);
        let human = write_human(&value);
        if listed.binary_search(&point).is_ok() {
            let escape = compact
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .and_then(|rest| rest.strip_prefix(BACKSLASH))
                .and_then(|rest| rest.strip_prefix('u'))
                .filter(|hex| hex.len() == 4 && hex.chars().all(|c| c.is_ascii_hexdigit()))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok());
            assert_eq!(
                escape,
                Some(point),
                "U+{point:04X} is listed, so the compact form escapes it, not as {compact:?}"
            );
            assert_eq!(
                human,
                format!(r#""<U+{point:04X}>""#),
                "U+{point:04X} is listed, so the human form shows it by its code point"
            );
        } else {
            let raw = format!(r#""{character}""#);
            assert_eq!(
                compact, raw,
                "U+{point:04X} is not listed, so the compact form writes it as UTF-8"
            );
            assert_eq!(
                human, raw,
                "U+{point:04X} is not listed, so the human form writes it as UTF-8"
            );
        }
    }
}

#[test]
fn quote_and_backslash_take_their_short_escapes() {
    let written = write_compact(&Json::String(format!("\"{BACKSLASH}")));
    assert_eq!(written, format!("\"{BACKSLASH}\"{BACKSLASH}{BACKSLASH}\""));
}

#[test]
fn numbers_keep_their_text() {
    let text = "[18446744073709551616,-0,1.0e+2,1E400,-12.5E-3,0]";
    let value = scan(text.as_bytes()).expect("every number here is valid JSON");
    assert_eq!(
        value,
        Json::Array(vec![
            number("18446744073709551616"),
            number("-0"),
            number("1.0e+2"),
            number("1E400"),
            number("-12.5E-3"),
            number("0"),
        ])
    );
    assert_eq!(write_compact(&value), text);
}

#[test]
fn a_number_is_built_only_from_number_text() {
    for text in [
        "0",
        "-0",
        "18446744073709551616",
        "1.0e+2",
        "1E400",
        "-12.5E-3",
    ] {
        let built = Number::new(text.to_string()).expect("RFC 8259's number grammar accepts it");
        assert_eq!(built.as_str(), text);
    }
    let with_control = format!("1{}", character(0x1b));
    for text in [
        "",
        "-",
        "01",
        "1.",
        ".5",
        "+1",
        "1e",
        "1e+",
        "1 ",
        " 1",
        "1,2",
        "NaN",
        "Infinity",
        "0x10",
        "\"1\"",
        &with_control,
    ] {
        assert_eq!(
            Number::new(text.to_string()),
            Err(text.to_string()),
            "{text:?} is not a JSON number and is handed back"
        );
    }
}

#[test]
fn duplicate_keys_are_refused() {
    let cases = [
        (r#"{"a":1,"a":2}"#.to_string(), 7, "/a"),
        (r#"[0,{"b":true,"b":false}]"#.to_string(), 13, "/1/b"),
        (
            format!(r#"{{"x":{{"y":[{{"a":1,"{}":2}}]}}}}"#, escaped("0061")),
            18,
            "/x/y/0/a",
        ),
        (format!(r#"{{"{}":1,"a":2}}"#, escaped("0061")), 12, "/a"),
        (r#"{"a/b":{"~":1,"~":2}}"#.to_string(), 14, "/a~1b/~0"),
    ];
    for (input, offset, path) in cases {
        assert_eq!(
            refused(input.as_bytes()),
            ScanError {
                offset,
                reason: Reason::DuplicateKey {
                    path: path.to_string()
                },
            },
            "{input}"
        );
    }
}

#[test]
fn a_wide_object_refuses_a_repeated_key() {
    for width in [8, 9, 20] {
        let members: Vec<String> = (0..width).map(|n| format!("\"k{n}\":{n}")).collect();
        let wide = format!("{{{}}}", members.join(","));
        assert!(scan(wide.as_bytes()).is_ok(), "{wide}");
        for repeat in [0, width / 2, width - 1] {
            let tail = format!("\"k{repeat}\":0}}");
            let text = format!("{{{},{tail}", members.join(","));
            assert_eq!(
                refused(text.as_bytes()),
                ScanError {
                    offset: text.len() - tail.len(),
                    reason: Reason::DuplicateKey {
                        path: format!("/k{repeat}")
                    },
                },
                "{text}"
            );
        }
    }
}

#[test]
fn a_key_may_repeat_in_another_object() {
    let value = scan(br#"{"a":{"a":1},"A":[{"a":2},{"a":3}]}"#).expect("no object repeats a key");
    assert_eq!(
        value,
        object(vec![
            ("a", object(vec![("a", number("1"))])),
            (
                "A",
                Json::Array(vec![
                    object(vec![("a", number("2"))]),
                    object(vec![("a", number("3"))]),
                ]),
            ),
        ])
    );
}

#[test]
fn the_grammar_is_strict() {
    let lone_high = format!("\"{}\"", escaped("D800"));
    let lone_low = format!("\"{}\"", escaped("DC00"));
    let high_then_letter = format!("\"{}{}\"", escaped("D800"), escaped("0041"));
    let unknown_escape = format!("\"{BACKSLASH}x\"");
    let bad_hex = format!("\"{}\"", escaped("12G4"));
    let bom = format!("{}{{}}", character(0xfeff));
    let cases: Vec<(Vec<u8>, usize, Reason)> = vec![
        (b"[1,]".to_vec(), 3, Reason::Expected("a value")),
        (b"{\"a\":1,}".to_vec(), 7, Reason::Expected("a string key")),
        (b"01".to_vec(), 1, Reason::Number),
        (b"[-01]".to_vec(), 3, Reason::Number),
        (b"[-]".to_vec(), 2, Reason::Number),
        (b"1.".to_vec(), 2, Reason::Number),
        (b"1.e5".to_vec(), 2, Reason::Number),
        (b"1e+".to_vec(), 3, Reason::Number),
        (b"\"a\x01b\"".to_vec(), 2, Reason::Control),
        (b"\"a\nb\"".to_vec(), 2, Reason::Control),
        (b"\"\x1f\"".to_vec(), 1, Reason::Control),
        (lone_high.into_bytes(), 1, Reason::LoneSurrogate),
        (lone_low.into_bytes(), 1, Reason::LoneSurrogate),
        (high_then_letter.into_bytes(), 1, Reason::LoneSurrogate),
        (unknown_escape.into_bytes(), 1, Reason::Escape),
        (bad_hex.into_bytes(), 1, Reason::Escape),
        (b"[\"\xff\"]".to_vec(), 2, Reason::NotUtf8),
        (b"\"\xc0\x80\"".to_vec(), 1, Reason::NotUtf8),
        (b"\"\xed\xa0\x80\"".to_vec(), 1, Reason::NotUtf8),
        (b"{} x".to_vec(), 3, Reason::Trailing),
        (b"{}{}".to_vec(), 2, Reason::Trailing),
        (b"1 2".to_vec(), 2, Reason::Trailing),
        (b"\"abc".to_vec(), 4, Reason::End),
        (b"".to_vec(), 0, Reason::End),
        (b"   ".to_vec(), 3, Reason::End),
        (b"[".to_vec(), 1, Reason::End),
        (b"[1,".to_vec(), 3, Reason::End),
        (b"{".to_vec(), 1, Reason::End),
        (b"{\"a\"".to_vec(), 4, Reason::End),
        (format!("\"{BACKSLASH}").into_bytes(), 2, Reason::End),
        (format!("\"{BACKSLASH}u12").into_bytes(), 5, Reason::End),
        (
            format!("\"{}{BACKSLASH}uZZZZ\"", escaped("D800")).into_bytes(),
            7,
            Reason::Escape,
        ),
        (b"{\"a\x01\":1}".to_vec(), 3, Reason::Control),
        (b"{\"a".to_vec(), 3, Reason::End),
        (b"{\"a\" 1}".to_vec(), 5, Reason::Expected("':'")),
        (b"{1:2}".to_vec(), 1, Reason::Expected("a string key")),
        (b"[1 2]".to_vec(), 3, Reason::Expected("',' or ']'")),
        (b"[1}".to_vec(), 2, Reason::Expected("',' or ']'")),
        (
            b"{\"a\":1 \"b\":2}".to_vec(),
            7,
            Reason::Expected("',' or '}'"),
        ),
        (b"{\"a\":1]".to_vec(), 6, Reason::Expected("',' or '}'")),
        (b"{\"a\":}".to_vec(), 5, Reason::Expected("a value")),
        (b"nul".to_vec(), 0, Reason::Expected("a value")),
        (b"[true,False]".to_vec(), 6, Reason::Expected("a value")),
        (b"+1".to_vec(), 0, Reason::Expected("a value")),
        (b".5".to_vec(), 0, Reason::Expected("a value")),
        (b"[1,\x0c2]".to_vec(), 3, Reason::Expected("a value")),
        (bom.into_bytes(), 0, Reason::Expected("a value")),
    ];
    for (input, offset, reason) in cases {
        assert_eq!(
            refused(&input),
            ScanError { offset, reason },
            "{:?}",
            String::from_utf8_lossy(&input)
        );
    }
}

#[test]
fn a_reply_at_the_bound_scans_and_one_byte_more_is_refused() {
    assert_eq!(MAX_REPLY_BYTES, 16_777_216, "spec 025's bound on one reply");
    let mut text = vec![b' '; MAX_REPLY_BYTES];
    text[0] = b'0';
    assert_eq!(scan(&text), Ok(number("0")));
    let too_large = ScanError {
        offset: MAX_REPLY_BYTES,
        reason: Reason::TooLarge,
    };
    text.push(b' ');
    assert_eq!(scan(&text), Err(too_large.clone()));
    assert_eq!(
        scan(&vec![0xff; MAX_REPLY_BYTES + 1]),
        Err(too_large),
        "the bound is checked before anything else is read"
    );
}

#[test]
fn valid_documents_scan() {
    let value = scan(b" \t\r\n{ \"a\" : [ 1 , true , false , null , \"s\" , { } , [ ] ] } \n")
        .expect("whitespace may surround every token");
    assert_eq!(
        value,
        object(vec![(
            "a",
            Json::Array(vec![
                number("1"),
                Json::Bool(true),
                Json::Bool(false),
                Json::Null,
                string("s"),
                Json::Object(vec![]),
                Json::Array(vec![]),
            ]),
        )])
    );
}

#[test]
fn escapes_decode() {
    let input = format!(
        "\"{b}\"{b}{b}{b}/{b}b{b}f{b}n{b}r{b}t{}{}{}{}\"",
        escaped("00e9"),
        escaped("00E9"),
        escaped("D83D"),
        escaped("DE00"),
        b = BACKSLASH
    );
    let expected: String = [
        '"',
        BACKSLASH,
        '/',
        character(0x08),
        character(0x0c),
        character(0x0a),
        character(0x0d),
        character(0x09),
        character(0xe9),
        character(0xe9),
        character(0x1f600),
    ]
    .into_iter()
    .collect();
    assert_eq!(scan(input.as_bytes()), Ok(Json::String(expected)));
}

#[test]
fn raw_characters_json_allows_are_accepted() {
    let raw: String = [0x7f, 0x80, 0x9b, 0x202e, 0x2066, 0x2028]
        .into_iter()
        .map(character)
        .collect();
    let input = format!("\"{raw}\"");
    assert_eq!(scan(input.as_bytes()), Ok(Json::String(raw)));
}

#[test]
fn get_reads_an_object_member() {
    let value = object(vec![("a", number("1")), ("b", Json::Null)]);
    assert_eq!(value.get("a"), Some(&number("1")));
    assert_eq!(value.get("b"), Some(&Json::Null));
    assert_eq!(value.get("c"), None);
    assert_eq!(Json::Array(vec![string("a")]).get("a"), None);
}

#[test]
fn a_scan_error_says_where() {
    let error = refused(b"[1 2]");
    assert_eq!(error.to_string(), "expected ',' or ']' at byte 3");
    let error = refused(br#"{"a":1,"a":2}"#);
    assert_eq!(error.to_string(), "the key /a repeats at byte 7");
}

#[test]
fn a_scan_error_shows_each_control_in_a_repeated_key() {
    let key = format!("a{}{}", escaped("001b"), escaped("202e"));
    let error = refused(format!(r#"{{"{key}":1,"{key}":2}}"#).as_bytes());
    assert_eq!(
        error.reason,
        Reason::DuplicateKey {
            path: format!("/a{}{}", character(0x1b), character(0x202e)),
        }
    );
    assert_eq!(
        error.reason.to_string(),
        "the key /a<U+001B><U+202E> repeats"
    );
    assert_eq!(
        error.to_string(),
        "the key /a<U+001B><U+202E> repeats at byte 19"
    );
}

#[test]
fn a_repeated_key_shows_every_control_by_its_code_point() {
    for control in terminal_controls() {
        let point = u32::from(control);
        let key = format!("k{}", escaped(&format!("{point:04x}")));
        let error = refused(format!(r#"{{"{key}":1,"{key}":2}}"#).as_bytes());
        assert_eq!(
            error.to_string(),
            format!("the key /k<U+{point:04X}> repeats at byte 13")
        );
    }
}

#[test]
fn a_repeated_key_shows_every_control_in_its_parent_key() {
    for control in terminal_controls() {
        let point = u32::from(control);
        let parent = format!("p{}", escaped(&format!("{point:04x}")));
        let error = refused(format!(r#"{{"{parent}":[{{"a":1,"a":2}}]}}"#).as_bytes());
        assert_eq!(
            error.to_string(),
            format!("the key /p<U+{point:04X}>/0/a repeats at byte 19")
        );
    }
}

#[test]
fn every_reason_reads_as_prose() {
    let cases = [
        (Reason::NotUtf8, "the text is not UTF-8"),
        (Reason::End, "the text ends inside a value"),
        (Reason::Expected("a value"), "expected a value"),
        (Reason::Number, "a number breaks the JSON grammar"),
        (Reason::Control, "a string holds a raw control character"),
        (Reason::Escape, "a string holds an invalid escape"),
        (Reason::LoneSurrogate, "a string escapes a lone surrogate"),
        (Reason::Trailing, "more text follows the value"),
        (Reason::TooLarge, "the text is longer than 16777216 bytes"),
        (
            Reason::DuplicateKey {
                path: "/a".to_string(),
            },
            "the key /a repeats",
        ),
    ];
    for (reason, text) in cases {
        assert_eq!(
            ScanError { offset: 9, reason }.to_string(),
            format!("{text} at byte 9")
        );
    }
}

#[test]
fn values_compare_by_structure_and_text() {
    assert_ne!(
        object(vec![("a", number("1"))]),
        object(vec![("b", number("1"))])
    );
    assert_ne!(
        object(vec![("a", number("1"))]),
        object(vec![("a", number("2"))])
    );
    assert_ne!(
        object(vec![("a", number("1"))]),
        object(vec![("a", number("1")), ("b", number("2"))])
    );
    assert_ne!(
        object(vec![("a", number("1")), ("b", number("2"))]),
        object(vec![("a", number("1"))])
    );
    assert_ne!(number("1"), number("1.0"));
    assert_ne!(number("1"), string("1"));
    assert_ne!(
        Json::Array(vec![Json::Null]),
        Json::Array(vec![Json::Null, Json::Null])
    );
    assert_eq!(
        object(vec![("a", Json::Array(vec![Json::Bool(true)]))]),
        object(vec![("a", Json::Array(vec![Json::Bool(true)]))])
    );
}

#[test]
fn debug_prints_the_compact_form() {
    let value = object(vec![("a", Json::Array(vec![number("1"), string("x")]))]);
    assert_eq!(format!("{value:?}"), r#"{"a":[1,"x"]}"#);
}

#[test]
fn compact_output_is_one_line() {
    let value = object(vec![
        (
            "a b",
            Json::Array(vec![number("1"), object(vec![("c", string("d e\n"))])]),
        ),
        ("f", Json::Null),
        ("g", Json::Bool(true)),
        ("h", Json::Bool(false)),
        ("i", Json::Array(vec![])),
        ("j", Json::Object(vec![])),
    ]);
    assert_eq!(
        write_compact(&value),
        format!(
            r#"{{"a b":[1,{{"c":"d e{}"}}],"f":null,"g":true,"h":false,"i":[],"j":{{}}}}"#,
            escaped("000a")
        )
    );
}

#[test]
fn the_human_form_shows_each_control() {
    let value = Json::Object(vec![
        (
            "esc".to_string(),
            Json::String(format!("a{}b", character(0x1b))),
        ),
        (
            "list".to_string(),
            Json::Array(vec![
                Json::String(character(0x9b).to_string()),
                number("1"),
                Json::Null,
            ]),
        ),
        (format!("k{}", character(0x7f)), Json::Object(vec![])),
        (
            "bidi".to_string(),
            Json::String(character(0x202e).to_string()),
        ),
        ("empty".to_string(), Json::Array(vec![])),
        ("t".to_string(), Json::Bool(true)),
        ("quote".to_string(), Json::String(format!("\"{BACKSLASH}"))),
    ]);
    let expected = [
        "{".to_string(),
        "  \"esc\": \"a<U+001B>b\",".to_string(),
        "  \"list\": [".to_string(),
        "    \"<U+009B>\",".to_string(),
        "    1,".to_string(),
        "    null".to_string(),
        "  ],".to_string(),
        "  \"k<U+007F>\": {},".to_string(),
        "  \"bidi\": \"<U+202E>\",".to_string(),
        "  \"empty\": [],".to_string(),
        "  \"t\": true,".to_string(),
        format!("  \"quote\": \"{BACKSLASH}\"{BACKSLASH}{BACKSLASH}\""),
        "}".to_string(),
    ]
    .join("\n");
    assert_eq!(write_human(&value), expected);
}

#[test]
fn the_human_form_shows_every_control_by_its_code_point() {
    let controls = terminal_controls();
    for &control in &controls {
        let shown = write_human(&Json::String(control.to_string()));
        assert_eq!(shown, format!("\"<U+{:04X}>\"", u32::from(control)));
    }
}
