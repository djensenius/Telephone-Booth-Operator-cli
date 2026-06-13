//! Nerd Font glyphs for chrome (tabs, status bar, toasts, auth, help).
//!
//! All glyphs are [Nerd Font](https://www.nerdfonts.com/) Private-Use-Area
//! codepoints, so they only render with a patched "Nerd Font" installed in the
//! terminal. The set is resolved once from [`UiConfig::nerd_fonts`](tbo_core::config::UiConfig::nerd_fonts):
//! when disabled every glyph accessor returns an empty string (or a plain ASCII
//! marker) so the interface degrades to text-only labels.

use crate::ui::screens::Screen;

/// A resolved glyph set. Cheap to copy; constructed from the `nerd_fonts` flag.
#[derive(Debug, Clone, Copy)]
pub struct Icons {
    enabled: bool,
}

impl Icons {
    /// Build the glyph set, enabling Nerd Font glyphs when `enabled` is true.
    #[must_use]
    pub const fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Whether Nerd Font glyphs are rendered.
    #[must_use]
    pub const fn enabled(self) -> bool {
        self.enabled
    }

    /// Pick the Nerd Font `glyph` when enabled, otherwise the ASCII `fallback`.
    const fn pick(self, glyph: &'static str, fallback: &'static str) -> &'static str {
        if self.enabled { glyph } else { fallback }
    }

    /// The tab glyph for `screen`, already padded with a trailing space, or an
    /// empty string when glyphs are disabled (so tabs render as plain text).
    #[must_use]
    pub const fn tab(self, screen: Screen) -> &'static str {
        if !self.enabled {
            return "";
        }
        match screen {
            Screen::Status => "\u{f0e7} ",       // bolt
            Screen::Messages => "\u{f0e0} ",     // envelope
            Screen::Questions => "\u{f059} ",    // question-circle
            Screen::Events => "\u{f03a} ",       // list
            Screen::Sessions => "\u{f095} ",     // phone
            Screen::Stats => "\u{f080} ",        // bar-chart
            Screen::LiveSystem => "\u{f108} ",   // desktop
            Screen::SystemHealth => "\u{f21e} ", // heartbeat
            Screen::Debug => "\u{f188} ",        // bug
            Screen::Tokens => "\u{f084} ",       // key
            Screen::Settings => "\u{f013} ",     // cog
            Screen::About => "\u{f129} ",        // info
        }
    }

    /// The brand glyph shown in the tab-bar title.
    #[must_use]
    pub const fn brand(self) -> &'static str {
        self.pick("\u{f095} ", "") // phone
    }

    /// The leading "ready" status dot in the status bar.
    #[must_use]
    pub const fn ready(self) -> &'static str {
        self.pick("\u{f111} ", "\u{25cf} ") // filled circle / ●
    }

    /// Glyph for an informational toast (padded), empty when disabled.
    #[must_use]
    pub const fn info(self) -> &'static str {
        self.pick("\u{f05a} ", "") // info-circle
    }

    /// Glyph for a warning toast (padded), empty when disabled.
    #[must_use]
    pub const fn warn(self) -> &'static str {
        self.pick("\u{f071} ", "") // exclamation-triangle
    }

    /// Glyph for an error toast (padded), empty when disabled.
    #[must_use]
    pub const fn error(self) -> &'static str {
        self.pick("\u{f06a} ", "") // exclamation-circle
    }

    /// Glyph for the "signed in" account state (padded), empty when disabled.
    #[must_use]
    pub const fn signed_in(self) -> &'static str {
        self.pick("\u{f007} ", "") // user
    }

    /// Glyph for the "signed out" account state (padded), empty when disabled.
    #[must_use]
    pub const fn signed_out(self) -> &'static str {
        self.pick("\u{f235} ", "") // user-times
    }

    /// Glyph for an in-progress login (padded), empty when disabled.
    #[must_use]
    pub const fn awaiting(self) -> &'static str {
        self.pick("\u{f252} ", "") // hourglass-half
    }

    /// Glyph for the help overlay title (padded), empty when disabled.
    #[must_use]
    pub const fn help(self) -> &'static str {
        self.pick("\u{f059} ", "") // question-circle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_icons_are_plain_or_empty() {
        let icons = Icons::new(false);
        assert!(!icons.enabled());
        assert_eq!(icons.tab(Screen::Status), "");
        assert_eq!(icons.info(), "");
        assert_eq!(icons.signed_in(), "");
        // The ready dot keeps an ASCII fallback so the status bar still reads.
        assert_eq!(icons.ready(), "\u{25cf} ");
    }

    #[test]
    fn enabled_icons_are_glyphs() {
        let icons = Icons::new(true);
        assert!(icons.enabled());
        assert_ne!(icons.tab(Screen::Messages), "");
        assert_ne!(icons.help(), "");
        // Every screen has a distinct, non-empty tab glyph.
        let mut seen = std::collections::HashSet::new();
        for screen in Screen::all() {
            let glyph = icons.tab(*screen);
            assert!(glyph.ends_with(' '), "tab glyph must be padded: {glyph:?}");
            assert!(seen.insert(glyph), "duplicate tab glyph: {glyph:?}");
        }
    }
}
