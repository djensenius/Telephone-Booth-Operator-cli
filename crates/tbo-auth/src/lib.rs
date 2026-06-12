//! Authentik authentication for the `tb-operator` console.
//!
//! Implements the OAuth 2.0 device authorization grant (RFC 8628) against
//! Authentik: endpoint resolution ([`endpoints`]), the device-code flow and
//! access-token refresh ([`client`]), the wire payloads ([`tokens`]), and an
//! HTTP transport abstraction ([`transport`]) that keeps the flow testable
//! without a network.
//!
//! The persisted session ([`session`]) is kept in the OS keychain via a
//! [`store::TokenStore`], and [`manager::SessionManager`] ties the client and
//! store together to keep a valid access token available (proactive refresh,
//! refresh-token rotation, sign-out on rejection).
//!
//! The device-code grant is used because the console runs in a terminal with
//! no embedded browser: the user is shown a short code and a verification URL
//! to open on a phone or computer, and the client polls until the user
//! approves.

pub mod client;
pub mod endpoints;
pub mod error;
pub mod manager;
pub mod session;
pub mod store;
pub mod tokens;
pub mod transport;

pub use client::{AuthClient, DeviceTokenOutcome};
pub use endpoints::Endpoints;
pub use error::{AuthError, Result};
pub use manager::SessionManager;
pub use session::StoredSession;
pub use store::{InMemoryTokenStore, KeyringTokenStore, TokenStore};
pub use tokens::{DeviceAuthorization, OidcTokens};
pub use transport::{HttpResponse, HttpTransport, ReqwestTransport};
