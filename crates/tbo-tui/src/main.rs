//! `tb-operator` — terminal operator console for the Telephone-Booth
//! installation.
//!
//! This binary hosts the `ratatui` user interface, input handling, and (in
//! later phases) the operator/booth clients and audio playback. Module layout:
//!
//! - [`cli`]: command-line argument parsing.
//! - [`logging`]: file-based tracing setup.
//! - [`tui`]: terminal init/restore and the panic hook.
//! - [`event`]: the async input/tick event source.
//! - [`auth`]: interactive Authentik device-code login.
//! - [`data`]: operator API data layer (token provider + screen controllers).
//! - [`app`]: application state and the main loop.
//! - [`ui`]: rendering (tab bar, screens, status bar, toasts, theme).

mod app;
mod auth;
mod cli;
mod data;
mod event;
mod logging;
mod tui;
mod ui;

use anyhow::Result;
use clap::Parser;
use tbo_core::config::Config;

use crate::app::App;
use crate::cli::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = match cli.config.as_deref() {
        Some(path) => Config::load_from(path)?,
        None => Config::load()?,
    };

    let _log_guard = logging::init();

    // Build the app (and its auth/session state) before taking over the
    // terminal, so a setup failure surfaces as a normal error.
    let app = App::new(config)?;

    let mut terminal = tui::init()?;
    let result = app.run(&mut terminal).await;
    tui::restore()?;
    result
}
