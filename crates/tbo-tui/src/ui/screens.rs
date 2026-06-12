//! Screen routing and Phase-0 placeholder rendering.
//!
//! Each [`Screen`] becomes a fully interactive view in later phases; for now
//! every screen renders a titled placeholder describing what it will show, so
//! the navigation chrome can be exercised end to end.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Block, Chart, Dataset, Gauge, GraphType, List, ListItem, ListState, Paragraph, Wrap,
};
use serde_json::Value;
use std::time::Duration;
use std::time::Instant;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use tbo_audio::PlaybackStatus;
use tbo_booth_client::{GpioPinSnapshot, LogEntry};
use tbo_core::domain::{
    ApiToken, ApiTokenCreated, ApiTokenUsageBucket, BoothEventRecord, BoothEventType, BoothState,
    BoothStatus, BoothSystemSnapshot, BoothSystemSnapshotEnvelope, CallOutcome, CallSession,
    CallSessionDetail, Message, MessageStatus, Moderation, ModerationRecommendation, Question,
    QuestionStatus, RuntimeMode, StatsOverview, StatsWindow, Transcription, TranscriptionStatus,
};
use tbo_metrics::{BoothMetrics, MetricsHistory};

use crate::app::App;
use crate::auth::AuthPhase;
use crate::data::{DebugController, Remote, SystemHealthController};
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

/// Maximum number of pretty-printed JSON payload lines shown in event detail.
const PAYLOAD_MAX_LINES: usize = 40;

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
    match app.screen() {
        Screen::Messages => render_messages(app, frame, area),
        Screen::Questions => render_questions(app, frame, area),
        Screen::Sessions => render_sessions(app, frame, area),
        Screen::Events => render_events(app, frame, area),
        Screen::Tokens => render_tokens(app, frame, area),
        Screen::Stats => {
            render_paragraph(frame, area, theme, "Statistics", stats_lines(app, theme));
        }
        Screen::LiveSystem => {
            render_paragraph(
                frame,
                area,
                theme,
                "Live System",
                live_system_lines(app, theme),
            );
        }
        Screen::Status => render_paragraph(frame, area, theme, "Status", status_lines(app, theme)),
        Screen::SystemHealth => render_system_health(app, frame, area),
        Screen::Settings => {
            render_paragraph(frame, area, theme, "Settings", settings_lines(app, theme));
        }
        Screen::Debug => render_debug(app, frame, area),
    }
}

