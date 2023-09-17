use nix_uri::{FlakeRef, NixUriError};

fn main() {
    let input = std::env::args().nth(1).expect("Please provide a uri.");

    let flake_ref: Result<FlakeRef, NixUriError> = input.parse();

    match flake_ref {
        Ok(flake_ref) => {
            println!(
                "The parsed representation of the uri is the following:\n{:#?}",
                flake_ref
            );
        }
        Err(e) => {
            println!("There was an error parsing the uri: {input} \n error:{e}");
        }
    }
}