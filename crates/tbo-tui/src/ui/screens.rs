//! Screen routing and Phase-0 placeholder rendering.
//!
//! Each [`Screen`] becomes a fully interactive view in later phases; for now
//! every screen renders a titled placeholder describing what it will show, so
//! the navigation chrome can be exercised end to end.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use std::time::Instant;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use tbo_core::domain::{BoothState, BoothStatus, RuntimeMode};

use crate::app::App;
use crate::auth::AuthPhase;
use crate::data::Remote;
use crate::ui::theme::Theme;

/// The set of top-level screens, in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Booth status and runtime mode.
    Status,
    /// Voicemail messages.
    Messages,
    /// Question prompts.
    Questions,
    /// Event log.
    Events,
    /// Call sessions.
    Sessions,
    /// Aggregate statistics.
    Stats,
    /// Operator-reported system readouts.
    LiveSystem,
    /// btm-style charts from the booth `/metrics`.
    SystemHealth,
    /// On-device debug panel.
    Debug,
    /// API tokens.
    Tokens,
    /// Settings, identity, and about.
    Settings,
}

/// All screens in display/tab order.
const ALL: [Screen; 11] = [
    Screen::Status,
    Screen::Messages,
    Screen::Questions,
    Screen::Events,
    Screen::Sessions,
    Screen::Stats,
    Screen::LiveSystem,
    Screen::SystemHealth,
    Screen::Debug,
    Screen::Tokens,
    Screen::Settings,
];

impl Screen {
    /// All screens in tab order.
    #[must_use]
    pub fn all() -> &'static [Screen] {
        &ALL
    }

    /// The position of this screen in tab order.
    #[must_use]
    pub fn index(self) -> usize {
        ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// The screen at `index`, if any.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Screen> {
        ALL.get(index).copied()
    }

    /// The next screen, wrapping around.
    #[must_use]
    pub fn next(self) -> Screen {
        let index = (self.index() + 1) % ALL.len();
        ALL[index]
    }

    /// The previous screen, wrapping around.
    #[must_use]
    pub fn prev(self) -> Screen {
        let index = (self.index() + ALL.len() - 1) % ALL.len();
        ALL[index]
    }

    /// Full screen title.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Screen::Status => "Status",
            Screen::Messages => "Messages",
            Screen::Questions => "Questions",
            Screen::Events => "Events",
            Screen::Sessions => "Sessions",
            Screen::Stats => "Statistics",
            Screen::LiveSystem => "Live System",
            Screen::SystemHealth => "System Health",
            Screen::Debug => "Debug",
            Screen::Tokens => "API Tokens",
            Screen::Settings => "Settings",
        }
    }

    /// Short label used in the tab bar.
    #[must_use]
    pub fn short(self) -> &'static str {
        match self {
            Screen::Status => "Status",
            Screen::Messages => "Messages",
            Screen::Questions => "Questions",
            Screen::Events => "Events",
            Screen::Sessions => "Sessions",
            Screen::Stats => "Stats",
            Screen::LiveSystem => "LiveSys",
            Screen::SystemHealth => "Health",
            Screen::Debug => "Debug",
            Screen::Tokens => "Tokens",
            Screen::Settings => "Settings",
        }
    }

    /// A one-line description of the screen's eventual contents.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Screen::Status => {
                "Booth state, runtime mode, last error, and state-transition history."
            }
            Screen::Messages => {
                "Voicemail messages: transcription, moderation, translation, and playback."
            }
            Screen::Questions => {
                "Question prompts: activate/deactivate, archive, create, and playback."
            }
            Screen::Events => "Live event log with filtering and a real-time tail (SSE).",
            Screen::Sessions => {
                "Call sessions with per-call timelines, outcomes, digits, and duration."
            }
            Screen::Stats => {
                "Aggregate statistics: calls, messages, uploads, top questions, busiest hours."
            }
            Screen::LiveSystem => {
                "Operator-reported readouts: CPU, memory, disk, network, and uptime."
            }
            Screen::SystemHealth => {
                "btm-style live charts scraped from the booth's Prometheus /metrics endpoint."
            }
            Screen::Debug => {
                "On-device debug panel: state, GPIO, audio meters, logs, config, and simulate."
            }
            Screen::Tokens => "API tokens: list, create (shown once), revoke, and usage.",
            Screen::Settings => {
                "Operator URL, OIDC issuer, configured booths, theme, and identity."
            }
        }
    }
}

/// Render the body for the active screen.
pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let theme = app.theme();
    let screen = app.screen();

    let lines = match screen {
        Screen::Status => status_lines(app, theme),
        Screen::Settings => settings_lines(app, theme),
        _ => placeholder_lines(screen, theme),
    };

    let block = Block::bordered()
        .border_style(Style::new().fg(theme.dim))
        .title(format!(" {} ", screen.title()));
    let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

