//! Authentik authentication for the `tb-operator` console.
//!
//! Implements the OAuth 2.0 device authorization grant against Authentik:
//! endpoint discovery, device-code polling, access-token refresh, and refresh
//! token storage in the OS keychain. The flow is added in a later phase.
