mod cli;
mod config;
mod errors;
mod model;
mod providers;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use providers::ProviderRegistry;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let registry = ProviderRegistry::new();

    match cli.command {
        Command::Usage(args) => cli::run_usage(args, &registry).await,
        Command::Cost(args) => cli::run_cost(args, &registry).await,
        Command::Config(cmd) => cli::run_config(cmd).await,
        Command::Setup(args) => cli::run_setup(args).await,
    }
}
