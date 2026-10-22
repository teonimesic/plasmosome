use std::collections::HashSet;
use std::fmt;

const MAX_INDENT: usize = 32;

const LINEAR_KEYS: usize = 8;

/// The longest reply line spec 025 lets the client read, its newline left out: 16 MiB.
/// [`scan`] refuses longer input, and a reader reads through a limit of this plus one byte.
pub const MAX_REPLY_BYTES: usize = 16 * 1024 * 1024;

/// One JSON value as a reply carried it.
///
/// Dropping, comparing and printing a value never recurse, so no depth of nesting overflows the
/// stack. Because `Json` implements `Drop`, a caller takes children out with `std::mem::take`
/// rather than by moving them out of a pattern. Two values are equal when they hold the same
/// members in the same order and the same number text, so `1` and `1.0` differ. `Debug` prints
/// the compact form.
#[derive(Default)]
pub enum Json {
    #[default]
    Null,
    Bool(bool),
    /// The number's text as it arrived, never decoded.
    Number(Number),
    /// The decoded string.
    String(String),
    Array(Vec<Json>),
    /// The members in the order they arrived. `scan` never gives a key twice, and a caller
    /// building one must not either.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Returns the value of `key` when this is an object that has it, and `None` otherwise.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(members) => member(members, key),
            _ => None,
        }
    }
}

/// A number's text, which RFC 8259's number grammar accepts in full. A value written from it is
/// therefore valid JSON and holds no terminal control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Number(String);

impl Number {
    /// Takes `text` when RFC 8259's number grammar accepts all of it, and hands it back otherwise.
    pub fn new(text: String) -> Result<Number, String> {
        let mut scanner = Scanner {
            text: &text,
            bytes: text.as_bytes(),
            at: 0,
        };
        let whole = scanner.number().is_ok() && scanner.at == text.len();
        if whole { Ok(Number(text)) } else { Err(text) }
    }

    /// Returns the number's text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub(crate) fn member<'a>(members: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    members
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

pub(crate) fn repeats_a_key(value: &Json) -> bool {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Json::Array(items) => pending.extend(items),
            Json::Object(members) => {
                if repeats(members) {
                    return true;
                }
                pending.extend(members.iter().map(|(_, value)| value));
            }
            _ => {}
        }
    }
    false
}

fn repeats(members: &[(String, Json)]) -> bool {
    if members.len() <= LINEAR_KEYS {
        return members
            .iter()
            .enumerate()
            .any(|(index, (key, _))| member(&members[..index], key).is_some());
    }
    let mut seen = HashSet::new();
    members.iter().any(|(key, _)| !seen.insert(key.as_str()))
}

impl Drop for Json {
    fn drop(&mut self) {
        let mut pending = Vec::new();
        take_children(self, &mut pending);
        while let Some(mut value) = pending.pop() {
            take_children(&mut value, &mut pending);
        }
    }
}

fn take_children(value: &mut Json, into: &mut Vec<Json>) {
    match value {
        Json::Array(items) => into.append(items),
        Json::Object(members) => into.extend(members.drain(..).map(|(_, value)| value)),
        _ => {}
    }
}

impl PartialEq for Json {
    fn eq(&self, other: &Json) -> bool {
        let mut pending = vec![(self, other)];
        while let Some(pair) = pending.pop() {
            match pair {
                (Json::Null, Json::Null) => {}
                (Json::Bool(left), Json::Bool(right)) if left == right => {}
                (Json::Number(left), Json::Number(right)) if left == right => {}
                (Json::String(left), Json::String(right)) if left == right => {}
                (Json::Array(left), Json::Array(right)) if left.len() == right.len() => {
                    pending.extend(left.iter().zip(right));
                }
                (Json::Object(left), Json::Object(right)) if left.len() == right.len() => {
                    for ((left_key, left_value), (right_key, right_value)) in left.iter().zip(right)
                    {
                        if left_key != right_key {
                            return false;
                        }
                        pending.push((left_value, right_value));
                    }
                }
                _ => return false,
            }
        }
        true
    }
}

impl Eq for Json {}

impl fmt::Debug for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&write_compact(self))
    }
}

/// Why [`scan`] refused its input, and the byte offset where it noticed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanError {
    pub offset: usize,
    pub reason: Reason,
}

