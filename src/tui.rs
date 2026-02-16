use crate::cli::{UsageArgs, collect_usage_outputs};
use crate::config::Config;
use crate::model::{ProviderCostSnapshot, ProviderPayload, RateWindow};
use crate::providers::ProviderRegistry;
use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::io;
use std::time::Duration;

pub async fn run_usage_watch(
    mut args: UsageArgs,
    registry: &ProviderRegistry,
    config: Config,
) -> Result<()> {
    let _guard = TuiGuard::enter()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    if args.interval == 0 {
        args.interval = 10;
    }
    if !args.refresh {
        args.refresh = true;
    }

    let mut state = LiveState::default();
    let mut ticker = tokio::time::interval(Duration::from_secs(args.interval));
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            _ = &mut ctrl_c => break,
            _ = ticker.tick() => {
                state.refresh_count += 1;
                match collect_usage_outputs(&args, &config, registry).await {
                    Ok(outputs) => {
                        state.outputs = outputs;
                        state.last_error = None;
                        state.last_updated = Some(Utc::now());
                    }
                    Err(err) => {
                        state.last_error = Some(err.to_string());
                    }
                }
            }
        }

        terminal.draw(|frame| draw(frame, &args, &state))?;
    }

    Ok(())
}

#[derive(Default)]
struct LiveState {
    outputs: Vec<ProviderPayload>,
    last_updated: Option<DateTime<Utc>>,
    last_error: Option<String>,
    refresh_count: u64,
}

struct TuiGuard;

impl TuiGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for TuiGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
}

fn draw(frame: &mut Frame<'_>, args: &UsageArgs, state: &LiveState) {
    let area = frame.size();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(0)])
        .split(area);

    draw_header(frame, layout[0], args, state);
    draw_body(frame, layout[1], args, state);
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, args: &UsageArgs, state: &LiveState) {
    let title_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let dim_style = Style::default().fg(Color::DarkGray);

    let provider_label = if args.providers.is_empty() {
        "auto".to_string()
    } else {
        args.providers
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let refresh_label = format!("Refresh: {}s", args.interval);
    let source_label = format!("Source: {}", args.source);
    let update_label = match state.last_updated {
        Some(dt) => format!("Last update: {}", format_timestamp(dt)),
        None => "Last update: waiting for first refresh".to_string(),
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Fuelcheck Live", title_style),
            Span::styled(" - usage + cost", dim_style),
        ]),
        Line::from(vec![
            Span::styled(format!("Providers: {}", provider_label), dim_style),
            Span::styled(" | ", dim_style),
            Span::styled(refresh_label, dim_style),
            Span::styled(" | ", dim_style),
            Span::styled(source_label, dim_style),
            Span::styled(" | ", dim_style),
            Span::styled("Ctrl+C to exit", dim_style),
        ]),
        Line::from(vec![Span::styled(update_label, dim_style)]),
        Line::from(vec![Span::styled(
            format!("Refresh count: {}", state.refresh_count),
            dim_style,
        )]),
    ];

    let header = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Status"))
        .wrap(Wrap { trim: false });
    frame.render_widget(header, area);
}

fn draw_body(frame: &mut Frame<'_>, area: Rect, args: &UsageArgs, state: &LiveState) {
    let mut lines = Vec::new();
    if let Some(err) = &state.last_error {
        lines.push(Line::from(Span::styled(
            format!("error: {}", err),
            Style::default().fg(Color::Red),
        )));
    }

    if state.outputs.is_empty() {
        if lines.is_empty() {
            lines.push(Line::from("Waiting for data..."));
        }
    } else {
        for payload in &state.outputs {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.extend(render_payload(payload, args));
        }
    }

    let body = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Usage"))
        .wrap(Wrap { trim: false });
    frame.render_widget(body, area);
}

