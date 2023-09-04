alias d := doc
alias l := lint
alias uf := update-flake-dependencies
alias uc := update-cargo-dependencies
alias r := run
alias t := cargo-test
alias b := build
alias rr := run-release
alias cw := cargo-watch

_default:
    -@just --choose

clippy:
    cargo clippy --all-targets --all-features
clippy-annoy:
    cargo clippy --all -- -W clippy::all -W clippy::pedantic -W clippy::restriction -W clippy::nursery -D warnings

actionlint:
    nix develop .#actionlintShell --command actionlint

deny:
    cargo deny check

cargo-test:
    cargo test

cargo-public-api:
    nix develop .#lintShell --command cargo public-api

cargo-diff:
    nix develop .#lintShell --command cargo public-api diff

lint:
    nix flake check
    typos
    cargo diet
    cargo bloat
    -cargo udeps
    -cargo outdated
    lychee *.md
    treefmt --fail-on-change
    cargo check --future-incompat-report

run:
    cargo run

build:
    cargo build

run-release:
    cargo run --release

doc:
    cargo doc --open --offline

# Update and then commit the `Cargo.lock` file
update-cargo-dependencies:
    cargo update
    git add Cargo.lock
    git commit Cargo.lock -m "chore(update): \`Cargo.lock\`"

# Future incompatibility report, run regularly
cargo-future:
    cargo check --future-incompat-report

update-flake-dependencies:
    nix flake update --commit-lock-file

cargo-watch:
    cargo watch -x check -x test -x build

cargo-tarpaulin:
    cargo tarpaulin --avoid-cfg-tarpaulin --out html
