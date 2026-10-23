//! Tests of the HS-1 hero timeline: segment derivation, the primary hero,
//! the stored timeline refresh and segment resolutions.

use super::tests::{mark_current, snap};
use super::*;
use chrono::Utc;

// --- HS-1 hero timeline --------------------------------------------------
/// A snapshot with an explicit hero/role and a distinct capture time
/// (`secs` offset), for exercising ordering and segment derivation.
fn snap_h(session_id: &str, hero: &str, role: &str, secs: i64) -> PersonalMatch {
    let mut m = snap(session_id, 0);
    m.hero = hero.into();
    m.role = role.into();
    m.played_at = SurrealDatetime::from(Utc::now() + chrono::Duration::seconds(secs));
    m
}

/// A derived hero segment, for exercising `primary_hero_role` directly.
fn seg(hero: &str, role: &str, snapshots: u32, confirmed: bool, secs: i64) -> HeroSegment {
    let t = SurrealDatetime::from(Utc::now() + chrono::Duration::seconds(secs));
    HeroSegment {
        hero: hero.into(),
        role: role.into(),
        first_seen: t,
        last_seen: t,
        snapshots,
        confirmed,
        resolution: None,
    }
}

#[test]
fn derive_orders_by_capture_time_regardless_of_input() {
    // Shuffled input; derivation sorts by played_at → Ana, Reinhardt, Cassidy.
    let snaps = vec![
        snap_h("s", "Cassidy", "Damage", 30),
        snap_h("s", "Ana", "Support", 10),
        snap_h("s", "Reinhardt", "Tank", 20),
    ];
    let segs = derive_hero_segments(&snaps, &[]);
    let order: Vec<&str> = segs.iter().map(|s| s.hero.as_str()).collect();
    assert_eq!(order, ["Ana", "Reinhardt", "Cassidy"]);
    assert!(segs.iter().all(|s| s.snapshots == 1 && !s.confirmed));
}

#[test]
fn derive_skips_unknown_and_empty_without_splitting_runs() {
    // Ana, Unknown, "", Ana → one Ana run of 2; the junk reads neither open
    // a segment nor split the surrounding run.
    let snaps = vec![
        snap_h("s", "Ana", "Support", 10),
        snap_h("s", "Unknown", "Support", 20),
        snap_h("s", "", "", 30),
        snap_h("s", "Ana", "Support", 40),
    ];
    let segs = derive_hero_segments(&snaps, &[]);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].hero, "Ana");
    assert_eq!(segs[0].snapshots, 2);
    assert!(segs[0].confirmed);
}

#[test]
fn derive_coalesces_consecutive_same_hero() {
    let snaps: Vec<PersonalMatch> = (0..25i64)
        .map(|i| snap_h("s", "Ana", "Support", i))
        .collect();
    let segs = derive_hero_segments(&snaps, &[]);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].snapshots, 25);
    assert!(segs[0].confirmed);
}

#[test]
fn derive_swap_back_makes_three_segments() {
    // A -> B -> A is three distinct runs, not two.
    let mut snaps: Vec<PersonalMatch> = (0..3i64)
        .map(|i| snap_h("s", "Ana", "Support", i))
        .collect();
    snaps.push(snap_h("s", "Genji", "Damage", 3));
    snaps.extend((4..7i64).map(|i| snap_h("s", "Ana", "Support", i)));
    let segs = derive_hero_segments(&snaps, &[]);
    assert_eq!(segs.len(), 3);
    assert_eq!(segs[0].hero, "Ana");
    assert_eq!(segs[1].hero, "Genji");
    assert_eq!(segs[2].hero, "Ana");
    assert_eq!(segs[1].snapshots, 1);
    assert!(!segs[1].confirmed, "a lone swap stays unconfirmed");
    assert!(segs[0].confirmed && segs[2].confirmed);
}

