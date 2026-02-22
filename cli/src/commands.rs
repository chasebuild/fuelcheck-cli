use anyhow::{Result, anyhow};
use fuelcheck_core::config::{Config, DetectResult};
use fuelcheck_core::model::{OutputFormat, ProviderErrorPayload, ProviderPayload};
use fuelcheck_core::providers::{ProviderRegistry, ProviderSelector};
use fuelcheck_core::service::{
    CostRequest, SetupRequest, UsageRequest, build_cost_report_collection, build_setup_config,
    collect_cost_outputs, collect_report_provider_ids, collect_usage_outputs,
};
use fuelcheck_ui::reports as ui_reports;
use fuelcheck_ui::text::{RenderOptions as TextRenderOptions, render_outputs};
use fuelcheck_ui::tui::{self, UsageArgs as WatchUsageArgs};

use crate::args::{
    ConfigArgs, ConfigCommand, ConfigCommandArgs, CostArgs, GlobalArgs, SetupArgs, UsageArgs,
};
use crate::logger::{self, LogLevel};

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

use std::io::IsTerminal;

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
            Some(
                serde_json::json!({ "path": path.display().to_string(), "missing": !path.exists() }),
            ),
        );
    }

    let format = if args.json || global.json_only {
        OutputFormat::Json
    } else {
        args.format.into()
    };

    if args.watch {
        if format == OutputFormat::Json || global.json_only {
            return Err(anyhow!("--watch only supports text output"));
        }

        let watch_args = WatchUsageArgs {
            providers: args.providers.into_iter().map(Into::into).collect(),
            source: args.source.into(),
            status: args.status,
            no_credits: args.no_credits,
            refresh: args.refresh,
            web_debug_dump_html: args.web_debug_dump_html,
            web_timeout: args.web_timeout,
            account: args.account,
            account_index: args.account_index,
            all_accounts: args.all_accounts,
            antigravity_plan_debug: args.antigravity_plan_debug,
            interval: args.interval,
        };
        return tui::run_usage_watch(watch_args, registry, config).await;
    }

    let request = UsageRequest {
        providers: args.providers.into_iter().map(Into::into).collect(),
        source: args.source.into(),
        status: args.status,
        no_credits: args.no_credits,
        refresh: args.refresh,
        web_debug_dump_html: args.web_debug_dump_html,
        web_timeout: args.web_timeout,
        account: args.account,
        account_index: args.account_index,
        all_accounts: args.all_accounts,
        antigravity_plan_debug: args.antigravity_plan_debug,
    };

    let outputs = collect_usage_outputs(&request, &config, registry).await?;
    let prefs = OutputPreferences {
        format,
        pretty: args.pretty,
        json_only: global.json_only,
        no_color: global.no_color,
    };
    print_outputs(&outputs, &prefs)
}

pub async fn run_cost(
    args: CostArgs,
    registry: &ProviderRegistry,
    global: &GlobalArgs,
) -> Result<()> {
    let config = Config::load(args.config.as_ref())?;

    let format = if args.json || global.json_only {
        OutputFormat::Json
    } else {
        args.format.into()
    };

    if let Some(report_kind) = args.report {
        let providers = collect_report_provider_ids(
            &args
                .providers
                .iter()
                .copied()
                .map(Into::into)
                .collect::<Vec<ProviderSelector>>(),
        );
        let report_collection = build_cost_report_collection(
            report_kind.into(),
            providers,
            args.since.as_deref(),
            args.until.as_deref(),
            args.timezone.as_deref(),
        )?;

        if format == OutputFormat::Json || global.json_only {
            let value = fuelcheck_core::reports::collection_to_json_value(&report_collection)?;
            if args.pretty {
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                println!("{}", serde_json::to_string(&value)?);
            }
            return Ok(());
        }

        if !global.json_only {
            println!(
                "{}",
                ui_reports::render_collection_text(
                    &report_collection,
                    args.compact,
                    args.timezone.as_deref()
                )
            );
        }
        return Ok(());
    }

    let request = CostRequest {
        providers: args.providers.into_iter().map(Into::into).collect(),
    };
    let outputs = collect_cost_outputs(&request, &config, registry).await?;

    let prefs = OutputPreferences {
        format,
        pretty: args.pretty,
        json_only: global.json_only,
        no_color: global.no_color,
    };
    print_outputs(&outputs, &prefs)
}

pub async fn run_config(cmd: ConfigCommandArgs, global: &GlobalArgs) -> Result<()> {
    let mut command = cmd.command;
    if global.json_only {
        match &mut command {
            ConfigCommand::Validate(args) => args.format = Some(crate::args::OutputFormatArg::Json),
            ConfigCommand::Dump(args) => args.format = Some(crate::args::OutputFormatArg::Json),
        }
    }

    match command {
        ConfigCommand::Validate(args) => validate_config(args),
        ConfigCommand::Dump(args) => dump_config(args),
    }
}

pub async fn run_setup(args: SetupArgs) -> Result<()> {
    let config_path = Config::path(args.config.as_ref())?;
    if config_path.exists() && !args.force {
        return Err(anyhow!(
            "Config already exists at {}. Use --force to overwrite.",
            config_path.display()
        ));
    }

    let detected = DetectResult::detect();
    let config = build_setup_config(
        &SetupRequest {
            enable_all: args.enable_all,
            claude_cookie: args.claude_cookie.clone(),
            cursor_cookie: args.cursor_cookie.clone(),
            factory_cookie: args.factory_cookie.clone(),
        },
        &detected,
    );
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

fn validate_config(args: ConfigArgs) -> Result<()> {
    let path = Config::path(args.config.as_ref())?;
    let missing = !path.exists();
    let _config = Config::load(args.config.as_ref())?;
    match args.format.map(Into::into).unwrap_or(OutputFormat::Text) {
        OutputFormat::Json => {
            let output = if missing {
                serde_json::json!({
                    "status": "ok",
                    "missing": true,
                    "path": path.display().to_string()
                })
            } else {
                serde_json::json!({"status": "ok"})
            };
            if args.pretty {
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("{}", serde_json::to_string(&output)?);
            }
        }
        OutputFormat::Text => {
            if missing {
                println!("config ok (missing; using defaults): {}", path.display());
            } else {
                println!("config ok: {}", path.display());
            }
        }
    }

    Ok(())
}

fn dump_config(args: ConfigArgs) -> Result<()> {
    let config = Config::load(args.config.as_ref())?;
    match args.format.map(Into::into).unwrap_or(OutputFormat::Json) {
        OutputFormat::Json => {
            if args.pretty {
                println!("{}", serde_json::to_string_pretty(&config)?);
            } else {
                println!("{}", serde_json::to_string(&config)?);
            }
        }
        OutputFormat::Text => {
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
    }

    Ok(())
}

fn print_outputs(outputs: &[ProviderPayload], prefs: &OutputPreferences) -> Result<()> {
    let rendered = render_outputs(
        outputs,
        &TextRenderOptions {
            format: prefs.format,
            pretty: prefs.pretty,
            json_only: prefs.json_only,
            use_color: prefs.use_color(),
        },
    )?;

    if let Some(text) = rendered {
        println!("{}", text);
    }

    Ok(())
}

pub fn cli_error_payload(
    code: i32,
    message: String,
    kind: fuelcheck_core::model::ErrorKind,
) -> ProviderPayload {
    ProviderPayload::error(
        "cli".to_string(),
        "cli".to_string(),
        ProviderErrorPayload {
            code,
            message,
            kind: Some(kind),
        },
    )
}
