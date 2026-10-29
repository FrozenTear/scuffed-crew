//! The one-per-game hero timeline (`hero_timeline`) and its additive move.
//! Numbers in the test names follow plan v2.1 section 5.1.

use super::migrate_timeline::{PRE_TIMELINE_BACKUP_PREFIX, TimelineBackup};
use super::tests::snap;
use super::*;
use chrono::{TimeZone, Utc};

fn at(secs: i64) -> SurrealDatetime {
    SurrealDatetime::from(Utc.timestamp_opt(1_800_000_000 + secs, 0).unwrap())
}

fn cap(session_id: &str, hero: &str, secs: i64) -> PersonalMatch {
    let mut m = snap(session_id, secs as u32);
    m.hero = hero.into();
    m.role = crate::parse::guess_role_public(hero);
    m.played_at = at(secs);
    m.map_name = "Busan".into();
    m.game_mode = "Control".into();
    m.synced = true;
    m.name_recognizer = Some("names-v1".into());
    m
}

fn session(session_id: &str) -> MatchSession {
    MatchSession {
        session_id: session_id.into(),
        hero: "Ana".into(),
        map_name: "Busan".into(),
        role: "Support".into(),
        started_at: at(0),
        last_capture_at: at(0),
        capture_count: 1,
        final_outcome: "victory".into(),
    }
}

/// An alpha.1 store: per-row timelines stamped by the old write path.
async fn alpha1_store(dir: &std::path::Path) -> LocalStore {
    let store = LocalStore::open(dir).await.unwrap();
    assert!(!store.timeline_migrated());
    // A to B to A swap.
    for (i, h) in ["Ana", "Ana", "Kiriko", "Kiriko", "Ana", "Ana"]
        .iter()
        .enumerate()
    {
        store.insert_match(cap("swap", h, i as i64)).await.unwrap();
    }
    store.create_session(&session("swap")).await.unwrap();
    store.refresh_session_hero_timeline("swap").await.unwrap();
    // A flap that was dismissed, then a confirmed one.
    for (i, h) in ["Mercy", "Mercy", "Lucio", "Mercy", "Mercy", "Zen", "Mercy"]
        .iter()
        .enumerate()
    {
        store
            .insert_match(cap("res", h, 100 + i as i64))
            .await
            .unwrap();
    }
    store.create_session(&session("res")).await.unwrap();
    store.refresh_session_hero_timeline("res").await.unwrap();
    store
        .resolve_hero_segment("res", 1, SEGMENT_DISMISS)
        .await
        .unwrap();
    store
        .resolve_hero_segment("res", 3, SEGMENT_CONFIRM)
        .await
        .unwrap();
    // A legacy game: rows never got a timeline.
    for i in 0..3 {
        store
            .insert_match(cap(
                "legacy",
                if i == 2 { "Genji" } else { "Tracer" },
                200 + i,
            ))
            .await
            .unwrap();
    }
    store.create_session(&session("legacy")).await.unwrap();
    // A pinned hero.
    for i in 0..3 {
        let mut m = cap("pinned", "Reinhardt", 300 + i);
        m.corrected_hero = Some("Sigma".into());
        store.insert_match(m).await.unwrap();
    }
    store.create_session(&session("pinned")).await.unwrap();
    store.refresh_session_hero_timeline("pinned").await.unwrap();
    // Blank session rows and an orphan keep their own copy.
    store.insert_match(cap("", "Ana", 400)).await.unwrap();
    store.insert_match(cap("orphan", "Ana", 500)).await.unwrap();
    store.refresh_session_hero_timeline("orphan").await.unwrap();
    // A deleted game.
    store.insert_match(cap("gone", "Ana", 600)).await.unwrap();
    store.create_session(&session("gone")).await.unwrap();
    store.refresh_session_hero_timeline("gone").await.unwrap();
    store.delete_session("gone").await.unwrap();
    store
}

async fn migrate(store: &LocalStore) -> usize {
    store
        .migrate_timeline_if_needed(&TimelineBackup::Ready(None))
        .await
        .unwrap()
}

fn json(rows: &[PersonalMatch]) -> Vec<String> {
    let mut v: Vec<String> = rows
        .iter()
        .map(|m| serde_json::to_string(m).unwrap())
        .collect();
    v.sort();
    v
}

