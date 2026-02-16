use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot};
use crate::providers::{env_var_nonempty, parse_rfc3339, Provider, ProviderId, SourcePreference};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

pub struct WarpProvider;

#[async_trait]
impl Provider for WarpProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Warp
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
        let api_key = cfg
            .as_ref()
            .and_then(|c| c.api_key.clone())
            .or_else(|| env_var_nonempty(&["WARP_API_KEY", "WARP_TOKEN"]))
            .ok_or_else(|| anyhow!("Warp API key missing. Set provider api_key or WARP_API_KEY."))?;

        let selected = match source {
            SourcePreference::Auto => SourcePreference::Api,
            other => other,
        };
        if selected != SourcePreference::Api {
            return Err(CliError::UnsupportedSource(self.id(), selected.to_string()).into());
        }

        let payload = warp_graphql_payload();
        let client = reqwest::Client::new();
        let resp = client
            .post("https://app.warp.dev/graphql/v2?op=GetRequestLimitInfo")
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .header("x-warp-client-id", "warp-app")
            .header("x-warp-os-category", "macOS")
            .header("x-warp-os-name", "macOS")
            .header("x-warp-os-version", "0.0.0")
            .header("authorization", format!("Bearer {}", api_key))
            .header("user-agent", "Warp/1.0")
            .json(&payload)
            .send()
            .await?;
        let status = resp.status();
        let data = resp.bytes().await?;
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(anyhow!("Warp unauthorized. API key may be invalid."));
        }
        if !status.is_success() {
            return Err(anyhow!("Warp API error (HTTP {})", status.as_u16()));
        }
        let json: Value = serde_json::from_slice(&data)?;
        let usage = parse_warp_usage(&json)?;
        Ok(self.ok_output("api", Some(usage)))
    }
}

fn warp_graphql_payload() -> Value {
    serde_json::json!({
        "query": "query GetRequestLimitInfo($requestContext: RequestContext!) { user(requestContext: $requestContext) { __typename ... on UserOutput { user { requestLimitInfo { isUnlimited nextRefreshTime requestLimit requestsUsedSinceLastRefresh } bonusGrants { requestCreditsGranted requestCreditsRemaining expiration } workspaces { bonusGrantsInfo { grants { requestCreditsGranted requestCreditsRemaining expiration } } } } } } }",
        "variables": {
            "requestContext": {
                "clientContext": {},
                "osContext": {
                    "category": "macOS",
                    "name": "macOS",
                    "version": "0.0.0"
                }
            }
        },
        "operationName": "GetRequestLimitInfo"
    })
}

fn parse_warp_usage(json: &Value) -> Result<UsageSnapshot> {
    let request_limit_info = json
        .get("data")
        .and_then(|v| v.get("user"))
        .and_then(|v| v.get("user"))
        .and_then(|v| v.get("requestLimitInfo"))
        .ok_or_else(|| anyhow!("Warp response missing requestLimitInfo"))?;

    let is_unlimited = request_limit_info
        .get("isUnlimited")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let request_limit = request_limit_info
        .get("requestLimit")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let used = request_limit_info
        .get("requestsUsedSinceLastRefresh")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let used_percent = if is_unlimited || request_limit <= 0.0 {
        0.0
    } else {
        (used / request_limit) * 100.0
    };
    let resets_at = request_limit_info
        .get("nextRefreshTime")
        .and_then(|v| v.as_str())
        .and_then(parse_rfc3339);

    let primary = RateWindow {
        used_percent,
        window_minutes: None,
        resets_at,
        reset_description: None,
    };

    let identity = ProviderIdentitySnapshot {
        provider_id: Some("warp".to_string()),
        account_email: None,
        account_organization: None,
        login_method: None,
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
