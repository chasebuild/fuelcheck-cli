use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot};
use crate::providers::{Provider, ProviderId, SourcePreference};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, Utc};
use directories::BaseDirs;
use regex::Regex;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

pub struct GeminiProvider;

#[async_trait]
impl Provider for GeminiProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Gemini
    }

    fn version(&self) -> &'static str {
        "2024-12-01"
    }

    async fn fetch_usage(
        &self,
        _args: &UsageArgs,
        _config: &Config,
        source: SourcePreference,
    ) -> Result<ProviderPayload> {
        let selected = match source {
            SourcePreference::Auto => SourcePreference::Api,
            other => other,
        };

        match selected {
            SourcePreference::Api => {
                let usage = fetch_gemini_usage().await?;
                Ok(self.ok_output("api", Some(usage)))
            }
            SourcePreference::Local
            | SourcePreference::Cli
            | SourcePreference::Web
            | SourcePreference::Oauth => {
                Err(CliError::UnsupportedSource(self.id(), selected.to_string()).into())
            }
            SourcePreference::Auto => {
                Err(CliError::UnsupportedSource(self.id(), "auto".into()).into())
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct SettingsAuth {
    #[serde(rename = "selectedType")]
    selected_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SettingsSecurity {
    auth: Option<SettingsAuth>,
}

#[derive(Debug, Deserialize)]
struct SettingsRoot {
    security: Option<SettingsSecurity>,
}

#[derive(Debug)]
struct OAuthCredentials {
    access_token: Option<String>,
    id_token: Option<String>,
    refresh_token: Option<String>,
    expiry_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct QuotaResponse {
    buckets: Option<Vec<QuotaBucket>>,
}

#[derive(Debug, Deserialize)]
struct QuotaBucket {
    #[serde(rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodeAssistResponse {
    #[serde(rename = "currentTier")]
    current_tier: Option<CodeAssistTier>,
    #[serde(rename = "cloudaicompanionProject")]
    cloudaicompanion_project: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct CodeAssistTier {
    id: Option<String>,
}

async fn fetch_gemini_usage() -> Result<UsageSnapshot> {
    let auth_type = read_gemini_auth_type()?;
    match auth_type.as_deref() {
        Some("api-key") => return Err(anyhow!("Gemini API key auth not supported. Use OAuth.")),
        Some("vertex-ai") => return Err(anyhow!("Gemini Vertex AI auth not supported.")),
        _ => {}
    }

    let mut creds = load_oauth_credentials()?;
    if creds.access_token.is_none() {
        return Err(anyhow!(
            "Gemini not logged in. Run `gemini` to authenticate."
        ));
    }
    if let Some(expiry) = creds.expiry_date
        && expiry < Utc::now()
            && let Some(refresh) = creds.refresh_token.clone() {
                let new_token = refresh_access_token(&refresh).await?;
                creds.access_token = Some(new_token);
            }

    let access_token = creds
        .access_token
        .clone()
        .ok_or_else(|| anyhow!("missing access token"))?;
    let claims = extract_claims(creds.id_token.as_deref());
    let code_assist = load_code_assist(&access_token)
        .await
        .unwrap_or((None, None));
    let project_id = if code_assist.1.is_some() {
        code_assist.1
    } else {
        discover_project_id(&access_token).await?
    };

    let quota = fetch_quota(&access_token, project_id.as_deref()).await?;
    let snapshot = parse_quota(quota, claims.0)?;
    let plan = match (code_assist.0.as_deref(), claims.1.as_deref()) {
        (Some("standard-tier"), _) => Some("Paid".to_string()),
        (Some("free-tier"), Some(_)) => Some("Workspace".to_string()),
        (Some("free-tier"), None) => Some("Free".to_string()),
        (Some("legacy-tier"), _) => Some("Legacy".to_string()),
        _ => None,
    };

    Ok(snapshot_with_plan(snapshot, plan))
}

fn read_gemini_auth_type() -> Result<Option<String>> {
    let path = gemini_home().join("settings.json");
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read(&path)?;
    let root: SettingsRoot = serde_json::from_slice(&data)?;
    Ok(root
        .security
        .and_then(|s| s.auth)
        .and_then(|a| a.selected_type))
}

fn load_oauth_credentials() -> Result<OAuthCredentials> {
    let path = gemini_home().join("oauth_creds.json");
    if !path.exists() {
        return Err(anyhow!("Gemini credentials not found"));
    }
    let data = fs::read(&path)?;
    let json: serde_json::Value = serde_json::from_slice(&data)?;
    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let id_token = json
        .get("id_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let refresh_token = json
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let expiry_date = json
        .get("expiry_date")
        .and_then(|v| v.as_f64())
        .and_then(|ms| DateTime::<Utc>::from_timestamp((ms / 1000.0) as i64, 0));

    Ok(OAuthCredentials {
        access_token,
        id_token,
        refresh_token,
        expiry_date,
    })
}

fn gemini_home() -> PathBuf {
    let home = BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".gemini")
}

async fn refresh_access_token(refresh_token: &str) -> Result<String> {
    let (client_id, client_secret) = extract_oauth_client()?;
    let url = "https://oauth2.googleapis.com/token";
    let body = format!(
        "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
        client_id, client_secret, refresh_token
    );
    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?;
    let status = resp.status();
    let data = resp.bytes().await?;
    if !status.is_success() {
        return Err(anyhow!(
            "Gemini token refresh failed (HTTP {})",
            status.as_u16()
        ));
    }
    let json: serde_json::Value = serde_json::from_slice(&data)?;
    let token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Gemini refresh response missing access_token"))?;
    Ok(token.to_string())
}

fn extract_oauth_client() -> Result<(String, String)> {
    let gemini_path = which("gemini").ok_or_else(|| anyhow!("gemini CLI not found on PATH"))?;
    let real_path = std::fs::read_link(&gemini_path).unwrap_or(gemini_path.clone());
    let bin_dir = real_path
        .parent()
        .ok_or_else(|| anyhow!("gemini path invalid"))?;
    let base_dir = bin_dir
        .parent()
        .ok_or_else(|| anyhow!("gemini path invalid"))?;

    let oauth_paths = vec![
        base_dir.join("libexec/lib/node_modules/@google/gemini-cli/node_modules/@google/gemini-cli-core/dist/src/code_assist/oauth2.js"),
        base_dir.join("lib/node_modules/@google/gemini-cli/node_modules/@google/gemini-cli-core/dist/src/code_assist/oauth2.js"),
        base_dir.join("share/gemini-cli/node_modules/@google/gemini-cli-core/dist/src/code_assist/oauth2.js"),
        base_dir.join("../gemini-cli-core/dist/src/code_assist/oauth2.js"),
        base_dir.join("node_modules/@google/gemini-cli-core/dist/src/code_assist/oauth2.js"),
    ];

    let client_re = Regex::new(r#"OAUTH_CLIENT_ID\s*=\s*['"]([\w\-.]+)['"]"#).unwrap();
    let secret_re = Regex::new(r#"OAUTH_CLIENT_SECRET\s*=\s*['"]([\w\-]+)['"]"#).unwrap();

    for path in oauth_paths {
        if let Ok(content) = fs::read_to_string(&path)
            && let (Some(id_cap), Some(secret_cap)) =
                (client_re.captures(&content), secret_re.captures(&content))
            {
                let client_id = id_cap.get(1).unwrap().as_str().to_string();
                let client_secret = secret_cap.get(1).unwrap().as_str().to_string();
                return Ok((client_id, client_secret));
            }
    }

    Err(anyhow!("Could not locate Gemini CLI OAuth credentials"))
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for entry in std::env::split_paths(&path) {
        let candidate = entry.join(bin);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

async fn load_code_assist(access_token: &str) -> Result<(Option<String>, Option<String>)> {
    let url = "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .body("{\"metadata\":{\"ideType\":\"GEMINI_CLI\",\"pluginType\":\"GEMINI\"}}")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok((None, None));
    }
    let data = resp.bytes().await?;
    let parsed: CodeAssistResponse = serde_json::from_slice(&data)?;
    let tier = parsed.current_tier.and_then(|t| t.id);
    let project = match parsed.cloudaicompanion_project {
        Some(serde_json::Value::String(val)) => Some(val),
        Some(serde_json::Value::Object(map)) => map
            .get("id")
            .and_then(|v| v.as_str())
            .or_else(|| map.get("projectId").and_then(|v| v.as_str()))
            .map(|s| s.to_string()),
        _ => None,
    };
    Ok((tier, project))
}

async fn discover_project_id(access_token: &str) -> Result<Option<String>> {
    let url = "https://cloudresourcemanager.googleapis.com/v1/projects";
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let data = resp.bytes().await?;
    let json: serde_json::Value = serde_json::from_slice(&data)?;
    let projects = json
        .get("projects")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for project in projects {
        if let Some(project_id) = project.get("projectId").and_then(|v| v.as_str()) {
            if project_id.starts_with("gen-lang-client") {
                return Ok(Some(project_id.to_string()));
            }
            if let Some(labels) = project.get("labels").and_then(|v| v.as_object())
                && labels.contains_key("generative-language") {
                    return Ok(Some(project_id.to_string()));
                }
        }
    }
    Ok(None)
}

async fn fetch_quota(access_token: &str, project_id: Option<&str>) -> Result<QuotaResponse> {
    let url = "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota";
    let body = if let Some(project) = project_id {
        serde_json::json!({ "project": project })
    } else {
        serde_json::json!({})
    };
    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    let data = resp.bytes().await?;
    if !status.is_success() {
        return Err(anyhow!("Gemini quota API error (HTTP {})", status.as_u16()));
    }
    Ok(serde_json::from_slice(&data)?)
}

fn parse_quota(response: QuotaResponse, email: Option<String>) -> Result<UsageSnapshot> {
    let buckets = response
        .buckets
        .ok_or_else(|| anyhow!("Gemini quota response missing buckets"))?;

    let mut model_map: std::collections::BTreeMap<String, (f64, Option<String>)> =
        std::collections::BTreeMap::new();
    for bucket in buckets {
        let model_id = match bucket.model_id {
            Some(v) => v,
            None => continue,
        };
        let fraction = match bucket.remaining_fraction {
            Some(v) => v,
            None => continue,
        };
        let entry = model_map
            .entry(model_id)
            .or_insert((fraction, bucket.reset_time.clone()));
        if fraction < entry.0 {
            *entry = (fraction, bucket.reset_time.clone());
        }
    }

    let mut quotas: Vec<(String, f64, Option<String>)> = Vec::new();
    for (model_id, (fraction, reset_time)) in model_map {
        quotas.push((model_id, fraction, reset_time));
    }

    let mut flash_min: Option<(f64, Option<String>)> = None;
    let mut pro_min: Option<(f64, Option<String>)> = None;
    for (model_id, fraction, reset_time) in quotas {
        let lower = model_id.to_lowercase();
        let target = if lower.contains("flash") {
            &mut flash_min
        } else if lower.contains("pro") {
            &mut pro_min
        } else {
            continue;
        };
        if target.is_none() || fraction < target.as_ref().unwrap().0 {
            *target = Some((fraction, reset_time));
        }
    }

    let primary = pro_min.map(|(fraction, reset)| make_gemini_window(fraction, reset));
    let secondary = flash_min.map(|(fraction, reset)| make_gemini_window(fraction, reset));

    let identity = ProviderIdentitySnapshot {
        provider_id: Some("gemini".to_string()),
        account_email: email,
        account_organization: None,
        login_method: None,
    };

    Ok(UsageSnapshot {
        primary,
        secondary,
        tertiary: None,
        provider_cost: None,
        updated_at: Utc::now(),
        account_email: identity.account_email.clone(),
        account_organization: None,
        login_method: None,
        identity: Some(identity),
    })
}

fn make_gemini_window(fraction_left: f64, reset_time: Option<String>) -> RateWindow {
    let resets_at = reset_time
        .as_ref()
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|dt| dt.with_timezone(&Utc));
    let reset_description = reset_time.as_ref().map(|raw| format_gemini_reset(raw));
    RateWindow {
        used_percent: 100.0 - (fraction_left * 100.0),
        window_minutes: Some(1440),
        resets_at,
        reset_description,
    }
}

fn format_gemini_reset(raw: &str) -> String {
    if let Ok(date) = DateTime::parse_from_rfc3339(raw) {
        let reset_at = date.with_timezone(&Utc);
        let now = Utc::now();
        let duration = reset_at.signed_duration_since(now);
        if duration.num_seconds() <= 0 {
            return "Resets soon".to_string();
        }
        let hours = duration.num_hours();
        let minutes = (duration.num_minutes() % 60).max(0);
        if hours > 0 {
            return format!("Resets in {}h {}m", hours, minutes);
        }
        return format!("Resets in {}m", minutes);
    }
    "Resets soon".to_string()
}

fn extract_claims(id_token: Option<&str>) -> (Option<String>, Option<String>) {
    let token = match id_token {
        Some(t) => t,
        None => return (None, None),
    };
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return (None, None);
    }
    let payload = parts[1].replace('-', "+").replace('_', "/");
    let padded = match payload.len() % 4 {
        0 => payload,
        rem => format!("{}{}", payload, "=".repeat(4 - rem)),
    };
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(padded)
        .ok();
    if let Some(decoded) = decoded
        && let Ok(json) = serde_json::from_slice::<serde_json::Value>(&decoded) {
            let email = json
                .get("email")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let hd = json
                .get("hd")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            return (email, hd);
        }
    (None, None)
}

fn snapshot_with_plan(mut snapshot: UsageSnapshot, plan: Option<String>) -> UsageSnapshot {
    if let Some(mut identity) = snapshot.identity.clone() {
        identity.login_method = plan.clone();
        snapshot.login_method = plan;
        snapshot.identity = Some(identity);
    }
    snapshot
}
