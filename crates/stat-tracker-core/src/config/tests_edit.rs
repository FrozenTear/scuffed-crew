use super::tests::settings_shaped_without_shadow;
use super::*;

#[test]
fn update_channel_is_edited_in_place_and_defaults_to_stable() {
    let raw = "\
# keep this comment
data_dir = \"/tmp/data\"
shadow_recognizer = false
";
    let loaded = Config::parse_file_contents(raw).expect("parse");
    assert!(!loaded.wants_prerelease_updates());
    assert!(!Config::default().wants_prerelease_updates());

    let mut next = loaded.clone();
    next.update_channel = "prerelease".to_string();
    let saved = Config::text_for_save(Some(raw), &next).expect("patch");
    assert!(saved.contains("# keep this comment"), "{saved}");
    assert!(saved.contains("shadow_recognizer = false"), "{saved}");
    assert!(saved.contains("update_channel = \"prerelease\""), "{saved}");
    let opted = Config::parse_file_contents(&saved).expect("reparse");
    assert!(opted.wants_prerelease_updates());

    let mut back = opted.clone();
    back.update_channel.clear();
    let stable = Config::text_for_save(Some(&saved), &back).expect("clear");
    assert!(stable.contains("# keep this comment"), "{stable}");
    assert!(stable.contains("shadow_recognizer = false"), "{stable}");
    assert!(stable.contains("update_channel = \"stable\""), "{stable}");
    assert!(
        !Config::parse_file_contents(&stable)
            .expect("stable")
            .wants_prerelease_updates()
    );
}

#[test]
fn capture_route_is_edited_in_place() {
    let existing = "\
# keep
data_dir = \"/tmp/sst-route\"
capture_output = \"DP-1\"
";
    let mut next: Config = toml::from_str(existing).unwrap();
    next.capture_route = CaptureRoute::Screenshot;
    let saved = Config::text_for_save(Some(existing), &next).unwrap();
    assert!(saved.contains("# keep"), "{saved}");
    assert!(saved.contains("capture_output = \"DP-1\""), "{saved}");
    assert!(saved.contains("capture_route = \"screenshot\""), "{saved}");
    let back: Config = toml::from_str(&saved).unwrap();
    assert_eq!(back.capture_route, CaptureRoute::Screenshot);
}

fn hand_edited_config(shadow_line: &str) -> String {
    format!(
        "\
# hand-edited tracker config. keep this comment.
data_dir = \"/tmp/sst-hand-edited\"

# scoreboard name, not a default key dump
player_name = \"FrozenTear\"

# extra number reader (private log only)
{shadow_line}

# a key Settings does not own
custom_note = \"leave this line alone\"

session_window_secs = 1200
"
    )
}

fn changed_lines<'a>(before: &'a str, after: &'a str) -> Vec<(&'a str, &'a str)> {
    let before_lines: Vec<_> = before.split_inclusive('\n').collect();
    let after_lines: Vec<_> = after.split_inclusive('\n').collect();
    let mut diffs = Vec::new();
    let count = before_lines.len().max(after_lines.len());
    for index in 0..count {
        let left = before_lines.get(index).copied().unwrap_or("");
        let right = after_lines.get(index).copied().unwrap_or("");
        if left != right {
            diffs.push((left, right));
        }
    }
    diffs
}