/// Render `lines` as a bordered, word-wrapped paragraph filling `area`.
fn render_paragraph(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    title: &str,
    lines: Vec<Line<'static>>,
) {
    let block = Block::bordered()
        .border_style(Style::new().fg(theme.dim))
        .title(format!(" {title} "));
    let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

/// Render the Messages screen: a master list beside a detail pane.
fn render_messages(app: &App, frame: &mut Frame, area: Rect) {
    let theme = app.theme();
    let controller = app.messages();
    match controller.state() {
        Remote::Ready { value, fetched_at } if !value.is_empty() => {
            let columns =
                Layout::horizontal([Constraint::Percentage(42), Constraint::Min(24)]).split(area);
            render_message_list(frame, columns[0], theme, value, controller.selected_index());

            let mut detail = message_detail_lines(theme, controller.selected_message());
            detail.push(Line::raw(""));
            detail.extend(playback_lines(app, theme));
            detail.push(Line::raw(""));
            if controller.is_refreshing() {
                detail.push(note_line(theme, "Refreshing…".to_owned()));
            } else {
                detail.push(note_line(theme, format!("Fetched {}.", ago(*fetched_at))));
            }
            render_paragraph(frame, columns[1], theme, "Detail", detail);
        }
        other => render_paragraph(
            frame,
            area,
            theme,
            "Messages",
            messages_status_lines(theme, other),
        ),
    }
}

/// Body lines for the non-list Messages states (loading, empty, or failed).
fn messages_status_lines(theme: &Theme, state: &Remote<Vec<Message>>) -> Vec<Line<'static>> {
    let mut lines = vec![header(theme, Screen::Messages.title()), Line::raw("")];
    match state {
        Remote::Idle | Remote::Loading => lines.push(hint_line(theme, "Loading messages…")),
        Remote::Ready { .. } => {
            lines.push(note_line(theme, "No messages.".to_owned()));
            lines.push(hint_line(theme, "Press r to reload."));
        }
        Remote::Failed { error, at } => {
            lines.push(Line::from(Span::styled(
                format!("Failed to load messages {}.", ago(*at)),
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
    lines
}

/// Render the scrollable list of messages with the selected row highlighted.
fn render_message_list(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    messages: &[Message],
    selected: usize,
) {
    let items: Vec<ListItem> = messages
        .iter()
        .map(|message| {
            ListItem::new(Line::from(vec![
                status_badge(theme, message.status),
                Span::raw(" "),
                Span::styled(short_time(message.created_at), Style::new().fg(theme.dim)),
                Span::raw("  "),
                Span::raw(transcription_snippet(message)),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::bordered()
                .border_style(Style::new().fg(theme.dim))
                .title(" Messages "),
        )
        .highlight_style(
            Style::new()
                .fg(theme.accent)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD),
        )
        .highlight_symbol("> ");
    let mut list_state = ListState::default();
    list_state.select(Some(selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Build the detail-pane lines for the selected message.
fn message_detail_lines(theme: &Theme, message: Option<&Message>) -> Vec<Line<'static>> {
    let Some(message) = message else {
        return vec![
            header(theme, "Message"),
            Line::raw(""),
            hint_line(theme, "Select a message."),
        ];
    };

    let mut lines = vec![
        header(theme, "Message"),
        Line::raw(""),
        kv_line(theme, "ID:        ", message.id.clone()),
        Line::from(vec![
            Span::styled("Status:    ", Style::new().fg(theme.dim)),
            status_badge(theme, message.status),
        ]),
    ];
    if let Some(question_id) = &message.question_id {
        lines.push(kv_line(theme, "Question:  ", question_id.clone()));
    }
    lines.push(kv_line(theme, "Created:   ", format_ts(message.created_at)));
    if let Some(received_at) = message.received_at {
        lines.push(kv_line(theme, "Received:  ", format_ts(received_at)));
    }
    if let Some(decided_at) = message.decided_at {
        lines.push(kv_line(theme, "Decided:   ", format_ts(decided_at)));
    }
    if let Some(notes) = &message.notes {
        lines.push(kv_line(theme, "Notes:     ", notes.clone()));
    }

    lines.push(Line::raw(""));
    lines.push(subheader(theme, "Transcription"));
    push_transcription_lines(&mut lines, theme, message.latest_transcription.as_ref());

    lines.push(Line::raw(""));
    lines.push(subheader(theme, "Moderation"));
    push_moderation_lines(&mut lines, theme, message.latest_moderation.as_ref());

    lines
}

/// Append the transcription detail rows (or a `none` note).
fn push_transcription_lines(
    lines: &mut Vec<Line<'static>>,
    theme: &Theme,
    transcription: Option<&Transcription>,
) {
    let Some(transcription) = transcription else {
        lines.push(note_line(theme, "  none".to_owned()));
        return;
    };
    lines.push(Line::from(vec![
        Span::styled("  Status:    ", Style::new().fg(theme.dim)),
        job_status_badge(theme, transcription.status),
    ]));
    if let Some(language) = &transcription.language {
        lines.push(kv_line(theme, "  Language:  ", language.clone()));
    }
    match &transcription.text {
        Some(text) => lines.push(kv_line(theme, "  Text:      ", text.clone())),
        None => lines.push(note_line(theme, "  (no text)".to_owned())),
    }
    if let Some(translated) = &transcription.translated_text {
        lines.push(kv_line(theme, "  Translated:", translated.clone()));
        if let Some(language) = &transcription.translated_language {
            lines.push(kv_line(theme, "  → Language:", language.clone()));
        }
    }
    if let Some(error) = &transcription.error {
        lines.push(Line::from(vec![
            Span::styled("  Error:     ", Style::new().fg(theme.dim)),
            Span::styled(error.clone(), Style::new().fg(theme.error)),
        ]));
    }
}

/// Append the moderation detail rows (or a `none` note).
fn push_moderation_lines(
    lines: &mut Vec<Line<'static>>,
    theme: &Theme,
    moderation: Option<&Moderation>,
) {
    let Some(moderation) = moderation else {
        lines.push(note_line(theme, "  none".to_owned()));
        return;
    };
    lines.push(Line::from(vec![
        Span::styled("  Status:    ", Style::new().fg(theme.dim)),
        job_status_badge(theme, moderation.status),
    ]));
    if let Some(recommendation) = moderation.recommendation {
        lines.push(Line::from(vec![
            Span::styled("  Recommend: ", Style::new().fg(theme.dim)),
            recommendation_badge(theme, recommendation),
        ]));
    }
    if let Some(flagged) = moderation.flagged {
        lines.push(Line::from(vec![
            Span::styled("  Flagged:   ", Style::new().fg(theme.dim)),
            Span::styled(
                if flagged { "yes" } else { "no" },
                Style::new().fg(if flagged { theme.error } else { theme.ok }),
            ),
        ]));
    }
    if let Some(score) = moderation.max_score {
        lines.push(kv_line(theme, "  Max score: ", format!("{score:.2}")));
    }
    if let Some(reason) = &moderation.reason_summary {
        lines.push(kv_line(theme, "  Reason:    ", reason.clone()));
    }
    if let Some(error) = &moderation.error {
        lines.push(Line::from(vec![
            Span::styled("  Error:     ", Style::new().fg(theme.dim)),
            Span::styled(error.clone(), Style::new().fg(theme.error)),
        ]));
    }
}

/// A bracketed, bold badge span in the given color.
fn badge(text: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!("[{text}]"),
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    )
}

/// Colored badge for a message moderation status.
fn status_badge(theme: &Theme, status: MessageStatus) -> Span<'static> {
    let (label, color) = match status {
        MessageStatus::Uploading => ("uploading", theme.dim),
        MessageStatus::Received => ("received", theme.accent),
        MessageStatus::Pending => ("pending", theme.warn),
        MessageStatus::Approved => ("approved", theme.ok),
        MessageStatus::Rejected => ("rejected", theme.error),
    };
    badge(label, color)
}

/// Colored badge for a transcription/moderation job status.
fn job_status_badge(theme: &Theme, status: TranscriptionStatus) -> Span<'static> {
    let (label, color) = match status {
        TranscriptionStatus::Pending => ("pending", theme.warn),
        TranscriptionStatus::Succeeded => ("succeeded", theme.ok),
        TranscriptionStatus::Failed => ("failed", theme.error),
    };
    badge(label, color)
}

/// Colored badge for a moderation recommendation.
fn recommendation_badge(theme: &Theme, recommendation: ModerationRecommendation) -> Span<'static> {
    let (label, color) = match recommendation {
        ModerationRecommendation::Approve => ("approve", theme.ok),
        ModerationRecommendation::Review => ("review", theme.warn),
        ModerationRecommendation::Reject => ("reject", theme.error),
    };
    badge(label, color)
}

/// An accent subsection heading within a detail body.
fn subheader(theme: &Theme, title: &'static str) -> Line<'static> {
    Line::from(Span::styled(title, Style::new().fg(theme.accent)))
}

/// A compact `MM-DD HH:MM` timestamp for list rows.
fn short_time(at: OffsetDateTime) -> String {
    let formatted = format_ts(at);
    if formatted.is_char_boundary(16) && formatted.len() >= 16 {
        formatted[5..16].replace('T', " ")
    } else {
        formatted
    }
}

/// A single-line transcription snippet for a list row.
fn transcription_snippet(message: &Message) -> String {
    message
        .latest_transcription
        .as_ref()
        .and_then(|transcription| transcription.text.as_deref())
        .map_or_else(
            || "(no transcription)".to_owned(),
            |text| truncate(&text.split_whitespace().collect::<Vec<_>>().join(" "), 40),
        )
}

/// Truncate `text` to at most `max` characters, appending an ellipsis when cut.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        let kept: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{kept}…")
    } else {
        text.to_owned()
    }
}

/// Render the Questions screen: a master list beside a detail pane.
fn render_questions(app: &App, frame: &mut Frame, area: Rect) {
    let theme = app.theme();
    let controller = app.questions();
    match controller.state() {
        Remote::Ready { value, fetched_at } if !value.is_empty() => {
            let columns =
                Layout::horizontal([Constraint::Percentage(42), Constraint::Min(24)]).split(area);
            render_question_list(frame, columns[0], theme, value, controller.selected_index());

            let mut detail = question_detail_lines(theme, controller.selected_question());
            detail.push(Line::raw(""));
            detail.extend(playback_lines(app, theme));
            detail.push(Line::raw(""));
            if controller.is_refreshing() {
                detail.push(note_line(theme, "Refreshing…".to_owned()));
            } else {
                detail.push(note_line(theme, format!("Fetched {}.", ago(*fetched_at))));
            }
            render_paragraph(frame, columns[1], theme, "Detail", detail);
        }
        other => render_paragraph(
            frame,
            area,
            theme,
            "Questions",
            questions_status_lines(theme, other),
        ),
    }
}

/// Body lines for the non-list Questions states (loading, empty, or failed).
fn questions_status_lines(theme: &Theme, state: &Remote<Vec<Question>>) -> Vec<Line<'static>> {
    let mut lines = vec![header(theme, Screen::Questions.title()), Line::raw("")];
    match state {
        Remote::Idle | Remote::Loading => lines.push(hint_line(theme, "Loading questions…")),
        Remote::Ready { .. } => {
            lines.push(note_line(theme, "No questions.".to_owned()));
            lines.push(hint_line(theme, "Press r to reload."));
        }
        Remote::Failed { error, at } => {
            lines.push(Line::from(Span::styled(
                format!("Failed to load questions {}.", ago(*at)),
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
    lines
}

/// Render the scrollable list of questions with the selected row highlighted.
fn render_question_list(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    questions: &[Question],
    selected: usize,
) {
    let items: Vec<ListItem> = questions
        .iter()
        .map(|question| {
            ListItem::new(Line::from(vec![
                question_status_badge(theme, question.status),
                Span::raw(" "),
                Span::raw(truncate(&question.prompt.replace('\n', " "), 40)),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::bordered()
                .border_style(Style::new().fg(theme.dim))
                .title(" Questions "),
        )
        .highlight_style(
            Style::new()
                .fg(theme.accent)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD),
        )
        .highlight_symbol("> ");
    let mut list_state = ListState::default();
    list_state.select(Some(selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Build the detail-pane lines for the selected question.
fn question_detail_lines(theme: &Theme, question: Option<&Question>) -> Vec<Line<'static>> {
    let Some(question) = question else {
        return vec![
            header(theme, "Question"),
            Line::raw(""),
            hint_line(theme, "Select a question."),
        ];
    };

    let mut lines = vec![
        header(theme, "Question"),
        Line::raw(""),
        kv_line(theme, "ID:       ", question.id.clone()),
        Line::from(vec![
            Span::styled("Status:   ", Style::new().fg(theme.dim)),
            question_status_badge(theme, question.status),
        ]),
        kv_line(theme, "Created:  ", format_ts(question.created_at)),
    ];
    if let Some(duration_ms) = question.audio.duration_ms {
        lines.push(kv_line(theme, "Duration: ", format_duration(duration_ms)));
    }

    lines.push(Line::raw(""));
    lines.push(subheader(theme, "Prompt"));
    lines.push(Line::raw(question.prompt.clone()));

    lines
}

/// Colored badge for a question publication status.
fn question_status_badge(theme: &Theme, status: QuestionStatus) -> Span<'static> {
    let (label, color) = match status {
        QuestionStatus::Draft => ("draft", theme.dim),
        QuestionStatus::Active => ("active", theme.ok),
        QuestionStatus::Archived => ("archived", theme.warn),
    };
    badge(label, color)
}

/// Format an audio duration in milliseconds as a human-readable string.
fn format_duration(duration_ms: i64) -> String {
    if duration_ms < 0 {
        return "—".to_owned();
    }
    let total_secs = duration_ms / 1000;
    let millis = duration_ms % 1000;
    let minutes = total_secs / 60;
    let seconds = total_secs % 60;
    if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}.{millis:03}s")
    }
}

/// Build the playback-status lines shown under the Messages and Questions
/// detail panes: availability, loading, the current transport status, and the
/// elapsed/total position when a track is loaded.
fn playback_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let playback = app.playback();
    let mut lines = vec![subheader(theme, "Playback")];

    if !playback.is_available() {
        let reason = playback
            .unavailable_reason()
            .unwrap_or("audio output is unavailable");
        lines.push(note_line(theme, format!("Unavailable: {reason}")));
        return lines;
    }

    if playback.is_loading() {
        lines.push(note_line(theme, "Loading audio…".to_owned()));
        return lines;
    }

    let Some(state) = playback.snapshot() else {
        lines.push(hint_line(theme, "Press p to play the selected audio."));
        return lines;
    };

    let (label, color) = match state.status() {
        PlaybackStatus::Idle => ("idle", theme.dim),
        PlaybackStatus::Playing => ("playing", theme.ok),
        PlaybackStatus::Paused => ("paused", theme.warn),
        PlaybackStatus::Ended => ("ended", theme.dim),
    };
    let position = duration_to_ms(state.position());
    let progress = state.total().map_or_else(
        || format_duration(position),
        |total| {
            format!(
                "{} / {}",
                format_duration(position),
                format_duration(duration_to_ms(total))
            )
        },
    );
    lines.push(Line::from(vec![
        Span::styled("Status:   ", Style::new().fg(theme.dim)),
        badge(label, color),
        Span::raw("  "),
        Span::styled(progress, Style::new().fg(theme.fg)),
    ]));
    if let Some(error) = state.error() {
        lines.push(Line::from(Span::styled(
            format!("Error: {error}"),
            Style::new().fg(theme.error),
        )));
    }
    if matches!(state.status(), PlaybackStatus::Idle | PlaybackStatus::Ended) {
        lines.push(hint_line(theme, "Press p to play the selected audio."));
    }
    lines
}

/// Convert a [`Duration`] to whole milliseconds as an `i64`, saturating rather
/// than wrapping for absurdly long inputs.
fn duration_to_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// Render the Tokens screen: a master list beside a detail pane that reveals a
/// freshly created secret and, on demand, per-token usage.
fn render_tokens(app: &App, frame: &mut Frame, area: Rect) {
    let theme = app.theme();
    let controller = app.tokens();
    match controller.state() {
        Remote::Ready { value, fetched_at } if !value.is_empty() => {
            let columns =
                Layout::horizontal([Constraint::Percentage(42), Constraint::Min(28)]).split(area);
            render_token_list(frame, columns[0], theme, value, controller.selected_index());

            let mut detail = Vec::new();
            if let Some(created) = controller.revealed() {
                detail.extend(token_reveal_lines(theme, created));
                detail.push(Line::raw(""));
            }
            detail.extend(token_detail_lines(theme, controller.selected_token()));
            if let Some(usage) = controller.usage() {
                detail.push(Line::raw(""));
                detail.extend(token_usage_lines(theme, usage));
            }
            detail.push(Line::raw(""));
            if controller.is_refreshing() {
                detail.push(note_line(theme, "Refreshing…".to_owned()));
            } else {
                detail.push(note_line(theme, format!("Fetched {}.", ago(*fetched_at))));
            }
            render_paragraph(frame, columns[1], theme, "Detail", detail);
        }
        other => {
            let mut lines = tokens_status_lines(theme, other);
            if let Some(created) = controller.revealed() {
                lines.push(Line::raw(""));
                lines.extend(token_reveal_lines(theme, created));
            }
            render_paragraph(frame, area, theme, "API Tokens", lines);
        }
    }
}

/// Body lines for the non-list Tokens states (loading, empty, or failed).
fn tokens_status_lines(theme: &Theme, state: &Remote<Vec<ApiToken>>) -> Vec<Line<'static>> {
    let mut lines = vec![header(theme, Screen::Tokens.title()), Line::raw("")];
    match state {
        Remote::Idle | Remote::Loading => lines.push(hint_line(theme, "Loading tokens…")),
        Remote::Ready { .. } => {
            lines.push(note_line(theme, "No API tokens.".to_owned()));
            lines.push(hint_line(theme, "Press n to create one; r to reload."));
        }
        Remote::Failed { error, at } => {
            lines.push(Line::from(Span::styled(
                format!("Failed to load tokens {}.", ago(*at)),
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
    lines
}

/// Render the scrollable list of tokens with the selected row highlighted.
fn render_token_list(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    tokens: &[ApiToken],
    selected: usize,
) {
    let now = OffsetDateTime::now_utc();
    let items: Vec<ListItem> = tokens
        .iter()
        .map(|token| {
            ListItem::new(Line::from(vec![
                token_status_badge(theme, token, now),
                Span::raw(" "),
                Span::raw(truncate(&token.name, 26)),
                Span::styled(format!("  ··{}", token.last4), Style::new().fg(theme.dim)),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::bordered()
                .border_style(Style::new().fg(theme.dim))
                .title(" Tokens "),
        )
        .highlight_style(
            Style::new()
                .fg(theme.accent)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD),
        )
        .highlight_symbol("> ");
    let mut list_state = ListState::default();
    list_state.select(Some(selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Build the detail-pane lines for the selected token.
fn token_detail_lines(theme: &Theme, token: Option<&ApiToken>) -> Vec<Line<'static>> {
    let Some(token) = token else {
        return vec![
            header(theme, "Token"),
            Line::raw(""),
            hint_line(theme, "Select a token."),
        ];
    };
    let now = OffsetDateTime::now_utc();
    vec![
        header(theme, "Token"),
        Line::raw(""),
        kv_line(theme, "ID:        ", token.id.clone()),
        kv_line(theme, "Name:      ", token.name.clone()),
        kv_line(theme, "Secret:    ", format!("··{}", token.last4)),
        Line::from(vec![
            Span::styled("Status:    ", Style::new().fg(theme.dim)),
            token_status_badge(theme, token, now),
        ]),
        kv_line(theme, "Created:   ", format_ts(token.created_at)),
        kv_line(
            theme,
            "Expires:   ",
            token
                .expires_at
                .map_or_else(|| "never".to_owned(), format_ts),
        ),
        kv_line(
            theme,
            "Last used: ",
            token
                .last_used_at
                .map_or_else(|| "never".to_owned(), format_ts),
        ),
        kv_line(
            theme,
            "Revoked:   ",
            token.revoked_at.map_or_else(|| "no".to_owned(), format_ts),
        ),
    ]
}

/// A colored badge describing a token's effective status.
fn token_status_badge(theme: &Theme, token: &ApiToken, now: OffsetDateTime) -> Span<'static> {
    let (label, color) = if token.revoked_at.is_some() {
        ("revoked", theme.error)
    } else if token.expires_at.is_some_and(|expires| expires <= now) {
        ("expired", theme.warn)
    } else {
        ("active", theme.ok)
    };
    badge(label, color)
}

/// Build the one-time secret reveal block for a freshly created token.
fn token_reveal_lines(theme: &Theme, created: &ApiTokenCreated) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!("New token \"{}\" created", created.name),
            Style::new().fg(theme.ok).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            created.plaintext.clone(),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Copy this secret now — it is shown only once. Press Esc to dismiss.",
            Style::new().fg(theme.warn),
        )),
    ]
}

/// Build the usage block for the selected token.
fn token_usage_lines(
    theme: &Theme,
    usage: &Remote<Vec<ApiTokenUsageBucket>>,
) -> Vec<Line<'static>> {
    let mut lines = vec![subheader(theme, "Usage (30d)")];
    match usage {
        Remote::Idle | Remote::Loading => lines.push(hint_line(theme, "Loading usage…")),
        Remote::Ready { value, .. } if value.is_empty() => {
            lines.push(note_line(theme, "No usage in the last 30 days.".to_owned()));
        }
        Remote::Ready { value, .. } => {
            for bucket in value {
                lines.push(Line::from(vec![
                    Span::styled(format!("{}  ", bucket.date), Style::new().fg(theme.dim)),
                    Span::raw(bucket.count.to_string()),
                ]));
            }
        }
        Remote::Failed { error, .. } => {
            lines.push(Line::from(vec![
                Span::styled("Usage failed: ", Style::new().fg(theme.error)),
                Span::raw(error.clone()),
            ]));
        }
    }
    lines
}

/// Render the Sessions screen: a master list beside a detail/timeline pane.
fn render_sessions(app: &App, frame: &mut Frame, area: Rect) {
    let theme = app.theme();
    let controller = app.sessions();
    match controller.state() {
        Remote::Ready { value, fetched_at } if !value.is_empty() => {
            let columns =
                Layout::horizontal([Constraint::Percentage(42), Constraint::Min(28)]).split(area);
            render_session_list(frame, columns[0], theme, value, controller.selected_index());

            let mut detail =
                session_detail_lines(theme, controller.selected_session(), controller.detail());
            detail.push(Line::raw(""));
            if controller.is_refreshing() {
                detail.push(note_line(theme, "Refreshing…".to_owned()));
            } else {
                detail.push(note_line(theme, format!("Fetched {}.", ago(*fetched_at))));
            }
            render_paragraph(frame, columns[1], theme, "Detail", detail);
        }
        other => render_paragraph(
            frame,
            area,
            theme,
            "Sessions",
            sessions_status_lines(theme, other),
        ),
    }
}

/// Body lines for the non-list Sessions states (loading, empty, or failed).
fn sessions_status_lines(theme: &Theme, state: &Remote<Vec<CallSession>>) -> Vec<Line<'static>> {
    let mut lines = vec![header(theme, Screen::Sessions.title()), Line::raw("")];
    match state {
        Remote::Idle | Remote::Loading => lines.push(hint_line(theme, "Loading sessions…")),
        Remote::Ready { .. } => {
            lines.push(note_line(theme, "No sessions.".to_owned()));
            lines.push(hint_line(theme, "Press r to reload."));
        }
        Remote::Failed { error, at } => {
            lines.push(Line::from(Span::styled(
                format!("Failed to load sessions {}.", ago(*at)),
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
    lines
}

/// Render the scrollable list of sessions with the selected row highlighted.
fn render_session_list(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    sessions: &[CallSession],
    selected: usize,
) {
    let items: Vec<ListItem> = sessions
        .iter()
        .map(|session| {
            ListItem::new(Line::from(vec![
                outcome_badge(theme, session.outcome),
                Span::raw(" "),
                Span::styled(short_time(session.started_at), Style::new().fg(theme.dim)),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::bordered()
                .border_style(Style::new().fg(theme.dim))
                .title(" Sessions "),
        )
        .highlight_style(
            Style::new()
                .fg(theme.accent)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD),
        )
        .highlight_symbol("> ");
    let mut list_state = ListState::default();
    list_state.select(Some(selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Build the detail-pane lines for the selected session and its timeline.
fn session_detail_lines(
    theme: &Theme,
    session: Option<&CallSession>,
    detail: &Remote<CallSessionDetail>,
) -> Vec<Line<'static>> {
    let Some(session) = session else {
        return vec![
            header(theme, "Session"),
            Line::raw(""),
            hint_line(theme, "Select a session."),
        ];
    };

    let mut lines = vec![
        header(theme, "Session"),
        Line::raw(""),
        kv_line(theme, "ID:        ", session.id.clone()),
        kv_line(theme, "Booth:     ", session.booth_id.clone()),
        Line::from(vec![
            Span::styled("Outcome:   ", Style::new().fg(theme.dim)),
            outcome_badge(theme, session.outcome),
        ]),
        kv_line(theme, "Started:   ", format_ts(session.started_at)),
    ];
    if let Some(ended_at) = session.ended_at {
        lines.push(kv_line(theme, "Ended:     ", format_ts(ended_at)));
    }
    if let Some(duration_ms) = session.duration_ms {
        lines.push(kv_line(theme, "Duration:  ", format_duration(duration_ms)));
    }
    if let Some(digits) = &session.digits_dialed {
        lines.push(kv_line(theme, "Digits:    ", digits.clone()));
    }
    if let Some(recording_id) = &session.recording_id {
        lines.push(kv_line(theme, "Recording: ", recording_id.clone()));
    }
    if let Some(version) = &session.version {
        lines.push(kv_line(theme, "Version:   ", version.clone()));
    }

    lines.push(Line::raw(""));
    lines.push(subheader(theme, "Timeline"));
    push_timeline_lines(&mut lines, theme, detail);

    lines
}

/// Append the session timeline rows for the current detail load state.
fn push_timeline_lines(
    lines: &mut Vec<Line<'static>>,
    theme: &Theme,
    detail: &Remote<CallSessionDetail>,
) {
    match detail {
        Remote::Idle | Remote::Loading => {
            lines.push(note_line(theme, "  Loading timeline…".to_owned()));
        }
        Remote::Ready { value, .. } => {
            if value.events.is_empty() {
                lines.push(note_line(theme, "  (no events)".to_owned()));
            } else {
                for event in &value.events {
                    lines.push(timeline_line(theme, event));
                }
            }
        }
        Remote::Failed { error, at } => {
            lines.push(Line::from(Span::styled(
                format!("  Failed to load timeline {}.", ago(*at)),
                Style::new().fg(theme.error),
            )));
            lines.push(Line::from(vec![
                Span::styled("  Reason: ", Style::new().fg(theme.dim)),
                Span::raw(error.clone()),
            ]));
        }
    }
}

/// Render the Events screen: a master list of events beside a detail pane.
fn render_events(app: &App, frame: &mut Frame, area: Rect) {
    let theme = app.theme();
    let controller = app.events();
    match controller.state() {
        Remote::Ready { value, fetched_at } if !value.is_empty() => {
            let columns =
                Layout::horizontal([Constraint::Percentage(42), Constraint::Min(28)]).split(area);
            render_event_list(frame, columns[0], theme, value, controller.selected_index());

            let mut detail = event_detail_lines(theme, controller.selected_event());
            detail.push(Line::raw(""));
            if controller.is_following() {
                detail.push(Line::from(Span::styled(
                    "● live tail on (f to pause)".to_owned(),
                    Style::new().fg(theme.ok),
                )));
            }
            if controller.is_refreshing() {
                detail.push(note_line(theme, "Refreshing…".to_owned()));
            } else {
                detail.push(note_line(theme, format!("Fetched {}.", ago(*fetched_at))));
            }
            render_paragraph(frame, columns[1], theme, "Detail", detail);
        }
        other => {
            let mut lines = events_status_lines(theme, other);
            if controller.is_following() {
                lines.push(Line::raw(""));
                lines.push(Line::from(Span::styled(
                    "● live tail on (f to pause)".to_owned(),
                    Style::new().fg(theme.ok),
                )));
            }
            render_paragraph(frame, area, theme, "Events", lines);
        }
    }
}

/// Body lines for the non-list Events states (loading, empty, or failed).
fn events_status_lines(theme: &Theme, state: &Remote<Vec<BoothEventRecord>>) -> Vec<Line<'static>> {
    let mut lines = vec![header(theme, Screen::Events.title()), Line::raw("")];
    match state {
        Remote::Idle | Remote::Loading => lines.push(hint_line(theme, "Loading events…")),
        Remote::Ready { .. } => {
            lines.push(note_line(theme, "No events.".to_owned()));
            lines.push(hint_line(theme, "Press r to reload."));
        }
        Remote::Failed { error, at } => {
            lines.push(Line::from(Span::styled(
                format!("Failed to load events {}.", ago(*at)),
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
    lines
}

/// Render the scrollable list of events with the selected row highlighted.
fn render_event_list(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    events: &[BoothEventRecord],
    selected: usize,
) {
    let items: Vec<ListItem> = events
        .iter()
        .map(|event| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{} ", short_time(event.occurred_at)),
                    Style::new().fg(theme.dim),
                ),
                Span::styled(
                    event_type_label(event.event_type),
                    Style::new().fg(event_type_color(theme, event.event_type)),
                ),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::bordered()
                .border_style(Style::new().fg(theme.dim))
                .title(" Events "),
        )
        .highlight_style(
            Style::new()
                .fg(theme.accent)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD),
        )
        .highlight_symbol("> ");
    let mut list_state = ListState::default();
    list_state.select(Some(selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Build the detail-pane lines for the selected event, including its payload.
fn event_detail_lines(theme: &Theme, event: Option<&BoothEventRecord>) -> Vec<Line<'static>> {
    let Some(event) = event else {
        return vec![note_line(theme, "No event selected.".to_owned())];
    };
    let mut lines = vec![
        Line::from(Span::styled(
            event_type_label(event.event_type),
            Style::new()
                .fg(event_type_color(theme, event.event_type))
                .add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        kv_line(theme, "Occurred:  ", format_ts(event.occurred_at)),
        kv_line(theme, "Received:  ", format_ts(event.received_at)),
        kv_line(theme, "Booth:     ", event.booth_id.clone()),
        kv_line(theme, "Boot:      ", event.boot_id.clone()),
        kv_line(theme, "Event id:  ", event.event_id.clone()),
    ];
    if let Some(session) = &event.session_id {
        lines.push(kv_line(theme, "Session:   ", session.clone()));
    }
    if let Some(recording) = &event.recording_id {
        lines.push(kv_line(theme, "Recording: ", recording.clone()));
    }
    if let Some(version) = &event.version {
        lines.push(kv_line(theme, "Version:   ", version.clone()));
    }
    push_payload_lines(&mut lines, theme, &event.payload);
    lines
}

/// Append a pretty-printed JSON `payload` block (truncated) to the detail lines.
fn push_payload_lines(lines: &mut Vec<Line<'static>>, theme: &Theme, payload: &Value) {
    if payload.is_null() {
        return;
    }
    let pretty = serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());
    lines.push(Line::raw(""));
    lines.push(subheader(theme, "Payload"));
    for (idx, raw) in pretty.lines().enumerate() {
        if idx >= PAYLOAD_MAX_LINES {
            lines.push(note_line(theme, "… payload truncated.".to_owned()));
            break;
        }
        lines.push(Line::from(Span::styled(
            raw.to_owned(),
            Style::new().fg(theme.dim),
        )));
    }
}

/// Theme colour used to highlight an event row/heading by its type.
fn event_type_color(theme: &Theme, event_type: BoothEventType) -> Color {
    match event_type {
        BoothEventType::Error | BoothEventType::UploadFailed => theme.error,
        BoothEventType::CallStarted | BoothEventType::CallEnded => theme.ok,
        _ => theme.fg,
    }
}

/// A single timeline row: clock time and the event type.
fn timeline_line(theme: &Theme, event: &BoothEventRecord) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {} ", clock_time(event.occurred_at)),
            Style::new().fg(theme.dim),
        ),
        Span::raw(event_type_label(event.event_type)),
    ])
}

/// Colored badge for a call outcome (or a neutral marker when in progress).
fn outcome_badge(theme: &Theme, outcome: Option<CallOutcome>) -> Span<'static> {
    let Some(outcome) = outcome else {
        return badge("active", theme.accent);
    };
    let (label, color) = match outcome {
        CallOutcome::RecordingCompleted => ("completed", theme.ok),
        CallOutcome::HungUpBeforeDial => ("hung-up:pre-dial", theme.warn),
        CallOutcome::HungUpDuringPrompt => ("hung-up:prompt", theme.warn),
        CallOutcome::HungUpDuringRecording => ("hung-up:recording", theme.warn),
        CallOutcome::HungUpDuringUpload => ("hung-up:upload", theme.warn),
        CallOutcome::RecordingFailed => ("recording-failed", theme.error),
        CallOutcome::UploadFailed => ("upload-failed", theme.error),
        CallOutcome::OperatorError => ("operator-error", theme.error),
        CallOutcome::Aborted => ("aborted", theme.error),
    };
    badge(label, color)
}

/// Human-readable label for a booth event type.
fn event_type_label(event_type: BoothEventType) -> &'static str {
    match event_type {
        BoothEventType::CallStarted => "call started",
        BoothEventType::CallEnded => "call ended",
        BoothEventType::DigitDialed => "digit dialed",
        BoothEventType::StateTransition => "state transition",
        BoothEventType::RecordingStarted => "recording started",
        BoothEventType::RecordingStopped => "recording stopped",
        BoothEventType::UploadStarted => "upload started",
        BoothEventType::UploadCompleted => "upload completed",
        BoothEventType::UploadFailed => "upload failed",
        BoothEventType::GpioEdge => "GPIO edge",
        BoothEventType::AudioDeviceChange => "audio device change",
        BoothEventType::OperatorRequest => "operator request",
        BoothEventType::OperatorResponse => "operator response",
        BoothEventType::Error => "error",
        BoothEventType::Log => "log",
        BoothEventType::SystemSample => "system sample",
    }
}

/// A `HH:MM:SS` clock time for timeline rows.
fn clock_time(at: OffsetDateTime) -> String {
    let formatted = format_ts(at);
    if formatted.is_char_boundary(11) && formatted.is_char_boundary(19) && formatted.len() >= 19 {
        formatted[11..19].to_owned()
    } else {
        formatted
    }
}

/// Body lines for the Statistics screen.
fn stats_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let controller = app.stats();
    match controller.state() {
        Remote::Ready { value, fetched_at } => {
            stats_ready_lines(theme, value, *fetched_at, controller.is_refreshing())
        }
        other => {
            let mut lines = vec![header(theme, Screen::Stats.title()), Line::raw("")];
            lines.push(kv_line(
                theme,
                "Window:    ",
                stats_window_label(controller.window()).to_owned(),
            ));
            lines.push(Line::raw(""));
            match other {
                Remote::Failed { error, at } => {
                    lines.push(Line::from(Span::styled(
                        format!("Failed to load statistics: {error}"),
                        Style::new().fg(theme.error),
                    )));
                    lines.push(note_line(theme, format!("Last attempt {}.", ago(*at))));
                    lines.push(hint_line(theme, "Press r to retry."));
                }
                _ => lines.push(note_line(theme, "Loading statistics…".to_owned())),
            }
            lines
        }
    }
}

/// The full statistics dashboard for a loaded overview.
fn stats_ready_lines(
    theme: &Theme,
    overview: &StatsOverview,
    fetched_at: Instant,
    refreshing: bool,
) -> Vec<Line<'static>> {
    let mut lines = vec![header(theme, Screen::Stats.title()), Line::raw("")];

    lines.push(kv_line(
        theme,
        "Window:    ",
        stats_window_label(overview.window).to_owned(),
    ));
    let range = overview.range_start.map_or_else(
        || format!("up to {}", short_time(overview.range_end)),
        |start| format!("{} → {}", short_time(start), short_time(overview.range_end)),
    );
    lines.push(kv_line(theme, "Range:     ", range));
    if let Some(last) = overview.last_activity_at {
        lines.push(kv_line(theme, "Activity:  ", format_ts(last)));
    }

    let calls = &overview.calls;
    lines.push(Line::raw(""));
    lines.push(subheader(theme, "Calls"));
    lines.push(kv_line(theme, "Total:     ", calls.total.to_string()));
    lines.push(kv_line(
        theme,
        "Completed: ",
        format!(
            "{} ({})",
            calls.completed,
            percent(calls.completed, calls.total)
        ),
    ));
    if calls.in_progress > 0 {
        lines.push(kv_line(theme, "In flight: ", calls.in_progress.to_string()));
    }
    if let Some(avg) = calls.average_duration_ms {
        lines.push(kv_line(theme, "Avg call:  ", format_millis_f64(avg)));
    }
    if let Some(longest) = calls.longest_duration_ms {
        lines.push(kv_line(theme, "Longest:   ", format_millis_f64(longest)));
    }
    push_count_map(&mut lines, theme, &calls.outcomes);

    let messages = &overview.messages;
    lines.push(Line::raw(""));
    lines.push(subheader(theme, "Messages"));
    lines.push(kv_line(theme, "Total:     ", messages.total.to_string()));
    if let Some(avg) = messages.average_duration_ms {
        lines.push(kv_line(theme, "Avg len:   ", format_millis_f64(avg)));
    }
    push_count_map(&mut lines, theme, &messages.by_status);

    let pickups = &overview.pickups_hangups;
    lines.push(Line::raw(""));
    lines.push(subheader(theme, "Activity"));
    lines.push(kv_line(
        theme,
        "Playbacks: ",
        overview.playback.total_playbacks.to_string(),
    ));
    lines.push(kv_line(theme, "Pickups:   ", pickups.pickups.to_string()));
    lines.push(kv_line(theme, "Hangups:   ", pickups.hangups.to_string()));
    if !pickups.digits_dialed.is_empty() {
        let digits = pickups
            .digits_dialed
            .iter()
            .map(|(digit, count)| format!("{digit}:{count}"))
            .collect::<Vec<_>>()
            .join("  ");
        lines.push(kv_line(theme, "Digits:    ", digits));
    }

    let uploads = &overview.uploads;
    lines.push(Line::raw(""));
    lines.push(subheader(theme, "Uploads"));
    lines.push(kv_line(theme, "Succeeded: ", uploads.succeeded.to_string()));
    lines.push(kv_line(theme, "Failed:    ", uploads.failed.to_string()));
    if let Some(rate) = uploads.failure_rate {
        lines.push(kv_line(
            theme,
            "Fail rate: ",
            format!("{:.1}%", rate * 100.0),
        ));
    }

    lines.push(Line::raw(""));
    lines.push(subheader(theme, "When"));
    let busy_hour = overview
        .busiest
        .hour
        .map_or_else(|| "—".to_owned(), |hour| format!("{hour:02}:00 UTC"));
    lines.push(kv_line(theme, "Busy hour: ", busy_hour));
    let busy_day = overview.busiest.day_of_week.map_or("—", day_of_week_label);
    lines.push(kv_line(theme, "Busy day:  ", busy_day.to_owned()));
    if !overview.hourly.is_empty() {
        let mut by_hour = [0_u64; 24];
        for bucket in &overview.hourly {
            if let Some(slot) = by_hour.get_mut(bucket.hour as usize) {
                *slot = bucket.calls;
            }
        }
        lines.push(kv_line(theme, "Calls/hr:  ", sparkline(&by_hour)));
        lines.push(note_line(
            theme,
            "(per hour, 00–23 UTC; leftmost = midnight)".to_owned(),
        ));
    }

    if !overview.top_questions.is_empty() {
        lines.push(Line::raw(""));
        lines.push(subheader(theme, "Top questions"));
        for question in overview.top_questions.iter().take(5) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:>4}  ", question.message_count),
                    Style::new().fg(theme.accent),
                ),
                Span::raw(truncate(&question.prompt, 60)),
            ]));
        }
    }

    if !overview.booth_breakdown.is_empty() {
        lines.push(Line::raw(""));
        lines.push(subheader(theme, "Booths"));
        for booth in &overview.booth_breakdown {
            let messages = booth
                .messages
                .map_or_else(String::new, |count| format!(", {count} msgs"));
            lines.push(Line::from(vec![
                Span::styled(format!("{}  ", booth.booth_id), Style::new().fg(theme.dim)),
                Span::raw(format!("{} calls{messages}", booth.calls)),
            ]));
        }
    }

    lines.push(Line::raw(""));
    if refreshing {
        lines.push(note_line(theme, "Refreshing…".to_owned()));
    } else {
        lines.push(note_line(
            theme,
            format!(
                "Fetched {} · generated {}.",
                ago(fetched_at),
                format_ts(overview.generated_at)
            ),
        ));
    }
    lines
}

/// Human-readable label for a statistics window.
fn stats_window_label(window: StatsWindow) -> &'static str {
    match window {
        StatsWindow::Day => "Last 24 hours",
        StatsWindow::Week => "Last 7 days",
        StatsWindow::Month => "Last 30 days",
        StatsWindow::All => "All time",
    }
}

/// Name of a day-of-week index (`0` = Sunday).
fn day_of_week_label(day: u8) -> &'static str {
    match day {
        0 => "Sunday",
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        6 => "Saturday",
        _ => "—",
    }
}

/// Append a dim, indented `key: count` line for each entry of a count map.
fn push_count_map(
    lines: &mut Vec<Line<'static>>,
    theme: &Theme,
    map: &std::collections::BTreeMap<String, u64>,
) {
    for (key, count) in map {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<11}", humanize_key(key)),
                Style::new().fg(theme.dim),
            ),
            Span::raw(count.to_string()),
        ]));
    }
}

/// Replace separators in an enum-style key with spaces for display.
fn humanize_key(key: &str) -> String {
    key.replace(['_', '-'], " ")
}

/// An integer percentage of `part` out of `whole` (e.g. `75%`).
fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "0%".to_owned();
    }
    format!("{}%", part.saturating_mul(100) / whole)
}

/// Format a millisecond duration held as `f64` (e.g. `1m 05s`, `4.2s`).
fn format_millis_f64(ms: f64) -> String {
    if !ms.is_finite() || ms < 0.0 {
        return "—".to_owned();
    }
    // Round to whole seconds first so splitting into minutes/seconds can never
    // produce a carry like "1m 60s".
    let total_seconds = (ms / 1000.0).round();
    if total_seconds >= 60.0 {
        let minutes = (total_seconds / 60.0).floor();
        let remainder = total_seconds - minutes * 60.0;
        format!("{minutes:.0}m {remainder:02.0}s")
    } else {
        format!("{:.1}s", ms / 1000.0)
    }
}

/// Render a slice of counts as a compact Unicode bar sparkline.
fn sparkline(values: &[u64]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = values.iter().copied().max().unwrap_or(0);
    if max == 0 {
        return BARS[0].to_string().repeat(values.len());
    }
    values
        .iter()
        .map(|&value| {
            let index = usize::try_from(value.saturating_mul(7) / max).unwrap_or(7);
            BARS[index.min(7)]
        })
        .collect()
}

/// Body lines for the Live System screen.
fn live_system_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let controller = app.system();
    match controller.state() {
        Remote::Ready { value, fetched_at } if !value.items.is_empty() => {
            let mut lines = vec![header(theme, Screen::LiveSystem.title()), Line::raw("")];
            for envelope in &value.items {
                push_system_envelope(&mut lines, theme, envelope);
                lines.push(Line::raw(""));
            }
            if controller.is_refreshing() {
                lines.push(note_line(theme, "Refreshing…".to_owned()));
            } else {
                lines.push(note_line(
                    theme,
                    format!("Polled {} · auto-refreshes every 5s.", ago(*fetched_at)),
                ));
            }
            lines
        }
        other => {
            let mut lines = vec![header(theme, Screen::LiveSystem.title()), Line::raw("")];
            match other {
                Remote::Failed { error, at } => {
                    lines.push(Line::from(Span::styled(
                        format!("Failed to load system snapshot: {error}"),
                        Style::new().fg(theme.error),
                    )));
                    lines.push(note_line(theme, format!("Last attempt {}.", ago(*at))));
                    lines.push(hint_line(theme, "Press r to retry."));
                }
                Remote::Ready { .. } => {
                    lines.push(note_line(
                        theme,
                        "No booths have reported a system snapshot yet.".to_owned(),
                    ));
                }
                _ => lines.push(note_line(theme, "Loading system snapshot…".to_owned())),
            }
            lines
        }
    }
}

/// Append one booth's readout section.
fn push_system_envelope(
    lines: &mut Vec<Line<'static>>,
    theme: &Theme,
    envelope: &BoothSystemSnapshotEnvelope,
) {
    let mut head = vec![Span::styled(
        format!("Booth {}", envelope.booth_id),
        Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
    )];
    if let Some(version) = &envelope.version {
        head.push(Span::styled(
            format!("  v{version}"),
            Style::new().fg(theme.dim),
        ));
    }
    lines.push(Line::from(head));
    lines.push(note_line(
        theme,
        format!("Updated {}", format_ts(envelope.received_at)),
    ));

    let snapshot = &envelope.snapshot;
    push_cpu_lines(lines, theme, snapshot);
    push_memory_lines(lines, theme, snapshot);
    push_disk_lines(lines, theme, snapshot);
    push_network_lines(lines, theme, snapshot);
    push_host_lines(lines, theme, snapshot);
}

/// CPU, temperature, and load-average lines.
fn push_cpu_lines(lines: &mut Vec<Line<'static>>, theme: &Theme, snapshot: &BoothSystemSnapshot) {
    if let Some(temp) = snapshot.temperature_celsius {
        lines.push(kv_line(theme, "Temp:      ", format!("{temp:.1} °C")));
    }
    if let Some(mode) = snapshot.runtime_mode {
        lines.push(kv_line(theme, "Mode:      ", mode_label(mode).to_owned()));
    }
    let Some(cpu) = &snapshot.cpu else {
        return;
    };
    if let Some(usage) = cpu.usage_ratio {
        lines.push(kv_line(
            theme,
            "CPU:       ",
            format!("{} {}", percent_bar(usage), format_ratio(usage)),
        ));
    }
    if cpu.load_avg1m.is_some() || cpu.load_avg5m.is_some() || cpu.load_avg15m.is_some() {
        lines.push(kv_line(
            theme,
            "Load:      ",
            format!(
                "{} / {} / {}  (1m/5m/15m)",
                optional_load(cpu.load_avg1m),
                optional_load(cpu.load_avg5m),
                optional_load(cpu.load_avg15m),
            ),
        ));
    }
    if let Some(cores) = cpu.physical_cores {
        lines.push(kv_line(theme, "Cores:     ", cores.to_string()));
    }
    if let Some(per_core) = &cpu.per_core_usage_ratio
        && !per_core.is_empty()
    {
        let bars: String = per_core.iter().map(|&ratio| ratio_block(ratio)).collect();
        lines.push(kv_line(theme, "Per-core:  ", bars));
    }
}

/// Memory and swap lines.
fn push_memory_lines(
    lines: &mut Vec<Line<'static>>,
    theme: &Theme,
    snapshot: &BoothSystemSnapshot,
) {
    let Some(memory) = &snapshot.memory else {
        return;
    };
    if let (Some(used), Some(total)) = (memory.used_bytes, memory.total_bytes)
        && total > 0
    {
        lines.push(kv_line(
            theme,
            "Memory:    ",
            format!(
                "{} {} / {} ({})",
                percent_bar(ratio_of(used, total)),
                format_bytes(used),
                format_bytes(total),
                percent(used, total),
            ),
        ));
    }
    if let (Some(used), Some(total)) = (memory.swap_used_bytes, memory.swap_total_bytes)
        && total > 0
    {
        lines.push(kv_line(
            theme,
            "Swap:      ",
            format!(
                "{} / {} ({})",
                format_bytes(used),
                format_bytes(total),
                percent(used, total)
            ),
        ));
    }
}

/// Per-mount disk usage lines.
fn push_disk_lines(lines: &mut Vec<Line<'static>>, theme: &Theme, snapshot: &BoothSystemSnapshot) {
    let Some(disks) = &snapshot.disks else {
        return;
    };
    for disk in disks {
        let used = disk.total_bytes.saturating_sub(disk.available_bytes);
        lines.push(Line::from(vec![
            Span::styled(
                format!("Disk {:<6} ", truncate(&disk.mount_point, 6)),
                Style::new().fg(theme.dim),
            ),
            Span::raw(format!(
                "{} {} / {} ({} free)",
                percent_bar(ratio_of(used, disk.total_bytes)),
                format_bytes(used),
                format_bytes(disk.total_bytes),
                format_bytes(disk.available_bytes),
            )),
        ]));
    }
}

/// Per-interface network counter lines.
fn push_network_lines(
    lines: &mut Vec<Line<'static>>,
    theme: &Theme,
    snapshot: &BoothSystemSnapshot,
) {
    let Some(networks) = &snapshot.networks else {
        return;
    };
    for network in networks {
        lines.push(Line::from(vec![
            Span::styled(
                format!("Net {:<7} ", truncate(&network.interface, 7)),
                Style::new().fg(theme.dim),
            ),
            Span::raw(format!(
                "↓ {}  ↑ {}",
                format_bytes(network.receive_bytes_total),
                format_bytes(network.transmit_bytes_total),
            )),
        ]));
    }
}

/// Uptime, process, audio, Tailscale, and throttling lines.
fn push_host_lines(lines: &mut Vec<Line<'static>>, theme: &Theme, snapshot: &BoothSystemSnapshot) {
    if let Some(uptime) = snapshot.uptime_seconds {
        lines.push(kv_line(theme, "Uptime:    ", format_uptime(uptime)));
    }
    if let Some(audio) = &snapshot.audio {
        let input = audio.input_device.clone().unwrap_or_else(|| "—".to_owned());
        let output = audio
            .output_device
            .clone()
            .unwrap_or_else(|| "—".to_owned());
        lines.push(kv_line(theme, "Audio in:  ", input));
        lines.push(kv_line(theme, "Audio out: ", output));
    }
    if let Some(tailscale) = &snapshot.tailscale {
        let connected = match tailscale.connected {
            Some(true) => "connected",
            Some(false) => "offline",
            None => "unknown",
        };
        let value = tailscale.hostname.as_ref().map_or_else(
            || connected.to_owned(),
            |host| format!("{connected} ({host})"),
        );
        lines.push(kv_line(theme, "Tailscale: ", value));
    }
    if let Some(throttling) = &snapshot.throttling {
        let mut flags = Vec::new();
        if throttling.undervoltage == Some(true) {
            flags.push("under-voltage");
        }
        if throttling.throttled == Some(true) {
            flags.push("throttled");
        }
        if throttling.arm_freq_capped == Some(true) {
            flags.push("freq-capped");
        }
        if throttling.soft_temp_limit == Some(true) {
            flags.push("soft-temp-limit");
        }
        if !flags.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Throttling:", Style::new().fg(theme.dim)),
                Span::styled(
                    format!(" {}", flags.join(", ")),
                    Style::new().fg(theme.warn),
                ),
            ]));
        }
    }
}

