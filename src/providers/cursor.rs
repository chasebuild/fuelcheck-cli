use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{
    ProviderCostSnapshot, ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot,
};
use crate::providers::{Provider, ProviderId, SourcePreference};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

pub struct CursorProvider;

#[async_trait]
impl Provider for CursorProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Cursor
    }

    fn version(&self) -> &'static str {
        "2024-08-01"
    }

    async fn fetch_usage(
        &self,
        _args: &UsageArgs,
        config: &Config,
        source: SourcePreference,
    ) -> Result<ProviderPayload> {
        let cfg = config.provider_config(self.id());
        let cookie_header = cfg
            .as_ref()
            .and_then(|c| c.cookie_header.clone())
            .or_else(|| std::env::var("CURSOR_COOKIE").ok())
            .ok_or_else(|| anyhow!("Cursor cookie header missing. Set provider cookie_header in config."))?;

        let selected = match source {
            SourcePreference::Auto => SourcePreference::Web,
            other => other,
        };

        match selected {
            SourcePreference::Web | SourcePreference::Api => {
                let usage = fetch_cursor_usage(&cookie_header).await?;
                Ok(self.ok_output("web", Some(usage)))
            }
            _ => Err(CliError::UnsupportedSource(self.id(), selected.to_string()).into()),
        }
    }
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CursorUsageSummary {
    #[serde(rename = "billingCycleStart")]
    billing_cycle_start: Option<String>,
    #[serde(rename = "billingCycleEnd")]
    billing_cycle_end: Option<String>,
    #[serde(rename = "membershipType")]
    membership_type: Option<String>,
    #[serde(rename = "limitType")]
    limit_type: Option<String>,
    #[serde(rename = "isUnlimited")]
    is_unlimited: Option<bool>,
    #[serde(rename = "autoModelSelectedDisplayMessage")]
    auto_model_selected_display_message: Option<String>,
    #[serde(rename = "namedModelSelectedDisplayMessage")]
    named_model_selected_display_message: Option<String>,
    #[serde(rename = "individualUsage")]
    individual_usage: Option<CursorIndividualUsage>,
    #[serde(rename = "teamUsage")]
    team_usage: Option<CursorTeamUsage>,
}

