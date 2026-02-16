use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot};
use crate::providers::Provider;
use crate::providers::{ProviderId, SourcePreference};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{Datelike, Local, NaiveDate, TimeZone, Utc};
use regex::Regex;
use tokio::process::Command;

pub struct KiroProvider;

#[async_trait]
impl Provider for KiroProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Kiro
    }

    fn version(&self) -> &'static str {
        "2025-01-01"
    }

    async fn fetch_usage(
        &self,
        _args: &UsageArgs,
        _config: &Config,
        source: SourcePreference,
    ) -> Result<ProviderPayload> {
        let selected = match source {
            SourcePreference::Auto => SourcePreference::Cli,
            other => other,
        };
        if selected != SourcePreference::Cli {
            return Err(CliError::UnsupportedSource(self.id(), selected.to_string()).into());
        }

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            Command::new("kiro-cli")
                .args(["chat", "--no-interactive", "/usage"])
                .output(),
        )
        .await
        .map_err(|_| anyhow!("Kiro CLI timeout"))??;

        if !output.status.success() {
            return Err(anyhow!(
                "Kiro CLI failed. Ensure kiro-cli is installed and logged in."
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let text = strip_ansi(&stdout);
        let usage = parse_kiro_usage(&text)?;
        Ok(self.ok_output("cli", Some(usage)))
    }
}

fn parse_kiro_usage(text: &str) -> Result<UsageSnapshot> {
    let plan = parse_plan_name(text);
    let primary = parse_monthly_window(text);
    let secondary = parse_bonus_window(text);

    let identity = ProviderIdentitySnapshot {
        provider_id: Some("kiro".to_string()),
        account_email: None,
        account_organization: plan.clone(),
        login_method: plan,
    };

    if primary.is_none() && secondary.is_none() {
        return Err(anyhow!("Kiro usage data missing"));
    }

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

fn parse_plan_name(text: &str) -> Option<String> {
    let re = Regex::new(r"\|\s*([A-Z0-9 ]+)\s*\|").ok()?;
    re.captures(text)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string())
}

fn parse_monthly_window(text: &str) -> Option<RateWindow> {
    let re = Regex::new(r"(?i)(\d{1,3})%.*resets on\s+(\d{1,2})/(\d{1,2})").ok()?;
    let caps = re.captures(text)?;
    let percent = caps.get(1)?.as_str().parse::<f64>().ok()?;
    let month = caps.get(2)?.as_str().parse::<u32>().ok()?;
    let day = caps.get(3)?.as_str().parse::<u32>().ok()?;
    let resets_at = parse_month_day(month, day);
    Some(RateWindow {
        used_percent: percent.clamp(0.0, 100.0),
        window_minutes: None,
        resets_at,
        reset_description: None,
    })
}

fn parse_bonus_window(text: &str) -> Option<RateWindow> {
    let re = Regex::new(
        r"(?i)bonus credits:\s*([0-9.]+)\s*/\s*([0-9.]+)\s*credits used.*expires in\s+(\d+)\s+days",
    )
    .ok()?;
    let caps = re.captures(text)?;
    let used = caps.get(1)?.as_str().parse::<f64>().ok()?;
    let total = caps.get(2)?.as_str().parse::<f64>().ok()?;
    let days = caps.get(3)?.as_str().parse::<i64>().ok()?;
    if total <= 0.0 {
        return None;
    }
    let used_percent = (used / total) * 100.0;
    let resets_at = Utc::now() + chrono::Duration::days(days);
    Some(RateWindow {
        used_percent,
        window_minutes: None,
        resets_at: Some(resets_at),
        reset_description: None,
    })
}

fn parse_month_day(month: u32, day: u32) -> Option<chrono::DateTime<Utc>> {
    let now = Local::now();
    let mut year = now.year();
    let naive = NaiveDate::from_ymd_opt(year, month, day)?;
    let local_dt = Local
        .from_local_datetime(&naive.and_hms_opt(0, 0, 0)?)
        .single()?;
    if local_dt < now {
        year += 1;
        let naive_next = NaiveDate::from_ymd_opt(year, month, day)?;
        let local_next = Local
            .from_local_datetime(&naive_next.and_hms_opt(0, 0, 0)?)
            .single()?;
        return Some(local_next.with_timezone(&Utc));
    }
    Some(local_dt.with_timezone(&Utc))
}

fn strip_ansi(text: &str) -> String {
    let re = Regex::new(r"\x1b\\[[0-9;]*m").unwrap_or_else(|_| Regex::new(r"").unwrap());
    re.replace_all(text, "").to_string()
}