/// Format a CPU load average, or `—` when absent.
fn optional_load(load: Option<f64>) -> String {
    load.map_or_else(|| "—".to_owned(), |value| format!("{value:.2}"))
}

/// Render the btm-style System Health dashboard for the configured booth.
///
/// Falls back to a guidance paragraph when no booth is configured, and to a
/// status paragraph while the first scrape is pending or has only failed.
fn render_system_health(app: &App, frame: &mut Frame, area: Rect) {
    let theme = app.theme();
    let Some(controller) = app.system_health() else {
        render_paragraph(
            frame,
            area,
            theme,
            "System Health",
            system_health_unconfigured_lines(theme),
        );
        return;
    };
    if controller.samples() == 0 {
        render_paragraph(
            frame,
            area,
            theme,
            &format!("System Health — {}", controller.label()),
            system_health_pending_lines(theme, controller),
        );
        return;
    }

    let block = section_block(theme, format!(" System Health — {} ", controller.label()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Min(8),
        Constraint::Min(7),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .split(inner);
    render_cpu_section(frame, rows[0], theme, controller);
    render_network_section(frame, rows[1], theme, controller.history());
    render_disk_section(frame, rows[2], theme, controller.history());
    render_health_status(frame, rows[3], theme, controller);
}

/// Guidance shown on the System Health screen when no booth is configured.
fn system_health_unconfigured_lines(theme: &Theme) -> Vec<Line<'static>> {
    vec![
        header(theme, "System Health"),
        Line::raw(""),
        Line::raw("No booth is configured, so there is no /metrics endpoint to scrape."),
        Line::raw(""),
        hint_line(
            theme,
            "Add a [[booths]] entry (id + debug-base-url, optional debug-token) to config.toml.",
        ),
    ]
}

/// Status shown before the first successful scrape (pending or only failed).
fn system_health_pending_lines(
    theme: &Theme,
    controller: &SystemHealthController,
) -> Vec<Line<'static>> {
    let mut lines = vec![header(theme, "System Health"), Line::raw("")];
    if let Some((error, at)) = controller.last_error() {
        lines.push(Line::from(Span::styled(
            format!("Failed to scrape booth /metrics: {error}"),
            Style::new().fg(theme.error),
        )));
        lines.push(note_line(theme, format!("Last attempt {}.", ago(*at))));
        lines.push(hint_line(
            theme,
            "Press r to retry. Check the booth URL/token and Tailscale reachability.",
        ));
    } else {
        lines.push(note_line(
            theme,
            format!("Scraping {} for /metrics…", controller.label()),
        ));
    }
    lines
}

