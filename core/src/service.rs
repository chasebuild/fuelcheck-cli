use crate::config::{Config, DetectResult, ProviderConfig};
use crate::errors::CliError;
use crate::model::{ErrorKind, ProviderErrorPayload, ProviderPayload};
use crate::providers::{
    ProviderId, ProviderRegistry, ProviderSelector, SourcePreference, expand_provider_selectors,
};
use crate::reports::{self, CostReportCollection, CostReportKind, CostReportRequest};
use anyhow::{Context, Result, anyhow};

#[derive(Debug, Clone)]
pub struct UsageRequest {
    pub providers: Vec<ProviderSelector>,
    pub source: SourcePreference,
    pub status: bool,
    pub no_credits: bool,
    pub refresh: bool,
    pub web_debug_dump_html: bool,
    pub web_timeout: u64,
    pub account: Option<String>,
    pub account_index: Option<usize>,
    pub all_accounts: bool,
    pub antigravity_plan_debug: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CostRequest {
    pub providers: Vec<ProviderSelector>,
}

#[derive(Debug, Clone, Default)]
pub struct SetupRequest {
    pub enable_all: bool,
    pub claude_cookie: Option<String>,
    pub cursor_cookie: Option<String>,
    pub factory_cookie: Option<String>,
}

pub async fn collect_usage_outputs(
    request: &UsageRequest,
    config: &Config,
    registry: &ProviderRegistry,
) -> Result<Vec<ProviderPayload>> {
    let provider_ids = if request.providers.is_empty() {
        config.enabled_providers_or_default()
    } else {
        expand_provider_selectors(&request.providers)
    };

    let wants_account_override =
        request.account.is_some() || request.account_index.is_some() || request.all_accounts;
    if wants_account_override && provider_ids.len() != 1 {
        return Err(anyhow!("account selection requires a single provider"));
    }
    if wants_account_override {
        let provider_id = provider_ids
            .first()
            .ok_or_else(|| anyhow!("no provider selected"))?;
        let provider = registry
            .get(provider_id)
            .ok_or_else(|| CliError::UnknownProvider(provider_id.to_string()))?;
        if !provider.supports_token_accounts() {
            return Err(anyhow!(
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
            .fetch_usage_all(request, config, request.source)
            .await
            .with_context(|| format!("provider {}", provider_id))
        {
            Ok(mut output_set) => outputs.append(&mut output_set),
            Err(err) => outputs.push(ProviderPayload::error(
                provider_id.to_string(),
                request.source.to_string(),
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

pub async fn collect_cost_outputs(
    request: &CostRequest,
    config: &Config,
    registry: &ProviderRegistry,
) -> Result<Vec<ProviderPayload>> {
    let provider_ids = if request.providers.is_empty() {
        config.enabled_providers_or_default()
    } else {
        expand_provider_selectors(&request.providers)
    };

    let mut outputs: Vec<ProviderPayload> = Vec::new();
    for provider_id in provider_ids {
        let provider = registry
            .get(&provider_id)
            .ok_or_else(|| CliError::UnknownProvider(provider_id.to_string()))?;
        match provider
            .fetch_cost(request, config)
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

    Ok(outputs)
}

pub fn collect_report_provider_ids(selectors: &[ProviderSelector]) -> Vec<ProviderId> {
    if selectors.is_empty() {
        return vec![ProviderId::Codex];
    }
    expand_provider_selectors(selectors)
}

pub fn build_cost_report_collection<'a>(
    report: CostReportKind,
    providers: Vec<ProviderId>,
    since: Option<&'a str>,
    until: Option<&'a str>,
    timezone: Option<&'a str>,
) -> Result<CostReportCollection> {
    reports::build_cost_report_collection(CostReportRequest {
        report,
        providers,
        since,
        until,
        timezone,
    })
}

pub fn build_setup_config(request: &SetupRequest, detected: &DetectResult) -> Config {
    let mut providers = Vec::new();

    let mut enable = |id: ProviderId, enabled: bool, mut extra: ProviderConfig| {
        extra.id = id;
        extra.enabled = Some(enabled);
        providers.push(extra);
    };

    let enable_all = request.enable_all;

    enable(
        ProviderId::Codex,
        enable_all || detected.codex_auth,
        ProviderConfig {
            id: ProviderId::Codex,
            enabled: Some(enable_all || detected.codex_auth),
            source: Some(SourcePreference::Oauth),
            ..ProviderConfig::default_provider(ProviderId::Codex)
        },
    );

    let mut claude_cfg = ProviderConfig {
        id: ProviderId::Claude,
        enabled: Some(enable_all || detected.claude_oauth || request.claude_cookie.is_some()),
        source: Some(SourcePreference::Oauth),
        ..ProviderConfig::default_provider(ProviderId::Claude)
    };
    if let Some(cookie) = request.claude_cookie.clone() {
        claude_cfg.cookie_header = Some(cookie);
        claude_cfg.source = Some(SourcePreference::Web);
    }
    enable(
        ProviderId::Claude,
        claude_cfg.enabled.unwrap_or(false),
        claude_cfg,
    );

    enable(
        ProviderId::Gemini,
        enable_all || detected.gemini_oauth,
        ProviderConfig {
            id: ProviderId::Gemini,
            enabled: Some(enable_all || detected.gemini_oauth),
            source: Some(SourcePreference::Api),
            ..ProviderConfig::default_provider(ProviderId::Gemini)
        },
    );

    let mut cursor_cfg = ProviderConfig {
        id: ProviderId::Cursor,
        enabled: Some(enable_all || request.cursor_cookie.is_some()),
        source: Some(SourcePreference::Web),
        ..ProviderConfig::default_provider(ProviderId::Cursor)
    };
    if let Some(cookie) = request.cursor_cookie.clone() {
        cursor_cfg.cookie_header = Some(cookie);
    }
    enable(
        ProviderId::Cursor,
        cursor_cfg.enabled.unwrap_or(false),
        cursor_cfg,
    );

    let mut factory_cfg = ProviderConfig {
        id: ProviderId::Factory,
        enabled: Some(enable_all || request.factory_cookie.is_some()),
        source: Some(SourcePreference::Web),
        ..ProviderConfig::default_provider(ProviderId::Factory)
    };
    if let Some(cookie) = request.factory_cookie.clone() {
        factory_cfg.cookie_header = Some(cookie);
    }
    enable(
        ProviderId::Factory,
        factory_cfg.enabled.unwrap_or(false),
        factory_cfg,
    );

    Config {
        version: Some(1),
        providers: Some(providers),
    }
}

pub fn format_error_chain(err: &anyhow::Error) -> String {
    let mut parts: Vec<String> = err.chain().map(|e| e.to_string()).collect();
    if parts.is_empty() {
        return "Unknown error".to_string();
    }
    parts.dedup();
    parts.join(": ")
}
