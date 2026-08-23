#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../tests/rust/fuzz_support.rs"]
mod fuzz_support;

fuzz_target!(|data: &[u8]| {
  fuzz_support::exercise_expression_pipeline(data);
});
