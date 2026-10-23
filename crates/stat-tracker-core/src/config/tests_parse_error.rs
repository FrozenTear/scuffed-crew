//! A config.toml that does not parse must never put the sync token in the
//! error text: not in `Display` (Settings, `tracing` `%e`), not in `Debug`,
//! and not in the daemon's `anyhow` startup error.

use super::parse_error::{REDACTED, redact_secrets};
use super::{Config, SetupDiskPatch, SyncConfig};

const TOKEN: &str = "tok-7f3a9c-must-never-leak";

fn with_sync(sync_body: &str, extra_root: &str) -> String {
    format!(
        "data_dir = \"/tmp/sst-parse-error\"\n{extra_root}player_name = \"FrozenTear\"\n\n[sync]\n{sync_body}"
    )
}

/// Every form of `err` a member or a log can see.
fn shown_forms(err: &(dyn std::error::Error + Send + Sync + 'static)) -> Vec<String> {
    vec![err.to_string(), format!("{err:?}"), format!("{err:#?}")]
}

fn assert_no_token(context: &str, texts: &[String]) {
    for text in texts {
        assert!(
            !text.contains(TOKEN),
            "{context}: the token leaked into the error text:\n{text}"
        );
        assert!(
            !text.contains("must-never-leak"),
            "{context}: part of the token leaked:\n{text}"
        );
    }
}

/// The broken files the tests read: a syntax error on the token line (an
/// unterminated string, a bare word), a syntax error on another line, a
/// type error that toml reports with the value, and an inline table.
fn broken_files() -> Vec<(&'static str, String)> {
    vec![
        (
            "unterminated token string",
            with_sync(
                &format!("server_url = \"https://crew.example\"\ntoken = \"{TOKEN}\n"),
                "",
            ),
        ),
        (
            "bare token word",
            with_sync(
                &format!("server_url = \"https://crew.example\"\ntoken = {TOKEN}\n"),
                "",
            ),
        ),
        (
            "token with a stray character after it",
            with_sync(
                &format!("server_url = \"https://crew.example\"\ntoken = \"{TOKEN}\" x\n"),
                "",
            ),
        ),
        (
            "syntax error on another line",
            with_sync(
                &format!("server_url = \"https://crew.example\"\ntoken = \"{TOKEN}\"\n"),
                "session_window_secs = = 3\n",
            ),
        ),
        (
            "type error on another line",
            with_sync(
                &format!("server_url = \"https://crew.example\"\ntoken = \"{TOKEN}\"\n"),
                "debug_ocr = \"maybe\"\n",
            ),
        ),
        (
            "inline sync table with a broken key",
            format!(
                "data_dir = \"/tmp/sst-parse-error\"\nsync = {{ server_url = \"https://crew.example\", token = \"{TOKEN}\", = }}\n"
            ),
        ),
        (
            "literal token string, unterminated",
            with_sync(
                &format!("server_url = \"https://crew.example\"\ntoken = '{TOKEN}\n"),
                "",
            ),
        ),
    ]
}

#[test]
fn parse_errors_never_carry_the_token() {
    for (name, text) in broken_files() {
        let err = Config::parse_file_contents(&text).expect_err(name);
        let forms = shown_forms(err.as_ref());
        assert_no_token(name, &forms);
        assert!(
            !forms[0].contains('\n'),
            "{name}: the message is one line, no quoted source: {}",
            forms[0]
        );
    }
}

#[test]
fn a_type_error_that_quotes_the_token_value_is_redacted() {
    // toml reports a wrong type with the value: "invalid type: integer
    // `…`". A number is not a usable token, but it is still what the member
    // typed into the secret field.
    let text = with_sync(
        "server_url = \"https://crew.example\"\ntoken = 98765432123\n",
        "",
    );
    let err = Config::parse_file_contents(&text).expect_err("integer token");
    for form in shown_forms(err.as_ref()) {
        assert!(!form.contains("98765432123"), "{form}");
    }
    assert!(err.to_string().contains(REDACTED), "{err}");
}

#[test]
fn the_error_still_names_the_line() {
    let text = with_sync(
        &format!("server_url = \"https://crew.example\"\ntoken = \"{TOKEN}\n"),
        "",
    );
    let err = Config::parse_file_contents(&text)
        .expect_err("unterminated")
        .to_string();
    assert!(
        err.starts_with("TOML parse error at line 6, column "),
        "{err}"
    );
}

#[test]
fn read_at_and_the_daemon_startup_error_never_carry_the_token() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    for (name, text) in broken_files() {
        std::fs::write(&path, &text).expect("seed");
        let err = Config::read_at(&path).expect_err(name);
        assert_no_token(name, &shown_forms(err.as_ref()));
        // The daemon's `main` wraps the load error the same way and returns
        // it, so Rust prints its `Debug`.
        let wrapped = anyhow::Error::from_boxed(err).context("failed to load config");
        assert_no_token(
            name,
            &[
                format!("{wrapped}"),
                format!("{wrapped:#}"),
                format!("{wrapped:?}"),
            ],
        );
    }
}

#[test]
fn settings_and_setup_writes_never_carry_the_token() {
    let patch = SetupDiskPatch {
        setup_completed: Some(true),
        sync: Some(SyncConfig {
            server_url: "https://crew.example".to_string(),
            token: "fresh-token-from-sign-in".to_string(),
        }),
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    for (name, text) in broken_files() {
        let err = Config::text_for_save(Some(&text), &Config::default()).expect_err(name);
        let forms = shown_forms(err.as_ref());
        assert_no_token(name, &forms);
        assert!(
            forms[0].starts_with("config.toml could not be parsed, so it was not replaced: "),
            "{}",
            forms[0]
        );

        let err = Config::text_for_setup_patch(Some(&text), &patch).expect_err(name);
        assert_no_token(name, &shown_forms(err.as_ref()));

        std::fs::write(&path, &text).expect("seed");
        let err = Config::default().save_at(&path).expect_err(name);
        assert_no_token(name, &shown_forms(err.as_ref()));
        let err = Config::apply_setup_patch(&path, &patch).expect_err(name);
        assert_no_token(name, &shown_forms(err.as_ref()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{name}");
    }
}

#[test]
fn redaction_finds_the_token_in_every_key_form() {
    for source in [
        format!("token = \"{TOKEN}\""),
        format!("  token=\"{TOKEN}\"   # comment"),
        format!("sync.token = \"{TOKEN}\""),
        format!("\"token\" = '{TOKEN}'"),
        format!("sync = {{ server_url = \"https://x\", token = \"{TOKEN}\" }}"),
        format!("token = {TOKEN}"),
        format!("token = \"\"\"{TOKEN}\"\"\""),
        format!("sync_token = \"{TOKEN}\""),
    ] {
        let shown = redact_secrets(&source, &format!("value `{TOKEN}` here"));
        assert_eq!(shown, format!("value `{REDACTED}` here"), "{source}");
    }
}

#[test]
fn redaction_leaves_other_values_alone() {
    let source = "server_url = \"https://crew.example\"\nplayer_name = \"FrozenTear\"\n";
    let text = "https://crew.example FrozenTear";
    assert_eq!(redact_secrets(source, text), text);
    // A very short value is not replaced; it would blank ordinary letters.
    assert_eq!(redact_secrets("token = \"ab\"", "about"), "about");
}