/// What broke RFC 8259, or this client's rule against a repeated key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The input is not UTF-8. The offset is the first byte that is not.
    NotUtf8,
    /// The input ended inside a value, or held no value.
    End,
    /// The grammar allowed only what is named here.
    Expected(&'static str),
    /// A number breaks the grammar, a leading zero included.
    Number,
    /// A string holds a raw character below U+0020.
    Control,
    /// A string holds a backslash that starts no valid escape.
    Escape,
    /// A string escapes half of a surrogate pair.
    LoneSurrogate,
    /// Bytes other than whitespace follow the value.
    Trailing,
    /// The input is longer than [`MAX_REPLY_BYTES`]. The offset is the first byte past the bound.
    TooLarge,
    /// An object repeats a key, compared after decoding. `path` is the RFC 6901 pointer to the
    /// repeat, as decoded. `Display` shows each terminal control in it as `<U+XXXX>`.
    DuplicateKey { path: String },
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::NotUtf8 => f.write_str("the text is not UTF-8"),
            Reason::End => f.write_str("the text ends inside a value"),
            Reason::Expected(what) => write!(f, "expected {what}"),
            Reason::Number => f.write_str("a number breaks the JSON grammar"),
            Reason::Control => f.write_str("a string holds a raw control character"),
            Reason::Escape => f.write_str("a string holds an invalid escape"),
            Reason::LoneSurrogate => f.write_str("a string escapes a lone surrogate"),
            Reason::Trailing => f.write_str("more text follows the value"),
            Reason::TooLarge => write!(f, "the text is longer than {MAX_REPLY_BYTES} bytes"),
            Reason::DuplicateKey { path } => {
                let mut shown = String::new();
                show(&mut shown, path);
                write!(f, "the key {shown} repeats")
            }
        }
    }
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.reason, self.offset)
    }
}

impl std::error::Error for ScanError {}

/// Reads one JSON value, with optional whitespace around it, from `bytes`.
///
/// Numbers keep their text, strings are decoded, and an object that repeats a key is refused.
/// Input longer than [`MAX_REPLY_BYTES`] is refused before any of it is read. Nesting depth has
/// no limit: the scan keeps its own stack, so nesting costs memory and never the call stack.
pub fn scan(bytes: &[u8]) -> Result<Json, ScanError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(ScanError {
            offset: MAX_REPLY_BYTES,
            reason: Reason::TooLarge,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|error| ScanError {
        offset: error.valid_up_to(),
        reason: Reason::NotUtf8,
    })?;
    Scanner {
        text,
        bytes: text.as_bytes(),
        at: 0,
    }
    .document()
}

enum Frame {
    Array(Vec<Json>),
    Object(Box<ObjectFrame>),
}

struct ObjectFrame {
    members: Vec<(String, Json)>,
    key: String,
    seen: Option<HashSet<String>>,
}

impl ObjectFrame {
    fn admit(&mut self, key: String) -> Result<(), String> {
        let repeated = match &mut self.seen {
            Some(seen) => !seen.insert(key.clone()),
            None => {
                let repeated = member(&self.members, &key).is_some();
                if !repeated && self.members.len() >= LINEAR_KEYS {
                    let mut seen: HashSet<String> =
                        self.members.iter().map(|(name, _)| name.clone()).collect();
                    seen.insert(key.clone());
                    self.seen = Some(seen);
                }
                repeated
            }
        };
        if repeated {
            return Err(key);
        }
        self.key = key;
        Ok(())
    }
}

struct Scanner<'a> {
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
}

