use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot};
use crate::providers::{
    Provider, ProviderId, SourcePreference, env_var_nonempty, normalize_host, parse_epoch,
};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

pub struct MiniMaxProvider;

#[async_trait]
impl Provider for MiniMaxProvider {
    fn id(&self) -> ProviderId {
        ProviderId::MiniMax
    }

    fn version(&self) -> &'static str {
        "2025-01-01"
    }

    async fn fetch_usage(
        &self,
        _args: &UsageArgs,
        config: &Config,
        source: SourcePreference,
    ) -> Result<ProviderPayload> {
        let cfg = config.provider_config(self.id());
        let api_key = cfg
            .as_ref()
            .and_then(|c| c.api_key.clone())
            .or_else(|| env_var_nonempty(&["MINIMAX_API_KEY"]));
        let cookie_header = cfg
            .as_ref()
            .and_then(|c| c.cookie_header.clone())
            .or_else(|| env_var_nonempty(&["MINIMAX_COOKIE", "MINIMAX_COOKIE_HEADER"]));

        let selected = match source {
            SourcePreference::Auto => {
                if api_key.is_some() {
                    SourcePreference::Api
                } else {
                    SourcePreference::Web
                }
            }
            other => other,
        };

        match selected {
            SourcePreference::Api => {
                let token = api_key.ok_or_else(|| anyhow!("MiniMax API key missing."))?;
                let url = minimax_api_url();
                let client = reqwest::Client::new();
                let resp = client
                    .get(url)
                    .header("authorization", format!("Bearer {}", token))
                    .header("accept", "application/json")
                    .send()
                    .await?;
                let status = resp.status();
                let data = resp.bytes().await?;
                if status.as_u16() == 401 || status.as_u16() == 403 {
                    return Err(anyhow!("MiniMax unauthorized. API key may be invalid."));
                }
                if !status.is_success() {
                    return Err(anyhow!("MiniMax API error (HTTP {})", status.as_u16()));
                }
                let payload: MiniMaxCodingPlanPayload = serde_json::from_slice(&data)?;
                let usage = map_minimax_usage(payload)?;
                Ok(self.ok_output("api", Some(usage)))
            }
            SourcePreference::Web => {
                let cookie_header = cookie_header.ok_or_else(|| anyhow!(
                    "MiniMax cookie header missing. Set provider cookie_header or MINIMAX_COOKIE."
                ))?;
                let url = minimax_remains_url(cfg.as_ref());
                let mut req = reqwest::Client::new().get(url);
                req = req.header("cookie", cookie_header.clone());
                if let Some(token) = extract_cookie_token(&cookie_header) {
                    req = req.header("authorization", format!("Bearer {}", token));
                }
                let resp = req.send().await?;
                let status = resp.status();
                let data = resp.bytes().await?;
                if status.as_u16() == 401 || status.as_u16() == 403 {
                    return Err(anyhow!("MiniMax unauthorized. Cookie may be invalid."));
                }
                if !status.is_success() {
                    return Err(anyhow!("MiniMax API error (HTTP {})", status.as_u16()));
                }
                let payload: MiniMaxCodingPlanPayload = serde_json::from_slice(&data)?;
                let usage = map_minimax_usage(payload)?;
                Ok(self.ok_output("web", Some(usage)))
            }
            _ => Err(CliError::UnsupportedSource(self.id(), selected.to_string()).into()),
        }
    }
}

fn minimax_api_url() -> String {
    env_var_nonempty(&["MINIMAX_REMAINS_URL"])
        .unwrap_or_else(|| "https://api.minimax.io/v1/coding_plan/remains".to_string())
}

fn minimax_remains_url(cfg: Option<&crate::config::ProviderConfig>) -> String {
    if let Some(url) = env_var_nonempty(&["MINIMAX_REMAINS_URL"]) {
        return url;
    }
    let host = if let Some(host) = env_var_nonempty(&["MINIMAX_HOST"]) {
        normalize_host(&host)
    } else if let Some(region) = cfg.and_then(|c| c.region.clone()) {
        if region.to_lowercase().contains("cn") {
            "https://platform.minimaxi.com".to_string()
        } else {
            "https://platform.minimax.io".to_string()
        }
    } else {
        "https://platform.minimax.io".to_string()
    };
    format!(
        "{}/v1/api/openplatform/coding_plan/remains",
        host.trim_end_matches('/')
    )
}