/// Top dashboard row: the CPU history chart beside a memory + vitals panel.
fn render_cpu_section(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    controller: &SystemHealthController,
) {
    let columns =
        Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).split(area);
    render_cpu_chart(frame, columns[0], theme, controller.history());
    render_vitals_panel(frame, columns[1], theme, controller.history());
}

/// The CPU-usage line chart (0–100%).
fn render_cpu_chart(frame: &mut Frame, area: Rect, theme: &Theme, history: &MetricsHistory) {
    let values = history.cpu_usage().to_vec();
    let points = series_points(&values, 100.0);
    let current = values.last().copied().unwrap_or(0.0);
    let x_max = axis_max(points.len());
    let datasets = vec![
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(theme.accent))
            .data(&points),
    ];
    let chart = Chart::new(datasets)
        .block(section_block(
            theme,
            format!(" CPU  {} ", format_ratio(current)),
        ))
        .x_axis(
            Axis::default()
                .style(Style::new().fg(theme.dim))
                .bounds([0.0, x_max]),
        )
        .y_axis(
            Axis::default()
                .style(Style::new().fg(theme.dim))
                .bounds([0.0, 100.0])
                .labels([Line::from("0"), Line::from("50"), Line::from("100")]),
        );
    frame.render_widget(chart, area);
}

