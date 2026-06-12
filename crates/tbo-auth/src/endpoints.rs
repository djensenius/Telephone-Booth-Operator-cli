//! Authentik OAuth endpoint resolution.
//!
//! Authentik's global OAuth endpoints (`authorize`, `token`, `device`) live at
//! the **parent** path of the per-application issuer and strict-match a
//! trailing slash. For an issuer such as
//! `https://auth.fluxhaus.io/application/o/telephone-booth-operator-mobile`,
//! the endpoints are derived by dropping the final path segment and appending
//! the endpoint name with a trailing slash, e.g.
//! `https://auth.fluxhaus.io/application/o/device/`.
//!
//! This mirrors the proven derivation used by the mobile operator app rather
//! than performing OIDC discovery, so the console works against the current
//! production provider without an extra round-trip.

/// Resolved OAuth endpoint URLs for a given issuer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// Device authorization endpoint (`.../device/`).
    pub device: String,
    /// Token endpoint (`.../token/`).
    pub token: String,
    /// Authorization endpoint (`.../authorize/`).
    pub authorize: String,
}

impl Endpoints {
    /// Derive the endpoints from a per-application issuer base URL.
    #[must_use]
    pub fn derive(issuer_base: &str) -> Self {
        let parent = parent_path(issuer_base);
        Self {
            device: format!("{parent}/device/"),
            token: format!("{parent}/token/"),
            authorize: format!("{parent}/authorize/"),
        }
    }
}

/// Drop the final path segment of `url`, ignoring a single trailing slash.
fn parent_path(url: &str) -> &str {
    let trimmed = url.strip_suffix('/').unwrap_or(url);
    trimmed
        .rfind('/')
        .map_or(trimmed, |index| &trimmed[..index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_authentik_endpoints() {
        let endpoints = Endpoints::derive(
            "https://auth.fluxhaus.io/application/o/telephone-booth-operator-mobile",
        );
        assert_eq!(
            endpoints.device,
            "https://auth.fluxhaus.io/application/o/device/"
        );
        assert_eq!(
            endpoints.token,
            "https://auth.fluxhaus.io/application/o/token/"
        );
        assert_eq!(
            endpoints.authorize,
            "https://auth.fluxhaus.io/application/o/authorize/"
        );
    }

    #[test]
    fn tolerates_a_trailing_slash_on_the_issuer() {
        let endpoints = Endpoints::derive(
            "https://auth.fluxhaus.io/application/o/telephone-booth-operator-mobile/",
        );
        assert_eq!(
            endpoints.token,
            "https://auth.fluxhaus.io/application/o/token/"
        );
    }
}
