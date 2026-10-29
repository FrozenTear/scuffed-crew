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
