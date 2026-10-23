//! Tests of the on-open pack-key hero migration.

use super::tests::snap;
use super::*;
use chrono::Utc;

#[tokio::test]
async fn pack_key_migration_rewrites_heroes_once_and_skips_unchanged_uploads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");

    let mut pack_key = snap("pack", 11);
    pack_key.hero = "wrecking-ball".into();
    pack_key.role = "Damage".into();
    pack_key.synced = true;
    pack_key.sync_rev = 4;
    store.insert_match(pack_key).await.unwrap();

    let mut canonical = snap("canonical", 3);
    canonical.hero = "Wrecking Ball".into();
    canonical.role = "Tank".into();
    canonical.synced = true;
    canonical.sync_rev = 2;
    store.insert_match(canonical).await.unwrap();

    let mut wrong_role = snap("wrong-role", 6);
    wrong_role.hero = "Wrecking Ball".into();
    wrong_role.role = "Damage".into();
    wrong_role.synced = true;
    wrong_role.sync_rev = 3;
    store.insert_match(wrong_role).await.unwrap();

    let mut other = snap("other", 1);
    other.hero = "Ana".into();
    other.role = "Support".into();
    other.synced = true;
    other.sync_rev = 1;
    store.insert_match(other).await.unwrap();

    let mut pinned = snap("pinned", 8);
    pinned.hero = "wrecking-ball".into();
    pinned.role = "Damage".into();
    pinned.corrected_hero = Some("Ana".into());
    pinned.corrected_role = Some("Support".into());
    pinned.synced = true;
    pinned.sync_rev = 7;
    store.insert_match(pinned).await.unwrap();

    store
        .create_session(&MatchSession {
            session_id: "pack".into(),
            hero: "wrecking-ball".into(),
            map_name: String::new(),
            role: "Damage".into(),
            started_at: SurrealDatetime::from(Utc::now()),
            last_capture_at: SurrealDatetime::from(Utc::now()),
            capture_count: 1,
            final_outcome: "victory".into(),
        })
        .await
        .unwrap();
    store
        .create_session(&MatchSession {
            session_id: "wrong-role".into(),
            hero: "Wrecking Ball".into(),
            map_name: String::new(),
            role: "Damage".into(),
            started_at: SurrealDatetime::from(Utc::now()),
            last_capture_at: SurrealDatetime::from(Utc::now()),
            capture_count: 1,
            final_outcome: "victory".into(),
        })
        .await
        .unwrap();

    store.migrate_pack_key_heroes().await.unwrap();

    let rows = store.get_all_matches().await.unwrap();
    let pack = rows.iter().find(|row| row.session_id == "pack").unwrap();
    assert_eq!(pack.hero, "Wrecking Ball");
    assert_eq!(pack.role, "Tank");
    assert!(!pack.synced, "a changed hero is queued for upload");
    assert_eq!(pack.sync_rev, 5);

    let canonical = rows
        .iter()
        .find(|row| row.session_id == "canonical")
        .unwrap();
    assert_eq!(canonical.hero, "Wrecking Ball");
    assert!(canonical.synced);
    assert_eq!(canonical.sync_rev, 2);

    let wrong_role = rows
        .iter()
        .find(|row| row.session_id == "wrong-role")
        .unwrap();
    assert_eq!(wrong_role.hero, "Wrecking Ball");
    assert_eq!(wrong_role.role, "Tank");
    assert!(!wrong_role.synced, "a corrected role is queued");
    assert_eq!(wrong_role.sync_rev, 4);

    let other = rows.iter().find(|row| row.session_id == "other").unwrap();
    assert_eq!(other.hero, "Ana");
    assert_eq!(other.role, "Support");
    assert!(other.synced);
    assert_eq!(other.sync_rev, 1);

    let pinned = rows.iter().find(|row| row.session_id == "pinned").unwrap();
    assert_eq!(pinned.hero, "Wrecking Ball");
    assert_eq!(pinned.role, "Tank");
    assert_eq!(pinned.display_hero(), "Ana");
    assert!(pinned.synced, "upload hero did not change");
    assert_eq!(pinned.sync_rev, 7);

    let session = store
        .get_all_sessions()
        .await
        .unwrap()
        .into_iter()
        .find(|session| session.session_id == "pack")
        .unwrap();
    assert_eq!(session.hero, "Wrecking Ball");
    assert_eq!(session.role, "Tank");
    let wrong_session = store
        .get_all_sessions()
        .await
        .unwrap()
        .into_iter()
        .find(|session| session.session_id == "wrong-role")
        .unwrap();
    assert_eq!(wrong_session.hero, "Wrecking Ball");
    assert_eq!(wrong_session.role, "Tank");

    store.migrate_pack_key_heroes().await.unwrap();
    let rows = store.get_all_matches().await.unwrap();
    let pack = rows.iter().find(|row| row.session_id == "pack").unwrap();
    assert_eq!(pack.hero, "Wrecking Ball");
    assert_eq!(pack.role, "Tank");
    assert!(!pack.synced);
    assert_eq!(
        pack.sync_rev, 5,
        "a second run must not queue the game again"
    );
    let canonical = rows
        .iter()
        .find(|row| row.session_id == "canonical")
        .unwrap();
    assert!(canonical.synced);
    assert_eq!(canonical.sync_rev, 2);
    let wrong_role = rows
        .iter()
        .find(|row| row.session_id == "wrong-role")
        .unwrap();
    assert_eq!(wrong_role.role, "Tank");
    assert!(!wrong_role.synced);
    assert_eq!(
        wrong_role.sync_rev, 4,
        "a second run must not queue the role again"
    );
    let pinned = rows.iter().find(|row| row.session_id == "pinned").unwrap();
    assert!(pinned.synced);
    assert_eq!(pinned.sync_rev, 7);
}

