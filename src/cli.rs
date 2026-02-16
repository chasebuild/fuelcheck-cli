use crate::config::{Config, ConfigCommand};
use crate::errors::CliError;
use crate::model::{ErrorKind, OutputFormat, ProviderErrorPayload, ProviderPayload};
use crate::providers::{ProviderId, ProviderRegistry, SourcePreference};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about = "Fuelcheck CLI (CodexBar-compatible)")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Fetch usage from providers (default).
    Usage(UsageArgs),
    /// Compute local cost usage (from local logs).
    Cost(CostArgs),
    /// Validate or dump config.
    Config(ConfigCommandArgs),
}

#[derive(Parser, Debug, Clone)]
pub struct UsageArgs {
    /// Provider to query (repeatable). If omitted, uses enabled providers from config.
    #[arg(short, long = "provider")]
    pub providers: Vec<ProviderId>,
    /// Source preference: auto, oauth, web, cli, api, local.
    #[arg(long, default_value = "auto")]
    pub source: SourcePreference,
    /// Output format: text or json.
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,
    /// Shortcut for --format json.
    #[arg(long)]
    pub json: bool,
    /// Pretty-print JSON output.
    #[arg(long)]
    pub pretty: bool,
    /// Show provider status.
    #[arg(long)]
    pub status: bool,
    /// JSON output only (no extra text).
    #[arg(long)]
    pub json_only: bool,
    /// Skip credits information.
    #[arg(long)]
    pub no_credits: bool,
    /// Force refresh (ignore caches).
    #[arg(long)]
    pub refresh: bool,
    /// Debug: dump HTML for web sources.
    #[arg(long)]
    pub web_debug_dump_html: bool,
    /// Web timeout in seconds.
    #[arg(long, default_value = "20")]
    pub web_timeout: u64,
    /// Explicit config path.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Prefer account by name (provider-dependent).
    #[arg(long)]
    pub account: Option<String>,
    /// Choose account index (provider-dependent).
    #[arg(long)]
    pub account_index: Option<usize>,
    /// Query all accounts when available.
    #[arg(long)]
    pub all_accounts: bool,
    /// Debug flag for Antigravity plan data.
    #[arg(long)]
    pub antigravity_plan_debug: bool,
}

#[derive(Parser, Debug, Clone)]
pub struct CostArgs {
    /// Provider to query (repeatable). If omitted, uses enabled providers from config.
    #[arg(short, long = "provider")]
    pub providers: Vec<ProviderId>,
    /// Output format: text or json.
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,
    /// Shortcut for --format json.
    #[arg(long)]
    pub json: bool,
    /// Pretty-print JSON output.
    #[arg(long)]
    pub pretty: bool,
    /// JSON output only (no extra text).
    #[arg(long)]
    pub json_only: bool,
    /// Explicit config path.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

pub async fn run_usage(args: UsageArgs, registry: &ProviderRegistry) -> Result<()> {
    let config = Config::load(args.config.as_ref())?;
    let mut args = args;
    if args.json {
        args.format = OutputFormat::Json;
    }
    let provider_ids = if args.providers.is_empty() {
        config.enabled_providers_or_default()
    } else {
        args.providers.clone()
    };

    let mut outputs: Vec<ProviderPayload> = Vec::new();
    for provider_id in provider_ids {
        let provider = registry
            .get(&provider_id)
            .ok_or_else(|| CliError::UnknownProvider(provider_id.to_string()))?;
        match provider
            .fetch_usage(&args, &config, args.source)
            .await
            .with_context(|| format!("provider {}", provider_id))
        {
            Ok(output) => outputs.push(output),
            Err(err) => outputs.push(ProviderPayload::error(
                provider_id.to_string(),
                args.source.to_string(),
                ProviderErrorPayload {
                    code: 1,
                    message: format_error_chain(&err),
                    kind: Some(ErrorKind::Provider),
                },
            )),
        }
    }

    cli_print_usage(&args, outputs)
}

pub async fn run_cost(args: CostArgs, registry: &ProviderRegistry) -> Result<()> {
    let config = Config::load(args.config.as_ref())?;
    let mut args = args;
    if args.json {
        args.format = OutputFormat::Json;
    }
    let provider_ids = if args.providers.is_empty() {
        config.enabled_providers_or_default()
    } else {
        args.providers.clone()
    };

    let mut outputs: Vec<ProviderPayload> = Vec::new();
    for provider_id in provider_ids {
        let provider = registry
            .get(&provider_id)
            .ok_or_else(|| CliError::UnknownProvider(provider_id.to_string()))?;
        match provider
            .fetch_cost(&args, &config)
            .await
            .with_context(|| format!("provider {}", provider_id))
        {
            Ok(output) => outputs.push(output),
            Err(err) => outputs.push(ProviderPayload::error(
                provider_id.to_string(),
                "local".to_string(),
                ProviderErrorPayload {
                    code: 1,
                    message: format_error_chain(&err),
                    kind: Some(ErrorKind::Provider),
                },
            )),
        }
    }

    cli_print_usage(&UsageArgs {
        providers: Vec::new(),
        source: SourcePreference::Auto,
        format: args.format,
        json: args.json,
        pretty: args.pretty,
        status: false,
        json_only: args.json_only,
        no_credits: false,
        refresh: false,
        web_debug_dump_html: false,
        web_timeout: 20,
        config: args.config,
        account: None,
        account_index: None,
        all_accounts: false,
        antigravity_plan_debug: false,
    }, outputs)
}

#[derive(Parser, Debug)]
pub struct ConfigCommandArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

pub async fn run_config(cmd: ConfigCommandArgs) -> Result<()> {
    cmd.command.execute().await
}

fn cli_print_usage(args: &UsageArgs, outputs: Vec<ProviderPayload>) -> Result<()> {
    match args.format {
        OutputFormat::Json => {
            if args.pretty {
                let json = serde_json::to_string_pretty(&outputs)?;
                println!("{}", json);
            } else {
                let json = serde_json::to_string(&outputs)?;
                println!("{}", json);
            }
        }
        OutputFormat::Text => {
            for output in outputs {
                if args.json_only {
                    continue;
                }
                println!("{}", format_payload_text(&output));
            }
        }
    }

    Ok(())
}

fn format_payload_text(payload: &ProviderPayload) -> String {
    if let Some(error) = &payload.error {
        return format!("{}: error: {}", payload.provider, error.message);
    }

    let mut lines = Vec::new();
    let header = if let Some(version) = &payload.version {
        format!("{} {} ({})", payload.provider, version, payload.source)
    } else {
        format!("{} ({})", payload.provider, payload.source)
    };
    lines.push(header);

    if let Some(usage) = &payload.usage {
        if let Some(primary) = &usage.primary {
            lines.push(format!("  primary: {:.2}%", primary.used_percent));
        }
        if let Some(secondary) = &usage.secondary {
            lines.push(format!("  secondary: {:.2}%", secondary.used_percent));
        }
        if let Some(tertiary) = &usage.tertiary {
            lines.push(format!("  tertiary: {:.2}%", tertiary.used_percent));
        }
        if let Some(cost) = &usage.provider_cost {
            lines.push(format!(
                "  cost: {:.2}/{:.2} {}",
                cost.used, cost.limit, cost.currency_code
            ));
        }
    }

    lines.join("\n")
}

fn format_error_chain(err: &anyhow::Error) -> String {
    let mut parts: Vec<String> = err.chain().map(|e| e.to_string()).collect();
    if parts.is_empty() {
        return "Unknown error".to_string();
    }
    // Avoid duplicate context strings if any.
    parts.dedup();
    parts.join(": ")
}
