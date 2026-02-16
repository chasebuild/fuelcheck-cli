use crate::config::{Config, ConfigCommand};
use crate::errors::CliError;
use crate::logger::{self, LogLevel};
use crate::model::{ErrorKind, OutputFormat, ProviderErrorPayload, ProviderPayload};
use crate::providers::{ProviderId, ProviderRegistry, ProviderSelector, SourcePreference};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::io::IsTerminal;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about = "Fuelcheck CLI (CodexBar-compatible)")]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Parser, Debug, Clone)]
pub struct GlobalArgs {
    /// Disable ANSI colors.
    #[arg(long, global = true)]
    pub no_color: bool,
    /// Log level: trace|verbose|debug|info|warning|error|critical.
    #[arg(long, global = true)]
    pub log_level: Option<LogLevel>,
    /// Emit JSONL logs on stderr.
    #[arg(long, global = true)]
    pub json_output: bool,
    /// JSON output only (no extra text).
    #[arg(long, global = true)]
    pub json_only: bool,
    /// Verbose logging (alias for --log-level verbose).
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,
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
    /// Provider to query (repeatable, accepts all|both). If omitted, uses enabled providers from config.
    #[arg(short, long = "provider")]
    pub providers: Vec<ProviderSelector>,
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
    // NOTE: json_only is global.
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
    /// Provider to query (repeatable, accepts all|both). If omitted, uses enabled providers from config.
    #[arg(short, long = "provider")]
    pub providers: Vec<ProviderSelector>,
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
    // NOTE: json_only is global.
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

pub struct OutputPreferences {
    pub format: OutputFormat,
    pub pretty: bool,
    pub json_only: bool,
    pub no_color: bool,
}

impl OutputPreferences {
    pub fn uses_json_output(&self) -> bool {
        self.json_only || self.format == OutputFormat::Json
    }

    pub fn use_color(&self) -> bool {
        if self.format == OutputFormat::Json {
            return false;
        }
        if self.no_color {
            return false;
        }
        if std::env::var("NO_COLOR").is_ok() {
            return false;
        }
        std::io::stdout().is_terminal()
    }
}

pub async fn run_usage(
    args: UsageArgs,
    registry: &ProviderRegistry,
    global: &GlobalArgs,
) -> Result<()> {
    let config = Config::load(args.config.as_ref())?;
    if let Ok(path) = Config::path(args.config.as_ref()) {
        logger::log(
            LogLevel::Info,
            "config_loaded",
            "Loaded config",
            Some(json!({ "path": path.display().to_string(), "missing": !path.exists() })),
        );
    }
    let mut args = args;
    if args.json || global.json_only {
        args.format = OutputFormat::Json;
    }
    if args.watch {
        if args.format == OutputFormat::Json || global.json_only {
            return Err(anyhow::anyhow!("--watch only supports text output"));
        }
        return crate::tui::run_usage_watch(args, registry, config).await;
    }

    let outputs = collect_usage_outputs(&args, &config, registry).await?;
    let prefs = OutputPreferences {
        format: args.format,
        pretty: args.pretty,
        json_only: global.json_only,
        no_color: global.no_color,
    };
    cli_print_usage(&args, outputs, &prefs)
}

pub(crate) async fn collect_usage_outputs(
    args: &UsageArgs,
    config: &Config,
    registry: &ProviderRegistry,
) -> Result<Vec<ProviderPayload>> {
    let provider_ids = if args.providers.is_empty() {
        config.enabled_providers_or_default()
    } else {
        crate::providers::expand_provider_selectors(&args.providers)
    };
    logger::log(
        LogLevel::Info,
        "providers_selected",
        "Resolved providers",
        Some(
            json!({ "providers": provider_ids.iter().map(|p| p.to_string()).collect::<Vec<_>>() }),
        ),
    );
    logger::log(
        LogLevel::Info,
        "providers_selected",
        "Resolved providers",
        Some(
            json!({ "providers": provider_ids.iter().map(|p| p.to_string()).collect::<Vec<_>>() }),
        ),
    );

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
        logger::log(
            LogLevel::Info,
            "provider_fetch_start",
            format!("Fetching provider {}", provider_id),
            Some(json!({ "provider": provider_id.to_string(), "source": args.source.to_string() })),
        );
        let provider = registry
            .get(&provider_id)
            .ok_or_else(|| CliError::UnknownProvider(provider_id.to_string()))?;
        match provider
            .fetch_usage_all(args, config, args.source)
            .await
            .with_context(|| format!("provider {}", provider_id))
        {
            Ok(mut output_set) => {
                logger::log(
                    LogLevel::Info,
                    "provider_fetch_ok",
                    format!("Provider {} fetched", provider_id),
                    None,
                );
                outputs.append(&mut output_set)
            }
            Err(err) => {
                logger::log(
                    LogLevel::Error,
                    "provider_fetch_error",
                    format!("Provider {} failed: {}", provider_id, err),
                    None,
                );
                outputs.push(ProviderPayload::error(
                    provider_id.to_string(),
                    args.source.to_string(),
                    ProviderErrorPayload {
                        code: 1,
                        message: format_error_chain(&err),
                        kind: Some(ErrorKind::Provider),
                    },
                ))
            }
        }
    }

