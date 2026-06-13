//! Color themes.
//!
//! Several palettes are available and selectable at runtime from the Settings
//! screen (and persisted to the config file); the default is Catppuccin Mocha.
//! The four [Catppuccin](https://catppuccin.com/) flavours and the original
//! Bell-Canada / high-contrast palettes are all selectable.

use ratatui::style::Color;

/// The theme names, in the order the Settings screen cycles through them. The
/// first entry is the default.
pub const NAMES: [&str; 7] = [
    "catppuccin-mocha",
    "catppuccin-macchiato",
    "catppuccin-frappe",
    "catppuccin-latte",
    "bell-canada",
    "bell-canada-blue",
    "high-contrast",
];

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
    /// Catppuccin Mocha — the default dark palette (Mauve accent on a deep
    /// blue-charcoal base). See <https://catppuccin.com/palette>.
    #[must_use]
    pub fn catppuccin_mocha() -> Self {
        Self {
            accent: Color::Rgb(203, 166, 247), // Mauve
            fg: Color::Rgb(205, 214, 244),     // Text
            dim: Color::Rgb(127, 132, 156),    // Overlay1
            bg: Color::Rgb(69, 71, 90),        // Surface1
            ok: Color::Rgb(166, 227, 161),     // Green
            warn: Color::Rgb(249, 226, 175),   // Yellow
            error: Color::Rgb(243, 139, 168),  // Red
        }
    }

    /// Catppuccin Macchiato — a slightly warmer, medium-dark flavour.
    #[must_use]
    pub fn catppuccin_macchiato() -> Self {
        Self {
            accent: Color::Rgb(198, 160, 246), // Mauve
            fg: Color::Rgb(202, 211, 245),     // Text
            dim: Color::Rgb(128, 135, 162),    // Overlay1
            bg: Color::Rgb(73, 77, 100),       // Surface1
            ok: Color::Rgb(166, 218, 149),     // Green
            warn: Color::Rgb(238, 212, 159),   // Yellow
            error: Color::Rgb(237, 135, 150),  // Red
        }
    }

    /// Catppuccin Frappé — a soft, low-contrast dark flavour.
    #[must_use]
    pub fn catppuccin_frappe() -> Self {
        Self {
            accent: Color::Rgb(202, 158, 230), // Mauve
            fg: Color::Rgb(198, 208, 245),     // Text
            dim: Color::Rgb(131, 139, 167),    // Overlay1
            bg: Color::Rgb(81, 87, 109),       // Surface1
            ok: Color::Rgb(166, 209, 137),     // Green
            warn: Color::Rgb(229, 200, 144),   // Yellow
            error: Color::Rgb(231, 130, 132),  // Red
        }
    }

    /// Catppuccin Latte — the light flavour, for bright terminals.
    #[must_use]
    pub fn catppuccin_latte() -> Self {
        Self {
            accent: Color::Rgb(136, 57, 239), // Mauve
            fg: Color::Rgb(76, 79, 105),      // Text
            dim: Color::Rgb(140, 143, 161),   // Overlay1
            bg: Color::Rgb(188, 192, 204),    // Surface1
            ok: Color::Rgb(64, 160, 43),      // Green
            warn: Color::Rgb(223, 142, 29),   // Yellow
            error: Color::Rgb(210, 15, 57),   // Red
        }
    }

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

    /// Resolve a theme by its configured name, falling back to the default
    /// (Catppuccin Mocha) for any unknown name.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "catppuccin-macchiato" => Self::catppuccin_macchiato(),
            "catppuccin-frappe" => Self::catppuccin_frappe(),
            "catppuccin-latte" => Self::catppuccin_latte(),
            "bell-canada" => Self::bell_canada(),
            "bell-canada-blue" => Self::bell_canada_blue(),
            "high-contrast" => Self::high_contrast(),
            // "catppuccin-mocha" and any unknown name use the default palette.
            _ => Self::catppuccin_mocha(),
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
        Self::catppuccin_mocha()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_name_resolves_known_palettes() {
        assert_eq!(
            Theme::from_name("catppuccin-mocha").accent,
            Color::Rgb(203, 166, 247)
        );
        assert_eq!(
            Theme::from_name("catppuccin-latte").accent,
            Color::Rgb(136, 57, 239)
        );
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
            Theme::catppuccin_mocha().accent
        );
    }

    #[test]
    fn next_name_cycles_through_all_then_wraps() {
        assert_eq!(Theme::next_name("catppuccin-mocha"), "catppuccin-macchiato");
        assert_eq!(Theme::next_name("catppuccin-latte"), "bell-canada");
        assert_eq!(Theme::next_name("high-contrast"), "catppuccin-mocha");
    }

    #[test]
    fn next_name_resolves_unknown_to_first() {
        assert_eq!(Theme::next_name("nonsense"), NAMES[0]);
    }

    #[test]
    fn every_name_resolves_and_cycles_back_to_start() {
        // Each registered name must round-trip through the cycle exactly once.
        let mut seen = std::collections::HashSet::new();
        let mut current = NAMES[0];
        for _ in 0..NAMES.len() {
            assert!(seen.insert(current), "duplicate in cycle: {current}");
            current = Theme::next_name(current);
        }
        assert_eq!(current, NAMES[0], "cycle must wrap to the first name");
    }
}
