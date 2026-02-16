use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot};
use crate::providers::{Provider, ProviderId, SourcePreference, env_var_nonempty};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::Utc;
use regex::Regex;
use serde_json::Value;
use uuid::Uuid;

pub struct OpenCodeProvider;

#[async_trait]
impl Provider for OpenCodeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::OpenCode
    }

    fn version(&self) -> &'static str {
        "2025-01-01"
    }

    async fn fetch_usage(
        &self,
        args: &UsageArgs,
        config: &Config,
        source: SourcePreference,
    ) -> Result<ProviderPayload> {
        let selected = match source {
            SourcePreference::Auto => SourcePreference::Web,
            other => other,
        };
        if selected != SourcePreference::Web {
            return Err(CliError::UnsupportedSource(self.id(), selected.to_string()).into());
        }

        let cfg = config.provider_config(self.id());
        let cookie = cfg
            .as_ref()
            .and_then(|c| c.cookie_header.clone())
            .or_else(|| env_var_nonempty(&["OPENCODE_COOKIE", "OPENCODE_COOKIE_HEADER"]))
            .ok_or_else(|| {
                anyhow!("OpenCode cookie header missing. Set provider cookie_header.")
            })?;
        let workspace_override = cfg
            .as_ref()
            .and_then(|c| c.workspace_id.clone())
            .or_else(|| env_var_nonempty(&["CODEXBAR_OPENCODE_WORKSPACE_ID"]));

        let workspace_id = if let Some(id) = workspace_override.and_then(normalize_workspace_id) {
            id
        } else {
            fetch_workspace_id(&cookie, args.web_timeout).await?
        };

        let subscription_text =
            fetch_subscription(&workspace_id, &cookie, args.web_timeout).await?;
        let usage = parse_opencode_usage(&subscription_text)?;
        Ok(self.ok_output("web", Some(usage)))
    }
}

const WORKSPACES_SERVER_ID: &str =
    "def39973159c7f0483d8793a822b8dbb10d067e12c65455fcb4608459ba0234f";
const SUBSCRIPTION_SERVER_ID: &str =
    "7abeebee372f304e050aaaf92be863f4a86490e382f8c79db68fd94040d691b4";

async fn fetch_workspace_id(cookie: &str, timeout: u64) -> Result<String> {
    let base_url = "https://opencode.ai";
    let text = fetch_server_text(
        base_url,
        WORKSPACES_SERVER_ID,
        "GET",
        None,
        cookie,
        timeout,
        base_url,
    )
    .await?;
    if let Some(id) = parse_workspace_id(&text) {
        return Ok(id);
    }
    let text = fetch_server_text(
        base_url,
        WORKSPACES_SERVER_ID,
        "POST",
        Some(&serde_json::json!([])),
        cookie,
        timeout,
        base_url,
    )
    .await?;
    parse_workspace_id(&text).ok_or_else(|| anyhow!("OpenCode workspace id missing"))
}

async fn fetch_subscription(workspace_id: &str, cookie: &str, timeout: u64) -> Result<String> {
    let base_url = "https://opencode.ai";
    let referer = format!("{}/workspace/{}/billing", base_url, workspace_id);
    let args = serde_json::json!([workspace_id]);
    let text = fetch_server_text(
        base_url,
        SUBSCRIPTION_SERVER_ID,
        "GET",
        Some(&args),
        cookie,
        timeout,
        &referer,
    )
    .await?;
    if parse_opencode_usage(&text).is_ok() {
        return Ok(text);
    }
    let text = fetch_server_text(
        base_url,
        SUBSCRIPTION_SERVER_ID,
        "POST",
        Some(&args),
        cookie,
        timeout,
        &referer,
    )
    .await?;
    Ok(text)
}

async fn fetch_server_text(
    base_url: &str,
    server_id: &str,
    method: &str,
    args: Option<&Value>,
    cookie: &str,
    timeout: u64,
    referer: &str,
) -> Result<String> {
    let url = server_request_url(base_url, server_id, args, method);
    let client = reqwest::Client::new();
    let mut req = match method {
        "POST" => client.post(url),
        _ => client.get(url),
    };
    req = req
        .header("cookie", cookie)
        .header("x-server-id", server_id)
        .header("x-server-instance", format!("server-fn:{}", Uuid::new_v4()))
        .header(
            "user-agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36",
        )
        .header("origin", base_url)
        .header("referer", referer)
        .header("accept", "text/javascript, application/json;q=0.9, */*;q=0.8")
        .timeout(std::time::Duration::from_secs(timeout.max(5)));
    if method != "GET"
        && let Some(args) = args {
            req = req.header("content-type", "application/json").json(args);
        }
    let resp = req.send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(anyhow!("OpenCode unauthorized. Cookie may be invalid."));
    }
    if !status.is_success() {
        return Err(anyhow!("OpenCode API error (HTTP {})", status.as_u16()));
    }
    Ok(body)
}

fn server_request_url(
    base_url: &str,
    server_id: &str,
    args: Option<&Value>,
    method: &str,
) -> String {
    let mut url = format!(
        "{}/_server?x-ssr=1&x-sfn={}&x-sr=1&x-tt=0",
        base_url, server_id
    );
    if method == "GET"
        && let Some(args) = args
            && let Ok(encoded) = serde_json::to_string(args) {
                url.push_str("&x-args=");
                url.push_str(&urlencoding::encode(&encoded));
            }
    url
}

