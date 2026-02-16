use crate::cli::UsageArgs;
use crate::config::Config;
use crate::errors::CliError;
use crate::model::{ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot};
use crate::providers::{Provider, ProviderId, SourcePreference, parse_rfc3339};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct VertexAIProvider;

#[async_trait]
impl Provider for VertexAIProvider {
    fn id(&self) -> ProviderId {
        ProviderId::VertexAI
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
            SourcePreference::Auto => SourcePreference::Oauth,
            other => other,
        };
        if selected != SourcePreference::Oauth {
            return Err(CliError::UnsupportedSource(self.id(), selected.to_string()).into());
        }

        let mut creds = VertexAIOAuthCredentials::load()?;
        if creds.needs_refresh() {
            creds = refresh_vertex_token(&creds).await?;
        }

        let usage = fetch_vertex_usage(&creds).await;
        let snapshot = match usage {
            Ok(Some(usage)) => map_vertex_usage(&usage, &creds),
            Ok(None) => map_vertex_usage_empty(&creds),
            Err(err) => return Err(err),
        };
        Ok(self.ok_output("oauth", Some(snapshot)))
    }
}

#[derive(Debug, Clone)]
struct VertexAIOAuthCredentials {
    access_token: String,
    refresh_token: String,
    client_id: String,
    client_secret: String,
    project_id: Option<String>,
    email: Option<String>,
    expiry_date: Option<DateTime<Utc>>,
}

impl VertexAIOAuthCredentials {
    fn needs_refresh(&self) -> bool {
        match self.expiry_date {
            Some(expiry) => expiry - chrono::Duration::minutes(5) <= Utc::now(),
            None => true,
        }
    }

    fn load() -> Result<Self> {
        let path = adc_credentials_path().ok_or_else(|| anyhow!("gcloud credentials not found"))?;
        let data = std::fs::read(&path)?;
        let json: serde_json::Value = serde_json::from_slice(&data)?;
        let client_id = json
            .get("client_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("gcloud credentials missing client_id"))?;
        let client_secret = json
            .get("client_secret")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("gcloud credentials missing client_secret"))?;
        let refresh_token = json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("gcloud credentials missing refresh_token"))?;
        let access_token = json
            .get("access_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let expiry_date = json
            .get("token_expiry")
            .and_then(|v| v.as_str())
            .and_then(parse_rfc3339);
        let project_id = load_project_id();
        let email = json
            .get("id_token")
            .and_then(|v| v.as_str())
            .and_then(extract_email_from_jwt);

        Ok(Self {
            access_token,
            refresh_token: refresh_token.to_string(),
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            project_id,
            email,
            expiry_date,
        })
    }
}

async fn refresh_vertex_token(
    creds: &VertexAIOAuthCredentials,
) -> Result<VertexAIOAuthCredentials> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!(
            "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
            urlencoding::encode(&creds.client_id),
            urlencoding::encode(&creds.client_secret),
            urlencoding::encode(&creds.refresh_token),
        ))
        .send()
        .await?;
    let status = resp.status();
    let data = resp.bytes().await?;
    if !status.is_success() {
        return Err(anyhow!(
            "Vertex AI token refresh failed (HTTP {})",
            status.as_u16()
        ));
    }
    let json: serde_json::Value = serde_json::from_slice(&data)?;
    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or(&creds.access_token)
        .to_string();
    let expires_in = json
        .get("expires_in")
        .and_then(|v| v.as_f64())
        .unwrap_or(3600.0);
    let expiry_date = Some(Utc::now() + chrono::Duration::seconds(expires_in.round() as i64));
    let email = json
        .get("id_token")
        .and_then(|v| v.as_str())
        .and_then(extract_email_from_jwt)
        .or_else(|| creds.email.clone());

    Ok(VertexAIOAuthCredentials {
        access_token,
        refresh_token: creds.refresh_token.clone(),
        client_id: creds.client_id.clone(),
        client_secret: creds.client_secret.clone(),
        project_id: creds.project_id.clone(),
        email,
        expiry_date,
    })
}

fn adc_credentials_path() -> Option<PathBuf> {
    if let Ok(config_dir) = std::env::var("CLOUDSDK_CONFIG") {
        let trimmed = config_dir.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed).join("application_default_credentials.json"));
        }
    }
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    Some(
        home.join(".config")
            .join("gcloud")
            .join("application_default_credentials.json"),
    )
}

