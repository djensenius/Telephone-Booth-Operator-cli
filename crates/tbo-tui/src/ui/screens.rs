//! Screen routing and Phase-0 placeholder rendering.
//!
//! Each [`Screen`] becomes a fully interactive view in later phases; for now
//! every screen renders a titled placeholder describing what it will show, so
//! the navigation chrome can be exercised end to end.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use std::time::Instant;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use tbo_core::domain::{
    BoothEventRecord, BoothEventType, BoothState, BoothStatus, CallOutcome, CallSession,
    CallSessionDetail, Message, MessageStatus, Moderation, ModerationRecommendation, Question,
    QuestionStatus, RuntimeMode, StatsOverview, StatsWindow, Transcription, TranscriptionStatus,
};

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
    match app.screen() {
        Screen::Messages => render_messages(app, frame, area),
        Screen::Questions => render_questions(app, frame, area),
        Screen::Sessions => render_sessions(app, frame, area),
        Screen::Stats => {
            render_paragraph(frame, area, theme, "Statistics", stats_lines(app, theme));
        }
        Screen::Status => render_paragraph(frame, area, theme, "Status", status_lines(app, theme)),
        Screen::Settings => {
            render_paragraph(frame, area, theme, "Settings", settings_lines(app, theme));
        }
        screen => {
            render_paragraph(
                frame,
                area,
                theme,
                screen.title(),
                placeholder_lines(screen, theme),
            );
        }
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
        lines.push(kv_line(theme, "Avg length:", format_millis_f64(avg)));
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
    use super::{Screen, format_millis_f64, percent, sparkline};

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
}