    Ok(outputs)
}

pub async fn run_cost(
    args: CostArgs,
    registry: &ProviderRegistry,
    global: &GlobalArgs,
) -> Result<()> {
    let config = Config::load(args.config.as_ref())?;
    if let Ok(path) = Config::path(args.config.as_ref()) {
        logger::log(
            LogLevel::Info,
            "config_loaded",
            "Loaded config",
            Some(json!({ "path": path.display().to_string(), "missing": !path.exists() })),
        );
    }
    let mut args = args;
    if args.json || global.json_only {
        args.format = OutputFormat::Json;
    }
    let provider_ids = if args.providers.is_empty() {
        config.enabled_providers_or_default()
    } else {
        crate::providers::expand_provider_selectors(&args.providers)
    };

    let mut outputs: Vec<ProviderPayload> = Vec::new();
    for provider_id in provider_ids {
        logger::log(
            LogLevel::Info,
            "provider_cost_start",
            format!("Fetching cost for provider {}", provider_id),
            Some(json!({ "provider": provider_id.to_string() })),
        );
        let provider = registry
            .get(&provider_id)
            .ok_or_else(|| CliError::UnknownProvider(provider_id.to_string()))?;
        match provider
            .fetch_cost(&args, &config)
            .await
            .with_context(|| format!("provider {}", provider_id))
        {
            Ok(output) => {
                logger::log(
                    LogLevel::Info,
                    "provider_cost_ok",
                    format!("Provider {} cost fetched", provider_id),
                    None,
                );
                outputs.push(output)
            }
            Err(err) => {
                logger::log(
                    LogLevel::Error,
                    "provider_cost_error",
                    format!("Provider {} cost failed: {}", provider_id, err),
                    None,
                );
                outputs.push(ProviderPayload::error(
                    provider_id.to_string(),
                    "local".to_string(),
                    ProviderErrorPayload {
                        code: 1,
                        message: format_error_chain(&err),
                        kind: Some(ErrorKind::Provider),
                    },
                ))
            }
        }
    }

    let prefs = OutputPreferences {
        format: args.format,
        pretty: args.pretty,
        json_only: global.json_only,
        no_color: global.no_color,
    };
    cli_print_usage(
        &UsageArgs {
            providers: Vec::new(),
            source: SourcePreference::Auto,
            format: args.format,
            json: args.json,
            pretty: args.pretty,
            status: false,
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
        },
        outputs,
        &prefs,
    )
}

#[derive(Parser, Debug)]
pub struct ConfigCommandArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

