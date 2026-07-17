//! Admin-only data export/import contracts (`/v1/admin/data`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Summary returned after importing a data archive
/// (`POST /v1/admin/data/import`).
///
/// Reports how many rows were restored per table and how many audio blobs were
/// uploaded versus skipped (already present, content-addressed by SHA-256).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataImportSummary {
    /// Number of rows restored, keyed by table name.
    pub rows: BTreeMap<String, u64>,
    /// Count of audio blobs newly uploaded during the import.
    pub blobs_uploaded: u64,
    /// Count of audio blobs skipped because they already existed.
    pub blobs_skipped: u64,
}
