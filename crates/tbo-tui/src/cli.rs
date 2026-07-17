//! Command-line interface definition.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Terminal operator console for the Telephone-Booth installation.
#[derive(Debug, Parser)]
#[command(name = "tb-operator", version, about, long_about = None)]
pub struct Cli {
    /// Use an alternate configuration file instead of the default location.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Run the interactive setup flow before launching, even if a configuration
    /// already exists.
    #[arg(long)]
    pub setup: bool,

    /// A non-interactive subcommand to run instead of launching the TUI.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Non-interactive subcommands. When present, the tool runs the command and
/// exits instead of starting the terminal UI.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Admin-only full data export/import (database plus all audio).
    #[command(subcommand)]
    Data(DataCommand),
}

/// Admin data backup operations against the operator API.
#[derive(Debug, Subcommand)]
pub enum DataCommand {
    /// Download a complete data archive to a file (requires an admin session).
    Export {
        /// Path to write the `.tar` archive to.
        #[arg(long, short, value_name = "FILE")]
        output: PathBuf,
    },
    /// Restore a complete data archive from a file (requires an admin session).
    Import {
        /// Path to the `.tar` archive to import.
        #[arg(long, short, value_name = "FILE")]
        input: PathBuf,
    },
}
