use plasmosome_backend::FakeBackend;
use plasmosome_testkit::conformance;

#[test]
fn fake_backend_grants_are_replayable() {
    conformance::grant_is_replayable(FakeBackend::new);
}

#[test]
fn fake_backend_rejects_an_unknown_handle() {
    conformance::revoke_unknown_handle_is_error(FakeBackend::new);
}

#[test]
fn fake_backend_removes_the_object_a_drained_revoke_owned() {
    conformance::drained_revoke_removes_object(FakeBackend::new);
}

#[test]
fn fake_backend_leaves_planted_residue_alone() {
    conformance::planted_residue_survives_unrelated_revoke(FakeBackend::new);
}

#[test]
fn fake_backend_snapshots_invent_nothing() {
    conformance::snapshot_never_invents_objects(FakeBackend::new);
}
