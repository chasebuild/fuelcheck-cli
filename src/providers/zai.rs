use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot};
use crate::providers::{
    Provider, ProviderId, SourcePreference, env_var_nonempty, normalize_host, value_to_f64,
    value_to_i64,
};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

pub struct ZaiProvider;

#[async_trait]
impl Provider for ZaiProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Zai
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
        let token = cfg
            .as_ref()
            .and_then(|c| c.api_key.clone())
            .or_else(|| env_var_nonempty(&["Z_AI_API_KEY"]))
            .ok_or_else(|| {
                anyhow!("z.ai API token missing. Set provider api_key or Z_AI_API_KEY.")
            })?;

        let selected = match source {
            SourcePreference::Auto => SourcePreference::Api,
            other => other,
        };
        if selected != SourcePreference::Api {
            return Err(CliError::UnsupportedSource(self.id(), selected.to_string()).into());
        }

        let url = resolve_zai_quota_url(cfg.as_ref());
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
            return Err(anyhow!("z.ai unauthorized. Token may be invalid."));
        }
        if !status.is_success() {
            return Err(anyhow!("z.ai quota API error (HTTP {})", status.as_u16()));
        }
        let json: Value = serde_json::from_slice(&data)?;
        let usage = parse_zai_usage(&json)?;
        Ok(self.ok_output("api", Some(usage)))
    }
}

fn resolve_zai_quota_url(cfg: Option<&crate::config::ProviderConfig>) -> String {
    if let Some(url) = env_var_nonempty(&["Z_AI_QUOTA_URL"]) {
        return url;
    }
    if let Some(host) = env_var_nonempty(&["Z_AI_API_HOST"]) {
        return format!("{}/api/monitor/usage/quota/limit", normalize_host(&host));
    }
    if let Some(region) = cfg.and_then(|c| c.region.clone()) {
        if region.to_lowercase().contains("cn") || region.to_lowercase().contains("bigmodel") {
            return "https://open.bigmodel.cn/api/monitor/usage/quota/limit".to_string();
        }
    }
    "https://api.z.ai/api/monitor/usage/quota/limit".to_string()
}

fn parse_zai_usage(json: &Value) -> Result<UsageSnapshot> {
    let data = json.get("data").unwrap_or(json);
    let plan = find_string(data, &["planName", "plan", "plan_type", "packageName"]);
    let limits = data
        .get("limits")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut primary: Option<RateWindow> = None;
    let mut secondary: Option<RateWindow> = None;
    for limit in limits {
        let kind = find_string(&limit, &["limitType", "limit_type", "type"]).unwrap_or_default();
        let window = parse_zai_limit(&limit);
        if window.is_none() {
            continue;
        }
        let window = window.unwrap();
        let kind_lower = kind.to_lowercase();
        if primary.is_none() && (kind_lower.contains("token") || kind_lower.contains("tokens")) {
            primary = Some(window);
        } else if secondary.is_none() && (kind_lower.contains("time") || kind_lower.contains("mcp"))
        {
            secondary = Some(window);
        } else if primary.is_none() {
            primary = Some(window);
        } else if secondary.is_none() {
            secondary = Some(window);
        }
    }

    let identity = ProviderIdentitySnapshot {
        provider_id: Some("zai".to_string()),
        account_email: None,
        account_organization: None,
        login_method: plan,
    };
    Ok(UsageSnapshot {
        primary,
        secondary,
        tertiary: None,
        provider_cost: None,
        updated_at: Utc::now(),
        identity: Some(identity.clone()),
        account_email: identity.account_email,
        account_organization: identity.account_organization,
        login_method: identity.login_method,
    })
}

fn parse_zai_limit(limit: &Value) -> Option<RateWindow> {
    let used_percent = find_number(
        limit,
        &[
            "usedPercent",
            "used_percent",
            "usagePercent",
            "usage_percent",
            "percentUsed",
            "percent_used",
        ],
    )
    .or_else(|| {
        let used = find_number(limit, &["used", "usage", "current", "consumed"]);
        let total = find_number(limit, &["limit", "quota", "total", "max"]);
        if let (Some(used), Some(total)) = (used, total) {
            Some((used / total) * 100.0)
        } else {
            let remaining = find_number(limit, &["remaining", "left"]);
            if let (Some(remaining), Some(total)) = (remaining, total) {
                Some(((total - remaining) / total) * 100.0)
            } else {
                None
            }
        }
    })?;

    let window_minutes = parse_window_minutes(limit);
    let resets_at = find_epoch(
        limit,
        &["nextResetTime", "resetTime", "resetAt", "next_reset_time"],
    )
    .or_else(|| find_rfc3339(limit, &["resetTime", "resetsAt", "reset_at"]));
    Some(RateWindow {
        used_percent,
        window_minutes,
        resets_at,
        reset_description: None,
    })
}

fn parse_window_minutes(limit: &Value) -> Option<i64> {
    let window = limit
        .get("window")
        .or_else(|| limit.get("timeWindow"))
        .or_else(|| limit.get("windowInfo"))
        .or_else(|| limit.get("period"));
    let window = window?;
    let number = find_number(window, &["number", "duration", "window", "size", "count"])?;
    let unit = find_string(window, &["unit", "timeUnit", "windowUnit", "type"]).unwrap_or_default();
    let unit_lower = unit.to_lowercase();
    let minutes = if unit_lower.contains("minute") {
        number
    } else if unit_lower.contains("hour") {
        number * 60.0
    } else if unit_lower.contains("day") {
        number * 60.0 * 24.0
    } else if unit_lower.contains("week") {
        number * 60.0 * 24.0 * 7.0
    } else if unit_lower.contains("month") {
        number * 60.0 * 24.0 * 30.0
    } else {
        number
    };
    Some(minutes.round() as i64)
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(val) = value.get(*key) {
            if let Some(s) = val.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

fn find_number(value: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(val) = value.get(*key) {
            if let Some(num) = value_to_f64(val) {
                return Some(num);
            }
        }
    }
    None
}

fn find_epoch(value: &Value, keys: &[&str]) -> Option<chrono::DateTime<Utc>> {
    for key in keys {
        if let Some(val) = value.get(*key) {
            if let Some(num) = value_to_i64(val) {
                return crate::providers::parse_epoch(num);
            }
        }
    }
    None
}

fn find_rfc3339(value: &Value, keys: &[&str]) -> Option<chrono::DateTime<Utc>> {
    for key in keys {
        if let Some(val) = value.get(*key) {
            if let Some(s) = val.as_str() {
                if let Some(dt) = crate::providers::parse_rfc3339(s) {
                    return Some(dt);
                }
            }
        }
    }
    None
}
