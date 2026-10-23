#![no_main]

#[path = "../../crates/plasmosome-core/tests/fuzz/control_line.rs"]
mod control_line;

libfuzzer_sys::fuzz_target!(|data: &[u8]| control_line::check(data));
