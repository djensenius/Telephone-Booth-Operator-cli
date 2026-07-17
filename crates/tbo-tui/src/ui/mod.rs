//! Top-level rendering: tab bar, body, status bar, and toast overlay.

pub mod icons;
pub mod modal;
pub mod screens;
pub mod theme;
pub mod toast;

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use time::OffsetDateTime;

use crate::app::App;
use crate::auth::AuthPhase;
use crate::ui::icons::Icons;
use crate::ui::modal::Modal;
use crate::ui::screens::Screen;
use crate::ui::theme::Theme;
use crate::ui::toast::{Level, Toasts};

/// Render the entire UI for one frame.
pub fn render(app: &App, frame: &mut Frame) {
    // Until an operator signs in, present only the login gate so no other part
    // of the interface is reachable or visible.
    if !app.is_authenticated() {
        render_login_gate(app, frame);
        return;
    }
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(frame.area());
    let (header_area, body_area, status_area) = (areas[0], areas[1], areas[2]);

    render_header(app, frame, header_area);
    screens::render(app, frame, body_area);
    render_status_bar(app, frame, status_area);
    render_toasts(app.toasts(), app.theme(), app.icons(), frame, body_area);
    if let Some(modal) = app.modal() {
        render_modal(modal, app.theme(), frame, body_area);
    }
    if app.show_help() {
        render_help(app, frame, body_area);
    }
}

/// Render compact location chrome for the active screen.
fn render_header(app: &App, frame: &mut Frame, area: Rect) {
    let theme = app.theme();
    let icons = app.icons();
    let screen = app.screen();
    let prev = app.prev_screen();
    let next = app.next_screen();
    let line = Line::from(vec![
        Span::styled(
            format!("{} {} ", prev.nav_key(), prev.short()),
            Style::new().fg(theme.dim),
        ),
        Span::styled("‹ ", Style::new().fg(theme.dim)),
        Span::styled(
            format!(
                "{} {}{}",
                screen.nav_key(),
                icons.tab(screen),
                screen.title()
            ),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::new().fg(theme.dim)),
        Span::styled("› ", Style::new().fg(theme.dim)),
        Span::styled(
            format!("{} {}", next.nav_key(), next.short()),
            Style::new().fg(theme.dim),
        ),
        Span::styled("  ? screens", Style::new().fg(theme.dim)),
    ]);
    let header = Paragraph::new(line).block(
        Block::bordered()
            .border_style(Style::new().fg(theme.dim))
            .title(format!(" {}tb-operator ", icons.brand())),
    );
    frame.render_widget(header, area);
}

