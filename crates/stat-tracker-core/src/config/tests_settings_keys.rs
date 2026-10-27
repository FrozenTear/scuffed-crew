//! The 0.6 Settings keys: a 0.5.0 file loads with their defaults and saves
//! back byte for byte, each key round-trips, and a save that sets one keeps
//! the rest of a hand-edited file as it was.

use super::*;

/// A config as 0.5.0 Settings wrote it, with a comment and an unknown key.
const V050: &str = "\
# my tracker
data_dir = \"/keys/data\"
capture_output = \"DP-3\"
player_name = \"FrozenTear\"
session_window_secs = 10800
finished_game_close_secs = 180
game_process_names = [\"Overwatch.exe\"]
debug_ocr = false
from_a_newer_build = 1

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120
";

fn save(existing: &str, edit: impl FnOnce(&mut Config)) -> (String, Config) {
    let mut next = Config::parse_file_contents(existing).expect("fixture parses");
    edit(&mut next);
    let text = Config::text_for_save(Some(existing), &next).expect("save text");
    let back = Config::parse_file_contents(&text).expect("saved text parses");
    assert_eq!(back, next, "saved text reads back as the saved config");
    (text, back)
}

#[test]
fn old_config_loads_with_0_6_defaults() {
    let c = Config::parse_file_contents(V050).expect("0.5.0 file parses");
    assert!(c.player_names.is_empty());
    assert!(c.live_upload);
    assert!(!c.rank_reader);
    assert!(!c.share_stat_pictures);
    assert!(!c.share_report_pictures);
    assert!(c.capture_only_on_screen);
    let d = Config::default();
    assert_eq!(c.player_names, d.player_names);
    assert_eq!(c.live_upload, d.live_upload);
    assert_eq!(c.rank_reader, d.rank_reader);
    assert_eq!(c.share_stat_pictures, d.share_stat_pictures);
    assert_eq!(c.share_report_pictures, d.share_report_pictures);
    assert_eq!(c.capture_only_on_screen, d.capture_only_on_screen);
}

#[test]
fn old_config_saves_back_unchanged() {
    let (text, _) = save(V050, |_| {});
    assert_eq!(text, V050);
}

/// A real edit goes past `text_for_save`'s nothing-changed return: the
/// edited line is the only one that changes, and no 0.6 key is written.
#[test]
fn old_config_edit_changes_only_the_edited_line() {
    let (text, _) = save(V050, |c| c.player_name = Some("FrozenAlt".into()));
    let changed: Vec<(&str, &str)> = V050
        .lines()
        .zip(text.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(
        changed,
        vec![(
            "player_name = \"FrozenTear\"",
            "player_name = \"FrozenAlt\""
        )]
    );
    assert_eq!(text.lines().count(), V050.lines().count(), "no line added");
    assert!(text.starts_with("# my tracker\n"), "comment kept");
    assert!(text.contains("from_a_newer_build = 1"), "unknown key kept");
}

/// A line the saved file must hold, and the edit that should write it.
type KeyEdit = (&'static str, fn(&mut Config));

#[test]
fn each_key_round_trips_off_its_default() {
    let edits: [KeyEdit; 6] = [
        ("player_names", |c| {
            c.player_names = vec!["FrozenAlt".into()]
        }),
        ("live_upload = false", |c| c.live_upload = false),
        ("rank_reader = true", |c| c.rank_reader = true),
        ("share_stat_pictures = true", |c| {
            c.share_stat_pictures = true
        }),
        ("share_report_pictures = true", |c| {
            c.share_report_pictures = true
        }),
        ("capture_only_on_screen = false", |c| {
            c.capture_only_on_screen = false
        }),
    ];
    for (line, edit) in edits {
        let (text, _) = save(V050, edit);
        assert!(text.contains(line), "{line} missing:\n{text}");
        assert!(text.starts_with("# my tracker\n"), "comment kept");
        assert!(text.contains("from_a_newer_build = 1"), "unknown key kept");
        // Back to the default removes the line again, leaving the 0.5.0 text.
        let (again, _) = save(&text, |c| {
            let d = Config::default();
            c.player_names = d.player_names;
            c.live_upload = d.live_upload;
            c.rank_reader = d.rank_reader;
            c.share_stat_pictures = d.share_stat_pictures;
            c.share_report_pictures = d.share_report_pictures;
            c.capture_only_on_screen = d.capture_only_on_screen;
        });
        assert_eq!(again, V050, "{line}: back to default");
    }
}

#[test]
fn explicit_default_values_load() {
    let text = V050.replace(
        "debug_ocr = false\n",
        "debug_ocr = false\nlive_upload = true\nrank_reader = false\nplayer_names = []\n",
    );
    let c = Config::parse_file_contents(&text).expect("parses");
    assert!(c.live_upload && !c.rank_reader && c.player_names.is_empty());
}

#[test]
fn own_player_names_merges_trims_and_dedupes() {
    let mut c = Config::default();
    assert!(c.own_player_names().is_empty());
    c.player_name = Some(" FrozenTear ".into());
    c.player_names = vec![
        "frozentear".into(),
        "  ".into(),
        "FrozenAlt".into(),
        "FrozenAlt".into(),
    ];
    assert_eq!(c.own_player_names(), vec!["FrozenTear", "FrozenAlt"]);
    c.player_name = None;
    assert_eq!(c.own_player_names(), vec!["frozentear", "FrozenAlt"]);
}

#[test]
fn own_player_names_dedupes_past_ascii() {
    let c = Config {
        player_name: Some("Öster".into()),
        player_names: vec![
            "öster".into(),
            "ÉCLAIR".into(),
            "éclair".into(),
            "Østen".into(),
        ],
        ..Config::default()
    };
    assert_eq!(c.own_player_names(), vec!["Öster", "ÉCLAIR", "Østen"]);
}

#[test]
fn own_player_names_drop_the_battletag_number() {
    let c = Config {
        player_name: Some("QXSelf#24680".into()),
        player_names: vec!["QXSelf".into(), "Alt#12".into(), "Tag#x1".into()],
        ..Config::default()
    };
    assert_eq!(c.own_player_names(), vec!["QXSelf", "Alt", "Tag#x1"]);
}

#[test]
fn own_player_names_drop_names_too_short_to_match() {
    let c = Config {
        player_name: Some("Ab".into()),
        player_names: vec!["x".into(), "Öst".into(), "Zo#1234".into()],
        ..Config::default()
    };
    assert_eq!(c.own_player_names(), vec!["Öst"]);
}
