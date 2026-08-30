use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::state::{CellId, GenomeName, PlasmidRecord};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredState {
    pub generation: u64,
    pub cells: BTreeMap<CellId, DesiredCell>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredCell {
    pub genome: Option<GenomeName>,
    pub plasmids: Vec<PlasmidRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ObservedState {
    pub generation: u64,
    pub cells: BTreeSet<CellId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcilePlan {
    Idle,
    Drift {
        missing: Vec<CellId>,
        unexpected: Vec<CellId>,
    },
}

pub struct Reconciler {
    desired: DesiredState,
    observed: ObservedState,
}

impl Reconciler {
    pub fn new(desired: DesiredState) -> Reconciler {
        Reconciler {
            desired,
            observed: ObservedState::default(),
        }
    }

    pub fn desired(&self) -> &DesiredState {
        &self.desired
    }

    pub fn observed(&self) -> &ObservedState {
        &self.observed
    }

    pub fn set_desired(&mut self, desired: DesiredState) {
        self.desired = desired;
    }

    pub fn observe(&mut self, observed: ObservedState) {
        self.observed = observed;
    }

    pub fn reconcile(&self) -> ReconcilePlan {
        let desired_cells: BTreeSet<CellId> = self.desired.cells.keys().cloned().collect();
        let missing: Vec<CellId> = desired_cells
            .difference(&self.observed.cells)
            .cloned()
            .collect();
        let unexpected: Vec<CellId> = self
            .observed
            .cells
            .difference(&desired_cells)
            .cloned()
            .collect();
        if missing.is_empty() && unexpected.is_empty() {
            ReconcilePlan::Idle
        } else {
            ReconcilePlan::Drift {
                missing,
                unexpected,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desired(cells: &[&str]) -> DesiredState {
        DesiredState {
            generation: 1,
            cells: cells
                .iter()
                .map(|id| {
                    (
                        CellId::from(*id),
                        DesiredCell {
                            genome: Some(GenomeName::from("researcher")),
                            plasmids: vec![PlasmidRecord {
                                plasmid: "github-pr".to_string(),
                                mock: crate::state::MockMode::Simulate,
                            }],
                        },
                    )
                })
                .collect(),
        }
    }

    fn observed(cells: &[&str]) -> ObservedState {
        ObservedState {
            generation: 1,
            cells: cells.iter().map(|id| CellId::from(*id)).collect(),
        }
    }

    #[test]
    fn a_fresh_reconciler_reports_every_desired_cell_missing() {
        let reconciler = Reconciler::new(desired(&["cell-1", "cell-2"]));
        match reconciler.reconcile() {
            ReconcilePlan::Drift {
                missing,
                unexpected,
            } => {
                assert_eq!(
                    missing,
                    vec![CellId::from("cell-1"), CellId::from("cell-2")]
                );
                assert!(unexpected.is_empty());
            }
            other => panic!("a fresh reconciler must see drift, got {other:?}"),
        }
    }

    #[test]
    fn observing_the_desired_cells_converges_to_idle() {
        let mut reconciler = Reconciler::new(desired(&["cell-1"]));
        reconciler.observe(observed(&["cell-1"]));
        assert_eq!(reconciler.reconcile(), ReconcilePlan::Idle);
    }

    #[test]
    fn an_observed_cell_the_genome_does_not_want_is_named() {
        let mut reconciler = Reconciler::new(desired(&["cell-1"]));
        reconciler.observe(observed(&["cell-1", "stray"]));
        match reconciler.reconcile() {
            ReconcilePlan::Drift {
                missing,
                unexpected,
            } => {
                assert!(missing.is_empty());
                assert_eq!(unexpected, vec![CellId::from("stray")]);
            }
            other => panic!("a stray cell is drift, got {other:?}"),
        }
    }

    #[test]
    fn reconcile_is_idempotent_over_the_same_observation() {
        let mut reconciler = Reconciler::new(desired(&["cell-1", "cell-2"]));
        reconciler.observe(observed(&["cell-1"]));
        let first = reconciler.reconcile();
        let second = reconciler.reconcile();
        assert_eq!(first, second, "reconciling twice must not change the plan");
        reconciler.observe(observed(&["cell-1"]));
        assert_eq!(
            reconciler.reconcile(),
            first,
            "a replayed observation converges instead of re-firing"
        );
    }

    #[test]
    fn desired_and_observed_state_round_trip_through_serde_with_generation() {
        let desired = desired(&["cell-1"]);
        let json = serde_json::to_string(&desired).unwrap();
        assert!(json.contains("\"generation\":1"), "{json}");
        let back: DesiredState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, desired);
        let observed = observed(&["cell-1"]);
        let json = serde_json::to_string(&observed).unwrap();
        let back: ObservedState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, observed);
    }

    #[test]
    fn a_newer_desired_generation_replaces_the_old_one() {
        let mut reconciler = Reconciler::new(desired(&["cell-1"]));
        let mut updated = desired(&["cell-1", "cell-2"]);
        updated.generation = 2;
        reconciler.set_desired(updated);
        assert_eq!(reconciler.desired().generation, 2);
        reconciler.observe(observed(&["cell-1", "cell-2"]));
        assert_eq!(reconciler.reconcile(), ReconcilePlan::Idle);
    }
}
