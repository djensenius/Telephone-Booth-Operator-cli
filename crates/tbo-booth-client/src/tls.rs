//! Self-signed-certificate fingerprint pinning for booth LAN TLS.
//!
//! The booth's debug server offers a LAN HTTPS front door (`:8443`) backed by a
//! self-signed certificate generated on the device (see the `booth-debug`
//! crate). Public CA validation therefore cannot apply; instead the operator
//! pins the certificate's SHA-256 fingerprint — read once over the loopback
//! `/v1/cert/fingerprint` endpoint and stored in config — exactly as the web
//! Debug client does.
//!
//! [`pinned_tls_config`] builds a [`rustls::ClientConfig`] whose certificate
//! verifier accepts a peer if and only if the SHA-256 of its leaf certificate
//! (the DER bytes) matches the pinned fingerprint. Hostname/SAN checks are
//! intentionally skipped: pinning the exact certificate is the trust anchor, so
//! the booth can be reached by Tailscale name or LAN IP without a matching SAN.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{
    CryptoProvider, verify_tls12_signature as rustls_verify_tls12,
    verify_tls13_signature as rustls_verify_tls13,
};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme};
use sha2::{Digest, Sha256};

use crate::error::{BoothError, Result};

/// Number of bytes in a SHA-256 digest.
const SHA256_LEN: usize = 32;

/// Builds a rustls client config that trusts a booth's LAN certificate solely
/// by its pinned SHA-256 fingerprint.
///
/// `fingerprint` is the expected SHA-256 of the leaf certificate's DER bytes,
/// as lower- or upper-case hex with or without `:` separators (both the
/// colon-separated form returned by `/v1/cert/fingerprint` and the bare form
/// stored in config are accepted).
///
/// # Errors
/// Returns [`BoothError::InvalidRequest`] when `fingerprint` is not a valid
/// 32-byte hex digest, or [`BoothError::Transport`] when the TLS config cannot
/// be constructed.
pub fn pinned_tls_config(fingerprint: &str) -> Result<ClientConfig> {
    let pinned = normalize_fingerprint(fingerprint)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(PinnedCertVerifier {
        pinned,
        provider: Arc::clone(&provider),
    });
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| BoothError::Transport(err.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    // The booth debug server speaks HTTP/1.1 only.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

/// A rustls certificate verifier that accepts exactly one leaf certificate,
/// identified by the SHA-256 of its DER encoding.
#[derive(Debug)]
struct PinnedCertVerifier {
    /// The pinned SHA-256 digest of the expected leaf certificate.
    pinned: [u8; SHA256_LEN],
    /// Crypto provider used to validate the handshake signatures.
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, TlsError> {
        let actual = Sha256::digest(end_entity.as_ref());
        if actual.as_slice() == self.pinned {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(TlsError::General(format!(
                "booth certificate fingerprint mismatch: expected {}, got {}",
                format_fingerprint(&self.pinned),
                format_fingerprint(actual.as_slice()),
            )))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        rustls_verify_tls12(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        rustls_verify_tls13(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Parse a hex fingerprint (optionally `:`-separated, any case) into its raw
/// 32 bytes.
fn normalize_fingerprint(input: &str) -> Result<[u8; SHA256_LEN]> {
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':')
        .collect();
    let bytes = hex::decode(&cleaned).map_err(|err| {
        BoothError::InvalidRequest(format!("invalid certificate fingerprint: {err}"))
    })?;
    <[u8; SHA256_LEN]>::try_from(bytes.as_slice()).map_err(|_| {
        BoothError::InvalidRequest(format!(
            "certificate fingerprint must be {SHA256_LEN} bytes ({} hex chars), got {}",
            SHA256_LEN * 2,
            bytes.len(),
        ))
    })
}

/// Format a digest as lower-case, colon-separated hex (matching the booth's
/// `/v1/cert/fingerprint` representation).
fn format_fingerprint(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn normalize_accepts_colon_separated_and_bare_and_uppercase() {
        let bare = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        let colons = "00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
                      00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff";
        let upper = bare.to_uppercase();

        let expected = normalize_fingerprint(bare).unwrap();
        assert_eq!(normalize_fingerprint(colons).unwrap(), expected);
        assert_eq!(normalize_fingerprint(&upper).unwrap(), expected);
        assert_eq!(expected[0], 0x00);
        assert_eq!(expected[31], 0xff);
    }

    #[test]
    fn normalize_rejects_wrong_length_and_non_hex() {
        assert!(normalize_fingerprint("aabbcc").is_err());
        assert!(normalize_fingerprint("zz").is_err());
        assert!(normalize_fingerprint("").is_err());
    }

    #[test]
    fn format_round_trips_through_normalize() {
        let bytes: [u8; SHA256_LEN] = std::array::from_fn(|i| u8::try_from(i).unwrap_or(0));
        let formatted = format_fingerprint(&bytes);
        assert_eq!(normalize_fingerprint(&formatted).unwrap(), bytes);
    }

    #[test]
    fn verifier_accepts_matching_and_rejects_mismatched_cert() {
        let der = CertificateDer::from(vec![1u8, 2, 3, 4, 5, 6, 7, 8]);
        let digest = Sha256::digest(der.as_ref());
        let fingerprint = format_fingerprint(digest.as_slice());
        let name = ServerName::try_from("localhost").unwrap();

        let good = PinnedCertVerifier {
            pinned: normalize_fingerprint(&fingerprint).unwrap(),
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        };
        assert!(
            good.verify_server_cert(&der, &[], &name, &[], UnixTime::now())
                .is_ok()
        );

        let bad = PinnedCertVerifier {
            pinned: [0u8; SHA256_LEN],
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        };
        assert!(
            bad.verify_server_cert(&der, &[], &name, &[], UnixTime::now())
                .is_err()
        );
    }

    #[test]
    fn pinned_tls_config_builds_for_valid_fingerprint() {
        let fingerprint = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        let config = pinned_tls_config(fingerprint).expect("config builds");
        assert_eq!(config.alpn_protocols, vec![b"http/1.1".to_vec()]);
    }

    #[test]
    fn pinned_tls_config_rejects_invalid_fingerprint() {
        assert!(pinned_tls_config("not-hex").is_err());
    }
}