/// The right-hand panel of the top row: a memory gauge over a vitals readout.
fn render_vitals_panel(frame: &mut Frame, area: Rect, theme: &Theme, history: &MetricsHistory) {
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);
    render_memory_gauge(frame, rows[0], theme, history);
    render_vitals_text(frame, rows[1], theme, history);
}

/// The memory-usage gauge.
fn render_memory_gauge(frame: &mut Frame, area: Rect, theme: &Theme, history: &MetricsHistory) {
    let latest = history.latest();
    let ratio = latest
        .and_then(BoothMetrics::memory_used_ratio)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let label = match (
        latest.and_then(|m| m.memory_used_bytes),
        latest.and_then(|m| m.memory_total_bytes),
    ) {
        (Some(used), Some(total)) => format!(
            "{} / {} ({})",
            format_bytes_f64(used),
            format_bytes_f64(total),
            format_ratio(ratio),
        ),
        _ => "—".to_owned(),
    };
    let gauge = Gauge::default()
        .block(section_block(theme, " Memory ".to_owned()))
        .gauge_style(Style::new().fg(theme.accent))
        .ratio(ratio)
        .label(label);
    frame.render_widget(gauge, area);
}

/// Temperature, load average, and uptime read-outs.
fn render_vitals_text(frame: &mut Frame, area: Rect, theme: &Theme, history: &MetricsHistory) {
    let mut lines = Vec::new();
    if let Some(metrics) = history.latest() {
        if let Some(temp) = metrics.cpu_temperature_celsius {
            lines.push(kv_line(theme, "Temp:   ", format!("{temp:.1} °C")));
        }
        lines.push(kv_line(
            theme,
            "Load:   ",
            format!(
                "{} / {} / {}",
                optional_load(metrics.load_average_1m),
                optional_load(metrics.load_average_5m),
                optional_load(metrics.load_average_15m),
            ),
        ));
        if let Some(uptime) = metrics.uptime_seconds {
            lines.push(kv_line(theme, "Uptime: ", format_uptime(uptime)));
        }
    }
    let paragraph = Paragraph::new(lines)
        .block(section_block(theme, " Vitals ".to_owned()))
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

/// The network throughput chart (receive and transmit rates, bytes/sec).
fn render_network_section(frame: &mut Frame, area: Rect, theme: &Theme, history: &MetricsHistory) {
    let rx = history.net_receive_rate().to_vec();
    let tx = history.net_transmit_rate().to_vec();
    if rx.is_empty() && tx.is_empty() {
        let paragraph = Paragraph::new(vec![note_line(
            theme,
            "Measuring network throughput…".to_owned(),
        )])
        .block(section_block(theme, " Network ".to_owned()));
        frame.render_widget(paragraph, area);
        return;
    }

    let rx_points = series_points(&rx, 1.0);
    let tx_points = series_points(&tx, 1.0);
    let current_rx = rx.last().copied().unwrap_or(0.0);
    let current_tx = tx.last().copied().unwrap_or(0.0);
    // Floor the y-axis at 1 KiB/s so a quiet link still renders a flat baseline
    // rather than a degenerate zero-height chart.
    let y_max = rx
        .iter()
        .chain(tx.iter())
        .copied()
        .fold(0.0_f64, f64::max)
        .max(1024.0);
    let x_max = axis_max(rx_points.len().max(tx_points.len()));
    let datasets = vec![
        Dataset::default()
            .name("rx")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(theme.ok))
            .data(&rx_points),
        Dataset::default()
            .name("tx")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(theme.warn))
            .data(&tx_points),
    ];
    let title = format!(
        " Network  ↓ {}/s  ↑ {}/s ",
        format_bytes_f64(current_rx),
        format_bytes_f64(current_tx),
    );
    let chart = Chart::new(datasets)
        .block(section_block(theme, title))
        .x_axis(
            Axis::default()
                .style(Style::new().fg(theme.dim))
                .bounds([0.0, x_max]),
        )
        .y_axis(
            Axis::default()
                .style(Style::new().fg(theme.dim))
                .bounds([0.0, y_max])
                .labels([
                    Line::from("0"),
                    Line::from(format!("{}/s", format_bytes_f64(y_max))),
                ]),
        );
    frame.render_widget(chart, area);
}

