#[path = "fuzz/hello_reply.rs"]
mod hello_reply;
#[path = "fuzz/readiness_reply.rs"]
mod readiness_reply;
#[path = "../../plasmosome-core/tests/fuzz/replay.rs"]
mod replay;

#[test]
fn the_hello_reply_corpus_replays() {
    replay::replay("hello_reply", hello_reply::check);
}

#[test]
fn the_readiness_reply_corpus_replays() {
    replay::replay("readiness_reply", readiness_reply::check);
}
