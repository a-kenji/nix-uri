#![allow(clippy::print_stdout, clippy::print_stderr)]

use nix_uri::FlakeRef;

fn main() {
    let maybe_input = std::env::args().nth(1);

    let Some(input) = maybe_input else {
        println!("Error: Please provide a uri.");
        return;
    };

    match input.parse::<FlakeRef>() {
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
