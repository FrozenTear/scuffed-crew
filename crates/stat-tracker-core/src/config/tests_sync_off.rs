//! A `[sync]` table without a usable token is sync off, and the rest of
//! config.toml still loads. Before 0.6 such a file did not parse at all,
//! so Settings showed it as unreadable and the daemon did not start.

use super::*;

const ROOT: &str = "\
# hand-edited
data_dir = \"/sync-off/data\"
player_name = \"FrozenTear\"
shadow_recognizer = false

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120
";

fn with_sync(sync: &str) -> String {
    format!("{ROOT}\n[sync]\n{sync}")
}

/// The rest of the file loaded, not defaults.
fn assert_rest_loaded(config: &Config, context: &str) {
    assert_eq!(
        config.data_dir,
        PathBuf::from("/sync-off/data"),
        "{context}"
    );
    assert_eq!(
        config.player_name.as_deref(),
        Some("FrozenTear"),
        "{context}"
    );
    assert!(config.auto_detect.enabled, "{context}");
}

fn token_less_files() -> Vec<(&'static str, String)> {
    vec![
        (
            "server without token",
            with_sync("# paste the token here later\nserver_url = \"https://crew.example\"\n"),
        ),
        (
            "empty token",
            with_sync("server_url = \"https://crew.example\"\ntoken = \"\"\n"),
        ),
        (
            "token of spaces",
            with_sync("server_url = \"https://crew.example\"\ntoken = \"   \"\n"),
        ),
        ("empty table", with_sync("")),
        (
            "inline table without token",
            format!("sync = {{ server_url = \"https://crew.example\" }}\n{ROOT}"),
        ),
    ]
}

#[test]
fn a_sync_table_without_a_token_is_sync_off() {
    for (name, text) in token_less_files() {
        let config = Config::parse_file_contents(&text)
            .unwrap_or_else(|err| panic!("{name}: must parse, got {err}"));
        assert_eq!(config.sync, None, "{name}");
        assert_rest_loaded(&config, name);
    }
}

#[test]
fn read_at_loads_a_token_less_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    for (name, text) in token_less_files() {
        std::fs::write(&path, &text).expect("seed");
        let config = Config::read_at(&path).unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(config.sync, None, "{name}");
        assert_rest_loaded(&config, name);
    }
}

#[test]
fn a_token_with_a_server_is_still_sync_on() {
    let text = with_sync("server_url = \"https://crew.example\"\ntoken = \"tok-1\"\n");
    let config = Config::parse_file_contents(&text).expect("parses");
    assert_eq!(
        config.sync,
        Some(SyncConfig {
            server_url: "https://crew.example".to_string(),
            token: "tok-1".to_string(),
        })
    );
}

#[test]
fn a_token_without_a_server_is_still_an_error() {
    // Reading it as off would let a later full save drop the token.
    let text = with_sync("token = \"tok-1\"\n");
    let err = Config::parse_file_contents(&text)
        .expect_err("token without server")
        .to_string();
    assert!(err.contains("missing field `server_url`"), "{err}");
    assert!(!err.contains("tok-1"), "{err}");
}

#[test]
fn an_unchanged_settings_save_keeps_the_token_less_table() {
    for (name, text) in token_less_files() {
        let loaded = Config::parse_file_contents(&text).expect("parses");
        assert_eq!(
            Config::text_for_save(Some(&text), &loaded).expect("save"),
            text,
            "{name}: an unchanged save returns the file byte for byte"
        );
    }
}

#[test]
fn an_in_place_settings_save_keeps_the_token_less_table() {
    let text = token_less_files().remove(0).1;
    let mut next = Config::parse_file_contents(&text).expect("parses");
    next.shadow_recognizer = true;
    let saved = Config::text_for_save(Some(&text), &next).expect("save");
    assert_eq!(
        saved,
        text.replacen("shadow_recognizer = false", "shadow_recognizer = true", 1),
        "only the toggled key changes"
    );
    let reloaded = Config::parse_file_contents(&saved).expect("reload");
    assert_eq!(reloaded, next);
}

#[test]
fn settings_can_turn_sync_on_from_a_token_less_table() {
    let text = token_less_files().remove(0).1;
    let mut next = Config::parse_file_contents(&text).expect("parses");
    next.sync = Some(SyncConfig {
        server_url: "https://crew.example".to_string(),
        token: "pasted-token".to_string(),
    });
    let saved = Config::text_for_save(Some(&text), &next).expect("save");
    let reloaded = Config::parse_file_contents(&saved).expect("reload");
    assert_eq!(reloaded, next);
}