/// Two passes over the same saved games. The first fixes pack keys and
/// queues only those games. The second leaves every row and the queue alone.
#[tokio::test]
async fn hero_name_migration_second_run_queues_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");

    let games = [
        ("sess-ball", "wrecking-ball", "Damage", true),
        ("sess-76", "soldier-76", "Support", true),
        ("sess-dva", "dva", "Damage", true),
        ("sess-ana", "Ana", "Support", false),
        ("sess-ball-ok", "Wrecking Ball", "Tank", false),
    ];
    for (session_id, hero, role, _) in games {
        let mut row = snap(session_id, 4);
        row.hero = hero.into();
        row.role = role.into();
        row.synced = true;
        row.sync_rev = 3;
        store.insert_match(row).await.unwrap();
    }
    assert!(
        store.get_unsynced().await.unwrap().is_empty(),
        "synced games are not on the upload queue"
    );

    let before = store.get_all_matches().await.unwrap();
    let before_ids: std::collections::BTreeMap<_, _> = before
        .iter()
        .map(|row| (row.session_id.clone(), row.id.clone()))
        .collect();
    assert_eq!(before_ids.len(), games.len());

    store.migrate_pack_key_heroes().await.unwrap();

    let fixed = store.get_all_matches().await.unwrap();
    assert_eq!(fixed.len(), games.len());
    let expect = [
        ("sess-ball", "Wrecking Ball", "Tank"),
        ("sess-76", "Soldier: 76", "Damage"),
        ("sess-dva", "D.Va", "Tank"),
        ("sess-ana", "Ana", "Support"),
        ("sess-ball-ok", "Wrecking Ball", "Tank"),
    ];
    for (session_id, hero, role) in expect {
        let row = fixed
            .iter()
            .find(|row| row.session_id == session_id)
            .unwrap_or_else(|| panic!("{session_id} missing"));
        assert_eq!(row.session_id, session_id);
        assert_eq!(row.id, before_ids[session_id]);
        assert_eq!(row.hero, hero);
        assert_eq!(row.role, role);
    }

    let mut queued: Vec<_> = store
        .get_unsynced()
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.session_id)
        .collect();
    queued.sort();
    assert_eq!(
        queued,
        vec![
            "sess-76".to_string(),
            "sess-ball".to_string(),
            "sess-dva".to_string()
        ],
        "only games whose hero changed are queued"
    );
    for session_id in ["sess-ana", "sess-ball-ok"] {
        let row = fixed
            .iter()
            .find(|row| row.session_id == session_id)
            .unwrap();
        assert!(row.synced, "{session_id} must stay off the queue");
        assert_eq!(row.sync_rev, 3);
    }

    let first_pass: Vec<_> = fixed
        .iter()
        .map(|row| {
            (
                row.session_id.clone(),
                row.id.clone(),
                row.hero.clone(),
                row.role.clone(),
                row.synced,
                row.sync_rev,
            )
        })
        .collect();

    store.migrate_pack_key_heroes().await.unwrap();

    let second = store.get_all_matches().await.unwrap();
    let second_pass: Vec<_> = second
        .iter()
        .map(|row| {
            (
                row.session_id.clone(),
                row.id.clone(),
                row.hero.clone(),
                row.role.clone(),
                row.synced,
                row.sync_rev,
            )
        })
        .collect();
    assert_eq!(
        second_pass, first_pass,
        "the second run must change nothing"
    );

    let mut queued_again: Vec<_> = store
        .get_unsynced()
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.session_id)
        .collect();
    queued_again.sort();
    assert_eq!(queued_again, queued, "the second run must queue nothing");
}