async fn record_ids(store: &LocalStore) -> Vec<String> {
    let mut v: Vec<String> = store
        .all_timelines()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.session_id)
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn t1_t2_t8_migration_changes_no_row_and_no_shown_game() {
    let dir = tempfile::tempdir().unwrap();
    let store = alpha1_store(dir.path()).await;
    let before = store.get_all_matches().await.unwrap();
    let raw_before = store.get_all_matches_raw().await.unwrap();

    assert_eq!(migrate(&store).await, 3, "swap, res, pinned");
    assert!(store.timeline_migrated());
    // No record for a game with no stored list (it shows as today), for
    // blank session ids, orphans or deleted games.
    assert_eq!(record_ids(&store).await, vec!["pinned", "res", "swap"]);

    // Additive: every stored row is unchanged...
    assert_eq!(
        json(&store.get_all_matches_raw().await.unwrap()),
        json(&raw_before)
    );
    // ...and every row reads back exactly as before (stored list wins).
    assert_eq!(json(&store.get_all_matches().await.unwrap()), json(&before));
    let legacy = store.get_session_snapshots("legacy").await.unwrap();
    assert!(legacy.iter().all(|m| m.heroes_played.is_empty()));
    assert_eq!(
        legacy[2].display_hero(),
        "Genji",
        "legacy shows its raw newest hero"
    );
    let compact = store.compact_snapshot().await.unwrap();
    assert!(compact.matches.iter().all(|m| m.session_id != "gone"));
}

#[tokio::test]
async fn t3_a_second_run_writes_nothing_and_t4_a_stop_part_way_is_finished_later() {
    let dir = tempfile::tempdir().unwrap();
    let store = alpha1_store(dir.path()).await;
    // Sessions in key order: legacy (no list, no record), pinned, res, swap.
    assert_eq!(store.fill_timelines(Some(2)).await.unwrap(), 1);
    assert_eq!(record_ids(&store).await, vec!["pinned"]);
    assert!(
        !store.timeline_marker().await.unwrap(),
        "no marker before the end"
    );
    assert!(!store.timeline_migrated());
    migrate(&store).await;
    assert!(
        store.timeline_marker().await.unwrap(),
        "marker written last"
    );
    let once = record_ids(&store).await;

    let clean_dir = tempfile::tempdir().unwrap();
    let clean = alpha1_store(clean_dir.path()).await;
    migrate(&clean).await;
    assert_eq!(once, record_ids(&clean).await);

    assert_eq!(migrate(&store).await, 0, "migrated: nothing to do");
    assert_eq!(
        store.fill_timelines(None).await.unwrap(),
        0,
        "equal records are kept"
    );
    assert_eq!(record_ids(&store).await, once);
}

#[tokio::test]
async fn t5_a_capture_after_the_move_writes_one_record_and_no_timeline_copy() {
    let dir = tempfile::tempdir().unwrap();
    let store = alpha1_store(dir.path()).await;
    migrate(&store).await;
    let raw_before = store.get_session_snapshots_raw_for_test("swap").await;

    // Kiriko is now the majority: three more Kiriko captures.
    for i in 0..3 {
        store
            .insert_match(cap("swap", "Kiriko", 10 + i))
            .await
            .unwrap();
        store.refresh_session_hero_timeline("swap").await.unwrap();
    }
    let rows = store.get_session_snapshots("swap").await.unwrap();
    assert!(rows.iter().all(|m| m.display_hero() == "Kiriko"));
    let t = store.get_timeline("swap").await.unwrap().unwrap();
    assert_eq!(
        t.heroes_played.last().unwrap().snapshots,
        3,
        "Ana Kiriko Ana Kiriko(3)"
    );

    let raw = store.get_session_snapshots_raw_for_test("swap").await;
    for old in &raw_before {
        let now = raw.iter().find(|m| m.id == old.id).unwrap();
        assert_eq!(
            now.heroes_played, old.heroes_played,
            "old rows keep their copy"
        );
        assert!(!now.synced, "the shown hero changed: re-queued");
    }
    assert!(
        raw.iter()
            .filter(|m| !raw_before.iter().any(|o| o.id == m.id))
            .all(|m| m.heroes_played.is_empty()),
        "new rows carry no timeline copy"
    );

    // The upload reads the stored timeline: new rows carry no list of their
    // own, yet the queue sends the new majority.
    let queued = store.get_unsynced().await.unwrap();
    let swap: Vec<_> = queued.iter().filter(|m| m.session_id == "swap").collect();
    assert!(!swap.is_empty());
    assert!(swap.iter().all(|m| m.display_hero() == "Kiriko"));

    // A pinned hero never re-queues.
    store
        .insert_match(cap("pinned", "Zarya", 310))
        .await
        .unwrap();
    store.refresh_session_hero_timeline("pinned").await.unwrap();
    let pinned = store.get_session_snapshots_raw_for_test("pinned").await;
    assert!(
        pinned
            .iter()
            .filter(|m| m.corrected_hero.is_some())
            .all(|m| m.synced)
    );
}