pub async fn run_config(cmd: ConfigCommandArgs, global: &GlobalArgs) -> Result<()> {
    let mut cmd = cmd;
    if global.json_only {
        match &mut cmd.command {
            ConfigCommand::Validate(args) => args.format = Some(OutputFormat::Json),
            ConfigCommand::Dump(args) => args.format = Some(OutputFormat::Json),
        }
    }
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
    enable(
        ProviderId::Claude,
        claude_cfg.enabled.unwrap_or(false),
        claude_cfg,
    );

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
    enable(
        ProviderId::Cursor,
        cursor_cfg.enabled.unwrap_or(false),
        cursor_cfg,
    );

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
    enable(
        ProviderId::Factory,
        factory_cfg.enabled.unwrap_or(false),
        factory_cfg,
    );

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
    enable(
        ProviderId::Factory,
        factory_cfg.enabled.unwrap_or(false),
        factory_cfg,
    );

    let config = Config {
        version: Some(1),
        providers: Some(providers),
    };

    config.save(args.config.as_ref())?;

    println!(
        "Setup complete. Config written to {}",
        config_path.display()
    );
    if !detected.codex_auth {
        println!("Codex: run `codex` to authenticate (creates ~/.codex/auth.json).");
    }
    if !detected.claude_oauth && args.claude_cookie.is_none() {
        println!("Claude: run `claude` to authenticate (creates ~/.claude/.credentials.json).");
        println!(
            "Claude: or provide a session cookie via `fuelcheck-cli setup --claude-cookie \"sessionKey=...\"`."
        );
    }
    if !detected.gemini_oauth {
        println!("Gemini: run `gemini` to authenticate (creates ~/.gemini/oauth_creds.json).");
    }
    if args.cursor_cookie.is_none() {
        println!("Cursor: add cookie header via `fuelcheck-cli setup --cursor-cookie \"...\"`.");
    }
    if args.factory_cookie.is_none() {
        println!(
            "Factory (Droid): add cookie header via `fuelcheck-cli setup --factory-cookie \"...\"`."
        );
    }

    Ok(())
}

pub(crate) fn cli_print_usage(
    _args: &UsageArgs,
    outputs: Vec<ProviderPayload>,
    prefs: &OutputPreferences,
) -> Result<()> {
    match prefs.format {
        OutputFormat::Json => {
            if prefs.pretty {
                let json = serde_json::to_string_pretty(&outputs)?;
                println!("{}", json);
            } else {
                let json = serde_json::to_string(&outputs)?;
                println!("{}", json);
            }
        }
        OutputFormat::Text => {
            if prefs.json_only {
                return Ok(());
            }
            for output in outputs {
                if let Some(error) = &output.error {
                    if let Some(account) = &output.account {
                        eprintln!(
                            "Error ({} - {}): {}",
                            output.provider, account, error.message
                        );
                    } else {
                        eprintln!("Error: {}", error.message);
                    }
                    continue;
                }
                println!("{}", format_payload_text(&output, prefs));
            }
        }
    }

    Ok(())
}

pub(crate) fn format_payload_text(payload: &ProviderPayload, prefs: &OutputPreferences) -> String {
    if let Some(error) = &payload.error {
        return format!("{}: error: {}", payload.provider, error.message);
    }

    let mut lines = Vec::new();
    let header = format!(
        "== {} ==",
        format_header_title(
            provider_display_name(&payload.provider),
            payload.version.as_deref(),
            &payload.source
        )
    );
    lines.push(colorize_header(&header, prefs.use_color()));

    if let Some(usage) = &payload.usage {
        if let Some(primary) = &usage.primary {
            lines.push(rate_line("Session", primary, prefs.use_color()));
            if let Some(reset) = reset_line(primary) {
                lines.push(subtle_line(&reset, prefs.use_color()));
            }
        }
        if let Some(secondary) = &usage.secondary {
            lines.push(rate_line("Weekly", secondary, prefs.use_color()));
            if let Some(pace) = pace_line(&payload.provider, secondary) {
                lines.push(label_line("Pace", &pace, prefs.use_color()));
            }
            if let Some(reset) = reset_line(secondary) {
                lines.push(subtle_line(&reset, prefs.use_color()));
            }
        }
        if let Some(tertiary) = &usage.tertiary {
            let label = tertiary_label(&payload.provider);
            lines.push(rate_line(label, tertiary, prefs.use_color()));
            if let Some(reset) = reset_line(tertiary) {
                lines.push(subtle_line(&reset, prefs.use_color()));
            }
        }
        if let Some(cost) = &usage.provider_cost {
            lines.push(cost_line(cost));
        } else {
            // no-op
        }
        if payload.provider == "codex" {
            if let Some(credits) = &payload.credits {
                lines.push(label_line(
                    "Credits",
                    &format_credits(credits.remaining),
                    prefs.use_color(),
                ));
            } else if let Some(dashboard) = &payload.openai_dashboard
                && let Some(credits) = dashboard.credits_remaining
            {
                lines.push(label_line(
                    "Credits",
                    &format_credits(credits),
                    prefs.use_color(),
                ));
            }
        }
        if let Some(account) = usage.account_email.clone().or_else(|| {
            usage
                .identity
                .as_ref()
                .and_then(|i| i.account_email.clone())
        }) {
            lines.push(label_line("Account", &account, prefs.use_color()));
        }
        if let Some(plan) = usage
            .login_method
            .clone()
            .or_else(|| usage.identity.as_ref().and_then(|i| i.login_method.clone()))
            && !plan.is_empty()
        {
            lines.push(label_line("Plan", &plan, prefs.use_color()));
        }
    }

    if let Some(status) = &payload.status {
        let status_text = status_line(status);
        lines.push(colorize_status(
            &status_text,
            status.indicator.clone(),
            prefs.use_color(),
        ));
    }

    lines.join("\n")
}

