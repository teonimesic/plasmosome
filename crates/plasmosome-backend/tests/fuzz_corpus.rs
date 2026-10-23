#[path = "fuzz/guest_control_reply.rs"]
mod guest_control_reply;
#[path = "../../plasmosome-core/tests/fuzz/replay.rs"]
mod replay;

#[test]
fn the_guest_control_reply_corpus_replays() {
    replay::replay("guest_control_reply", guest_control_reply::check);
}
