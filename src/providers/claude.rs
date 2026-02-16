use crate::accounts::{AccountSelectionArgs, account_label, select_accounts};
use crate::cli::UsageArgs;
use crate::config::{Config, TokenAccount};
use crate::errors::CliError;
use crate::model::{
    ProviderCostSnapshot, ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot,
};
use crate::providers::{Provider, ProviderId, SourcePreference, fetch_status_payload};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use directories::BaseDirs;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub struct ClaudeProvider;

#[async_trait]
impl Provider for ClaudeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }

    fn version(&self) -> &'static str {
        "2025-04-20"
    }

    fn supports_token_accounts(&self) -> bool {
        true
    }

    async fn fetch_usage_all(
        &self,
        args: &UsageArgs,
        config: &Config,
        source: SourcePreference,
    ) -> Result<Vec<ProviderPayload>> {
        let cfg = config.provider_config(self.id());
        let selection = AccountSelectionArgs {
            account: args.account.clone(),
            account_index: args.account_index.map(|idx| idx.saturating_sub(1)),
            all_accounts: args.all_accounts,
        };
        let selected = select_accounts(
            cfg.as_ref().and_then(|c| c.token_accounts.as_ref()),
            &selection,
        )?;
        let Some(selected) = selected else {
            return Ok(vec![self.fetch_usage(args, config, source).await?]);
        };

        let effective = self.resolve_source(cfg, source);
        let selected_source = match effective {
            SourcePreference::Auto | SourcePreference::Oauth => SourcePreference::Oauth,
            other => other,
        };
        if selected_source != SourcePreference::Oauth {
            return Err(CliError::UnsupportedSource(self.id(), selected_source.to_string()).into());
        }

        let status = if args.status {
            fetch_status_payload("https://status.claude.com", args.web_timeout).await
        } else {
            None
        };

        let mut outputs = Vec::new();
        for account in selected {
            let creds =
                ClaudeOAuthCredentials::from_token_account(&account.account, account.index)?;
            let usage = fetch_claude_oauth_usage_with_creds(&creds).await?;
            let mut payload = self.ok_output("oauth", Some(usage));
            payload.status = status.clone();
            payload.account = Some(account_label(&account.account, account.index));
            outputs.push(payload);
        }

        Ok(outputs)
    }

    async fn fetch_usage(
        &self,
        args: &UsageArgs,
        config: &Config,
        source: SourcePreference,
    ) -> Result<ProviderPayload> {
        let cfg = config.provider_config(self.id());
        let cookie_header = cfg
            .as_ref()
            .and_then(|c| c.cookie_header.clone())
            .or_else(|| std::env::var("CLAUDE_COOKIE").ok());
        let has_cookie = cookie_header
            .as_ref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        let effective = self.resolve_source(cfg, source);
        let selected = match effective {
            SourcePreference::Auto => {
                if claude_credentials_file_exists() {
                    SourcePreference::Oauth
                } else if has_cookie {
                    SourcePreference::Web
                } else {
                    SourcePreference::Oauth
                }
            }
            other => other,
        };

        let status = if args.status {
            fetch_status_payload("https://status.claude.com", args.web_timeout).await
        } else {
            None
        };

        match selected {
            SourcePreference::Oauth => {
                let usage = fetch_claude_oauth_usage().await?;
                let mut payload = self.ok_output("oauth", Some(usage));
                payload.status = status;
                Ok(payload)
            }
            SourcePreference::Cli => Err(anyhow!(
                "Claude CLI source not implemented in this build. Use --source oauth or --source web."
            )),
            SourcePreference::Web => {
                let header = cookie_header
                    .ok_or_else(|| {
                        anyhow!("Claude cookie header missing. Set provider cookie_header in config or CLAUDE_COOKIE.")
                    })?;
                let usage = fetch_claude_web_usage(&header).await?;
                let mut payload = self.ok_output("web", Some(usage));
                payload.status = status;
                Ok(payload)
            }
            SourcePreference::Api => {
                Err(CliError::UnsupportedSource(self.id(), "api".into()).into())
            }
            SourcePreference::Local => {
                Err(CliError::UnsupportedSource(self.id(), "local".into()).into())
            }
            SourcePreference::Auto => {
                Err(CliError::UnsupportedSource(self.id(), "auto".into()).into())
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct ClaudeCredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<ClaudeOAuthRoot>,
}

#[derive(Debug, Deserialize)]
struct ClaudeOAuthRoot {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "refreshToken")]
    refresh_token: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<f64>,
    scopes: Option<Vec<String>>,
    #[serde(rename = "rateLimitTier")]
    rate_limit_tier: Option<String>,
}

