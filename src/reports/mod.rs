pub mod codex;
pub mod render;
pub mod types;

use crate::model::{ErrorKind, ProviderErrorPayload};
use crate::providers::ProviderId;
use anyhow::{Result, anyhow};
use chrono_tz::Tz;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

pub use types::{
    CostReportCollection, CostReportKind, ProviderReportOutcome, ProviderReportResult,
};

pub struct CostReportRequest<'a> {
    pub report: CostReportKind,
    pub providers: Vec<ProviderId>,
    pub since: Option<&'a str>,
    pub until: Option<&'a str>,
    pub timezone: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct ValidatedReportFilters {
    pub since: Option<String>,
    pub until: Option<String>,
    pub timezone: Option<String>,
}

pub fn validate_report_filters(
    since: Option<&str>,
    until: Option<&str>,
    timezone: Option<&str>,
) -> Result<ValidatedReportFilters> {
    let since = normalize_filter_date(since)?;
    let until = normalize_filter_date(until)?;

    if let (Some(since), Some(until)) = (&since, &until)
        && since > until
    {
        return Err(anyhow!("--since must be less than or equal to --until"));
    }

    let timezone = match timezone {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(anyhow!("timezone cannot be empty"));
            }
            trimmed
                .parse::<Tz>()
                .map_err(|_| anyhow!("invalid timezone: {}", trimmed))?;
            Some(trimmed.to_string())
        }
        None => None,
    };

    Ok(ValidatedReportFilters {
        since,
        until,
        timezone,
    })
}

pub fn build_cost_report_collection(
    request: CostReportRequest<'_>,
) -> Result<CostReportCollection> {
    let filters = validate_report_filters(request.since, request.until, request.timezone)?;

    let mut providers = Vec::new();
    for provider_id in request.providers {
        let outcome = match provider_id {
            ProviderId::Codex => {
                let options = codex::CodexReportOptions {
                    report: request.report,
                    since: filters.since.as_deref(),
                    until: filters.until.as_deref(),
                    timezone: filters.timezone.as_deref(),
                };
                match codex::build_report(&options) {
                    Ok(report) => ProviderReportOutcome::Report(report),
                    Err(err) => {
                        ProviderReportOutcome::Error(provider_error_payload_from_error(&err))
                    }
                }
            }
            _ => ProviderReportOutcome::Error(ProviderErrorPayload {
                code: 1,
                message: format!("provider {} report not implemented yet", provider_id),
                kind: Some(ErrorKind::Provider),
            }),
        };

        providers.push(ProviderReportResult {
            provider: provider_id.to_string(),
            outcome,
        });
    }

    Ok(CostReportCollection {
        report: request.report,
        providers,
    })
}

pub fn collection_to_json_value(collection: &CostReportCollection) -> Result<Value> {
    if collection.providers.len() == 1 {
        let single = collection
            .providers
            .first()
            .expect("single provider must exist");
        return match &single.outcome {
            ProviderReportOutcome::Report(report) => {
                serde_json::to_value(report).map_err(Into::into)
            }
            ProviderReportOutcome::Error(error) => Ok(json!({ "error": error })),
        };
    }

    let mut providers_json = Map::new();
    for provider in &collection.providers {
        let value = match &provider.outcome {
            ProviderReportOutcome::Report(report) => serde_json::to_value(report)?,
            ProviderReportOutcome::Error(error) => json!({ "error": error }),
        };
        providers_json.insert(provider.provider.clone(), value);
    }

    Ok(Value::Object(Map::from_iter([
        (
            "report".to_string(),
            Value::String(collection.report.to_string()),
        ),
        ("providers".to_string(), Value::Object(providers_json)),
    ])))
}

pub fn render_collection_text(
    collection: &CostReportCollection,
    force_compact: bool,
    timezone: Option<&str>,
) -> String {
    let render_options = render::RenderOptions {
        force_compact,
        timezone,
        compact_override: None,
    };

    let mut sections = Vec::new();
    for provider in &collection.providers {
        let section = match &provider.outcome {
            ProviderReportOutcome::Report(report) => {
                render::render_provider_report(&provider.provider, report, &render_options)
            }
            ProviderReportOutcome::Error(error) => {
                format!(
                    "== {} report ({}) ==\nerror: {}",
                    provider.provider, collection.report, error.message
                )
            }
        };
        sections.push(section);
    }

    sections.join("\n\n")
}

pub fn provider_error_payload_from_error(err: &anyhow::Error) -> ProviderErrorPayload {
    ProviderErrorPayload {
        code: 1,
        message: format_error_chain(err),
        kind: Some(ErrorKind::Provider),
    }
}

fn format_error_chain(err: &anyhow::Error) -> String {
    let mut parts: Vec<String> = err.chain().map(|e| e.to_string()).collect();
    parts.dedup();
    parts.join(": ")
}

fn normalize_filter_date(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };

    let compact = value.trim().replace('-', "");
    if compact.is_empty() {
        return Ok(None);
    }
    if compact.len() != 8 || !compact.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(anyhow!(
            "invalid date format: {}. expected YYYYMMDD or YYYY-MM-DD",
            value
        ));
    }

    Ok(Some(format!(
        "{}-{}-{}",
        &compact[0..4],
        &compact[4..6],
        &compact[6..8]
    )))
}

pub fn normalize_model_name(model: &str) -> String {
    let trimmed = model.trim();
    for prefix in ["openrouter/openai/", "openai/", "azure/"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.to_string();
        }
    }
    trimmed.to_string()
}

pub fn annotate_models_with_fallback(models: &BTreeMap<String, types::ModelUsage>) -> Vec<String> {
    models
        .iter()
        .map(|(name, usage)| {
            if usage.is_fallback == Some(true) {
                format!("{} (fallback)", name)
            } else {
                name.clone()
            }
        })
        .collect()
}
