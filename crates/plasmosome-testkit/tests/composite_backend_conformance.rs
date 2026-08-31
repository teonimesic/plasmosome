use plasmosome_backend::{CompositeBackend, EnforcementBackend, FakeBackend};
use plasmosome_testkit::conformance;

fn composite_over_fake_leaves() -> CompositeBackend {
    fn leaf() -> Box<dyn EnforcementBackend> {
        Box::new(FakeBackend::new())
    }
    CompositeBackend::new(leaf(), leaf(), leaf())
}

#[test]
fn composite_backend_grants_are_replayable() {
    conformance::grant_is_replayable(composite_over_fake_leaves);
}

#[test]
fn composite_backend_rejects_an_unknown_handle() {
    conformance::revoke_unknown_handle_is_error(composite_over_fake_leaves);
}

#[test]
fn composite_backend_removes_the_object_a_drained_revoke_owned() {
    conformance::drained_revoke_removes_object(composite_over_fake_leaves);
}

#[test]
fn composite_backend_leaves_planted_residue_alone() {
    conformance::planted_residue_survives_unrelated_revoke(composite_over_fake_leaves);
}

#[test]
fn composite_backend_snapshots_invent_nothing() {
    conformance::snapshot_never_invents_objects(composite_over_fake_leaves);
}

#[test]
fn composite_backend_gives_every_live_grant_its_own_handle() {
    conformance::live_grants_hold_distinct_handles(composite_over_fake_leaves);
}

#[test]
fn composite_backend_applies_and_removes_universe_objects() {
    conformance::apply_and_removal_reach_the_universe(composite_over_fake_leaves);
}

#[test]
fn composite_backend_rejects_a_handle_it_already_revoked() {
    conformance::revoke_of_a_revoked_handle_is_error(composite_over_fake_leaves);
}
