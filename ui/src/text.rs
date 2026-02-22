use anyhow::Result;
use fuelcheck_core::model::{
    OutputFormat, ProviderCostSnapshot, ProviderPayload, ProviderStatusIndicator,
    ProviderStatusPayload, RateWindow,
};

#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub format: OutputFormat,
    pub pretty: bool,
    pub json_only: bool,
    pub use_color: bool,
}

pub fn render_outputs(
    outputs: &[ProviderPayload],
    options: &RenderOptions,
) -> Result<Option<String>> {
    match options.format {
        OutputFormat::Json => {
            let json = if options.pretty {
                serde_json::to_string_pretty(outputs)?
            } else {
                serde_json::to_string(outputs)?
            };
            Ok(Some(json))
        }
        OutputFormat::Text => {
            if options.json_only {
                return Ok(None);
            }
            let text = outputs
                .iter()
                .map(|output| format_payload_text(output, options))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(Some(text))
        }
    }
}

pub fn format_payload_text(payload: &ProviderPayload, options: &RenderOptions) -> String {
    if let Some(error) = &payload.error {
        return format!("{}: error: {}", payload.provider, error.message);
    }

    let mut lines = Vec::new();
    let header = format!(
        "== {} ==",
        format_header_title(
            provider_display_name(&payload.provider),
            payload.version.as_deref(),
            &payload.source
        )
    );
    lines.push(colorize_header(&header, options.use_color));

    if let Some(usage) = &payload.usage {
        if let Some(primary) = &usage.primary {
            lines.push(rate_line("Session", primary, options.use_color));
            if let Some(reset) = reset_line(primary) {
                lines.push(subtle_line(&reset, options.use_color));
            }
        }
        if let Some(secondary) = &usage.secondary {
            lines.push(rate_line("Weekly", secondary, options.use_color));
            if let Some(pace) = pace_line(&payload.provider, secondary) {
                lines.push(label_line("Pace", &pace, options.use_color));
            }
            if let Some(reset) = reset_line(secondary) {
                lines.push(subtle_line(&reset, options.use_color));
            }
        }
        if let Some(tertiary) = &usage.tertiary {
            let label = tertiary_label(&payload.provider);
            lines.push(rate_line(label, tertiary, options.use_color));
            if let Some(reset) = reset_line(tertiary) {
                lines.push(subtle_line(&reset, options.use_color));
            }
        }
        if let Some(cost) = &usage.provider_cost {
            lines.push(cost_line(cost));
        }
        if payload.provider == "codex" {
            if let Some(credits) = &payload.credits {
                lines.push(label_line(
                    "Credits",
                    &format_credits(credits.remaining),
                    options.use_color,
                ));
            } else if let Some(dashboard) = &payload.openai_dashboard
                && let Some(credits) = dashboard.credits_remaining
            {
                lines.push(label_line(
                    "Credits",
                    &format_credits(credits),
                    options.use_color,
                ));
            }
        }
        if let Some(account) = usage.account_email.clone().or_else(|| {
            usage
                .identity
                .as_ref()
                .and_then(|i| i.account_email.clone())
        }) {
            lines.push(label_line("Account", &account, options.use_color));
        }
        if let Some(plan) = usage
            .login_method
            .clone()
            .or_else(|| usage.identity.as_ref().and_then(|i| i.login_method.clone()))
            && !plan.is_empty()
        {
            lines.push(label_line("Plan", &plan, options.use_color));
        }
    }

    if let Some(status) = &payload.status {
        let status_text = status_line(status);
        lines.push(colorize_status(
            &status_text,
            status.indicator.clone(),
            options.use_color,
        ));
    }

    lines.join("\n")
}

fn format_header_title(provider: String, version: Option<&str>, source: &str) -> String {
    match version {
        Some(ver) => format!("{} {} ({})", provider, ver, source),
        None => format!("{} ({})", provider, source),
    }
}

fn provider_display_name(raw: &str) -> String {
    match raw {
        "codex" => "Codex".to_string(),
        "claude" => "Claude".to_string(),
        "gemini" => "Gemini".to_string(),
        "cursor" => "Cursor".to_string(),
        "factory" => "Factory".to_string(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => other.to_string(),
            }
        }
    }
}

fn tertiary_label(provider: &str) -> &'static str {
    match provider {
        "claude" => "Sonnet",
        _ => "Tertiary",
    }
}

fn rate_line(label: &str, window: &RateWindow, use_color: bool) -> String {
    let remaining = remaining_percent(window.used_percent);
    let usage_text = usage_line(remaining, window.used_percent);
    let colored_usage = colorize_usage(&usage_text, remaining, use_color);
    let bar = usage_bar(remaining, use_color);
    format!("{}: {} {}", label, colored_usage, bar)
}

fn usage_line(remaining: f64, used: f64) -> String {
    let percent = remaining.clamp(0.0, 100.0);
    if used.is_nan() {
        format!("{:.0}% left", percent)
    } else {
        format!("{:.0}% left", percent)
    }
}