fn render_payload(payload: &ProviderPayload, args: &UsageArgs) -> Vec<Line<'static>> {
    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let dim_style = Style::default().fg(Color::DarkGray);
    let mut lines = Vec::new();

    let header = provider_header(payload, header_style, dim_style);
    lines.push(header);

    if let Some(error) = &payload.error {
        lines.push(Line::from(Span::styled(
            format!("error: {}", error.message),
            Style::default().fg(Color::Red),
        )));
        return lines;
    }

    if let Some(usage) = &payload.usage {
        if let Some(primary) = usage.primary.as_ref() {
            lines.push(rate_window_line("primary", primary));
        }
        if let Some(secondary) = usage.secondary.as_ref() {
            lines.push(rate_window_line("secondary", secondary));
        }
        if let Some(tertiary) = usage.tertiary.as_ref() {
            lines.push(rate_window_line("tertiary", tertiary));
        }
        if let Some(cost) = usage.provider_cost.as_ref() {
            lines.push(cost_line(cost));
        } else {
            lines.push(Line::from("cost: n/a"));
        }
        if !args.no_credits {
            if let Some(credits) = payload.credits.as_ref() {
                lines.push(Line::from(format!("credits: {:.2}", credits.remaining)));
            } else if let Some(dashboard) = payload.openai_dashboard.as_ref() {
                if let Some(credits) = dashboard.credits_remaining {
                    lines.push(Line::from(format!("credits: {:.2}", credits)));
                }
            }
        }
        lines.push(Line::from(format!(
            "updated: {}",
            format_timestamp(usage.updated_at)
        )));
    } else {
        lines.push(Line::from(Span::styled("no usage data", dim_style)));
    }

    lines
}

fn provider_header(
    payload: &ProviderPayload,
    header_style: Style,
    dim_style: Style,
) -> Line<'static> {
    let mut label = payload.provider.clone();
    if let Some(version) = &payload.version {
        label.push(' ');
        label.push_str(version);
    }
    let header = format!("{} ({})", label, payload.source);

    let mut spans = vec![Span::styled(header, header_style)];

    if let Some(account) = resolve_account(payload) {
        spans.push(Span::styled(format!(" | account: {}", account), dim_style));
    }
    if let Some(plan) = payload
        .usage
        .as_ref()
        .and_then(|usage| usage.login_method.clone())
    {
        spans.push(Span::styled(format!(" | plan: {}", plan), dim_style));
    }

    Line::from(spans)
}

fn resolve_account(payload: &ProviderPayload) -> Option<String> {
    payload
        .account
        .clone()
        .or_else(|| payload.usage.as_ref().and_then(|u| u.account_email.clone()))
        .or_else(|| {
            payload
                .usage
                .as_ref()
                .and_then(|u| u.account_organization.clone())
        })
}

fn rate_window_line(label: &str, window: &RateWindow) -> Line<'static> {
    let bar = percent_bar(window.used_percent, 18);
    let mut parts = vec![format!(
        "{}: {:>5.1}% [{}]",
        label, window.used_percent, bar
    )];
    if let Some(desc) = &window.reset_description {
        parts.push(desc.clone());
    }
    if let Some(minutes) = window.window_minutes {
        parts.push(format!("window {}m", minutes));
    }

    let style = usage_style(window.used_percent);
    Line::from(Span::styled(parts.join(" | "), style))
}

fn cost_line(cost: &ProviderCostSnapshot) -> Line<'static> {
    let mut parts = vec![format!(
        "cost: {:.2}/{:.2} {}",
        cost.used, cost.limit, cost.currency_code
    )];
    if let Some(period) = &cost.period {
        parts.push(period.clone());
    }
    if let Some(resets_at) = cost.resets_at {
        parts.push(format!("resets {}", format_timestamp(resets_at)));
    }
    Line::from(parts.join(" | "))
}

fn usage_style(percent: f64) -> Style {
    if percent >= 90.0 {
        Style::default().fg(Color::Red)
    } else if percent >= 75.0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Green)
    }
}

fn percent_bar(percent: f64, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let clamped = percent.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let mut bar = String::with_capacity(width);
    for _ in 0..filled {
        bar.push('#');
    }
    for _ in filled..width {
        bar.push('-');
    }
    bar
}

fn format_timestamp(dt: DateTime<Utc>) -> String {
    dt.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}