fn load_project_id() -> Option<String> {
    if let Ok(value) = std::env::var("GOOGLE_CLOUD_PROJECT") {
        if !value.trim().is_empty() {
            return Some(value);
        }
    }
    if let Ok(value) = std::env::var("GCLOUD_PROJECT") {
        if !value.trim().is_empty() {
            return Some(value);
        }
    }
    if let Ok(value) = std::env::var("CLOUDSDK_CORE_PROJECT") {
        if !value.trim().is_empty() {
            return Some(value);
        }
    }
    let config_path = if let Ok(config_dir) = std::env::var("CLOUDSDK_CONFIG") {
        let trimmed = config_dir.trim();
        if trimmed.is_empty() {
            return None;
        }
        PathBuf::from(trimmed)
            .join("configurations")
            .join("config_default")
    } else {
        let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
        home.join(".config")
            .join("gcloud")
            .join("configurations")
            .join("config_default")
    };
    let content = std::fs::read_to_string(config_path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("project") {
            let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
            if parts.len() == 2 {
                let project = parts[1].trim();
                if !project.is_empty() {
                    return Some(project.to_string());
                }
            }
        }
    }
    None
}

fn extract_email_from_jwt(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let mut payload = parts[1].replace('-', "+").replace('_', "/");
    let remainder = payload.len() % 4;
    if remainder > 0 {
        payload.push_str(&"=".repeat(4 - remainder));
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    json.get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[derive(Debug)]
struct VertexAIUsage {
    requests_used_percent: f64,
    resets_at: Option<DateTime<Utc>>,
}

async fn fetch_vertex_usage(creds: &VertexAIOAuthCredentials) -> Result<Option<VertexAIUsage>> {
    let project_id = creds
        .project_id
        .clone()
        .ok_or_else(|| anyhow!("No Google Cloud project configured."))?;
    let usage_filter = "metric.type=\"serviceruntime.googleapis.com/quota/allocation/usage\" AND resource.type=\"consumer_quota\" AND resource.label.service=\"aiplatform.googleapis.com\"";
    let limit_filter = "metric.type=\"serviceruntime.googleapis.com/quota/limit\" AND resource.type=\"consumer_quota\" AND resource.label.service=\"aiplatform.googleapis.com\"";
    let usage_series = fetch_time_series(&project_id, usage_filter, &creds.access_token).await?;
    let limit_series = fetch_time_series(&project_id, limit_filter, &creds.access_token).await?;

    let usage_map = aggregate_series(&usage_series);
    let limit_map = aggregate_series(&limit_series);
    if usage_map.is_empty() || limit_map.is_empty() {
        return Ok(None);
    }

    let mut max_percent: Option<f64> = None;
    for (key, limit) in &limit_map {
        if *limit <= 0.0 {
            continue;
        }
        if let Some(usage) = usage_map.get(key) {
            let percent = (usage / limit) * 100.0;
            max_percent = Some(max_percent.map(|v| v.max(percent)).unwrap_or(percent));
        }
    }

    let used_percent = match max_percent {
        Some(v) => v,
        None => return Ok(None),
    };
    Ok(Some(VertexAIUsage {
        requests_used_percent: used_percent,
        resets_at: None,
    }))
}

async fn fetch_time_series(
    project_id: &str,
    filter: &str,
    access_token: &str,
) -> Result<Vec<MonitoringTimeSeries>> {
    let now = Utc::now();
    let start = now - chrono::Duration::hours(24);
    let mut all = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut params = vec![
            ("filter", filter.to_string()),
            ("interval.startTime", now_to_rfc3339(start)),
            ("interval.endTime", now_to_rfc3339(now)),
            ("view", "FULL".to_string()),
        ];
        if let Some(token) = &page_token {
            params.push(("pageToken", token.clone()));
        }
        let url = format!(
            "https://monitoring.googleapis.com/v3/projects/{}/timeSeries",
            project_id
        );
        let client = reqwest::Client::new();
        let resp = client
            .get(url)
            .bearer_auth(access_token)
            .query(&params)
            .send()
            .await?;
        let status = resp.status();
        let data = resp.bytes().await?;
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(anyhow!(
                "Vertex AI unauthorized. Re-run gcloud auth application-default login."
            ));
        }
        if !status.is_success() {
            let body = String::from_utf8_lossy(&data);
            return Err(anyhow!(
                "Vertex AI monitoring error (HTTP {}): {}",
                status.as_u16(),
                body
            ));
        }
        let decoded: MonitoringTimeSeriesResponse = serde_json::from_slice(&data)?;
        if let Some(series) = decoded.time_series {
            all.extend(series);
        }
        page_token = decoded
            .next_page_token
            .and_then(|t| if t.is_empty() { None } else { Some(t) });
        if page_token.is_none() {
            break;
        }
    }
    Ok(all)
}