fn remaining_percent(used_percent: f64) -> f64 {
    (100.0 - used_percent).clamp(0.0, 100.0)
}

fn usage_bar(remaining: f64, use_color: bool) -> String {
    let clamped = remaining.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * 12.0).round() as usize;
    let filled = filled.min(12);
    let empty = 12 - filled;
    let bar = format!("[{}{}]", "=".repeat(filled), "-".repeat(empty));
    if use_color { ansi("95", &bar) } else { bar }
}

fn reset_line(window: &RateWindow) -> Option<String> {
    if let Some(resets_at) = window.resets_at {
        return Some(format!("Resets {}", reset_countdown_description(resets_at)));
    }
    if let Some(desc) = &window.reset_description {
        let trimmed = desc.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.to_lowercase().starts_with("resets") {
            return Some(trimmed.to_string());
        }
        return Some(format!("Resets {}", trimmed));
    }
    None
}

fn reset_countdown_description(resets_at: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let delta = resets_at.signed_duration_since(now);
    if delta.num_seconds() < 1 {
        return "now".to_string();
    }
    let total_minutes = (delta.num_seconds() as f64 / 60.0).ceil() as i64;
    let total_minutes = total_minutes.max(1);
    let days = total_minutes / (24 * 60);
    let hours = (total_minutes / 60) % 24;
    let minutes = total_minutes % 60;
    if days > 0 {
        if hours > 0 {
            return format!("in {}d {}h", days, hours);
        }
        return format!("in {}d", days);
    }
    if hours > 0 {
        if minutes > 0 {
            return format!("in {}h {}m", hours, minutes);
        }
        return format!("in {}h", hours);
    }
    format!("in {}m", minutes)
}

fn pace_line(provider: &str, window: &RateWindow) -> Option<String> {
    if provider != "codex" && provider != "claude" {
        return None;
    }
    if remaining_percent(window.used_percent) <= 0.0 {
        return None;
    }
    let pace = usage_pace_weekly(window)?;
    if pace.expected_used_percent < 3.0 {
        return None;
    }
    let expected = pace.expected_used_percent.round() as i64;
    let mut parts = Vec::new();
    parts.push(pace_left_label(&pace));
    parts.push(format!("Expected {}% used", expected));
    if let Some(right) = pace_right_label(&pace) {
        parts.push(right);
    }
    Some(parts.join(" | "))
}

struct UsagePaceSummary {
    stage: UsagePaceStage,
    delta_percent: f64,
    expected_used_percent: f64,
    actual_used_percent: f64,
    eta_seconds: Option<i64>,
    will_last_to_reset: bool,
}

enum UsagePaceStage {
    OnTrack,
    SlightlyAhead,
    Ahead,
    FarAhead,
    SlightlyBehind,
    Behind,
    FarBehind,
}

fn usage_pace_weekly(window: &RateWindow) -> Option<UsagePaceSummary> {
    let resets_at = window.resets_at?;
    let minutes = window.window_minutes.unwrap_or(10080);
    if minutes <= 0 {
        return None;
    }
    let now = chrono::Utc::now();
    let duration_secs = minutes * 60;
    let time_until_reset = (resets_at - now).num_seconds();
    if time_until_reset <= 0 || time_until_reset > duration_secs {
        return None;
    }
    let elapsed = (duration_secs - time_until_reset).clamp(0, duration_secs);
    let expected = ((elapsed as f64 / duration_secs as f64) * 100.0).clamp(0.0, 100.0);
    let actual = window.used_percent.clamp(0.0, 100.0);
    if elapsed == 0 && actual > 0.0 {
        return None;
    }
    let delta = actual - expected;
    let stage = usage_pace_stage(delta);

    let mut eta_seconds = None;
    let mut will_last_to_reset = false;
    if elapsed > 0 && actual > 0.0 {
        let rate = actual / elapsed as f64;
        if rate > 0.0 {
            let remaining = (100.0 - actual).max(0.0);
            let candidate = (remaining / rate).round() as i64;
            if candidate >= time_until_reset {
                will_last_to_reset = true;
            } else {
                eta_seconds = Some(candidate);
            }
        }
    } else if elapsed > 0 && actual == 0.0 {
        will_last_to_reset = true;
    }

    Some(UsagePaceSummary {
        stage,
        delta_percent: delta,
        expected_used_percent: expected,
        actual_used_percent: actual,
        eta_seconds,
        will_last_to_reset,
    })
}

fn usage_pace_stage(delta: f64) -> UsagePaceStage {
    let abs_delta = delta.abs();
    if abs_delta <= 2.0 {
        UsagePaceStage::OnTrack
    } else if abs_delta <= 6.0 {
        if delta >= 0.0 {
            UsagePaceStage::SlightlyAhead
        } else {
            UsagePaceStage::SlightlyBehind
        }
    } else if abs_delta <= 12.0 {
        if delta >= 0.0 {
            UsagePaceStage::Ahead
        } else {
            UsagePaceStage::Behind
        }
    } else if delta >= 0.0 {
        UsagePaceStage::FarAhead
    } else {
        UsagePaceStage::FarBehind
    }
}