#[test]
fn flipping_shadow_recognizer_changes_only_that_line() {
    for (before_line, after_line) in [
        ("shadow_recognizer = false", "shadow_recognizer = true"),
        ("shadow_recognizer = true", "shadow_recognizer = false"),
    ] {
        let raw = hand_edited_config(before_line);
        let mut next: Config = toml::from_str(&raw).expect("hand-edited file must parse");
        next.shadow_recognizer = !next.shadow_recognizer;
        let saved = Config::text_for_save(Some(&raw), &next).expect("shadow-only save");
        let diffs = changed_lines(&raw, &saved);
        assert_eq!(
            diffs.len(),
            1,
            "flipping the toggle must change one line, got {diffs:?}\n{saved}"
        );
        assert!(
            diffs[0].0.contains(before_line),
            "old line: {:?}",
            diffs[0].0
        );
        assert!(
            diffs[0].1.contains(after_line),
            "new line: {:?}",
            diffs[0].1
        );
        assert!(saved.contains("# hand-edited tracker config. keep this comment."));
        assert!(saved.contains("# scoreboard name, not a default key dump"));
        assert!(saved.contains("# extra number reader (private log only)"));
        assert!(saved.contains("custom_note = \"leave this line alone\""));
        assert!(saved.contains("player_name = \"FrozenTear\""));
        assert!(
            !saved.contains("game_process_names"),
            "a shadow toggle must not add default keys: {saved}"
        );
    }
}

#[test]
fn save_skips_write_when_nothing_changed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let raw = hand_edited_config("shadow_recognizer = true");
    std::fs::write(&path, &raw).expect("seed");
    let stamped = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .expect("open")
        .set_modified(stamped)
        .expect("stamp mtime");

    let cfg: Config = toml::from_str(&raw).expect("parse");
    cfg.save_to_path(&path).expect("unchanged save");

    assert_eq!(std::fs::read_to_string(&path).expect("read"), raw);
    assert_eq!(
        std::fs::metadata(&path)
            .expect("meta")
            .modified()
            .expect("mtime"),
        stamped,
        "save() must not touch the file when the text is unchanged"
    );
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .expect("dir")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("config.toml")]);
}

#[test]
fn save_refuses_to_replace_an_unparseable_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let raw = "\
this is not toml
server_url = \"https://crew.example\"
token = \"secret-token-must-stay\"
";
    std::fs::write(&path, raw).expect("seed");
    let err = Config::default()
        .save_to_path(&path)
        .expect_err("a broken config must not be replaced");
    let message = err.to_string();
    assert!(
        message.contains("not replaced"),
        "error should say the file was kept: {message}"
    );
    assert!(
        !message.contains('—') && !message.contains('–'),
        "{message}"
    );
    assert_eq!(std::fs::read_to_string(&path).expect("read"), raw);
    assert!(
        std::fs::read_to_string(&path)
            .expect("read")
            .contains("secret-token-must-stay")
    );
}

#[test]
fn save_replaces_the_file_atomically_with_mode_0600() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let raw = hand_edited_config("shadow_recognizer = false");
    std::fs::write(&path, &raw).expect("seed");
    let mut next: Config = toml::from_str(&raw).expect("parse");
    next.shadow_recognizer = true;
    next.save_to_path(&path).expect("save");

    let saved = std::fs::read_to_string(&path).expect("read");
    let diffs = changed_lines(&raw, &saved);
    assert_eq!(diffs.len(), 1, "{diffs:?}\n{saved}");
    assert!(diffs[0].1.contains("shadow_recognizer = true"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "saved config must be owner-only");
    }
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .expect("dir")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(
        names,
        vec![std::ffi::OsString::from("config.toml")],
        "the temp file must be renamed, not left behind"
    );
}

/// The first-run guide's writer is owner-only too: a changed file is
/// replaced with mode 0600, an unchanged one is tightened where it is, and
/// a missing one is created 0600.
#[cfg(unix)]
#[test]
fn setup_patch_writes_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |path: &std::path::Path| {
        std::fs::metadata(path).expect("meta").permissions().mode() & 0o777
    };
    let open_up = |path: &std::path::Path| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).expect("chmod")
    };
    let patch = SetupDiskPatch {
        setup_completed: Some(true),
        sync: None,
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");

    std::fs::write(&path, hand_edited_config("shadow_recognizer = false")).expect("seed");
    open_up(&path);
    Config::apply_setup_patch(&path, &patch).expect("patch");
    let patched = std::fs::read_to_string(&path).expect("read");
    assert!(patched.contains("setup_completed = true"), "{patched}");
    assert_eq!(mode(&path), 0o600, "a patched config must be owner-only");

    open_up(&path);
    Config::apply_setup_patch(&path, &patch).expect("unchanged patch");
    assert_eq!(std::fs::read_to_string(&path).expect("read"), patched);
    assert_eq!(mode(&path), 0o600, "an unchanged config is tightened");

    let fresh = dir.path().join("fresh").join("config.toml");
    Config::apply_setup_patch(&fresh, &patch).expect("fresh patch");
    assert_eq!(mode(&fresh), 0o600, "a new config must be owner-only");

    let mut names: Vec<_> = std::fs::read_dir(dir.path())
        .expect("dir")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["config.toml", "fresh"].map(std::ffi::OsString::from),
        "the temp file must be renamed, not left behind"
    );
}

