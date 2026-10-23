#![no_main]

#[path = "../../crates/plasmosome-membrane/tests/fuzz/hello_reply.rs"]
mod hello_reply;

libfuzzer_sys::fuzz_target!(|data: &[u8]| hello_reply::check(data));