impl Scanner<'_> {
    fn document(mut self) -> Result<Json, ScanError> {
        let mut frames: Vec<Frame> = Vec::new();
        loop {
            self.skip_whitespace();
            let mut value = match self.peek() {
                Some(b'[') => {
                    self.at += 1;
                    self.skip_whitespace();
                    if !self.eat(b']') {
                        frames.push(Frame::Array(Vec::new()));
                        continue;
                    }
                    Json::Array(Vec::new())
                }
                Some(b'{') => {
                    self.at += 1;
                    self.skip_whitespace();
                    if !self.eat(b'}') {
                        let (_, key) = self.key()?;
                        frames.push(Frame::Object(Box::new(ObjectFrame {
                            members: Vec::new(),
                            key,
                            seen: None,
                        })));
                        continue;
                    }
                    Json::Object(Vec::new())
                }
                _ => self.scalar()?,
            };
            loop {
                self.skip_whitespace();
                match frames.last_mut() {
                    None if self.at == self.bytes.len() => return Ok(value),
                    None => return Err(self.error(Reason::Trailing)),
                    Some(Frame::Array(items)) => {
                        items.push(value);
                        if self.eat(b',') {
                            break;
                        }
                        if !self.eat(b']') {
                            return Err(self.unexpected("',' or ']'"));
                        }
                        let mut items = std::mem::take(items);
                        items.shrink_to_fit();
                        frames.pop();
                        value = Json::Array(items);
                    }
                    Some(Frame::Object(object)) => {
                        let key = std::mem::take(&mut object.key);
                        object.members.push((key, value));
                        if self.eat(b',') {
                            let (offset, key) = self.key()?;
                            if let Err(key) = object.admit(key) {
                                let path = pointer(&frames, &key);
                                return Err(ScanError {
                                    offset,
                                    reason: Reason::DuplicateKey { path },
                                });
                            }
                            break;
                        }
                        if !self.eat(b'}') {
                            return Err(self.unexpected("',' or '}'"));
                        }
                        let mut members = std::mem::take(&mut object.members);
                        members.shrink_to_fit();
                        frames.pop();
                        value = Json::Object(members);
                    }
                }
            }
        }
    }

    fn key(&mut self) -> Result<(usize, String), ScanError> {
        self.skip_whitespace();
        let offset = self.at;
        if self.peek() != Some(b'"') {
            return Err(self.unexpected("a string key"));
        }
        let key = self.string()?;
        self.skip_whitespace();
        if !self.eat(b':') {
            return Err(self.unexpected("':'"));
        }
        Ok((offset, key))
    }

    fn scalar(&mut self) -> Result<Json, ScanError> {
        match self.peek() {
            Some(b'"') => self.string().map(Json::String),
            Some(b'-' | b'0'..=b'9') => self.number().map(Json::Number),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            _ => Err(self.unexpected("a value")),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, ScanError> {
        if !self.bytes[self.at..].starts_with(word.as_bytes()) {
            return Err(self.error(Reason::Expected("a value")));
        }
        self.at += word.len();
        Ok(value)
    }

    fn number(&mut self) -> Result<Number, ScanError> {
        let start = self.at;
        self.eat(b'-');
        match self.peek() {
            Some(b'0') => {
                self.at += 1;
                if self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    return Err(self.error(Reason::Number));
                }
            }
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(self.error(Reason::Number)),
        }
        if self.eat(b'.') {
            self.required_digits()?;
        }
        if self.eat(b'e') || self.eat(b'E') {
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            self.required_digits()?;
        }
        Ok(Number(self.text[start..self.at].to_string()))
    }

    fn required_digits(&mut self) -> Result<(), ScanError> {
        if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            return Err(self.error(Reason::Number));
        }
        self.digits();
        Ok(())
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.at += 1;
        }
    }

    fn string(&mut self) -> Result<String, ScanError> {
        self.at += 1;
        let mut text = String::new();
        loop {
            let run = self.at;
            while self
                .peek()
                .is_some_and(|byte| byte != b'"' && byte != b'\\' && byte >= 0x20)
            {
                self.at += 1;
            }
            text.push_str(&self.text[run..self.at]);
            match self.peek() {
                None => return Err(self.error(Reason::End)),
                Some(b'"') => {
                    self.at += 1;
                    return Ok(text);
                }
                Some(b'\\') => text.push(self.escape()?),
                Some(_) => return Err(self.error(Reason::Control)),
            }
        }
    }

    fn escape(&mut self) -> Result<char, ScanError> {
        let start = self.at;
        self.at += 1;
        let Some(byte) = self.peek() else {
            return Err(self.error(Reason::End));
        };
        self.at += 1;
        let decoded = match byte {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => char::from(0x08),
            b'f' => char::from(0x0c),
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => return self.unicode(start),
            _ => {
                return Err(ScanError {
                    offset: start,
                    reason: Reason::Escape,
                });
            }
        };
        Ok(decoded)
    }

    fn unicode(&mut self, start: usize) -> Result<char, ScanError> {
        let lone = ScanError {
            offset: start,
            reason: Reason::LoneSurrogate,
        };
        let first = self.hex(start)?;
        let code_point = match first {
            0xd800..=0xdbff => {
                let second_start = self.at;
                if !(self.eat(b'\\') && self.eat(b'u')) {
                    return Err(lone);
                }
                let second = self.hex(second_start)?;
                if !(0xdc00..=0xdfff).contains(&second) {
                    return Err(lone);
                }
                0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00)
            }
            other => other,
        };
        char::from_u32(code_point).ok_or(lone)
    }

    fn hex(&mut self, escape: usize) -> Result<u32, ScanError> {
        let mut value = 0;
        for _ in 0..4 {
            let Some(byte) = self.peek() else {
                return Err(self.error(Reason::End));
            };
            let digit = char::from(byte).to_digit(16).ok_or(ScanError {
                offset: escape,
                reason: Reason::Escape,
            })?;
            value = value * 16 + digit;
            self.at += 1;
        }
        Ok(value)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn eat(&mut self, byte: u8) -> bool {
        let found = self.peek() == Some(byte);
        if found {
            self.at += 1;
        }
        found
    }

    fn error(&self, reason: Reason) -> ScanError {
        ScanError {
            offset: self.at,
            reason,
        }
    }

    fn unexpected(&self, expected: &'static str) -> ScanError {
        if self.at == self.bytes.len() {
            self.error(Reason::End)
        } else {
            self.error(Reason::Expected(expected))
        }
    }
}

