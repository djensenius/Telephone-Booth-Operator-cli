//! `tb-operator` — terminal operator console for the Telephone-Booth
//! installation.
//!
//! This binary hosts the `ratatui` user interface, input handling, and audio
//! playback. The application shell and screens are added in later phases; for
//! now it reports its version.

fn main() {
    println!("tb-operator {}", env!("CARGO_PKG_VERSION"));
}
