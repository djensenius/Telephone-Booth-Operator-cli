//! `tb-operator` — terminal operator console for the Telephone-Booth
//! installation.
//!
//! This binary hosts the `ratatui` user interface, input handling, and (in
//! later phases) the operator/booth clients and audio playback. Module layout:
//!
//! - [`cli`]: command-line argument parsing.
//! - [`logging`]: file-based tracing setup.
//! - [`onboarding`]: interactive first-run setup.
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
mod datacmd;
mod event;
mod logging;
mod onboarding;
mod tui;
mod ui;

use std::io::IsTerminal;

use anyhow::Result;
use clap::Parser;
use tbo_core::Secrets;
use tbo_core::config::Config;

use crate::app::App;
use crate::cli::{Cli, Command, DataCommand};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = match cli.config.as_deref() {
        Some(path) => path.to_path_buf(),
        None => Config::default_path()?,
    };

    // Run the interactive setup when explicitly requested, or on first launch
    // (no config yet) when attached to a terminal that can answer prompts.
    // Non-interactive subcommands never trigger onboarding.
    let first_run = !config_path.exists();
    let is_interactive_terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if cli.command.is_none() && (cli.setup || (first_run && is_interactive_terminal)) {
        onboarding::run(&config_path).await?;
    }

    let mut config = Config::load_from(&config_path)?;
    config.merge_secrets(&Secrets::load().unwrap_or_default());

    let _log_guard = logging::init();

    // Non-interactive subcommands run to completion and exit without starting
    // the terminal UI.
    if let Some(command) = cli.command {
        return run_command(command, &config).await;
    }

    // Build the app (and its auth/session state) before taking over the
    // terminal, so a setup failure surfaces as a normal error.
    let app = App::new(config, config_path)?;

    let mut terminal = tui::init()?;
    let result = app.run(&mut terminal).await;
    tui::restore()?;
    result
}

/// Dispatch a non-interactive subcommand.
async fn run_command(command: Command, config: &Config) -> Result<()> {
    match command {
        Command::Data(DataCommand::Export { output }) => datacmd::export(config, &output).await,
        Command::Data(DataCommand::Import { input }) => datacmd::import(config, &input).await,
    }
}
