use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot};
use crate::providers::{Provider, ProviderId, SourcePreference, env_var_nonempty};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::Utc;
use regex::Regex;

pub struct AmpProvider;

#[async_trait]
impl Provider for AmpProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Amp
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
            .or_else(|| env_var_nonempty(&["AMP_COOKIE", "AMP_COOKIE_HEADER"]))
            .ok_or_else(|| {
                anyhow!("Amp cookie header missing. Set provider cookie_header or AMP_COOKIE.")
            })?;

        let client = reqwest::Client::new();
        let resp = client
            .get("https://ampcode.com/settings")
            .header("cookie", cookie)
            .header("accept", "text/html")
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(anyhow!("Amp unauthorized. Cookie may be invalid."));
        }
        if !status.is_success() {
            return Err(anyhow!("Amp request failed (HTTP {})", status.as_u16()));
        }

        let snapshot = parse_amp_usage(&body)?;
        Ok(self.ok_output("web", Some(snapshot)))
    }
}

fn parse_amp_usage(html: &str) -> Result<UsageSnapshot> {
    let usage = extract_free_tier_usage(html).ok_or_else(|| anyhow!("Amp usage data missing"))?;
    let quota = usage.quota.max(0.0);
    let used = usage.used.max(0.0);
    let used_percent = if quota > 0.0 {
        (used / quota) * 100.0
    } else {
        0.0
    };
    let window_minutes = usage.window_hours.map(|h| (h * 60.0).round() as i64);
    let resets_at = if quota > 0.0 && usage.hourly_replenishment > 0.0 {
        let hours_to_full = used / usage.hourly_replenishment;
        let seconds = (hours_to_full * 3600.0).max(0.0);
        Some(Utc::now() + chrono::Duration::seconds(seconds.round() as i64))
    } else {
        None
    };

    let primary = RateWindow {
        used_percent,
        window_minutes,
        resets_at,
        reset_description: None,
    };
    let identity = ProviderIdentitySnapshot {
        provider_id: Some("amp".to_string()),
        account_email: None,
        account_organization: None,
        login_method: Some("Amp Free".to_string()),
    };
    Ok(UsageSnapshot {
        primary: Some(primary),
        secondary: None,
        tertiary: None,
        provider_cost: None,
        updated_at: Utc::now(),
        identity: Some(identity.clone()),
        account_email: identity.account_email,
        account_organization: identity.account_organization,
        login_method: identity.login_method,
    })
}

#[derive(Debug)]
struct FreeTierUsage {
    quota: f64,
    used: f64,
    hourly_replenishment: f64,
    window_hours: Option<f64>,
}

fn extract_free_tier_usage(html: &str) -> Option<FreeTierUsage> {
    let tokens = ["freeTierUsage", "getFreeTierUsage"];
    for token in tokens {
        if let Some(object) = extract_object_named(html, token)
            && let Some(usage) = parse_free_tier_object(&object)
        {
            return Some(usage);
        }
    }
    None
}

fn extract_object_named(text: &str, token: &str) -> Option<String> {
    let index = text.find(token)?;
    let slice = &text[index + token.len()..];
    let brace_index = slice.find('{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let mut end = None;
    for (i, ch) in slice[brace_index..].char_indices() {
        let c = ch;
        if in_string {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
        } else if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                end = Some(brace_index + i);
                break;
            }
        }
    }
    let end_index = end?;
    Some(slice[brace_index..=end_index].to_string())
}

fn parse_free_tier_object(object: &str) -> Option<FreeTierUsage> {
    let quota = number_for_key(object, "quota")?;
    let used = number_for_key(object, "used")?;
    let hourly = number_for_key(object, "hourlyReplenishment")?;
    let window_hours = number_for_key(object, "windowHours");
    Some(FreeTierUsage {
        quota,
        used,
        hourly_replenishment: hourly,
        window_hours,
    })
}

fn number_for_key(text: &str, key: &str) -> Option<f64> {
    let pattern = format!(r#"{}\s*:\s*([0-9]+(?:\.[0-9]+)?)"#, regex::escape(key));
    let regex = Regex::new(&pattern).ok()?;
    let caps = regex.captures(text)?;
    caps.get(1)?.as_str().parse::<f64>().ok()
}
