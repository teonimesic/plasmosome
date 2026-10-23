#![no_main]

#[path = "../../crates/plasmosome-membrane/tests/fuzz/readiness_reply.rs"]
mod readiness_reply;

libfuzzer_sys::fuzz_target!(|data: &[u8]| readiness_reply::check(data));
