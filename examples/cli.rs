#![allow(clippy::print_stdout, clippy::print_stderr)]

use nix_uri::{Context, FlakeRef};

fn main() {
    let mut args = std::env::args().skip(1);

    let Some(input) = args.next() else {
        println!("Usage: cli <uri> [standalone|input|installable]");
        return;
    };

    let context = match args.next().as_deref() {
        None | Some("input") => Context::FlakeInput,
        Some("standalone") => Context::Standalone,
        Some("installable") => Context::Installable,
        Some(other) => {
            eprintln!("Unknown context `{other}`, expected standalone, input or installable.");
            return;
        }
    };

    match FlakeRef::parse(&input, context) {
        Ok(flake_ref) => {
            println!(
                "The parsed representation of the uri is the following:\n{:#?}",
                flake_ref
            );
            println!("This is the flake_ref:\n{}", flake_ref);
        }
        Err(e) => {
            eprintln!("Parsing error on input: {}\nError: {}", input, e);
        }
    }
}
