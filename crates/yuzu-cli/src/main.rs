use clap::Parser;

mod commands;

use commands::Cmd;

#[derive(Parser)]
#[command(name = "yuzu-cli", about = "Native batch backtest runner")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    commands::dispatch(Cli::parse().cmd)
}
