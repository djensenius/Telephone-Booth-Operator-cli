//! The authenticated operator's identity (`GET /v1/auth/me`).

use serde::{Deserialize, Serialize};

/// Identity of the currently authenticated operator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorMe {
    /// Stable subject id.
    pub id: String,
    /// Email address.
    pub email: String,
    /// Display name.
    pub name: String,
    /// Group memberships.
    pub groups: Vec<String>,
    /// Avatar URL, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// Identity-provider name.
    pub provider_name: String,
}