#[derive(Debug, Clone)]
struct ClaudeOAuthCredentials {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<DateTime<Utc>>,
    scopes: Vec<String>,
    rate_limit_tier: Option<String>,
}

impl ClaudeOAuthCredentials {
    fn load() -> Result<Self> {
        match fs::read(claude_credentials_path()) {
            Ok(data) => return Self::parse(data),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
        if let Ok(data) = load_claude_keychain_credentials() {
            return Self::parse(data);
        }
        Err(anyhow!("Claude OAuth credentials not found"))
    }

    fn from_token_account(account: &TokenAccount, index: usize) -> Result<Self> {
        let token = account
            .token
            .clone()
            .filter(|val| !val.trim().is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "Claude token account {} missing token",
                    account_label(account, index)
                )
            })?;
        Ok(Self {
            access_token: token,
            refresh_token: None,
            expires_at: None,
            scopes: Vec::new(),
            rate_limit_tier: None,
        })
    }

    fn is_expired(&self) -> bool {
        self.expires_at.map(|dt| dt <= Utc::now()).unwrap_or(true)
    }

    fn parse(data: Vec<u8>) -> Result<Self> {
        let root: ClaudeCredentialsFile = serde_json::from_slice(&data)?;
        let oauth = root
            .claude_ai_oauth
            .ok_or_else(|| anyhow!("Claude OAuth missing"))?;
        let access_token = oauth
            .access_token
            .ok_or_else(|| anyhow!("Claude OAuth missing access token"))?;
        let expires_at = oauth
            .expires_at
            .map(|ms| DateTime::<Utc>::from_timestamp((ms / 1000.0) as i64, 0))
            .flatten();
        Ok(Self {
            access_token,
            refresh_token: oauth.refresh_token,
            expires_at,
            scopes: oauth.scopes.unwrap_or_default(),
            rate_limit_tier: oauth.rate_limit_tier,
        })
    }
}

fn claude_credentials_path() -> PathBuf {
    let home = BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".claude").join(".credentials.json")
}

fn claude_credentials_file_exists() -> bool {
    claude_credentials_path().exists()
}

fn load_claude_keychain_credentials() -> Result<Vec<u8>> {
    if !cfg!(target_os = "macos") {
        return Err(anyhow!(
            "Claude OAuth keychain read is only supported on macOS"
        ));
    }
    let output = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()?;
    if !output.status.success() {
        return Err(anyhow!("Claude OAuth keychain entry not found"));
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Claude OAuth keychain entry empty"));
    }
    Ok(trimmed.as_bytes().to_vec())
}

#[derive(Debug, Deserialize)]
struct OAuthUsageResponse {
    #[serde(rename = "five_hour")]
    five_hour: Option<OAuthUsageWindow>,
    #[serde(rename = "seven_day")]
    seven_day: Option<OAuthUsageWindow>,
    #[serde(rename = "seven_day_opus")]
    seven_day_opus: Option<OAuthUsageWindow>,
    #[serde(rename = "seven_day_sonnet")]
    seven_day_sonnet: Option<OAuthUsageWindow>,
    #[serde(rename = "extra_usage")]
    extra_usage: Option<OAuthExtraUsage>,
}

