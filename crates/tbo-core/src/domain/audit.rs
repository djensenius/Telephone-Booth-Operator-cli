//! Audit trail: who took each write action, from where, and when.
//!
//! Mirrors the operator API's `/v1/audit-logs` surface. Every mutating request
//! against the operator API produces one entry, including the ones that were
//! rejected — `status_code` is what tells them apart.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

/// What kind of principal took the action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuditActorType {
    /// A signed-in operator.
    Operator,
    /// A phone-side or worker API token.
    ApiToken,
    /// No credentials were presented.
    Anonymous,
    /// The API itself (background jobs).
    System,
}

impl AuditActorType {
    /// The wire value used in `actorType` query filters.
    #[must_use]
    pub const fn as_query(self) -> &'static str {
        match self {
            AuditActorType::Operator => "operator",
            AuditActorType::ApiToken => "apiToken",
            AuditActorType::Anonymous => "anonymous",
            AuditActorType::System => "system",
        }
    }

    /// Short human label for lists.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            AuditActorType::Operator => "operator",
            AuditActorType::ApiToken => "token",
            AuditActorType::Anonymous => "anon",
            AuditActorType::System => "system",
        }
    }
}

/// A single recorded write action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogEntry {
    /// Entry id.
    pub id: String,
    /// Stable action name, e.g. `message.approve`.
    pub action: String,
    /// What was acted on, when the handler names a target.
    #[serde(default)]
    pub target_type: Option<String>,
    /// The id of the thing acted on.
    #[serde(default)]
    pub target_id: Option<String>,
    /// The kind of principal that acted.
    pub actor_type: AuditActorType,
    /// The operator that acted, if any and still present.
    #[serde(default)]
    pub actor_user_id: Option<String>,
    /// The API token that acted, if any and still present.
    #[serde(default)]
    pub actor_token_id: Option<String>,
    /// Denormalized actor name captured at the time of the action.
    pub actor_label: String,
    /// Client address the action came from.
    #[serde(default)]
    pub ip: Option<String>,
    /// Request user agent, truncated by the API.
    #[serde(default)]
    pub user_agent: Option<String>,
    /// HTTP method.
    pub method: String,
    /// Request route.
    pub path: String,
    /// Response status, so the outcome is part of the record.
    pub status_code: u16,
    /// Small action-specific detail; never carries secrets or transcript text.
    #[serde(default)]
    pub metadata: Option<Value>,
    /// When the action happened.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl AuditLogEntry {
    /// Whether the action succeeded.
    #[must_use]
    pub const fn succeeded(&self) -> bool {
        self.status_code < 300
    }

    /// Whether the action was refused for lack of authority.
    #[must_use]
    pub const fn was_denied(&self) -> bool {
        self.status_code == 401 || self.status_code == 403
    }
}

/// A page of audit entries, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogPage {
    /// Entries, newest first.
    pub items: Vec<AuditLogEntry>,
    /// Opaque cursor for the next (older) page, or `None` at the end.
    pub next_cursor: Option<String>,
}

/// Filter for [`AuditLogPage`] queries.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    /// Action prefix: `message.` matches the whole family, `message.approve`
    /// just the one action.
    pub action: Option<String>,
    /// Restrict to one kind of principal.
    pub actor_type: Option<AuditActorType>,
    /// Restrict to a single operator.
    pub actor_user_id: Option<String>,
    /// Opaque pagination cursor from a previous page.
    pub cursor: Option<String>,
    /// Maximum number of entries to return.
    pub limit: Option<u32>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn decodes_an_entry_from_the_wire() {
        let json = r#"{
            "id": "a1",
            "action": "message.approve",
            "targetType": "message",
            "targetId": "m1",
            "actorType": "operator",
            "actorUserId": "u1",
            "actorTokenId": null,
            "actorLabel": "operator@example.com",
            "ip": "203.0.113.7",
            "userAgent": "tb-operator/0.6.0",
            "method": "POST",
            "path": "/v1/messages/m1/decision",
            "statusCode": 200,
            "metadata": {"decision": "approve"},
            "createdAt": "2026-07-20T12:00:00Z"
        }"#;
        let entry: AuditLogEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.actor_type, AuditActorType::Operator);
        assert_eq!(entry.actor_label, "operator@example.com");
        assert_eq!(entry.ip.as_deref(), Some("203.0.113.7"));
        assert!(entry.succeeded());
        assert!(!entry.was_denied());
    }

    #[test]
    fn a_denied_write_is_still_an_entry() {
        let json = r#"{
            "id": "a2",
            "action": "message.approve",
            "actorType": "anonymous",
            "actorLabel": "anonymous",
            "method": "POST",
            "path": "/v1/messages/m1/decision",
            "statusCode": 401,
            "createdAt": "2026-07-20T12:00:00Z"
        }"#;
        let entry: AuditLogEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.actor_type, AuditActorType::Anonymous);
        assert!(entry.target_id.is_none());
        assert!(entry.was_denied());
        assert!(!entry.succeeded());
    }
}
