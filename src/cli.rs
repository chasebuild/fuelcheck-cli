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
    /// Setup config with detected credentials.
    Setup(SetupArgs),
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
    /// Choose account index (1-based, provider-dependent).
    #[arg(long)]
    pub account_index: Option<usize>,
    /// Query all accounts when available.
    #[arg(long)]
    pub all_accounts: bool,
    /// Debug flag for Antigravity plan data.
    #[arg(long)]
    pub antigravity_plan_debug: bool,
    /// Watch usage + cost in a live TUI.
    #[arg(long)]
    pub watch: bool,
    /// Refresh interval (seconds) for --watch.
    #[arg(long, default_value = "10")]
    pub interval: u64,
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

#[derive(Parser, Debug, Clone)]
pub struct SetupArgs {
    /// Overwrite existing config file if present.
    #[arg(long)]
    pub force: bool,
    /// Enable all providers regardless of detected credentials.
    #[arg(long)]
    pub enable_all: bool,
    /// Provide Claude cookie header or sessionKey explicitly (uses web source).
    #[arg(long)]
    pub claude_cookie: Option<String>,
    /// Provide Cursor cookie header explicitly (skips browser import).
    #[arg(long)]
    pub cursor_cookie: Option<String>,
    /// Provide Factory (Droid) cookie header explicitly.
    #[arg(long, alias = "droid-cookie")]
    pub factory_cookie: Option<String>,
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
    if args.watch {
        if args.format == OutputFormat::Json || args.json_only {
            return Err(anyhow::anyhow!("--watch only supports text output"));
        }
        return crate::tui::run_usage_watch(args, registry, config).await;
    }

    let outputs = collect_usage_outputs(&args, &config, registry).await?;

    cli_print_usage(&args, outputs)
}

