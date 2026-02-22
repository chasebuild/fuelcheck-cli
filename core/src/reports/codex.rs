use crate::reports::normalize_model_name;
use crate::reports::types::{
    CostReportKind, DailyReportResponse, DailyReportRow, ModelUsage, MonthlyReportResponse,
    MonthlyReportRow, ProviderReport, ReportTotals, SessionReportResponse, SessionReportRow,
};
use anyhow::{Result, anyhow};
use chrono::{DateTime, SecondsFormat, Utc};
use chrono_tz::Tz;
use directories::BaseDirs;
use globwalk::GlobWalkerBuilder;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub struct CodexReportOptions<'a> {
    pub report: CostReportKind,
    pub since: Option<&'a str>,
    pub until: Option<&'a str>,
    pub timezone: Option<&'a str>,
}

#[cfg(test)]
pub(crate) static CODEX_ENV_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Clone)]
struct TokenUsageEvent {
    session_id: String,
    timestamp: DateTime<Utc>,
    model: String,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
    is_fallback_model: bool,
}

#[derive(Debug, Clone, Copy)]
struct RawUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
}

#[derive(Debug, Clone, Copy)]
struct ModelPricing {
    input_cost_per_m_token: f64,
    cached_input_cost_per_m_token: f64,
    output_cost_per_m_token: f64,
}

pub fn build_report(options: &CodexReportOptions<'_>) -> Result<ProviderReport> {
    let timezone = resolve_timezone(options.timezone)?;
    let events = load_token_usage_events()?;

    match options.report {
        CostReportKind::Daily => {
            build_daily_report(&events, options.since, options.until, timezone)
        }
        CostReportKind::Monthly => {
            build_monthly_report(&events, options.since, options.until, timezone)
        }
        CostReportKind::Session => {
            build_session_report(&events, options.since, options.until, timezone)
        }
    }
}

fn build_daily_report(
    events: &[TokenUsageEvent],
    since: Option<&str>,
    until: Option<&str>,
    timezone: Tz,
) -> Result<ProviderReport> {
    let mut summaries: HashMap<String, UsageSummary> = HashMap::new();

    for event in events {
        let date_key = to_date_key(event.timestamp, timezone);
        if !is_within_range(&date_key, since, until) {
            continue;
        }

        let summary = summaries
            .entry(date_key.clone())
            .or_insert_with(UsageSummary::default);
        add_event(summary, event);
    }

    let model_pricing = resolve_model_pricing(&summaries)?;

    let mut keys: Vec<String> = summaries.keys().cloned().collect();
    keys.sort();

    let mut rows = Vec::new();
    let mut totals = ReportTotals::default();

    for key in keys {
        let summary = summaries
            .get(&key)
            .ok_or_else(|| anyhow!("missing daily summary for {}", key))?;
        let cost = calculate_summary_cost(summary, &model_pricing)?;
        let row_models = to_sorted_models(&summary.models);

        let row = DailyReportRow {
            date: key,
            input_tokens: summary.input_tokens,
            cached_input_tokens: summary.cached_input_tokens,
            output_tokens: summary.output_tokens,
            reasoning_output_tokens: summary.reasoning_output_tokens,
            total_tokens: summary.total_tokens,
            cost_usd: cost,
            models: row_models,
        };

        add_row_to_totals(&mut totals, &row);
        rows.push(row);
    }

    Ok(ProviderReport::Daily(DailyReportResponse {
        daily: rows,
        totals,
    }))
}

