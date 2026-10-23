//! Tests of the `matches.jsonl` rewrite.

use super::tests::snap;
use super::*;

#[test]
fn match_log_rewrite_backfills_and_deletes() {
    let dir = tempfile::tempdir().expect("tempdir");
    append_match_log(dir.path(), &snap("keep", 1));
    append_match_log(dir.path(), &snap("fix", 1));
    append_match_log(dir.path(), &snap("fix", 2));
    append_match_log(dir.path(), &snap("gone", 1));

    rewrite_match_log_session(dir.path(), "fix", Some(&|m| m.outcome = "defeat".into()));
    rewrite_match_log_session(dir.path(), "gone", None);

    let rows = read_match_log(dir.path());
    assert_eq!(rows.len(), 3);
    assert!(!rows.iter().any(|m| m.session_id == "gone"));
    assert!(
        rows.iter()
            .filter(|m| m.session_id == "fix")
            .all(|m| m.outcome == "defeat")
    );
    assert!(
        rows.iter()
            .filter(|m| m.session_id == "keep")
            .all(|m| m.outcome == "victory")
    );
}
