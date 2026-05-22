#![no_main]

use gumgum_cli::bucket_paths::{is_local_bucket_path, split_remote_bucket_path};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        let _ = is_local_bucket_path(input);
        let _ = split_remote_bucket_path(input);
    }
});
