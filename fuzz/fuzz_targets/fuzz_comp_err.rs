#![no_main]

use libfuzzer_sys::fuzz_target;
use nix_uri::{FlakeRef, NixUriError, NixUriResult};

// Check if the errors are the same.

fuzz_target!(|data: String| {
    let parsed: NixUriResult<FlakeRef> = data.parse();
    let nix_cmd = check_ref(&data);
    match parsed {
        Err(err) => {
            // Discard registry and file errors
            if let Err(ref cmd_err) = nix_cmd {
                if (cmd_err.contains("error: cannot find flake")
                    && cmd_err.contains("in the flake registries"))
                    || cmd_err.contains("No such file or directory")
                {
                } else {
                    assert!(nix_cmd.ok().is_none())
                }
            }
        }
        Ok(_) => {
            if let Err(err) = nix_cmd {
                // Discard registry and file errors
                if (err.contains("error: cannot find flake")
                    && err.contains("in the flake registries"))
                    || err.contains("No such file or directory")
                {
                } else {
                    panic!();
                }
            }
        }
    }
});

fn check_ref(stream: &str) -> Result<(), String> {
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
            return Err(stderr.into());
        }
    }
    Ok(())
}
