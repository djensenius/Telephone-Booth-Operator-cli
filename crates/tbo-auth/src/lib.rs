//! Authentik authentication for the `tb-operator` console.
//!
//! Implements the OAuth 2.0 device authorization grant (RFC 8628) against
//! Authentik: endpoint resolution ([`endpoints`]), the device-code flow and
//! access-token refresh ([`client`]), the wire payloads ([`tokens`]), and an
//! HTTP transport abstraction ([`transport`]) that keeps the flow testable
//! without a network.
//!
//! The device-code grant is used because the console runs in a terminal with
//! no embedded browser: the user is shown a short code and a verification URL
//! to open on a phone or computer, and the client polls until the user
//! approves. Refresh-token storage in the OS keychain is added in a later
//! phase.

pub mod client;
pub mod endpoints;
pub mod error;
pub mod tokens;
pub mod transport;

pub use client::{AuthClient, DeviceTokenOutcome};
pub use endpoints::Endpoints;
pub use error::{AuthError, Result};
pub use tokens::{DeviceAuthorization, OidcTokens};
pub use transport::{HttpResponse, HttpTransport, ReqwestTransport};
