use crate::cli::{CostArgs, UsageArgs};
use crate::config::{Config, ProviderConfig};
use crate::errors::CliError;
use crate::model::{ProviderPayload, UsageSnapshot};
use anyhow::Result;
use async_trait::async_trait;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

mod claude;
mod codex;
mod cursor;
mod factory;
mod gemini;
mod utils;
mod zai;
mod minimax;
mod kimi;
mod kimi_k2;
mod copilot;

pub use claude::ClaudeProvider;
pub use codex::CodexProvider;
pub use cursor::CursorProvider;
pub use factory::FactoryProvider;
pub use gemini::GeminiProvider;
pub use zai::ZaiProvider;
pub use minimax::MiniMaxProvider;
pub use kimi::KimiProvider;
pub use kimi_k2::KimiK2Provider;
pub use copilot::CopilotProvider;
pub(crate) use utils::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderId {
    Codex,
    Claude,
    Gemini,
    Cursor,
    #[serde(alias = "droid")]
    #[value(alias = "droid")]
    Factory,
    Zai,
    MiniMax,
    Kimi,
    #[serde(rename = "kimik2")]
    KimiK2,
    Copilot,
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            ProviderId::Codex => "codex",
            ProviderId::Claude => "claude",
            ProviderId::Gemini => "gemini",
            ProviderId::Cursor => "cursor",
            ProviderId::Factory => "factory",
            ProviderId::Zai => "zai",
            ProviderId::MiniMax => "minimax",
            ProviderId::Kimi => "kimi",
            ProviderId::KimiK2 => "kimik2",
            ProviderId::Copilot => "copilot",
        };
        write!(f, "{}", label)
    }
}

impl ProviderId {
    pub fn ordered() -> Vec<ProviderId> {
        vec![
            ProviderId::Codex,
            ProviderId::Claude,
            ProviderId::Gemini,
            ProviderId::Cursor,
            ProviderId::Factory,
            ProviderId::Zai,
            ProviderId::MiniMax,
            ProviderId::Kimi,
            ProviderId::KimiK2,
            ProviderId::Copilot,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub enum ProviderSelector {
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
    All,
    Both,
}

impl ProviderSelector {
    pub fn expand(self) -> Vec<ProviderId> {
        match self {
            ProviderSelector::All => ProviderId::ordered(),
            ProviderSelector::Both => vec![ProviderId::Codex, ProviderId::Claude],
            ProviderSelector::Codex => vec![ProviderId::Codex],
            ProviderSelector::Claude => vec![ProviderId::Claude],
            ProviderSelector::Gemini => vec![ProviderId::Gemini],
            ProviderSelector::Cursor => vec![ProviderId::Cursor],
            ProviderSelector::Factory => vec![ProviderId::Factory],
            ProviderSelector::Zai => vec![ProviderId::Zai],
            ProviderSelector::MiniMax => vec![ProviderId::MiniMax],
            ProviderSelector::Kimi => vec![ProviderId::Kimi],
            ProviderSelector::KimiK2 => vec![ProviderId::KimiK2],
            ProviderSelector::Copilot => vec![ProviderId::Copilot],
        }
    }
}

impl fmt::Display for ProviderSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            ProviderSelector::Codex => "codex",
            ProviderSelector::Claude => "claude",
            ProviderSelector::Gemini => "gemini",
            ProviderSelector::Cursor => "cursor",
            ProviderSelector::Factory => "factory",
            ProviderSelector::Zai => "zai",
            ProviderSelector::MiniMax => "minimax",
            ProviderSelector::Kimi => "kimi",
            ProviderSelector::KimiK2 => "kimik2",
            ProviderSelector::Copilot => "copilot",
            ProviderSelector::All => "all",
            ProviderSelector::Both => "both",
        };
        write!(f, "{}", label)
    }
}

pub fn expand_provider_selectors(selectors: &[ProviderSelector]) -> Vec<ProviderId> {
    let mut ordered = Vec::new();
    let mut seen: std::collections::HashSet<ProviderId> = std::collections::HashSet::new();
    for selector in selectors {
        for provider in selector.expand() {
            if seen.insert(provider) {
                ordered.push(provider);
            }
        }
    }
    ordered
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum SourcePreference {
    Auto,
    Oauth,
    Web,
    Cli,
    Api,
    Local,
}

impl fmt::Display for SourcePreference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            SourcePreference::Auto => "auto",
            SourcePreference::Oauth => "oauth",
            SourcePreference::Web => "web",
            SourcePreference::Cli => "cli",
            SourcePreference::Api => "api",
            SourcePreference::Local => "local",
        };
        write!(f, "{}", label)
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn version(&self) -> &'static str;
    fn supports_token_accounts(&self) -> bool {
        false
    }

    async fn fetch_usage(
        &self,
        args: &UsageArgs,
        config: &Config,
        source: SourcePreference,
    ) -> Result<ProviderPayload>;

    async fn fetch_usage_all(
        &self,
        args: &UsageArgs,
        config: &Config,
        source: SourcePreference,
    ) -> Result<Vec<ProviderPayload>> {
        Ok(vec![self.fetch_usage(args, config, source).await?])
    }

    async fn fetch_cost(&self, _args: &CostArgs, _config: &Config) -> Result<ProviderPayload> {
        Err(CliError::ProviderNotImplemented(self.id()).into())
    }

    fn resolve_source(
        &self,
        config: Option<ProviderConfig>,
        source: SourcePreference,
    ) -> SourcePreference {
        match source {
            SourcePreference::Auto => config
                .and_then(|cfg| cfg.source)
                .unwrap_or(SourcePreference::Auto),
            _ => source,
        }
    }

    fn ok_output(&self, source: &str, usage: Option<UsageSnapshot>) -> ProviderPayload {
        ProviderPayload {
            provider: self.id().to_string(),
            account: None,
            version: Some(self.version().to_string()),
            source: source.to_string(),
            status: None,
            usage,
            credits: None,
            antigravity_plan_info: None,
            openai_dashboard: None,
            error: None,
        }
    }
}

