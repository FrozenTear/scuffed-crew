//! The general in-place save (`merge::edit_in_place`): any set of changes
//! keeps the file's comments, key order and unknown keys, and the text reads
//! back as exactly the config that was saved.

use super::*;

const HAND: &str = "\
# top comment
data_dir = \"/merge/data\" # the store

# who you are
player_name = \"FrozenTear\"
capture_output = \"DP-1\" # main screen
screencast_max_fps = 3
game_process_names = [\"Overwatch.exe\"]
mystery_key = \"from a newer build\"

[auto_detect]
# poll settings
enabled = true # watch
poll_interval_secs = 4
cooldown_secs = 120
unknown_poll_key = 7
";

const COMMENTS: [&str; 6] = [
    "# top comment",
    "# the store",
    "# who you are",
    "# main screen",
    "# poll settings",
    "# watch",
];

/// Save `edit` on `existing` and check the text reads back as the config.
fn save(existing: &str, edit: impl FnOnce(&mut Config)) -> (String, Config) {
    let mut next = Config::parse_file_contents(existing).expect("fixture parses");
    edit(&mut next);
    let text = Config::text_for_save(Some(existing), &next).expect("save text");
    assert_eq!(
        Config::parse_file_contents(&text).expect("saved text parses"),
        next,
        "the saved text reads back as exactly the saved config:\n{text}"
    );
    (text, next)
}

fn assert_kept(text: &str, lines: &[&str]) {
    for line in lines {
        assert!(text.contains(line), "{line:?} must survive:\n{text}");
    }
}

/// One named change to apply to a parsed config.
type NamedEdit = (&'static str, Box<dyn Fn(&mut Config)>);

#[test]
fn comments_survive_every_kind_of_change() {
    let edits: Vec<NamedEdit> = vec![
        (
            "replace root values",
            Box::new(|c| {
                c.player_name = Some("Ada".to_string());
                c.screencast_max_fps = Some(2);
            }),
        ),
        (
            "insert root keys",
            Box::new(|c| {
                c.debug_ocr = true;
                c.ocr_threads = Some(2);
            }),
        ),
        (
            "change table keys",
            Box::new(|c| {
                c.auto_detect.enabled = false;
                c.auto_detect.cooldown_secs = 45;
            }),
        ),
        (
            "insert a table key",
            Box::new(|c| {
                c.auto_detect.poll_interval_ms = Some(900);
                c.session_window_secs = 600;
            }),
        ),
        (
            "remove keys",
            Box::new(|c| {
                c.screencast_max_fps = None;
                c.debug_ocr = true;
            }),
        ),
        (
            "replace an array",
            Box::new(|c| {
                c.game_process_names = vec!["Overwatch.exe".to_string(), "ow2.exe".to_string()];
                c.debug_ocr = true;
            }),
        ),
        (
            "add a sync table",
            Box::new(|c| {
                c.sync = Some(SyncConfig {
                    server_url: "https://crew.example".to_string(),
                    token: "pasted".to_string(),
                });
                c.debug_ocr = true;
            }),
        ),
    ];
    for (name, edit) in edits {
        let (text, _) = save(HAND, |c| edit(c));
        assert_kept(&text, &COMMENTS);
        assert_kept(&text, &["mystery_key", "unknown_poll_key = 7"]);
        assert!(
            text.starts_with("# top comment\ndata_dir = \"/merge/data\" # the store\n"),
            "{name}: the head of the file stays:\n{text}"
        );
    }
}

#[test]
fn a_replaced_value_keeps_its_line_and_trailing_comment() {
    let (text, _) = save(HAND, |c| {
        c.capture_output = Some("HDMI-1".to_string());
        c.auto_detect.enabled = false;
    });
    assert_eq!(
        text,
        HAND.replacen(
            "capture_output = \"DP-1\" # main screen",
            "capture_output = \"HDMI-1\" # main screen",
            1
        )
        .replacen("enabled = true # watch", "enabled = false # watch", 1)
    );
}

#[test]
fn unknown_keys_keep_their_place() {
    let (text, _) = save(HAND, |c| {
        c.player_name = None;
        c.auto_detect.cooldown_secs = 30;
    });
    let mystery = text.find("mystery_key").expect("root unknown key");
    let table = text.find("[auto_detect]").expect("table");
    let poll = text
        .find("unknown_poll_key = 7")
        .expect("table unknown key");
    assert!(mystery < table && table < poll, "{text}");
}

#[test]
fn a_field_the_struct_no_longer_writes_is_removed() {
    let (text, next) = save(HAND, |c| {
        c.capture_output = None;
        c.screencast_max_fps = None;
    });
    assert!(!text.contains("capture_output"), "{text}");
    assert!(!text.contains("screencast_max_fps"), "{text}");
    assert_eq!(next.capture_output, None);
    assert_kept(&text, &["# top comment", "# who you are", "mystery_key"]);
}

#[test]
fn a_new_table_is_added_whole_when_the_file_has_none() {
    let existing = "# only a comment\ndata_dir = \"/merge/data\"\n";
    let (text, _) = save(existing, |c| {
        c.auto_detect.enabled = false;
        c.debug_ocr = true;
    });
    assert!(text.starts_with(existing), "{text}");
    assert!(text.contains("[auto_detect]"), "{text}");
    assert!(text.contains("cooldown_secs = 120"), "{text}");
}

#[test]
fn a_token_less_sync_url_survives_an_unrelated_change() {
    let existing =
        format!("{HAND}\n[sync]\n# paste the token later\nserver_url = \"https://crew.example\"\n");
    let (text, next) = save(&existing, |c| {
        c.debug_ocr = true;
        c.player_name = Some("Ada".to_string());
    });
    assert_eq!(next.sync, None);
    assert_kept(
        &text,
        &[
            "[sync]\n# paste the token later\nserver_url = \"https://crew.example\"\n",
            "debug_ocr = true",
        ],
    );
}

#[test]
fn turning_sync_on_fills_a_token_less_table_in_place() {
    let existing =
        format!("{HAND}\n[sync]\n# paste the token later\nserver_url = \"https://crew.example\"\n");
    let (text, _) = save(&existing, |c| {
        c.sync = Some(SyncConfig {
            server_url: "https://crew.example".to_string(),
            token: "pasted".to_string(),
        });
        c.debug_ocr = true;
    });
    assert_kept(
        &text,
        &[
            "# paste the token later\nserver_url = \"https://crew.example\"\n",
            "token = \"pasted\"",
        ],
    );
    assert_eq!(text.matches("[sync]").count(), 1, "{text}");
}

#[test]
fn turning_sync_off_removes_only_the_keys_settings_wrote() {
    let existing = format!(
        "{HAND}\n[sync]\nserver_url = \"https://crew.example\"\ntoken = \"tok\"\nnote = \"mine\"\n"
    );
    let (text, next) = save(&existing, |c| {
        c.sync = None;
        c.debug_ocr = true;
    });
    assert_eq!(next.sync, None);
    assert!(!text.contains("token = \"tok\""), "{text}");
    assert_kept(&text, &["[sync]", "note = \"mine\""]);
}

#[test]
fn a_removed_key_leaves_the_comments_above_it() {
    // A root key: its comment moves onto the next key.
    let (text, _) = save(HAND, |c| c.player_name = None);
    assert!(!text.contains("player_name"), "{text}");
    assert_kept(
        &text,
        &["\n# who you are\ncapture_output = \"DP-1\" # main screen\n"],
    );

    // The first key of the file: the head comment stays at the top.
    let head = "# top\nplayer_name = \"Ada\"\ndata_dir = \"/merge/data\"\n";
    let (text, _) = save(head, |c| c.player_name = None);
    assert!(
        text.starts_with("# top\ndata_dir = \"/merge/data\"\n"),
        "{text}"
    );

    // A key inside a table.
    let table = "\
data_dir = \"/merge/data\"

[auto_detect]
enabled = true
# how often
poll_interval_secs = 4
cooldown_secs = 120
";
    let (text, next) = save(table, |c| c.auto_detect.poll_interval_secs = None);
    assert_eq!(next.auto_detect.poll_interval_secs, None);
    assert!(!text.contains("poll_interval_secs"), "{text}");
    assert_kept(
        &text,
        &["enabled = true\n# how often\ncooldown_secs = 120\n"],
    );

    // The last root key: the comment goes onto the table header after it.
    let last = "\
data_dir = \"/merge/data\"
# the screen
capture_output = \"DP-1\"

[auto_detect]
enabled = true
";
    let (text, _) = save(last, |c| c.capture_output = None);
    assert!(!text.contains("capture_output"), "{text}");
    assert_kept(&text, &["# the screen\n\n[auto_detect]"]);
}
