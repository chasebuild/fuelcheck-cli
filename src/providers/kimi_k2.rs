use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot};
use crate::providers::{Provider, ProviderId, SourcePreference, env_var_nonempty, value_to_f64};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

pub struct KimiK2Provider;

#[async_trait]
impl Provider for KimiK2Provider {
    fn id(&self) -> ProviderId {
        ProviderId::KimiK2
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
            .or_else(|| env_var_nonempty(&["KIMI_K2_API_KEY", "KIMI_API_KEY", "KIMI_KEY"]))
            .ok_or_else(|| {
                anyhow!("Kimi K2 API key missing. Set provider api_key or KIMI_K2_API_KEY.")
            })?;

        let selected = match source {
            SourcePreference::Auto => SourcePreference::Api,
            other => other,
        };
        if selected != SourcePreference::Api {
            return Err(CliError::UnsupportedSource(self.id(), selected.to_string()).into());
        }

        let client = reqwest::Client::new();
        let resp = client
            .get("https://kimi-k2.ai/api/user/credits")
            .header("authorization", format!("Bearer {}", token))
            .header("accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        let headers = resp.headers().clone();
        let data = resp.bytes().await?;
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(anyhow!("Kimi K2 unauthorized. API key may be invalid."));
        }
        if !status.is_success() {
            return Err(anyhow!("Kimi K2 API error (HTTP {})", status.as_u16()));
        }
        let json: Value = serde_json::from_slice(&data)?;
        let usage = map_kimi_k2_usage(&json, &headers)?;
        Ok(self.ok_output("api", Some(usage)))
    }
}

fn map_kimi_k2_usage(json: &Value, headers: &reqwest::header::HeaderMap) -> Result<UsageSnapshot> {
    let remaining = find_number(
        json,
        &[
            "creditsRemaining",
            "remainingCredits",
            "remaining",
            "credits_remaining",
            "available",
        ],
    )
    .or_else(|| {
        headers
            .get("X-Credits-Remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<f64>().ok())
    });
    let consumed = find_number(
        json,
        &["creditsConsumed", "consumed", "used", "credits_used"],
    );
    let total = find_number(json, &["totalCredits", "total", "creditsTotal"]).or({
        match (remaining, consumed) {
            (Some(r), Some(c)) => Some(r + c),
            _ => None,
        }
    });

    let used_percent = if let (Some(c), Some(t)) = (consumed, total) {
        if t > 0.0 { Some((c / t) * 100.0) } else { None }
    } else if let (Some(r), Some(t)) = (remaining, total) {
        if t > 0.0 {
            Some(((t - r) / t) * 100.0)
        } else {
            None
        }
    } else {
        None
    };

    let primary = used_percent.map(|used| RateWindow {
        used_percent: used,
        window_minutes: None,
        resets_at: None,
        reset_description: None,
    });

    let identity = ProviderIdentitySnapshot {
        provider_id: Some("kimik2".to_string()),
        account_email: None,
        account_organization: None,
        login_method: None,
    };

    Ok(UsageSnapshot {
        primary,
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

fn find_number(value: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(val) = value.get(*key)
            && let Some(num) = value_to_f64(val) {
                return Some(num);
            }
    }
    None
}