#[tokio::test]
async fn t6_segment_decisions_give_the_same_timeline_before_and_after_the_move() {
    async fn run(migrated: bool) -> Vec<HeroSegment> {
        let dir = tempfile::tempdir().unwrap();
        let store = alpha1_store(dir.path()).await;
        if migrated {
            migrate(&store).await;
        }
        store
            .resolve_hero_segment("swap", 1, SEGMENT_DISMISS)
            .await
            .unwrap();
        store
            .resolve_hero_segment("swap", 2, SEGMENT_CONFIRM)
            .await
            .unwrap();
        store
            .resolve_hero_segment("swap", 2, SEGMENT_CLEAR)
            .await
            .unwrap();
        let rows = store.get_session_snapshots("swap").await.unwrap();
        assert!(
            rows.iter()
                .all(|m| m.heroes_played == rows[0].heroes_played)
        );
        rows[0].heroes_played.clone()
    }
    assert_eq!(run(true).await, run(false).await);
}

#[tokio::test]
async fn t7_backup_first_and_no_backup_means_no_move() {
    let dir = tempfile::tempdir().unwrap();
    drop(alpha1_store(dir.path()).await);
    assert!(
        crate::storage::wait_for_engine_close(dir.path(), std::time::Duration::from_secs(10)).await
    );

    // A failed backup (no free space): no move, the store opens as before.
    let failed = migrate_timeline::prepare_with(dir.path(), |_| Some(0)).await;
    assert!(matches!(failed, TimelineBackup::Failed(_)), "{failed:?}");
    {
        let store = LocalStore::open(dir.path()).await.unwrap();
        assert_eq!(store.migrate_timeline_if_needed(&failed).await.unwrap(), 0);
        assert!(!store.timeline_migrated());
        assert!(record_ids(&store).await.is_empty());
    }
    assert!(
        crate::storage::wait_for_engine_close(dir.path(), std::time::Duration::from_secs(10)).await
    );

    let backup = prepare_timeline_migration(dir.path()).await;
    let TimelineBackup::Ready(Some(path)) = &backup else {
        panic!("{backup:?}");
    };
    let live = dir.path().join("stats.surrealkv");
    let files = |p: &std::path::Path| {
        let mut n = 0usize;
        fn walk(p: &std::path::Path, n: &mut usize) {
            for e in std::fs::read_dir(p).unwrap().flatten() {
                if e.path().is_dir() {
                    walk(&e.path(), n)
                } else {
                    *n += 1
                }
            }
        }
        walk(p, &mut n);
        n
    };
    assert_eq!(files(path), files(&live));
    assert_eq!(maintain::dir_size(path), maintain::dir_size(&live));
    assert!(!path.to_string_lossy().ends_with(".partial"));

    // A start that stops before the marker leaves the copy pending; the
    // next start reuses it (it holds the state before any write).
    let pending = std::path::PathBuf::from(format!("{}.pending", path.display()));
    assert!(pending.exists());
    assert_eq!(prepare_timeline_migration(dir.path()).await, backup);
    let copies = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(PRE_TIMELINE_BACKUP_PREFIX)
        })
        .count();
    assert_eq!(copies, 1);
    // Right after prepare, the daemon's own open succeeds.
    let store = LocalStore::open(dir.path()).await.unwrap();
    assert!(store.migrate_timeline_if_needed(&backup).await.unwrap() > 0);
    assert!(
        !pending.exists(),
        "the move finished: the copy is no longer pending"
    );
    drop(store);
    assert!(
        crate::storage::wait_for_engine_close(dir.path(), std::time::Duration::from_secs(10)).await
    );
    assert_eq!(
        prepare_timeline_migration(dir.path()).await,
        TimelineBackup::NotNeeded
    );
    // A migrated store opens straight after the check, with no wait.
    drop(LocalStore::open(dir.path()).await.unwrap());
}

