use std::fmt;
use std::path::PathBuf;

/// Why a cell journal was refused: exactly spec 008's closed list of kinds. This codec does not
/// produce `InvalidClosure` or `InvalidLifetime` yet; they come with the closure and lifetime
/// fields of a replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CellJournalFaultKind {
    Io,
    Unparseable,
    UnsupportedFormat,
    GenerationZero,
    GenerationOrder,
    EventOrder,
    InvalidEffect,
    InvalidSource,
    InvalidClosure,
    InvalidLifetime,
}

/// A rule one record, or one record in its history, breaks. `detail` is for people; no code may
/// branch on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalRuleError {
    pub kind: CellJournalFaultKind,
    pub detail: String,
}

impl JournalRuleError {
    pub(crate) fn new(kind: CellJournalFaultKind, detail: impl Into<String>) -> JournalRuleError {
        JournalRuleError {
            kind,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for JournalRuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.detail)
    }
}

impl std::error::Error for JournalRuleError {}

/// Why a cell journal file gave no history. `line` is the physical line of the fault, counted
/// from 1, and is `None` when the file could not be opened. `lines_parsed` counts the records
/// accepted before it. `detail` is for people; no code may branch on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellJournalFault {
    pub path: PathBuf,
    pub line: Option<u64>,
    pub lines_parsed: u64,
    pub kind: CellJournalFaultKind,
    pub detail: String,
}

impl fmt::Display for CellJournalFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cell journal {}", self.path.display())?;
        if let Some(line) = self.line {
            write!(f, " line {line}")?;
        }
        write!(
            f,
            ": {:?} after {} accepted records: {}",
            self.kind, self.lines_parsed, self.detail
        )
    }
}

impl std::error::Error for CellJournalFault {}
