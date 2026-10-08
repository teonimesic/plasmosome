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

