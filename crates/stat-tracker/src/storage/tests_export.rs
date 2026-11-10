//! The compact `live_snapshot.json`: which rows it keeps, paging, and that
//! the GUI picks the same game rows from it as from the old full export.

use super::tests::snap;
use super::*;
use chrono::{TimeZone, Utc};

fn at(secs: i64) -> SurrealDatetime {
    SurrealDatetime::from(Utc.timestamp_opt(1_800_000_000 + secs, 0).unwrap())
}

fn row(session_id: &str, secs: i64, map: &str, elims: u32) -> PersonalMatch {
    let mut m = snap(session_id, elims);
    m.played_at = at(secs);
    m.map_name = map.into();
    m.game_mode = crate::parse::stored_game_mode(map);
    m.name_recognizer = Some("names-v1".into());
    m
}

fn session(session_id: &str, secs: i64) -> MatchSession {
    MatchSession {
        session_id: session_id.into(),
        hero: "Test".into(),
        map_name: "Busan".into(),
        role: "Tank".into(),
        started_at: at(secs),
        last_capture_at: at(secs),
        capture_count: 1,
        final_outcome: "victory".into(),
    }
}

/// What `games_from_snapshot` (UI crate) does: filter untracked rows, then
/// keep the first row per session and every blank-session row. Kept here
/// as (session, elims, played_at) so the storage tests need no UI types.
fn shown(snap: &Snapshot) -> Vec<(String, u32, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = snap
        .matches
        .iter()
        .filter(|m| super::export::row_is_tracked(m))
        .filter(|m| m.session_id.is_empty() || seen.insert(m.session_id.clone()))
        .map(|m| (m.session_id.clone(), m.elims, m.played_at.to_string()))
        .collect::<Vec<_>>();
    // Rows captured in the same instant have no defined order in the store:
    // keep newest first and order a tie by session and elims, so only the
    // order across instants is compared.
    out.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    out
}

/// The pre-0.6.0-alpha.1.1 export: every row.
async fn full_snapshot(store: &LocalStore) -> Snapshot {
    Snapshot {
        matches: store.get_all_matches().await.unwrap(),
        sessions: store.get_all_sessions().await.unwrap(),
        pending_deletes: store.get_pending_tombstones().await.unwrap(),
    }
}

/// A store with every case the GUI and the report bundle care about.
async fn fixture(store: &LocalStore) {
    // A plain game: three captures, the newest is shown.
    for (i, e) in [10, 11, 12].into_iter().enumerate() {
        store
            .insert_match(row("plain", i as i64, "Busan", e))
            .await
            .unwrap();
    }
    store.create_session(&session("plain", 0)).await.unwrap();
    // The newest row was corrected to a deathmatch map: the GUI skips it and
    // shows the newest tracked row of the same game.
    store
        .insert_match(row("dm-last", 10, "Busan", 20))
        .await
        .unwrap();
    store
        .insert_match(row("dm-last", 11, "Ilios", 21))
        .await
        .unwrap();
    let mut newest = row("dm-last", 12, "Busan", 22);
    newest.corrected_map_name = Some("Château Guillard".into());
    store.insert_match(newest).await.unwrap();
    store.create_session(&session("dm-last", 10)).await.unwrap();
    // A game whose rows are all untracked: the GUI shows nothing, the
    // report still finds its newest row.
    store
        .insert_match(row("practice", 20, "Practice Range", 30))
        .await
        .unwrap();
    store
        .insert_match(row("practice", 21, "Practice Range", 31))
        .await
        .unwrap();
    store
        .create_session(&session("practice", 20))
        .await
        .unwrap();
    // Rows with no session row (an orphan) still show.
    store
        .insert_match(row("orphan", 30, "Busan", 40))
        .await
        .unwrap();
    store
        .insert_match(row("orphan", 31, "Busan", 41))
        .await
        .unwrap();
    // Rows with no session id are each a game.
    store.insert_match(row("", 40, "Busan", 50)).await.unwrap();
    store.insert_match(row("", 41, "Busan", 51)).await.unwrap();
    // A deleted game is gone and pending for the site.
    store
        .insert_match(row("gone", 50, "Busan", 60))
        .await
        .unwrap();
    store.create_session(&session("gone", 50)).await.unwrap();
    store.delete_session("gone").await.unwrap();
    // Two games captured in the same instant (a page boundary tie).
    store
        .insert_match(row("tie-a", 60, "Busan", 70))
        .await
        .unwrap();
    store
        .insert_match(row("tie-b", 60, "Busan", 71))
        .await
        .unwrap();
}

