#![allow(clippy::print_stdout)]

use nix_uri::FlakeRef;

fn main() {
    let uri = "github:nixos/nixpkgs";
    let r#ref = "nixos-unstable";
    let mut flake_ref: FlakeRef = uri.parse().unwrap();
    flake_ref.set_ref(Some(r#ref.to_owned()));

    println!("The uri is: {uri}");
    println!("The ref is: {ref}", ref = r#ref);
    println!("The changed uri is: {flake_ref}");
}
