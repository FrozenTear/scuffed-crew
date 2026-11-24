//! The direct children of the saved-games folder that "Remove saved games"
//! deletes (`uninstall.rs`). The shell uninstaller keeps the same list
//! (`tracker_data_name` in `dist/uninstall.sh`).

/// Files and folders the tracker writes in its saved-games folder (`stats.surrealkv.pre-*`:
/// store backups, `training_crops`: the opt-in training crop outbox and its
/// consent, owner and status files). Anything else stays, also in
/// `~/.config`, `~/Documents` or `~/Games`. Never add `templates` or
/// `templates-kept`: reader packs and the folders sync kept aside stay, and
/// the member deletes `templates-kept` by hand (both lists skip them).
pub(crate) fn is_tracker_data_name(name: &str) -> bool {
    matches!(
        name,
        "stats.surrealkv"
            | "vacuum.tmp"
            | "vacuum.last-size"
            | "live_snapshot.json"
            | "live_snapshot.json.tmp"
            | "matches.jsonl"
            | "commands"
            | "daemon.pid"
            | "daemon.log"
            | "daemon.log.1"
            | "debug"
            | "shadow"
            | "tessdata"
            | "active_game.json"
            | "sync_auth.json"
            | "portraits"
            | "ui_state.json"
            | "report-kit"
            | "run.marker"
            | "last_crash.marker"
            | "ocr_switch.json"
            | "ocr_switch.json.tmp"
            | "update-install.log"
            | "tracker.sock"
            | "training_crops"
    ) || name.starts_with("stats.surrealkv.pre-")
}
