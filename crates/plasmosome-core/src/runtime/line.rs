use super::strict_json::{self, StrictJsonError};
use serde_json::Value;

/// The most bytes a config record may hold before its newline, as spec 023
/// §4 bounds it; a recipe line has the same bound. The largest record is this
/// many bytes and one newline, so a reader that stops after
/// `MAX_RECORD_BYTES + 2` bytes has read more than any record can be, and
/// [`SupervisorConfig::decode`](super::supervisor_config::SupervisorConfig::decode)
/// refuses that as `TooLarge`.
pub const MAX_RECORD_BYTES: usize = 1_048_576;

pub(crate) enum LineFault {
    NotOneLine,
    TooLarge { bytes: usize },
    NotUtf8,
    NotJson { detail: String },
    DuplicateKey { field: String },
}

pub(crate) fn read_line(record: &[u8]) -> Result<Value, LineFault> {
    let Some(body) = record.strip_suffix(b"\n") else {
        return Err(LineFault::NotOneLine);
    };
    if body.iter().any(|byte| matches!(byte, b'\n' | b'\r')) {
        return Err(LineFault::NotOneLine);
    }
    if body.len() > MAX_RECORD_BYTES {
        return Err(LineFault::TooLarge { bytes: body.len() });
    }
    std::str::from_utf8(body).map_err(|_| LineFault::NotUtf8)?;
    strict_json::parse_value(body).map_err(|error| match error {
        StrictJsonError::DuplicateKey { field, .. } => LineFault::DuplicateKey { field },
        StrictJsonError::NotJson {
            line,
            column,
            reason,
        } => LineFault::NotJson {
            detail: format!("{reason} at line {line}, column {column}"),
        },
    })
}
