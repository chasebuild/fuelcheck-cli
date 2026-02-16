use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot};
use crate::providers::{env_var_nonempty, parse_rfc3339, Provider, ProviderId, SourcePreference};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

pub struct KimiProvider;

#[async_trait]
impl Provider for KimiProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Kimi
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
            .or_else(|| env_var_nonempty(&["KIMI_AUTH_TOKEN"]))
            .ok_or_else(|| anyhow!("Kimi auth token missing. Set provider api_key or KIMI_AUTH_TOKEN."))?;

        let selected = match source {
            SourcePreference::Auto => SourcePreference::Api,
            other => other,
        };
        if selected != SourcePreference::Api {
            return Err(CliError::UnsupportedSource(self.id(), selected.to_string()).into());
        }

        let client = reqwest::Client::new();
        let resp = client
            .post("https://www.kimi.com/apiv2/kimi.gateway.billing.v1.BillingService/GetUsages")
            .header("authorization", format!("Bearer {}", token))
            .header("accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        let data = resp.bytes().await?;
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(anyhow!("Kimi unauthorized. Token may be invalid."));
        }
        if !status.is_success() {
            return Err(anyhow!("Kimi API error (HTTP {})", status.as_u16()));
        }
        let response: KimiUsageResponse = serde_json::from_slice(&data)?;
        let usage = map_kimi_usage(response)?;
        Ok(self.ok_output("api", Some(usage)))
    }
}

#[derive(Debug, Deserialize)]
struct KimiUsageResponse {
    usages: Option<Vec<KimiUsageScope>>,
}

#[derive(Debug, Deserialize)]
struct KimiUsageScope {
    scope: Option<String>,
    detail: Option<KimiUsageDetail>,
    limits: Option<Vec<KimiUsageLimit>>,
}

#[derive(Debug, Deserialize)]
struct KimiUsageDetail {
    limit: Option<String>,
    used: Option<String>,
    remaining: Option<String>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KimiUsageLimit {
    window: Option<KimiUsageWindow>,
    detail: Option<KimiUsageDetail>,
}

#[derive(Debug, Deserialize)]
struct KimiUsageWindow {
    duration: Option<i64>,
    #[serde(rename = "timeUnit")]
    time_unit: Option<String>,
}

fn map_kimi_usage(response: KimiUsageResponse) -> Result<UsageSnapshot> {
    let usages = response.usages.unwrap_or_default();
    let selected = usages
        .iter()
        .find(|u| u.scope.as_deref() == Some("FEATURE_CODING"))
        .or_else(|| usages.first())
        .ok_or_else(|| anyhow!("Kimi usage data missing"))?;

    let primary = selected
        .limits
        .as_ref()
        .and_then(|limits| limits.first())
        .and_then(make_kimi_window);

    let secondary = selected
        .detail
        .as_ref()
        .and_then(|detail| make_kimi_window_from_detail(detail, None));

    let identity = ProviderIdentitySnapshot {
        provider_id: Some("kimi".to_string()),
        account_email: None,
        account_organization: None,
        login_method: None,
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

fn make_kimi_window(limit: &KimiUsageLimit) -> Option<RateWindow> {
    let window_minutes = limit
        .window
        .as_ref()
        .and_then(|w| window_minutes_from(w.duration, w.time_unit.as_deref()));
    limit
        .detail
        .as_ref()
        .and_then(|detail| make_kimi_window_from_detail(detail, window_minutes))
}

fn make_kimi_window_from_detail(detail: &KimiUsageDetail, window_minutes: Option<i64>) -> Option<RateWindow> {
    let used = detail.used.as_ref()?.trim().parse::<f64>().ok()?;
    let limit = detail.limit.as_ref()?.trim().parse::<f64>().ok()?;
    if limit <= 0.0 {
        return None;
    }
    let used_percent = (used / limit) * 100.0;
    let resets_at = detail
        .reset_time
        .as_ref()
        .and_then(|raw| parse_rfc3339(raw));
    Some(RateWindow {
        used_percent,
        window_minutes,
        resets_at,
        reset_description: None,
    })
}

fn window_minutes_from(duration: Option<i64>, unit: Option<&str>) -> Option<i64> {
    let duration = duration?;
    let unit = unit.unwrap_or("TIME_UNIT_MINUTE").to_lowercase();
    let minutes = if unit.contains("hour") {
        duration * 60
    } else if unit.contains("day") {
        duration * 60 * 24
    } else {
        duration
    };
    Some(minutes)
}
