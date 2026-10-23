//! Tests of the back-fill and manual-edit writers (`set_session_*`,
//! `edit_match`).

use super::tests::{mark_current, snap};
use super::*;

#[tokio::test]
async fn set_session_hero_writes_through_to_capture_rows() {
    // The 07-18 field case: majority-correct captures with wrong-hero
    // stragglers from post-match row browsing. The repair must reach the
    // personal_match rows (what the GUI and sync read), not just the
    // session row, and must flip only mismatched rows to synced=false.
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    for i in 0..3 {
        let mut m = snap("s1", i);
        if i < 2 {
            m.hero = "Mizuki".into();
            m.role = "Support".into();
        } else {
            m.hero = "Tracer".into();
            m.role = "Damage".into();
        }
        store.insert_match(m).await.unwrap();
    }
    // Everything already synced, as after a normal upload cycle.
    mark_current(&store).await;

    store
        .set_session_hero("s1", "Mizuki", "Support")
        .await
        .unwrap();

    let rows = store.get_all_matches().await.unwrap();
    assert!(
        rows.iter()
            .all(|m| m.hero == "Mizuki" && m.role == "Support")
    );
    // Only the repaired (previously Tracer) row is queued for re-sync.
    assert_eq!(rows.iter().filter(|m| !m.synced).count(), 1);
}

#[tokio::test]
async fn edit_match_overlays_corrections_and_preserves_ocr() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    // Two snapshots of one game, both already synced (post-upload state).
    for i in 0..2 {
        let mut m = snap("s1", 5 + i); // ocr elims 5, 6
        append_match_log(dir.path(), &m);
        m.map_name = "Ilios".into();
        store.insert_match(m).await.unwrap();
    }
    mark_current(&store).await;

    let edit = MatchEdit {
        elims: Some(30),
        hero: Some("Ana".into()),
        map_name: Some("Nepal".into()),
        ..Default::default()
    };
    store.edit_match("s1", &edit).await.unwrap();

    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows.len(), 2);
    for m in &rows {
        // Corrected values overlay; OCR reads stay immutable underneath.
        assert_eq!(m.corrected_elims, Some(30));
        assert_eq!(m.display_elims(), 30);
        assert!(m.elims == 5 || m.elims == 6, "OCR elims preserved");
        assert_eq!(m.display_hero(), "Ana");
        assert_eq!(m.hero, "Test", "OCR hero preserved");
        assert_eq!(m.display_map_name(), "Nepal");
        assert!(m.is_edited());
        assert!(m.field_edited("elims") && m.field_edited("hero") && m.field_edited("map_name"));
        assert!(m.edited_at.is_some());
        // Re-queued for sync.
        assert!(!m.synced);
    }

    // jsonl fallback carries the same overlay (rewrite parity).
    let logged = read_match_log(dir.path());
    assert_eq!(logged.len(), 2);
    assert!(logged.iter().all(|m| m.display_elims() == 30
        && m.display_hero() == "Ana"
        && m.elims != 30
        && m.is_edited()));
}

#[tokio::test]
async fn picked_map_and_hero_clear_the_upload_hold() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    let mut m = snap("s1", 4);
    m.hero = "Unknown".into();
    m.role = "Damage".into();
    m.map_name.clear();
    m.game_mode.clear();
    m.outcome = "victory".into();
    store.insert_match(m).await.unwrap();

    let before = store.get_unsynced().await.unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(
        crate::parse::review_suspect_fields(
            before[0].display_map_name(),
            &before[0].game_mode,
            before[0].display_hero()
        ),
        vec!["map", "mode", "hero"]
    );
    assert!(crate::sync::upload_request(&before, &[]).matches.is_empty());

    store
        .edit_match(
            "s1",
            &MatchEdit {
                map_name: Some("King's Row".into()),
                hero: Some("Ana".into()),
                ..MatchEdit::default()
            },
        )
        .await
        .unwrap();

    let after = store.get_all_matches().await.unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].display_map_name(), "King's Row");
    assert_eq!(after[0].game_mode, "Hybrid");
    assert_eq!(after[0].display_hero(), "Ana");
    assert_eq!(after[0].display_role(), "Support");
    assert_eq!(after[0].hero, "Unknown", "OCR hero stays on the row");
    assert!(after[0].map_name.is_empty(), "OCR map stays on the row");
    assert!(!crate::sync::row_needs_review(&after[0]));
    let uploaded = crate::sync::upload_request(&after, &[]);
    assert_eq!(uploaded.matches.len(), 1);
    assert_eq!(uploaded.matches[0].map_name, "King's Row");
    assert_eq!(uploaded.matches[0].game_mode, "Hybrid");
    assert_eq!(uploaded.matches[0].hero, "Ana");
    let body = serde_json::to_string(&uploaded).unwrap();
    assert!(!body.contains("\"map_name\":\"\""));
    assert!(!body.contains("\"game_mode\":\"\""));
    assert!(!body.contains("Unknown"));
}

#[tokio::test]
async fn set_session_hero_rewrites_match_log() {
    // HW-1a: the hero write-through must also reach matches.jsonl, exactly
    // like the outcome/map back-fills. Without the rewrite the fallback log
    // keeps the stale hero forever — this test fails.
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    for i in 0..3 {
        let mut m = snap("s1", i);
        m.hero = if i < 2 {
            "Mizuki".into()
        } else {
            "Tracer".into()
        };
        m.role = if i < 2 {
            "Support".into()
        } else {
            "Damage".into()
        };
        append_match_log(dir.path(), &m);
        store.insert_match(m).await.unwrap();
    }

    store
        .set_session_hero("s1", "Mizuki", "Support")
        .await
        .unwrap();

    let logged = read_match_log(dir.path());
    assert_eq!(logged.len(), 3);
    assert!(
        logged
            .iter()
            .all(|m| m.hero == "Mizuki" && m.role == "Support"),
        "hero repair must reach the jsonl fallback (HW-1a)"
    );
}

/// F252-5: a map change re-queues the session's rows for upload.
#[tokio::test]
async fn set_session_map_unsyncs_rows_when_the_map_changes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    let mut row = snap("s1", 5);
    row.map_name = "Busan".into();
    row.outcome = "victory".into();
    store.insert_match(row).await.unwrap();
    let claims = SyncClaim::capture(&store.get_unsynced().await.unwrap());
    store.mark_synced(&claims).await.unwrap();
    assert!(store.get_all_matches().await.unwrap()[0].synced);
    store.set_session_map("s1", "Ilios").await.unwrap();
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows[0].map_name, "Ilios");
    assert!(!rows[0].synced, "a renamed row is uploaded again");
}

/// F252-3: a session rename keeps a member's own map choice: the
/// correction and its edited mark stay.
#[tokio::test]
async fn set_session_map_keeps_a_member_map_edit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    let mut row = snap("s1", 5);
    row.map_name = "Busan".into();
    store.insert_match(row).await.unwrap();
    store
        .apply_command(&StoreCommand::EditMatch {
            session_id: "s1".into(),
            edit: MatchEdit {
                map_name: Some("King's Row".into()),
                ..Default::default()
            },
        })
        .await
        .unwrap();
    store.set_session_map("s1", "Ilios").await.unwrap();
    let rows = store.get_session_snapshots("s1").await.unwrap();
    assert_eq!(rows[0].corrected_map_name.as_deref(), Some("King's Row"));
    assert!(rows[0].field_edited("map_name"));
    assert_eq!(rows[0].display_map_name(), "King's Row");
}
