use nix_uri::FlakeRef;
use nom::Finish;

fn main() {
    let maybe_input = std::env::args().nth(1);

    if let Some(input) = maybe_input {
        // let flake_ref: Result<FlakeRef, NixUriError> = input.parse();
        let flake_ref = FlakeRef::parse(&input).finish();

        match flake_ref {
            Ok((_, flake_ref)) => {
                println!(
                    "The parsed representation of the uri is the following:\n{:#?}",
                    flake_ref
                );
                println!("This is the flake_ref:\n{}", flake_ref);
            }
            Err(e) => {
                println!(
                    "There was an error parsing the uri: {}\nError: {}",
                    input, e
                );
            }
        }
    } else {
        println!("Error: Please provide a uri.");
    }
}
