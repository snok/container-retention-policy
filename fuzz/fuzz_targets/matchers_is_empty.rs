#![no_main]
extern crate _matchers;
extern crate libfuzzer_sys;

use _matchers::Matchers;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(filters) = std::str::from_utf8(data) {
        let filter_vec: Vec<String> = filters.split('\n').map(|s| s.to_string()).collect();
        let matchers = Matchers::from(&filter_vec);
        let _ = matchers.is_empty();
    }
});
