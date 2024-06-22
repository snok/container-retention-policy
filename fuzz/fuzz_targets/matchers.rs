#![no_main]
extern crate _matchers;
extern crate libfuzzer_sys;

use _matchers::Matchers;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Convert data to a Vec<String>
    let filters: Vec<String> = data
        .split(|&b| b == 0) // Split the input data at null bytes
        .map(|slice| String::from_utf8_lossy(slice).to_string())
        .collect();

    // Call the `from` method
    let _ = Matchers::from(&filters);
});
