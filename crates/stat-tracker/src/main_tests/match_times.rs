//! Daemon tests: the game's real start and end. A pre-game screen that
//! opens the game is its start, the result the tracker read itself is its
//! end, and both go on the stored rows. A missed start or end still records
//! the game. See `mod.rs`.

use super::*;
use stat_tracker::model::MatchTimes;

/// End minus start, to the nearest second; `None` without both.
fn length(t: &MatchTimes) -> Option<i64> {
    let start: chrono::DateTime<Utc> = t.started_at?.into();
    let end: chrono::DateTime<Utc> = t.ended_at?.into();
    Some(((end - start).num_milliseconds() as f64 / 1000.0).round() as i64)
}

fn seen(outcome: detect::MatchOutcome, at: Instant) -> Option<ResultMark> {
    Some(ResultMark {
        outcome,
        confirmed: true,
        seen_at: at,
    })
}

/// Close `g` the way a new-game boundary does and read back its rows.
async fn close_and_read(
    g: ActiveGame,
    seal: Option<detect::MatchOutcome>,
) -> Vec<storage::PersonalMatch> {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let sid = g.session_id.clone();
    store.insert_match(unfinished_row(&sid)).await.unwrap();
    let mut st = session(Some(g), test_now());
    retire_active_game(&mut st, &store, dir.path(), seal, "test close").await;
    store.get_session_snapshots(&sid).await.unwrap()
}

#[test]
fn a_hero_select_opens_the_game_with_its_real_start_and_a_tab_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let now = test_now();
    let mut st = session(None, now);
    open_detected_game(
        &mut st,
        dir.path(),
        &boundary::StartScreen::HeroSelect,
        now,
        Duration::from_secs(120),
    );
    let g = st.active_game.as_ref().expect("hero select opens a game");
    assert!(g.match_times.started_at.is_some());
    assert_eq!(g.match_times.ended_at, None);
    // A game a Tab opens has no real start: its length falls back.
    let tab = tab_opened_game("tab".into(), None, None, now);
    assert_eq!(tab.match_times, MatchTimes::default());
}

#[test]
fn the_end_is_when_the_result_was_first_seen_and_stays() {
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.note_match_start(now);
    g.result_mark = seen(
        detect::MatchOutcome::Victory,
        now + Duration::from_secs(754),
    );
    g.record_outcome_at(
        detect::MatchOutcome::Victory,
        now + Duration::from_secs(790),
    );
    assert_eq!(length(&g.match_times), Some(754));
    let first = g.match_times.ended_at;
    g.note_match_end(
        detect::MatchOutcome::Victory,
        now + Duration::from_secs(900),
    );
    assert_eq!(g.match_times.ended_at, first, "the first end stays");
    // No result word kept: the end is when the result was recorded.
    let mut no_mark = game(detect::MatchOutcome::Unknown, None, now);
    no_mark.note_match_start(now);
    no_mark.record_outcome_at(detect::MatchOutcome::Defeat, now + Duration::from_secs(600));
    assert_eq!(length(&no_mark.match_times), Some(600));
}

#[test]
fn a_result_set_in_the_app_is_not_an_end() {
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.note_match_start(now);
    g.record_member_outcome(detect::MatchOutcome::Victory);
    assert!(g.finished());
    assert!(g.match_times.started_at.is_some());
    assert_eq!(g.match_times.ended_at, None);
    // An undecided result is no end either.
    g.note_match_end(detect::MatchOutcome::Unknown, now);
    assert_eq!(g.match_times.ended_at, None);
}

#[tokio::test]
async fn a_sealed_game_stores_its_real_start_and_end() {
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_id = "sealed".into();
    g.note_match_start(now);
    g.result_mark = seen(
        detect::MatchOutcome::Victory,
        now + Duration::from_secs(554),
    );
    let rows = close_and_read(g, Some(detect::MatchOutcome::Victory)).await;
    assert_eq!(rows.len(), 1);
    let times = rows[0].match_times.as_ref().expect("times stored");
    assert_eq!(length(times), Some(554));
}

#[tokio::test]
async fn a_missed_end_still_records_the_game_with_its_start_only() {
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_id = "no-end".into();
    g.note_match_start(now);
    let rows = close_and_read(g, None).await;
    assert_eq!(rows.len(), 1, "the game stays recorded");
    let times = rows[0].match_times.as_ref().expect("times stored");
    assert!(times.started_at.is_some());
    assert_eq!(times.ended_at, None);
    assert_eq!(length(times), None);
}

#[tokio::test]
async fn a_missed_start_still_records_the_game_with_its_end_only() {
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_id = "no-start".into();
    g.record_outcome_at(detect::MatchOutcome::Defeat, now);
    let rows = close_and_read(g, None).await;
    assert_eq!(rows.len(), 1, "the game stays recorded");
    let times = rows[0].match_times.as_ref().expect("times stored");
    assert_eq!(times.started_at, None);
    assert!(times.ended_at.is_some());
}

#[tokio::test]
async fn a_game_with_no_stored_row_writes_no_times() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut g = game(detect::MatchOutcome::Unknown, None, test_now());
    g.session_created = false;
    g.note_match_start(test_now());
    store_match_times(&store, &g).await;
    assert!(store.get_all_matches().await.unwrap().is_empty());
}

#[test]
fn the_start_and_end_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let now = Instant::now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_created = false;
    g.note_match_start(now);
    g.record_outcome_at(detect::MatchOutcome::Victory, now);
    let want = g.match_times.clone();
    persist_active_game(dir.path(), Some(&g));
    let back = startup_session(dir.path())
        .active_game
        .expect("the game resumes");
    assert_eq!(back.match_times, want);
}
