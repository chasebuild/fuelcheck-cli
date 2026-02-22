use chrono::{DateTime, Utc};
use serde_json::Value;

pub fn env_var_nonempty(names: &[&str]) -> Option<String> {
    for name in names {
        if let Ok(value) = std::env::var(name) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

pub fn parse_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub fn parse_epoch(value: i64) -> Option<DateTime<Utc>> {
    if value <= 0 {
        return None;
    }
    if value > 1_000_000_000_000 {
        DateTime::<Utc>::from_timestamp(value / 1000, 0)
    } else {
        DateTime::<Utc>::from_timestamp(value, 0)
    }
}

pub fn parse_epoch_f64(value: f64) -> Option<DateTime<Utc>> {
    if value <= 0.0 {
        return None;
    }
    let as_i64 = value.round() as i64;
    parse_epoch(as_i64)
}

pub fn used_percent_from(used: Option<f64>, limit: Option<f64>) -> Option<f64> {
    let used = used?;
    let limit = limit?;
    if limit <= 0.0 {
        return None;
    }
    Some((used / limit) * 100.0)
}

pub fn used_percent_from_remaining(remaining: Option<f64>, limit: Option<f64>) -> Option<f64> {
    let remaining = remaining?;
    let limit = limit?;
    if limit <= 0.0 {
        return None;
    }
    Some(((limit - remaining) / limit) * 100.0)
}

pub fn value_to_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(num) => num.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

pub fn value_to_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(num) => num.as_i64().or_else(|| num.as_f64().map(|v| v as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

pub fn normalize_host(host: &str) -> String {
    let trimmed = host.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed)
    }
}
