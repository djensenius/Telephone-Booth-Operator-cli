//! Headless admin data commands: full export and import of operator data.
//!
//! These run outside the TUI (see [`crate::cli::Command`]) so an administrator
//! can back up or restore an instance from a script. Both call the admin-only
//! `/v1/admin/data` endpoints with the stored operator session token; a
//! non-admin session is rejected by the server with an authorization error.

use std::path::Path;

use anyhow::{Context, Result, bail};
use tbo_core::config::Config;
use tbo_operator_client::OperatorClient;

use crate::data::{SessionTokenProvider, build_shared_session};

/// Download a full data-export archive to `output`.
///
/// # Errors
/// Returns an error when the session/client cannot be built, the export request
/// fails (including a non-admin `403`), or the archive cannot be written.
pub async fn export(config: &Config, output: &Path) -> Result<()> {
    let client = build_client(config)?;
    eprintln!("Requesting data export from {}…", config.operator.base_url);
    let bytes = client
        .export_data()
        .await
        .context("data export request failed")?;
    tokio::fs::write(output, &bytes)
        .await
        .with_context(|| format!("could not write export to {}", output.display()))?;
    println!("Wrote {} bytes to {}.", bytes.len(), output.display());
    Ok(())
}

/// Restore a previously exported archive read from `input`.
///
/// # Errors
/// Returns an error when the file is missing or empty, the session/client
/// cannot be built, or the import request fails (including a non-admin `403` or
/// a malformed archive `400`).
pub async fn import(config: &Config, input: &Path) -> Result<()> {
    let bytes = tokio::fs::read(input)
        .await
        .with_context(|| format!("could not read archive {}", input.display()))?;
    if bytes.is_empty() {
        bail!("archive {} is empty", input.display());
    }
    let client = build_client(config)?;
    eprintln!(
        "Importing {} bytes into {}…",
        bytes.len(),
        config.operator.base_url
    );
    let summary = client
        .import_data(bytes)
        .await
        .context("data import request failed")?;
    println!("Import complete.");
    println!(
        "  audio blobs: {} uploaded, {} skipped (already present)",
        summary.blobs_uploaded, summary.blobs_skipped
    );
    if summary.rows.is_empty() {
        println!("  rows restored: none");
    } else {
        println!("  rows restored:");
        for (table, count) in &summary.rows {
            println!("    {table}: {count}");
        }
    }
    Ok(())
}

/// Build an operator client backed by the stored session token.
fn build_client(
    config: &Config,
) -> Result<OperatorClient<tbo_operator_client::ReqwestTransport, SessionTokenProvider>> {
    let (session, warning) =
        build_shared_session(config).context("could not open session store")?;
    if let Some(warning) = warning {
        eprintln!("warning: {warning}");
    }
    let provider = SessionTokenProvider::new(session);
    let client = OperatorClient::new(config.operator.base_url.clone(), provider)
        .context("could not build the operator client")?;
    Ok(client)
}