#[test]
fn the_setup_guide_fills_in_the_missing_token_in_place() {
    let text = token_less_files().remove(0).1;
    let patch = SetupDiskPatch {
        setup_completed: None,
        sync: Some(SyncConfig {
            server_url: "https://crew.example".to_string(),
            token: "fresh-token".to_string(),
        }),
    };
    let saved = Config::text_for_setup_patch(Some(&text), &patch).expect("patch");
    assert_eq!(
        saved,
        text.replacen("[sync]\n", "[sync]\ntoken = \"fresh-token\"\n", 1),
        "the token goes under the header; comments and the URL stay"
    );
    let reloaded = Config::parse_file_contents(&saved).expect("reload");
    assert_eq!(reloaded.sync, patch.sync);

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, &text).expect("seed");
    let written = Config::apply_setup_patch(&path, &patch).expect("apply");
    assert_eq!(written.sync, patch.sync);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
}

#[test]
fn a_cli_token_pairs_with_the_url_of_a_token_less_table() {
    let file_on = Some(SyncConfig {
        server_url: "https://file.example".to_string(),
        token: "file-token".to_string(),
    });
    let url = |raw: &str| Some(raw.to_string());
    // No CLI token: the file decides.
    assert_eq!(
        Config::sync_with_cli(file_on.clone(), None, None, url("https://cli.example")),
        file_on
    );
    assert_eq!(
        Config::sync_with_cli(None, url("https://stored.example"), None, None),
        None
    );
    // A CLI token takes the file's URL, from a loaded table or a token-less one.
    let pair = |server: &str, token: &str| {
        Some(SyncConfig {
            server_url: server.to_string(),
            token: token.to_string(),
        })
    };
    assert_eq!(
        Config::sync_with_cli(file_on.clone(), None, url("cli-token"), None),
        pair("https://file.example", "cli-token")
    );
    assert_eq!(
        Config::sync_with_cli(None, url("https://stored.example"), url("cli-token"), None),
        pair("https://stored.example", "cli-token")
    );
    // The CLI server wins over both.
    assert_eq!(
        Config::sync_with_cli(
            None,
            url("https://stored.example"),
            url("cli-token"),
            url("https://cli.example")
        ),
        pair("https://cli.example", "cli-token")
    );
}

#[test]
fn the_url_of_a_token_less_table_is_found() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let (_, server_only) = token_less_files().remove(0);
    std::fs::write(&path, &server_only).expect("seed");
    assert_eq!(
        Config::token_less_sync_url(&path),
        Some(Some("https://crew.example".to_string()))
    );
    std::fs::write(&path, with_sync("")).expect("seed");
    assert_eq!(Config::token_less_sync_url(&path), Some(None));
    std::fs::write(&path, ROOT).expect("seed");
    assert_eq!(Config::token_less_sync_url(&path), None);
}

#[test]
fn a_blank_cli_token_or_server_counts_as_not_given() {
    let file_on = Some(SyncConfig {
        server_url: "https://file.example".to_string(),
        token: "file-token".to_string(),
    });
    let some = |raw: &str| Some(raw.to_string());
    for blank in ["", " ", "\t", "  \n "] {
        // `SCUFFED_TOKEN=` or `--token ""` keeps the working file token.
        assert_eq!(
            Config::sync_with_cli(file_on.clone(), None, some(blank), None),
            file_on,
            "blank token {blank:?}"
        );
        assert_eq!(
            Config::sync_with_cli(
                file_on.clone(),
                None,
                some(blank),
                some("https://cli.example")
            ),
            file_on,
            "blank token {blank:?} with a CLI server"
        );
        // A token-less file stays off rather than turning on with no token.
        assert_eq!(
            Config::sync_with_cli(None, some("https://stored.example"), some(blank), None),
            None,
            "blank token {blank:?} over a token-less table"
        );
        // A blank server leaves the file's URL in place for a real CLI token.
        assert_eq!(
            Config::sync_with_cli(file_on.clone(), None, some("cli-token"), some(blank)),
            Some(SyncConfig {
                server_url: "https://file.example".to_string(),
                token: "cli-token".to_string(),
            }),
            "blank server {blank:?}"
        );
        assert_eq!(
            Config::sync_with_cli(
                None,
                some("https://stored.example"),
                some("cli-token"),
                some(blank)
            ),
            Some(SyncConfig {
                server_url: "https://stored.example".to_string(),
                token: "cli-token".to_string(),
            }),
            "blank server {blank:?} over a token-less table"
        );
    }
    // A non-blank CLI token still overrides the file token.
    assert_eq!(
        Config::sync_with_cli(file_on, None, some("cli-token"), None),
        Some(SyncConfig {
            server_url: "https://file.example".to_string(),
            token: "cli-token".to_string(),
        })
    );
}

#[test]
fn non_blank_drops_empty_and_whitespace() {
    assert_eq!(super::file::non_blank(None), None);
    assert_eq!(super::file::non_blank(Some(String::new())), None);
    assert_eq!(super::file::non_blank(Some(" \t\n".to_string())), None);
    assert_eq!(
        super::file::non_blank(Some("tok".to_string())),
        Some("tok".to_string())
    );
}
