//! Color theme.
//!
//! Phase 0 ships a single Bell-Canada-inspired palette; richer theming and a
//! settings toggle arrive in a later phase.

use ratatui::style::Color;

/// A resolved set of UI colors.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Primary accent (selected tab, headings).
    pub accent: Color,
    /// Default foreground text.
    pub fg: Color,
    /// De-emphasized text (hints, borders).
    pub dim: Color,
    /// Background for highlighted chrome (status bar pill).
    pub bg: Color,
    /// Success / healthy indicator.
    pub ok: Color,
    /// Warning indicator.
    pub warn: Color,
    /// Error indicator.
    pub error: Color,
}

impl Theme {
    /// The Bell-Canada-inspired palette (deep blue + red accent).
    #[must_use]
    pub fn bell_canada() -> Self {
        Self {
            accent: Color::Rgb(0, 87, 184),
            fg: Color::Rgb(229, 233, 240),
            dim: Color::Rgb(122, 132, 148),
            bg: Color::Rgb(0, 30, 64),
            ok: Color::Rgb(64, 192, 120),
            warn: Color::Rgb(230, 170, 60),
            error: Color::Rgb(218, 41, 28),
        }
    }

    /// Resolve a theme by its configured name, falling back to the default.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "bell-canada" => Self::bell_canada(),
            // Additional palettes can be added here in a later phase; unknown
            // names fall back to the default.
            _ => Self::bell_canada(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::bell_canada()
    }
}
