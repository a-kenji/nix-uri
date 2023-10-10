#![no_main]

use libfuzzer_sys::fuzz_target;
use nix_uri::FlakeRef;

fuzz_target!(|data: String| {
    if let Ok(parsed) = data.parse::<FlakeRef>() {
        parsed.to_string();
    }
});
