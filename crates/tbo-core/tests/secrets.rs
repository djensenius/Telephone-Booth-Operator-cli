//! Tests for the out-of-config secrets store and its merge with [`Config`].
#![allow(clippy::unwrap_used, clippy::expect_used)]

use tbo_core::config::{BoothConfig, Config};
use tbo_core::secrets::Secrets;

fn booth(id: &str, token: Option<&str>) -> BoothConfig {
    BoothConfig {
        id: id.to_owned(),
        name: None,
        debug_base_url: "http://localhost:8080".to_owned(),
        debug_token: token.map(str::to_owned),
        pinned_sha256: None,
    }
}

#[test]
fn secrets_round_trip_through_toml() {
    let mut secrets = Secrets::default();
    secrets.set_booth_token("booth-1", "tok-1");
    secrets.set_booth_token("booth-2", "tok-2");

    let toml = secrets.to_toml().unwrap();
    let reparsed: Secrets = toml::from_str(&toml).unwrap();
    assert_eq!(secrets, reparsed);
    assert_eq!(reparsed.booth_token("booth-1"), Some("tok-1"));
}

#[test]
fn set_empty_token_removes_entry() {
    let mut secrets = Secrets::default();
    secrets.set_booth_token("booth-1", "tok-1");
    secrets.set_booth_token("booth-1", "");
    assert!(secrets.is_empty());
}

#[test]
fn save_then_load_from_path() {
    let dir = std::env::temp_dir().join(format!("tb-operator-secrets-{}", std::process::id()));
    let path = dir.join("secrets.toml");
    let mut secrets = Secrets::default();
    secrets.set_booth_token("booth-1", "tok-1");
    secrets.save_to(&path).unwrap();

    let loaded = Secrets::load_from(&path).unwrap();
    assert_eq!(loaded.booth_token("booth-1"), Some("tok-1"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "secrets file must be owner-only");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_missing_returns_empty() {
    let path = std::env::temp_dir().join("tb-operator-secrets-missing.toml");
    let _ = std::fs::remove_file(&path);
    assert_eq!(Secrets::load_from(&path).unwrap(), Secrets::default());
}

#[test]
fn merge_secrets_fills_and_overrides_tokens() {
    let mut config = Config {
        booths: vec![booth("booth-1", None), booth("booth-2", Some("inline"))],
        ..Config::default()
    };
    let mut secrets = Secrets::default();
    secrets.set_booth_token("booth-1", "from-secrets");
    secrets.set_booth_token("booth-2", "wins-over-inline");

    config.merge_secrets(&secrets);

    assert_eq!(
        config.booths[0].debug_token.as_deref(),
        Some("from-secrets")
    );
    assert_eq!(
        config.booths[1].debug_token.as_deref(),
        Some("wins-over-inline")
    );
}

#[test]
fn take_secrets_moves_tokens_out_of_config() {
    let mut config = Config {
        booths: vec![booth("booth-1", Some("tok-1")), booth("booth-2", None)],
        ..Config::default()
    };

    let secrets = config.take_secrets();

    assert_eq!(secrets.booth_token("booth-1"), Some("tok-1"));
    assert!(config.booths[0].debug_token.is_none());
    // A serialized config no longer contains the secret.
    let toml = config.to_toml().unwrap();
    assert!(!toml.contains("tok-1"));
}
