#[path = "fuzz/hello_reply.rs"]
mod hello_reply;
#[path = "../../plasmosome-core/tests/fuzz/replay.rs"]
mod replay;

#[test]
fn the_hello_reply_corpus_replays() {
    replay::replay("hello_reply", hello_reply::check);
}