/// The title header line for a screen body.
fn header(theme: &Theme, title: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        title,
        Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
    ))
}

/// A placeholder body for screens that are not yet implemented.
fn placeholder_lines(screen: Screen, theme: &Theme) -> Vec<Line<'static>> {
    vec![
        header(theme, screen.title()),
        Line::raw(""),
        Line::raw(screen.description()),
        Line::raw(""),
        Line::from(Span::styled(
            "Coming soon.",
            Style::new().fg(theme.dim).add_modifier(Modifier::ITALIC),
        )),
    ]
}

/// The Settings body: configuration summary and account/auth section.
fn settings_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let config = app.config();
    let mut lines = vec![
        header(theme, Screen::Settings.title()),
        Line::raw(""),
        Line::raw(Screen::Settings.description()),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Operator API: ", Style::new().fg(theme.dim)),
            Span::raw(config.operator.base_url.clone()),
        ]),
        Line::from(vec![
            Span::styled("OIDC issuer:  ", Style::new().fg(theme.dim)),
            Span::raw(config.auth.issuer.clone()),
        ]),
        Line::from(vec![
            Span::styled("Booths:       ", Style::new().fg(theme.dim)),
            Span::raw(config.booths.len().to_string()),
        ]),
    ];
    push_account_lines(&mut lines, theme, app.auth().phase());
    lines
}

/// The Status body: live booth state from the operator API.
fn status_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![header(theme, Screen::Status.title()), Line::raw("")];
    match app.status().state() {
        Remote::Idle | Remote::Loading => lines.push(hint_line(theme, "Loading booth status…")),
        Remote::Ready { value, fetched_at } => {
            push_status_detail(&mut lines, theme, value);
            lines.push(Line::raw(""));
            lines.push(note_line(theme, format!("Fetched {}.", ago(*fetched_at))));
        }
        Remote::Failed { error, at } => {
            lines.push(Line::from(Span::styled(
                format!("Failed to load status {}.", ago(*at)),
                Style::new().fg(theme.error),
            )));
            lines.push(Line::from(vec![
                Span::styled("Reason: ", Style::new().fg(theme.dim)),
                Span::raw(error.clone()),
            ]));
            lines.push(hint_line(
                theme,
                "Press r to retry; sign in via Settings if unauthorized.",
            ));
        }
    }
    if app.status().is_refreshing() {
        lines.push(note_line(theme, "Refreshing…".to_owned()));
    }
    lines
}

/// Append the detail rows for a loaded [`BoothStatus`].
fn push_status_detail(lines: &mut Vec<Line<'static>>, theme: &Theme, status: &BoothStatus) {
    lines.push(Line::from(vec![
        Span::styled("State:        ", Style::new().fg(theme.dim)),
        Span::styled(
            state_label(status.state),
            Style::new()
                .fg(state_color(theme, status.state))
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Runtime mode: ", Style::new().fg(theme.dim)),
        Span::raw(
            status
                .runtime_mode
                .map_or_else(|| "—".to_owned(), |mode| mode_label(mode).to_owned()),
        ),
    ]));
    if let Some(question_id) = &status.current_question_id {
        lines.push(kv_line(theme, "Question:     ", question_id.clone()));
    }
    if let Some(message_id) = &status.current_message_id {
        lines.push(kv_line(theme, "Message:      ", message_id.clone()));
    }
    if let Some(last_error) = &status.last_error {
        lines.push(Line::from(vec![
            Span::styled("Last error:   ", Style::new().fg(theme.dim)),
            Span::styled(last_error.clone(), Style::new().fg(theme.error)),
        ]));
    }
    lines.push(kv_line(
        theme,
        "Updated:      ",
        format_ts(status.updated_at),
    ));
}

/// A dim-`label` / plain-`value` line for owned values.
fn kv_line(theme: &Theme, label: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(label, Style::new().fg(theme.dim)),
        Span::raw(value),
    ])
}

/// Human-readable label for a booth state.
fn state_label(state: BoothState) -> &'static str {
    match state {
        BoothState::Idle => "Idle",
        BoothState::DialTone => "Dial tone",
        BoothState::Dialing => "Dialing",
        BoothState::PlayingQuestion => "Playing question",
        BoothState::Beep => "Beep",
        BoothState::Recording => "Recording",
        BoothState::Uploading => "Uploading",
        BoothState::PlayingMessage => "Playing message",
        BoothState::PlayingInstructions => "Playing instructions",
        BoothState::Error => "Error",
    }
}

/// Accent color for a booth state (red for errors, dim when idle).
fn state_color(theme: &Theme, state: BoothState) -> Color {
    match state {
        BoothState::Error => theme.error,
        BoothState::Idle => theme.dim,
        _ => theme.ok,
    }
}