fn now_to_rfc3339(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339()
}

#[derive(Debug, Deserialize)]
struct MonitoringTimeSeriesResponse {
    #[serde(rename = "timeSeries")]
    time_series: Option<Vec<MonitoringTimeSeries>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MonitoringTimeSeries {
    metric: MonitoringMetric,
    resource: MonitoringResource,
    points: Vec<MonitoringPoint>,
}

#[derive(Debug, Deserialize)]
struct MonitoringMetric {
    labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct MonitoringResource {
    labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct MonitoringPoint {
    value: MonitoringValue,
}

#[derive(Debug, Deserialize)]
struct MonitoringValue {
    #[serde(rename = "doubleValue")]
    double_value: Option<f64>,
    #[serde(rename = "int64Value")]
    int64_value: Option<String>,
}

#[derive(Debug, Hash, Eq, PartialEq)]
struct QuotaKey {
    quota_metric: String,
    limit_name: String,
    location: String,
}

fn aggregate_series(series: &[MonitoringTimeSeries]) -> HashMap<QuotaKey, f64> {
    let mut buckets: HashMap<QuotaKey, f64> = HashMap::new();
    for entry in series {
        if let Some(key) = quota_key(entry) {
            if let Some(value) = max_point_value(&entry.points) {
                let existing = buckets.get(&key).copied().unwrap_or(0.0);
                if value > existing {
                    buckets.insert(key, value);
                }
            }
        }
    }
    buckets
}

fn quota_key(series: &MonitoringTimeSeries) -> Option<QuotaKey> {
    let metric_labels = series.metric.labels.as_ref()?;
    let empty_labels: HashMap<String, String> = HashMap::new();
    let resource_labels = series.resource.labels.as_ref().unwrap_or(&empty_labels);
    let quota_metric = metric_labels
        .get("quota_metric")
        .or_else(|| resource_labels.get("quota_id"))?
        .to_string();
    let limit_name = metric_labels.get("limit_name").cloned().unwrap_or_default();
    let location = resource_labels
        .get("location")
        .cloned()
        .unwrap_or_else(|| "global".to_string());
    Some(QuotaKey {
        quota_metric,
        limit_name,
        location,
    })
}

fn max_point_value(points: &[MonitoringPoint]) -> Option<f64> {
    points
        .iter()
        .filter_map(point_value)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
}

fn point_value(point: &MonitoringPoint) -> Option<f64> {
    if let Some(val) = point.value.double_value {
        return Some(val);
    }
    if let Some(val) = point.value.int64_value.as_ref() {
        return val.parse::<f64>().ok();
    }
    None
}

fn map_vertex_usage(usage: &VertexAIUsage, creds: &VertexAIOAuthCredentials) -> UsageSnapshot {
    let primary = RateWindow {
        used_percent: usage.requests_used_percent,
        window_minutes: None,
        resets_at: usage.resets_at,
        reset_description: None,
    };
    let identity = ProviderIdentitySnapshot {
        provider_id: Some("vertexai".to_string()),
        account_email: creds.email.clone(),
        account_organization: creds.project_id.clone(),
        login_method: Some("gcloud".to_string()),
    };
    UsageSnapshot {
        primary: Some(primary),
        secondary: None,
        tertiary: None,
        provider_cost: None,
        updated_at: Utc::now(),
        identity: Some(identity.clone()),
        account_email: identity.account_email,
        account_organization: identity.account_organization,
        login_method: identity.login_method,
    }
}

fn map_vertex_usage_empty(creds: &VertexAIOAuthCredentials) -> UsageSnapshot {
    let identity = ProviderIdentitySnapshot {
        provider_id: Some("vertexai".to_string()),
        account_email: creds.email.clone(),
        account_organization: creds.project_id.clone(),
        login_method: Some("gcloud".to_string()),
    };
    UsageSnapshot {
        primary: None,
        secondary: None,
        tertiary: None,
        provider_cost: None,
        updated_at: Utc::now(),
        identity: Some(identity.clone()),
        account_email: identity.account_email,
        account_organization: identity.account_organization,
        login_method: identity.login_method,
    }
}
