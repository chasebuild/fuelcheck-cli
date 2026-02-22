use crate::model::ProviderErrorPayload;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CostReportKind {
    Daily,
    Monthly,
    Session,
}

impl fmt::Display for CostReportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Daily => "daily",
            Self::Monthly => "monthly",
            Self::Session => "session",
        };
        write!(f, "{}", value)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_fallback: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportTotals {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    #[serde(rename = "costUSD")]
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyReportRow {
    pub date: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    #[serde(rename = "costUSD")]
    pub cost_usd: f64,
    pub models: BTreeMap<String, ModelUsage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyReportRow {
    pub month: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    #[serde(rename = "costUSD")]
    pub cost_usd: f64,
    pub models: BTreeMap<String, ModelUsage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionReportRow {
    pub session_id: String,
    pub last_activity: String,
    pub session_file: String,
    pub directory: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
    #[serde(rename = "costUSD")]
    pub cost_usd: f64,
    pub models: BTreeMap<String, ModelUsage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DailyReportResponse {
    pub daily: Vec<DailyReportRow>,
    pub totals: ReportTotals,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthlyReportResponse {
    pub monthly: Vec<MonthlyReportRow>,
    pub totals: ReportTotals,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionReportResponse {
    pub sessions: Vec<SessionReportRow>,
    pub totals: ReportTotals,
}

#[derive(Debug, Clone)]
pub enum ProviderReport {
    Daily(DailyReportResponse),
    Monthly(MonthlyReportResponse),
    Session(SessionReportResponse),
}

impl ProviderReport {
    pub fn kind(&self) -> CostReportKind {
        match self {
            Self::Daily(_) => CostReportKind::Daily,
            Self::Monthly(_) => CostReportKind::Monthly,
            Self::Session(_) => CostReportKind::Session,
        }
    }
}

impl Serialize for ProviderReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Daily(data) => data.serialize(serializer),
            Self::Monthly(data) => data.serialize(serializer),
            Self::Session(data) => data.serialize(serializer),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ProviderReportOutcome {
    Report(ProviderReport),
    Error(ProviderErrorPayload),
}

#[derive(Debug, Clone)]
pub struct ProviderReportResult {
    pub provider: String,
    pub outcome: ProviderReportOutcome,
}

#[derive(Debug, Clone)]
pub struct CostReportCollection {
    pub report: CostReportKind,
    pub providers: Vec<ProviderReportResult>,
}

#[derive(Debug, Clone, Copy)]
pub struct SplitUsageTokens {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
}

pub fn split_usage_tokens(
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
) -> SplitUsageTokens {
    let cache_read_tokens = cached_input_tokens.min(input_tokens);
    let input_tokens = input_tokens.saturating_sub(cache_read_tokens);
    let reasoning_tokens = reasoning_output_tokens.min(output_tokens);

    SplitUsageTokens {
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_read_tokens,
    }
}
