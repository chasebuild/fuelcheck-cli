use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use crossterm::terminal;
use fuelcheck_core::reports::annotate_models_with_fallback;
use fuelcheck_core::reports::types::{
    DailyReportResponse, MonthlyReportResponse, ProviderReport, SessionReportResponse,
    split_usage_tokens,
};
use fuelcheck_core::reports::{CostReportCollection, ProviderReportOutcome};

pub struct RenderOptions<'a> {
    pub force_compact: bool,
    pub timezone: Option<&'a str>,
    pub compact_override: Option<bool>,
}

pub fn render_collection_text(
    collection: &CostReportCollection,
    force_compact: bool,
    timezone: Option<&str>,
) -> String {
    let render_options = RenderOptions {
        force_compact,
        timezone,
        compact_override: None,
    };

    let mut sections = Vec::new();
    for provider in &collection.providers {
        let section = match &provider.outcome {
            ProviderReportOutcome::Report(report) => {
                render_provider_report(&provider.provider, report, &render_options)
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

pub fn render_provider_report(
    provider: &str,
    report: &ProviderReport,
    options: &RenderOptions<'_>,
) -> String {
    let timezone = parse_timezone_or_utc(options.timezone);
    let compact = options
        .compact_override
        .unwrap_or_else(|| options.force_compact || is_compact_terminal());

    let mut out = String::new();
    out.push_str(&format!("== {} report ({}) ==\n", provider, report.kind()));

    let table = match report {
        ProviderReport::Daily(data) => render_daily(data, compact),
        ProviderReport::Monthly(data) => render_monthly(data, compact),
        ProviderReport::Session(data) => render_sessions(data, compact, timezone),
    };
    out.push_str(&table);

    if compact {
        out.push_str("\n\nRunning in Compact Mode");
    }

    out
}

fn render_daily(data: &DailyReportResponse, compact: bool) -> String {
    if compact {
        let headers = ["Date", "Models", "Input", "Output", "Cost (USD)"];
        let mut rows = Vec::new();
        for row in &data.daily {
            let split = split_usage_tokens(
                row.input_tokens,
                row.cached_input_tokens,
                row.output_tokens,
                row.reasoning_output_tokens,
            );
            rows.push(vec![
                row.date.clone(),
                annotate_models_with_fallback(&row.models).join(", "),
                format_number(split.input_tokens),
                format_number(split.output_tokens),
                format_currency(row.cost_usd),
            ]);
        }

        let totals = split_usage_tokens(
            data.totals.input_tokens,
            data.totals.cached_input_tokens,
            data.totals.output_tokens,
            data.totals.reasoning_output_tokens,
        );
        rows.push(vec![
            "Total".to_string(),
            String::new(),
            format_number(totals.input_tokens),
            format_number(totals.output_tokens),
            format_currency(data.totals.cost_usd),
        ]);
        return render_table(&headers, &rows);
    }

    let headers = [
        "Date",
        "Models",
        "Input",
        "Output",
        "Reasoning",
        "Cache Read",
        "Total Tokens",
        "Cost (USD)",
    ];
    let mut rows = Vec::new();

    for row in &data.daily {
        let split = split_usage_tokens(
            row.input_tokens,
            row.cached_input_tokens,
            row.output_tokens,
            row.reasoning_output_tokens,
        );
        rows.push(vec![
            row.date.clone(),
            annotate_models_with_fallback(&row.models).join(", "),
            format_number(split.input_tokens),
            format_number(split.output_tokens),
            format_number(split.reasoning_tokens),
            format_number(split.cache_read_tokens),
            format_number(row.total_tokens),
            format_currency(row.cost_usd),
        ]);
    }

    let totals = split_usage_tokens(
        data.totals.input_tokens,
        data.totals.cached_input_tokens,
        data.totals.output_tokens,
        data.totals.reasoning_output_tokens,
    );
    rows.push(vec![
        "Total".to_string(),
        String::new(),
        format_number(totals.input_tokens),
        format_number(totals.output_tokens),
        format_number(totals.reasoning_tokens),
        format_number(totals.cache_read_tokens),
        format_number(data.totals.total_tokens),
        format_currency(data.totals.cost_usd),
    ]);

    render_table(&headers, &rows)
}

fn render_monthly(data: &MonthlyReportResponse, compact: bool) -> String {
    if compact {
        let headers = ["Month", "Models", "Input", "Output", "Cost (USD)"];
        let mut rows = Vec::new();
        for row in &data.monthly {
            let split = split_usage_tokens(
                row.input_tokens,
                row.cached_input_tokens,
                row.output_tokens,
                row.reasoning_output_tokens,
            );
            rows.push(vec![
                row.month.clone(),
                annotate_models_with_fallback(&row.models).join(", "),
                format_number(split.input_tokens),
                format_number(split.output_tokens),
                format_currency(row.cost_usd),
            ]);
        }

        let totals = split_usage_tokens(
            data.totals.input_tokens,
            data.totals.cached_input_tokens,
            data.totals.output_tokens,
            data.totals.reasoning_output_tokens,
        );
        rows.push(vec![
            "Total".to_string(),
            String::new(),
            format_number(totals.input_tokens),
            format_number(totals.output_tokens),
            format_currency(data.totals.cost_usd),
        ]);
        return render_table(&headers, &rows);
    }

    let headers = [
        "Month",
        "Models",
        "Input",
        "Output",
        "Reasoning",
        "Cache Read",
        "Total Tokens",
        "Cost (USD)",
    ];
    let mut rows = Vec::new();

    for row in &data.monthly {
        let split = split_usage_tokens(
            row.input_tokens,
            row.cached_input_tokens,
            row.output_tokens,
            row.reasoning_output_tokens,
        );
        rows.push(vec![
            row.month.clone(),
            annotate_models_with_fallback(&row.models).join(", "),
            format_number(split.input_tokens),
            format_number(split.output_tokens),
            format_number(split.reasoning_tokens),
            format_number(split.cache_read_tokens),
            format_number(row.total_tokens),
            format_currency(row.cost_usd),
        ]);
    }

    let totals = split_usage_tokens(
        data.totals.input_tokens,
        data.totals.cached_input_tokens,
        data.totals.output_tokens,
        data.totals.reasoning_output_tokens,
    );
    rows.push(vec![
        "Total".to_string(),
        String::new(),
        format_number(totals.input_tokens),
        format_number(totals.output_tokens),
        format_number(totals.reasoning_tokens),
        format_number(totals.cache_read_tokens),
        format_number(data.totals.total_tokens),
        format_currency(data.totals.cost_usd),
    ]);

    render_table(&headers, &rows)
}

fn render_sessions(data: &SessionReportResponse, compact: bool, timezone: Tz) -> String {
    if compact {
        let headers = [
            "Date",
            "Directory",
            "Session",
            "Input",
            "Output",
            "Cost (USD)",
        ];
        let mut rows = Vec::new();

        for row in &data.sessions {
            let split = split_usage_tokens(
                row.input_tokens,
                row.cached_input_tokens,
                row.output_tokens,
                row.reasoning_output_tokens,
            );
            rows.push(vec![
                format_session_date(&row.last_activity, timezone),
                if row.directory.is_empty() {
                    "-".to_string()
                } else {
                    row.directory.clone()
                },
                shorten_session(&row.session_file),
                format_number(split.input_tokens),
                format_number(split.output_tokens),
                format_currency(row.cost_usd),
            ]);
        }

        let totals = split_usage_tokens(
            data.totals.input_tokens,
            data.totals.cached_input_tokens,
            data.totals.output_tokens,
            data.totals.reasoning_output_tokens,
        );
        rows.push(vec![
            String::new(),
            String::new(),
            "Total".to_string(),
            format_number(totals.input_tokens),
            format_number(totals.output_tokens),
            format_currency(data.totals.cost_usd),
        ]);

        return render_table(&headers, &rows);
    }

    let headers = [
        "Date",
        "Directory",
        "Session",
        "Models",
        "Input",
        "Output",
        "Reasoning",
        "Cache Read",
        "Total Tokens",
        "Cost (USD)",
        "Last Activity",
    ];
    let mut rows = Vec::new();

    for row in &data.sessions {
        let split = split_usage_tokens(
            row.input_tokens,
            row.cached_input_tokens,
            row.output_tokens,
            row.reasoning_output_tokens,
        );
        rows.push(vec![
            format_session_date(&row.last_activity, timezone),
            if row.directory.is_empty() {
                "-".to_string()
            } else {
                row.directory.clone()
            },
            shorten_session(&row.session_file),
            annotate_models_with_fallback(&row.models).join(", "),
            format_number(split.input_tokens),
            format_number(split.output_tokens),
            format_number(split.reasoning_tokens),
            format_number(split.cache_read_tokens),
            format_number(row.total_tokens),
            format_currency(row.cost_usd),
            format_session_datetime(&row.last_activity, timezone),
        ]);
    }

    let totals = split_usage_tokens(
        data.totals.input_tokens,
        data.totals.cached_input_tokens,
        data.totals.output_tokens,
        data.totals.reasoning_output_tokens,
    );
    rows.push(vec![
        String::new(),
        String::new(),
        "Total".to_string(),
        String::new(),
        format_number(totals.input_tokens),
        format_number(totals.output_tokens),
        format_number(totals.reasoning_tokens),
        format_number(totals.cache_read_tokens),
        format_number(data.totals.total_tokens),
        format_currency(data.totals.cost_usd),
        String::new(),
    ]);

    render_table(&headers, &rows)
}

fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();

    for row in rows {
        for (idx, value) in row.iter().enumerate() {
            if idx >= widths.len() {
                widths.push(value.len());
            } else {
                widths[idx] = widths[idx].max(value.len());
            }
        }
    }

    let mut output = String::new();
    output.push_str(&render_row(
        &headers.iter().map(|h| h.to_string()).collect::<Vec<_>>(),
        &widths,
    ));
    output.push('\n');
    output.push_str(&render_separator(&widths));

    for row in rows {
        output.push('\n');
        output.push_str(&render_row(row, &widths));
    }

    output
}

fn render_row(row: &[String], widths: &[usize]) -> String {
    row.iter()
        .enumerate()
        .map(|(idx, value)| format!("{value:<width$}", width = widths[idx]))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn render_separator(widths: &[usize]) -> String {
    widths
        .iter()
        .map(|width| "-".repeat(*width))
        .collect::<Vec<_>>()
        .join("-+-")
}

fn is_compact_terminal() -> bool {
    terminal::size()
        .map(|(width, _)| width < 100)
        .unwrap_or(false)
}

fn format_number(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (idx, ch) in digits.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn format_currency(value: f64) -> String {
    format!("{:.4}", value)
}

fn parse_timezone_or_utc(raw: Option<&str>) -> Tz {
    raw.and_then(|value| value.parse::<Tz>().ok())
        .unwrap_or(chrono_tz::UTC)
}

fn format_session_date(timestamp: &str, timezone: Tz) -> String {
    parse_timestamp(timestamp)
        .map(|value| {
            value
                .with_timezone(&timezone)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|| "-".to_string())
}

fn format_session_datetime(timestamp: &str, timezone: Tz) -> String {
    parse_timestamp(timestamp)
        .map(|value| {
            value
                .with_timezone(&timezone)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "-".to_string())
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

fn shorten_session(value: &str) -> String {
    if value.len() <= 8 {
        value.to_string()
    } else {
        format!("…{}", &value[value.len() - 8..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuelcheck_core::reports::types::{
        DailyReportResponse, DailyReportRow, ModelUsage, ProviderReport, ReportTotals,
        SessionReportResponse, SessionReportRow,
    };
    use std::collections::BTreeMap;

    #[test]
    fn renders_daily_full_columns() {
        let mut models = BTreeMap::new();
        models.insert(
            "gpt-5".to_string(),
            ModelUsage {
                input_tokens: 1200,
                cached_input_tokens: 200,
                output_tokens: 500,
                reasoning_output_tokens: 10,
                total_tokens: 1700,
                is_fallback: None,
            },
        );

        let report = ProviderReport::Daily(DailyReportResponse {
            daily: vec![DailyReportRow {
                date: "2025-09-11".to_string(),
                input_tokens: 1200,
                cached_input_tokens: 200,
                output_tokens: 500,
                reasoning_output_tokens: 10,
                total_tokens: 1700,
                cost_usd: 0.1234,
                models,
            }],
            totals: ReportTotals {
                input_tokens: 1200,
                cached_input_tokens: 200,
                output_tokens: 500,
                reasoning_output_tokens: 10,
                total_tokens: 1700,
                cost_usd: 0.1234,
            },
        });

        let text = render_provider_report(
            "codex",
            &report,
            &RenderOptions {
                force_compact: false,
                timezone: Some("UTC"),
                compact_override: Some(false),
            },
        );

        assert!(text.contains("Reasoning"));
        assert!(text.contains("Cache Read"));
        assert!(text.contains("Total Tokens"));
    }

    #[test]
    fn renders_daily_compact_columns() {
        let report = ProviderReport::Daily(DailyReportResponse {
            daily: vec![],
            totals: ReportTotals::default(),
        });

        let text = render_provider_report(
            "codex",
            &report,
            &RenderOptions {
                force_compact: false,
                timezone: Some("UTC"),
                compact_override: Some(true),
            },
        );

        assert!(text.contains("Input"));
        assert!(text.contains("Output"));
        assert!(text.contains("Cost (USD)"));
        assert!(!text.contains("Reasoning"));
    }

    #[test]
    fn renders_session_totals_row() {
        let mut models = BTreeMap::new();
        models.insert(
            "gpt-5".to_string(),
            ModelUsage {
                input_tokens: 100,
                cached_input_tokens: 10,
                output_tokens: 20,
                reasoning_output_tokens: 3,
                total_tokens: 120,
                is_fallback: None,
            },
        );

        let report = ProviderReport::Session(SessionReportResponse {
            sessions: vec![SessionReportRow {
                session_id: "proj/a-session".to_string(),
                last_activity: "2025-09-11T18:25:40Z".to_string(),
                session_file: "a-session".to_string(),
                directory: "proj".to_string(),
                input_tokens: 100,
                cached_input_tokens: 10,
                output_tokens: 20,
                reasoning_output_tokens: 3,
                total_tokens: 120,
                cost_usd: 0.001,
                models,
            }],
            totals: ReportTotals {
                input_tokens: 100,
                cached_input_tokens: 10,
                output_tokens: 20,
                reasoning_output_tokens: 3,
                total_tokens: 120,
                cost_usd: 0.001,
            },
        });

        let text = render_provider_report(
            "codex",
            &report,
            &RenderOptions {
                force_compact: false,
                timezone: Some("UTC"),
                compact_override: Some(false),
            },
        );

        assert!(text.contains("Total"));
        assert!(text.contains("120"));
        assert!(text.contains("0.0010"));
    }
}
