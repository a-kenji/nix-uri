#![allow(clippy::print_stdout, clippy::print_stderr)]

use nix_uri::{Context, FlakeRef};

fn main() {
    let mut args = std::env::args().skip(1);

    let Some(input) = args.next() else {
        println!("Usage: cli <uri> [standalone|input|fragment]");
        return;
    };

    let parsed = match args.next().as_deref() {
        None | Some("standalone") => {
            FlakeRef::parse(&input, Context::Standalone).map(|r| (r, None))
        }
        Some("input") => FlakeRef::parse(&input, Context::FlakeInput).map(|r| (r, None)),
        Some("fragment") => FlakeRef::parse_with_fragment(&input),
        Some(other) => {
            eprintln!("Unknown context `{other}`, expected standalone, input or fragment.");
            return;
        }
    };

    match parsed {
        Ok((flake_ref, fragment)) => {
            println!(
                "{}",
                serde_json::to_string(&flake_ref.to_attrs()).expect("attributes serialise")
            );
            println!("{flake_ref}");
            if let Some(fragment) = fragment {
                println!("#{fragment}");
            }
        }
        Err(e) => {
            eprintln!("Parsing error on input: {input}\nError: {e}");
            std::process::exit(1);
        }
    }
}