pub(crate) async fn collect_usage_outputs(
    args: &UsageArgs,
    config: &Config,
    registry: &ProviderRegistry,
) -> Result<Vec<ProviderPayload>> {
    let provider_ids = if args.providers.is_empty() {
        config.enabled_providers_or_default()
    } else {
        args.providers.clone()
    };

    let wants_account_override =
        args.account.is_some() || args.account_index.is_some() || args.all_accounts;
    if wants_account_override && provider_ids.len() != 1 {
        return Err(anyhow::anyhow!(
            "account selection requires a single provider"
        ));
    }
    if wants_account_override {
        let provider_id = provider_ids
            .first()
            .ok_or_else(|| anyhow::anyhow!("no provider selected"))?;
        let provider = registry
            .get(provider_id)
            .ok_or_else(|| CliError::UnknownProvider(provider_id.to_string()))?;
        if !provider.supports_token_accounts() {
            return Err(anyhow::anyhow!(
                "provider {} does not support token accounts",
                provider_id
            ));
        }
    }

    let mut outputs: Vec<ProviderPayload> = Vec::new();
    for provider_id in provider_ids {
        let provider = registry
            .get(&provider_id)
            .ok_or_else(|| CliError::UnknownProvider(provider_id.to_string()))?;
        match provider
            .fetch_usage_all(args, config, args.source)
            .await
            .with_context(|| format!("provider {}", provider_id))
        {
            Ok(mut output_set) => outputs.append(&mut output_set),
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

    Ok(outputs)
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
        watch: false,
        interval: 10,
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

pub async fn run_setup(args: SetupArgs) -> Result<()> {
    let config_path = Config::path(args.config.as_ref())?;
    if config_path.exists() && !args.force {
        return Err(anyhow::anyhow!(
            "Config already exists at {}. Use --force to overwrite.",
            config_path.display()
        ));
    }

    let mut providers = Vec::new();
    let detected = crate::config::DetectResult::detect();

    let mut enable = |id: ProviderId, enabled: bool, mut extra: crate::config::ProviderConfig| {
        extra.id = id;
        extra.enabled = Some(enabled);
        providers.push(extra);
    };

    let enable_all = args.enable_all;

    // Codex
    enable(
        ProviderId::Codex,
        enable_all || detected.codex_auth,
        crate::config::ProviderConfig {
            id: ProviderId::Codex,
            enabled: Some(enable_all || detected.codex_auth),
            source: Some(crate::providers::SourcePreference::Oauth),
            ..crate::config::ProviderConfig::default_provider(ProviderId::Codex)
        },
    );

    // Claude
    let mut claude_cfg = crate::config::ProviderConfig {
        id: ProviderId::Claude,
        enabled: Some(enable_all || detected.claude_oauth || args.claude_cookie.is_some()),
        source: Some(crate::providers::SourcePreference::Oauth),
        ..crate::config::ProviderConfig::default_provider(ProviderId::Claude)
    };
    if let Some(cookie) = args.claude_cookie.clone() {
        claude_cfg.cookie_header = Some(cookie);
        claude_cfg.source = Some(crate::providers::SourcePreference::Web);
    }
    enable(ProviderId::Claude, claude_cfg.enabled.unwrap_or(false), claude_cfg);

    // Gemini
    enable(
        ProviderId::Gemini,
        enable_all || detected.gemini_oauth,
        crate::config::ProviderConfig {
            id: ProviderId::Gemini,
            enabled: Some(enable_all || detected.gemini_oauth),
            source: Some(crate::providers::SourcePreference::Api),
            ..crate::config::ProviderConfig::default_provider(ProviderId::Gemini)
        },
    );

    // Cursor
    let mut cursor_cfg = crate::config::ProviderConfig {
        id: ProviderId::Cursor,
        enabled: Some(enable_all || args.cursor_cookie.is_some()),
        source: Some(crate::providers::SourcePreference::Web),
        ..crate::config::ProviderConfig::default_provider(ProviderId::Cursor)
    };
    if let Some(cookie) = args.cursor_cookie.clone() {
        cursor_cfg.cookie_header = Some(cookie);
    }
    enable(ProviderId::Cursor, cursor_cfg.enabled.unwrap_or(false), cursor_cfg);

    // Factory (Droid)
    let mut factory_cfg = crate::config::ProviderConfig {
        id: ProviderId::Factory,
        enabled: Some(enable_all || args.factory_cookie.is_some()),
        source: Some(crate::providers::SourcePreference::Web),
        ..crate::config::ProviderConfig::default_provider(ProviderId::Factory)
    };
    if let Some(cookie) = args.factory_cookie.clone() {
        factory_cfg.cookie_header = Some(cookie);
    }
    enable(ProviderId::Factory, factory_cfg.enabled.unwrap_or(false), factory_cfg);

    let config = Config {
        version: Some(1),
        providers: Some(providers),
    };

    config.save(args.config.as_ref())?;

    println!("Setup complete. Config written to {}", config_path.display());
    if !detected.codex_auth {
        println!("Codex: run `codex` to authenticate (creates ~/.codex/auth.json).");
    }
    if !detected.claude_oauth && args.claude_cookie.is_none() {
        println!("Claude: run `claude` to authenticate (creates ~/.claude/.credentials.json).");
        println!("Claude: or provide a session cookie via `fuelcheck-cli setup --claude-cookie \"sessionKey=...\"`.");
    }
    if !detected.gemini_oauth {
        println!("Gemini: run `gemini` to authenticate (creates ~/.gemini/oauth_creds.json).");
    }
    if args.cursor_cookie.is_none() {
        println!("Cursor: add cookie header via `fuelcheck-cli setup --cursor-cookie \"...\"`.");
    }
    if args.factory_cookie.is_none() {
        println!("Factory (Droid): add cookie header via `fuelcheck-cli setup --factory-cookie \"...\"`.");
    }

    Ok(())
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
    let mut header = if let Some(version) = &payload.version {
        format!("{} {} ({})", payload.provider, version, payload.source)
    } else {
        format!("{} ({})", payload.provider, payload.source)
    };
    if let Some(account) = resolve_account_label(payload) {
        header.push_str(&format!(" | account: {}", account));
    }
    if let Some(plan) = payload
        .usage
        .as_ref()
        .and_then(|usage| usage.login_method.clone())
    {
        header.push_str(&format!(" | plan: {}", plan));
    }
    lines.push(header);

    if let Some(usage) = &payload.usage {
        if let Some(primary) = &usage.primary {
            lines.push(format_rate_window_text("primary", primary));
        }
        if let Some(secondary) = &usage.secondary {
            lines.push(format_rate_window_text("secondary", secondary));
        }
        if let Some(tertiary) = &usage.tertiary {
            lines.push(format_rate_window_text("tertiary", tertiary));
        }
        if let Some(cost) = &usage.provider_cost {
            lines.push(format_provider_cost_text(cost));
        } else {
            lines.push("  cost: n/a".to_string());
        }
        if let Some(credits) = &payload.credits {
            lines.push(format!("  credits: {:.2}", credits.remaining));
        } else if let Some(dashboard) = &payload.openai_dashboard {
            if let Some(credits) = dashboard.credits_remaining {
                lines.push(format!("  credits: {:.2}", credits));
            }
        }
        lines.push(format!("  updated: {}", format_timestamp(usage.updated_at)));
    }

    if payload.usage.is_none() && payload.credits.is_some() {
        if let Some(credits) = &payload.credits {
            lines.push(format!("  credits: {:.2}", credits.remaining));
        }
    }

    lines.join("\n")
}

fn format_rate_window_text(label: &str, window: &crate::model::RateWindow) -> String {
    let bar = percent_bar(window.used_percent, 20);
    let mut parts = vec![format!(
        "  {}: {:>5.1}% [{}]",
        label, window.used_percent, bar
    )];
    if let Some(desc) = &window.reset_description {
        parts.push(desc.clone());
    }
    if let Some(minutes) = window.window_minutes {
        parts.push(format!("window {}m", minutes));
    }
    parts.join(" | ")
}

fn resolve_account_label(payload: &ProviderPayload) -> Option<String> {
    payload
        .account
        .clone()
        .or_else(|| payload.usage.as_ref().and_then(|u| u.account_email.clone()))
        .or_else(|| {
            payload
                .usage
                .as_ref()
                .and_then(|u| u.account_organization.clone())
        })
}

fn format_provider_cost_text(cost: &crate::model::ProviderCostSnapshot) -> String {
    let mut parts = vec![format!(
        "  cost: {:.2}/{:.2} {}",
        cost.used, cost.limit, cost.currency_code
    )];
    if let Some(period) = &cost.period {
        parts.push(period.clone());
    }
    if let Some(resets_at) = cost.resets_at {
        parts.push(format!("resets {}", format_timestamp(resets_at)));
    }
    parts.join(" | ")
}

fn percent_bar(percent: f64, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let clamped = percent.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let mut bar = String::with_capacity(width);
    for _ in 0..filled {
        bar.push('#');
    }
    for _ in filled..width {
        bar.push('-');
    }
    bar
}

fn format_timestamp(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
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