fn format_header_title(provider: String, version: Option<&str>, source: &str) -> String {
    match version {
        Some(ver) => format!("{} {} ({})", provider, ver, source),
        None => format!("{} ({})", provider, source),
    }
}

fn provider_display_name(raw: &str) -> String {
    match raw {
        "codex" => "Codex".to_string(),
        "claude" => "Claude".to_string(),
        "gemini" => "Gemini".to_string(),
        "cursor" => "Cursor".to_string(),
        "factory" => "Factory".to_string(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => other.to_string(),
            }
        }
    }
}

fn tertiary_label(provider: &str) -> &'static str {
    match provider {
        "claude" => "Sonnet",
        _ => "Tertiary",
    }
}

fn rate_line(label: &str, window: &crate::model::RateWindow, use_color: bool) -> String {
    let remaining = remaining_percent(window.used_percent);
    let usage_text = usage_line(remaining, window.used_percent);
    let colored_usage = colorize_usage(&usage_text, remaining, use_color);
    let bar = usage_bar(remaining, use_color);
    format!("{}: {} {}", label, colored_usage, bar)
}

fn usage_line(remaining: f64, used: f64) -> String {
    let percent = remaining.clamp(0.0, 100.0);
    if used.is_nan() {
        format!("{:.0}% left", percent)
    } else {
        format!("{:.0}% left", percent)
    }
}

fn remaining_percent(used_percent: f64) -> f64 {
    (100.0 - used_percent).clamp(0.0, 100.0)
}

fn usage_bar(remaining: f64, use_color: bool) -> String {
    let clamped = remaining.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * 12.0).round() as usize;
    let filled = filled.min(12);
    let empty = 12 - filled;
    let bar = format!("[{}{}]", "=".repeat(filled), "-".repeat(empty));
    if use_color { ansi("95", &bar) } else { bar }
}

fn reset_line(window: &crate::model::RateWindow) -> Option<String> {
    if let Some(resets_at) = window.resets_at {
        return Some(format!("Resets {}", reset_countdown_description(resets_at)));
    }
    if let Some(desc) = &window.reset_description {
        let trimmed = desc.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.to_lowercase().starts_with("resets") {
            return Some(trimmed.to_string());
        }
        return Some(format!("Resets {}", trimmed));
    }
    None
}

fn reset_countdown_description(resets_at: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let delta = resets_at.signed_duration_since(now);
    if delta.num_seconds() < 1 {
        return "now".to_string();
    }
    let total_minutes = (delta.num_seconds() as f64 / 60.0).ceil() as i64;
    let total_minutes = total_minutes.max(1);
    let days = total_minutes / (24 * 60);
    let hours = (total_minutes / 60) % 24;
    let minutes = total_minutes % 60;
    if days > 0 {
        if hours > 0 {
            return format!("in {}d {}h", days, hours);
        }
        return format!("in {}d", days);
    }
    if hours > 0 {
        if minutes > 0 {
            return format!("in {}h {}m", hours, minutes);
        }
        return format!("in {}h", hours);
    }
    format!("in {}m", minutes)
}