#[test]
fn derive_applies_confirm_and_dismiss_resolutions() {
    // Ana x3 (confirmed by count), Genji x1 (unconfirmed flap).
    let mut snaps: Vec<PersonalMatch> = (0..3i64)
        .map(|i| snap_h("s", "Ana", "Support", i))
        .collect();
    snaps.push(snap_h("s", "Genji", "Damage", 3));
    let res = vec![
        SegmentResolution {
            segment: 1,
            action: SEGMENT_CONFIRM.into(),
        },
        SegmentResolution {
            segment: 0,
            action: SEGMENT_DISMISS.into(),
        },
    ];
    let segs = derive_hero_segments(&snaps, &res);
    assert_eq!(segs.len(), 2);
    assert!(
        segs[1].confirmed,
        "confirm forces a 1-snapshot segment confirmed"
    );
    assert_eq!(segs[1].resolution.as_deref(), Some(SEGMENT_CONFIRM));
    assert!(!segs[0].confirmed, "dismiss clears confirmed");
    assert!(segs[0].is_dismissed());
}

#[test]
fn primary_picks_snapshot_count_majority() {
    let segs = vec![
        seg("Ana", "Support", 25, true, 0),
        seg("Genji", "Damage", 3, true, 10),
    ];
    assert_eq!(primary_hero_role(&segs), Some(("Ana", "Support")));
}

#[test]
fn primary_dismissed_segments_never_vote() {
    let mut segs = vec![
        seg("Ana", "Support", 25, true, 0),
        seg("Genji", "Damage", 3, true, 10),
    ];
    // Dismiss the majority hero → the remaining segment wins.
    segs[0].resolution = Some(SEGMENT_DISMISS.into());
    assert_eq!(primary_hero_role(&segs), Some(("Genji", "Damage")));
}

#[test]
fn primary_unconfirmed_votes_only_when_all_unconfirmed() {
    // Mixed: a confirmed Ana(2) and an unconfirmed Genji(5). Despite more
    // Genji snapshots, only the confirmed segment votes.
    let mixed = vec![
        seg("Ana", "Support", 2, true, 0),
        seg("Genji", "Damage", 5, false, 10),
    ];
    assert_eq!(primary_hero_role(&mixed), Some(("Ana", "Support")));

    // All-unconfirmed game: the unconfirmed segments DO vote (a label is
    // still needed), so the larger count wins.
    let all_unconfirmed = vec![
        seg("Ana", "Support", 1, false, 0),
        seg("Genji", "Damage", 3, false, 10),
    ];
    assert_eq!(
        primary_hero_role(&all_unconfirmed),
        Some(("Genji", "Damage"))
    );
}

#[test]
fn primary_lone_flap_never_outvotes_solid_run() {
    // The core HS-1 case: 25 confirmed Ana vs a single unconfirmed flap.
    let segs = vec![
        seg("Ana", "Support", 25, true, 0),
        seg("Wuyang", "Support", 1, false, 10),
    ];
    assert_eq!(primary_hero_role(&segs), Some(("Ana", "Support")));
}

#[test]
fn primary_tie_breaks_latest_then_alpha() {
    // Equal counts, distinct last_seen → the later segment wins.
    let segs = vec![
        seg("Ana", "Support", 3, true, 0),
        seg("Zenyatta", "Support", 3, true, 50),
    ];
    assert_eq!(primary_hero_role(&segs), Some(("Zenyatta", "Support")));

    // Equal counts AND equal last_seen → alphabetical (deterministic).
    let t = SurrealDatetime::from(Utc::now());
    let mk = |hero: &str| HeroSegment {
        hero: hero.into(),
        role: "Support".into(),
        first_seen: t,
        last_seen: t,
        snapshots: 3,
        confirmed: true,
        resolution: None,
    };
    assert_eq!(
        primary_hero_role(&[mk("Ana"), mk("Zenyatta")]),
        Some(("Ana", "Support"))
    );
}

