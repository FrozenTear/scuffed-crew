use super::*;

#[test]
fn sync_url_https_is_allowed() {
    assert!(validate_sync_server_url("https://crew.example").is_ok());
    assert!(validate_sync_server_url("https://crew.example/stats").is_ok());
    assert!(validate_sync_server_url("HTTPS://Crew.Example:443").is_ok());
    assert!(validate_sync_server_url("  https://crew.example  ").is_ok());
}

#[test]
fn sync_url_http_non_loopback_is_rejected() {
    for raw in [
        "http://crew.example",
        "http://crew.example:8080/api",
        "http://10.0.0.5",
        "http://192.168.1.1:3030",
        "http://203.0.113.10",
        "http://[2001:db8::1]",
        "http://0.0.0.0",
        "http://127.0.0.1.evil.example",
        "ftp://localhost",
        "ws://localhost",
        "https://",
        "not a url",
        "",
    ] {
        assert!(
            validate_sync_server_url(raw).is_err(),
            "{raw} must not be allowed to carry the token"
        );
    }
    assert_eq!(
        validate_sync_server_url("http://crew.example"),
        Err(SyncUrlReject::HttpsRequired)
    );
    assert_eq!(
        validate_sync_server_url("http://crew.example")
            .unwrap_err()
            .message(),
        SYNC_URL_HTTPS_REQUIRED
    );
    assert_eq!(
        validate_sync_server_url("not a url"),
        Err(SyncUrlReject::Invalid)
    );
}

#[test]
fn sync_url_http_loopback_is_allowed() {
    for raw in [
        "http://localhost",
        "http://localhost:3030",
        "http://LOCALHOST/api",
        "http://127.0.0.1",
        "http://127.0.0.1:3030/api",
        "http://[::1]",
        "http://[::1]:3030",
    ] {
        assert!(
            validate_sync_server_url(raw).is_ok(),
            "{raw} is loopback and must stay available for local dev"
        );
    }
}

#[test]
fn existing_http_config_still_parses() {
    // Fail safe at send time. A stored cleartext URL must still load so
    // the daemon can refuse it without crashing and without dropping the
    // rest of the file.
    let raw = r#"
data_dir = "/tmp/sst-m20"
capture_output = "DP-1"
player_name = "Ada"
session_window_secs = 1800
debug_ocr = false
game_process_names = ["Overwatch.exe"]

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120

[sync]
server_url = "http://example.com"
token = "secret"
"#;
    let cfg: Config = toml::from_str(raw).expect("existing shape must still parse");
    assert!(
        !cfg.shadow_recognizer,
        "the shadow recognizer is off unless a config asks for it"
    );
    assert_eq!(
        cfg.finished_game_close_secs, FINISHED_GAME_CLOSE_DEFAULT_SECS,
        "an older config file has no quiet-close setting"
    );
    let sync = cfg.sync.expect("sync block");
    assert_eq!(sync.token, "secret");
    assert_eq!(
        validate_sync_server_url(&sync.server_url),
        Err(SyncUrlReject::HttpsRequired)
    );
}
