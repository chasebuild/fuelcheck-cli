use anyhow::Result;
use clap::Parser;
use fuelcheck_core::providers::ProviderRegistry;

use fuelcheck_core::model::OutputFormat;

use fuelcheck_cli::args::{Cli, Command};
use fuelcheck_cli::commands::{
    OutputPreferences, cli_error_payload, run_config, run_cost, run_setup, run_usage,
};
use fuelcheck_cli::exit_codes::{error_kind_for_error, exit_code_for_error};
use fuelcheck_cli::logger::{self, LogLevel, LoggerConfig};

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
                    OutputFormat::Json
                } else {
                    args.format.into()
                },
                pretty: args.pretty,
                json_only: cli.global.json_only,
                no_color: cli.global.no_color,
            };
            (run_usage(args, &registry, &cli.global).await, Some(prefs))
        }
        Command::Cost(args) => {
            let prefs = OutputPreferences {
                format: if args.json || cli.global.json_only {
                    OutputFormat::Json
                } else {
                    args.format.into()
                },
                pretty: args.pretty,
                json_only: cli.global.json_only,
                no_color: cli.global.no_color,
            };
            (run_cost(args, &registry, &cli.global).await, Some(prefs))
        }
        Command::Config(cmd) => {
            let mut format = cmd.command.format();
            if cli.global.json_only {
                format = OutputFormat::Json;
            }
            let prefs = OutputPreferences {
                format,
                pretty: cmd.command.pretty(),
                json_only: cli.global.json_only,
                no_color: cli.global.no_color,
            };
            (run_config(cmd, &cli.global).await, Some(prefs))
        }
        Command::Setup(args) => (run_setup(args).await, None),
    };

    if let Err(err) = result {
        let code = exit_code_for_error(&err);
        let kind = error_kind_for_error(&err);
        if let Some(prefs) = output_prefs {
            if prefs.uses_json_output() {
                let payload = cli_error_payload(code, err.to_string(), kind);
                let outputs = vec![payload];
                if prefs.pretty {
                    if let Ok(json) = serde_json::to_string_pretty(&outputs) {
                        println!("{}", json);
                    }
                } else if let Ok(json) = serde_json::to_string(&outputs) {
                    println!("{}", json);
                }
            } else {
                eprintln!("Error: {}", err);
            }
        } else {
            eprintln!("Error: {}", err);
        }
        std::process::exit(code);
    }

    Ok(())
}