#[test]
fn primary_none_when_empty_or_all_dismissed() {
    assert_eq!(primary_hero_role(&[]), None);
    let mut segs = vec![seg("Ana", "Support", 3, true, 0)];
    segs[0].resolution = Some(SEGMENT_DISMISS.into());
    assert_eq!(primary_hero_role(&segs), None);
}

#[tokio::test]
async fn refresh_hero_timeline_stamps_segments_and_preserves_raw() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    // 3x Ana then a Genji swap, all already synced (post-upload state).
    let seq = [
        ("Ana", "Support"),
        ("Ana", "Support"),
        ("Ana", "Support"),
        ("Genji", "Damage"),
    ];
    for (i, (hero, role)) in seq.into_iter().enumerate() {
        let m = snap_h("s1", hero, role, i as i64);
        append_match_log(dir.path(), &m);
        store.insert_match(m).await.unwrap();
    }
    mark_current(&store).await;

    store.refresh_session_hero_timeline("s1").await.unwrap();

    let rows = store.get_session_snapshots("s1").await.unwrap();
    assert_eq!(rows.len(), 4);
    for m in &rows {
        // Every row round-trips the same derived timeline...
        assert_eq!(m.heroes_played.len(), 2);
        assert_eq!(m.heroes_played[0].hero, "Ana");
        assert_eq!(m.heroes_played[1].hero, "Genji");
        // ...while the RAW per-snapshot read is preserved, not overwritten.
        assert!(m.hero == "Ana" || m.hero == "Genji");
        // Primary = majority (Ana 3 vs Genji 1) drives display.
        assert_eq!(m.display_hero(), "Ana");
        assert_eq!(m.display_role(), "Support");
    }
    assert!(
        rows.iter().any(|m| m.hero == "Genji"),
        "raw Genji read survived"
    );

    // jsonl fallback carries the timeline too, raw hero preserved.
    let logged = read_match_log(dir.path());
    assert_eq!(logged.len(), 4);
    assert!(
        logged
            .iter()
            .all(|m| m.heroes_played.len() == 2 && m.display_hero() == "Ana")
    );
    assert!(logged.iter().any(|m| m.hero == "Genji"));
}

#[tokio::test]
async fn refresh_hero_timeline_stable_hero_unchanged_no_churn() {
    // A single stable hero must behave exactly like the raw read, and the
    // every-capture refresh must not re-queue already-synced rows.
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    for i in 0..5i64 {
        store
            .insert_match(snap_h("s1", "Ana", "Support", i))
            .await
            .unwrap();
    }
    mark_current(&store).await;

    store.refresh_session_hero_timeline("s1").await.unwrap();
    let rows = store.get_session_snapshots("s1").await.unwrap();
    assert!(rows.iter().all(|m| m.display_hero() == "Ana"));
    // display hero == raw hero → no effective change → nothing re-queued.
    assert!(
        rows.iter().all(|m| m.synced),
        "stable hero must not churn sync"
    );

    // Idempotent: a second refresh is a no-op (timeline already current).
    store.refresh_session_hero_timeline("s1").await.unwrap();
    let rows2 = store.get_session_snapshots("s1").await.unwrap();
    assert!(rows2.iter().all(|m| m.synced));
}