fn build_monthly_report(
    events: &[TokenUsageEvent],
    since: Option<&str>,
    until: Option<&str>,
    timezone: Tz,
) -> Result<ProviderReport> {
    let mut summaries: HashMap<String, UsageSummary> = HashMap::new();

    for event in events {
        let date_key = to_date_key(event.timestamp, timezone);
        if !is_within_range(&date_key, since, until) {
            continue;
        }

        let month_key = to_month_key(event.timestamp, timezone);
        let summary = summaries
            .entry(month_key.clone())
            .or_insert_with(UsageSummary::default);
        add_event(summary, event);
    }

    let model_pricing = resolve_model_pricing(&summaries)?;

    let mut keys: Vec<String> = summaries.keys().cloned().collect();
    keys.sort();

    let mut rows = Vec::new();
    let mut totals = ReportTotals::default();

    for key in keys {
        let summary = summaries
            .get(&key)
            .ok_or_else(|| anyhow!("missing monthly summary for {}", key))?;
        let cost = calculate_summary_cost(summary, &model_pricing)?;
        let row_models = to_sorted_models(&summary.models);

        let row = MonthlyReportRow {
            month: key,
            input_tokens: summary.input_tokens,
            cached_input_tokens: summary.cached_input_tokens,
            output_tokens: summary.output_tokens,
            reasoning_output_tokens: summary.reasoning_output_tokens,
            total_tokens: summary.total_tokens,
            cost_usd: cost,
            models: row_models,
        };

        totals.input_tokens += row.input_tokens;
        totals.cached_input_tokens += row.cached_input_tokens;
        totals.output_tokens += row.output_tokens;
        totals.reasoning_output_tokens += row.reasoning_output_tokens;
        totals.total_tokens += row.total_tokens;
        totals.cost_usd += row.cost_usd;

        rows.push(row);
    }

    Ok(ProviderReport::Monthly(MonthlyReportResponse {
        monthly: rows,
        totals,
    }))
}

fn build_session_report(
    events: &[TokenUsageEvent],
    since: Option<&str>,
    until: Option<&str>,
    timezone: Tz,
) -> Result<ProviderReport> {
    let mut summaries: HashMap<String, SessionSummary> = HashMap::new();

    for event in events {
        let date_key = to_date_key(event.timestamp, timezone);
        if !is_within_range(&date_key, since, until) {
            continue;
        }

        let summary = summaries
            .entry(event.session_id.clone())
            .or_insert_with(|| SessionSummary {
                usage: UsageSummary::default(),
                last_activity: event.timestamp,
            });

        add_event(&mut summary.usage, event);
        if event.timestamp > summary.last_activity {
            summary.last_activity = event.timestamp;
        }
    }

    let usage_map: HashMap<String, UsageSummary> = summaries
        .iter()
        .map(|(session, summary)| (session.clone(), summary.usage.clone()))
        .collect();
    let model_pricing = resolve_model_pricing(&usage_map)?;

    let mut rows = Vec::new();
    let mut totals = ReportTotals::default();

    let mut ordered: Vec<(&String, &SessionSummary)> = summaries.iter().collect();
    ordered.sort_by_key(|(_, summary)| summary.last_activity);

    for (session_id, summary) in ordered {
        let cost = calculate_summary_cost(&summary.usage, &model_pricing)?;
        let (directory, session_file) = split_session_path(session_id);

        let row = SessionReportRow {
            session_id: session_id.clone(),
            last_activity: summary
                .last_activity
                .to_rfc3339_opts(SecondsFormat::Millis, true),
            session_file,
            directory,
            input_tokens: summary.usage.input_tokens,
            cached_input_tokens: summary.usage.cached_input_tokens,
            output_tokens: summary.usage.output_tokens,
            reasoning_output_tokens: summary.usage.reasoning_output_tokens,
            total_tokens: summary.usage.total_tokens,
            cost_usd: cost,
            models: to_sorted_models(&summary.usage.models),
        };

        totals.input_tokens += row.input_tokens;
        totals.cached_input_tokens += row.cached_input_tokens;
        totals.output_tokens += row.output_tokens;
        totals.reasoning_output_tokens += row.reasoning_output_tokens;
        totals.total_tokens += row.total_tokens;
        totals.cost_usd += row.cost_usd;

        rows.push(row);
    }

    Ok(ProviderReport::Session(SessionReportResponse {
        sessions: rows,
        totals,
    }))
}

#[derive(Debug, Clone, Default)]
struct UsageSummary {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
    models: HashMap<String, ModelUsage>,
}

