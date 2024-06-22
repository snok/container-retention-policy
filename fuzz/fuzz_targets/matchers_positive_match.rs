#![no_main]
extern crate _matchers;
extern crate libfuzzer_sys;

use _matchers::Matchers;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(filters_and_value) = std::str::from_utf8(data) {
        let parts: Vec<&str> = filters_and_value.split('\n').collect();
        if parts.len() > 1 {
            let filters: Vec<String> = parts[..parts.len() - 1].iter().map(|s| s.to_string()).collect();
            let value = parts[parts.len() - 1];
            let matchers = Matchers::from(&filters);
            let _ = matchers.negative_match(value);
        }
    }
});
