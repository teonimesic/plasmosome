#![no_main]

#[path = "../../crates/plasmosome-backend/tests/fuzz/guest_control_reply.rs"]
mod guest_control_reply;

libfuzzer_sys::fuzz_target!(|data: &[u8]| guest_control_reply::check(data));