fn normalize_workspace_id(raw: String) -> Option<String> {
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains("opencode.ai") {
        let re = Regex::new(r"wrk_[A-Za-z0-9]+").ok()?;
        return re.find(&trimmed).map(|m| m.as_str().to_string());
    }
    Some(trimmed)
}

fn parse_workspace_id(text: &str) -> Option<String> {
    let re = Regex::new(r"wrk_[A-Za-z0-9]+").ok()?;
    re.find(text).map(|m| m.as_str().to_string())
}

fn parse_opencode_usage(text: &str) -> Result<UsageSnapshot> {
    if let Some(snapshot) = parse_opencode_usage_from_text(text) {
        return Ok(snapshot);
    }
    if let Some(value) = extract_json_object(text)
        && let Some(snapshot) = parse_opencode_usage_from_value(&value) {
            return Ok(snapshot);
        }
    Err(anyhow!("OpenCode usage data missing"))
}

fn parse_opencode_usage_from_text(text: &str) -> Option<UsageSnapshot> {
    let rolling_percent = extract_double(
        r#"rollingUsage[^}]*usagePercent\s*:\s*([0-9]+(?:\.[0-9]+)?)"#,
        text,
    )?;
    let rolling_reset = extract_int(r#"rollingUsage[^}]*resetInSec\s*:\s*(\d+)"#, text)?;
    let weekly_percent = extract_double(
        r#"weeklyUsage[^}]*usagePercent\s*:\s*([0-9]+(?:\.[0-9]+)?)"#,
        text,
    )?;
    let weekly_reset = extract_int(r#"weeklyUsage[^}]*resetInSec\s*:\s*(\d+)"#, text)?;
    Some(build_usage_snapshot(
        rolling_percent,
        weekly_percent,
        rolling_reset,
        weekly_reset,
    ))
}

fn parse_opencode_usage_from_value(value: &Value) -> Option<UsageSnapshot> {
    if let Some(snapshot) = find_usage_value(value) {
        return Some(snapshot);
    }
    None
}

fn find_usage_value(value: &Value) -> Option<UsageSnapshot> {
    if let Some(obj) = value.as_object() {
        let rolling = obj
            .get("rollingUsage")
            .or_else(|| obj.get("rolling"))
            .or_else(|| obj.get("rolling_usage"));
        let weekly = obj
            .get("weeklyUsage")
            .or_else(|| obj.get("weekly"))
            .or_else(|| obj.get("weekly_usage"));
        if let (Some(rolling), Some(weekly)) = (rolling, weekly)
            && let (Some(rp), Some(rr), Some(wp), Some(wr)) = (
                rolling.get("usagePercent").and_then(|v| v.as_f64()),
                rolling.get("resetInSec").and_then(|v| v.as_i64()),
                weekly.get("usagePercent").and_then(|v| v.as_f64()),
                weekly.get("resetInSec").and_then(|v| v.as_i64()),
            ) {
                return Some(build_usage_snapshot(rp, wp, rr, wr));
            }
        for val in obj.values() {
            if let Some(snapshot) = find_usage_value(val) {
                return Some(snapshot);
            }
        }
    } else if let Some(arr) = value.as_array() {
        for item in arr {
            if let Some(snapshot) = find_usage_value(item) {
                return Some(snapshot);
            }
        }
    }
    None
}

fn build_usage_snapshot(
    rolling_percent: f64,
    weekly_percent: f64,
    rolling_reset_in_sec: i64,
    weekly_reset_in_sec: i64,
) -> UsageSnapshot {
    let now = Utc::now();
    let rolling_reset = now + chrono::Duration::seconds(rolling_reset_in_sec);
    let weekly_reset = now + chrono::Duration::seconds(weekly_reset_in_sec);
    let primary = RateWindow {
        used_percent: rolling_percent,
        window_minutes: Some(5 * 60),
        resets_at: Some(rolling_reset),
        reset_description: None,
    };
    let secondary = RateWindow {
        used_percent: weekly_percent,
        window_minutes: Some(7 * 24 * 60),
        resets_at: Some(weekly_reset),
        reset_description: None,
    };
    let identity = ProviderIdentitySnapshot {
        provider_id: Some("opencode".to_string()),
        account_email: None,
        account_organization: None,
        login_method: None,
    };
    UsageSnapshot {
        primary: Some(primary),
        secondary: Some(secondary),
        tertiary: None,
        provider_cost: None,
        updated_at: now,
        identity: Some(identity.clone()),
        account_email: identity.account_email,
        account_organization: identity.account_organization,
        login_method: identity.login_method,
    }
}

fn extract_double(pattern: &str, text: &str) -> Option<f64> {
    let re = Regex::new(pattern).ok()?;
    let caps = re.captures(text)?;
    caps.get(1)?.as_str().parse::<f64>().ok()
}

fn extract_int(pattern: &str, text: &str) -> Option<i64> {
    let re = Regex::new(pattern).ok()?;
    let caps = re.captures(text)?;
    caps.get(1)?.as_str().parse::<i64>().ok()
}

fn extract_json_object(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let slice = &text[start..=end];
    serde_json::from_str(slice).ok()
}
