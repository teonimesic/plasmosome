const BYTE_ORDER_MARK: &[u8] = &[0xef, 0xbb, 0xbf];

/// Whether `text` nests deeper than `limit` levels anywhere, counted as
/// `MAX_MANIFEST_DEPTH` says. It reads each byte once, without recursion,
/// holds at most `limit` open arrays and inline tables, and stops at the first
/// level past `limit`. Text toml rejects may count deeper than any tree toml
/// builds from it.
pub(crate) fn deeper_than(text: &str, limit: usize) -> bool {
    Count::new(text.as_bytes(), limit).walk().is_err()
}

struct PastLimit;

enum Container {
    Array,
    InlineTable { in_key: bool },
}

struct Open {
    container: Container,
    level: usize,
}

struct Count<'a> {
    bytes: &'a [u8],
    at: usize,
    limit: usize,
    table_level: usize,
    key_dots: usize,
    line_start: bool,
    top_in_key: bool,
    open: Vec<Open>,
}

impl<'a> Count<'a> {
    fn new(bytes: &'a [u8], limit: usize) -> Self {
        let at = if bytes.starts_with(BYTE_ORDER_MARK) {
            BYTE_ORDER_MARK.len()
        } else {
            0
        };
        Count {
            bytes,
            at,
            limit,
            table_level: 1,
            key_dots: 0,
            line_start: true,
            top_in_key: true,
            open: Vec::new(),
        }
    }

    fn walk(mut self) -> Result<(), PastLimit> {
        self.reach(self.table_level)?;
        while let Some(&byte) = self.bytes.get(self.at) {
            match byte {
                b'\n' => self.end_line(),
                b' ' | b'\t' | b'\r' => self.at += 1,
                b'#' => self.skip_comment(),
                b'"' | b'\'' => self.skip_string(),
                b'[' if self.line_start && self.open.is_empty() => self.header()?,
                b'[' | b'{' => self.open_container(byte)?,
                b']' | b'}' => self.close_container(),
                b',' => self.next_entry(),
                b'=' => self.start_value(),
                b'.' => self.dot()?,
                _ => self.step(),
            }
        }
        Ok(())
    }

    fn reach(&self, level: usize) -> Result<(), PastLimit> {
        if level > self.limit {
            Err(PastLimit)
        } else {
            Ok(())
        }
    }

    fn level(&self) -> usize {
        self.open.last().map_or(self.table_level, |open| open.level)
    }

    fn in_key(&self) -> bool {
        match self.open.last() {
            Some(Open {
                container: Container::InlineTable { in_key },
                ..
            }) => *in_key,
            Some(Open {
                container: Container::Array,
                ..
            }) => false,
            None => self.top_in_key,
        }
    }

    fn step(&mut self) {
        self.line_start = false;
        self.at += 1;
    }

    fn end_line(&mut self) {
        self.at += 1;
        if self.open.is_empty() {
            self.line_start = true;
            self.top_in_key = true;
            self.key_dots = 0;
        }
    }