fn table_block(text: &str) -> &str {
    text.find("[auto_detect]")
        .map(|index| &text[index..])
        .unwrap_or(text)
}

fn assert_root_shadow(text: &str, on: bool) {
    let parsed = toml_edit::Document::parse(text).expect("edited file must parse");
    let root = parsed
        .get("shadow_recognizer")
        .and_then(|item| item.as_value())
        .and_then(|value| value.as_bool());
    assert_eq!(
        root,
        Some(on),
        "exactly one root shadow_recognizer:\n{text}"
    );
    let loaded = Config::parse_file_contents(text).expect("Config::load parse");
    assert_eq!(
        loaded.shadow_recognizer, on,
        "Config::load must read the root value"
    );
    let header = text
        .find("\n[")
        .map(|index| index + 1)
        .or_else(|| text.starts_with('[').then_some(0))
        .expect("table header");
    assert!(
        text[..header].contains("shadow_recognizer"),
        "the root key must sit before the first table:\n{text}"
    );
    assert_eq!(
        text[..header].matches("shadow_recognizer").count(),
        1,
        "the root key must not be duplicated:\n{text}"
    );
}

#[test]
fn missing_shadow_recognizer_toggles_stay_on_the_root() {
    let original = settings_shaped_without_shadow();
    let tables = table_block(&original).to_string();
    assert!(!original.contains("shadow_recognizer"));
    let mut current = Config::parse_file_contents(&original).expect("seed parses");
    assert!(!current.shadow_recognizer);

    let mut text = original.clone();
    for on in [true, false, true] {
        current.shadow_recognizer = on;
        text = Config::text_for_save(Some(&text), &current).expect("toggle");
        assert_root_shadow(&text, on);
        assert_eq!(
            table_block(&text),
            tables,
            "tables must stay byte for byte on toggle {on}:\n{text}"
        );
        current = Config::parse_file_contents(&text).expect("reload");
    }
}

#[test]
fn nested_shadow_recognizer_is_left_alone_and_root_is_inserted_once() {
    let original = "\
data_dir = \"/tmp/sst-nested-shadow\"
player_name = \"FrozenTear\"

[sync]
server_url = \"https://crew.example\"
token = \"not-a-real-token\"
shadow_recognizer = true
";
    let mut next = Config::parse_file_contents(original).expect("nested key is not the root");
    assert!(!next.shadow_recognizer);
    next.shadow_recognizer = true;
    let saved = Config::text_for_save(Some(original), &next).expect("insert root");
    assert_root_shadow(&saved, true);
    let doc = toml_edit::Document::parse(&saved).expect("parse");
    let nested = doc
        .get("sync")
        .and_then(|item| item.as_table())
        .and_then(|table| table.get("shadow_recognizer"))
        .and_then(|item| item.as_value())
        .and_then(|value| value.as_bool());
    assert_eq!(nested, Some(true), "the mistaken table key stays:\n{saved}");
    let again = Config::text_for_save(Some(&saved), &next).expect("second toggle");
    assert_eq!(again, saved, "a second on must not add another key");
    assert_root_shadow(&again, true);
}