#[derive(Debug, Clone)]
struct SessionSummary {
    usage: UsageSummary,
    last_activity: DateTime<Utc>,
}

fn add_event(summary: &mut UsageSummary, event: &TokenUsageEvent) {
    summary.input_tokens += event.input_tokens;
    summary.cached_input_tokens += event.cached_input_tokens;
    summary.output_tokens += event.output_tokens;
    summary.reasoning_output_tokens += event.reasoning_output_tokens;
    summary.total_tokens += event.total_tokens;

    let model_usage = summary
        .models
        .entry(event.model.clone())
        .or_insert_with(ModelUsage::default);
    model_usage.input_tokens += event.input_tokens;
    model_usage.cached_input_tokens += event.cached_input_tokens;
    model_usage.output_tokens += event.output_tokens;
    model_usage.reasoning_output_tokens += event.reasoning_output_tokens;
    model_usage.total_tokens += event.total_tokens;
    if event.is_fallback_model {
        model_usage.is_fallback = Some(true);
    }
}

fn to_sorted_models(models: &HashMap<String, ModelUsage>) -> BTreeMap<String, ModelUsage> {
    let mut sorted = BTreeMap::new();
    for (name, usage) in models {
        sorted.insert(name.clone(), usage.clone());
    }
    sorted
}

fn add_row_to_totals(totals: &mut ReportTotals, row: &DailyReportRow) {
    totals.input_tokens += row.input_tokens;
    totals.cached_input_tokens += row.cached_input_tokens;
    totals.output_tokens += row.output_tokens;
    totals.reasoning_output_tokens += row.reasoning_output_tokens;
    totals.total_tokens += row.total_tokens;
    totals.cost_usd += row.cost_usd;
}

fn resolve_model_pricing(
    summaries: &HashMap<String, UsageSummary>,
) -> Result<HashMap<String, ModelPricing>> {
    let mut models = HashSet::new();
    for summary in summaries.values() {
        for model in summary.models.keys() {
            models.insert(model.clone());
        }
    }

    let mut pricing = HashMap::new();
    for model in models {
        pricing.insert(model.clone(), resolve_model_pricing_entry(&model)?);
    }

    Ok(pricing)
}

fn calculate_summary_cost(
    summary: &UsageSummary,
    model_pricing: &HashMap<String, ModelPricing>,
) -> Result<f64> {
    let mut cost = 0.0;

    for (model, usage) in &summary.models {
        let pricing = model_pricing
            .get(model)
            .ok_or_else(|| anyhow!("pricing not found for model {}", model))?;
        cost += calculate_usage_cost(usage, *pricing);
    }

    Ok(cost)
}

fn calculate_usage_cost(usage: &ModelUsage, pricing: ModelPricing) -> f64 {
    let non_cached_input = usage.input_tokens.saturating_sub(usage.cached_input_tokens);
    let cached_input = usage.cached_input_tokens.min(usage.input_tokens);

    let input_cost = (non_cached_input as f64 / 1_000_000.0) * pricing.input_cost_per_m_token;
    let cached_cost = (cached_input as f64 / 1_000_000.0) * pricing.cached_input_cost_per_m_token;
    let output_cost = (usage.output_tokens as f64 / 1_000_000.0) * pricing.output_cost_per_m_token;

    input_cost + cached_cost + output_cost
}

fn resolve_model_pricing_entry(model: &str) -> Result<ModelPricing> {
    let canonical = canonicalize_model_name(model);

    let pricing = match canonical.as_str() {
        "gpt-5" => ModelPricing {
            input_cost_per_m_token: 1.25,
            cached_input_cost_per_m_token: 0.125,
            output_cost_per_m_token: 10.0,
        },
        "gpt-5-mini" => ModelPricing {
            input_cost_per_m_token: 0.6,
            cached_input_cost_per_m_token: 0.06,
            output_cost_per_m_token: 2.0,
        },
        "gpt-5-nano" => ModelPricing {
            input_cost_per_m_token: 0.2,
            cached_input_cost_per_m_token: 0.02,
            output_cost_per_m_token: 0.8,
        },
        _ => {
            return Err(anyhow!("pricing not found for model {}", model));
        }
    };

    Ok(pricing)
}

