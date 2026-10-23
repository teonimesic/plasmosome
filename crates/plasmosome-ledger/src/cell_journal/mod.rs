use plasmosome_backend::{CellOwner, Handle, MockMode, PluginId, UniverseOp, UniverseRemoval};

use crate::{Compensation, Effect, Inverse, InverseVia, Reversibility};

mod fault;
mod parse;
mod source;

pub use fault::{CellJournalFault, CellJournalFaultKind, JournalRuleError};
pub use parse::{encode_record, parse_record};
pub use source::AttachmentSource;

/// The only format spec 008's per-cell journal has. Every record carries it as `"format": 1`.
pub const CELL_JOURNAL_FORMAT: u64 = 1;

/// One line of a cell journal: a generation and one event of that generation.
///
/// It has no serde of its own. Read a line with `parse_record`, which checks it against its cell,
/// and write one with `encode_record`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellJournalRecord {
    pub generation: u64,
    pub event: JournalEvent,
}

/// A generation's events, in the order spec 008 allows them: prepare, then commit or abort,
/// then finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalEvent {
    Prepare(Prepare),
    Commit,
    Abort,
    Finish,
}

/// The changes one generation makes to a cell, and the operator's Force if it was given.
/// `changes` is nonempty and names each plugin once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepare {
    pub changes: Vec<Change>,
    pub force: Option<ForceAssertion>,
}

/// An operator's recorded Force: who asserted it and why. Neither field is blank. It is an
/// assertion, not authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForceAssertion {
    pub operator: String,
    pub reason: String,
}

/// One plugin's attachment after the prepare. `None` removes the attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub plugin: PluginId,
    pub replacement: Option<Replacement>,
}

/// The complete attachment a change installs: its resolved mode, where it came from, and its
/// effects in grant order. Empty `effects` is a real attachment, not a removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub mock: MockMode,
    pub source: AttachmentSource,
    pub effects: Vec<RecordedEffect>,
}

/// An effect with the operation that created it. Exact universe and compensating effects carry
/// their operation. Backend-handle, external and delayed effects carry `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedEffect {
    pub effect: Effect,
    pub operation: Option<UniverseOp>,
}

impl RecordedEffect {
    /// The exact address this effect holds: the handle of a backend-handle effect, or the class
    /// and ID of a universe inverse or compensation witness. External and delayed effects hold
    /// none.
    pub fn address(&self) -> Option<Handle> {
        if let Reversibility::Exact(Inverse {
            via: InverseVia::Backend(handle),
            ..
        }) = &self.effect.reversibility
        {
            return Some(*handle);
        }
        universe_inverse(&self.effect).map(|removal| Handle {
            class: removal.class(),
            id: removal.id,
        })
    }

    /// Checks that the operation agrees with the effect. An exact universe effect or a
    /// compensation needs an operation whose removal equals its inverse or witness, and whose
    /// owner is `owner`, the journal's cell and the change's plugin. Any other effect must carry
    /// no operation. A refusal is `InvalidEffect`.
    pub fn validate(&self, owner: &CellOwner) -> Result<(), JournalRuleError> {
        let detail = match (universe_inverse(&self.effect), &self.operation) {
            (Some(removal), Some(op)) if op.removal() != *removal => {
                format!(
                    "operation {} does not create inverse {}",
                    op.id(),
                    removal.id
                )
            }
            (Some(_), Some(op)) if op.owner() != owner => {
                format!(
                    "operation {} belongs to {}, not {owner}",
                    op.id(),
                    op.owner()
                )
            }
            (Some(_), Some(_)) | (None, None) => return Ok(()),
            (Some(removal), None) => format!("effect {} has no operation", removal.id),
            (None, Some(op)) => format!("operation {} has no universe inverse to match", op.id()),
        };
        Err(JournalRuleError::new(
            CellJournalFaultKind::InvalidEffect,
            detail,
        ))
    }
}

fn universe_inverse(effect: &Effect) -> Option<&UniverseRemoval> {
    match &effect.reversibility {
        Reversibility::Exact(Inverse {
            via: InverseVia::Universe(removal),
            ..
        })
        | Reversibility::Compensating(Compensation { witness: removal }) => Some(removal),
        _ => None,
    }
}
