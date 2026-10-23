#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| plasmosome_core::fuzz_operator_file(data));