#[derive(Debug, Deserialize)]
struct OAuthUsageWindow {
    utilization: Option<f64>,
    #[serde(rename = "resets_at")]
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthExtraUsage {
    #[serde(rename = "is_enabled")]
    is_enabled: Option<bool>,
    #[serde(rename = "monthly_limit")]
    monthly_limit: Option<f64>,
    #[serde(rename = "used_credits")]
    used_credits: Option<f64>,
    #[allow(dead_code)]
    utilization: Option<f64>,
    currency: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct WebOrganizationResponse {
    uuid: String,
    name: Option<String>,
    capabilities: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct WebUsageResponse {
    #[serde(rename = "five_hour")]
    five_hour: Option<WebUsageWindow>,
    #[serde(rename = "seven_day")]
    seven_day: Option<WebUsageWindow>,
    #[serde(rename = "seven_day_opus")]
    seven_day_opus: Option<WebUsageWindow>,
    #[serde(rename = "seven_day_sonnet")]
    seven_day_sonnet: Option<WebUsageWindow>,
}

#[derive(Debug, Deserialize)]
struct WebUsageWindow {
    utilization: Option<f64>,
    #[serde(rename = "resets_at")]
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WebOverageSpendLimitResponse {
    #[serde(rename = "monthly_credit_limit")]
    monthly_credit_limit: Option<f64>,
    currency: Option<String>,
    #[serde(rename = "used_credits")]
    used_credits: Option<f64>,
    #[serde(rename = "is_enabled")]
    is_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct WebAccountResponse {
    #[serde(rename = "email_address")]
    email_address: Option<String>,
    memberships: Option<Vec<WebAccountMembership>>,
}

#[derive(Debug, Deserialize)]
struct WebAccountMembership {
    organization: WebAccountOrganization,
}

#[derive(Debug, Deserialize)]
struct WebAccountOrganization {
    uuid: Option<String>,
    name: Option<String>,
    #[serde(rename = "rate_limit_tier")]
    rate_limit_tier: Option<String>,
    #[serde(rename = "billing_type")]
    billing_type: Option<String>,
}

async fn fetch_claude_oauth_usage() -> Result<UsageSnapshot> {
    let mut creds = ClaudeOAuthCredentials::load()?;
    if creds.is_expired() {
        if let Some(refresh_token) = creds.refresh_token.clone() {
            if let Ok(updated) =
                refresh_claude_token(&refresh_token, &creds.scopes, creds.rate_limit_tier.clone())
                    .await
            {
                creds = updated;
            }
        }
    }
    fetch_claude_oauth_usage_with_creds(&creds).await
}

async fn fetch_claude_oauth_usage_with_creds(
    creds: &ClaudeOAuthCredentials,
) -> Result<UsageSnapshot> {
    let usage = claude_oauth_fetch(&creds.access_token).await?;
    map_claude_usage(&usage, creds)
}

async fn refresh_claude_token(
    refresh_token: &str,
    scopes: &[String],
    rate_limit_tier: Option<String>,
) -> Result<ClaudeOAuthCredentials> {
    let client_id = std::env::var("CODEXBAR_CLAUDE_OAUTH_CLIENT_ID")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "9d1c250a-e61b-44d9-88ed-5944d1962f5e".to_string());
    let url = "https://platform.claude.com/v1/oauth/token";
    let body = format!(
        "grant_type=refresh_token&refresh_token={}&client_id={}",
        refresh_token, client_id
    );

    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(body)
        .send()
        .await?;
    let status = resp.status();
    let data = resp.bytes().await?;
    if !status.is_success() {
        return Err(anyhow!(
            "Claude OAuth refresh failed (HTTP {})",
            status.as_u16()
        ));
    }
    let json: serde_json::Value = serde_json::from_slice(&data)?;
    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Claude OAuth refresh response missing access_token"))?;
    let expires_in = json.get("expires_in").and_then(|v| v.as_i64()).unwrap_or(0);
    let expires_at = if expires_in > 0 {
        Some(Utc::now() + chrono::Duration::seconds(expires_in))
    } else {
        None
    };
    let new_refresh = json
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or(refresh_token)
        .to_string();

    Ok(ClaudeOAuthCredentials {
        access_token: access_token.to_string(),
        refresh_token: Some(new_refresh),
        expires_at,
        scopes: scopes.to_vec(),
        rate_limit_tier,
    })
}

async fn claude_oauth_fetch(access_token: &str) -> Result<OAuthUsageResponse> {
    let url = "https://api.anthropic.com/api/oauth/usage";
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "FuelcheckCLI")
        .send()
        .await?;
    let status = resp.status();
    let data = resp.bytes().await?;
    if status.as_u16() == 401 {
        return Err(anyhow!(
            "Claude OAuth unauthorized. Run `claude` to re-authenticate."
        ));
    }
    if !status.is_success() {
        return Err(anyhow!(
            "Claude OAuth usage fetch failed (HTTP {})",
            status.as_u16()
        ));
    }
    let usage: OAuthUsageResponse = serde_json::from_slice(&data)?;
    Ok(usage)
}

fn map_claude_usage(
    usage: &OAuthUsageResponse,
    creds: &ClaudeOAuthCredentials,
) -> Result<UsageSnapshot> {
    let primary = make_window(usage.five_hour.as_ref(), 5 * 60)
        .ok_or_else(|| anyhow!("missing session data"))?;
    let weekly = make_window(usage.seven_day.as_ref(), 7 * 24 * 60);
    let model_specific = make_window(
        usage
            .seven_day_sonnet
            .as_ref()
            .or(usage.seven_day_opus.as_ref()),
        7 * 24 * 60,
    );

    let login_method = infer_plan(creds.rate_limit_tier.as_deref());
    let provider_cost = oauth_extra_usage_cost(usage.extra_usage.as_ref(), login_method.as_deref());

    let identity = ProviderIdentitySnapshot {
        provider_id: Some("claude".to_string()),
        account_email: None,
        account_organization: None,
        login_method: login_method.clone(),
    };

    Ok(UsageSnapshot {
        primary: Some(primary),
        secondary: weekly,
        tertiary: model_specific,
        provider_cost,
        updated_at: Utc::now(),
        account_email: identity.account_email.clone(),
        account_organization: identity.account_organization.clone(),
        login_method: identity.login_method.clone(),
        identity: Some(identity),
    })
}

fn make_window(window: Option<&OAuthUsageWindow>, minutes: i64) -> Option<RateWindow> {
    let window = window?;
    let utilization = window.utilization?;
    let resets_at = window
        .resets_at
        .as_ref()
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|dt| dt.with_timezone(&Utc));
    let reset_description = resets_at.map(format_reset_description);
    Some(RateWindow {
        used_percent: utilization,
        window_minutes: Some(minutes),
        resets_at,
        reset_description,
    })
}

fn oauth_extra_usage_cost(
    extra: Option<&OAuthExtraUsage>,
    login_method: Option<&str>,
) -> Option<ProviderCostSnapshot> {
    let extra = extra?;
    if extra.is_enabled != Some(true) {
        return None;
    }
    let used = extra.used_credits?;
    let limit = extra.monthly_limit?;
    let currency = extra.currency.clone().unwrap_or_else(|| "USD".to_string());
    let used_norm = used / 100.0;
    let limit_norm = limit / 100.0;
    let mut cost = ProviderCostSnapshot {
        used: used_norm,
        limit: limit_norm,
        currency_code: currency,
        period: Some("Monthly".to_string()),
        resets_at: None,
        updated_at: Utc::now(),
    };
    if let Some(plan) = login_method {
        if !plan.to_lowercase().contains("enterprise") && cost.limit >= 1000.0 {
            cost.used /= 100.0;
            cost.limit /= 100.0;
        }
    }
    Some(cost)
}

async fn fetch_claude_web_usage(cookie_header: &str) -> Result<UsageSnapshot> {
    let cookie_header = normalize_claude_cookie_header(cookie_header);
    let org = claude_web_fetch_org(&cookie_header).await?;
    let usage = claude_web_fetch_usage(&org.uuid, &cookie_header).await?;
    let extra = claude_web_fetch_overage(&org.uuid, &cookie_header)
        .await
        .ok()
        .flatten();
    let account = claude_web_fetch_account(&cookie_header, Some(&org.uuid))
        .await
        .ok()
        .flatten();

    let primary = make_web_window(usage.five_hour.as_ref(), 5 * 60)
        .ok_or_else(|| anyhow!("missing session data"))?;
    let weekly = make_web_window(usage.seven_day.as_ref(), 7 * 24 * 60);
    let model_specific = make_web_window(
        usage
            .seven_day_sonnet
            .as_ref()
            .or(usage.seven_day_opus.as_ref()),
        7 * 24 * 60,
    );

    let account_org = sanitize_label(org.name.clone())
        .or_else(|| account.as_ref().and_then(|info| info.organization.clone()));
    let login_method = account.as_ref().and_then(|info| info.login_method.clone());

    let identity = ProviderIdentitySnapshot {
        provider_id: Some("claude".to_string()),
        account_email: account.as_ref().and_then(|info| info.email.clone()),
        account_organization: account_org.clone(),
        login_method: login_method.clone(),
    };

    Ok(UsageSnapshot {
        primary: Some(primary),
        secondary: weekly,
        tertiary: model_specific,
        provider_cost: extra,
        updated_at: Utc::now(),
        account_email: identity.account_email.clone(),
        account_organization: identity.account_organization.clone(),
        login_method: identity.login_method.clone(),
        identity: Some(identity),
    })
}

async fn claude_web_fetch_org(cookie_header: &str) -> Result<WebOrganizationResponse> {
    let url = "https://claude.ai/api/organizations";
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("Cookie", cookie_header)
        .header("Accept", "application/json")
        .header("User-Agent", "FuelcheckCLI")
        .send()
        .await?;
    let status = resp.status();
    let data = resp.bytes().await?;
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(anyhow!("Claude web unauthorized. Cookie may be expired."));
    }
    if !status.is_success() {
        return Err(anyhow!(
            "Claude web organizations fetch failed (HTTP {})",
            status.as_u16()
        ));
    }
    let orgs: Vec<WebOrganizationResponse> = serde_json::from_slice(&data)?;
    let selected =
        select_claude_org(&orgs).ok_or_else(|| anyhow!("Claude web organization missing"))?;
    Ok(selected)
}

async fn claude_web_fetch_usage(org_id: &str, cookie_header: &str) -> Result<WebUsageResponse> {
    let url = format!("https://claude.ai/api/organizations/{}/usage", org_id);
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("Cookie", cookie_header)
        .header("Accept", "application/json")
        .header("User-Agent", "FuelcheckCLI")
        .send()
        .await?;
    let status = resp.status();
    let data = resp.bytes().await?;
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(anyhow!("Claude web unauthorized. Cookie may be expired."));
    }
    if !status.is_success() {
        return Err(anyhow!(
            "Claude web usage fetch failed (HTTP {})",
            status.as_u16()
        ));
    }
    let usage: WebUsageResponse = serde_json::from_slice(&data)?;
    Ok(usage)
}

