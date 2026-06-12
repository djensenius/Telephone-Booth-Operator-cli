//! Top-level rendering: tab bar, body, status bar, and toast overlay.

pub mod screens;
pub mod theme;
pub mod toast;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Tabs};

use crate::app::App;
use crate::ui::screens::Screen;
use crate::ui::theme::Theme;
use crate::ui::toast::{Level, Toasts};

/// Render the entire UI for one frame.
pub fn render(app: &App, frame: &mut Frame) {
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(frame.area());
    let (tabs_area, body_area, status_area) = (areas[0], areas[1], areas[2]);

    render_tabs(app, frame, tabs_area);
    screens::render(app, frame, body_area);
    render_status_bar(app, frame, status_area);
    render_toasts(app.toasts(), app.theme(), frame, body_area);
}

/// Render the top tab bar.
fn render_tabs(app: &App, frame: &mut Frame, area: Rect) {
    let theme = app.theme();
    let titles = Screen::all()
        .iter()
        .enumerate()
        .map(|(index, screen)| format!("{} {}", index + 1, screen.short()));
    let tabs = Tabs::new(titles)
        .select(app.screen().index())
        .style(Style::new().fg(theme.dim))
        .highlight_style(Style::new().fg(theme.accent).add_modifier(Modifier::BOLD))
        .block(
            Block::bordered()
                .border_style(Style::new().fg(theme.dim))
                .title(" tb-operator "),
        );
    frame.render_widget(tabs, area);
}

/// Render the bottom status/help bar.
fn render_status_bar(app: &App, frame: &mut Frame, area: Rect) {
    let theme = app.theme();
    let ready = Span::styled("● ", Style::new().fg(theme.ok));
    let pill = Span::styled(
        format!(" {} ", app.screen().title()),
        Style::new()
            .fg(theme.fg)
            .bg(theme.bg)
            .add_modifier(Modifier::BOLD),
    );
    let hints = Span::styled(status_hints(app), Style::new().fg(theme.dim));
    let line = Line::from(vec![ready, pill, hints]);
    frame.render_widget(Paragraph::new(line), area);
}

/// Context-sensitive help text for the status bar.
fn status_hints(app: &App) -> &'static str {
    // A login can be cancelled from any screen, so surface the hint whenever
    // one is in progress regardless of the focused screen.
    if app.auth().is_in_progress() {
        return "  Esc cancel login | Tab/Right next | 1-9 jump | q quit";
    }
    match app.screen() {
        Screen::Settings => "  L log in | O sign out | Tab/Right next | 1-9 jump | q quit",
        Screen::Status => "  r refresh | Tab/Right next | Shift-Tab/Left prev | 1-9 jump | q quit",
        Screen::Messages => "  ↑/↓ select | r reload | Tab/Right next | 1-9 jump | q quit",
        Screen::Questions => "  ↑/↓ select | r reload | Tab/Right next | 1-9 jump | q quit",
        _ => "  Tab/Right next | Shift-Tab/Left prev | 1-9 jump | q quit",
    }
}

/// Render the toast overlay anchored to the bottom-right of `area`.
fn render_toasts(toasts: &Toasts, theme: &Theme, frame: &mut Frame, area: Rect) {
    if toasts.is_empty() {
        return;
    }

    let visible: Vec<Line> = toasts
        .iter()
        .map(|toast| {
            let color = match toast.level {
                Level::Info => theme.fg,
                Level::Warn => theme.warn,
                Level::Error => theme.error,
            };
            Line::from(Span::styled(toast.text.clone(), Style::new().fg(color)))
        })
        .collect();

    let width = 48.min(area.width);
    let height = u16::try_from(visible.len())
        .unwrap_or(0)
        .saturating_add(2)
        .min(area.height);
    if width < 4 || height < 3 {
        return;
    }
    let rect = Rect {
        x: area.x + area.width - width,
        y: area.y + area.height - height,
        width,
        height,
    };

    let block = Block::bordered()
        .border_style(Style::new().fg(theme.dim))
        .title(" notices ");
    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(visible).block(block), rect);
}
