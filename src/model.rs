use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::Serialize;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPayload {
    pub provider: String,
    pub account: Option<String>,
    pub version: Option<String>,
    pub source: String,
    pub status: Option<ProviderStatusPayload>,
    pub usage: Option<UsageSnapshot>,
    pub credits: Option<CreditsSnapshot>,
    pub antigravity_plan_info: Option<serde_json::Value>,
    pub openai_dashboard: Option<OpenAIDashboardSnapshot>,
    pub error: Option<ProviderErrorPayload>,
}

impl ProviderPayload {
    pub fn error(provider: String, source: String, error: ProviderErrorPayload) -> Self {
        Self {
            provider,
            account: None,
            version: None,
            source,
            status: None,
            usage: None,
            credits: None,
            antigravity_plan_info: None,
            openai_dashboard: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatusPayload {
    pub indicator: ProviderStatusIndicator,
    pub description: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderStatusIndicator {
    None,
    Minor,
    Major,
    Critical,
    Maintenance,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderErrorPayload {
    pub code: i32,
    pub message: String,
    pub kind: Option<ErrorKind>,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
#[serde(rename_all = "lowercase")]
pub enum ErrorKind {
    Args,
    Config,
    Provider,
    Runtime,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RateWindow {
    pub used_percent: f64,
    pub window_minutes: Option<i64>,
    pub resets_at: Option<DateTime<Utc>>,
    pub reset_description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderIdentitySnapshot {
    pub provider_id: Option<String>,
    pub account_email: Option<String>,
    pub account_organization: Option<String>,
    pub login_method: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub primary: Option<RateWindow>,
    pub secondary: Option<RateWindow>,
    pub tertiary: Option<RateWindow>,
    pub provider_cost: Option<ProviderCostSnapshot>,
    pub updated_at: DateTime<Utc>,
    pub identity: Option<ProviderIdentitySnapshot>,
    pub account_email: Option<String>,
    pub account_organization: Option<String>,
    pub login_method: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCostSnapshot {
    pub used: f64,
    pub limit: f64,
    pub currency_code: String,
    pub period: Option<String>,
    pub resets_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditEvent {
    pub id: String,
    pub date: DateTime<Utc>,
    pub service: String,
    pub credits_used: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditsSnapshot {
    pub remaining: f64,
    pub events: Vec<CreditEvent>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAIDashboardSnapshot {
    pub signed_in_email: Option<String>,
    pub code_review_remaining_percent: Option<f64>,
    pub credit_events: Vec<CreditEvent>,
    pub daily_breakdown: Vec<OpenAIDashboardDailyBreakdown>,
    pub usage_breakdown: Vec<OpenAIDashboardDailyBreakdown>,
    pub credits_purchase_url: Option<String>,
    pub primary_limit: Option<RateWindow>,
    pub secondary_limit: Option<RateWindow>,
    pub credits_remaining: Option<f64>,
    pub account_plan: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAIDashboardDailyBreakdown {
    pub day: String,
    pub services: Vec<OpenAIDashboardServiceUsage>,
    pub total_credits_used: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAIDashboardServiceUsage {
    pub service: String,
    pub credits_used: f64,
}
