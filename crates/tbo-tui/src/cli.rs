//! Command-line interface definition.

use std::path::PathBuf;

use clap::Parser;

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
}
