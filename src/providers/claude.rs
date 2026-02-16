use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{
    ProviderCostSnapshot, ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot,
};
use crate::providers::{fetch_status_payload, Provider, ProviderId, SourcePreference};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use directories::BaseDirs;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

pub struct ClaudeProvider;

#[async_trait]
impl Provider for ClaudeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }

    fn version(&self) -> &'static str {
        "2025-04-20"
    }

    async fn fetch_usage(
        &self,
        args: &UsageArgs,
        config: &Config,
        source: SourcePreference,
    ) -> Result<ProviderPayload> {
        let cfg = config.provider_config(self.id());
        let effective = self.resolve_source(cfg, source);
        let selected = match effective {
            SourcePreference::Auto => {
                if ClaudeOAuthCredentials::load().is_ok() {
                    SourcePreference::Oauth
                } else {
                    SourcePreference::Cli
                }
            }
            other => other,
        };

        let status = if args.status {
            fetch_status_payload("https://status.claude.com").await
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
                "Claude CLI source not implemented in this build. Use --source oauth or log in with Claude CLI."
            )),
            SourcePreference::Web => Err(CliError::UnsupportedSource(self.id(), "web".into()).into()),
            SourcePreference::Api => Err(CliError::UnsupportedSource(self.id(), "api".into()).into()),
            SourcePreference::Local => Err(CliError::UnsupportedSource(self.id(), "local".into()).into()),
            SourcePreference::Auto => Err(CliError::UnsupportedSource(self.id(), "auto".into()).into()),
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
        let path = claude_credentials_path();
        let data = fs::read(path)?;
        let root: ClaudeCredentialsFile = serde_json::from_slice(&data)?;
        let oauth = root.claude_ai_oauth.ok_or_else(|| anyhow!("Claude OAuth missing"))?;
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

    fn is_expired(&self) -> bool {
        self.expires_at.map(|dt| dt <= Utc::now()).unwrap_or(true)
    }
}

fn claude_credentials_path() -> PathBuf {
    let home = BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".claude").join(".credentials.json")
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

async fn fetch_claude_oauth_usage() -> Result<UsageSnapshot> {
    let mut creds = ClaudeOAuthCredentials::load()?;
    if creds.is_expired() {
        if let Some(refresh_token) = creds.refresh_token.clone() {
            if let Ok(updated) = refresh_claude_token(&refresh_token, &creds.scopes, creds.rate_limit_tier.clone()).await
            {
                creds = updated;
            }
        }
    }
    let usage = claude_oauth_fetch(&creds.access_token).await?;
    map_claude_usage(&usage, &creds)
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
        return Err(anyhow!("Claude OAuth unauthorized. Run `claude` to re-authenticate."));
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

fn map_claude_usage(usage: &OAuthUsageResponse, creds: &ClaudeOAuthCredentials) -> Result<UsageSnapshot> {
    let primary = make_window(usage.five_hour.as_ref(), 5 * 60)
        .ok_or_else(|| anyhow!("missing session data"))?;
    let weekly = make_window(usage.seven_day.as_ref(), 7 * 24 * 60);
    let model_specific = make_window(
        usage.seven_day_sonnet.as_ref().or(usage.seven_day_opus.as_ref()),
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

fn oauth_extra_usage_cost(extra: Option<&OAuthExtraUsage>, login_method: Option<&str>) -> Option<ProviderCostSnapshot> {
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
