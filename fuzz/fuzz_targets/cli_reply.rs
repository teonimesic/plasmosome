#![no_main]

#[path = "../../crates/plasmosome-cli/tests/fuzz/cli_reply.rs"]
mod cli_reply;

libfuzzer_sys::fuzz_target!(|data: &[u8]| cli_reply::check(data));
