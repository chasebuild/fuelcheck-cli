use crate::accounts::{account_label, select_accounts, AccountSelectionArgs};
use crate::cli::UsageArgs;
use crate::config::{Config, TokenAccount};
use crate::errors::CliError;
use crate::model::{
    CreditsSnapshot, ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot,
};
use crate::providers::{fetch_status_payload, Provider, ProviderId, SourcePreference};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use directories::BaseDirs;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

pub struct CodexProvider;

#[async_trait]
impl Provider for CodexProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }

    fn version(&self) -> &'static str {
        "2024-06-04"
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
        let selected = select_accounts(cfg.as_ref().and_then(|c| c.token_accounts.as_ref()), &selection)?;
        let Some(selected) = selected else {
            return Ok(vec![self.fetch_usage(args, config, source).await?]);
        };

        let effective = self.resolve_source(cfg.clone(), source);
        let selected_source = match effective {
            SourcePreference::Auto | SourcePreference::Oauth => SourcePreference::Oauth,
            other => other,
        };
        if selected_source != SourcePreference::Oauth {
            return Err(CliError::UnsupportedSource(self.id(), selected_source.to_string()).into());
        }

        let status = if args.status {
            fetch_status_payload("https://status.openai.com").await
        } else {
            None
        };

        let mut outputs = Vec::new();
        for account in selected {
            let creds = CodexOAuthCredentials::from_token_account(&account.account, account.index)?;
            let (usage, credits) = fetch_oauth_usage_with_creds(&creds).await?;
            let mut payload = self.ok_output("oauth", Some(usage));
            if !args.no_credits {
                payload.credits = credits;
            }
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
        let effective = self.resolve_source(cfg.clone(), source);
        let selected = match effective {
            SourcePreference::Auto => {
                if CodexOAuthCredentials::load().is_ok() {
                    SourcePreference::Oauth
                } else {
                    SourcePreference::Cli
                }
            }
            other => other,
        };

        let status = if args.status {
            fetch_status_payload("https://status.openai.com").await
        } else {
            None
        };

        match selected {
            SourcePreference::Oauth => {
                let (usage, credits) = fetch_oauth_usage().await?;
                let mut payload = self.ok_output("oauth", Some(usage));
                if !args.no_credits {
                    payload.credits = credits;
                }
                payload.status = status;
                Ok(payload)
            }
            SourcePreference::Cli => Err(anyhow!(
                "Codex CLI source not implemented in this build. Use --source oauth or log in with Codex CLI."
            )),
            SourcePreference::Web => Err(CliError::UnsupportedSource(self.id(), "web".into()).into()),
            SourcePreference::Api => Err(CliError::UnsupportedSource(self.id(), "api".into()).into()),
            SourcePreference::Local => Err(CliError::UnsupportedSource(self.id(), "local".into()).into()),
            SourcePreference::Auto => Err(CliError::UnsupportedSource(self.id(), "auto".into()).into()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct CodexUsageResponse {
    #[serde(rename = "plan_type")]
    plan_type: Option<String>,
    #[serde(rename = "rate_limit")]
    rate_limit: Option<RateLimitDetails>,
    credits: Option<CreditDetails>,
}

#[derive(Debug, Deserialize)]
struct RateLimitDetails {
    #[serde(rename = "primary_window")]
    primary_window: Option<WindowSnapshot>,
    #[serde(rename = "secondary_window")]
    secondary_window: Option<WindowSnapshot>,
}

#[derive(Debug, Deserialize)]
struct WindowSnapshot {
    #[serde(rename = "used_percent")]
    used_percent: i64,
    #[serde(rename = "reset_at")]
    reset_at: i64,
    #[serde(rename = "limit_window_seconds")]
    limit_window_seconds: i64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CreditDetails {
    #[serde(rename = "has_credits")]
    has_credits: Option<bool>,
    unlimited: Option<bool>,
    balance: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct AuthJson {
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    tokens: Option<AuthTokens>,
    #[serde(rename = "last_refresh")]
    last_refresh: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthTokens {
    #[serde(rename = "access_token")]
    access_token: Option<String>,
    #[serde(rename = "refresh_token")]
    refresh_token: Option<String>,
    #[serde(rename = "id_token")]
    id_token: Option<String>,
    #[serde(rename = "account_id")]
    account_id: Option<String>,
}

#[derive(Debug, Clone)]
struct CodexOAuthCredentials {
    access_token: String,
    refresh_token: String,
    id_token: Option<String>,
    account_id: Option<String>,
    last_refresh: Option<DateTime<Utc>>,
}

impl CodexOAuthCredentials {
    fn load() -> Result<Self> {
        let auth_path = codex_auth_path();
        let data = fs::read(&auth_path)
            .with_context(|| format!("read {}", auth_path.display()))?;
        let auth: AuthJson = serde_json::from_slice(&data)?;

        if let Some(api_key) = auth.openai_api_key.clone().filter(|s| !s.trim().is_empty()) {
            return Ok(Self {
                access_token: api_key,
                refresh_token: String::new(),
                id_token: None,
                account_id: None,
                last_refresh: None,
            });
        }

        let tokens = auth.tokens.ok_or_else(|| anyhow!("Codex auth.json missing tokens"))?;
        let access_token = tokens
            .access_token
            .ok_or_else(|| anyhow!("Codex auth.json missing access_token"))?;
        let refresh_token = tokens.refresh_token.unwrap_or_default();
        let id_token = tokens.id_token;
        let account_id = tokens.account_id;
        let last_refresh = auth
            .last_refresh
            .and_then(|raw| DateTime::parse_from_rfc3339(&raw).ok())
            .map(|dt| dt.with_timezone(&Utc));

        Ok(Self {
            access_token,
            refresh_token,
            id_token,
            account_id,
            last_refresh,
        })
    }

    fn from_token_account(account: &TokenAccount, index: usize) -> Result<Self> {
        let token = account
            .token
            .clone()
            .filter(|val| !val.trim().is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "Codex token account {} missing token",
                    account_label(account, index)
                )
            })?;
        let account_id = account
            .id
            .clone()
            .filter(|val| !val.trim().is_empty());
        Ok(Self {
            access_token: token,
            refresh_token: String::new(),
            id_token: None,
            account_id,
            last_refresh: None,
        })
    }

    fn needs_refresh(&self) -> bool {
        let Some(last) = self.last_refresh else { return true };
        let age = Utc::now().signed_duration_since(last);
        age.num_days() >= 8
    }

    fn save(&self) -> Result<()> {
        let auth_path = codex_auth_path();
        let mut json: serde_json::Value = if auth_path.exists() {
            serde_json::from_slice(&fs::read(&auth_path)?)?
        } else {
            serde_json::json!({})
        };
        json["tokens"] = serde_json::json!({
            "access_token": self.access_token,
            "refresh_token": self.refresh_token,
            "id_token": self.id_token,
            "account_id": self.account_id,
        });
        json["last_refresh"] = serde_json::json!(Utc::now().to_rfc3339());

        if let Some(parent) = auth_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&auth_path, serde_json::to_vec_pretty(&json)?)?;
        Ok(())
    }
}

fn codex_auth_path() -> PathBuf {
    let home = BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let codex_home = std::env::var("CODEX_HOME")
        .ok()
        .and_then(|v| if v.trim().is_empty() { None } else { Some(v) });
    match codex_home {
        Some(root) => PathBuf::from(root).join("auth.json"),
        None => home.join(".codex").join("auth.json"),
    }
}

async fn fetch_oauth_usage() -> Result<(UsageSnapshot, Option<CreditsSnapshot>)> {
    let mut creds = CodexOAuthCredentials::load()?;
    if creds.needs_refresh() && !creds.refresh_token.is_empty() {
        creds = refresh_codex_token(&creds).await?;
        let _ = creds.save();
    }
    fetch_oauth_usage_with_creds(&creds).await
}

async fn fetch_oauth_usage_with_creds(
    creds: &CodexOAuthCredentials,
) -> Result<(UsageSnapshot, Option<CreditsSnapshot>)> {
    let usage = codex_oauth_fetch(creds).await?;
    let usage_snapshot = map_codex_usage(&usage, creds)?;
    let credits = map_codex_credits(&usage);
    Ok((usage_snapshot, credits))
}

async fn refresh_codex_token(creds: &CodexOAuthCredentials) -> Result<CodexOAuthCredentials> {
    let url = "https://auth.openai.com/oauth/token";
    let body = serde_json::json!({
        "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
        "grant_type": "refresh_token",
        "refresh_token": creds.refresh_token,
        "scope": "openid profile email"
    });

    let client = reqwest::Client::new();
    let resp = client.post(url).json(&body).send().await?;
    let status = resp.status();
    let data = resp.bytes().await?;
    if !status.is_success() {
        return Err(anyhow!(
            "Codex OAuth refresh failed (HTTP {})",
            status.as_u16()
        ));
    }
    let json: serde_json::Value = serde_json::from_slice(&data)?;
    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or(&creds.access_token)
        .to_string();
    let refresh_token = json
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or(&creds.refresh_token)
        .to_string();
    let id_token = json.get("id_token").and_then(|v| v.as_str()).map(|s| s.to_string());

    Ok(CodexOAuthCredentials {
        access_token,
        refresh_token,
        id_token,
        account_id: creds.account_id.clone(),
        last_refresh: Some(Utc::now()),
    })
}

async fn codex_oauth_fetch(creds: &CodexOAuthCredentials) -> Result<CodexUsageResponse> {
    let url = resolve_codex_usage_url()?;
    let client = reqwest::Client::new();
    let mut req = client.get(url);
    req = req
        .header("Authorization", format!("Bearer {}", creds.access_token))
        .header("User-Agent", "FuelcheckCLI")
        .header("Accept", "application/json");
    if let Some(account_id) = &creds.account_id {
        if !account_id.trim().is_empty() {
            req = req.header("ChatGPT-Account-Id", account_id.clone());
        }
    }
    let resp = req.send().await?;
    let status = resp.status();
    let data = resp.bytes().await?;
    if !status.is_success() {
        return Err(anyhow!(
            "Codex OAuth usage fetch failed (HTTP {})",
            status.as_u16()
        ));
    }
    let usage: CodexUsageResponse = serde_json::from_slice(&data)?;
    Ok(usage)
}

fn resolve_codex_usage_url() -> Result<String> {
    let default_base = "https://chatgpt.com/backend-api";
    let config_base = load_codex_base_url_from_config().unwrap_or_else(|| default_base.to_string());
    let normalized = if config_base.contains("/backend-api") {
        config_base
    } else {
        format!("{}/backend-api", config_base.trim_end_matches('/'))
    };
    let path = if normalized.contains("/backend-api") {
        "/wham/usage"
    } else {
        "/api/codex/usage"
    };
    Ok(format!("{}{}", normalized, path))
}

fn load_codex_base_url_from_config() -> Option<String> {
    let home = BaseDirs::new()?.home_dir().to_path_buf();
    let codex_home = std::env::var("CODEX_HOME").ok().and_then(|v| {
        let trimmed = v.trim().to_string();
        if trimmed.is_empty() { None } else { Some(trimmed) }
    });
    let root = codex_home.unwrap_or_else(|| home.join(".codex").to_string_lossy().to_string());
    let config_path = PathBuf::from(root).join("config.toml");
    let contents = fs::read_to_string(config_path).ok()?;
    for line in contents.lines() {
        let trimmed = line.split('#').next().unwrap_or("").trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.splitn(2, '=');
        let key = parts.next()?.trim();
        let value = parts.next()?.trim();
        if key == "chatgpt_base_url" {
            let val = value.trim_matches('"').trim_matches('\'').to_string();
            if val.is_empty() {
                return None;
            }
            return Some(val);
        }
    }
    None
}

fn map_codex_usage(usage: &CodexUsageResponse, creds: &CodexOAuthCredentials) -> Result<UsageSnapshot> {
    let primary = make_window(usage.rate_limit.as_ref().and_then(|r| r.primary_window.as_ref()));
    let secondary = make_window(usage.rate_limit.as_ref().and_then(|r| r.secondary_window.as_ref()));

    let identity = ProviderIdentitySnapshot {
        provider_id: Some("codex".to_string()),
        account_email: resolve_account_email(creds.id_token.as_deref()),
        account_organization: None,
        login_method: resolve_plan(usage, creds.id_token.as_deref()),
    };

    Ok(UsageSnapshot {
        primary: primary.or_else(|| {
            Some(RateWindow {
                used_percent: 0.0,
                window_minutes: None,
                resets_at: None,
                reset_description: None,
            })
        }),
        secondary,
        tertiary: None,
        provider_cost: None,
        updated_at: Utc::now(),
        account_email: identity.account_email.clone(),
        account_organization: identity.account_organization.clone(),
        login_method: identity.login_method.clone(),
        identity: Some(identity),
    })
}

fn map_codex_credits(usage: &CodexUsageResponse) -> Option<CreditsSnapshot> {
    let credits = usage.credits.as_ref()?;
    let balance = credits.balance.as_ref().and_then(|v| match v {
        serde_json::Value::Number(num) => num.as_f64(),
        serde_json::Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    })?;
    Some(CreditsSnapshot {
        remaining: balance,
        events: Vec::new(),
        updated_at: Utc::now(),
    })
}

fn make_window(window: Option<&WindowSnapshot>) -> Option<RateWindow> {
    let window = window?;
    let resets_at = DateTime::<Utc>::from_timestamp(window.reset_at as i64, 0);
    let reset_description = resets_at.map(|dt| format_reset_description(dt));
    Some(RateWindow {
        used_percent: window.used_percent as f64,
        window_minutes: Some(window.limit_window_seconds / 60),
        resets_at,
        reset_description,
    })
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

fn resolve_account_email(id_token: Option<&str>) -> Option<String> {
    let payload = parse_jwt_payload(id_token)?;
    payload
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            payload
                .get("https://api.openai.com/profile")
                .and_then(|v| v.as_object())
                .and_then(|obj| obj.get("email"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}

fn resolve_plan(usage: &CodexUsageResponse, id_token: Option<&str>) -> Option<String> {
    if let Some(plan) = &usage.plan_type {
        if !plan.trim().is_empty() {
            return Some(plan.clone());
        }
    }
    let payload = parse_jwt_payload(id_token)?;
    payload
        .get("https://api.openai.com/auth")
        .and_then(|v| v.as_object())
        .and_then(|obj| obj.get("chatgpt_plan_type"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            payload
                .get("chatgpt_plan_type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}

fn parse_jwt_payload(token: Option<&str>) -> Option<serde_json::Value> {
    let token = token?;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let payload = parts[1];
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&decoded).ok()
}
