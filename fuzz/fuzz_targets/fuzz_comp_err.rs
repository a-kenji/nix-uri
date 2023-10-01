#![no_main]

use libfuzzer_sys::fuzz_target;
use nix_uri::{FlakeRef, NixUriError, NixUriResult};

// Check if the errors are the same.

fuzz_target!(|data: String| {
    let parsed: NixUriResult<FlakeRef> = data.parse();
    let nix_cmd = check_ref(&data);
    match parsed {
        Err(err) => {
            assert!(nix_cmd.ok().is_none())
        }
        Ok(_) => nix_cmd.unwrap(),
    }
});

fn check_ref(stream: &str) -> Result<(), ()> {
    let cmd = "nix";
    let mut args = vec!["flake", "check"];
    args.push(stream);
    let mut child = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok();

    // Discard IO Errors
    if let Some(pipe) = child {
        if !pipe.status.success() {
            let stderr = pipe.stderr;
            let stderr = std::str::from_utf8(&stderr).unwrap();
            // Discard registry and file errors
            if (stderr.contains("error: cannot find flake")
                && stderr.contains("in the flake registries"))
                || stderr.contains("No such file or directory")
            {
                return Ok(());
            }
            return Err(());
        }
    }
    Ok(())
}