/// Render the bottom status/help bar.
fn render_status_bar(app: &App, frame: &mut Frame, area: Rect) {
    let theme = app.theme();
    let ready = Span::styled(app.icons().ready(), Style::new().fg(theme.ok));
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
    // The help overlay captures all input; show how to leave it.
    if app.show_help() {
        return "  choose screen | L log in | O sign out | Esc/? close";
    }
    // A modal captures all input; show its controls regardless of screen.
    if let Some(modal) = app.modal() {
        return match modal {
            Modal::Confirm(_) => "  y confirm | n/Esc cancel",
            Modal::Prompt(_) => "  type text | Enter submit | Esc cancel",
        };
    }
    // A login can be cancelled from any screen, so surface the hint whenever
    // one is in progress regardless of the focused screen.
    if app.auth().is_in_progress() {
        return "  Esc cancel login | Tab/Right next | ? screens | q quit";
    }
    match app.screen() {
        Screen::Settings => {
            "  u API URL | b booth URL | k booth token | p poll ms | t theme | L/O auth | ? screens"
        }
        Screen::About => "  ? screens | Tab/Right next | Shift-Tab/Left prev | q quit",
        Screen::Status => "  r refresh | ? screens | Tab/Right next | Shift-Tab/Left prev | q quit",
        Screen::Messages => {
            "  ↑/↓ select | a approve | x reject | t transcribe | m moderate | g translate | d delete | p play | space pause | s stop | r reload | ? screens"
        }
        Screen::Questions => {
            "  ↑/↓ select | a activate | e deactivate | d archive | n new | p play | space pause | s stop | r reload | ? screens"
        }
        Screen::Sessions => "  ↑/↓ select | r reload | ? screens | Tab/Right next | q quit",
        Screen::Events => {
            "  ↑/↓ select | r reload | f follow | ? screens | Tab/Right next | q quit"
        }
        Screen::Stats => "  r reload | w window | ? screens | Tab/Right next | q quit",
        Screen::LiveSystem => {
            "  r refresh | ? screens | Tab/Right next | Shift-Tab/Left prev | q quit"
        }
        Screen::SystemHealth => {
            "  r refresh | ? screens | Tab/Right next | Shift-Tab/Left prev | q quit"
        }
        Screen::Debug => {
            if app
                .debug()
                .is_some_and(crate::data::DebugController::controls_allowed)
            {
                "  r refresh | f live | v level | o hook-off | h hook-on | p playback | d dial | ? screens"
            } else {
                "  r refresh | f live | v level | ? screens | Tab/Right next | q quit"
            }
        }
        Screen::Tokens => {
            "  ↑/↓ select | n new | d revoke | u usage | r reload | Esc dismiss secret | ? screens"
        }
    }
}

/// Render the toast overlay anchored to the bottom-right of `area`.
fn render_toasts(toasts: &Toasts, theme: &Theme, icons: Icons, frame: &mut Frame, area: Rect) {
    if toasts.is_empty() {
        return;
    }

    let visible: Vec<Line> = toasts
        .iter()
        .map(|toast| {
            let (color, glyph) = match toast.level {
                Level::Info => (theme.fg, icons.info()),
                Level::Warn => (theme.warn, icons.warn()),
                Level::Error => (theme.error, icons.error()),
            };
            Line::from(Span::styled(
                format!("{glyph}{}", toast.text),
                Style::new().fg(color),
            ))
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

/// Render a centered modal overlay (confirmation or text prompt).
fn render_modal(modal: &Modal, theme: &Theme, frame: &mut Frame, area: Rect) {
    let (title, lines) = match modal {
        Modal::Confirm(confirm) => (
            confirm.title.clone(),
            vec![
                Line::from(Span::styled(
                    confirm.body.clone(),
                    Style::new().fg(theme.fg),
                )),
                Line::raw(""),
                Line::from(Span::styled(
                    "y confirm   n/Esc cancel",
                    Style::new().fg(theme.dim),
                )),
            ],
        ),
        Modal::Prompt(prompt) => (
            prompt.title.clone(),
            vec![
                Line::from(Span::styled(
                    format!("{}:", prompt.label),
                    Style::new().fg(theme.dim),
                )),
                Line::from(Span::styled(
                    format!("{}\u{2588}", modal.input()),
                    Style::new().fg(theme.fg),
                )),
                Line::raw(""),
                Line::from(Span::styled(
                    "Enter submit   Esc cancel",
                    Style::new().fg(theme.dim),
                )),
            ],
        ),
    };

    let width = 60.min(area.width.saturating_sub(4)).max(20);
    let height = u16::try_from(lines.len())
        .unwrap_or(4)
        .saturating_add(2)
        .min(area.height);
    let rect = center(area, width, height);

    let block = Block::bordered()
        .border_style(Style::new().fg(theme.accent))
        .title(format!(" {title} "));
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        rect,
    );
}

/// Render the sign-in gate shown before an operator session exists. Only the
/// login instructions (and the device code while a login is in progress) are
/// presented, keeping the rest of the interface hidden until authenticated.
fn render_login_gate(app: &App, frame: &mut Frame) {
    let theme = app.theme();
    let icons = app.icons();
    let area = frame.area();
    frame.render_widget(Clear, area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("{}tb-operator", icons.brand()),
        Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Sign in to the operator API to continue.",
        Style::new().fg(theme.fg),
    )));
    lines.push(Line::raw(""));
    push_help_account(&mut lines, theme, icons, app.auth().phase());
    lines.push(Line::raw(""));
    let hint = if app.auth().is_in_progress() {
        "Esc cancel login · q quit"
    } else {
        "L log in · q quit"
    };
    lines.push(Line::from(Span::styled(hint, Style::new().fg(theme.dim))));

    let width = 60.min(area.width.saturating_sub(4)).max(30);
    let height = u16::try_from(lines.len())
        .unwrap_or(10)
        .saturating_add(2)
        .min(area.height);
    let rect = center(area, width, height);
    let block = Block::bordered()
        .border_style(Style::new().fg(theme.accent))
        .title(format!(" {}Sign in ", icons.brand()));
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        rect,
    );

    render_toasts(app.toasts(), theme, icons, frame, area);
}