async fn claude_web_fetch_overage(
    org_id: &str,
    cookie_header: &str,
) -> Result<Option<ProviderCostSnapshot>> {
    let url = format!(
        "https://claude.ai/api/organizations/{}/overage_spend_limit",
        org_id
    );
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("Cookie", cookie_header)
        .header("Accept", "application/json")
        .header("User-Agent", "FuelcheckCLI")
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        return Ok(None);
    }
    let data = resp.bytes().await?;
    let decoded: WebOverageSpendLimitResponse = serde_json::from_slice(&data)?;
    if decoded.is_enabled != Some(true) {
        return Ok(None);
    }
    let used = decoded.used_credits.unwrap_or(0.0) / 100.0;
    let limit = decoded.monthly_credit_limit.unwrap_or(0.0) / 100.0;
    let currency = decoded
        .currency
        .clone()
        .unwrap_or_else(|| "USD".to_string());
    if limit <= 0.0 {
        return Ok(None);
    }
    Ok(Some(ProviderCostSnapshot {
        used,
        limit,
        currency_code: currency,
        period: Some("Monthly".to_string()),
        resets_at: None,
        updated_at: Utc::now(),
    }))
}

struct WebAccountInfo {
    email: Option<String>,
    organization: Option<String>,
    login_method: Option<String>,
}