fn pace_line(provider: &str, window: &crate::model::RateWindow) -> Option<String> {
    if provider != "codex" && provider != "claude" {
        return None;
    }
    if remaining_percent(window.used_percent) <= 0.0 {
        return None;
    }
    let pace = usage_pace_weekly(window)?;
    if pace.expected_used_percent < 3.0 {
        return None;
    }
    let expected = pace.expected_used_percent.round() as i64;
    let mut parts = Vec::new();
    parts.push(pace_left_label(&pace));
    parts.push(format!("Expected {}% used", expected));
    if let Some(right) = pace_right_label(&pace) {
        parts.push(right);
    }
    Some(parts.join(" | "))
}

struct UsagePaceSummary {
    stage: UsagePaceStage,
    delta_percent: f64,
    expected_used_percent: f64,
    actual_used_percent: f64,
    eta_seconds: Option<i64>,
    will_last_to_reset: bool,
}

enum UsagePaceStage {
    OnTrack,
    SlightlyAhead,
    Ahead,
    FarAhead,
    SlightlyBehind,
    Behind,
    FarBehind,
}

fn usage_pace_weekly(window: &crate::model::RateWindow) -> Option<UsagePaceSummary> {
    let resets_at = window.resets_at?;
    let minutes = window.window_minutes.unwrap_or(10080);
    if minutes <= 0 {
        return None;
    }
    let now = chrono::Utc::now();
    let duration_secs = minutes * 60;
    let time_until_reset = (resets_at - now).num_seconds();
    if time_until_reset <= 0 || time_until_reset > duration_secs {
        return None;
    }
    let elapsed = (duration_secs - time_until_reset).clamp(0, duration_secs);
    let expected = ((elapsed as f64 / duration_secs as f64) * 100.0).clamp(0.0, 100.0);
    let actual = window.used_percent.clamp(0.0, 100.0);
    if elapsed == 0 && actual > 0.0 {
        return None;
    }
    let delta = actual - expected;
    let stage = usage_pace_stage(delta);

    let mut eta_seconds = None;
    let mut will_last_to_reset = false;
    if elapsed > 0 && actual > 0.0 {
        let rate = actual / elapsed as f64;
        if rate > 0.0 {
            let remaining = (100.0 - actual).max(0.0);
            let candidate = (remaining / rate).round() as i64;
            if candidate >= time_until_reset {
                will_last_to_reset = true;
            } else {
                eta_seconds = Some(candidate);
            }
        }
    } else if elapsed > 0 && actual == 0.0 {
        will_last_to_reset = true;
    }

    Some(UsagePaceSummary {
        stage,
        delta_percent: delta,
        expected_used_percent: expected,
        actual_used_percent: actual,
        eta_seconds,
        will_last_to_reset,
    })
}

fn usage_pace_stage(delta: f64) -> UsagePaceStage {
    let abs_delta = delta.abs();
    if abs_delta <= 2.0 {
        UsagePaceStage::OnTrack
    } else if abs_delta <= 6.0 {
        if delta >= 0.0 {
            UsagePaceStage::SlightlyAhead
        } else {
            UsagePaceStage::SlightlyBehind
        }
    } else if abs_delta <= 12.0 {
        if delta >= 0.0 {
            UsagePaceStage::Ahead
        } else {
            UsagePaceStage::Behind
        }
    } else if delta >= 0.0 {
        UsagePaceStage::FarAhead
    } else {
        UsagePaceStage::FarBehind
    }
}

fn pace_left_label(pace: &UsagePaceSummary) -> String {
    let delta = pace.delta_percent.abs().round() as i64;
    match pace.stage {
        UsagePaceStage::OnTrack => "On pace".to_string(),
        UsagePaceStage::SlightlyAhead | UsagePaceStage::Ahead | UsagePaceStage::FarAhead => {
            format!("{}% in deficit", delta)
        }
        UsagePaceStage::SlightlyBehind | UsagePaceStage::Behind | UsagePaceStage::FarBehind => {
            format!("{}% in reserve", delta)
        }
    }
}