pub struct ProviderRegistry {
    providers: HashMap<ProviderId, Box<dyn Provider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        let mut providers: HashMap<ProviderId, Box<dyn Provider>> = HashMap::new();
        providers.insert(ProviderId::Codex, Box::new(CodexProvider));
        providers.insert(ProviderId::Claude, Box::new(ClaudeProvider));
        providers.insert(ProviderId::Gemini, Box::new(GeminiProvider));
        providers.insert(ProviderId::Cursor, Box::new(CursorProvider));
        providers.insert(ProviderId::Factory, Box::new(FactoryProvider));
        providers.insert(ProviderId::Zai, Box::new(ZaiProvider));
        providers.insert(ProviderId::MiniMax, Box::new(MiniMaxProvider));
        providers.insert(ProviderId::Kimi, Box::new(KimiProvider));
        providers.insert(ProviderId::KimiK2, Box::new(KimiK2Provider));
        providers.insert(ProviderId::Copilot, Box::new(CopilotProvider));
        Self { providers }
    }

    pub fn get(&self, id: &ProviderId) -> Option<&Box<dyn Provider>> {
        self.providers.get(id)
    }
}

pub async fn fetch_status_payload(
    base_url: &str,
    timeout_secs: u64,
) -> Option<crate::model::ProviderStatusPayload> {
    let api_url = format!("{}/api/v2/status.json", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .build()
        .ok()?;
    let resp = client.get(api_url).send().await.ok()?;
    let status = resp.status();
    if !status.is_success() {
        return Some(crate::model::ProviderStatusPayload {
            indicator: crate::model::ProviderStatusIndicator::Unknown,
            description: Some(format!("HTTP {}", status.as_u16())),
            updated_at: None,
            url: base_url.to_string(),
        });
    }
    let body = resp.bytes().await.ok()?;
    #[derive(Deserialize)]
    struct StatusResponse {
        status: StatusBlock,
        page: Option<PageBlock>,
    }
    #[derive(Deserialize)]
    struct StatusBlock {
        indicator: String,
        description: Option<String>,
    }
    #[derive(Deserialize)]
    struct PageBlock {
        #[serde(rename = "updated_at")]
        updated_at: Option<String>,
    }

    let parsed: StatusResponse = serde_json::from_slice(&body).ok()?;
    let indicator = match parsed.status.indicator.as_str() {
        "none" => crate::model::ProviderStatusIndicator::None,
        "minor" => crate::model::ProviderStatusIndicator::Minor,
        "major" => crate::model::ProviderStatusIndicator::Major,
        "critical" => crate::model::ProviderStatusIndicator::Critical,
        "maintenance" => crate::model::ProviderStatusIndicator::Maintenance,
        _ => crate::model::ProviderStatusIndicator::Unknown,
    };
    let updated_at = parsed
        .page
        .and_then(|p| p.updated_at)
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(&raw).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    Some(crate::model::ProviderStatusPayload {
        indicator,
        description: parsed.status.description,
        updated_at,
        url: base_url.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_provider_selectors_all() {
        let expanded = expand_provider_selectors(&[ProviderSelector::All]);
        assert_eq!(expanded, ProviderId::ordered());
    }

    #[test]
    fn expand_provider_selectors_both_plus_extra() {
        let expanded =
            expand_provider_selectors(&[ProviderSelector::Both, ProviderSelector::Gemini]);
        assert_eq!(
            expanded,
            vec![ProviderId::Codex, ProviderId::Claude, ProviderId::Gemini]
        );
    }

    #[test]
    fn expand_provider_selectors_dedup_preserves_order() {
        let expanded = expand_provider_selectors(&[
            ProviderSelector::Gemini,
            ProviderSelector::Both,
            ProviderSelector::Gemini,
        ]);
        assert_eq!(
            expanded,
            vec![ProviderId::Gemini, ProviderId::Codex, ProviderId::Claude]
        );
    }
}