/// Render the `?` screen palette plus a live account section with
/// login/logout options.
fn render_help(app: &App, frame: &mut Frame, area: Rect) {
    let theme = app.theme();
    let icons = app.icons();

    let dim = Style::new().fg(theme.dim);
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(Span::styled(
        "Go to screen",
        Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
    )));
    push_screen_group(
        &mut lines,
        theme,
        icons,
        "Operator",
        &[
            Screen::Status,
            Screen::Messages,
            Screen::Questions,
            Screen::Events,
            Screen::Sessions,
            Screen::Stats,
        ],
        app.screen(),
        app.is_admin(),
    );
    push_screen_group(
        &mut lines,
        theme,
        icons,
        "System",
        &[Screen::LiveSystem, Screen::SystemHealth, Screen::Debug],
        app.screen(),
        app.is_admin(),
    );
    push_screen_group(
        &mut lines,
        theme,
        icons,
        "Admin",
        &[Screen::Tokens, Screen::Settings, Screen::About],
        app.screen(),
        app.is_admin(),
    );
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Account",
        Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
    )));
    push_help_account(&mut lines, theme, icons, app.auth().phase());
    lines.push(palette_action_line(
        theme,
        "L",
        "Log in (Authentik device code)",
    ));
    lines.push(palette_action_line(theme, "O", "Sign out"));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Tab/Shift-Tab still cycle screens. Esc or ? closes this palette.",
        dim.add_modifier(Modifier::ITALIC),
    )));

    let width = 72.min(area.width.saturating_sub(4)).max(28);
    let height = u16::try_from(lines.len())
        .unwrap_or(10)
        .saturating_add(2)
        .min(area.height);
    let rect = center(area, width, height);

    let block = Block::bordered()
        .border_style(Style::new().fg(theme.accent))
        .title(format!(" {}Screens ", icons.help()));
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        rect,
    );
}

/// Append one grouped section of screen shortcuts to the palette.
fn push_screen_group(
    lines: &mut Vec<Line<'static>>,
    theme: &Theme,
    icons: Icons,
    title: &'static str,
    screens: &[Screen],
    current: Screen,
    is_admin: bool,
) {
    lines.push(Line::from(Span::styled(
        format!("  {title}"),
        Style::new().fg(theme.dim).add_modifier(Modifier::BOLD),
    )));
    for screen in screens {
        let locked = screen.is_admin_only() && !is_admin;
        let marker = if *screen == current { "›" } else { " " };
        let style = if locked {
            Style::new()
                .fg(theme.dim)
                .add_modifier(Modifier::DIM | Modifier::ITALIC)
        } else if *screen == current {
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.fg)
        };
        let key_style = if locked {
            Style::new().fg(theme.dim)
        } else {
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)
        };
        let label = if locked {
            format!("{}{} (admin)", icons.tab(*screen), screen.title())
        } else {
            format!("{}{}", icons.tab(*screen), screen.title())
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {marker} "), Style::new().fg(theme.accent)),
            Span::styled(format!("{:<2}", screen.nav_key()), key_style),
            Span::styled(label, style),
        ]));
    }
}

