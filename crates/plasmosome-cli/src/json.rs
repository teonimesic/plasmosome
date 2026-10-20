use std::fmt;

/// One JSON value as a reply carried it.
#[derive(Debug, PartialEq, Eq, Default)]
pub enum Json {
    #[default]
    Null,
    Bool(bool),
    /// The number's text as it arrived, never decoded. A caller building one passes text that
    /// RFC 8259's number grammar accepts.
    Number(String),
    /// The decoded string.
    String(String),
    Array(Vec<Json>),
    /// The members in the order they arrived. A key appears at most once.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Returns the value of `key` when this is an object that has it, and `None` otherwise.
    pub fn get(&self, key: &str) -> Option<&Json> {
        let _ = key;
        todo!()
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
    /// An object repeats a key, compared after decoding. `path` is the RFC 6901 pointer to the
    /// repeat.
    DuplicateKey { path: String },
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!()
    }
}

impl std::error::Error for ScanError {}

/// Reads one JSON value, with optional whitespace around it, from `bytes`.
pub fn scan(bytes: &[u8]) -> Result<Json, ScanError> {
    let _ = bytes;
    todo!()
}

/// Writes `value` as one compact line with no newline.
pub fn write_compact(value: &Json) -> String {
    let _ = value;
    todo!()
}

/// Writes `value` indented for a person to read. Nothing may parse it.
pub fn write_human(value: &Json) -> String {
    let _ = value;
    todo!()
}
