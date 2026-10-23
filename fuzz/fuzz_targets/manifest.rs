#![no_main]

#[path = "../../crates/plasmosome-core/tests/fuzz/manifest.rs"]
mod manifest;

libfuzzer_sys::fuzz_target!(|data: &[u8]| manifest::check(data));
