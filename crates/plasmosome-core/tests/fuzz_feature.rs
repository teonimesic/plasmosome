#[cfg(feature = "fuzzing")]
#[path = "fuzz/replay.rs"]
mod replay;

#[cfg(feature = "fuzzing")]
#[test]
fn the_public_check_replays_the_operator_corpus_and_invalid_bytes() {
    replay::replay("operator_file", plasmosome_core::fuzz_operator_file);
    for bytes in [b"".as_ref(), b"\xff", b"{", b"[]"] {
        plasmosome_core::fuzz_operator_file(bytes);
    }
}
