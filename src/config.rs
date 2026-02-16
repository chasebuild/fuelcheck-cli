use crate::errors::CliError;
use crate::providers::{ProviderId, SourcePreference};
use anyhow::{Context, Result};
use clap::Subcommand;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub version: Option<u32>,
    pub providers: Option<Vec<ProviderConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    pub id: ProviderId,
    pub enabled: Option<bool>,
    pub source: Option<SourcePreference>,
    pub cookie_source: Option<String>,
    pub cookie_header: Option<String>,
    pub api_key: Option<String>,
    pub region: Option<String>,
    pub workspace_id: Option<String>,
    pub token_accounts: Option<TokenAccounts>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenAccounts {
    pub version: Option<u32>,
    pub active_index: Option<usize>,
    pub accounts: Option<Vec<TokenAccount>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenAccount {
    pub id: Option<String>,
    pub label: Option<String>,
    pub token: Option<String>,
    pub added_at: Option<i64>,
    pub last_used: Option<i64>,
}

impl Config {
    pub fn load(path_override: Option<&PathBuf>) -> Result<Self> {
        let path = path_override
            .cloned()
            .or_else(default_config_path)
            .ok_or(CliError::ConfigPathUnavailable)?;

        if !path.exists() {
            return Ok(Config::default());
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("read config {}", path.display()))?;
        let config: Config = serde_json::from_str(&contents)
            .with_context(|| format!("parse config {}", path.display()))?;
        Ok(config)
    }

    pub fn path(path_override: Option<&PathBuf>) -> Result<PathBuf> {
        path_override
            .cloned()
            .or_else(default_config_path)
            .ok_or_else(|| CliError::ConfigPathUnavailable.into())
    }

    pub fn enabled_providers_or_default(&self) -> Vec<ProviderId> {
        let mut enabled: Vec<ProviderId> = self
            .providers
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|cfg| cfg.enabled.unwrap_or(true))
            .map(|cfg| cfg.id)
            .collect();

        if enabled.is_empty() {
            enabled = vec![
                ProviderId::Codex,
                ProviderId::Claude,
                ProviderId::Gemini,
                ProviderId::Cursor,
            ];
        }

        enabled
    }

    pub fn provider_config(&self, id: ProviderId) -> Option<ProviderConfig> {
        self.providers
            .clone()
            .unwrap_or_default()
            .into_iter()
            .find(|cfg| cfg.id == id)
    }
}

fn default_config_path() -> Option<PathBuf> {
    let home = BaseDirs::new()?.home_dir().to_path_buf();
    Some(home.join(".codexbar").join("config.json"))
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Validate config file shape.
    Validate(ConfigArgs),
    /// Dump normalized config file.
    Dump(ConfigArgs),
}

#[derive(clap::Parser, Debug, Clone)]
pub struct ConfigArgs {
    #[arg(long)]
    pub format: Option<crate::model::OutputFormat>,
    #[arg(long)]
    pub pretty: bool,
    #[arg(long)]
    pub config: Option<PathBuf>,
}

impl ConfigCommand {
    pub async fn execute(self) -> Result<()> {
        match self {
            ConfigCommand::Validate(args) => validate_config(args),
            ConfigCommand::Dump(args) => dump_config(args),
        }
    }
}

fn validate_config(args: ConfigArgs) -> Result<()> {
    let path = Config::path(args.config.as_ref())?;
    if !path.exists() {
        return Err(CliError::ConfigMissing(path).into());
    }

    let _config = Config::load(args.config.as_ref())?;
    match args.format.unwrap_or(crate::model::OutputFormat::Text) {
        crate::model::OutputFormat::Json => {
            let output = serde_json::json!({"status": "ok"});
            if args.pretty {
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("{}", serde_json::to_string(&output)?);
            }
        }
        crate::model::OutputFormat::Text => {
            println!("config ok: {}", path.display());
        }
    }

    Ok(())
}

fn dump_config(args: ConfigArgs) -> Result<()> {
    let config = Config::load(args.config.as_ref())?;
    let format = args.format.unwrap_or(crate::model::OutputFormat::Json);
    match format {
        crate::model::OutputFormat::Json => {
            if args.pretty {
                println!("{}", serde_json::to_string_pretty(&config)?);
            } else {
                println!("{}", serde_json::to_string(&config)?);
            }
        }
        crate::model::OutputFormat::Text => {
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
    }

    Ok(())
}
