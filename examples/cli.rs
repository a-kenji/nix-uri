use nix_uri::{FlakeRef, NixUriError};
use nom::Finish;

fn main() {
    let maybe_input = std::env::args().nth(1);

    if let Some(input) = maybe_input {
        let strparse_ref: Result<FlakeRef, NixUriError> = input.parse();
        let parse_str_ref = FlakeRef::parse(&input).finish();

        match strparse_ref {
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

        match parse_str_ref {
            Ok((_, flakeref)) => println!("The nommed parse:\n{:#?}", flakeref),
            Err(e) => eprintln!("The verbose error info:\n{}", e),
        }
    } else {
        println!("Error: Please provide a uri.");
    }
}
