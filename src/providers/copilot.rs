use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot};
use crate::providers::{Provider, ProviderId, SourcePreference, env_var_nonempty};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

pub struct CopilotProvider;

#[async_trait]
impl Provider for CopilotProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Copilot
    }

    fn version(&self) -> &'static str {
        "2025-04-01"
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
            .or_else(|| env_var_nonempty(&["COPILOT_API_TOKEN", "GITHUB_TOKEN"]))
            .ok_or_else(|| {
                anyhow!("Copilot API token missing. Set provider api_key or COPILOT_API_TOKEN.")
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
            .get("https://api.github.com/copilot_internal/user")
            .header("authorization", format!("token {}", token))
            .header("accept", "application/json")
            .header("editor-version", "vscode/1.96.2")
            .header("editor-plugin-version", "copilot-chat/0.26.7")
            .header("user-agent", "GitHubCopilotChat/0.26.7")
            .header("x-github-api-version", "2025-04-01")
            .send()
            .await?;
        let status = resp.status();
        let data = resp.bytes().await?;
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(anyhow!("Copilot unauthorized. Token may be invalid."));
        }
        if !status.is_success() {
            return Err(anyhow!("Copilot API error (HTTP {})", status.as_u16()));
        }
        let response: CopilotUsageResponse = serde_json::from_slice(&data)?;
        let usage = map_copilot_usage(response);
        Ok(self.ok_output("api", Some(usage)))
    }
}

#[derive(Debug, Deserialize)]
struct CopilotUsageResponse {
    #[serde(rename = "quota_snapshots")]
    quota_snapshots: CopilotQuotaSnapshots,
    #[serde(rename = "copilot_plan")]
    copilot_plan: String,
}

#[derive(Debug, Deserialize)]
struct CopilotQuotaSnapshots {
    #[serde(rename = "premium_interactions")]
    premium_interactions: Option<CopilotQuotaSnapshot>,
    chat: Option<CopilotQuotaSnapshot>,
}

#[derive(Debug, Deserialize)]
struct CopilotQuotaSnapshot {
    #[serde(rename = "percent_remaining")]
    percent_remaining: f64,
}

fn map_copilot_usage(response: CopilotUsageResponse) -> UsageSnapshot {
    let primary = response
        .quota_snapshots
        .premium_interactions
        .map(|snap| RateWindow {
            used_percent: (100.0 - snap.percent_remaining).clamp(0.0, 100.0),
            window_minutes: None,
            resets_at: None,
            reset_description: None,
        });
    let secondary = response.quota_snapshots.chat.map(|snap| RateWindow {
        used_percent: (100.0 - snap.percent_remaining).clamp(0.0, 100.0),
        window_minutes: None,
        resets_at: None,
        reset_description: None,
    });

    let identity = ProviderIdentitySnapshot {
        provider_id: Some("copilot".to_string()),
        account_email: None,
        account_organization: None,
        login_method: Some(response.copilot_plan.to_lowercase()),
    };

    UsageSnapshot {
        primary: primary.or({
            Some(RateWindow {
                used_percent: 0.0,
                window_minutes: None,
                resets_at: None,
                reset_description: None,
            })
        }),
        secondary,
        tertiary: None,
        provider_cost: None,
        updated_at: Utc::now(),
        identity: Some(identity.clone()),
        account_email: identity.account_email,
        account_organization: identity.account_organization,
        login_method: identity.login_method,
    }
}
