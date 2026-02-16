use crate::cli::{UsageArgs, collect_usage_outputs};
use crate::config::Config;
use crate::model::{ProviderCostSnapshot, ProviderPayload, RateWindow};
use crate::providers::ProviderRegistry;
use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs, Wrap};
use ratatui::{Frame, Terminal};
use std::collections::HashSet;
use std::io;
use std::time::Duration;

#[derive(Clone, Copy)]
struct TuiTheme {
    accent: Color,
    dim: Color,
    alert: Color,
}

impl TuiTheme {
    fn accent_style(self) -> Style {
        Style::default().fg(self.accent)
    }

    fn accent_bold(self) -> Style {
        Style::default().fg(self.accent).add_modifier(Modifier::BOLD)
    }

    fn dim_style(self) -> Style {
        Style::default().fg(self.dim)
    }

    fn alert_style(self) -> Style {
        Style::default().fg(self.alert).add_modifier(Modifier::BOLD)
    }
}

impl Default for TuiTheme {
    fn default() -> Self {
        Self {
            accent: Color::Cyan,
            dim: Color::DarkGray,
            alert: Color::Red,
        }
    }
}

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
    let mut ui_tick = tokio::time::interval(Duration::from_millis(100));
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let mut needs_redraw = true;
    let mut should_quit = false;

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
                needs_redraw = true;
            }
            _ = ui_tick.tick() => {
                if event::poll(Duration::from_millis(0))?
                    && let Event::Key(key) = event::read()? {
                        if is_ctrl_c(key) {
                            should_quit = true;
                        } else {
                            let tabs = build_account_tabs(&state.outputs);
                            if handle_key_event(key, &mut state, &tabs) {
                                needs_redraw = true;
                            }
                        }
                    }
            }
        }

        if should_quit {
            break;
        }

        if needs_redraw {
            let tabs = build_account_tabs(&state.outputs);
            sync_active_tab(&mut state, &tabs);
            terminal.draw(|frame| draw(frame, &args, &state, &tabs))?;
            needs_redraw = false;
        }
    }

    Ok(())
}

#[derive(Default)]
struct LiveState {
    outputs: Vec<ProviderPayload>,
    last_updated: Option<DateTime<Utc>>,
    last_error: Option<String>,
    refresh_count: u64,
    active_tab: usize,
    active_tab_key: Option<String>,
}

#[derive(Debug, Clone)]
struct AccountTab {
    key: String,
    label: String,
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

fn draw(frame: &mut Frame<'_>, args: &UsageArgs, state: &LiveState, tabs: &[AccountTab]) {
    let theme = TuiTheme::default();
    let area = frame.size();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    draw_header(frame, layout[0], args, state, theme);
    draw_tabs(frame, layout[1], tabs, state.active_tab, theme);
    draw_body(frame, layout[2], args, state, tabs, theme);
}

fn draw_header(
    frame: &mut Frame<'_>,
    area: Rect,
    args: &UsageArgs,
    state: &LiveState,
    theme: TuiTheme,
) {
    let title_style = theme.accent_bold();
    let dim_style = theme.dim_style();

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
            Span::styled("Tabs: ←/→ or Tab", dim_style),
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

fn draw_tabs(
    frame: &mut Frame<'_>,
    area: Rect,
    tabs: &[AccountTab],
    active_tab: usize,
    theme: TuiTheme,
) {
    let titles: Vec<Line<'static>> = tabs
        .iter()
        .map(|tab| Line::from(Span::raw(tab.label.clone())))
        .collect();
    let tab_bar = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("Accounts"))
        .select(active_tab)
        .style(theme.dim_style())
        .highlight_style(theme.accent_bold())
        .divider(Span::raw(" | "));
    frame.render_widget(tab_bar, area);
}

fn draw_body(
    frame: &mut Frame<'_>,
    area: Rect,
    args: &UsageArgs,
    state: &LiveState,
    tabs: &[AccountTab],
    theme: TuiTheme,
) {
    let mut lines = Vec::new();
    if let Some(err) = &state.last_error {
        lines.push(Line::from(Span::styled(
            format!("error: {}", err),
            theme.alert_style(),
        )));
    }

    let selected_tab = tabs
        .get(state.active_tab)
        .or_else(|| tabs.first())
        .map(|tab| tab.key.as_str());
    let mut rendered_payloads = 0usize;

    if state.outputs.is_empty() {
        if lines.is_empty() {
            lines.push(Line::from("Waiting for data..."));
        }
    } else {
        for payload in &state.outputs {
            if let Some(key) = selected_tab
                && key != "all"
                && tab_key_for_payload(payload) != key
            {
                continue;
            }
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.extend(render_payload(payload, args, theme));
            rendered_payloads += 1;
        }
    }

    if rendered_payloads == 0 && state.last_error.is_none() {
        lines.push(Line::from("No data for this account yet."));
    }

    let body = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Usage"))
        .wrap(Wrap { trim: false });
    frame.render_widget(body, area);
}

fn render_payload(
    payload: &ProviderPayload,
    args: &UsageArgs,
    theme: TuiTheme,
) -> Vec<Line<'static>> {
    let dim_style = theme.dim_style();
    let mut lines = Vec::new();