#[tokio::test]
async fn the_compact_snapshot_keeps_one_row_per_game_and_every_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::open(dir.path()).await.unwrap();
    fixture(&store).await;

    let full = full_snapshot(&store).await;
    let compact = store.compact_snapshot().await.unwrap();

    let ids = |s: &Snapshot, sid: &str| {
        s.matches
            .iter()
            .filter(|m| m.session_id == sid)
            .map(|m| m.elims)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&compact, "plain"), vec![12], "newest row only");
    assert_eq!(
        ids(&compact, "dm-last"),
        vec![22, 21],
        "newest, then newest tracked"
    );
    assert_eq!(
        ids(&compact, "practice"),
        vec![31],
        "untracked game: newest only"
    );
    assert_eq!(ids(&compact, "orphan"), vec![41]);
    assert_eq!(ids(&compact, ""), vec![51, 50], "every blank-session row");
    assert!(ids(&compact, "gone").is_empty());
    assert_eq!(compact.pending_deletes, vec!["gone".to_string()]);
    assert_eq!(compact.sessions.len(), full.sessions.len());
    assert_eq!(
        ids(&compact, "tie-a").len() + ids(&compact, "tie-b").len(),
        2
    );
    assert!(compact.matches.len() < full.matches.len());

    // Newest first, like the full export.
    let times: Vec<String> = compact
        .matches
        .iter()
        .map(|m| m.played_at.to_string())
        .collect();
    let mut sorted = times.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(times, sorted);

    // The GUI picks the same rows, in the same order, from either file.
    assert_eq!(shown(&compact), shown(&full));
    // The report bundle finds the same newest row per game.
    for sid in ["plain", "dm-last", "practice", "orphan", "tie-a", "tie-b"] {
        let first = |s: &Snapshot| s.matches.iter().find(|m| m.session_id == sid).cloned();
        assert_eq!(first(&compact), first(&full), "{sid}");
    }
}

#[tokio::test]
async fn small_pages_give_the_same_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::open(dir.path()).await.unwrap();
    fixture(&store).await;
    // Many rows on one timestamp across page edges, too.
    for i in 0..7u32 {
        store
            .insert_match(row(&format!("same-{i}"), 70, "Busan", i))
            .await
            .unwrap();
    }
    let whole = store.compact_snapshot_paged(10_000).await.unwrap();
    let key = |s: &Snapshot| {
        let mut v: Vec<(String, u32)> = s
            .matches
            .iter()
            .map(|m| (m.session_id.clone(), m.elims))
            .collect();
        v.sort();
        v
    };
    for page in [1usize, 2, 3, 5, 7, 8, 13] {
        let paged = store.compact_snapshot_paged(page).await.unwrap();
        assert_eq!(key(&paged), key(&whole), "page {page}");
        assert_eq!(shown(&paged), shown(&whole), "page {page}");
    }
}

#[tokio::test]
async fn export_snapshot_writes_the_compact_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::open(dir.path()).await.unwrap();
    fixture(&store).await;
    store.export_snapshot(dir.path()).await.unwrap();
    let read = read_snapshot(dir.path()).expect("snapshot parses");
    let mut compact = store.compact_snapshot().await.unwrap();
    // The file leaves out record ids (`#[serde(skip)]`).
    for m in &mut compact.matches {
        m.id = None;
    }
    assert_eq!(read.matches, compact.matches);
    assert_eq!(read.pending_deletes, compact.pending_deletes);
    assert!(!dir.path().join("live_snapshot.json.tmp").exists());
}
