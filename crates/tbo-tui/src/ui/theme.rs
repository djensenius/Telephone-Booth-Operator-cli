//! Color themes.
//!
//! Several palettes are available and selectable at runtime from the Settings
//! screen (and persisted to the config file); the default is the Bell-Canada
//! soft-red palette.

use ratatui::style::Color;

/// The theme names, in the order the Settings screen cycles through them. The
/// first entry is the default.
pub const NAMES: [&str; 3] = ["bell-canada", "bell-canada-blue", "high-contrast"];

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
    /// The default Bell-Canada palette: a soft-red accent on a deep, slightly
    /// warm background.
    #[must_use]
    pub fn bell_canada() -> Self {
        Self {
            accent: Color::Rgb(214, 73, 73),
            fg: Color::Rgb(236, 228, 228),
            dim: Color::Rgb(150, 120, 122),
            bg: Color::Rgb(58, 18, 20),
            ok: Color::Rgb(64, 192, 120),
            warn: Color::Rgb(230, 170, 60),
            error: Color::Rgb(218, 41, 28),
        }
    }

    /// The original Bell-Canada palette: a deep blue with a red error accent.
    #[must_use]
    pub fn bell_canada_blue() -> Self {
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

    /// A high-contrast palette using the terminal's bright ANSI colors, for
    /// accessibility and low-color terminals.
    #[must_use]
    pub fn high_contrast() -> Self {
        Self {
            accent: Color::LightYellow,
            fg: Color::White,
            dim: Color::Gray,
            bg: Color::Black,
            ok: Color::LightGreen,
            warn: Color::LightYellow,
            error: Color::LightRed,
        }
    }

    /// Resolve a theme by its configured name, falling back to the default for
    /// any unknown name.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "bell-canada-blue" => Self::bell_canada_blue(),
            "high-contrast" => Self::high_contrast(),
            // "bell-canada" and any unknown name use the default palette.
            _ => Self::bell_canada(),
        }
    }

    /// The theme name following `current` in [`NAMES`], wrapping around. An
    /// unknown `current` resolves to the first name.
    #[must_use]
    pub fn next_name(current: &str) -> &'static str {
        NAMES
            .iter()
            .position(|name| *name == current)
            .map_or(NAMES[0], |index| NAMES[(index + 1) % NAMES.len()])
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::bell_canada()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_name_resolves_known_palettes() {
        assert_eq!(
            Theme::from_name("bell-canada").accent,
            Color::Rgb(214, 73, 73)
        );
        assert_eq!(
            Theme::from_name("bell-canada-blue").accent,
            Color::Rgb(0, 87, 184)
        );
        assert_eq!(Theme::from_name("high-contrast").accent, Color::LightYellow);
    }

    #[test]
    fn from_name_falls_back_to_default_for_unknown() {
        assert_eq!(
            Theme::from_name("does-not-exist").accent,
            Theme::bell_canada().accent
        );
    }

    #[test]
    fn next_name_cycles_through_all_then_wraps() {
        assert_eq!(Theme::next_name("bell-canada"), "bell-canada-blue");
        assert_eq!(Theme::next_name("bell-canada-blue"), "high-contrast");
        assert_eq!(Theme::next_name("high-contrast"), "bell-canada");
    }

    #[test]
    fn next_name_resolves_unknown_to_first() {
        assert_eq!(Theme::next_name("nonsense"), NAMES[0]);
    }
}