/// Human-readable label for a runtime mode.
fn mode_label(mode: RuntimeMode) -> &'static str {
    match mode {
        RuntimeMode::Real => "Real hardware",
        RuntimeMode::Mock => "Mock",
        RuntimeMode::Simulator => "Simulator",
    }
}

/// Format a timestamp for display, falling back to `unknown`.
fn format_ts(at: OffsetDateTime) -> String {
    at.format(&Rfc3339).unwrap_or_else(|_| "unknown".to_owned())
}

/// A relative-age phrase for a monotonic instant (e.g. `3s ago`).
fn ago(at: Instant) -> String {
    let secs = at.elapsed().as_secs();
    match secs {
        0 => "just now".to_owned(),
        s if s < 60 => format!("{s}s ago"),
        s => format!("{}m {}s ago", s / 60, s % 60),
    }
}

/// A dim, italic note line for owned text.
fn note_line(theme: &Theme, text: String) -> Line<'static> {
    Line::from(Span::styled(
        text,
        Style::new().fg(theme.dim).add_modifier(Modifier::ITALIC),
    ))
}

/// Append the account/authentication section to the Settings body.
fn push_account_lines(lines: &mut Vec<Line<'static>>, theme: &Theme, phase: &AuthPhase) {
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Account",
        Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
    )));
    match phase {
        AuthPhase::SignedOut => {
            lines.push(status_line(theme, "Status: ", "signed out", theme.dim));
            lines.push(hint_line(theme, "Press L to log in."));
        }
        AuthPhase::Starting => {
            lines.push(status_line(
                theme,
                "Status: ",
                "starting login…",
                theme.warn,
            ));
        }
        AuthPhase::AwaitingApproval(pending) => {
            lines.push(status_line(
                theme,
                "Status: ",
                "awaiting approval",
                theme.warn,
            ));
            lines.push(Line::from(vec![
                Span::styled("Visit:  ", Style::new().fg(theme.dim)),
                Span::raw(pending.verification_uri.clone()),
            ]));
            lines.push(Line::from(vec![
                Span::styled("Code:   ", Style::new().fg(theme.dim)),
                Span::styled(
                    pending.user_code.clone(),
                    Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
                ),
            ]));
            if let Some(complete) = &pending.verification_uri_complete {
                lines.push(Line::from(vec![
                    Span::styled("Direct: ", Style::new().fg(theme.dim)),
                    Span::raw(complete.clone()),
                ]));
            }
            lines.push(hint_line(theme, "Press Esc to cancel."));
        }
        AuthPhase::SignedIn { expires_at } => {
            lines.push(status_line(theme, "Status: ", "signed in", theme.ok));
            lines.push(Line::from(vec![
                Span::styled("Expires:", Style::new().fg(theme.dim)),
                Span::raw(format!(" {}", format_expiry(*expires_at))),
            ]));
            lines.push(hint_line(theme, "Press O to sign out."));
        }
        AuthPhase::Failed(message) => {
            lines.push(status_line(theme, "Status: ", "login failed", theme.error));
            lines.push(Line::from(vec![
                Span::styled("Reason: ", Style::new().fg(theme.dim)),
                Span::raw(message.clone()),
            ]));
            lines.push(hint_line(theme, "Press L to try again."));
        }
    }
}

/// A `label`/`value` line where the value carries `color`.
fn status_line(
    theme: &Theme,
    label: &'static str,
    value: &'static str,
    color: ratatui::style::Color,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(label, Style::new().fg(theme.dim)),
        Span::styled(value, Style::new().fg(color)),
    ])
}

/// A dim, italic hint line.
fn hint_line(theme: &Theme, text: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        text,
        Style::new().fg(theme.dim).add_modifier(Modifier::ITALIC),
    ))
}

/// Format an access-token expiry for display, falling back to `unknown`.
fn format_expiry(expires_at: Option<OffsetDateTime>) -> String {
    expires_at.map_or_else(
        || "unknown".to_owned(),
        |at| at.format(&Rfc3339).unwrap_or_else(|_| "unknown".to_owned()),
    )
}

#[cfg(test)]
mod tests {
    use super::Screen;

    #[test]
    fn index_round_trips_for_every_screen() {
        for (index, screen) in Screen::all().iter().enumerate() {
            assert_eq!(screen.index(), index);
            assert_eq!(Screen::from_index(index), Some(*screen));
        }
        assert_eq!(Screen::from_index(Screen::all().len()), None);
    }

    #[test]
    fn next_and_prev_wrap_around() {
        let first = Screen::all()[0];
        let last = Screen::all()[Screen::all().len() - 1];
        assert_eq!(first.prev(), last);
        assert_eq!(last.next(), first);
        assert_eq!(first.next().prev(), first);
    }

    #[test]
    fn labels_are_non_empty() {
        for screen in Screen::all() {
            assert!(!screen.title().is_empty());
            assert!(!screen.short().is_empty());
            assert!(!screen.description().is_empty());
        }
    }
}
