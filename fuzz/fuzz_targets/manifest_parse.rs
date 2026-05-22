#![no_main]

use gumgum_core::validate_str;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        let _ = validate_str(input);
    }
});
