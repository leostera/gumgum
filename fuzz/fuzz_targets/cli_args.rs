#![no_main]

use gumgum_cli::cli_args::parse_cli_args_for_fuzz;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        let _ = parse_cli_args_for_fuzz(input);
    }
});