#[test]
fn missing_shadow_recognizer_without_tables_is_appended_at_the_end() {
    let original = "\
data_dir = \"/tmp/sst-no-tables\"
player_name = \"FrozenTear\"
";
    let mut next = Config::parse_file_contents(original).expect("parse");
    next.shadow_recognizer = true;
    let saved = Config::text_for_save(Some(original), &next).expect("insert");
    let doc = toml_edit::Document::parse(&saved).expect("parse");
    assert_eq!(
        doc.get("shadow_recognizer")
            .and_then(|item| item.as_value())
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert!(
        !saved.contains('['),
        "a file with no tables stays free of headers:\n{saved}"
    );
    assert!(
        Config::parse_file_contents(&saved)
            .expect("load")
            .shadow_recognizer
    );
}

#[test]
fn setup_patch_edits_completed_and_sync_in_place_and_never_writes_device_code() {
    let secret = "dc-SECRET-9f3a-not-for-disk";
    let original = "\
# hand-edited tracker config. keep this comment.
data_dir = \"/tmp/sst-hand-edited\"

# scoreboard name, not a default key dump
player_name = \"FrozenTear\"

# extra number reader (private log only)
shadow_recognizer = true

# a key Settings does not own
custom_note = \"leave this line alone\"

session_window_secs = 1200

[sync]
server_url = \"https://old.example\"
token = \"old-token\"
";
    let patch = SetupDiskPatch {
        setup_completed: Some(true),
        sync: Some(SyncConfig {
            server_url: "https://crew.example".to_string(),
            token: "pasted-token".to_string(),
        }),
    };
    let saved = Config::text_for_setup_patch(Some(original), &patch).expect("patch");
    assert!(saved.contains("# hand-edited tracker config. keep this comment."));
    assert!(saved.contains("shadow_recognizer = true"));
    assert!(saved.contains("custom_note = \"leave this line alone\""));
    assert!(saved.contains("player_name = \"FrozenTear\""));
    assert!(saved.contains("setup_completed = true"));
    assert!(saved.contains("https://crew.example"));
    assert!(saved.contains("pasted-token"));
    assert!(!saved.contains("old-token"));
    assert!(!saved.contains(secret));
    assert!(
        !saved.contains("device_code"),
        "the guide patch must not invent a device_code key:\n{saved}"
    );
    let loaded = Config::parse_file_contents(&saved).expect("parse");
    assert!(loaded.setup_completed);
    assert!(loaded.shadow_recognizer);
    let sync = loaded.sync.expect("sync");
    assert_eq!(sync.server_url, "https://crew.example");
    assert_eq!(sync.token, "pasted-token");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, original).expect("seed");
    Config::apply_setup_patch(&path, &patch).expect("write");
    let disk = std::fs::read_to_string(&path).expect("read");
    assert!(!disk.contains(secret));
    assert!(disk.contains("pasted-token"));
    assert!(disk.contains("shadow_recognizer = true"));
}

#[test]
fn setup_patch_refuses_an_unparseable_file() {
    let original = "this is not toml {";
    let err = Config::text_for_setup_patch(
        Some(original),
        &SetupDiskPatch {
            setup_completed: Some(true),
            sync: None,
        },
    )
    .expect_err("broken file");
    assert!(err.to_string().contains("not replaced"));
}

#[test]
fn fresh_setup_patch_writes_completed_without_a_device_code() {
    let saved = Config::text_for_setup_patch(
        None,
        &SetupDiskPatch {
            setup_completed: Some(true),
            sync: Some(SyncConfig {
                server_url: "https://crew.example".into(),
                token: "tok".into(),
            }),
        },
    )
    .expect("fresh");
    let loaded = Config::parse_file_contents(&saved).expect("parse");
    assert!(loaded.setup_completed);
    assert_eq!(loaded.sync.expect("sync").token, "tok");
    assert!(!saved.contains("device_code"));
}