#[tokio::test]
async fn delete_all_removes_the_timeline_backups() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("stats.surrealkv")).unwrap();
    for name in ["20261029_101500", "20261029_101500.partial"] {
        let p = dir
            .path()
            .join(format!("{PRE_TIMELINE_BACKUP_PREFIX}{name}"));
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("db"), "games").unwrap();
    }
    let note = format!("{PRE_TIMELINE_BACKUP_PREFIX}20261029_101500.pending");
    std::fs::write(dir.path().join(note), "").unwrap();
    force_clear_data_dir(dir.path()).unwrap();
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
    assert!(left.is_empty(), "{left:?}");
}

/// Item 2 (M7): the record copies the list the game shows (its newest
/// row's), not a fresh derivation and not an older row's.
#[tokio::test]
async fn t2_the_stored_list_wins_over_a_fresh_derivation() {
    let dir = tempfile::tempdir().unwrap();
    let store = alpha1_store(dir.path()).await;
    for i in 0..4 {
        store
            .insert_match(cap("drift", "Ana", 700 + i))
            .await
            .unwrap();
    }
    store.create_session(&session("drift")).await.unwrap();
    // Raw reads say Ana; the stored lists say Lucio (newest) and Zen (older).
    let seg = |hero: &str| HeroSegment {
        hero: hero.into(),
        role: "Support".into(),
        first_seen: at(700),
        last_seen: at(703),
        snapshots: 4,
        confirmed: true,
        resolution: None,
    };
    store
        .db
        .query("UPDATE personal_match SET heroes_played = $hp WHERE session_id = 'drift'")
        .bind(("hp", vec![seg("Zen")]))
        .await
        .unwrap();
    store
        .db
        .query(
            "UPDATE personal_match SET heroes_played = $hp \
             WHERE session_id = 'drift' AND played_at = $at",
        )
        .bind(("hp", vec![seg("Lucio")]))
        .bind(("at", at(703)))
        .await
        .unwrap();
    let before = store.get_session_snapshots("drift").await.unwrap();
    assert_eq!(before.last().unwrap().display_hero(), "Lucio");
    migrate(&store).await;
    let t = store.get_timeline("drift").await.unwrap().unwrap();
    assert_eq!(t.heroes_played, vec![seg("Lucio")]);
    let after = store.get_session_snapshots("drift").await.unwrap();
    assert_eq!(
        after.last().unwrap().display_hero(),
        "Lucio",
        "shows as before"
    );
}

/// A move that stopped part way leaves records the reads ignore; an edit
/// made before the next start is kept by the rebuild.
#[tokio::test]
async fn an_edit_between_a_partial_move_and_the_next_start_survives() {
    let dir = tempfile::tempdir().unwrap();
    let store = alpha1_store(dir.path()).await;
    store.fill_timelines(None).await.unwrap(); // no marker
    assert!(!store.timeline_migrated());
    store
        .resolve_hero_segment("swap", 1, SEGMENT_CONFIRM)
        .await
        .unwrap();
    let edited = store.get_session_snapshots("swap").await.unwrap();
    assert_eq!(
        edited[0].heroes_played[1].resolution.as_deref(),
        Some(SEGMENT_CONFIRM)
    );
    migrate(&store).await;
    let after = store.get_session_snapshots("swap").await.unwrap();
    assert_eq!(after[0].heroes_played, edited[0].heroes_played, "kept");
}

#[tokio::test]
async fn t9_an_older_tracker_still_reads_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let store = alpha1_store(dir.path()).await;
    let raw = store.get_all_matches_raw().await.unwrap();
    migrate(&store).await;
    // An alpha.1 build reads rows as stored, without the extra tables.
    assert_eq!(
        json(&store.get_all_matches_raw().await.unwrap()),
        json(&raw)
    );
    let swap: Vec<_> = raw.iter().filter(|m| m.session_id == "swap").collect();
    assert!(swap.iter().all(|m| m.heroes_played.len() == 3));
}