fn pace_right_label(pace: &UsagePaceSummary) -> Option<String> {
    if pace.will_last_to_reset {
        return Some("Lasts until reset".to_string());
    }
    let eta = pace.eta_seconds?;
    let text = pace_duration_text(eta);
    if text == "now" {
        Some("Runs out now".to_string())
    } else {
        Some(format!("Runs out in {}", text))
    }
}

fn pace_duration_text(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 1 {
        return "now".to_string();
    }
    let minutes = ((seconds as f64) / 60.0).ceil() as i64;
    let minutes = minutes.max(1);
    let days = minutes / (24 * 60);
    let hours = (minutes / 60) % 24;
    let mins = minutes % 60;
    if days > 0 {
        if hours > 0 {
            return format!("{}d {}h", days, hours);
        }
        return format!("{}d", days);
    }
    if hours > 0 {
        if mins > 0 {
            return format!("{}h {}m", hours, mins);
        }
        return format!("{}h", hours);
    }
    format!("{}m", mins)
}

fn cost_line(cost: &crate::model::ProviderCostSnapshot) -> String {
    let mut parts = vec![format!(
        "Cost: {:.1} / {:.1} {}",
        cost.used, cost.limit, cost.currency_code
    )];
    if let Some(period) = &cost.period {
        parts.push(period.clone());
    }
    if let Some(resets_at) = cost.resets_at {
        parts.push(format!("Resets {}", reset_countdown_description(resets_at)));
    }
    parts.join(" | ")
}

fn label_line(label: &str, value: &str, use_color: bool) -> String {
    let label_text = if use_color {
        ansi("95", label)
    } else {
        label.to_string()
    };
    format!("{}: {}", label_text, value)
}

fn subtle_line(text: &str, use_color: bool) -> String {
    if use_color {
        ansi("90", text)
    } else {
        text.to_string()
    }
}

fn colorize_header(text: &str, use_color: bool) -> String {
    if use_color {
        ansi("1;95", text)
    } else {
        text.to_string()
    }
}

fn colorize_usage(text: &str, remaining_percent: f64, use_color: bool) -> String {
    if !use_color {
        return text.to_string();
    }
    let code = if remaining_percent < 10.0 {
        "31"
    } else if remaining_percent < 25.0 {
        "33"
    } else {
        "32"
    };
    ansi(code, text)
}

fn colorize_status(
    text: &str,
    indicator: crate::model::ProviderStatusIndicator,
    use_color: bool,
) -> String {
    if !use_color {
        return text.to_string();
    }
    let code = match indicator {
        crate::model::ProviderStatusIndicator::None => "32",
        crate::model::ProviderStatusIndicator::Minor => "33",
        crate::model::ProviderStatusIndicator::Major
        | crate::model::ProviderStatusIndicator::Critical => "31",
        crate::model::ProviderStatusIndicator::Maintenance => "34",
        crate::model::ProviderStatusIndicator::Unknown => "90",
    };
    ansi(code, text)
}

fn status_line(status: &crate::model::ProviderStatusPayload) -> String {
    let label = match status.indicator.clone() {
        crate::model::ProviderStatusIndicator::None => "Operational",
        crate::model::ProviderStatusIndicator::Minor => "Partial outage",
        crate::model::ProviderStatusIndicator::Major => "Major outage",
        crate::model::ProviderStatusIndicator::Critical => "Critical issue",
        crate::model::ProviderStatusIndicator::Maintenance => "Maintenance",
        crate::model::ProviderStatusIndicator::Unknown => "Status unknown",
    };
    let mut text = format!("Status: {}", label);
    if let Some(desc) = &status.description
        && !desc.trim().is_empty()
    {
        text.push_str(&format!(" - {}", desc));
    }
    text
}

fn format_credits(value: f64) -> String {
    let formatted = format!("{:.2}", value);
    format!("{} left", add_thousand_separators(&formatted))
}

fn add_thousand_separators(value: &str) -> String {
    let mut parts = value.splitn(2, '.');
    let int_part = parts.next().unwrap_or("");
    let frac_part = parts.next();
    let mut chars: Vec<char> = int_part.chars().collect();
    let mut out = String::new();
    let mut count = 0;
    while let Some(ch) = chars.pop() {
        if count == 3 {
            out.push(',');
            count = 0;
        }
        out.push(ch);
        count += 1;
    }
    let int_rev: String = out.chars().rev().collect();
    if let Some(frac) = frac_part {
        format!("{}.{}", int_rev, frac)
    } else {
        int_rev
    }
}