fn pointer(frames: &[Frame], repeated: &str) -> String {
    let mut path = String::new();
    for frame in &frames[..frames.len().saturating_sub(1)] {
        path.push('/');
        match frame {
            Frame::Array(items) => path.push_str(&items.len().to_string()),
            Frame::Object(object) => push_pointer_segment(&mut path, &object.key),
        }
    }
    path.push('/');
    push_pointer_segment(&mut path, repeated);
    path
}

fn push_pointer_segment(path: &mut String, key: &str) {
    for character in key.chars() {
        match character {
            '~' => path.push_str("~0"),
            '/' => path.push_str("~1"),
            other => path.push(other),
        }
    }
}

pub(crate) trait Sink {
    fn push_str(&mut self, text: &str);

    fn push(&mut self, character: char) {
        self.push_str(character.encode_utf8(&mut [0; 4]));
    }
}

impl Sink for String {
    fn push_str(&mut self, text: &str) {
        String::push_str(self, text);
    }

    fn push(&mut self, character: char) {
        String::push(self, character);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Style {
    Compact,
    Human,
}

/// Writes `value` as one compact line with no newline.
///
/// No whitespace is written outside strings, numbers keep their text, and every terminal
/// control of spec 025 section 3, in a key or a value, is written as JSON's six-character escape.
pub fn write_compact(value: &Json) -> String {
    let mut out = String::new();
    write_value(&mut out, value, Style::Compact, 0);
    out
}

/// Writes `value` indented for a person to read, one member or item per line, with each
/// terminal control shown as `<U+XXXX>`. Nothing may parse it.
pub fn write_human(value: &Json) -> String {
    let mut out = String::new();
    write_value(&mut out, value, Style::Human, 0);
    out
}

#[derive(Clone, Copy)]
enum Rest<'a> {
    Items(&'a [Json]),
    Members(&'a [(String, Json)]),
}

struct Level<'a> {
    rest: Rest<'a>,
    next: usize,
}

pub(crate) fn write_value(out: &mut impl Sink, value: &Json, style: Style, depth: usize) {
    let mut levels: Vec<Level<'_>> = Vec::new();
    let mut next = Some(value);
    loop {
        if let Some(value) = next.take() {
            match value {
                Json::Null => out.push_str("null"),
                Json::Bool(true) => out.push_str("true"),
                Json::Bool(false) => out.push_str("false"),
                Json::Number(number) => write_number(out, number),
                Json::String(text) => write_string(out, text, style),
                Json::Array(items) if items.is_empty() => out.push_str("[]"),
                Json::Object(members) if members.is_empty() => out.push_str("{}"),
                Json::Array(items) => {
                    out.push('[');
                    levels.push(Level {
                        rest: Rest::Items(items),
                        next: 0,
                    });
                }
                Json::Object(members) => {
                    out.push('{');
                    levels.push(Level {
                        rest: Rest::Members(members),
                        next: 0,
                    });
                }
            }
        }
        let inner = depth + levels.len();
        let Some(level) = levels.last_mut() else {
            return;
        };
        let item = match level.rest {
            Rest::Items(items) => items.get(level.next).map(|value| (None, value)),
            Rest::Members(members) => members
                .get(level.next)
                .map(|(key, value)| (Some(key.as_str()), value)),
        };
        match item {
            Some((key, value)) => {
                separator(out, level.next == 0, style, inner);
                level.next += 1;
                if let Some(key) = key {
                    write_key(out, key, style);
                }
                next = Some(value);
            }
            None => {
                let close = match level.rest {
                    Rest::Items(_) => ']',
                    Rest::Members(_) => '}',
                };
                levels.pop();
                close_container(out, close, style, depth + levels.len());
            }
        }
    }
}

pub(crate) fn separator(out: &mut impl Sink, first: bool, style: Style, depth: usize) {
    if !first {
        out.push(',');
    }
    if style == Style::Human {
        out.push('\n');
        for _ in 0..depth.min(MAX_INDENT) {
            out.push_str("  ");
        }
    }
}

pub(crate) fn close_container(out: &mut impl Sink, close: char, style: Style, depth: usize) {
    if style == Style::Human {
        separator(out, true, style, depth);
    }
    out.push(close);
}

pub(crate) fn write_key(out: &mut impl Sink, key: &str, style: Style) {
    write_string(out, key, style);
    out.push(':');
    if style == Style::Human {
        out.push(' ');
    }
}

pub(crate) fn write_string(out: &mut impl Sink, text: &str, style: Style) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' | '\\' => {
                out.push('\\');
                out.push(character);
            }
            control if is_terminal_control(control) => write_control(out, control, style),
            other => out.push(other),
        }
    }
    out.push('"');
}

