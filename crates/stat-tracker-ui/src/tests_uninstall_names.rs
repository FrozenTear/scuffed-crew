//! The tracker data names "Remove saved games" deletes (`uninstall.rs`).

use crate::uninstall::is_tracker_data_name;

#[test]
fn store_backups_are_tracker_data() {
    for name in [
        "stats.surrealkv",
        "stats.surrealkv.pre-vacuum-20261028_223456",
        "stats.surrealkv.pre-timeline-20261029_101500",
        "stats.surrealkv.pre-timeline-20261029_101500.partial",
        "stats.surrealkv.pre-timeline-20261029_101500.pending",
        "matches.jsonl",
        "live_snapshot.json",
        "tracker.sock",
    ] {
        assert!(is_tracker_data_name(name), "{name}");
    }
    for name in ["keep-me.txt", "stats.surrealkv-other", "other-app"] {
        assert!(!is_tracker_data_name(name), "{name}");
    }
}

/// The shell uninstaller keeps the same list (dist/uninstall.sh).
#[test]
fn the_shell_uninstaller_removes_the_timeline_backups_too() {
    let sh = include_str!("../../stat-tracker/dist/uninstall.sh");
    // Both match every `stats.surrealkv.pre-*` backup (vacuum, hero
    // timeline, a half-done `.partial` copy and its `.pending` note).
    assert!(sh.contains("        stats.surrealkv.pre-*)\n"));
}

/// The training crop folder (`<data_dir>/training_crops/`: waiting crops,
/// game tokens, owner, consent, status and the delete marker) goes with
/// the saved games, in both uninstallers, and the shell prompt names it.
#[test]
fn both_uninstallers_remove_the_training_crops() {
    assert!(is_tracker_data_name("training_crops"));
    let sh = include_str!("../../stat-tracker/dist/uninstall.sh");
    let start = sh.find("tracker_data_name() {").expect("the shell list");
    let end = start + sh[start..].find("\n}\n").expect("end of the shell list");
    let list = &sh[start..end];
    let names: Vec<&str> = list
        .split(['|', ')', '\\', '\n', ' '])
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    assert!(names.contains(&"training_crops"), "{list}");
    let prompt = sh
        .lines()
        .find(|line| line.contains("Also delete local data"))
        .expect("the purge prompt");
    assert!(prompt.contains("training crops"), "{prompt}");
}