    let header = provider_header(payload, theme);
    lines.push(header);

    if let Some(error) = &payload.error {
        lines.push(Line::from(Span::styled(
            format!("error: {}", error.message),
            theme.alert_style(),
        )));
        return lines;
    }

    if let Some(usage) = &payload.usage {
        if let Some(primary) = usage.primary.as_ref() {
            lines.push(rate_window_line("primary", primary, theme));
        }
        if let Some(secondary) = usage.secondary.as_ref() {
            lines.push(rate_window_line("secondary", secondary, theme));
        }
        if let Some(tertiary) = usage.tertiary.as_ref() {
            lines.push(rate_window_line("tertiary", tertiary, theme));
        }
        if let Some(cost) = usage.provider_cost.as_ref() {
            lines.push(cost_line(cost));
        } else {
            lines.push(Line::from("cost: n/a"));
        }
        if !args.no_credits {
            if let Some(credits) = payload.credits.as_ref() {
                lines.push(Line::from(format!("credits: {:.2}", credits.remaining)));
            } else if let Some(dashboard) = payload.openai_dashboard.as_ref()
                && let Some(credits) = dashboard.credits_remaining
            {
                lines.push(Line::from(format!("credits: {:.2}", credits)));
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
    theme: TuiTheme,
) -> Line<'static> {
    let header_style = theme.accent_bold();
    let dim_style = theme.dim_style();
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

fn build_account_tabs(outputs: &[ProviderPayload]) -> Vec<AccountTab> {
    let mut tabs = Vec::new();
    tabs.push(AccountTab {
        key: "all".to_string(),
        label: "All".to_string(),
    });

    let mut seen = HashSet::new();
    for payload in outputs {
        let key = tab_key_for_payload(payload);
        if seen.insert(key.clone()) {
            tabs.push(AccountTab {
                key,
                label: tab_label_for_payload(payload),
            });
        }
    }

    tabs
}

fn sync_active_tab(state: &mut LiveState, tabs: &[AccountTab]) {
    if let Some(active_key) = state.active_tab_key.as_ref()
        && let Some(index) = tabs.iter().position(|tab| tab.key == *active_key)
    {
        state.active_tab = index;
        return;
    }

    if tabs.is_empty() {
        state.active_tab = 0;
        state.active_tab_key = None;
        return;
    }

    if state.active_tab >= tabs.len() {
        state.active_tab = tabs.len().saturating_sub(1);
    }
    state.active_tab_key = tabs.get(state.active_tab).map(|tab| tab.key.clone());
}

fn handle_key_event(key: KeyEvent, state: &mut LiveState, tabs: &[AccountTab]) -> bool {
    if key.kind != KeyEventKind::Press || tabs.is_empty() {
        return false;
    }

    let last_index = tabs.len().saturating_sub(1);
    let mut next_index = None;
    match key.code {
        KeyCode::Right | KeyCode::Tab => {
            next_index = Some((state.active_tab + 1) % tabs.len());
        }
        KeyCode::Left | KeyCode::BackTab => {
            if state.active_tab == 0 {
                next_index = Some(last_index);
            } else {
                next_index = Some(state.active_tab - 1);
            }
        }
        KeyCode::Home => {
            next_index = Some(0);
        }
        KeyCode::End => {
            next_index = Some(last_index);
        }
        _ => {}
    }

    if let Some(index) = next_index {
        state.active_tab = index;
        state.active_tab_key = tabs.get(index).map(|tab| tab.key.clone());
        return true;
    }

    false
}

fn is_ctrl_c(key: KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && key.code == KeyCode::Char('c')
        && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn tab_key_for_payload(payload: &ProviderPayload) -> String {
    let account = resolve_account(payload).unwrap_or_else(|| "default".to_string());
    format!("{}::{}", payload.provider, account)
}

fn tab_label_for_payload(payload: &ProviderPayload) -> String {
    let account = resolve_account(payload).unwrap_or_else(|| "default".to_string());
    format!("{}: {}", payload.provider, account)
}

fn rate_window_line(label: &str, window: &RateWindow, theme: TuiTheme) -> Line<'static> {
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

    let style = usage_style(window.used_percent, theme);
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

fn usage_style(percent: f64, theme: TuiTheme) -> Style {
    if percent >= 90.0 {
        theme.alert_style()
    } else if percent >= 75.0 {
        theme.accent_style()
    } else {
        Style::default()
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