#[tokio::test]
async fn resolve_hero_segment_confirm_then_dismiss() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    // 25 Ana then a lone Wuyang flap.
    for i in 0..25i64 {
        store
            .insert_match(snap_h("s1", "Ana", "Support", i))
            .await
            .unwrap();
    }
    store
        .insert_match(snap_h("s1", "Wuyang", "Support", 25))
        .await
        .unwrap();
    store.refresh_session_hero_timeline("s1").await.unwrap();
    mark_current(&store).await;

    // Flap is unconfirmed → Ana is primary.
    let rows = store.get_session_snapshots("s1").await.unwrap();
    assert!(rows.iter().all(|m| m.display_hero() == "Ana"));

    // Confirm the flap (segment 1): now a real swap, but Ana still majority.
    store
        .resolve_hero_segment("s1", 1, SEGMENT_CONFIRM)
        .await
        .unwrap();
    let rows = store.get_session_snapshots("s1").await.unwrap();
    assert!(rows.iter().all(|m| m.display_hero() == "Ana"));
    assert!(rows.iter().all(|m| m.heroes_played[1].confirmed));
    // Primary unchanged → no re-queue.
    assert!(
        rows.iter().all(|m| m.synced),
        "confirm w/o primary change must not churn"
    );

    // Dismiss segment 0 (the Ana run) → Wuyang is the only voter.
    store
        .resolve_hero_segment("s1", 0, SEGMENT_DISMISS)
        .await
        .unwrap();
    let rows = store.get_session_snapshots("s1").await.unwrap();
    assert!(rows.iter().all(|m| m.display_hero() == "Wuyang"));
    assert!(rows.iter().all(|m| m.heroes_played[0].is_dismissed()));
    // Effective hero changed → rows re-queued for sync.
    assert!(rows.iter().all(|m| !m.synced));
    // Resolution persisted uniformly on every row.
    assert!(
        rows.iter()
            .all(|m| m.segment_resolutions.iter().any(|r| r.segment == 0))
    );
}

#[tokio::test]
async fn apply_command_resolves_hero_segment() {
    // The queued-command path (GUI → daemon) dispatches to resolve.
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalStore::open(dir.path()).await.expect("open store");
    for i in 0..3i64 {
        store
            .insert_match(snap_h("s1", "Ana", "Support", i))
            .await
            .unwrap();
    }
    store
        .insert_match(snap_h("s1", "Genji", "Damage", 3))
        .await
        .unwrap();
    store.refresh_session_hero_timeline("s1").await.unwrap();

    let cmd = StoreCommand::ResolveSegment {
        session_id: "s1".into(),
        segment: 1,
        action: SEGMENT_DISMISS.into(),
    };
    store.apply_command(&cmd).await.unwrap();

    let rows = store.get_session_snapshots("s1").await.unwrap();
    assert!(rows.iter().all(|m| m.display_hero() == "Ana"));
    assert!(rows.iter().all(|m| m.heroes_played[1].is_dismissed()));
}

#[test]
fn resolve_segment_command_json_roundtrips() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cmd = StoreCommand::ResolveSegment {
        session_id: "s1".into(),
        segment: 2,
        action: SEGMENT_CONFIRM.into(),
    };
    queue_command(dir.path(), &cmd).unwrap();
    let cmds = read_commands(dir.path());
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0].1,
        StoreCommand::ResolveSegment { session_id, segment, action }
        if session_id == "s1" && *segment == 2 && action == SEGMENT_CONFIRM
    ));
}

#[test]
fn latest_per_game_derives_timeline_for_legacy_rows() {
    // Legacy session rows: heroes_played empty. Newest-first input (the
    // contract of latest_per_game). Ana(final) / Genji / Ana.
    let rows = vec![
        snap_h("s", "Ana", "Support", 30),
        snap_h("s", "Genji", "Damage", 20),
        snap_h("s", "Ana", "Support", 10),
    ];
    let games = latest_per_game(rows);
    assert_eq!(games.len(), 1);
    let segs = &games[0].heroes_played;
    let order: Vec<&str> = segs.iter().map(|s| s.hero.as_str()).collect();
    assert_eq!(
        order,
        ["Ana", "Genji", "Ana"],
        "legacy swap history derived on the fly"
    );
    // Kept row is the final (newest) snapshot; display uses the derived
    // primary (Ana appears in two 1-snapshot segments → 2 votes vs Genji 1).
    assert_eq!(games[0].hero, "Ana");
    assert_eq!(games[0].display_hero(), "Ana");
}