fn pace_left_label(pace: &UsagePaceSummary) -> String {
    let delta = pace.delta_percent.abs().round() as i64;
    match pace.stage {
        UsagePaceStage::OnTrack => "On pace".to_string(),
        UsagePaceStage::SlightlyAhead | UsagePaceStage::Ahead | UsagePaceStage::FarAhead => {
            format!("{}% in deficit", delta)
        }
        UsagePaceStage::SlightlyBehind | UsagePaceStage::Behind | UsagePaceStage::FarBehind => {
            format!("{}% in reserve", delta)
        }
    }
}

fn pace_right_label(pace: &UsagePaceSummary) -> Option<String> {
    if pace.will_last_to_reset {
        return Some("Lasts until reset".to_string());
    }
    let eta = pace.eta_seconds?;
    let text = pace_duration_text(eta);
    if text == "now" {
        Some("Runs out now".to_string())
    } else {
        Some(format!("Runs out in {}", text))
    }
}

fn pace_duration_text(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 1 {
        return "now".to_string();
    }
    let minutes = ((seconds as f64) / 60.0).ceil() as i64;
    let minutes = minutes.max(1);
    let days = minutes / (24 * 60);
    let hours = (minutes / 60) % 24;
    let mins = minutes % 60;
    if days > 0 {
        if hours > 0 {
            return format!("{}d {}h", days, hours);
        }
        return format!("{}d", days);
    }
    if hours > 0 {
        if mins > 0 {
            return format!("{}h {}m", hours, mins);
        }
        return format!("{}h", hours);
    }
    format!("{}m", mins)
}

fn cost_line(cost: &ProviderCostSnapshot) -> String {
    let mut parts = vec![format!(
        "Cost: {:.1} / {:.1} {}",
        cost.used, cost.limit, cost.currency_code
    )];
    if let Some(period) = &cost.period {
        parts.push(period.clone());
    }
    if let Some(resets_at) = cost.resets_at {
        parts.push(format!("Resets {}", reset_countdown_description(resets_at)));
    }
    parts.join(" | ")
}

fn label_line(label: &str, value: &str, use_color: bool) -> String {
    let label_text = if use_color {
        ansi("95", label)
    } else {
        label.to_string()
    };
    format!("{}: {}", label_text, value)
}

fn subtle_line(text: &str, use_color: bool) -> String {
    if use_color {
        ansi("90", text)
    } else {
        text.to_string()
    }
}

fn colorize_header(text: &str, use_color: bool) -> String {
    if use_color {
        ansi("1;95", text)
    } else {
        text.to_string()
    }
}

fn colorize_usage(text: &str, remaining_percent: f64, use_color: bool) -> String {
    if !use_color {
        return text.to_string();
    }
    let code = if remaining_percent < 10.0 {
        "31"
    } else if remaining_percent < 25.0 {
        "33"
    } else {
        "32"
    };
    ansi(code, text)
}

fn colorize_status(text: &str, indicator: ProviderStatusIndicator, use_color: bool) -> String {
    if !use_color {
        return text.to_string();
    }
    let code = match indicator {
        ProviderStatusIndicator::None => "32",
        ProviderStatusIndicator::Minor => "33",
        ProviderStatusIndicator::Major | ProviderStatusIndicator::Critical => "31",
        ProviderStatusIndicator::Maintenance => "34",
        ProviderStatusIndicator::Unknown => "90",
    };
    ansi(code, text)
}

fn status_line(status: &ProviderStatusPayload) -> String {
    let label = match status.indicator.clone() {
        ProviderStatusIndicator::None => "Operational",
        ProviderStatusIndicator::Minor => "Partial outage",
        ProviderStatusIndicator::Major => "Major outage",
        ProviderStatusIndicator::Critical => "Critical issue",
        ProviderStatusIndicator::Maintenance => "Maintenance",
        ProviderStatusIndicator::Unknown => "Status unknown",
    };
    let mut text = format!("Status: {}", label);
    if let Some(desc) = &status.description
        && !desc.trim().is_empty()
    {
        text.push_str(&format!(" - {}", desc));
    }
    text
}

fn format_credits(value: f64) -> String {
    let formatted = format!("{:.2}", value);
    format!("{} left", add_thousand_separators(&formatted))
}

fn add_thousand_separators(value: &str) -> String {
    let mut parts = value.splitn(2, '.');
    let int_part = parts.next().unwrap_or("");
    let frac_part = parts.next();
    let mut chars: Vec<char> = int_part.chars().collect();
    let mut out = String::new();
    let mut count = 0;
    while let Some(ch) = chars.pop() {
        if count == 3 {
            out.push(',');
            count = 0;
        }
        out.push(ch);
        count += 1;
    }
    let int_rev: String = out.chars().rev().collect();
    if let Some(frac) = frac_part {
        format!("{}.{}", int_rev, frac)
    } else {
        int_rev
    }
}

fn ansi(code: &str, text: &str) -> String {
    format!("\u{001B}[{}m{}\u{001B}[0m", code, text)
}
