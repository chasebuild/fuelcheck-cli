use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot};
use crate::providers::{Provider, ProviderId, SourcePreference, fetch_status_payload};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

pub struct FactoryProvider;

#[async_trait]
impl Provider for FactoryProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Factory
    }

    fn version(&self) -> &'static str {
        "2026-02-16"
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
            .or_else(|| std::env::var("FACTORY_COOKIE").ok())
            .or_else(|| std::env::var("DROID_COOKIE").ok())
            .ok_or_else(|| {
                anyhow!(
                    "Factory (Droid) cookie header missing. Set provider cookie_header in config."
                )
            })?;

        let bearer_token = cfg
            .as_ref()
            .and_then(|c| c.api_key.clone())
            .or_else(|| std::env::var("FACTORY_BEARER_TOKEN").ok())
            .or_else(|| extract_access_token(&cookie_header));

        let base_url = std::env::var("FACTORY_BASE_URL")
            .unwrap_or_else(|_| "https://app.factory.ai".to_string());
        let selected = match source {
            SourcePreference::Auto => SourcePreference::Web,
            other => other,
        };

        let status = if args.status {
            fetch_status_payload("https://status.factory.ai", args.web_timeout).await
        } else {
            None
        };

        match selected {
            SourcePreference::Web | SourcePreference::Api => {
                let usage =
                    fetch_factory_usage(&cookie_header, bearer_token.as_deref(), &base_url).await?;
                let mut payload = self.ok_output("web", Some(usage));
                payload.status = status;
                Ok(payload)
            }
            _ => Err(CliError::UnsupportedSource(self.id(), selected.to_string()).into()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct FactoryAuthResponse {
    organization: Option<FactoryOrganization>,
}

#[derive(Debug, Deserialize)]
struct FactoryOrganization {
    id: Option<String>,
    name: Option<String>,
    subscription: Option<FactorySubscription>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactorySubscription {
    factory_tier: Option<String>,
    orb_subscription: Option<FactoryOrbSubscription>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactoryOrbSubscription {
    plan: Option<FactoryPlan>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FactoryPlan {
    name: Option<String>,
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactoryUsageResponse {
    usage: Option<FactoryUsageData>,
    source: Option<String>,
    user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactoryUsageData {
    start_date: Option<i64>,
    end_date: Option<i64>,
    standard: Option<FactoryTokenUsage>,
    premium: Option<FactoryTokenUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactoryTokenUsage {
    user_tokens: Option<i64>,
    org_total_tokens_used: Option<i64>,
    total_allowance: Option<i64>,
    used_ratio: Option<f64>,
    org_overage_used: Option<i64>,
    basic_allowance: Option<i64>,
    org_overage_limit: Option<i64>,
}

async fn fetch_factory_usage(
    cookie_header: &str,
    bearer_token: Option<&str>,
    base_url: &str,
) -> Result<UsageSnapshot> {
    let auth = fetch_factory_auth(cookie_header, bearer_token, base_url).await?;
    let usage =
        fetch_factory_subscription_usage(cookie_header, bearer_token, base_url, None).await?;
    Ok(build_snapshot(auth, usage))
}

async fn fetch_factory_auth(
    cookie_header: &str,
    bearer_token: Option<&str>,
    base_url: &str,
) -> Result<FactoryAuthResponse> {
    let url = format!("{}/api/app/auth/me", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let mut request = client
        .get(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("Origin", "https://app.factory.ai")
        .header("Referer", "https://app.factory.ai/")
        .header("x-factory-client", "web-app");

    if !cookie_header.is_empty() {
        request = request.header("Cookie", cookie_header);
    }
    if let Some(token) = bearer_token {
        request = request.header("Authorization", format!("Bearer {}", token));
    }

    let resp = request.send().await?;
    let status = resp.status();
    let data = resp.bytes().await?;

    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(anyhow!("Factory not logged in. Update cookie header."));
    }
    if !status.is_success() {
        return Err(anyhow!(
            "Factory API error (HTTP {}{})",
            status.as_u16(),
            response_snippet(&data)
        ));
    }

    serde_json::from_slice(&data).map_err(|err| {
        anyhow!(
            "Factory auth decode failed: {}{}",
            err,
            response_snippet(&data)
        )
    })
}

async fn fetch_factory_subscription_usage(
    cookie_header: &str,
    bearer_token: Option<&str>,
    base_url: &str,
    user_id: Option<&str>,
) -> Result<FactoryUsageResponse> {
    let url = format!(
        "{}/api/organization/subscription/usage",
        base_url.trim_end_matches('/')
    );
    let client = reqwest::Client::new();
    let mut request = client
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("Origin", "https://app.factory.ai")
        .header("Referer", "https://app.factory.ai/")
        .header("x-factory-client", "web-app");

    if !cookie_header.is_empty() {
        request = request.header("Cookie", cookie_header);
    }
    if let Some(token) = bearer_token {
        request = request.header("Authorization", format!("Bearer {}", token));
    }

    let mut body = serde_json::json!({ "useCache": true });
    if let Some(user_id) = user_id {
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "userId".to_string(),
                serde_json::Value::String(user_id.to_string()),
            );
        }
    }

    let resp = request.json(&body).send().await?;
    let status = resp.status();
    let data = resp.bytes().await?;

    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(anyhow!("Factory not logged in. Update cookie header."));
    }
    if !status.is_success() {
        return Err(anyhow!(
            "Factory API error (HTTP {}{})",
            status.as_u16(),
            response_snippet(&data)
        ));
    }

    serde_json::from_slice(&data).map_err(|err| {
        anyhow!(
            "Factory usage decode failed: {}{}",
            err,
            response_snippet(&data)
        )
    })
}

fn build_snapshot(auth: FactoryAuthResponse, usage: FactoryUsageResponse) -> UsageSnapshot {
    let usage_data = usage.usage.unwrap_or(FactoryUsageData {
        start_date: None,
        end_date: None,
        standard: None,
        premium: None,
    });

    let period_end = usage_data.end_date.and_then(timestamp_millis);
    let reset_description = period_end.map(format_reset_description);

    let standard_used = usage_data
        .standard
        .as_ref()
        .and_then(|u| u.user_tokens)
        .unwrap_or(0);
    let standard_allowance = usage_data
        .standard
        .as_ref()
        .and_then(|u| u.total_allowance)
        .unwrap_or(0);
    let standard_ratio = usage_data.standard.as_ref().and_then(|u| u.used_ratio);

    let premium_used = usage_data
        .premium
        .as_ref()
        .and_then(|u| u.user_tokens)
        .unwrap_or(0);
    let premium_allowance = usage_data
        .premium
        .as_ref()
        .and_then(|u| u.total_allowance)
        .unwrap_or(0);
    let premium_ratio = usage_data.premium.as_ref().and_then(|u| u.used_ratio);

    let primary = RateWindow {
        used_percent: calculate_usage_percent(standard_used, standard_allowance, standard_ratio),
        window_minutes: None,
        resets_at: period_end,
        reset_description: reset_description.clone(),
    };

    let secondary = RateWindow {
        used_percent: calculate_usage_percent(premium_used, premium_allowance, premium_ratio),
        window_minutes: None,
        resets_at: period_end,
        reset_description,
    };

    let org_name = auth.organization.as_ref().and_then(|o| o.name.clone());
    let tier = auth
        .organization
        .as_ref()
        .and_then(|o| o.subscription.as_ref())
        .and_then(|s| s.factory_tier.clone());
    let plan = auth
        .organization
        .as_ref()
        .and_then(|o| o.subscription.as_ref())
        .and_then(|s| s.orb_subscription.as_ref())
        .and_then(|o| o.plan.as_ref())
        .and_then(|p| p.name.clone());

    let login_method = format_login_method(tier.as_deref(), plan.as_deref());
    let identity = ProviderIdentitySnapshot {
        provider_id: Some("factory".to_string()),
        account_email: None,
        account_organization: org_name.clone(),
        login_method: login_method.clone(),
    };

    UsageSnapshot {
        primary: Some(primary),
        secondary: Some(secondary),
        tertiary: None,
        provider_cost: None,
        updated_at: Utc::now(),
        account_email: None,
        account_organization: org_name,
        login_method,
        identity: Some(identity),
    }
}

fn calculate_usage_percent(used: i64, allowance: i64, api_ratio: Option<f64>) -> f64 {
    let unlimited_threshold: i64 = 1_000_000_000_000;
    if let Some(ratio) = api_ratio {
        if let Some(percent) = percent_from_api_ratio(ratio, allowance, unlimited_threshold) {
            return percent;
        }
    }

    if allowance > unlimited_threshold {
        let reference_tokens = 100_000_000_f64;
        return (used as f64 / reference_tokens * 100.0).min(100.0);
    }

    if allowance <= 0 {
        return 0.0;
    }

    (used as f64 / allowance as f64 * 100.0).min(100.0)
}

fn percent_from_api_ratio(ratio: f64, allowance: i64, unlimited_threshold: i64) -> Option<f64> {
    if !ratio.is_finite() {
        return None;
    }

    if (-0.001..=1.001).contains(&ratio) {
        return Some((ratio * 100.0).clamp(0.0, 100.0));
    }

    let allowance_is_reliable = allowance > 0 && allowance <= unlimited_threshold;
    if !allowance_is_reliable && (-0.1..=100.1).contains(&ratio) {
        return Some(ratio.clamp(0.0, 100.0));
    }

    None
}

fn timestamp_millis(ms: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_millis_opt(ms).single()
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

fn format_login_method(tier: Option<&str>, plan: Option<&str>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(tier) = tier.map(|t| t.trim()).filter(|t| !t.is_empty()) {
        parts.push(format!("Factory {}", capitalize_first(tier)));
    }
    if let Some(plan) = plan.map(|p| p.trim()).filter(|p| !p.is_empty()) {
        if !plan.to_lowercase().contains("factory") {
            parts.push(plan.to_string());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" - "))
    }
}

fn capitalize_first(raw: &str) -> String {
    let mut chars = raw.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn extract_access_token(cookie_header: &str) -> Option<String> {
    for part in cookie_header.split(';') {
        let mut iter = part.trim().splitn(2, '=');
        let name = iter.next()?.trim();
        let value = iter.next().unwrap_or("").trim();
        if name == "access-token" && !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn response_snippet(data: &[u8]) -> String {
    let raw = String::from_utf8_lossy(data).trim().to_string();
    if raw.is_empty() {
        "".to_string()
    } else {
        format!(": {}", raw.chars().take(200).collect::<String>())
    }
}
