use plasmosome_backend::{
    Diff, EnforcementBackend, FakeBackend, PluginId, UniverseOp, UniverseRemoval,
};
use plasmosome_ledger::{Closure, Effect, Force, InverseVia, Ledger};
use proptest::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    ExactFile,
    CompensatingProxy,
    DelayedUnpublished,
}

fn kind_strategy() -> impl Strategy<Value = Kind> {
    prop_oneof![
        3 => Just(Kind::ExactFile),
        2 => Just(Kind::CompensatingProxy),
        1 => Just(Kind::DelayedUnpublished),
    ]
}

fn generated(kind: Kind, index: usize) -> GeneratedEffect {
    match kind {
        Kind::ExactFile => GeneratedEffect::ExactFile { index },
        Kind::CompensatingProxy => GeneratedEffect::CompensatingProxy { index },
        Kind::DelayedUnpublished => GeneratedEffect::DelayedUnpublished { index },
    }
}

#[derive(Debug, Clone)]
enum GeneratedEffect {
    ExactFile { index: usize },
    CompensatingProxy { index: usize },
    DelayedUnpublished { index: usize },
}

impl GeneratedEffect {
    fn setup(&self) -> Option<UniverseOp> {
        match self {
            GeneratedEffect::ExactFile { index } => Some(UniverseOp::WriteSessionFile {
                path: format!("skills/generated-{index}.md"),
                owner: PluginId::from("generated"),
            }),
            GeneratedEffect::CompensatingProxy { index } => Some(UniverseOp::SetProxyMap {
                host: format!("host-{index}.example.test"),
                route: "staged".to_string(),
                owner: PluginId::from("generated"),
            }),
            GeneratedEffect::DelayedUnpublished { .. } => None,
        }
    }

    fn effect(&self) -> Effect {
        match self {
            GeneratedEffect::ExactFile { index } => Effect::exact(
                format!("exact file {index}"),
                InverseVia::Universe(UniverseRemoval::RemoveSessionFile {
                    path: format!("skills/generated-{index}.md"),
                }),
            ),
            GeneratedEffect::CompensatingProxy { index } => Effect::compensating(
                format!("compensating proxy {index}"),
                UniverseRemoval::RemoveProxyMap {
                    host: format!("host-{index}.example.test"),
                },
            ),
            GeneratedEffect::DelayedUnpublished { index } => {
                Effect::delayed_unpublished("outbox/generated", &format!("payload-{index}"))
            }
        }
    }

    fn replayed_description(&self) -> Option<String> {
        match self {
            GeneratedEffect::ExactFile { index } => Some(format!("exact file {index}")),
            GeneratedEffect::CompensatingProxy { index } => {
                Some(format!("compensating proxy {index}"))
            }
            GeneratedEffect::DelayedUnpublished { .. } => None,
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn random_ledgers_close_safely_replay_in_reverse_and_leave_an_empty_diff(
        kinds in proptest::collection::vec(kind_strategy(), 0..=12),
    ) {
        let mut next_index = [0usize; 3];
        let effects: Vec<GeneratedEffect> = kinds
            .iter()
            .map(|kind| {
                let slot = match kind {
                    Kind::ExactFile => 0,
                    Kind::CompensatingProxy => 1,
                    Kind::DelayedUnpublished => 2,
                };
                let generated = generated(*kind, next_index[slot]);
                next_index[slot] += 1;
                generated
            })
            .collect();

        let mut backend = FakeBackend::new();
        let before = backend.snapshot_os_state();
        let mut ledger = Ledger::new("generated");
        for generated in &effects {
            if let Some(op) = generated.setup() {
                backend.apply(op).expect("generated setup op must apply");
            }
            ledger.push(generated.effect());
        }

        let Closure::ExternalFree(mut sealed) = ledger.close() else {
            panic!("ledgers without External or published Delayed entries must close safely");
        };
        let drain = plasmosome_backend::DrainSpec::graceful(std::time::Duration::from_millis(1));
        let report = sealed.detach(&mut backend, drain).expect("replay over generated ledgers must succeed");

        let mut expected_replay: Vec<String> = effects.iter().filter_map(GeneratedEffect::replayed_description).collect();
        expected_replay.reverse();
        prop_assert_eq!(report.replayed, expected_replay, "replay must be LIFO over the push order");
        prop_assert_eq!(report.delayed_discarded, effects.iter().filter(|e| matches!(e, GeneratedEffect::DelayedUnpublished { .. })).count());
        assert!(report.asserted.is_empty());
        assert!(report.forced.is_none());

        let after = backend.snapshot_os_state();
        assert!(Diff::between(&before, &after).is_empty(), "random ledger replay must leave no residue");
    }

    #[test]
    fn any_single_external_entry_forces_the_closure_and_lands_in_the_report(
        (total, position) in (1usize..=6usize).prop_flat_map(|total| (Just(total), 0..total)),
    ) {
        let mut ledger = Ledger::new("generated");
        for index in 0..total {
            if index == position {
                ledger.push(Effect::external("emission crossed the boundary"));
            } else {
                ledger.push(Effect::delayed_unpublished("outbox/generated", "payload"));
            }
        }
        let Closure::OutstandingExternal(forced_ledger) = ledger.close() else {
            panic!("one External entry anywhere must force the closure");
        };
        let mut backend = FakeBackend::new();
        let mut forced_ledger = forced_ledger;
        let report = forced_ledger
            .detach_forced(
                &mut backend,
                plasmosome_backend::DrainSpec::forcing(),
                Force::operator_asserted("property-test", "asserted by generation"),
            )
            .expect("forced detach must succeed");
        prop_assert_eq!(report.asserted, vec!["emission crossed the boundary".to_string()]);
        assert!(report.forced.is_some());
        assert!(backend.snapshot_os_state().is_empty());
    }
}
