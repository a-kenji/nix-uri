#![allow(clippy::print_stdout)]

use nix_uri::FlakeRef;

fn main() {
    let uri = "github:nixos/nixpkgs";
    let branch = "nixos-unstable";
    let flake_ref: FlakeRef = uri.parse().unwrap();
    let flake_ref = flake_ref.with_ref(Some(branch.to_owned())).unwrap();

    println!("The uri is: {uri}");
    println!("The ref is: {branch}");
    println!("The changed uri is: {flake_ref}");
}
