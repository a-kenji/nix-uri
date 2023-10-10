#![no_main]

use libfuzzer_sys::fuzz_target;
use nix_uri::FlakeRef;

fuzz_target!(|data: String| {
    if let Ok(parsed) = data.parse::<FlakeRef>() {
        let uri = parsed.to_string();
        let re_parsed = uri.parse::<FlakeRef>().unwrap();
        assert_eq!(parsed, re_parsed);
    }
});