/// Per-mountpoint disk-usage rows.
fn render_disk_section(frame: &mut Frame, area: Rect, theme: &Theme, history: &MetricsHistory) {
    let mut lines = Vec::new();
    if let Some(metrics) = history.latest() {
        if metrics.disks.is_empty() {
            lines.push(note_line(theme, "No disk metrics reported.".to_owned()));
        } else {
            for disk in &metrics.disks {
                lines.push(disk_line(theme, disk));
            }
        }
    }
    let paragraph = Paragraph::new(lines)
        .block(section_block(theme, " Disks ".to_owned()))
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

/// One disk row: a usage bar with used / total / percentage detail.
fn disk_line(theme: &Theme, disk: &tbo_metrics::DiskUsage) -> Line<'static> {
    let ratio = disk.used_ratio().unwrap_or(0.0);
    let detail = match (disk.used_bytes, disk.total_bytes) {
        (Some(used), Some(total)) => format!(
            "{} / {} ({})",
            format_bytes_f64(used),
            format_bytes_f64(total),
            format_ratio(ratio),
        ),
        _ => "—".to_owned(),
    };
    Line::from(vec![
        Span::styled(
            format!("{:<10} ", truncate(&disk.mountpoint, 10)),
            Style::new().fg(theme.dim),
        ),
        Span::raw(format!("{} {detail}", percent_bar(ratio))),
    ])
}

