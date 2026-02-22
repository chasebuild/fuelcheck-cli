use crate::config::Config;
use crate::errors::CliError;
use crate::model::{ProviderIdentitySnapshot, ProviderPayload, RateWindow, UsageSnapshot};
use crate::providers::{Provider, ProviderId, SourcePreference, parse_rfc3339};
use crate::service::UsageRequest;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::Utc;
use globwalk::GlobWalkerBuilder;
use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct JetBrainsProvider;

#[async_trait]
impl Provider for JetBrainsProvider {
    fn id(&self) -> ProviderId {
        ProviderId::JetBrains
    }

    fn version(&self) -> &'static str {
        "2025-01-01"
    }

    async fn fetch_usage(
        &self,
        _args: &UsageRequest,
        _config: &Config,
        source: SourcePreference,
    ) -> Result<ProviderPayload> {
        let selected = match source {
            SourcePreference::Auto => SourcePreference::Local,
            other => other,
        };
        if selected != SourcePreference::Local {
            return Err(CliError::UnsupportedSource(self.id(), selected.to_string()).into());
        }

        let file =
            find_jetbrains_quota_file().ok_or_else(|| anyhow!("JetBrains quota file not found"))?;
        let contents = std::fs::read_to_string(&file)?;
        let usage = parse_jetbrains_quota(&contents, &file)?;
        Ok(self.ok_output("local", Some(usage)))
    }
}

fn find_jetbrains_quota_file() -> Option<PathBuf> {
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    let mut roots = Vec::new();
    if cfg!(target_os = "macos") {
        roots.push(
            home.join("Library")
                .join("Application Support")
                .join("JetBrains"),
        );
        roots.push(
            home.join("Library")
                .join("Application Support")
                .join("Google"),
        );
    } else {
        roots.push(home.join(".config").join("JetBrains"));
        roots.push(home.join(".config").join("Google"));
    }

    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for root in roots {
        if !root.exists() {
            continue;
        }
        let walker =
            GlobWalkerBuilder::from_patterns(&root, &["**/options/AIAssistantQuotaManager2.xml"])
                .case_insensitive(true)
                .build()
                .ok()?;
        for entry in walker.filter_map(Result::ok) {
            let path = entry.path().to_path_buf();
            if let Ok(meta) = std::fs::metadata(&path)
                && let Ok(modified) = meta.modified()
            {
                let replace = match &best {
                    Some((_, best_time)) => modified > *best_time,
                    None => true,
                };
                if replace {
                    best = Some((path.clone(), modified));
                }
            }
        }
    }
    best.map(|(path, _)| path)
}

fn parse_jetbrains_quota(contents: &str, path: &Path) -> Result<UsageSnapshot> {
    let quota_info = extract_attribute_json(contents, "quotaInfo")
        .ok_or_else(|| anyhow!("JetBrains quotaInfo missing"))?;
    let next_refill = extract_attribute_json(contents, "nextRefill");

    let quota_json: Value = serde_json::from_str(&quota_info)?;
    let next_json: Option<Value> = next_refill.and_then(|raw| serde_json::from_str(&raw).ok());

    let maximum = quota_json
        .get("maximum")
        .and_then(|v| v.as_f64())
        .or_else(|| quota_json.get("max").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);
    let available = quota_json
        .get("tariffQuota")
        .and_then(|v| v.get("available"))
        .and_then(|v| v.as_f64())
        .or_else(|| quota_json.get("available").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);
    if maximum <= 0.0 {
        return Err(anyhow!("JetBrains quota maximum missing"));
    }
    let remaining_percent = (available / maximum) * 100.0;
    let used_percent = (100.0 - remaining_percent).clamp(0.0, 100.0);

    let resets_at = next_json
        .as_ref()
        .and_then(|v| v.get("next"))
        .and_then(|v| v.as_str())
        .and_then(parse_rfc3339);

    let identity_label = derive_jetbrains_identity(path);
    let identity = ProviderIdentitySnapshot {
        provider_id: Some("jetbrains".to_string()),
        account_email: None,
        account_organization: identity_label.clone(),
        login_method: identity_label,
    };

    Ok(UsageSnapshot {
        primary: Some(RateWindow {
            used_percent,
            window_minutes: None,
            resets_at,
            reset_description: None,
        }),
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

fn extract_attribute_json(text: &str, name: &str) -> Option<String> {
    let pattern = format!(r#"name="{name}"\s+value="([^"]+)""#);
    let regex = Regex::new(&pattern).ok()?;
    let caps = regex.captures(text)?;
    let raw = caps.get(1)?.as_str();
    Some(html_unescape(raw))
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&#10;", "\n")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn derive_jetbrains_identity(path: &Path) -> Option<String> {
    let mut current = path.parent();
    while let Some(dir) = current {
        if dir.ends_with("options")
            && let Some(parent) = dir.parent()
        {
            return parent
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string());
        }
        current = dir.parent();
    }
    None
}
