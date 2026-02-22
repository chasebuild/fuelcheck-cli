use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use fuelcheck_core::model::OutputFormat;
use fuelcheck_core::providers::{ProviderSelector, SourcePreference};
use fuelcheck_core::reports::CostReportKind;

use crate::logger::LogLevel;

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
    #[arg(long, global = true)]
    pub no_color: bool,
    #[arg(long, global = true)]
    pub log_level: Option<LogLevel>,
    #[arg(long, global = true)]
    pub json_output: bool,
    #[arg(long, global = true)]
    pub json_only: bool,
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Usage(UsageArgs),
    Cost(CostArgs),
    Config(ConfigCommandArgs),
    Setup(SetupArgs),
}

#[derive(Parser, Debug, Clone)]
pub struct UsageArgs {
    #[arg(short, long = "provider")]
    pub providers: Vec<ProviderSelectorArg>,
    #[arg(long, default_value = "auto")]
    pub source: SourcePreferenceArg,
    #[arg(long, default_value = "text")]
    pub format: OutputFormatArg,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub pretty: bool,
    #[arg(long)]
    pub status: bool,
    #[arg(long)]
    pub no_credits: bool,
    #[arg(long)]
    pub refresh: bool,
    #[arg(long)]
    pub web_debug_dump_html: bool,
    #[arg(long, default_value = "20")]
    pub web_timeout: u64,
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub account: Option<String>,
    #[arg(long)]
    pub account_index: Option<usize>,
    #[arg(long)]
    pub all_accounts: bool,
    #[arg(long)]
    pub antigravity_plan_debug: bool,
    #[arg(long)]
    pub watch: bool,
    #[arg(long, default_value = "10")]
    pub interval: u64,
}

#[derive(Parser, Debug, Clone)]
pub struct CostArgs {
    #[arg(short, long = "provider")]
    pub providers: Vec<ProviderSelectorArg>,
    #[arg(long, default_value = "text")]
    pub format: OutputFormatArg,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub pretty: bool,
    #[arg(long)]
    pub report: Option<CostReportKindArg>,
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long)]
    pub until: Option<String>,
    #[arg(long)]
    pub timezone: Option<String>,
    #[arg(long)]
    pub compact: bool,
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Parser, Debug, Clone)]
pub struct SetupArgs {
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub enable_all: bool,
    #[arg(long)]
    pub claude_cookie: Option<String>,
    #[arg(long)]
    pub cursor_cookie: Option<String>,
    #[arg(long, alias = "droid-cookie")]
    pub factory_cookie: Option<String>,
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Parser, Debug)]
pub struct ConfigCommandArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    Validate(ConfigArgs),
    Dump(ConfigArgs),
}

#[derive(Parser, Debug, Clone)]
pub struct ConfigArgs {
    #[arg(long)]
    pub format: Option<OutputFormatArg>,
    #[arg(long)]
    pub pretty: bool,
    #[arg(long)]
    pub config: Option<PathBuf>,
}

impl ConfigCommand {
    pub fn format(&self) -> OutputFormat {
        match self {
            Self::Validate(args) => args.format.map(Into::into).unwrap_or(OutputFormat::Text),
            Self::Dump(args) => args.format.map(Into::into).unwrap_or(OutputFormat::Json),
        }
    }

    pub fn pretty(&self) -> bool {
        match self {
            Self::Validate(args) | Self::Dump(args) => args.pretty,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormatArg {
    Text,
    Json,
}

impl From<OutputFormatArg> for OutputFormat {
    fn from(value: OutputFormatArg) -> Self {
        match value {
            OutputFormatArg::Text => OutputFormat::Text,
            OutputFormatArg::Json => OutputFormat::Json,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SourcePreferenceArg {
    Auto,
    Oauth,
    Web,
    Cli,
    Api,
    Local,
}

impl From<SourcePreferenceArg> for SourcePreference {
    fn from(value: SourcePreferenceArg) -> Self {
        match value {
            SourcePreferenceArg::Auto => SourcePreference::Auto,
            SourcePreferenceArg::Oauth => SourcePreference::Oauth,
            SourcePreferenceArg::Web => SourcePreference::Web,
            SourcePreferenceArg::Cli => SourcePreference::Cli,
            SourcePreferenceArg::Api => SourcePreference::Api,
            SourcePreferenceArg::Local => SourcePreference::Local,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub enum ProviderSelectorArg {
    Codex,
    Claude,
    Gemini,
    Cursor,
    #[value(alias = "droid")]
    Factory,
    Zai,
    MiniMax,
    Kimi,
    #[value(alias = "kimik2")]
    KimiK2,
    Copilot,
    Kiro,
    VertexAI,
    JetBrains,
    Amp,
    Warp,
    OpenCode,
    All,
    Both,
}

impl From<ProviderSelectorArg> for ProviderSelector {
    fn from(value: ProviderSelectorArg) -> Self {
        match value {
            ProviderSelectorArg::Codex => ProviderSelector::Codex,
            ProviderSelectorArg::Claude => ProviderSelector::Claude,
            ProviderSelectorArg::Gemini => ProviderSelector::Gemini,
            ProviderSelectorArg::Cursor => ProviderSelector::Cursor,
            ProviderSelectorArg::Factory => ProviderSelector::Factory,
            ProviderSelectorArg::Zai => ProviderSelector::Zai,
            ProviderSelectorArg::MiniMax => ProviderSelector::MiniMax,
            ProviderSelectorArg::Kimi => ProviderSelector::Kimi,
            ProviderSelectorArg::KimiK2 => ProviderSelector::KimiK2,
            ProviderSelectorArg::Copilot => ProviderSelector::Copilot,
            ProviderSelectorArg::Kiro => ProviderSelector::Kiro,
            ProviderSelectorArg::VertexAI => ProviderSelector::VertexAI,
            ProviderSelectorArg::JetBrains => ProviderSelector::JetBrains,
            ProviderSelectorArg::Amp => ProviderSelector::Amp,
            ProviderSelectorArg::Warp => ProviderSelector::Warp,
            ProviderSelectorArg::OpenCode => ProviderSelector::OpenCode,
            ProviderSelectorArg::All => ProviderSelector::All,
            ProviderSelectorArg::Both => ProviderSelector::Both,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CostReportKindArg {
    Daily,
    Monthly,
    Session,
}

impl From<CostReportKindArg> for CostReportKind {
    fn from(value: CostReportKindArg) -> Self {
        match value {
            CostReportKindArg::Daily => CostReportKind::Daily,
            CostReportKindArg::Monthly => CostReportKind::Monthly,
            CostReportKindArg::Session => CostReportKind::Session,
        }
    }
}
