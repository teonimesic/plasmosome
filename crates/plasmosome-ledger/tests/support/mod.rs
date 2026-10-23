use plasmosome_backend::{CellId, CellOwner, GrantId, MockMode, PluginId, UdsRecipe, UniverseOp};
use plasmosome_ledger::cell_journal::{
    AttachmentSource, CellJournalRecord, Change, JournalEvent, Prepare, RecordedEffect, Replacement,
};
use plasmosome_ledger::{Effect, InverseVia};

pub fn record(generation: u64, event: JournalEvent) -> CellJournalRecord {
    CellJournalRecord { generation, event }
}

pub fn prepare(generation: u64, changes: Vec<Change>) -> CellJournalRecord {
    record(
        generation,
        JournalEvent::Prepare(Prepare {
            changes,
            force: None,
        }),
    )
}

pub fn attach(plugin: &str, effects: Vec<RecordedEffect>) -> Change {
    Change {
        plugin: PluginId::from(plugin),
        replacement: Some(Replacement {
            mock: MockMode::Simulate,
            source: AttachmentSource::Local,
            effects,
        }),
    }
}

pub fn exact_uds(cell: &str, plugin: &str) -> RecordedEffect {
    let op = UniverseOp::BindUds {
        id: GrantId::new(),
        path: "/run/test.sock".into(),
        recipe: UdsRecipe {
            upstream: "/run/upstream.sock".into(),
            guest_path: "/run/guest.sock".into(),
        },
        owner: CellOwner {
            cell: CellId::from(cell),
            plugin: PluginId::from(plugin),
        },
    };
    RecordedEffect {
        effect: Effect::exact("socket", InverseVia::Universe(op.removal())),
        operation: Some(op),
    }
}
