#[path = "fuzz/cli_reply.rs"]
mod cli_reply;
#[path = "../../plasmosome-core/tests/fuzz/replay.rs"]
mod replay;

#[test]
fn the_cli_reply_corpus_replays() {
    replay::replay("cli_reply", cli_reply::check);
}