#[derive(Debug, Deserialize)]
struct CursorIndividualUsage {
    plan: Option<CursorPlanUsage>,
    #[serde(rename = "onDemand")]
    on_demand: Option<CursorOnDemandUsage>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CursorPlanUsage {
    enabled: Option<bool>,
    used: Option<i64>,
    limit: Option<i64>,
    remaining: Option<i64>,
    #[serde(rename = "autoPercentUsed")]
    auto_percent_used: Option<f64>,
    #[serde(rename = "apiPercentUsed")]
    api_percent_used: Option<f64>,
    #[serde(rename = "totalPercentUsed")]
    total_percent_used: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CursorOnDemandUsage {
    enabled: Option<bool>,
    used: Option<i64>,
    limit: Option<i64>,
    remaining: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CursorTeamUsage {
    #[serde(rename = "onDemand")]
    on_demand: Option<CursorOnDemandUsage>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CursorUserInfo {
    email: Option<String>,
    name: Option<String>,
    sub: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CursorUsageResponse {
    #[serde(rename = "gpt-4")]
    gpt4: Option<CursorModelUsage>,
    #[serde(rename = "startOfMonth")]
    start_of_month: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CursorModelUsage {
    #[serde(rename = "numRequests")]
    num_requests: Option<i64>,
    #[serde(rename = "numRequestsTotal")]
    num_requests_total: Option<i64>,
    #[serde(rename = "numTokens")]
    num_tokens: Option<i64>,
    #[serde(rename = "maxRequestUsage")]
    max_request_usage: Option<i64>,
    #[serde(rename = "maxTokenUsage")]
    max_token_usage: Option<i64>,
}

async fn fetch_cursor_usage(cookie_header: &str) -> Result<UsageSnapshot> {
    let (summary, _raw) = fetch_usage_summary(cookie_header).await?;
    let user_info = fetch_user_info(cookie_header).await.ok();
    let request_usage = if let Some(user) = &user_info {
        if let Some(sub) = &user.sub {
            fetch_request_usage(sub, cookie_header).await.ok()
        } else {
            None
        }
    } else {
        None
    };

    let billing_cycle_end = summary.billing_cycle_end.as_ref().and_then(parse_iso8601);

    let plan_used_raw = summary
        .individual_usage
        .as_ref()
        .and_then(|u| u.plan.as_ref())
        .and_then(|p| p.used)
        .unwrap_or(0) as f64;
    let plan_limit_raw = summary
        .individual_usage
        .as_ref()
        .and_then(|u| u.plan.as_ref())
        .and_then(|p| p.limit)
        .unwrap_or(0) as f64;

    let plan_percent_used = if plan_limit_raw > 0.0 {
        (plan_used_raw / plan_limit_raw) * 100.0
    } else {
        summary
            .individual_usage
            .as_ref()
            .and_then(|u| u.plan.as_ref())
            .and_then(|p| p.total_percent_used)
            .map(|v| if v <= 1.0 { v * 100.0 } else { v })
            .unwrap_or(0.0)
    };

    let on_demand_used = summary
        .individual_usage
        .as_ref()
        .and_then(|u| u.on_demand.as_ref())
        .and_then(|o| o.used)
        .unwrap_or(0) as f64
        / 100.0;
    let on_demand_limit = summary
        .individual_usage
        .as_ref()
        .and_then(|u| u.on_demand.as_ref())
        .and_then(|o| o.limit)
        .map(|v| v as f64 / 100.0);

    let _team_on_demand_used = summary
        .team_usage
        .as_ref()
        .and_then(|t| t.on_demand.as_ref())
        .and_then(|o| o.used)
        .map(|v| v as f64 / 100.0);
    let _team_on_demand_limit = summary
        .team_usage
        .as_ref()
        .and_then(|t| t.on_demand.as_ref())
        .and_then(|o| o.limit)
        .map(|v| v as f64 / 100.0);

    let requests_used = request_usage
        .as_ref()
        .and_then(|r| r.gpt4.as_ref())
        .and_then(|g| g.num_requests_total.or(g.num_requests));
    let requests_limit = request_usage
        .as_ref()
        .and_then(|r| r.gpt4.as_ref())
        .and_then(|g| g.max_request_usage);

    let primary_used_percent = if let (Some(used), Some(limit)) = (requests_used, requests_limit) {
        if limit > 0 {
            (used as f64 / limit as f64) * 100.0
        } else {
            plan_percent_used
        }
    } else {
        plan_percent_used
    };

    let primary = RateWindow {
        used_percent: primary_used_percent,
        window_minutes: Some(30 * 24 * 60),
        resets_at: billing_cycle_end,
        reset_description: billing_cycle_end.map(format_reset_description),
    };

    let provider_cost = if on_demand_used > 0.0 || on_demand_limit.is_some() {
        Some(ProviderCostSnapshot {
            used: on_demand_used,
            limit: on_demand_limit.unwrap_or(0.0),
            currency_code: "USD".to_string(),
            period: Some("Monthly".to_string()),
            resets_at: billing_cycle_end,
            updated_at: Utc::now(),
        })
    } else {
        None
    };

    let identity = ProviderIdentitySnapshot {
        provider_id: Some("cursor".to_string()),
        account_email: user_info.as_ref().and_then(|u| u.email.clone()),
        account_organization: None,
        login_method: summary.membership_type.clone(),
    };

    Ok(UsageSnapshot {
        primary: Some(primary),
        secondary: None,
        tertiary: None,
        provider_cost,
        updated_at: Utc::now(),
        account_email: identity.account_email.clone(),
        account_organization: None,
        login_method: identity.login_method.clone(),
        identity: Some(identity),
    })
}

async fn fetch_usage_summary(cookie_header: &str) -> Result<(CursorUsageSummary, String)> {
    let url = "https://cursor.com/api/usage-summary";
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("Accept", "application/json")
        .header("Cookie", cookie_header)
        .send()
        .await?;
    let status = resp.status();
    let data = resp.bytes().await?;
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(anyhow!("Cursor not logged in. Update cookie header."));
    }
    if !status.is_success() {
        return Err(anyhow!("Cursor API error (HTTP {})", status.as_u16()));
    }
    let raw = String::from_utf8_lossy(&data).to_string();
    let summary: CursorUsageSummary = serde_json::from_slice(&data)?;
    Ok((summary, raw))
}

async fn fetch_user_info(cookie_header: &str) -> Result<CursorUserInfo> {
    let url = "https://cursor.com/api/auth/me";
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("Accept", "application/json")
        .header("Cookie", cookie_header)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("Cursor user info fetch failed"));
    }
    let data = resp.bytes().await?;
    Ok(serde_json::from_slice(&data)?)
}

async fn fetch_request_usage(user_id: &str, cookie_header: &str) -> Result<CursorUsageResponse> {
    let url = format!("https://cursor.com/api/usage?user={}", user_id);
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("Accept", "application/json")
        .header("Cookie", cookie_header)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("Cursor request usage fetch failed"));
    }
    let data = resp.bytes().await?;
    Ok(serde_json::from_slice(&data)?)
}

fn parse_iso8601(raw: &String) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
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