fn canonicalize_model_name(model: &str) -> String {
    let normalized = normalize_model_name(model);
    if normalized == "gpt-5-codex" {
        return "gpt-5".to_string();
    }
    if normalized.starts_with("gpt-5-mini") {
        return "gpt-5-mini".to_string();
    }
    if normalized.starts_with("gpt-5-nano") {
        return "gpt-5-nano".to_string();
    }
    if normalized.starts_with("gpt-5") {
        return "gpt-5".to_string();
    }
    normalized
}

fn to_date_key(timestamp: DateTime<Utc>, timezone: Tz) -> String {
    timestamp
        .with_timezone(&timezone)
        .format("%Y-%m-%d")
        .to_string()
}

fn to_month_key(timestamp: DateTime<Utc>, timezone: Tz) -> String {
    timestamp
        .with_timezone(&timezone)
        .format("%Y-%m")
        .to_string()
}

fn is_within_range(date_key: &str, since: Option<&str>, until: Option<&str>) -> bool {
    let value = date_key.replace('-', "");
    let since_value = since.map(|v| v.replace('-', ""));
    let until_value = until.map(|v| v.replace('-', ""));

    if let Some(since_value) = since_value
        && value < since_value
    {
        return false;
    }
    if let Some(until_value) = until_value
        && value > until_value
    {
        return false;
    }
    true
}

fn resolve_timezone(raw: Option<&str>) -> Result<Tz> {
    if let Some(value) = raw {
        return value
            .trim()
            .parse::<Tz>()
            .map_err(|_| anyhow!("invalid timezone: {}", value));
    }

    if let Ok(value) = std::env::var("TZ") {
        let trimmed = value.trim();
        if !trimmed.is_empty()
            && let Ok(timezone) = trimmed.parse::<Tz>()
        {
            return Ok(timezone);
        }
    }

    Ok(chrono_tz::UTC)
}

fn split_session_path(session_id: &str) -> (String, String) {
    if let Some(index) = session_id.rfind('/') {
        (
            session_id[..index].to_string(),
            session_id[index + 1..].to_string(),
        )
    } else {
        (String::new(), session_id.to_string())
    }
}

fn load_token_usage_events() -> Result<Vec<TokenUsageEvent>> {
    let sessions_dir = codex_sessions_dir()?;
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let walker = GlobWalkerBuilder::from_patterns(&sessions_dir, &["**/*.jsonl"])
        .build()
        .map_err(|err| anyhow!("failed to scan codex sessions: {}", err))?;

    let mut events = Vec::new();
    for entry in walker.flatten() {
        let path = entry.path();
        let mut file_events = parse_events_from_file(path, &sessions_dir)?;
        events.append(&mut file_events);
    }

    events.sort_by_key(|event| event.timestamp);
    Ok(events)
}

