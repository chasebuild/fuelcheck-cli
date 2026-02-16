mod accounts;
mod cli;
mod accounts;
mod config;
mod errors;
mod logger;
mod model;
mod providers;
mod tui;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command, OutputPreferences};
use logger::{LogLevel, LoggerConfig};
use providers::ProviderRegistry;
use std::process;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let registry = ProviderRegistry::new();

    let log_level = if let Some(level) = cli.global.log_level {
        level
    } else if cli.global.verbose {
        LogLevel::Verbose
    } else {
        LogLevel::Warning
    };
    logger::init(LoggerConfig {
        level: log_level,
        json_output: cli.global.json_output,
        json_only: cli.global.json_only,
    });

    let (result, output_prefs) = match cli.command {
        Command::Usage(args) => {
            let prefs = OutputPreferences {
                format: if args.json || cli.global.json_only {
                    crate::model::OutputFormat::Json
                } else {
                    args.format
                },
                pretty: args.pretty,
                json_only: cli.global.json_only,
                no_color: cli.global.no_color,
            };
            (
                cli::run_usage(args, &registry, &cli.global).await,
                Some(prefs),
            )
        }
        Command::Cost(args) => {
            let prefs = OutputPreferences {
                format: if args.json || cli.global.json_only {
                    crate::model::OutputFormat::Json
                } else {
                    args.format
                },
                pretty: args.pretty,
                json_only: cli.global.json_only,
                no_color: cli.global.no_color,
            };
            (
                cli::run_cost(args, &registry, &cli.global).await,
                Some(prefs),
            )
        }
        Command::Config(cmd) => {
            let mut format = cmd.command.format();
            if cli.global.json_only {
                format = crate::model::OutputFormat::Json;
            }
            let prefs = OutputPreferences {
                format,
                pretty: cmd.command.pretty(),
                json_only: cli.global.json_only,
                no_color: cli.global.no_color,
            };
            (cli::run_config(cmd, &cli.global).await, Some(prefs))
        }
        Command::Setup(args) => (cli::run_setup(args).await, None),
    };

    if let Err(err) = result {
        let code = cli::exit_code_for_error(&err);
        let kind = cli::error_kind_for_error(&err);
        if let Some(prefs) = output_prefs {
            if prefs.uses_json_output() {
                let payload = crate::model::ProviderPayload::error(
                    "cli".to_string(),
                    "cli".to_string(),
                    crate::model::ProviderErrorPayload {
                        code,
                        message: err.to_string(),
                        kind: Some(kind),
                    },
                );
                if prefs.pretty {
                    if let Ok(json) = serde_json::to_string_pretty(&vec![payload]) {
                        println!("{}", json);
                    }
                } else if let Ok(json) = serde_json::to_string(&vec![payload]) {
                    println!("{}", json);
                }
            } else {
                eprintln!("Error: {}", err);
            }
        } else {
            eprintln!("Error: {}", err);
        }
        process::exit(code);
    }

    Ok(())
}