pub(crate) fn write_number(out: &mut impl Sink, number: &Number) {
    for character in number.as_str().chars() {
        out.push(character);
    }
}

pub(crate) fn show(out: &mut impl Sink, text: &str) {
    for character in text.chars() {
        if is_terminal_control(character) {
            write_control(out, character, Style::Human);
        } else {
            out.push(character);
        }
    }
}

fn write_control(out: &mut impl Sink, control: char, style: Style) {
    let code_point = u32::from(control);
    match style {
        Style::Compact => {
            out.push('\\');
            out.push('u');
            out.push_str(&format!("{code_point:04x}"));
        }
        Style::Human => out.push_str(&format!("<U+{code_point:04X}>")),
    }
}

pub(crate) fn is_terminal_control(character: char) -> bool {
    matches!(
        u32::from(character),
        0x00..=0x1f | 0x7f..=0x9f | 0x061c | 0x200e..=0x200f | 0x202a..=0x202e | 0x2066..=0x2069
    )
}

#[cfg(test)]
mod tests {
    use super::{is_terminal_control, show};

    #[test]
    fn show_writes_each_terminal_control_by_its_code_point_and_every_other_character_raw() {
        let every: String = (0..=0x10ffff).filter_map(char::from_u32).collect();
        let mut shown = String::new();
        show(&mut shown, &every);
        let mut rest = shown.as_str();
        for character in every.chars() {
            let point = u32::from(character);
            let listed = is_terminal_control(character);
            let after = if listed {
                rest.strip_prefix(format!("<U+{point:04X}>").as_str())
            } else {
                rest.strip_prefix(character)
            };
            rest = after.unwrap_or_else(|| {
                let found: String = rest.chars().take(8).collect();
                if listed {
                    panic!("U+{point:04X} is a terminal control, so show writes its code point, not {found:?}")
                }
                panic!("U+{point:04X} is not a terminal control, so show writes it raw, not as {found:?}")
            });
        }
        assert!(
            rest.is_empty(),
            "show writes {rest:?} after the last character"
        );
    }
}
