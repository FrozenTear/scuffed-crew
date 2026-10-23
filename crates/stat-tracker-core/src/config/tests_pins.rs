//! Byte-exact pins for the `config.toml` format.
//!
//! The desktop app and the daemon read and write the same file, and older
//! releases read what this one writes. These tests fix the text that a full
//! save writes (key names, key order, which keys are left out) and what each
//! in-place save path changes in a hand-edited file with comments. A change
//! to the `Config` struct, its serde attributes, the default fns or the
//! in-place editor that changes a byte here is a format change, not a
//! refactor.
//!
//! The expected text is also the output of the `toml` and `toml_edit`
//! crates, so bumping either can change it. When that happens, re-capture
//! the expected strings on purpose and review the diff as a format change.

use super::*;

/// A hand-edited file without any in-place key: each path inserts its key.
const BASE: &str = "\
# scuffed-stat-tracker config. Every comment here must survive a Settings save.
data_dir = \"/pin/data\" # where the store lives

# who you are on the scoreboard
player_name = \"FrozenTear\"
capture_output = \"DP-1\"
session_window_secs = 1800
game_process_names = [\"Overwatch.exe\"]

# a key Settings does not own
custom_note = \"leave this line alone\"

[auto_detect]
enabled = true # watch for games
poll_interval_secs = 4
cooldown_secs = 120
";

const SYNC_BLOCK: &str = "
[sync]
# the crew site
server_url = \"https://old.example\"
token = \"old-token\"
";

/// A hand-edited file with every in-place key set to a non-default value,
/// with odd spacing and a literal string: each path splices its value.
const SET: &str = "\
# scuffed-stat-tracker config. Every comment here must survive a Settings save.
data_dir = \"/pin/data\" # where the store lives

# in-place keys, each with its own comment
shadow_recognizer   =   false # extra number reader
update_channel = 'prerelease' # alphas and rcs
reader = \"ocr-v1\" # classic reader
ocr = \"off\"
capture_route = \"screenshot\"
setup_completed = false

# a key Settings does not own
custom_note = \"leave this line alone\"

[auto_detect]
enabled = true # watch for games
poll_interval_secs = 4
cooldown_secs = 120

[sync]
server_url = \"https://old.example\"
# the token below
token = \"old-token\"
";

/// The last root line of [`BASE`]. A missing root key is inserted after it,
/// before the blank line and the first table.
const LAST_ROOT_LINE: &str = "custom_note = \"leave this line alone\"\n";

fn pinned_default() -> Config {
    Config {
        data_dir: PathBuf::from("/pin/data"),
        ..Config::default()
    }
}

fn every_field_set() -> Config {
    Config {
        data_dir: PathBuf::from("/pin/data"),
        capture_output: Some("DP-1".to_string()),
        capture_route: CaptureRoute::Kwin,
        debug_portal_frames: true,
        screencast_max_fps: Some(2),
        screencast_pause_between_grabs: Some(false),
        player_name: Some("FrozenTear".to_string()),
        sync: Some(SyncConfig {
            server_url: "https://crew.example".to_string(),
            token: "not-a-real-token".to_string(),
        }),
        auto_detect: AutoDetectConfig {
            enabled: false,
            poll_interval_secs: Some(8),
            poll_interval_ms: Some(750),
            cooldown_secs: 30,
        },
        session_window_secs: 900,
        finished_game_close_secs: 240,
        game_process_names: vec!["Overwatch.exe".to_string(), "wine64".to_string()],
        debug_ocr: true,
        report_kit_max_games: Some(80),
        report_kit_max_mb: Some(1000),
        ocr_threads: Some(2),
        shadow_recognizer: true,
        update_channel: "prerelease".to_string(),
        setup_completed: true,
        reader_pack_url: Some("https://crew.example/packs/reader.zip".to_string()),
        reader: ReaderSetting::OcrV1,
        ocr: OcrSetting::Off,
        name_reader: false,
    }
}

const DEFAULT_TEXT: &str = "\
data_dir = \"/pin/data\"
session_window_secs = 1800
finished_game_close_secs = 180
game_process_names = [\"Overwatch.exe\"]
debug_ocr = false

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120
";

const EVERY_FIELD_TEXT: &str = "\
data_dir = \"/pin/data\"
capture_output = \"DP-1\"
capture_route = \"kwin\"
debug_portal_frames = true
screencast_max_fps = 2
screencast_pause_between_grabs = false
player_name = \"FrozenTear\"
session_window_secs = 900
finished_game_close_secs = 240
game_process_names = [
    \"Overwatch.exe\",
    \"wine64\",
]
debug_ocr = true
report_kit_max_games = 80
report_kit_max_mb = 1000
ocr_threads = 2
shadow_recognizer = true
update_channel = \"prerelease\"
setup_completed = true
reader_pack_url = \"https://crew.example/packs/reader.zip\"
reader = \"ocr-v1\"
ocr = \"off\"
name_reader = false

[sync]
server_url = \"https://crew.example\"
token = \"not-a-real-token\"

[auto_detect]
enabled = false
poll_interval_secs = 8
poll_interval_ms = 750
cooldown_secs = 30
";

fn base_with_sync() -> String {
    format!("{BASE}{SYNC_BLOCK}")
}

/// `text` with the one occurrence of `old` replaced by `new`.
fn one_edit(text: &str, old: &str, new: &str) -> String {
    assert_eq!(text.matches(old).count(), 1, "{old:?} must occur once");
    text.replacen(old, new, 1)
}

/// `text` with `lines` inserted after [`LAST_ROOT_LINE`].
fn root_insert(text: &str, lines: &str) -> String {
    one_edit(text, LAST_ROOT_LINE, &format!("{LAST_ROOT_LINE}{lines}"))
}

/// What a Settings save writes when `edit` is the change.
fn save(existing: &str, edit: impl FnOnce(&mut Config)) -> String {
    let mut next = Config::parse_file_contents(existing).expect("fixture parses");
    edit(&mut next);
    Config::text_for_save(Some(existing), &next).expect("save text")
}

fn setup_patch() -> SetupDiskPatch {
    SetupDiskPatch {
        setup_completed: Some(true),
        sync: Some(SyncConfig {
            server_url: "https://crew.example".to_string(),
            token: "pasted-token".to_string(),
        }),
    }
}

#[test]
fn pin_default_config_text() {
    let text = toml::to_string_pretty(&pinned_default()).expect("serialize");
    assert_eq!(text, DEFAULT_TEXT);
    assert_eq!(
        Config::parse_file_contents(&text).unwrap(),
        pinned_default()
    );
}

#[test]
fn pin_every_field_set_text() {
    let text = toml::to_string_pretty(&every_field_set()).expect("serialize");
    assert_eq!(text, EVERY_FIELD_TEXT);
    assert_eq!(
        Config::parse_file_contents(&text).unwrap(),
        every_field_set()
    );
}

/// Field order is the output order. Root values come first, then the
/// tables in field order (`sync` before `auto_detect`).
#[test]
fn pin_key_order() {
    let keys: Vec<&str> = EVERY_FIELD_TEXT
        .lines()
        .filter_map(|line| {
            if line.starts_with('[') {
                Some(line)
            } else {
                line.split_once(" = ").map(|(key, _)| key)
            }
        })
        .collect();
    assert_eq!(
        keys,
        [
            "data_dir",
            "capture_output",
            "capture_route",
            "debug_portal_frames",
            "screencast_max_fps",
            "screencast_pause_between_grabs",
            "player_name",
            "session_window_secs",
            "finished_game_close_secs",
            "game_process_names",
            "debug_ocr",
            "report_kit_max_games",
            "report_kit_max_mb",
            "ocr_threads",
            "shadow_recognizer",
            "update_channel",
            "setup_completed",
            "reader_pack_url",
            "reader",
            "ocr",
            "name_reader",
            "[sync]",
            "server_url",
            "token",
            "[auto_detect]",
            "enabled",
            "poll_interval_secs",
            "poll_interval_ms",
            "cooldown_secs",
        ]
    );
    assert_eq!(
        toml::to_string_pretty(&every_field_set()).unwrap(),
        EVERY_FIELD_TEXT
    );
}

/// A file that sets only `data_dir` is the default config.
#[test]
fn pin_minimal_file_is_the_default() {
    let loaded = Config::parse_file_contents("data_dir = \"/pin/data\"\n").expect("parse");
    assert_eq!(loaded, pinned_default());
}

/// What each field is when the file sets only `data_dir`, spelled out
/// value by value. [`pin_minimal_file_is_the_default`] compares with
/// `Config::default()`, so it would not notice a default that changed in
/// both places at once (for example in an enum's `#[default]`, `Default`
/// and `is_auto` together).
#[test]
fn pin_minimal_file_field_values() {
    let c = Config::parse_file_contents("data_dir = \"/pin/data\"\n").expect("parse");
    assert_eq!(c.data_dir, PathBuf::from("/pin/data"));
    assert_eq!(c.capture_output, None);
    assert_eq!(c.capture_route.as_str(), "auto");
    assert!(!c.debug_portal_frames);
    assert_eq!(c.screencast_max_fps, None);
    assert_eq!(c.screencast_pause_between_grabs, None);
    assert_eq!(c.player_name, None);
    assert_eq!(c.sync, None);
    assert!(c.auto_detect.enabled);
    assert_eq!(c.auto_detect.poll_interval_secs, None);
    assert_eq!(c.auto_detect.poll_interval_ms, None);
    assert_eq!(c.auto_detect.cooldown_secs, 120);
    assert_eq!(c.session_window_secs, 1800);
    assert_eq!(c.finished_game_close_secs, 180);
    assert_eq!(c.game_process_names, ["Overwatch.exe"]);
    assert!(!c.debug_ocr);
    assert_eq!(c.report_kit_max_games, None);
    assert_eq!(c.report_kit_max_mb, None);
    assert_eq!(c.ocr_threads, None);
    assert!(!c.shadow_recognizer);
    assert_eq!(c.update_channel, "");
    assert!(!c.setup_completed);
    assert_eq!(c.reader_pack_url, None);
    assert_eq!(c.reader.as_str(), "new");
    assert_eq!(c.ocr.as_str(), "on");
    assert!(c.name_reader);
}

#[test]
fn pin_unchanged_save_returns_the_file() {
    let base = base_with_sync();
    assert_eq!(save(&base, |_| {}), base);
    assert_eq!(save(SET, |_| {}), SET);
}

#[test]
fn pin_shadow_recognizer_in_place() {
    let base = base_with_sync();
    assert_eq!(
        save(&base, |c| c.shadow_recognizer = true),
        root_insert(&base, "shadow_recognizer = true\n")
    );
    assert_eq!(
        save(SET, |c| c.shadow_recognizer = true),
        one_edit(
            SET,
            "shadow_recognizer   =   false #",
            "shadow_recognizer   =   true #"
        )
    );
}

#[test]
fn pin_update_channel_in_place() {
    let base = base_with_sync();
    assert_eq!(
        save(&base, |c| c.update_channel = "prerelease".to_string()),
        root_insert(&base, "update_channel = \"prerelease\"\n")
    );
    assert_eq!(
        save(SET, |c| c.update_channel.clear()),
        one_edit(
            SET,
            "update_channel = 'prerelease' #",
            "update_channel = \"stable\" #"
        )
    );
}

#[test]
fn pin_reader_in_place() {
    let base = base_with_sync();
    assert_eq!(
        save(&base, |c| c.reader = ReaderSetting::OcrV1),
        root_insert(&base, "reader = \"ocr-v1\"\n")
    );
    assert_eq!(
        save(SET, |c| c.reader = ReaderSetting::New),
        one_edit(SET, "reader = \"ocr-v1\" #", "reader = \"new\" #")
    );
}

#[test]
fn pin_reader_and_shadow_in_place() {
    let base = base_with_sync();
    assert_eq!(
        save(&base, |c| {
            c.reader = ReaderSetting::OcrV1;
            c.shadow_recognizer = true;
        }),
        root_insert(&base, "shadow_recognizer = true\nreader = \"ocr-v1\"\n")
    );
    let both = one_edit(SET, "reader = \"ocr-v1\" #", "reader = \"new\" #");
    let both = one_edit(
        &both,
        "shadow_recognizer   =   false #",
        "shadow_recognizer   =   true #",
    );
    assert_eq!(
        save(SET, |c| {
            c.reader = ReaderSetting::New;
            c.shadow_recognizer = true;
        }),
        both
    );
}

#[test]
fn pin_ocr_in_place() {
    let base = base_with_sync();
    assert_eq!(
        save(&base, |c| c.ocr = OcrSetting::Off),
        root_insert(&base, "ocr = \"off\"\n")
    );
    assert_eq!(
        save(SET, |c| c.ocr = OcrSetting::On),
        one_edit(SET, "ocr = \"off\"\n", "ocr = \"on\"\n")
    );
}

#[test]
fn pin_capture_route_in_place() {
    let base = base_with_sync();
    assert_eq!(
        save(&base, |c| c.capture_route = CaptureRoute::Kwin),
        root_insert(&base, "capture_route = \"kwin\"\n")
    );
    assert_eq!(
        save(SET, |c| c.capture_route = CaptureRoute::Auto),
        one_edit(
            SET,
            "capture_route = \"screenshot\"\n",
            "capture_route = \"auto\"\n"
        )
    );
}

/// Any other change is a full pretty-printed rewrite: comments and keys the
/// struct does not know are dropped.
#[test]
fn pin_other_changes_rewrite_the_whole_file() {
    let saved = save(&base_with_sync(), |c| {
        c.player_name = Some("Ada".to_string());
        c.debug_ocr = true;
    });
    assert_eq!(
        saved,
        "\
data_dir = \"/pin/data\"
capture_output = \"DP-1\"
player_name = \"Ada\"
session_window_secs = 1800
finished_game_close_secs = 180
game_process_names = [\"Overwatch.exe\"]
debug_ocr = true

[sync]
server_url = \"https://old.example\"
token = \"old-token\"

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120
"
    );
}

/// Settings can change `reader` and `ocr` in one save. Only `reader` +
/// `shadow_recognizer` has a combined in-place path, so this pair takes the
/// full rewrite, from either fixture.
#[test]
fn pin_reader_and_ocr_together_rewrite_the_whole_file() {
    let saved = save(&base_with_sync(), |c| {
        c.reader = ReaderSetting::OcrV1;
        c.ocr = OcrSetting::Off;
    });
    assert_eq!(
        saved,
        "\
data_dir = \"/pin/data\"
capture_output = \"DP-1\"
player_name = \"FrozenTear\"
session_window_secs = 1800
finished_game_close_secs = 180
game_process_names = [\"Overwatch.exe\"]
debug_ocr = false
reader = \"ocr-v1\"
ocr = \"off\"

[sync]
server_url = \"https://old.example\"
token = \"old-token\"

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120
"
    );
    let saved = save(SET, |c| {
        c.reader = ReaderSetting::New;
        c.ocr = OcrSetting::On;
    });
    assert_eq!(
        saved,
        "\
data_dir = \"/pin/data\"
capture_route = \"screenshot\"
session_window_secs = 1800
finished_game_close_secs = 180
game_process_names = [\"Overwatch.exe\"]
debug_ocr = false
update_channel = \"prerelease\"

[sync]
server_url = \"https://old.example\"
token = \"old-token\"

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120
"
    );
}

#[test]
fn pin_setup_patch_in_place() {
    let base = base_with_sync();
    let patched = one_edit(
        &root_insert(&base, "setup_completed = true\n"),
        "server_url = \"https://old.example\"\ntoken = \"old-token\"\n",
        "server_url = \"https://crew.example\"\ntoken = \"pasted-token\"\n",
    );
    assert_eq!(
        Config::text_for_setup_patch(Some(&base), &setup_patch()).unwrap(),
        patched
    );
    let completed_only = SetupDiskPatch {
        setup_completed: Some(true),
        sync: None,
    };
    assert_eq!(
        Config::text_for_setup_patch(Some(SET), &completed_only).unwrap(),
        one_edit(SET, "setup_completed = false\n", "setup_completed = true\n")
    );
    // No `[sync]` yet: the table is appended after the last table.
    assert_eq!(
        Config::text_for_setup_patch(Some(BASE), &setup_patch()).unwrap(),
        format!(
            "{}\n[sync]\nserver_url = \"https://crew.example\"\ntoken = \"pasted-token\"\n",
            root_insert(BASE, "setup_completed = true\n")
        )
    );
}

/// `setup_completed = false` is written, not left out, when the key is
/// missing, and spliced when it is there.
#[test]
fn pin_setup_patch_writes_completed_false() {
    let not_done = SetupDiskPatch {
        setup_completed: Some(false),
        sync: None,
    };
    assert_eq!(
        Config::text_for_setup_patch(Some(BASE), &not_done).unwrap(),
        root_insert(BASE, "setup_completed = false\n")
    );
    let done = one_edit(SET, "setup_completed = false\n", "setup_completed = true\n");
    assert_eq!(
        Config::text_for_setup_patch(Some(&done), &not_done).unwrap(),
        SET
    );
}

/// A `[sync]` table without `token` does not parse as a config, so no save
/// path edits it and neither writer touches the file.
#[test]
fn pin_sync_table_missing_a_key_is_not_replaced() {
    let broken = format!("{BASE}\n[sync]\nserver_url = \"https://old.example\"\n");
    let prefix = "config.toml could not be parsed, so it was not replaced: ";
    let err = Config::text_for_setup_patch(Some(&broken), &setup_patch())
        .unwrap_err()
        .to_string();
    assert!(err.starts_with(prefix), "{err}");
    assert!(err.contains("missing field `token`"), "{err}");
    let err = Config::text_for_save(Some(&broken), &pinned_default())
        .unwrap_err()
        .to_string();
    assert!(err.starts_with(prefix), "{err}");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, &broken).expect("seed");
    assert!(Config::apply_setup_patch(&path, &setup_patch()).is_err());
    assert!(pinned_default().save_at(&path).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
}

/// The `data_dir` value of a serialized config and the text after its line.
/// The value's quoting depends on the path: toml 1 writes a path with a `"`
/// as `"""…"""`, and one with a newline as a multi-line string over
/// several lines. So the value is found by its parsed span, not by the
/// first line break.
fn split_data_dir(text: &str) -> (PathBuf, &str) {
    let doc = toml_edit::Document::parse(text).expect("serialized config parses");
    let value = doc
        .get("data_dir")
        .and_then(|item| item.as_value())
        .expect("a root data_dir value");
    let span = value.span().expect("a parsed value has a span");
    assert_eq!(
        &text[..span.start],
        "data_dir = ",
        "data_dir comes first:\n{text}"
    );
    let rest = text[span.end..]
        .strip_prefix('\n')
        .expect("a line break after the data_dir value");
    (PathBuf::from(value.as_str().expect("a string")), rest)
}

/// A missing file is the default config with the patch applied. `data_dir`
/// is the host's data dir, so it is checked by value, and only the text
/// after it is pinned byte for byte.
#[test]
fn pin_setup_patch_on_a_missing_file() {
    let fresh = Config::text_for_setup_patch(None, &setup_patch()).unwrap();
    let (data_dir, rest) = split_data_dir(&fresh);
    assert_eq!(data_dir, Config::default().data_dir, "{fresh}");
    // The same split works whatever the host path holds.
    for odd in [
        "/home/a\"b'c/share",
        "/home/a\nb/share",
        "/home/'''\"\"\"\\/share",
    ] {
        let odd = Config {
            data_dir: PathBuf::from(odd),
            ..Config::default()
        };
        let text = toml::to_string_pretty(&odd).unwrap();
        let (odd_dir, odd_rest) = split_data_dir(&text);
        assert_eq!(odd_dir, odd.data_dir, "{text}");
        assert_eq!(odd_rest, DEFAULT_TEXT.split_once('\n').unwrap().1, "{text}");
    }
    assert_eq!(
        rest,
        "\
session_window_secs = 1800
finished_game_close_secs = 180
game_process_names = [\"Overwatch.exe\"]
debug_ocr = false
setup_completed = true

[sync]
server_url = \"https://crew.example\"
token = \"pasted-token\"

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120
"
    );
}

/// The two writers that touch the disk write the same bytes as the text
/// functions above.
#[test]
fn pin_disk_writes_match_the_text() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");

    std::fs::write(&path, SET).expect("seed");
    let mut next = Config::parse_file_contents(SET).unwrap();
    next.reader = ReaderSetting::New;
    next.save_at(&path).expect("save");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        one_edit(SET, "reader = \"ocr-v1\" #", "reader = \"new\" #")
    );

    let base = base_with_sync();
    std::fs::write(&path, &base).expect("seed");
    Config::apply_setup_patch(&path, &setup_patch()).expect("setup patch");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        Config::text_for_setup_patch(Some(&base), &setup_patch()).unwrap()
    );
}

/// [`SET`] after [`setup_patch`]: `setup_completed` and both `[sync]`
/// strings spliced in place.
fn set_after_setup_patch() -> String {
    let text = one_edit(SET, "setup_completed = false\n", "setup_completed = true\n");
    let text = one_edit(
        &text,
        "server_url = \"https://old.example\"\n",
        "server_url = \"https://crew.example\"\n",
    );
    one_edit(
        &text,
        "token = \"old-token\"\n",
        "token = \"pasted-token\"\n",
    )
}

/// [`base_with_sync`] after [`setup_patch`]: `setup_completed` inserted on
/// the root, both `[sync]` strings spliced.
fn base_after_setup_patch() -> String {
    one_edit(
        &root_insert(&base_with_sync(), "setup_completed = true\n"),
        "server_url = \"https://old.example\"\ntoken = \"old-token\"\n",
        "server_url = \"https://crew.example\"\ntoken = \"pasted-token\"\n",
    )
}

/// CRLF line endings, as the code handles them today (captured, not
/// designed). An unchanged save returns the file as it is, and a splice
/// changes only the value bytes, so CRLF stays. An insert goes through
/// `toml_edit::DocumentMut`, which writes the whole file back with LF.
/// A full rewrite is LF.
#[test]
fn pin_crlf_files_through_the_in_place_paths() {
    let crlf = |text: &str| text.replace('\n', "\r\n");
    let base = crlf(&base_with_sync());
    let set = crlf(SET);
    assert_eq!(save(&base, |_| {}), base);
    assert_eq!(save(&set, |_| {}), set);
    // Splices keep CRLF.
    assert_eq!(
        save(&set, |c| c.shadow_recognizer = true),
        crlf(&one_edit(
            SET,
            "shadow_recognizer   =   false #",
            "shadow_recognizer   =   true #"
        ))
    );
    assert_eq!(
        save(&set, |c| c.reader = ReaderSetting::New),
        crlf(&one_edit(
            SET,
            "reader = \"ocr-v1\" #",
            "reader = \"new\" #"
        ))
    );
    assert_eq!(
        Config::text_for_setup_patch(Some(&set), &setup_patch()).unwrap(),
        crlf(&set_after_setup_patch())
    );
    // Inserts write the whole file back with LF.
    assert_eq!(
        save(&base, |c| c.shadow_recognizer = true),
        root_insert(&base_with_sync(), "shadow_recognizer = true\n")
    );
    assert_eq!(
        save(&base, |c| c.reader = ReaderSetting::OcrV1),
        root_insert(&base_with_sync(), "reader = \"ocr-v1\"\n")
    );
    assert_eq!(
        Config::text_for_setup_patch(Some(&base), &setup_patch()).unwrap(),
        base_after_setup_patch()
    );
    // A full rewrite is the usual LF text.
    assert_eq!(
        save(&base, |c| c.debug_ocr = true),
        save(&base_with_sync(), |c| c.debug_ocr = true)
    );
}

/// A UTF-8 byte-order mark, as the code handles it today (captured, not
/// designed). The file parses. An unchanged save and a splice keep the BOM.
/// An insert (`toml_edit::DocumentMut`) and a full rewrite drop it.
#[test]
fn pin_bom_files_through_the_in_place_paths() {
    let bom = |text: &str| format!("\u{feff}{text}");
    let base = bom(&base_with_sync());
    let set = bom(SET);
    assert_eq!(
        Config::parse_file_contents(&set).unwrap(),
        Config::parse_file_contents(SET).unwrap()
    );
    assert_eq!(save(&base, |_| {}), base);
    assert_eq!(save(&set, |_| {}), set);
    // Splices keep the BOM.
    assert_eq!(
        save(&set, |c| c.shadow_recognizer = true),
        bom(&one_edit(
            SET,
            "shadow_recognizer   =   false #",
            "shadow_recognizer   =   true #"
        ))
    );
    assert_eq!(
        save(&set, |c| c.reader = ReaderSetting::New),
        bom(&one_edit(
            SET,
            "reader = \"ocr-v1\" #",
            "reader = \"new\" #"
        ))
    );
    assert_eq!(
        Config::text_for_setup_patch(Some(&set), &setup_patch()).unwrap(),
        bom(&set_after_setup_patch())
    );
    // Inserts and a full rewrite drop it.
    assert_eq!(
        save(&base, |c| c.shadow_recognizer = true),
        root_insert(&base_with_sync(), "shadow_recognizer = true\n")
    );
    assert_eq!(
        Config::text_for_setup_patch(Some(&base), &setup_patch()).unwrap(),
        base_after_setup_patch()
    );
    assert_eq!(
        save(&base, |c| c.debug_ocr = true),
        save(&base_with_sync(), |c| c.debug_ocr = true)
    );
}