/// The dashboard footer: sample count, freshness, refresh hint, and any error.
fn render_health_status(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    controller: &SystemHealthController,
) {
    let mut spans = vec![Span::styled(
        format!("{} samples", controller.samples()),
        Style::new().fg(theme.dim),
    )];
    if let Some(last) = controller.last_ok() {
        spans.push(Span::styled(
            format!(" · scraped {}", ago(last)),
            Style::new().fg(theme.dim),
        ));
    }
    if controller.is_refreshing() {
        spans.push(Span::styled(" · refreshing…", Style::new().fg(theme.warn)));
    } else {
        spans.push(Span::styled(
            " · auto every 2s · r to refresh",
            Style::new().fg(theme.dim),
        ));
    }
    if let Some((error, _)) = controller.last_error() {
        spans.push(Span::styled(
            format!(" · last error: {error}"),
            Style::new().fg(theme.error),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Render the booth Debug panel for the configured booth.
///
/// Falls back to a guidance paragraph when no booth is configured, and to a
/// status paragraph while the first poll is pending or has only failed.
fn render_debug(app: &App, frame: &mut Frame, area: Rect) {
    let theme = app.theme();
    let Some(controller) = app.debug() else {
        render_paragraph(frame, area, theme, "Debug", debug_unconfigured_lines(theme));
        return;
    };
    if controller.state().is_none() && controller.config().is_none() {
        render_paragraph(
            frame,
            area,
            theme,
            &format!("Debug — {}", controller.label()),
            debug_pending_lines(theme, controller),
        );
        return;
    }

    let block = section_block(
        theme,
        format!(
            " Debug — {} · {} ",
            controller.label(),
            controller.base_url()
        ),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(7),
        Constraint::Min(6),
        Constraint::Length(8),
        Constraint::Length(1),
    ])
    .split(inner);

    let top =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[0]);
    render_debug_state(frame, top[0], theme, controller);
    render_debug_audio(frame, top[1], theme, controller);

    let middle =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).split(rows[1]);
    render_debug_gpio(frame, middle[0], theme, controller);
    render_debug_logs(frame, middle[1], theme, controller);

    render_debug_config(frame, rows[2], theme, controller);
    render_debug_status(frame, rows[3], theme, controller);
}

/// Guidance shown on the Debug screen when no booth is configured.
fn debug_unconfigured_lines(theme: &Theme) -> Vec<Line<'static>> {
    vec![
        header(theme, "Debug"),
        Line::raw(""),
        Line::raw("No booth is configured, so there is no debug server to reach."),
        Line::raw(""),
        hint_line(
            theme,
            "Add a [[booths]] entry (id + debug-base-url, optional debug-token) to config.toml.",
        ),
    ]
}

/// Status shown before the first successful poll (pending or only failed).
fn debug_pending_lines(theme: &Theme, controller: &DebugController) -> Vec<Line<'static>> {
    let mut lines = vec![header(theme, "Debug"), Line::raw("")];
    if let Some((error, at)) = controller.last_error() {
        lines.push(Line::from(Span::styled(
            format!("Failed to reach booth debug server: {error}"),
            Style::new().fg(theme.error),
        )));
        lines.push(note_line(theme, format!("Last attempt {}.", ago(*at))));
        lines.push(hint_line(
            theme,
            "Press r to retry. Check the booth URL/token and Tailscale reachability.",
        ));
    } else {
        lines.push(note_line(
            theme,
            format!("Polling {} for debug snapshots…", controller.label()),
        ));
    }
    lines
}