fn codex_sessions_dir() -> Result<PathBuf> {
    let codex_home = std::env::var("CODEX_HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| BaseDirs::new().map(|dirs| dirs.home_dir().join(".codex")))
        .ok_or_else(|| anyhow!("unable to resolve CODEX_HOME"))?;

    Ok(codex_home.join("sessions"))
}

fn parse_events_from_file(path: &Path, sessions_dir: &Path) -> Result<Vec<TokenUsageEvent>> {
    let file = File::open(path).map_err(|err| anyhow!("read {}: {}", path.display(), err))?;
    let reader = BufReader::new(file);
    let session_id = session_id_from_path(path, sessions_dir);

    let mut events = Vec::new();
    let mut previous_totals: Option<RawUsage> = None;
    let mut current_model: Option<String> = None;
    let mut current_model_is_fallback = false;

    for line in reader.lines() {
        let line = match line {
            Ok(value) => value,
            Err(_) => continue,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parsed: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(_) => continue,
        };

        let entry_type = parsed
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let payload = parsed.get("payload");

        if entry_type == "turn_context" {
            if let Some(model) = payload.and_then(extract_model) {
                current_model = Some(model);
                current_model_is_fallback = false;
            }
            continue;
        }

        if entry_type != "event_msg" {
            continue;
        }

        let Some(payload) = payload else {
            continue;
        };

        if payload.get("type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }

        let timestamp_raw = match parsed.get("timestamp").and_then(Value::as_str) {
            Some(value) => value,
            None => continue,
        };
        let timestamp = match DateTime::parse_from_rfc3339(timestamp_raw) {
            Ok(value) => value.with_timezone(&Utc),
            Err(_) => continue,
        };

        let info = payload.get("info");
        let last_usage = normalize_raw_usage(info.and_then(|value| value.get("last_token_usage")));
        let total_usage =
            normalize_raw_usage(info.and_then(|value| value.get("total_token_usage")));

        let raw_usage = if let Some(last_usage) = last_usage {
            Some(last_usage)
        } else if let Some(total_usage) = total_usage {
            Some(subtract_raw_usage(total_usage, previous_totals))
        } else {
            None
        };

        if let Some(total_usage) = total_usage {
            previous_totals = Some(total_usage);
        }

        let Some(raw_usage) = raw_usage else {
            continue;
        };

        let delta = convert_to_delta(raw_usage);
        if delta.input_tokens == 0
            && delta.cached_input_tokens == 0
            && delta.output_tokens == 0
            && delta.reasoning_output_tokens == 0
        {
            continue;
        }

        let extracted_model = extract_model(payload).or_else(|| info.and_then(extract_model));
        if let Some(model) = extracted_model.clone() {
            current_model = Some(model);
            current_model_is_fallback = false;
        }

        let (model, is_fallback_model) = if let Some(model) = extracted_model {
            (model, false)
        } else if let Some(model) = current_model.clone() {
            (model, current_model_is_fallback)
        } else {
            current_model_is_fallback = true;
            let fallback = "gpt-5".to_string();
            current_model = Some(fallback.clone());
            (fallback, true)
        };

        events.push(TokenUsageEvent {
            session_id: session_id.clone(),
            timestamp,
            model,
            input_tokens: delta.input_tokens,
            cached_input_tokens: delta.cached_input_tokens,
            output_tokens: delta.output_tokens,
            reasoning_output_tokens: delta.reasoning_output_tokens,
            total_tokens: delta.total_tokens,
            is_fallback_model,
        });
    }

    Ok(events)
}

fn session_id_from_path(path: &Path, sessions_dir: &Path) -> String {
    let relative = path.strip_prefix(sessions_dir).unwrap_or(path);
    let mut session_id = relative.to_string_lossy().replace('\\', "/");
    if let Some(stripped) = session_id.strip_suffix(".jsonl") {
        session_id = stripped.to_string();
    }
    session_id
}

fn normalize_raw_usage(value: Option<&Value>) -> Option<RawUsage> {
    let record = value?.as_object()?;
    let input_tokens = ensure_u64(record.get("input_tokens"));
    let cached_input_tokens = ensure_u64(
        record
            .get("cached_input_tokens")
            .or_else(|| record.get("cache_read_input_tokens")),
    );
    let output_tokens = ensure_u64(record.get("output_tokens"));
    let reasoning_output_tokens = ensure_u64(record.get("reasoning_output_tokens"));
    let total_tokens = ensure_u64(record.get("total_tokens"));
    let total_tokens = if total_tokens > 0 {
        total_tokens
    } else {
        input_tokens.saturating_add(output_tokens)
    };

    Some(RawUsage {
        input_tokens,
        cached_input_tokens,
        output_tokens,
        reasoning_output_tokens,
        total_tokens,
    })
}

fn subtract_raw_usage(current: RawUsage, previous: Option<RawUsage>) -> RawUsage {
    let previous = previous.unwrap_or(RawUsage {
        input_tokens: 0,
        cached_input_tokens: 0,
        output_tokens: 0,
        reasoning_output_tokens: 0,
        total_tokens: 0,
    });

    RawUsage {
        input_tokens: current.input_tokens.saturating_sub(previous.input_tokens),
        cached_input_tokens: current
            .cached_input_tokens
            .saturating_sub(previous.cached_input_tokens),
        output_tokens: current.output_tokens.saturating_sub(previous.output_tokens),
        reasoning_output_tokens: current
            .reasoning_output_tokens
            .saturating_sub(previous.reasoning_output_tokens),
        total_tokens: current.total_tokens.saturating_sub(previous.total_tokens),
    }
}

fn convert_to_delta(raw: RawUsage) -> RawUsage {
    let total_tokens = if raw.total_tokens > 0 {
        raw.total_tokens
    } else {
        raw.input_tokens.saturating_add(raw.output_tokens)
    };

    RawUsage {
        input_tokens: raw.input_tokens,
        cached_input_tokens: raw.cached_input_tokens.min(raw.input_tokens),
        output_tokens: raw.output_tokens,
        reasoning_output_tokens: raw.reasoning_output_tokens,
        total_tokens,
    }
}

fn ensure_u64(value: Option<&Value>) -> u64 {
    let Some(value) = value else {
        return 0;
    };

    match value {
        Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                value
            } else if let Some(value) = number.as_i64() {
                value.max(0) as u64
            } else {
                number.as_f64().unwrap_or(0.0).max(0.0) as u64
            }
        }
        Value::String(raw) => raw.trim().parse::<u64>().unwrap_or(0),
        _ => 0,
    }
}