async fn claude_web_fetch_account(
    cookie_header: &str,
    org_id: Option<&str>,
) -> Result<Option<WebAccountInfo>> {
    let url = "https://claude.ai/api/account";
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("Cookie", cookie_header)
        .header("Accept", "application/json")
        .header("User-Agent", "FuelcheckCLI")
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        return Ok(None);
    }
    let data = resp.bytes().await?;
    let response: WebAccountResponse = serde_json::from_slice(&data)?;
    let email = sanitize_label(response.email_address);
    let membership = select_claude_membership(response.memberships.as_ref(), org_id);
    let login_method = membership.and_then(|m| {
        infer_web_plan(
            m.organization.rate_limit_tier.as_deref(),
            m.organization.billing_type.as_deref(),
        )
    });
    let organization = membership.and_then(|m| sanitize_label(m.organization.name.clone()));
    Ok(Some(WebAccountInfo {
        email,
        organization,
        login_method,
    }))
}

fn select_claude_org(orgs: &[WebOrganizationResponse]) -> Option<WebOrganizationResponse> {
    let mut selected: Option<&WebOrganizationResponse> = None;
    for org in orgs {
        let has_chat = org
            .capabilities
            .as_ref()
            .map(|caps| caps.iter().any(|c| c.eq_ignore_ascii_case("chat")))
            .unwrap_or(false);
        if has_chat {
            selected = Some(org);
            break;
        }
    }
    if selected.is_none() {
        for org in orgs {
            let is_api_only = org
                .capabilities
                .as_ref()
                .map(|caps| {
                    let normalized: Vec<String> = caps.iter().map(|c| c.to_lowercase()).collect();
                    !normalized.is_empty() && normalized.iter().all(|c| c == "api")
                })
                .unwrap_or(false);
            if !is_api_only {
                selected = Some(org);
                break;
            }
        }
    }
    selected.or_else(|| orgs.first()).cloned()
}