#[test]
fn missing_reader_toggles_stay_on_the_root_and_keep_comments() {
    let original = settings_shaped_without_shadow();
    let tables = table_block(&original).to_string();
    assert!(!original.contains("reader"));
    let mut current = Config::parse_file_contents(&original).expect("seed parses");
    assert!(current.uses_new_reader(), "no reader line: the new reader");

    // Switching back to ocr-v1 has to write the key: missing means new.
    current.reader = ReaderSetting::OcrV1;
    let back = Config::text_for_save(Some(&original), &current).expect("switch back");
    assert!(back.contains("reader = \"ocr-v1\""), "{back}");
    assert_eq!(table_block(&back), tables, "tables stay:\n{back}");
    assert!(
        Config::parse_file_contents(&back)
            .expect("reload back")
            .reader
            .is_ocr_v1(),
        "the switch back survives a reload"
    );

    current.reader = ReaderSetting::New;
    let on = Config::text_for_save(Some(&back), &current).expect("turn on");
    assert!(on.contains("# ") || !original.contains("# "));
    assert!(on.contains("reader = \"new\""), "{on}");
    assert_eq!(
        table_block(&on),
        tables,
        "tables must stay byte for byte:\n{on}"
    );
    assert!(
        on.find("reader").unwrap() < on.find("[auto_detect]").unwrap(),
        "reader stays on the root:\n{on}"
    );
    let loaded = Config::parse_file_contents(&on).expect("reload on");
    assert!(loaded.uses_new_reader());

    current.reader = ReaderSetting::OcrV1;
    let off = Config::text_for_save(Some(&on), &current).expect("turn off");
    assert!(off.contains("reader = \"ocr-v1\""), "{off}");
    assert_eq!(
        table_block(&off),
        tables,
        "tables must stay byte for byte:\n{off}"
    );
    assert!(
        Config::parse_file_contents(&off)
            .expect("reload off")
            .reader
            .is_ocr_v1()
    );
}

#[test]
fn reader_toggle_preserves_a_hand_edited_comment() {
    let raw = "\
# hand-edited tracker config. keep this comment.
data_dir = \"/tmp/sst-hand-edited\"
player_name = \"FrozenTear\"
reader = \"ocr-v1\"
custom_note = \"leave this line alone\"
";
    let mut next: Config = toml::from_str(raw).expect("hand-edited file must parse");
    next.reader = ReaderSetting::New;
    let saved = Config::text_for_save(Some(raw), &next).expect("reader-only save");
    assert!(saved.contains("# hand-edited tracker config. keep this comment."));
    assert!(saved.contains("custom_note = \"leave this line alone\""));
    assert!(saved.contains("player_name = \"FrozenTear\""));
    assert!(saved.contains("reader = \"new\""));
    assert!(!saved.contains("reader = \"ocr-v1\""));
    assert!(
        !saved.contains("game_process_names"),
        "a reader toggle must not add default keys: {saved}"
    );
}

#[test]
fn an_ocr_toggle_edits_one_key_and_keeps_the_rest_of_the_file() {
    let raw = "\
# hand-edited tracker config. keep this comment.
data_dir = \"/tmp/sst-hand-edited\"
player_name = \"FrozenTear\"
reader = \"ocr-v1\"

[auto_detect]
enabled = true # watch for games
poll_interval_secs = 4
cooldown_secs = 120
";
    let mut next: Config = toml::from_str(raw).expect("hand-edited file must parse");
    next.ocr = OcrSetting::Off;
    let off = Config::text_for_save(Some(raw), &next).expect("turn OCR off");
    assert!(off.contains("ocr = \"off\""), "{off}");
    assert!(
        off.find("ocr = ").unwrap() < off.find("[auto_detect]").unwrap(),
        "ocr stays on the root:\n{off}"
    );
    assert_eq!(
        off.replace("ocr = \"off\"\n", ""),
        raw,
        "nothing else changed"
    );
    assert_eq!(Config::parse_file_contents(&off).unwrap(), next);

    next.ocr = OcrSetting::On;
    let on = Config::text_for_save(Some(&off), &next).expect("turn OCR on");
    assert_eq!(
        on,
        off.replace("ocr = \"off\"", "ocr = \"on\""),
        "the key is spliced in place"
    );
    assert_eq!(
        Config::parse_file_contents(&on).unwrap().ocr,
        OcrSetting::On
    );
}