/// The booth state-machine panel.
fn render_debug_state(frame: &mut Frame, area: Rect, theme: &Theme, controller: &DebugController) {
    let mut lines = Vec::new();
    if let Some(state) = controller.state() {
        lines.push(Line::from(vec![
            Span::styled("State:    ", Style::new().fg(theme.dim)),
            Span::styled(
                state.state.clone(),
                Style::new()
                    .fg(debug_state_color(theme, &state.state))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(kv_line(theme, "Updated:  ", state.updated_at.clone()));
        if let Some(question) = &state.current_question_id {
            lines.push(kv_line(theme, "Question: ", question.clone()));
        }
        if let Some(message) = &state.current_message_id {
            lines.push(kv_line(theme, "Message:  ", message.clone()));
        }
        if let Some(error) = &state.last_error {
            lines.push(Line::from(vec![
                Span::styled("Error:    ", Style::new().fg(theme.dim)),
                Span::styled(error.clone(), Style::new().fg(theme.error)),
            ]));
        }
    } else {
        lines.push(note_line(theme, "No state reported.".to_owned()));
    }
    let paragraph = Paragraph::new(lines)
        .block(section_block(theme, " State ".to_owned()))
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

/// Accent color for a booth state name (red for errors, dim when idle).
fn debug_state_color(theme: &Theme, state: &str) -> Color {
    match state {
        "error" => theme.error,
        "idle" => theme.dim,
        _ => theme.ok,
    }
}

/// The audio level-meter panel.
fn render_debug_audio(frame: &mut Frame, area: Rect, theme: &Theme, controller: &DebugController) {
    let mut lines = Vec::new();
    if let Some(audio) = controller.audio() {
        lines.push(audio_meter_line(
            theme,
            "In  ",
            audio.input_level_dbfs,
            audio.input_peak_dbfs,
        ));
        lines.push(audio_meter_line(
            theme,
            "Out ",
            audio.output_level_dbfs,
            audio.output_peak_dbfs,
        ));
        if let Some(device) = &audio.current_device {
            lines.push(kv_line(theme, "Device: ", device.clone()));
        }
        if let Some(rate) = audio.sample_rate_hz {
            lines.push(kv_line(theme, "Rate:   ", format!("{rate} Hz")));
        }
    } else {
        lines.push(note_line(theme, "No audio meters reported.".to_owned()));
    }
    let paragraph = Paragraph::new(lines)
        .block(section_block(theme, " Audio ".to_owned()))
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

/// One audio channel meter: a level bar with level and peak dBFS read-outs.
fn audio_meter_line(
    theme: &Theme,
    label: &'static str,
    level_dbfs: f32,
    peak_dbfs: f32,
) -> Line<'static> {
    let ratio = dbfs_ratio(level_dbfs);
    Line::from(vec![
        Span::styled(label, Style::new().fg(theme.dim)),
        Span::raw(format!(
            "{} {:>6.1} dBFS  peak {:>6.1}",
            percent_bar(ratio),
            f64::from(level_dbfs),
            f64::from(peak_dbfs),
        )),
    ])
}

/// Map a dBFS level (`-120..=0`) to a `0.0..=1.0` meter ratio.
fn dbfs_ratio(dbfs: f32) -> f64 {
    ((f64::from(dbfs) + 120.0) / 120.0).clamp(0.0, 1.0)
}

/// The GPIO pin-state panel.
fn render_debug_gpio(frame: &mut Frame, area: Rect, theme: &Theme, controller: &DebugController) {
    let mut lines = Vec::new();
    match controller.gpio() {
        Some(gpio) if !gpio.pins.is_empty() => {
            for pin in &gpio.pins {
                lines.push(gpio_pin_line(theme, pin));
            }
        }
        Some(_) => lines.push(note_line(theme, "No GPIO pins reported.".to_owned())),
        None => lines.push(note_line(theme, "No GPIO snapshot.".to_owned())),
    }
    let paragraph = Paragraph::new(lines)
        .block(section_block(theme, " GPIO ".to_owned()))
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

/// One GPIO pin row: role, current level, and last telemetry id.
fn gpio_pin_line(theme: &Theme, pin: &GpioPinSnapshot) -> Line<'static> {
    let (text, color) = if pin.level {
        ("HIGH", theme.ok)
    } else {
        ("LOW ", theme.dim)
    };
    Line::from(vec![
        Span::styled(
            format!("{:<13} ", truncate(&pin.role, 13)),
            Style::new().fg(theme.dim),
        ),
        Span::styled(text, Style::new().fg(color)),
        Span::styled(
            format!("  #{}", pin.last_event_id),
            Style::new().fg(theme.dim),
        ),
    ])
}

/// The live-logs panel (newest first), filtered by the current level.
fn render_debug_logs(frame: &mut Frame, area: Rect, theme: &Theme, controller: &DebugController) {
    let logs = controller.logs();
    let lines: Vec<Line<'static>> = if logs.is_empty() {
        vec![note_line(theme, "No log lines.".to_owned())]
    } else {
        logs.iter()
            .rev()
            .map(|entry| log_line(theme, entry))
            .collect()
    };
    let title = format!(" Logs · {} ", controller.log_level());
    let paragraph = Paragraph::new(lines)
        .block(section_block(theme, title))
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

/// One log row: time, level (colored), and message.
fn log_line(theme: &Theme, entry: &LogEntry) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{} ", short_ts_str(&entry.ts)),
            Style::new().fg(theme.dim),
        ),
        Span::styled(
            format!("{:<5} ", entry.level.to_uppercase()),
            Style::new().fg(log_level_color(theme, &entry.level)),
        ),
        Span::raw(truncate(&entry.message, 80)),
    ])
}

/// Accent color for a tracing level.
fn log_level_color(theme: &Theme, level: &str) -> Color {
    match level {
        "error" => theme.error,
        "warn" => theme.warn,
        "debug" | "trace" => theme.dim,
        _ => theme.ok,
    }
}

/// Extract the `HH:MM:SS` time portion from an RFC3339 timestamp string.
fn short_ts_str(ts: &str) -> String {
    if ts.len() >= 19 && ts.is_char_boundary(11) && ts.is_char_boundary(19) {
        ts[11..19].to_owned()
    } else {
        ts.to_owned()
    }
}

/// The redacted-config panel plus the pinned certificate fingerprint footer.
fn render_debug_config(frame: &mut Frame, area: Rect, theme: &Theme, controller: &DebugController) {
    let mut lines = Vec::new();
    if let Some(config) = controller.config() {
        let debug = &config.debug;
        let (controls_text, controls_color) = if debug.allow_controls {
            ("allowed", theme.warn)
        } else {
            ("blocked", theme.dim)
        };
        lines.push(Line::from(vec![
            Span::styled("Mode: ", Style::new().fg(theme.dim)),
            Span::raw(mode_label(debug.runtime_mode)),
            Span::styled("   Controls: ", Style::new().fg(theme.dim)),
            Span::styled(controls_text, Style::new().fg(controls_color)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Listeners: ", Style::new().fg(theme.dim)),
            Span::raw(format!(
                "tailscale {} · lan {} · ring {}",
                bool_label(debug.tailscale_enabled),
                bool_label(debug.lan_enabled),
                debug.ring_buffer_capacity,
            )),
        ]));
        if let Some(base) = &config.operator.base_url {
            lines.push(kv_line(theme, "Operator: ", base.clone()));
        }
        lines.push(kv_line(theme, "Op token: ", config.operator.token.clone()));
    } else {
        lines.push(note_line(theme, "No config reported.".to_owned()));
    }
    let fingerprint = controller
        .pinned_sha256()
        .map_or_else(|| "not pinned".to_owned(), str::to_owned);
    lines.push(kv_line(theme, "Cert fp:  ", fingerprint));
    let paragraph = Paragraph::new(lines)
        .block(section_block(theme, " Config ".to_owned()))
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

/// Render `on`/`off` for a boolean flag.
fn bool_label(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// The Debug footer: log level, freshness, key hints, controls, and any error.
fn render_debug_status(frame: &mut Frame, area: Rect, theme: &Theme, controller: &DebugController) {
    let mut spans = vec![Span::styled(
        format!("level {}", controller.log_level()),
        Style::new().fg(theme.dim),
    )];
    if controller.is_live() {
        spans.push(Span::styled(
            " · ● LIVE",
            Style::new().fg(theme.ok).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            " · f stop · v level",
            Style::new().fg(theme.dim),
        ));
    } else {
        if let Some(last) = controller.last_ok() {
            spans.push(Span::styled(
                format!(" · polled {}", ago(last)),
                Style::new().fg(theme.dim),
            ));
        }
        if controller.is_refreshing() {
            spans.push(Span::styled(" · refreshing…", Style::new().fg(theme.warn)));
        } else {
            spans.push(Span::styled(
                " · auto every 2s · r refresh · f live · v level",
                Style::new().fg(theme.dim),
            ));
        }
    }
    if controller.controls_allowed() {
        spans.push(Span::styled(
            " · controls allowed",
            Style::new().fg(theme.warn),
        ));
    }
    if let Some((error, _)) = controller.last_error() {
        spans.push(Span::styled(
            format!(" · last error: {error}"),
            Style::new().fg(theme.error),
        ));
    }
    if let Some((error, _)) = controller.live_error() {
        spans.push(Span::styled(
            format!(" · live: {error}"),
            Style::new().fg(theme.error),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// A bordered section block titled `title`.
fn section_block(theme: &Theme, title: String) -> Block<'static> {
    Block::bordered()
        .border_style(Style::new().fg(theme.dim))
        .title(title)
}

/// Convert a series of `f64` values into `(index, value * scale)` chart points.
fn series_points(values: &[f64], scale: f64) -> Vec<(f64, f64)> {
    values
        .iter()
        .enumerate()
        .map(|(index, &value)| {
            let x = f64::from(u16::try_from(index).unwrap_or(u16::MAX));
            (x, value * scale)
        })
        .collect()
}

/// The x-axis upper bound for a series of `len` points (at least `1.0`).
fn axis_max(len: usize) -> f64 {
    f64::from(u16::try_from(len.saturating_sub(1)).unwrap_or(u16::MAX)).max(1.0)
}

/// Human-readable byte size from an `f64` count (e.g. `1.5 GiB`).
fn format_bytes_f64(bytes: f64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if !bytes.is_finite() || bytes < 0.0 {
        return "—".to_owned();
    }
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Format a `0.0..=1.0` ratio as a whole percentage.
fn format_ratio(ratio: f64) -> String {
    format!("{:.0}%", ratio.clamp(0.0, 1.0) * 100.0)
}

/// The ratio of `part` to `whole` as a clamped `f64`, guarding against zero.
fn ratio_of(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    let scaled = part.saturating_mul(1000) / whole;
    f64::from(u16::try_from(scaled.min(1000)).unwrap_or(1000)) / 1000.0
}

/// A fixed-width 10-cell bar representing a `0.0..=1.0` ratio.
fn percent_bar(ratio: f64) -> String {
    let clamped = ratio.clamp(0.0, 1.0);
    (0..10)
        .map(|cell| {
            if clamped > f64::from(cell) / 10.0 {
                '█'
            } else {
                '░'
            }
        })
        .collect()
}

/// A single Unicode bar block representing a `0.0..=1.0` ratio.
fn ratio_block(ratio: f64) -> char {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let clamped = ratio.clamp(0.0, 1.0);
    let mut chosen = BARS[0];
    for (slot, bar) in BARS.iter().enumerate() {
        let lower = f64::from(u8::try_from(slot).unwrap_or(0)) / 8.0;
        if clamped >= lower {
            chosen = *bar;
        }
    }
    chosen
}

/// Human-readable byte size with one decimal (e.g. `1.5 GiB`).
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut whole = bytes;
    let mut frac_tenths = 0;
    let mut unit = 0;
    while whole >= 1024 && unit < UNITS.len() - 1 {
        frac_tenths = (whole % 1024) * 10 / 1024;
        whole /= 1024;
        unit += 1;
    }
    if unit == 0 {
        format!("{whole} {}", UNITS[unit])
    } else {
        format!("{whole}.{frac_tenths} {}", UNITS[unit])
    }
}

/// Format an uptime in seconds as `Xd Yh Zm` (omitting leading zero units).
fn format_uptime(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "—".to_owned();
    }
    let total = std::time::Duration::from_secs_f64(seconds).as_secs();
    let days = total / 86_400;
    let hours = (total % 86_400) / 3_600;
    let minutes = (total % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

/// The title header line for a screen body.
fn header(theme: &Theme, title: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        title,
        Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
    ))
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
    use super::{
        Screen, Theme, event_detail_lines, event_type_color, format_bytes, format_millis_f64,
        format_uptime, percent, percent_bar, push_payload_lines, ratio_block, ratio_of, sparkline,
    };
    use tbo_core::domain::BoothEventType;

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

    #[test]
    fn format_millis_never_carries_to_sixty_seconds() {
        // 119_500ms rounds to 120s, which must render as "2m 00s", not "1m 60s".
        assert_eq!(format_millis_f64(119_500.0), "2m 00s");
        assert_eq!(format_millis_f64(65_000.0), "1m 05s");
        assert_eq!(format_millis_f64(4_200.0), "4.2s");
        assert_eq!(format_millis_f64(-1.0), "—");
        assert_eq!(format_millis_f64(f64::NAN), "—");
    }

    #[test]
    fn percent_handles_zero_denominator() {
        assert_eq!(percent(0, 0), "0%");
        assert_eq!(percent(3, 4), "75%");
        assert_eq!(percent(2, 2), "100%");
    }

    #[test]
    fn sparkline_scales_and_handles_empty_max() {
        assert_eq!(sparkline(&[0, 0, 0]), "▁▁▁");
        let line = sparkline(&[0, 4, 8]);
        assert_eq!(line.chars().count(), 3);
        assert!(line.starts_with('▁'));
        assert!(line.ends_with('█'));
    }

    #[test]
    fn event_type_color_flags_errors_and_calls() {
        let theme = Theme::default();
        assert_eq!(event_type_color(&theme, BoothEventType::Error), theme.error);
        assert_eq!(
            event_type_color(&theme, BoothEventType::UploadFailed),
            theme.error
        );
        assert_eq!(
            event_type_color(&theme, BoothEventType::CallStarted),
            theme.ok
        );
        assert_eq!(event_type_color(&theme, BoothEventType::Log), theme.fg);
    }

    #[test]
    fn event_detail_lines_handle_missing_selection() {
        let theme = Theme::default();
        assert_eq!(event_detail_lines(&theme, None).len(), 1);
    }

    #[test]
    fn payload_lines_skip_null_and_render_objects() {
        let theme = Theme::default();
        let mut lines = Vec::new();
        push_payload_lines(&mut lines, &theme, &serde_json::Value::Null);
        assert!(lines.is_empty());

        push_payload_lines(
            &mut lines,
            &theme,
            &serde_json::json!({ "digit": "5", "outcome": "answered" }),
        );
        // A blank spacer, the "Payload" subheader, and the pretty-printed body.
        assert!(lines.len() > 2);
    }

    #[test]
    fn format_bytes_uses_binary_units_with_one_decimal() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GiB");
    }

    #[test]
    fn format_uptime_omits_leading_zero_units() {
        assert_eq!(format_uptime(90.0), "1m");
        assert_eq!(format_uptime(3_660.0), "1h 1m");
        assert_eq!(format_uptime(90_061.0), "1d 1h 1m");
        assert_eq!(format_uptime(-1.0), "—");
        assert_eq!(format_uptime(f64::NAN), "—");
    }

    #[test]
    fn ratio_of_guards_zero_and_clamps() {
        assert!((ratio_of(0, 0) - 0.0).abs() < f64::EPSILON);
        assert!((ratio_of(1, 2) - 0.5).abs() < 0.01);
        assert!(ratio_of(5, 4) <= 1.0);
    }

    #[test]
    fn bars_have_expected_widths() {
        assert_eq!(percent_bar(0.0).chars().count(), 10);
        assert_eq!(percent_bar(1.0), "██████████");
        assert_eq!(ratio_block(0.0), '▁');
        assert_eq!(ratio_block(1.0), '█');
    }
}