/// Format a non-screen action in the screen palette.
fn palette_action_line(theme: &Theme, key: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("    ", Style::new().fg(theme.dim)),
        Span::styled(
            format!("{key:<2}"),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc.to_owned(), Style::new().fg(theme.fg)),
    ])
}

/// Append the current authentication status to the help overlay's account
/// section, mirroring the Settings screen so the device code is visible here
/// while a login is in progress.
fn push_help_account(
    lines: &mut Vec<Line<'static>>,
    theme: &Theme,
    icons: Icons,
    phase: &AuthPhase,
) {
    match phase {
        AuthPhase::SignedOut => lines.push(Line::from(vec![
            Span::styled(
                format!("  {}", icons.signed_out()),
                Style::new().fg(theme.dim),
            ),
            Span::styled("Signed out", Style::new().fg(theme.dim)),
        ])),
        AuthPhase::Starting => lines.push(Line::from(vec![
            Span::styled(
                format!("  {}", icons.awaiting()),
                Style::new().fg(theme.warn),
            ),
            Span::styled("Starting login…", Style::new().fg(theme.warn)),
        ])),
        AuthPhase::AwaitingApproval(pending) => {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {}", icons.awaiting()),
                    Style::new().fg(theme.warn),
                ),
                Span::styled("Awaiting approval", Style::new().fg(theme.warn)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Visit: ", Style::new().fg(theme.dim)),
                Span::raw(pending.verification_uri.clone()),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Code:  ", Style::new().fg(theme.dim)),
                Span::styled(
                    pending.user_code.clone(),
                    Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Expires: ", Style::new().fg(theme.dim)),
                Span::styled(
                    format_login_expiry(pending.expires_at, OffsetDateTime::now_utc()),
                    Style::new().fg(theme.warn),
                ),
            ]));
        }
        AuthPhase::SignedIn { .. } => lines.push(Line::from(vec![
            Span::styled(
                format!("  {}", icons.signed_in()),
                Style::new().fg(theme.ok),
            ),
            Span::styled("Signed in", Style::new().fg(theme.ok)),
        ])),
        AuthPhase::Failed(message) => {
            lines.push(Line::from(vec![
                Span::styled(format!("  {}", icons.error()), Style::new().fg(theme.error)),
                Span::styled("Login failed", Style::new().fg(theme.error)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Reason: ", Style::new().fg(theme.dim)),
                Span::raw(message.clone()),
            ]));
        }
    }
}

/// Format a device-code expiry relative to `now`.
fn format_login_expiry(expires_at: OffsetDateTime, now: OffsetDateTime) -> String {
    let remaining = expires_at - now;
    let seconds = remaining.whole_seconds();
    if seconds <= 0 {
        return "expired".to_owned();
    }
    if seconds < 60 {
        return format!("{seconds}s");
    }
    format!("{}m {:02}s", seconds / 60, seconds % 60)
}

/// Center a `width`×`height` rectangle within `area`.
fn center(area: Rect, width: u16, height: u16) -> Rect {
    let horizontal = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .split(area);
    let vertical = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .split(horizontal[0]);
    vertical[0]
}

#[cfg(test)]
mod tests {
    use time::{Duration, OffsetDateTime};

    use super::format_login_expiry;

    #[test]
    fn format_login_expiry_counts_down_and_expires() {
        let now = OffsetDateTime::UNIX_EPOCH;

        assert_eq!(format_login_expiry(now + Duration::seconds(59), now), "59s");
        assert_eq!(
            format_login_expiry(now + Duration::seconds(60), now),
            "1m 00s"
        );
        assert_eq!(
            format_login_expiry(now + Duration::seconds(125), now),
            "2m 05s"
        );
        assert_eq!(format_login_expiry(now, now), "expired");
    }
}
