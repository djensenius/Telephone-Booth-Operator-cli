//! Wire-contract round-trip tests: decode representative API JSON payloads and
//! re-encode configuration, guarding the `serde` renames and time formats.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use tbo_core::config::{Config, DEFAULT_OPERATOR_BASE_URL};
use tbo_core::domain::{
    BoothState, BoothStatus, Message, MessageStatus, RuntimeMode, StatsOverview, StatsWindow,
    WsEnvelope,
};

#[test]
fn booth_status_decodes_camel_case_enum() {
    let json = r#"{
        "state": "playingQuestion",
        "updatedAt": "2024-06-01T12:00:00Z",
        "currentQuestionId": "11111111-1111-1111-1111-111111111111",
        "runtimeMode": "simulator"
    }"#;
    let status: BoothStatus = serde_json::from_str(json).unwrap();
    assert_eq!(status.state, BoothState::PlayingQuestion);
    assert_eq!(status.runtime_mode, Some(RuntimeMode::Simulator));
    assert_eq!(
        status.current_question_id.as_deref(),
        Some("11111111-1111-1111-1111-111111111111")
    );
    assert!(status.current_message_id.is_none());

    // Re-serialize and ensure the enum keeps its camelCase wire form.
    let round = serde_json::to_string(&status).unwrap();
    assert!(round.contains("\"playingQuestion\""));
    assert!(round.contains("\"updatedAt\""));
}

#[test]
fn message_with_nested_ai_decodes() {
    let json = r#"{
        "id": "22222222-2222-2222-2222-222222222222",
        "status": "pending",
        "questionId": null,
        "createdAt": "2024-06-01T12:00:00Z",
        "audio": {
            "url": "https://example.com/a.flac",
            "sha256": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "durationMs": 4200
        },
        "latestTranscription": {
            "id": "33333333-3333-3333-3333-333333333333",
            "messageId": "22222222-2222-2222-2222-222222222222",
            "provider": "openai",
            "model": "whisper-1",
            "status": "succeeded",
            "text": "hello",
            "language": "en",
            "durationMs": 4200,
            "latencyMs": 1200,
            "error": null,
            "requestedById": null,
            "createdAt": "2024-06-01T12:00:01Z",
            "completedAt": "2024-06-01T12:00:05Z",
            "translationStatus": null,
            "translatedText": null,
            "translatedLanguage": null,
            "translationProvider": null,
            "translationModel": null,
            "translationError": null,
            "translationLatencyMs": null,
            "translationCompletedAt": null
        },
        "latestModeration": {
            "id": "44444444-4444-4444-4444-444444444444",
            "messageId": "22222222-2222-2222-2222-222222222222",
            "transcriptionId": "33333333-3333-3333-3333-333333333333",
            "provider": "openai",
            "model": "omni-moderation",
            "status": "succeeded",
            "flagged": false,
            "recommendation": "approve",
            "maxScore": 0.01,
            "categories": {"violence": 0.001, "hate": 0.0},
            "reasonSummary": null,
            "latencyMs": 90,
            "error": null,
            "requestedById": null,
            "createdAt": "2024-06-01T12:00:02Z",
            "completedAt": "2024-06-01T12:00:03Z"
        }
    }"#;
    let message: Message = serde_json::from_str(json).unwrap();
    assert_eq!(message.status, MessageStatus::Pending);
    assert_eq!(message.audio.duration_ms, Some(4200));
    let transcription = message.latest_transcription.expect("transcription present");
    assert_eq!(transcription.text.as_deref(), Some("hello"));
    let moderation = message.latest_moderation.expect("moderation present");
    assert_eq!(moderation.flagged, Some(false));
    assert_eq!(
        moderation
            .categories
            .as_ref()
            .and_then(|c| c.get("violence")),
        Some(&0.001)
    );
}

#[test]
fn ws_envelope_is_tagged_by_kind() {
    let json = r#"{
        "kind": "system",
        "boothId": "booth-1",
        "snapshot": {"cpu": {"usageRatio": 0.5}, "temperatureCelsius": 41.2},
        "receivedAt": "2024-06-01T12:00:00Z"
    }"#;
    let env: WsEnvelope = serde_json::from_str(json).unwrap();
    match env {
        WsEnvelope::System {
            booth_id, snapshot, ..
        } => {
            assert_eq!(booth_id, "booth-1");
            assert_eq!(snapshot.cpu.and_then(|c| c.usage_ratio), Some(0.5));
        }
        other => panic!("expected system frame, got {other:?}"),
    }
}

#[test]
fn stats_window_query_and_serde() {
    assert_eq!(StatsWindow::Day.as_query(), "24h");
    assert_eq!(StatsWindow::All.as_query(), "all");
    let decoded: StatsWindow = serde_json::from_str("\"7d\"").unwrap();
    assert_eq!(decoded, StatsWindow::Week);
}

#[test]
fn stats_overview_minimal_decodes() {
    let json = r#"{
        "window": "24h",
        "rangeStart": null,
        "rangeEnd": "2024-06-01T12:00:00Z",
        "generatedAt": "2024-06-01T12:00:00Z",
        "timezone": "UTC",
        "calls": {"total": 3, "completed": 2, "inProgress": 0,
            "averageDurationMs": 1200.5, "longestDurationMs": 5000.0,
            "outcomes": {"recording_completed": 2, "aborted": 1}, "perDay": []},
        "messages": {"total": 2, "byStatus": {"approved": 1, "pending": 1},
            "averageDurationMs": null},
        "playback": {"totalPlaybacks": 5},
        "pickupsHangups": {"pickups": 3, "hangups": 3, "digitsDialed": {"0": 1}},
        "uploads": {"succeeded": 2, "failed": 0, "failureRate": 0.0},
        "topQuestions": [],
        "hourly": [],
        "busiest": {"hour": 14, "dayOfWeek": 3},
        "lastActivityAt": null,
        "boothBreakdown": []
    }"#;
    let stats: StatsOverview = serde_json::from_str(json).unwrap();
    assert_eq!(stats.window, StatsWindow::Day);
    assert_eq!(stats.calls.outcomes.get("recording_completed"), Some(&2));
    assert_eq!(stats.busiest.hour, Some(14));
}

#[test]
fn config_defaults_and_round_trip() {
    let cfg = Config::default();
    assert_eq!(cfg.operator.base_url, DEFAULT_OPERATOR_BASE_URL);
    assert!(cfg.booths.is_empty());

    let toml = cfg.to_toml().unwrap();
    let reparsed: Config = toml::from_str(&toml).unwrap();
    assert_eq!(cfg, reparsed);
}

#[test]
fn config_load_missing_returns_default() {
    let path = std::env::temp_dir().join("tb-operator-test-does-not-exist.toml");
    let _ = std::fs::remove_file(&path);
    let cfg = Config::load_from(&path).unwrap();
    assert_eq!(cfg, Config::default());
}

#[test]
fn config_save_then_load() {
    let dir = std::env::temp_dir().join(format!("tb-operator-cfg-{}", std::process::id()));
    let path = dir.join("config.toml");
    let mut cfg = Config::default();
    cfg.operator.base_url = "https://api.example.test".to_owned();
    cfg.save_to(&path).unwrap();

    let loaded = Config::load_from(&path).unwrap();
    assert_eq!(loaded.operator.base_url, "https://api.example.test");

    let _ = std::fs::remove_dir_all(&dir);
}