    fn skip_comment(&mut self) {
        self.at = self.bytes[self.at..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(self.bytes.len(), |line| self.at + line);
    }

    fn skip_string(&mut self) {
        self.at = string_end(self.bytes, self.at);
        self.line_start = false;
    }

    fn header(&mut self) -> Result<(), PastLimit> {
        self.at += 1;
        if self.bytes.get(self.at) == Some(&b'[') {
            self.at += 1;
        }
        let mut parts = 1;
        self.reach(1 + 2 * parts)?;
        while let Some(&byte) = self.bytes.get(self.at) {
            match byte {
                b']' | b'\n' => break,
                b'"' | b'\'' => self.at = string_end(self.bytes, self.at),
                b'.' => {
                    parts += 1;
                    self.reach(1 + 2 * parts)?;
                    self.at += 1;
                }
                _ => self.at += 1,
            }
        }
        while self.bytes.get(self.at) == Some(&b']') {
            self.at += 1;
        }
        self.table_level = 1 + 2 * parts;
        self.key_dots = 0;
        self.line_start = false;
        self.top_in_key = true;
        Ok(())
    }

    fn open_container(&mut self, byte: u8) -> Result<(), PastLimit> {
        let level = self.level() + self.key_dots + 1;
        self.reach(level)?;
        let container = if byte == b'[' {
            Container::Array
        } else {
            Container::InlineTable { in_key: true }
        };
        self.open.push(Open { container, level });
        self.key_dots = 0;
        self.step();
        Ok(())
    }

    fn close_container(&mut self) {
        self.open.pop();
        self.key_dots = 0;
        self.step();
    }

    fn next_entry(&mut self) {
        if let Some(Open {
            container: Container::InlineTable { in_key },
            ..
        }) = self.open.last_mut()
        {
            *in_key = true;
        }
        self.key_dots = 0;
        self.step();
    }

    fn start_value(&mut self) {
        match self.open.last_mut() {
            Some(Open {
                container: Container::InlineTable { in_key },
                ..
            }) => *in_key = false,
            Some(Open {
                container: Container::Array,
                ..
            }) => {}
            None => self.top_in_key = false,
        }
        self.step();
    }

    fn dot(&mut self) -> Result<(), PastLimit> {
        if self.in_key() {
            self.key_dots += 1;
            self.reach(self.level() + self.key_dots)?;
        }
        self.step();
        Ok(())
    }
}

fn string_end(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let escapes = quote == b'"';
    if bytes[start..].starts_with(&[quote; 3]) {
        multi_line_string_end(bytes, start + 3, quote, escapes)
    } else {
        single_line_string_end(bytes, start + 1, quote, escapes)
    }
}

fn single_line_string_end(bytes: &[u8], mut at: usize, quote: u8, escapes: bool) -> usize {
    while let Some(&byte) = bytes.get(at) {
        if byte == b'\n' {
            return at;
        }
        if byte == quote {
            return at + 1;
        }
        let escaped_line_end = bytes.get(at + 1) == Some(&b'\n');
        at += if escapes && byte == b'\\' && !escaped_line_end {
            2
        } else {
            1
        };
    }
    bytes.len()
}

fn multi_line_string_end(bytes: &[u8], mut at: usize, quote: u8, escapes: bool) -> usize {
    while let Some(&byte) = bytes.get(at) {
        if escapes && byte == b'\\' {
            at += 2;
        } else if byte == quote {
            let run = bytes[at..]
                .iter()
                .take_while(|&&next| next == quote)
                .count();
            if run >= 3 {
                return at + run;
            }
            at += run;
        } else {
            at += 1;
        }
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use proptest::collection::vec;
    use proptest::prelude::*;
    use proptest::sample::select;
    use proptest::test_runner::TestCaseError;
    use toml_edit::{ImDocument, Item, Value};

    use super::deeper_than;

    const LIMIT: usize = 5;

    const OPENING: &str = "[{.=,[{";

    const CLOSING: &str = "]}]}#";

    fn at_and_past(at: &str, past: &str) {
        assert!(
            !deeper_than(at, LIMIT),
            "{at:?} counts no deeper than {LIMIT}"
        );
        assert!(
            deeper_than(past, LIMIT),
            "{past:?} counts deeper than {LIMIT}"
        );
    }

    #[test]
    fn the_root_table_is_the_first_level() {
        assert!(deeper_than("", 0));
        assert!(!deeper_than("", 1));
        assert!(!deeper_than("a = 1", 1));
        assert!(deeper_than("a.b = 1", 1));
        assert!(deeper_than("a = []", 1));
    }

    #[test]
    fn each_array_inline_table_and_key_dot_counts_a_level() {
        at_and_past("x = [[[[1]]]]", "x = [[[[[1]]]]]");
        at_and_past(
            "x = { a = { b = { c = { d = 1 } } } }",
            "x = { a = { b = { c = { d = { e = 1 } } } } }",
        );
        at_and_past("a.b.c.d.e = 1", "a.b.c.d.e.f = 1");
        at_and_past("x.y = { a.b = [1] }", "x.y = { a.b = [[1]] }");
    }

    #[test]
    fn each_header_part_counts_two_levels_and_each_header_starts_again() {
        at_and_past("[a.b]\nk = 1", "[a.b]\nk.j = 1");
        at_and_past("[[a]]\n[[a.b]]\nk = 1", "[[a]]\n[[a.b]]\nk.j = 1");
        at_and_past("[a]\nk = [[1]]", "[a]\nk = [[[1]]]");
        at_and_past(
            "[a.b]\nk = 1\n[c]\nd = [[1]]",
            "[a.b]\nk = 1\n[c]\nd = [[[1]]]",
        );
        at_and_past("  [a.b] # x.y.z\nk = 1", "  [a.b] # x.y.z\nk.j = 1");
    }

    #[test]
    fn structure_inside_any_string_value_counts_nothing() {
        for quote in ["\"", "'", "\"\"\"", "'''"] {
            at_and_past(
                &format!("x = [{quote}{OPENING}{quote}, [[[1]]]]"),
                &format!("x = [{quote}{CLOSING}{quote}, [[[[1]]]]]"),
            );
        }
    }

    #[test]
    fn structure_inside_a_quoted_key_counts_nothing() {
        for quote in ["\"", "'"] {
            at_and_past(
                &format!("{quote}{OPENING}{quote} = [[[[1]]]]"),
                &format!("{quote}{CLOSING}{quote} = [[[[[1]]]]]"),
            );
            at_and_past(
                &format!("x = {{ {quote}a.b.c{quote} = [[[1]]] }}"),
                &format!("x = {{ {quote}a.b.c{quote}.d = [[[1]]] }}"),
            );
        }
    }

    #[test]
    fn structure_inside_a_quoted_header_part_counts_nothing() {
        for quote in ["\"", "'"] {
            at_and_past(
                &format!("[{quote}a.b.c{quote}.d]\nk = 1"),
                &format!("[{quote}a]b{quote}.c]\nk.j = 1"),
            );
        }
    }

    #[test]
    fn an_escaped_quote_or_backslash_does_not_end_a_basic_string() {
        at_and_past(r#"x = ["a\"[[[[", [[[1]]]]"#, r#"x = ["a\"", [[[[1]]]]]"#);
        at_and_past(r#"x = ["a\\", [[[1]]]]"#, r#"x = ["a\\", [[[[1]]]]]"#);
        at_and_past(
            r#"x = ["""a\"""[[[[""", [[[1]]]]"#,
            r#"x = ["""a\"""]]]]#""", [[[[1]]]]]"#,
        );
    }

    #[test]
    fn a_backslash_ends_nothing_in_a_literal_string() {
        at_and_past(r"x = ['a\', [[[1]]]]", r"x = ['a\', [[[[1]]]]]");
        at_and_past(r"x = ['''a\''', [[[1]]]]", r"x = ['''a\''', [[[[1]]]]]");
    }

    #[test]
    fn a_multi_line_string_keeps_the_quotes_before_its_closing_three() {
        for quote in ["\"", "'"] {
            let three = quote.repeat(3);
            for tail in [1, 2] {
                let content = format!("a{}", quote.repeat(tail));
                at_and_past(
                    &format!("x = [{three}{content}{three}, [[[1]]]]"),
                    &format!("x = [{three}{content}{three}, [[[[1]]]]]"),
                );
            }
        }
    }

    #[test]
    fn a_single_line_string_ends_at_its_line() {
        at_and_past("a = \"x\n[b]\nk = [1]", "a = \"x\n[b.c]\nk = [1]");
        at_and_past("a = \"x\\\n[b]\nk = [1]", "a = \"x\\\n[b.c]\nk = [1]");
        at_and_past("a = 'x\n[b]\nk = [1]", "a = 'x\n[b.c]\nk = [1]");
    }

    #[test]
    fn a_comment_inside_a_multi_line_array_counts_nothing() {
        at_and_past(
            "x = [ # [[[[\n  [[[1]]],\n]",
            "x = [ # ]}]}\n  [[[[1]]]],\n]",
        );
    }

    #[test]
    fn a_dot_in_a_value_counts_nothing() {
        at_and_past("x = [[[[0.5]]]]", "x = [[[[[0.5]]]]]");
        at_and_past(
            "x = { a = { b = { c = [1979-05-27T07:32:00.999Z, 07:32:00.5] } } }",
            "x = { a = { b = { c = { d = [1979-05-27T07:32:00.999Z] } } } }",
        );
        at_and_past(
            "x = { a = { b = { c = { d = 1.5 } } } }",
            "x = { a = { b = { c = { d = { e = 1.5 } } } } }",
        );
    }

    #[test]
    fn a_key_after_a_comma_starts_its_dots_again() {
        at_and_past(
            "x = { a.b.c = 1, d = [[[1]]] }",
            "x = { a.b.c = 1, d = [[[[1]]]] }",
        );
    }

    #[test]
    fn a_line_starting_with_a_bracket_inside_an_array_is_an_array_not_a_header() {
        at_and_past("x = [\n[[[1]]],\n]", "x = [\n[[[[1]]]],\n]");
    }

    #[test]
    fn crlf_line_endings_and_spaces_around_dots_count_the_same() {
        at_and_past(
            "a = 1\r\n[b.c]\r\nd = 1\r\n",
            "a = 1\r\n[b.c]\r\nd.e = 1\r\n",
        );
        at_and_past("x = [\r\n[[[1]]],\r\n]\r\n", "x = [\r\n[[[[1]]]],\r\n]\r\n");
        at_and_past("a . b .\tc . d . e = 1", "a . b .\tc . d . e . f = 1");
        at_and_past("[ a . b ]\nk = 1", "[ a . b ]\nk . j = 1");
        at_and_past("x = { a . b . c = [1] }", "x = { a . b . c . d = [1] }");
    }

    #[test]
    fn a_leading_byte_order_mark_is_not_text() {
        at_and_past("\u{feff}[a.b]\nk = 1", "\u{feff}[a.b]\nk.j = 1");
    }

    #[derive(Clone, Debug)]
    enum Part {
        Bare,
        Basic(&'static str),
        Literal(&'static str),
    }

    #[derive(Clone, Debug)]
    struct Key {
        parts: Vec<Part>,
        spaced: bool,
    }

    type Layout = (&'static str, &'static str, &'static str, bool);

    #[derive(Clone, Debug)]
    enum Node {
        Scalar(String),
        Array(Vec<Node>, Layout),
        Table(Vec<(Key, Node)>),
    }

    #[derive(Clone, Debug)]
    struct Header {
        array: bool,
        name: Key,
        subtable: Option<Part>,
    }

    #[derive(Clone, Debug)]
    struct Section {
        header: Option<Header>,
        entries: Vec<(Key, Node, &'static str)>,
    }

    #[derive(Clone, Debug)]
    struct Document {
        byte_order_mark: bool,
        sections: Vec<Section>,
    }

    const BASIC_KEY_TRAPS: &[&str] = &[
        ".", "[", "]", "{", "}", "#", "=", ",", " ", "'", "é", "\\\"", "\\\\",
    ];

    const LITERAL_KEY_TRAPS: &[&str] =
        &[".", "[", "]", "{", "}", "#", "=", ",", " ", "é", "\"", "\\"];

    const PLAIN_SCALARS: &[&str] = &[
        "1",
        "1.5",
        "-0.5e3",
        "+inf",
        "nan",
        "true",
        "0x1F",
        "1979-05-27T07:32:00.999Z",
        "1979-05-27 07:32:00.25",
        "07:32:00.5",
        "1979-05-27",
        "\"\"",
        "''",
    ];

    const BASIC_PIECES: &[&str] = &[
        ".", "[", "]", "{", "}", "#", "=", ",", " ", "'", "x", "é", "\\\"", "\\\\",
    ];

    const LITERAL_PIECES: &[&str] = &[
        ".", "[", "]", "{", "}", "#", "=", ",", " ", "x", "é", "\"", "\\",
    ];

    const MULTI_LINE_BASIC_PIECES: &[&str] = &[
        ".", "[", "]", "{", "}", "#", "\n", "x", "\"", "\\\\", "\\\"\"\"",
    ];

    const MULTI_LINE_LITERAL_PIECES: &[&str] =
        &[".", "[", "]", "{", "}", "#", "\n", "x", "'", "\\", "\""];

    const ARRAY_LAYOUTS: &[Layout] = &[
        ("[", ", ", "]", false),
        ("[\n", ",\n", "\n]", false),
        ("[\n  ", ",\n  ", "\n]", true),
        ("[ # [ {\n", ", # ] } [ ' \"\n", " # ]]\n]", false),
    ];

    const LINE_ENDS: &[&str] = &["\n", "\r\n", " # x.y [ { ' \"\n", "  # ]] }\n"];

    fn part() -> impl Strategy<Value = Part> {
        prop_oneof![
            2 => Just(Part::Bare),
            1 => select(BASIC_KEY_TRAPS).prop_map(Part::Basic),
            1 => select(LITERAL_KEY_TRAPS).prop_map(Part::Literal),
        ]
    }

    fn key() -> impl Strategy<Value = Key> {
        (vec(part(), 1..5), any::<bool>()).prop_map(|(parts, spaced)| Key { parts, spaced })
    }

    fn quoted(
        quote: &'static str,
        pieces: &'static [&'static str],
        tails: &'static [&'static str],
    ) -> impl Strategy<Value = String> {
        (vec(select(pieces), 0..4), select(tails))
            .prop_map(move |(body, tail)| format!("{quote}{}{tail}{quote}", body.concat()))
    }

    fn scalar() -> impl Strategy<Value = String> {
        prop_oneof![
            select(PLAIN_SCALARS).prop_map(str::to_string),
            quoted("\"", BASIC_PIECES, &[""]),
            quoted("'", LITERAL_PIECES, &[""]),
            quoted("\"\"\"", MULTI_LINE_BASIC_PIECES, &["", "\"", "\"\""]),
            quoted("'''", MULTI_LINE_LITERAL_PIECES, &["", "'", "''"]),
        ]
    }

    fn node() -> impl Strategy<Value = Node> {
        scalar()
            .prop_map(Node::Scalar)
            .prop_recursive(4, 24, 4, |inner| {
                prop_oneof![
                    (vec(inner.clone(), 0..4), select(ARRAY_LAYOUTS))
                        .prop_map(|(items, layout)| Node::Array(items, layout)),
                    vec((key(), inner), 0..4).prop_map(Node::Table),
                ]
            })
    }

    fn entry() -> impl Strategy<Value = (Key, Node, &'static str)> {
        (key(), node(), select(LINE_ENDS))
    }

    fn header() -> impl Strategy<Value = Header> {
        (any::<bool>(), key(), proptest::option::of(part())).prop_map(|(array, name, subtable)| {
            Header {
                array,
                name,
                subtable,
            }
        })
    }

    fn document() -> impl Strategy<Value = Document> {
        let without_headers = vec(entry(), 1..5).prop_map(|entries| {
            vec![Section {
                header: None,
                entries,
            }]
        });
        let with_headers = vec(
            (proptest::option::of(header()), vec(entry(), 0..4))
                .prop_map(|(header, entries)| Section { header, entries }),
            1..4,
        );
        (
            proptest::bool::weighted(0.25),
            prop_oneof![without_headers, with_headers],
        )
            .prop_map(|(byte_order_mark, sections)| Document {
                byte_order_mark,
                sections,
            })
    }

    #[derive(Default)]
    struct Render {
        written: usize,
    }

    impl Render {
        fn part(&mut self, part: &Part) -> String {
            self.written += 1;
            let unique = self.written;
            match part {
                Part::Bare => format!("k{unique}"),
                Part::Basic(trap) => format!("\"k{unique}{trap}\""),
                Part::Literal(trap) => format!("'k{unique}{trap}'"),
            }
        }

        fn key(&mut self, key: &Key) -> String {
            let parts: Vec<String> = key.parts.iter().map(|part| self.part(part)).collect();
            parts.join(if key.spaced { " . " } else { "." })
        }

        fn node(&mut self, node: &Node) -> String {
            match node {
                Node::Scalar(scalar) => scalar.clone(),
                Node::Array(items, (open, separator, close, trailing_comma)) => {
                    let items: Vec<String> = items.iter().map(|item| self.node(item)).collect();
                    let trailing = if *trailing_comma && !items.is_empty() {
                        ","
                    } else {
                        ""
                    };
                    format!("{open}{}{trailing}{close}", items.join(separator))
                }
                Node::Table(entries) if entries.is_empty() => "{}".to_string(),
                Node::Table(entries) => {
                    let entries: Vec<String> = entries
                        .iter()
                        .map(|(key, value)| format!("{} = {}", self.key(key), self.node(value)))
                        .collect();
                    format!("{{ {} }}", entries.join(", "))
                }
            }
        }
    }

    impl Document {
        fn has_header(&self) -> bool {
            self.sections.iter().any(|section| section.header.is_some())
        }

        fn render(&self) -> String {
            let mut render = Render::default();
            let mut text = String::new();
            if self.byte_order_mark {
                text.push('\u{feff}');
            }
            for section in &self.sections {
                if let Some(header) = &section.header {
                    let name = render.key(&header.name);
                    if header.array {
                        text.push_str(&format!("[[{name}]]\n"));
                    } else {
                        text.push_str(&format!("[{name}]\n"));
                    }
                    if let Some(subtable) = &header.subtable {
                        let subtable = render.part(subtable);
                        text.push_str(&format!("[{name}.{subtable}]\n"));
                    }
                }
                for (key, value, end) in &section.entries {
                    let key = render.key(key);
                    let value = render.node(value);
                    text.push_str(&format!("{key} = {value}{end}"));
                }
            }
            text
        }
    }

    enum Held<'a> {
        Item(&'a Item),
        Value(&'a Value),
    }

    fn tree_depth(text: &str) -> Option<usize> {
        let document = ImDocument::parse(text).ok()?;
        let mut deepest = 1;
        let mut pending: Vec<(Held<'_>, usize)> = document
            .as_table()
            .iter()
            .map(|(_, item)| (Held::Item(item), 1))
            .collect();
        while let Some((held, level)) = pending.pop() {
            match held {
                Held::Item(Item::Table(table)) => {
                    deepest = deepest.max(level + 1);
                    pending.extend(table.iter().map(|(_, item)| (Held::Item(item), level + 1)));
                }
                Held::Item(Item::ArrayOfTables(array)) => {
                    deepest = deepest.max(level + 1);
                    for table in array.iter() {
                        deepest = deepest.max(level + 2);
                        pending.extend(table.iter().map(|(_, item)| (Held::Item(item), level + 2)));
                    }
                }
                Held::Item(Item::Value(value)) => pending.push((Held::Value(value), level)),
                Held::Item(Item::None) => {}
                Held::Value(Value::Array(array)) => {
                    deepest = deepest.max(level + 1);
                    pending.extend(array.iter().map(|value| (Held::Value(value), level + 1)));
                }
                Held::Value(Value::InlineTable(table)) => {
                    deepest = deepest.max(level + 1);
                    pending.extend(
                        table
                            .iter()
                            .map(|(_, value)| (Held::Value(value), level + 1)),
                    );
                }
                Held::Value(_) => {}
            }
        }
        Some(deepest)
    }

    #[test]
    fn the_tree_walk_counts_the_documented_levels() {
        assert_eq!(tree_depth("a = 1"), Some(1));
        assert_eq!(tree_depth("a.b = [{ c = 1 }]"), Some(4));
        assert_eq!(tree_depth("[[a]]\n[a.b]\nc = []"), Some(5));
        assert_eq!(tree_depth("a = "), None);
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 2048,
            max_global_rejects: 4096,
            ..ProptestConfig::default()
        })]

        #[test]
        fn the_count_is_never_below_the_tree_and_equals_it_without_headers(
            document in document()
        ) {
            let text = document.render();
            let Some(depth) = tree_depth(&text) else {
                return Err(TestCaseError::reject("toml_edit refuses the document"));
            };
            prop_assert!(
                deeper_than(&text, depth - 1),
                "{text:?} is {depth} deep, and its count is below that"
            );
            if !document.has_header() {
                prop_assert!(
                    !deeper_than(&text, depth),
                    "{text:?} has no header and is {depth} deep, and its count is above that"
                );
            }
        }
    }
}
