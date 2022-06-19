use clap::Parser;

#[derive(Parser, Default, Debug, Clone)]
#[clap(version)]
pub struct CliArgs {
    pub max_panes: Option<usize>,
}

fn main() {
    let opts = CliArgs::parse();
    println!("Hello, world!");
}