fn extract_cookie_token(cookie: &str) -> Option<String> {
    for part in cookie.split(';') {
        let mut kv = part.trim().splitn(2, '=');
        let key = kv.next()?.trim();
        let value = kv.next()?.trim();
        if key.eq_ignore_ascii_case("access_token") || key.eq_ignore_ascii_case("accessToken") {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

#[derive(Debug, Deserialize)]
struct MiniMaxCodingPlanPayload {
    data: Option<MiniMaxCodingPlanData>,
    #[serde(rename = "base_resp")]
    base_resp: Option<MiniMaxBaseResponse>,
    #[serde(rename = "model_remains")]
    model_remains: Option<Vec<MiniMaxModelRemains>>,
}

#[derive(Debug, Deserialize)]
struct MiniMaxCodingPlanData {
    #[serde(rename = "base_resp")]
    base_resp: Option<MiniMaxBaseResponse>,
    #[serde(rename = "plan_name")]
    plan_name: Option<String>,
    #[serde(rename = "current_subscribe_title")]
    current_subscribe_title: Option<String>,
    #[serde(rename = "combo_title")]
    combo_title: Option<String>,
    #[serde(rename = "current_plan_title")]
    current_plan_title: Option<String>,
    #[serde(rename = "current_combo_card")]
    current_combo_card: Option<MiniMaxComboCard>,
    #[serde(rename = "model_remains")]
    model_remains: Option<Vec<MiniMaxModelRemains>>,
}

#[derive(Debug, Deserialize)]
struct MiniMaxComboCard {
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MiniMaxBaseResponse {
    #[serde(rename = "status_code")]
    status_code: Option<i64>,
    #[serde(rename = "status_msg")]
    status_msg: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct MiniMaxModelRemains {
    #[serde(rename = "current_interval_total_count")]
    current_interval_total_count: Option<i64>,
    #[serde(rename = "current_interval_usage_count")]
    current_interval_usage_count: Option<i64>,
    #[serde(rename = "start_time")]
    start_time: Option<i64>,
    #[serde(rename = "end_time")]
    end_time: Option<i64>,
    #[serde(rename = "remains_time")]
    remains_time: Option<i64>,
}

fn map_minimax_usage(payload: MiniMaxCodingPlanPayload) -> Result<UsageSnapshot> {
    let data = payload.data.as_ref();
    let base_resp = data
        .and_then(|d| d.base_resp.as_ref())
        .or(payload.base_resp.as_ref());
    if let Some(status) = base_resp.and_then(|b| b.status_code) {
        if status != 0 {
            let msg = base_resp
                .and_then(|b| b.status_msg.clone())
                .unwrap_or_else(|| format!("status_code {}", status));
            return Err(anyhow!("MiniMax API error: {}", msg));
        }
    }

    let model_remains: Vec<MiniMaxModelRemains> = data
        .and_then(|d| d.model_remains.clone())
        .or(payload.model_remains.clone())
        .unwrap_or_default();
    let first = model_remains
        .first()
        .ok_or_else(|| anyhow!("MiniMax usage data missing"))?;
    let total = first.current_interval_total_count.unwrap_or(0);
    let remaining = first.current_interval_usage_count.unwrap_or(0);
    let used_percent = if total > 0 {
        ((total - remaining) as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    let start = first.start_time.and_then(parse_epoch);
    let end = first.end_time.and_then(parse_epoch);
    let window_minutes = match (start, end) {
        (Some(s), Some(e)) => {
            let minutes = (e - s).num_minutes();
            if minutes > 0 { Some(minutes) } else { None }
        }
        _ => None,
    };
    let resets_at = match first.remains_time {
        Some(remains) if remains > 0 => Some(derive_reset_from_remains(remains, Utc::now())),
        _ => end,
    };

    let plan = data
        .and_then(|d| d.plan_name.clone())
        .or_else(|| data.and_then(|d| d.current_plan_title.clone()))
        .or_else(|| data.and_then(|d| d.current_subscribe_title.clone()))
        .or_else(|| data.and_then(|d| d.combo_title.clone()))
        .or_else(|| data.and_then(|d| d.current_combo_card.as_ref().and_then(|c| c.title.clone())));

    let identity = ProviderIdentitySnapshot {
        provider_id: Some("minimax".to_string()),
        account_email: None,
        account_organization: None,
        login_method: plan,
    };

    Ok(UsageSnapshot {
        primary: Some(RateWindow {
            used_percent,
            window_minutes,
            resets_at,
            reset_description: None,
        }),
        secondary: None,
        tertiary: None,
        provider_cost: None,
        updated_at: Utc::now(),
        identity: Some(identity.clone()),
        account_email: identity.account_email,
        account_organization: identity.account_organization,
        login_method: identity.login_method,
    })
}

fn derive_reset_from_remains(remains: i64, now: DateTime<Utc>) -> DateTime<Utc> {
    if remains > 1_000_000 {
        now + chrono::Duration::seconds(remains / 1000)
    } else {
        now + chrono::Duration::seconds(remains)
    }
}
