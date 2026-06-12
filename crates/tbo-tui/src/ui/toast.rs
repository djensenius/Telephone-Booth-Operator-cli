//! Transient on-screen notifications.

use std::time::{Duration, Instant};

/// How long a toast remains visible.
const TTL: Duration = Duration::from_secs(4);

/// Severity of a toast, controlling its color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Informational.
    Info,
    /// Warning.
    Warn,
    /// Error.
    Error,
}

/// A single notification with an expiry time.
#[derive(Debug, Clone)]
pub struct Toast {
    /// Severity level.
    pub level: Level,
    /// Message text.
    pub text: String,
    /// When the toast should disappear.
    expires: Instant,
}

/// A bounded queue of active toasts.
#[derive(Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

impl Toasts {
    /// Push a toast with the given level and text.
    pub fn push(&mut self, level: Level, text: impl Into<String>) {
        self.items.push(Toast {
            level,
            text: text.into(),
            expires: Instant::now() + TTL,
        });
    }

    /// Push an informational toast.
    pub fn info(&mut self, text: impl Into<String>) {
        self.push(Level::Info, text);
    }

    /// Push a warning toast.
    pub fn warn(&mut self, text: impl Into<String>) {
        self.push(Level::Warn, text);
    }

    /// Push an error toast.
    pub fn error(&mut self, text: impl Into<String>) {
        self.push(Level::Error, text);
    }

    /// Drop any toasts whose lifetime has elapsed.
    pub fn prune(&mut self) {
        let now = Instant::now();
        self.items.retain(|toast| toast.expires > now);
    }

    /// Whether there are any active toasts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterate over active toasts, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Toast> {
        self.items.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::{Level, Toasts};

    #[test]
    fn push_records_level_and_text() {
        let mut toasts = Toasts::default();
        assert!(toasts.is_empty());
        toasts.info("hello");
        toasts.warn("careful");
        toasts.error("boom");

        let collected: Vec<(Level, &str)> =
            toasts.iter().map(|t| (t.level, t.text.as_str())).collect();
        assert_eq!(
            collected,
            vec![
                (Level::Info, "hello"),
                (Level::Warn, "careful"),
                (Level::Error, "boom"),
            ]
        );
    }

    #[test]
    fn prune_keeps_unexpired() {
        let mut toasts = Toasts::default();
        toasts.info("still here");
        toasts.prune();
        assert!(!toasts.is_empty());
        assert_eq!(toasts.iter().count(), 1);
    }
}