fn ansi(code: &str, text: &str) -> String {
    format!("\u{001B}[{}m{}\u{001B}[0m", code, text)
}

pub fn exit_code_for_error(err: &anyhow::Error) -> i32 {
    if let Some(cli_err) = err.downcast_ref::<CliError>() {
        return match cli_err {
            CliError::UnknownProvider(_) => 2,
            CliError::ProviderNotImplemented(_) => 2,
            CliError::ConfigMissing(_) | CliError::ConfigPathUnavailable => 3,
            CliError::ProviderNotConfigured(_) => 2,
            CliError::UnsupportedSource(_, _) => 3,
        };
    }
    if let Some(req_err) = err.downcast_ref::<reqwest::Error>()
        && req_err.is_timeout()
    {
        return 4;
    }
    if err.downcast_ref::<tokio::time::error::Elapsed>().is_some() {
        return 4;
    }
    if err.downcast_ref::<serde_json::Error>().is_some() {
        return 3;
    }
    1
}

pub fn error_kind_for_error(err: &anyhow::Error) -> ErrorKind {
    if let Some(cli_err) = err.downcast_ref::<CliError>() {
        return match cli_err {
            CliError::UnknownProvider(_) => ErrorKind::Args,
            CliError::ProviderNotImplemented(_) => ErrorKind::Provider,
            CliError::ConfigMissing(_) | CliError::ConfigPathUnavailable => ErrorKind::Config,
            CliError::ProviderNotConfigured(_) => ErrorKind::Provider,
            CliError::UnsupportedSource(_, _) => ErrorKind::Args,
        };
    }
    ErrorKind::Runtime
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProviderStatusIndicator, ProviderStatusPayload, RateWindow, UsageSnapshot};

    #[test]
    fn renders_codexbar_style_text() {
        let usage = UsageSnapshot {
            primary: Some(RateWindow {
                used_percent: 25.0,
                window_minutes: Some(300),
                resets_at: None,
                reset_description: Some("in 2h".to_string()),
            }),
            secondary: Some(RateWindow {
                used_percent: 40.0,
                window_minutes: Some(10080),
                resets_at: None,
                reset_description: Some("in 3d".to_string()),
            }),
            tertiary: None,
            provider_cost: None,
            updated_at: chrono::Utc::now(),
            identity: Some(crate::model::ProviderIdentitySnapshot {
                provider_id: Some("codex".to_string()),
                account_email: Some("user@example.com".to_string()),
                account_organization: None,
                login_method: Some("plus".to_string()),
            }),
            account_email: None,
            account_organization: None,
            login_method: None,
        };

        let payload = ProviderPayload {
            provider: "codex".to_string(),
            account: None,
            version: Some("2024-06-04".to_string()),
            source: "oauth".to_string(),
            status: Some(ProviderStatusPayload {
                indicator: ProviderStatusIndicator::None,
                description: Some("All systems operational".to_string()),
                updated_at: None,
                url: "https://status.openai.com".to_string(),
            }),
            usage: Some(usage),
            credits: Some(crate::model::CreditsSnapshot {
                remaining: 112.4,
                events: Vec::new(),
                updated_at: chrono::Utc::now(),
            }),
            antigravity_plan_info: None,
            openai_dashboard: None,
            error: None,
        };

        let prefs = OutputPreferences {
            format: OutputFormat::Text,
            pretty: false,
            json_only: false,
            no_color: true,
        };

        let text = format_payload_text(&payload, &prefs);
        let expected = [
            "== Codex 2024-06-04 (oauth) ==",
            "Session: 75% left [=========---]",
            "Resets in 2h",
            "Weekly: 60% left [=======-----]",
            "Resets in 3d",
            "Credits: 112.40 left",
            "Account: user@example.com",
            "Plan: plus",
            "Status: Operational - All systems operational",
        ]
        .join("\n");
        assert_eq!(text, expected);
    }
}
