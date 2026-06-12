//! File-based logging.
//!
//! A TUI writes to the alternate screen, so log records must never go to
//! stdout/stderr while the UI is running. Instead we append to a daily-rotated
//! file under the platform state directory (falling back to the cache
//! directory). Logging is best-effort: failures are reported once and the app
//! continues without a log file.

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Initialize file logging, returning a guard that must be kept alive for the
/// lifetime of the process so buffered records are flushed on exit.
///
/// Returns `None` when no writable log directory can be resolved or when a
/// subscriber is already installed.
#[must_use]
pub fn init() -> Option<WorkerGuard> {
    let dirs = directories::ProjectDirs::from("io", "telephonebooth", "tb-operator")?;
    let dir = dirs
        .state_dir()
        .unwrap_or_else(|| dirs.cache_dir())
        .to_path_buf();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!(
            "warning: could not create log directory {}: {err}",
            dir.display()
        );
        return None;
    }

    let appender = tracing_appender::rolling::daily(&dir, "tb-operator.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    if let Err(err) = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
    {
        eprintln!("warning: could not initialize logging: {err}");
        return None;
    }

    Some(guard)
}