fn extract_model(value: &Value) -> Option<String> {
    let object = value.as_object()?;

    if let Some(model) = object.get("model").and_then(as_non_empty_string) {
        return Some(model);
    }
    if let Some(model_name) = object.get("model_name").and_then(as_non_empty_string) {
        return Some(model_name);
    }

    if let Some(info) = object.get("info")
        && let Some(model) = extract_model(info)
    {
        return Some(model);
    }

    if let Some(metadata) = object.get("metadata")
        && let Some(model) = metadata
            .as_object()
            .and_then(|meta| meta.get("model"))
            .and_then(as_non_empty_string)
    {
        return Some(model);
    }

    None
}

fn as_non_empty_string(value: &Value) -> Option<String> {
    let value = value.as_str()?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reports::types::ProviderReport;
    use std::fs;

    struct EnvVarGuard {
        key: String,
        prev: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            // SAFETY: tests run in a controlled process and this key is restored on Drop.
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key: key.to_string(),
                prev,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(value) => {
                    // SAFETY: restoring env var for this process in test teardown.
                    unsafe {
                        std::env::set_var(&self.key, value);
                    }
                }
                None => {
                    // SAFETY: restoring env var for this process in test teardown.
                    unsafe {
                        std::env::remove_var(&self.key);
                    }
                }
            }
        }
    }

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("fuelcheck-codex-report-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_session_file(base: &Path, relative: &str, content: &str) {
        let path = base.join("sessions").join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(path, content).expect("write session file");
    }

    #[test]
    fn parses_turn_context_and_token_events() {
        let _lock = CODEX_ENV_TEST_MUTEX.lock().expect("lock env mutex");
        let temp = TempDirGuard::new();
        write_session_file(
            temp.path(),
            "project-a.jsonl",
            &[
                r#"{"timestamp":"2025-09-11T18:25:30.000Z","type":"turn_context","payload":{"model":"gpt-5"}}"#,
                r#"{"timestamp":"2025-09-11T18:25:40.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1200,"cached_input_tokens":200,"output_tokens":500,"reasoning_output_tokens":0,"total_tokens":1700},"total_token_usage":{"input_tokens":1200,"cached_input_tokens":200,"output_tokens":500,"reasoning_output_tokens":0,"total_tokens":1700}}}}"#,
                r#"{"timestamp":"2025-09-11T20:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2000,"cached_input_tokens":300,"output_tokens":800,"reasoning_output_tokens":0,"total_tokens":2800}}}}"#,
            ]
            .join("\n"),
        );

        let _guard = EnvVarGuard::set("CODEX_HOME", &temp.path().display().to_string());

        let report = build_report(&CodexReportOptions {
            report: CostReportKind::Daily,
            since: None,
            until: None,
            timezone: Some("UTC"),
        })
        .expect("build report");

        let ProviderReport::Daily(data) = report else {
            panic!("expected daily report");
        };

        assert_eq!(data.daily.len(), 1);
        assert_eq!(data.daily[0].input_tokens, 2000);
        assert_eq!(data.daily[0].cached_input_tokens, 300);
    }

    #[test]
    fn applies_fallback_model_for_legacy_sessions() {
        let _lock = CODEX_ENV_TEST_MUTEX.lock().expect("lock env mutex");
        let temp = TempDirGuard::new();
        write_session_file(
            temp.path(),
            "legacy.jsonl",
            r#"{"timestamp":"2025-09-15T13:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":5000,"cached_input_tokens":0,"output_tokens":1000,"reasoning_output_tokens":0,"total_tokens":6000}}}}"#,
        );

        let _guard = EnvVarGuard::set("CODEX_HOME", &temp.path().display().to_string());

        let report = build_report(&CodexReportOptions {
            report: CostReportKind::Daily,
            since: None,
            until: None,
            timezone: Some("UTC"),
        })
        .expect("build report");

        let ProviderReport::Daily(data) = report else {
            panic!("expected daily report");
        };

        let row = &data.daily[0];
        let usage = row.models.get("gpt-5").expect("fallback model");
        assert_eq!(usage.is_fallback, Some(true));
    }

    #[test]
    fn filters_by_range_and_timezone() {
        let _lock = CODEX_ENV_TEST_MUTEX.lock().expect("lock env mutex");
        let temp = TempDirGuard::new();
        write_session_file(
            temp.path(),
            "timezone.jsonl",
            &[
                r#"{"timestamp":"2025-09-11T23:30:00.000Z","type":"turn_context","payload":{"model":"gpt-5"}}"#,
                r#"{"timestamp":"2025-09-11T23:31:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":50,"reasoning_output_tokens":0,"total_tokens":150}}}}"#,
                r#"{"timestamp":"2025-09-12T00:10:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":200,"cached_input_tokens":0,"output_tokens":60,"reasoning_output_tokens":0,"total_tokens":260}}}}"#,
            ]
            .join("\n"),
        );

        let _guard = EnvVarGuard::set("CODEX_HOME", &temp.path().display().to_string());

        let report = build_report(&CodexReportOptions {
            report: CostReportKind::Daily,
            since: Some("2025-09-11"),
            until: Some("2025-09-11"),
            timezone: Some("America/Los_Angeles"),
        })
        .expect("build report");

        let ProviderReport::Daily(data) = report else {
            panic!("expected daily report");
        };

        assert_eq!(data.daily.len(), 1);
        assert_eq!(data.daily[0].date, "2025-09-11");
        assert_eq!(data.daily[0].input_tokens, 300);
    }

    #[test]
    fn unknown_model_returns_error() {
        let _lock = CODEX_ENV_TEST_MUTEX.lock().expect("lock env mutex");
        let temp = TempDirGuard::new();
        write_session_file(
            temp.path(),
            "unknown-model.jsonl",
            &[
                r#"{"timestamp":"2025-09-11T10:00:00.000Z","type":"turn_context","payload":{"model":"mystery-model"}}"#,
                r#"{"timestamp":"2025-09-11T10:00:10.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":0,"total_tokens":110}}}}"#,
            ]
            .join("\n"),
        );

        let _guard = EnvVarGuard::set("CODEX_HOME", &temp.path().display().to_string());

        let err = build_report(&CodexReportOptions {
            report: CostReportKind::Daily,
            since: None,
            until: None,
            timezone: Some("UTC"),
        })
        .expect_err("expected pricing error");

        assert!(
            err.to_string()
                .contains("pricing not found for model mystery-model")
        );
    }
}