#[tokio::test]
async fn t10_vacuum_keeps_the_records_and_an_old_vacuum_is_repaired() {
    let dir = tempfile::tempdir().unwrap();
    let store = alpha1_store(dir.path()).await;
    migrate(&store).await;
    store.insert_match(cap("swap", "Kiriko", 20)).await.unwrap();
    store.refresh_session_hero_timeline("swap").await.unwrap();
    let before = json(&store.get_all_matches().await.unwrap());
    let ids = record_ids(&store).await;
    drop(store);
    assert!(
        crate::storage::wait_for_engine_close(dir.path(), std::time::Duration::from_secs(10)).await
    );

    LocalStore::vacuum(dir.path()).await.unwrap();
    let store = LocalStore::open(dir.path()).await.unwrap();
    assert!(store.timeline_migrated(), "marker copied");
    assert_eq!(record_ids(&store).await, ids);
    let strip = |v: Vec<String>| -> Vec<String> {
        v.into_iter()
            .map(|s| s.replace("\"synced\":false", "\"synced\":true"))
            .collect()
    };
    assert_eq!(
        strip(json(&store.get_all_matches().await.unwrap())),
        strip(before)
    );

    // An alpha.1 vacuum dropped both tables: the next move rebuilds the
    // records from the rows that still have their lists.
    store
        .db
        .query("DELETE hero_timeline; DELETE store_meta;")
        .await
        .unwrap();
    store.set_timeline_migrated(false);
    assert_eq!(migrate(&store).await, 3);
    assert_eq!(record_ids(&store).await, ids);
}

#[tokio::test]
async fn t11_delete_and_clear_leave_no_record() {
    let dir = tempfile::tempdir().unwrap();
    let store = alpha1_store(dir.path()).await;
    migrate(&store).await;
    store.delete_session("swap").await.unwrap();
    assert!(store.get_timeline("swap").await.unwrap().is_none());
    store.clear_all_data().await.unwrap();
    assert!(record_ids(&store).await.is_empty());
    assert!(
        store.timeline_marker().await.unwrap(),
        "an empty store keeps the new layout: no second backup"
    );
}

#[tokio::test]
async fn t12_open_vacuum_and_clear_never_write_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    drop(alpha1_store(dir.path()).await);
    assert!(
        crate::storage::wait_for_engine_close(dir.path(), std::time::Duration::from_secs(10)).await
    );
    LocalStore::vacuum(dir.path()).await.unwrap();
    {
        let store = LocalStore::open(dir.path()).await.unwrap();
        assert!(!store.timeline_migrated(), "vacuum and open do not move");
        assert!(!store.timeline_marker().await.unwrap());
        assert!(record_ids(&store).await.is_empty());
        store.clear_all_data().await.unwrap();
    }
    let backups = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(PRE_TIMELINE_BACKUP_PREFIX)
        })
        .count();
    assert_eq!(backups, 0);
}

#[tokio::test]
async fn pack_key_heroes_are_renamed_in_rows_and_in_the_record() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = LocalStore::open(dir.path()).await.unwrap();
        for i in 0..3 {
            store.insert_match(cap("wb", "Ana", i)).await.unwrap();
        }
        store.create_session(&session("wb")).await.unwrap();
        store.refresh_session_hero_timeline("wb").await.unwrap();
        migrate(&store).await;
        // A pack key slipped into the record and a new row.
        let mut t = store.get_timeline("wb").await.unwrap().unwrap();
        t.heroes_played[0].hero = "wrecking-ball".into();
        store.put_timeline(&t).await.unwrap();
        store
            .insert_match(cap("wb", "wrecking-ball", 9))
            .await
            .unwrap();
        store
            .db
            .query("UPDATE personal_match SET synced = true")
            .await
            .unwrap();
    }
    assert!(
        crate::storage::wait_for_engine_close(dir.path(), std::time::Duration::from_secs(10)).await
    );
    let store = LocalStore::open(dir.path()).await.unwrap(); // runs the pass
    let t = store.get_timeline("wb").await.unwrap().unwrap();
    assert_eq!(t.heroes_played[0].hero, "Wrecking Ball");
    assert_eq!(t.heroes_played[0].role, "Tank");
    let rows = store.get_session_snapshots_raw_for_test("wb").await;
    assert!(rows.iter().all(|m| m.hero != "wrecking-ball"));
    assert!(rows.iter().all(|m| !m.synced), "the uploaded hero changed");
    drop(store);
    assert!(
        crate::storage::wait_for_engine_close(dir.path(), std::time::Duration::from_secs(10)).await
    );
    let store = LocalStore::open(dir.path()).await.unwrap();
    let again = store.get_timeline("wb").await.unwrap().unwrap();
    assert_eq!(again, t, "a second pass changes nothing");
}