fn select_claude_membership<'a>(
    memberships: Option<&'a Vec<WebAccountMembership>>,
    org_id: Option<&str>,
) -> Option<&'a WebAccountMembership> {
    let memberships = memberships?;
    if let Some(org_id) = org_id {
        if let Some(match_org) = memberships
            .iter()
            .find(|m| m.organization.uuid.as_deref() == Some(org_id))
        {
            return Some(match_org);
        }
    }
    memberships.first()
}

fn make_web_window(window: Option<&WebUsageWindow>, minutes: i64) -> Option<RateWindow> {
    let window = window?;
    let utilization = window.utilization?;
    let resets_at = window.resets_at.as_ref().and_then(|raw| parse_rfc3339(raw));
    let reset_description = resets_at.map(format_reset_description);
    Some(RateWindow {
        used_percent: utilization,
        window_minutes: Some(minutes),
        resets_at,
        reset_description,
    })
}

fn parse_rfc3339(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn normalize_claude_cookie_header(raw: &str) -> String {
    let mut value = raw.trim().to_string();
    if let Some(stripped) = value.strip_prefix("Cookie:") {
        value = stripped.trim().to_string();
    } else if let Some(stripped) = value.strip_prefix("cookie:") {
        value = stripped.trim().to_string();
    }
    let lower = value.to_lowercase();
    if lower.contains("sessionkey=") {
        value
    } else {
        format!("sessionKey={}", value)
    }
}

fn sanitize_label(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn infer_plan(rate_limit_tier: Option<&str>) -> Option<String> {
    let tier = rate_limit_tier.unwrap_or("").to_lowercase();
    if tier.contains("max") {
        return Some("Claude Max".to_string());
    }
    if tier.contains("pro") {
        return Some("Claude Pro".to_string());
    }
    if tier.contains("team") {
        return Some("Claude Team".to_string());
    }
    if tier.contains("enterprise") {
        return Some("Claude Enterprise".to_string());
    }
    None
}

fn infer_web_plan(rate_limit_tier: Option<&str>, billing_type: Option<&str>) -> Option<String> {
    if let Some(plan) = infer_plan(rate_limit_tier) {
        return Some(plan);
    }
    let tier = rate_limit_tier.unwrap_or("").to_lowercase();
    let billing = billing_type.unwrap_or("").to_lowercase();
    if billing.contains("stripe") && tier.contains("claude") {
        return Some("Claude Pro".to_string());
    }
    None
}

fn format_reset_description(reset_at: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = reset_at.signed_duration_since(now);
    if duration.num_seconds() <= 0 {
        return "Resets soon".to_string();
    }
    let hours = duration.num_hours();
    let minutes = (duration.num_minutes() % 60).max(0);
    if hours > 0 {
        format!("Resets in {}h {}m", hours, minutes)
    } else {
        format!("Resets in {}m", minutes)
    }
}