/// Every non-test read of whole `personal_match` rows either hydrates them
/// or is a named raw read. A new read must be added here with a decision.
#[test]
fn every_row_read_is_hydrated_or_named_raw() {
    let files = [
        ("export.rs", include_str!("export.rs")),
        ("migrate_timeline.rs", include_str!("migrate_timeline.rs")),
        ("rows.rs", include_str!("rows.rs")),
        ("upload_queue.rs", include_str!("upload_queue.rs")),
        ("edits.rs", include_str!("edits.rs")),
        ("migrate.rs", include_str!("migrate.rs")),
        ("tombstones.rs", include_str!("tombstones.rs")),
        ("maintain.rs", include_str!("maintain.rs")),
    ];
    // (file, reads of `SELECT * FROM personal_match`): every one of these is
    // either hydrated by its caller or raw on purpose:
    // export.rs: RowPages (3 queries), hydrated by compact_snapshot_paged;
    //   raw for the pack-key pass, which compares on the hydrated view.
    // migrate_timeline.rs: session_rows_raw, the move reads rows as stored.
    // rows.rs: get_all_matches (hydrated), get_all_matches_raw (vacuum),
    //   get_session_snapshots (hydrated).
    // upload_queue.rs: pending_upload_rows (hydrated, 2 queries).
    let expected = [
        ("export.rs", 3),
        ("migrate_timeline.rs", 1),
        ("rows.rs", 3),
        ("upload_queue.rs", 2),
        ("edits.rs", 0),
        ("migrate.rs", 0),
        ("tombstones.rs", 0),
        ("maintain.rs", 0),
    ];
    for ((name, text), (ename, n)) in files.iter().zip(expected) {
        assert_eq!(*name, ename);
        let prod = text.split("#[cfg(test)]").next().unwrap();
        assert_eq!(
            prod.matches("SELECT * FROM personal_match").count(),
            n,
            "{name}: a row read was added or removed; decide hydration and update this test"
        );
    }
    assert_eq!(
        include_str!("upload_queue.rs")
            .matches("hydrate_rows")
            .count(),
        1
    );
    assert_eq!(include_str!("rows.rs").matches("hydrate_rows").count(), 2);
    assert_eq!(include_str!("export.rs").matches("hydrate_rows").count(), 1);
}

impl LocalStore {
    async fn get_session_snapshots_raw_for_test(&self, sid: &str) -> Vec<PersonalMatch> {
        self.session_rows_raw(sid).await.unwrap()
    }
}

/// A finished move's copy is not reused when the marker is lost later (an
/// older tracker's vacuum dropped the tables): the next move takes a fresh
/// copy, and once it is done only that newest copy is kept.
#[tokio::test]
async fn a_finished_copy_is_not_reused_and_older_copies_are_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let wait =
        || crate::storage::wait_for_engine_close(dir.path(), std::time::Duration::from_secs(10));
    drop(alpha1_store(dir.path()).await);
    assert!(wait().await);
    let first = prepare_timeline_migration(dir.path()).await;
    let TimelineBackup::Ready(Some(first_path)) = first.clone() else {
        panic!("{first:?}");
    };
    {
        let store = LocalStore::open(dir.path()).await.unwrap();
        assert!(store.migrate_timeline_if_needed(&first).await.unwrap() > 0);
        // An older tracker's vacuum drops both tables.
        store
            .db
            .query("DELETE hero_timeline; DELETE store_meta;")
            .await
            .unwrap();
    }
    assert!(wait().await);
    // Backup names carry the second; make the next one differ.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let second = prepare_timeline_migration(dir.path()).await;
    let TimelineBackup::Ready(Some(second_path)) = second.clone() else {
        panic!("{second:?}");
    };
    assert_ne!(second_path, first_path, "a finished copy is not reused");
    {
        let store = LocalStore::open(dir.path()).await.unwrap();
        assert!(store.migrate_timeline_if_needed(&second).await.unwrap() > 0);
    }
    assert!(second_path.is_dir());
    assert!(!first_path.exists(), "the older finished copy is pruned");
}
