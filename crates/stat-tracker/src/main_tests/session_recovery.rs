//! Daemon tests: session_recovery. See `mod.rs`.

use super::*;

#[test]
fn tab_with_no_game_starts_fresh() {
    assert!(should_start_fresh_session(None, test_now()));
}

#[test]
fn tab_mid_game_reuses_session() {
    let now = test_now();
    let g = game(detect::MatchOutcome::Unknown, None, now);
    assert!(!should_start_fresh_session(Some(&g), now));
}

#[test]
fn tab_on_post_match_scoreboard_reuses_finished_session() {
    // Within the grace window the Tab capture is the post-match scoreboard
    // of the SAME match, a fresh session would double-count the game.
    let now = test_now();
    let g = game(detect::MatchOutcome::Defeat, Some(30), now);
    assert!(!should_start_fresh_session(Some(&g), now));
}

#[test]
fn tab_long_after_finish_starts_fresh() {
    let now = test_now();
    let g = game(detect::MatchOutcome::Victory, Some(120), now);
    assert!(should_start_fresh_session(Some(&g), now));
    // An unstamped outcome is treated as stale: a finished result must
    // never leak onto the next match's captures.
    let g = game(detect::MatchOutcome::Victory, None, now);
    assert!(should_start_fresh_session(Some(&g), now));
}

#[test]
fn idle_unfinished_session_goes_stale() {
    // An unfinished session with recent activity is reusable...
    let now = test_now() + UNFINISHED_SESSION_IDLE * 2;
    let g = game(detect::MatchOutcome::Unknown, None, now);
    assert!(!should_start_fresh_session(Some(&g), now));
    // ...but one idle past the bound can't plausibly be the same game:
    // yesterday's session must not absorb today's first Tab.
    let mut stale = game(detect::MatchOutcome::Unknown, None, now);
    stale.last_activity = now - UNFINISHED_SESSION_IDLE - Duration::from_secs(1);
    assert!(should_start_fresh_session(Some(&stale), now));
}

/// Career profile (Competitive Open Queue): **one** Neon Junction,
/// ~10:02, 3–0 Victory. Oasis defeat and Ilios victory are different maps.
/// The Games pair (21:49 empty `, `, 21:55 WIN, same Tank / Neon Junction /
/// Wrecking Ball) is a double capture of that one match, not two queues.
/// Do not invent a second Neon Junction session.
#[test]
fn unfinished_neon_junction_ball_2149_empty_2155_win_reuses() {
    let first_tab = test_now();
    let second = first_tab + Duration::from_secs(6 * 60);
    let mut g = game(detect::MatchOutcome::Unknown, None, first_tab);
    g.map = Some("Neon Junction".into());
    g.hero_auth.accepted_hero = Some("Wrecking Ball".into());
    g.last_activity = first_tab;
    g.opened_at = first_tab;

    let incoming = IncomingIdentity {
        map: Some("Neon Junction"),
        hero: Some("Wrecking Ball"),
        vote_candidates: &[],
    };
    assert!(
        should_reuse_unfinished_same_match(Some(&g), incoming, second),
        "double capture: merge 21:55 WIN into the 21:49 empty row; no second match"
    );

    // Map-vote after cooldown (default 120s): lingering vote includes the
    // played map. Pre-fix this opened a second session.
    let vote = [String::from("Neon Junction"), String::from("Dorado")];
    assert!(
        !map_vote_should_open_new_game(
            Some(&g),
            Some(first_tab),
            second,
            Duration::from_secs(120),
            &vote,
        ),
        "lingering map-vote 6 min after Tab must not open a second game"
    );

    // Stat-regression split path: unfinished + same identity, age > 120s.
    assert!(
        same_unfinished_match(
            true,
            Duration::from_secs(6 * 60),
            Some("Neon Junction"),
            Some("Wrecking Ball"),
            IncomingIdentity {
                map: Some("Neon Junction"),
                hero: Some("Wrecking Ball"),
                vote_candidates: &[],
            },
        ),
        "stat-looking regression 6 min later must not split this match"
    );

    // Ilios on the career profile is a later, different match, a vote
    // that does not include Neon Junction may still open. Same-map must not.
    let other_vote = [String::from("Ilios"), String::from("Busan")];
    assert!(map_vote_should_open_new_game(
        Some(&g),
        Some(first_tab),
        second,
        Duration::from_secs(120),
        &other_vote,
    ));

    // Outcome already recorded, this is a finished game, not the pair.
    let mut won = game(detect::MatchOutcome::Victory, Some(6 * 60 - 30), second);
    won.map = Some("Neon Junction".into());
    won.hero_auth.accepted_hero = Some("Wrecking Ball".into());
    won.last_activity = first_tab + Duration::from_secs(30);
    assert!(map_vote_should_open_new_game(
        Some(&won),
        Some(first_tab),
        second,
        Duration::from_secs(120),
        &vote,
    ));
}

#[test]
fn map_vote_cooldown_still_blocks_without_identity() {
    // No map/hero on the unfinished game: identity reuse does not apply,
    // so the 120s debounce is still the only guard (lingering vote at
    // start, before any Tab). Tab itself is not delayed.
    let opened = test_now();
    let g = game(detect::MatchOutcome::Unknown, None, opened);
    let vote = [String::from("Oasis")];
    assert!(!map_vote_should_open_new_game(
        Some(&g),
        Some(opened),
        opened + Duration::from_secs(30),
        Duration::from_secs(120),
        &vote,
    ));
    assert!(map_vote_should_open_new_game(
        Some(&g),
        Some(opened),
        opened + Duration::from_secs(121),
        Duration::from_secs(120),
        &vote,
    ));
}

#[test]
fn live_map_vote_uses_the_open_game() {
    let now = test_now();
    let opened = now - Duration::from_secs(400);
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.map = Some("Busan".into());
    g.opened_at = opened;
    g.last_activity = now - Duration::from_secs(30);
    g.gate = Some(GateState {
        accepted: Counters {
            elims: 8,
            assists: 3,
            deaths: 2,
            damage: 1800,
            healing: 400,
            mitigation: 100,
        },
        ..GateState::default()
    });
    let debounce = Duration::from_secs(120);
    let other = vec!["Junkertown".into(), "Ilios".into()];
    let same = vec!["Busan".into(), "Dorado".into()];
    assert!(
        map_vote_should_open_new_game(Some(&g), Some(opened), now, debounce, &other),
        "an early leave plus a different vote closes the unfinished session"
    );
    assert!(!map_vote_should_open_new_game(
        Some(&g),
        Some(opened),
        now,
        debounce,
        &same,
    ));
    let open = boundary::decide_poll(&boundary::PollInput {
        outcome: g.outcome,
        result: g.result_mark,
        pending_boundary: g.pending_boundary,
        awaiting_first_board: g.awaiting_first_board,
        has_board: g.gate.is_some(),
        reset_streak: g.reset_streak,
        map: g.map.as_deref(),
        map_trusted: g.map_is_trusted(),
        hero: g.hero_auth.accepted_hero.as_deref(),
        signal: None,
        signal_confirmed: false,
        accolade_map: None,
        start_screen: Some(boundary::StartScreen::MapVote {
            candidates: other.clone(),
        }),
        block_map_vote: !map_vote_should_open_new_game(
            Some(&g),
            Some(opened),
            now,
            debounce,
            &other,
        ),
        now,

        deferred: false,
        text_fallback_locked: false,
    });
    match open {
        boundary::PollDecision::Open(opened_game) => {
            assert_eq!(opened_game.reason, boundary::CloseReason::MapVote);
            assert!(opened_game.seal_outcome.is_none());
            assert_eq!(opened_game.candidates, other);
        }
        other => panic!("expected a split, got {other:?}"),
    }
    let blocked = boundary::decide_poll(&boundary::PollInput {
        outcome: g.outcome,
        result: g.result_mark,
        pending_boundary: g.pending_boundary,
        awaiting_first_board: g.awaiting_first_board,
        has_board: g.gate.is_some(),
        reset_streak: g.reset_streak,
        map: g.map.as_deref(),
        map_trusted: g.map_is_trusted(),
        hero: g.hero_auth.accepted_hero.as_deref(),
        signal: None,
        signal_confirmed: false,
        accolade_map: None,
        start_screen: Some(boundary::StartScreen::MapVote {
            candidates: same.clone(),
        }),
        block_map_vote: !map_vote_should_open_new_game(
            Some(&g),
            Some(opened),
            now,
            debounce,
            &same,
        ),
        now,

        deferred: false,
        text_fallback_locked: false,
    });
    assert!(
        !matches!(blocked, boundary::PollDecision::Open(_)),
        "the same-map vote does not open a second session"
    );
}

#[test]
fn active_game_persists_and_recovers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut g = ActiveGame::open_now(
        "abc123".into(),
        detect::MatchOutcome::Unknown,
        vec!["Oasis".into(), "Busan".into()],
    );
    g.session_created = true;
    g.map = Some("Oasis".into());
    g.map_source = Some(boundary::MapSource::TopBar);
    let gate_state = GateState {
        accepted: Counters {
            elims: 12,
            assists: 4,
            deaths: 3,
            damage: 5400,
            healing: 900,
            mitigation: 1200,
        },
        last_raw: Counters {
            elims: 12,
            assists: 4,
            deaths: 3,
            damage: 5400,
            healing: 900,
            mitigation: 1200,
        },
        ..Default::default()
    };
    g.gate = Some(gate_state);
    g.last_stats_at = Some(Instant::now());
    g.note_result(detect::MatchOutcome::Defeat, false);
    g.pending_boundary = true;
    g.awaiting_first_board = true;
    g.deferred = Some(Counters {
        elims: 1,
        assists: 0,
        deaths: 0,
        damage: 100,
        healing: 0,
        mitigation: 50,
    });
    g.deferred_hero = Some("Wrecking Ball".into());
    g.deferred_at = Some(Utc::now() - chrono::Duration::seconds(30));
    g.deferred_imported = true;
    g.progressed_boards = 2;
    g.baseline_at = Some(Instant::now());
    persist_active_game(dir.path(), Some(&g));

    let r = recover_active_game(dir.path()).expect("recent game recovers");
    assert_eq!(r.session_id, "abc123");
    assert_eq!(r.outcome, detect::MatchOutcome::Unknown);
    assert_eq!(r.map.as_deref(), Some("Oasis"));
    assert_eq!(r.map_source, Some(boundary::MapSource::TopBar));
    assert_eq!(
        r.map_candidates,
        vec!["Oasis".to_string(), "Busan".to_string()]
    );
    assert!(r.session_created);
    assert_eq!(
        r.gate.map(|s| s.accepted.edd()),
        Some((12, 3, 5400)),
        "regression baseline must survive a daemon restart"
    );
    assert_eq!(
        r.gate.map(|s| s.accepted.elims),
        Some(12),
        "gate state (all six counters) survives a daemon restart"
    );
    assert!(r.last_stats_at.is_some());
    assert_eq!(
        r.result_mark.map(|m| (m.outcome, m.confirmed)),
        Some((detect::MatchOutcome::Defeat, false)),
        "a defeat streak must survive a restart so the next queue can close this session"
    );
    assert!(r.pending_boundary);
    assert!(r.awaiting_first_board);
    assert_eq!(r.deferred.map(|c| c.elims), Some(1));
    assert_eq!(r.deferred_hero.as_deref(), Some("Wrecking Ball"));
    assert!(r.deferred_at.is_some());
    assert!(r.deferred_imported);
    assert_eq!(r.progressed_boards, 2);
    assert!(r.baseline_at.is_some());
    assert!(
        start_screen_blocked(
            Some(&r),
            Some(r.opened_at),
            r.opened_at + Duration::from_secs(30),
            Duration::from_secs(120),
            &boundary::StartScreen::HeroSelect,
        ),
        "a recovered session still inside the debounce does not split on select"
    );

    // Clearing removes the file, nothing to recover.
    persist_active_game(dir.path(), None);
    assert!(recover_active_game(dir.path()).is_none());
}

#[test]
fn future_result_timestamp_does_not_drop_the_recovered_game() {
    let dir = tempfile::tempdir().expect("tempdir");
    let g = ActiveGame::open_now("abc123".into(), detect::MatchOutcome::Unknown, Vec::new());
    persist_active_game(dir.path(), Some(&g));
    let path = active_game_path(dir.path());
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["result_outcome"] = serde_json::json!("defeat");
    value["result_confirmed"] = serde_json::json!(false);
    value["result_seen_at"] = serde_json::json!("2999-01-01T00:00:00Z");
    value["pending_boundary"] = serde_json::json!(true);
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let recovered = recover_active_game(dir.path()).expect("a bad hint timestamp is not fatal");
    assert_eq!(recovered.session_id, "abc123");
    assert!(recovered.result_mark.is_none());
    assert!(
        !recovered.pending_boundary,
        "an arm without a recoverable hint must not hold full cadence"
    );
}

#[test]
fn split_session_does_not_inherit_post_match_grace() {
    let now = test_now();
    let opened = session_opened_by_split(
        "fresh".into(),
        detect::MatchOutcome::Unknown,
        HeroAuthState::default(),
        false,
        None,
        None,
        now,
    );
    assert!(!opened.finished());
    assert!(opened.outcome_recorded_at.is_none());
    assert!(
        !should_start_fresh_session(Some(&opened), now + Duration::from_secs(10)),
        "the new session is in progress, not a finished defeat inside the 75s grace"
    );
    let inherited = game(detect::MatchOutcome::Defeat, Some(10), now);
    assert!(inherited.finished());
    assert!(
        !should_start_fresh_session(Some(&inherited), now),
        "a defeat stamped now would swallow the next tab"
    );
}

#[test]
fn identified_row_id_is_the_player_row() {
    assert_eq!(identified_row_id(Some(3)), Some(3));
    assert_eq!(identified_row_id(None), None);
}

#[test]
fn gap_split_keeps_the_frame_outcome_on_the_new_session() {
    let defeat = session_opened_by_split(
        "gap".into(),
        detect::MatchOutcome::Defeat,
        HeroAuthState::default(),
        false,
        None,
        None,
        Instant::now(),
    );
    assert_eq!(defeat.outcome, detect::MatchOutcome::Defeat);
    assert!(defeat.outcome_recorded_at.is_some());
    let reset = session_opened_by_split(
        "reset".into(),
        detect::MatchOutcome::Unknown,
        HeroAuthState {
            career_ever_ok: true,
            accepted_hero: Some("Zenyatta".into()),
            portrait_pending: Some(("Illari".into(), 2)),
        },
        false,
        None,
        None,
        Instant::now(),
    );
    assert!(
        !reset.hero_auth.career_ever_ok,
        "a carried career flag is not this capture's career panel"
    );
    assert!(reset.hero_auth.portrait_pending.is_none());
    assert_eq!(reset.hero_auth.accepted_hero.as_deref(), Some("Zenyatta"));
    let seeded = session_opened_by_split(
        "seeded".into(),
        detect::MatchOutcome::Unknown,
        HeroAuthState {
            career_ever_ok: true,
            accepted_hero: Some("Wrecking Ball".into()),
            portrait_pending: None,
        },
        true,
        None,
        None,
        Instant::now(),
    );
    assert!(seeded.hero_auth.career_ever_ok);
    assert_eq!(
        seeded.hero_auth.accepted_hero.as_deref(),
        Some("Wrecking Ball")
    );
    assert!(!reset.finished());
    assert!(reset.outcome_recorded_at.is_none());
}

#[tokio::test]
async fn held_board_is_inserted_on_the_new_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let held = Counters {
        elims: 2,
        assists: 1,
        deaths: 0,
        damage: 400,
        healing: 80,
        mitigation: 900,
    };
    let played_at = Utc::now() - chrono::Duration::seconds(90);
    store
        .create_session(&storage::MatchSession {
            session_id: "junkertown".into(),
            hero: "Wrecking Ball".into(),
            map_name: "Junkertown".into(),
            role: "Tank".into(),
            started_at: SurrealDatetime::from(played_at),
            last_capture_at: SurrealDatetime::from(played_at),
            capture_count: 0,
            final_outcome: "unknown".into(),
        })
        .await
        .unwrap();
    store_held_board(
        &store,
        dir.path(),
        "junkertown",
        "Wrecking Ball",
        "Junkertown",
        "unknown",
        held,
        played_at,
    )
    .await
    .unwrap();
    let snaps = store.get_session_snapshots("junkertown").await.unwrap();
    assert_eq!(snaps.len(), 1);
    assert_eq!(snaps[0].hero, "Wrecking Ball");
    assert_eq!(snaps[0].elims, 2);
    assert_eq!(snaps[0].damage, 400);
    assert_eq!(snaps[0].outcome, "unknown");
    let stored: chrono::DateTime<Utc> = snaps[0].played_at.into();
    assert!((stored - played_at).num_seconds().abs() < 2);
}

#[tokio::test]
async fn apply_poll_decision_stamps_grace_at_confirm_and_writes_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(test_match("sess-grace", "unknown"))
        .await
        .unwrap();
    let sighting = Instant::now() - Duration::from_secs(200);
    let mut g = game(detect::MatchOutcome::Unknown, None, Instant::now());
    g.session_id = "sess-grace".into();
    g.session_created = true;
    g.result_mark = Some(ResultMark {
        outcome: detect::MatchOutcome::Defeat,
        confirmed: false,
        seen_at: sighting,
    });
    g.pending_boundary = false;
    let mut st = session(Some(g), Instant::now());
    let decision = {
        let g = st.active_game.as_ref().unwrap();
        boundary::decide_poll(&boundary::PollInput {
            outcome: g.outcome,
            result: g.result_mark,
            pending_boundary: g.pending_boundary,
            awaiting_first_board: g.awaiting_first_board,
            has_board: g.gate.is_some(),
            reset_streak: g.reset_streak,
            map: g.map.as_deref(),
            map_trusted: g.map_is_trusted(),
            hero: g.hero_auth.accepted_hero.as_deref(),
            signal: Some(detect::MatchOutcome::Defeat),
            signal_confirmed: true,
            accolade_map: None,
            start_screen: None,
            block_map_vote: false,
            deferred: g.deferred.is_some(),
            text_fallback_locked: g.text_fallback_locked,
            now: Instant::now(),
        })
    };
    apply_poll_decision(&mut st, &store, dir.path(), decision, Instant::now(), false).await;
    let g = st.active_game.expect("the confirm stays on this session");
    assert_eq!(g.outcome, detect::MatchOutcome::Defeat);
    let stamped = g.outcome_recorded_at.expect("grace is stamped");
    assert!(
        stamped.saturating_duration_since(sighting) > Duration::from_secs(150),
        "grace is not the word's first sighting"
    );
    assert!(
        stamped.elapsed() < Duration::from_secs(5),
        "grace starts when apply_poll_decision records the result"
    );
    assert!(!should_start_fresh_session(
        Some(&g),
        stamped + Duration::from_secs(10)
    ));
    assert!(should_start_fresh_session(
        Some(&g),
        stamped + POST_MATCH_GRACE + Duration::from_secs(1)
    ));
    let snaps = store.get_session_snapshots("sess-grace").await.unwrap();
    assert_eq!(snaps.len(), 1);
    assert_eq!(snaps[0].outcome, "defeat");
}

#[tokio::test]
async fn retire_active_game_writes_a_seal_and_leaves_an_unsealed_hint() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store
        .insert_match(test_match("sess-seal", "unknown"))
        .await
        .unwrap();
    store
        .insert_match(test_match("sess-hint", "unknown"))
        .await
        .unwrap();

    let mut sealed = game(detect::MatchOutcome::Unknown, None, Instant::now());
    sealed.session_id = "sess-seal".into();
    sealed.session_created = true;
    sealed.result_mark = Some(ResultMark {
        outcome: detect::MatchOutcome::Defeat,
        confirmed: false,
        seen_at: Instant::now() - Duration::from_secs(20),
    });
    let mut st = session(Some(sealed), Instant::now());
    retire_active_game(
        &mut st,
        &store,
        dir.path(),
        Some(detect::MatchOutcome::Defeat),
        "superseded by hero select/ban",
    )
    .await;
    assert!(st.active_game.is_none());
    let snaps = store.get_session_snapshots("sess-seal").await.unwrap();
    assert_eq!(snaps[0].outcome, "defeat");

    let mut hinted = game(detect::MatchOutcome::Unknown, None, Instant::now());
    hinted.session_id = "sess-hint".into();
    hinted.session_created = true;
    hinted.result_mark = Some(ResultMark {
        outcome: detect::MatchOutcome::Victory,
        confirmed: false,
        seen_at: Instant::now() - Duration::from_secs(10),
    });
    let mut st = session(Some(hinted), Instant::now());
    retire_active_game(
        &mut st,
        &store,
        dir.path(),
        None,
        "superseded by stat reset",
    )
    .await;
    assert!(st.active_game.is_none());
    let snaps = store.get_session_snapshots("sess-hint").await.unwrap();
    assert_eq!(
        snaps[0].outcome, "unknown",
        "retire with no seal leaves the hint unstored"
    );
}

/// A game close with no active game still clears the rank-screen log,
/// so the next game logs its first rank result even when it matches.
/// Draw is used because no other test here reads a draw rank screen.
#[tokio::test]
async fn retire_with_no_active_game_still_clears_the_rank_log() {
    use stat_tracker::detect::match_end::{clear_rank_result_log, rank_result_is_new};
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut st = session(None, Instant::now());
    let _ = rank_result_is_new(detect::MatchOutcome::Draw);
    assert!(
        !rank_result_is_new(detect::MatchOutcome::Draw),
        "a repeat of the same rank result stays quiet"
    );
    retire_active_game(&mut st, &store, dir.path(), None, "test: no game").await;
    assert!(st.active_game.is_none());
    assert!(
        rank_result_is_new(detect::MatchOutcome::Draw),
        "the close cleared the rank log with no active game"
    );
    clear_rank_result_log();
}

#[test]
fn stale_persisted_game_is_not_recovered() {
    let dir = tempfile::tempdir().expect("tempdir");
    let stale = PersistedGame {
        rejoin_carry: None,
        replaced_text_fallback_map: None,
        best_guess: None,
        session_id: "old".into(),
        outcome: detect::MatchOutcome::Unknown,
        map: None,
        map_source: None,
        map_candidates: Vec::new(),
        session_created: true,
        opened_at: Utc::now() - chrono::Duration::hours(9),
        last_activity: Utc::now() - chrono::Duration::hours(8),
        outcome_recorded_at: None,
        gate: None,
        last_stats_at: None,
        hero_auth: HeroAuthState::default(),
        result_outcome: None,
        result_confirmed: false,
        result_seen_at: None,
        pending_boundary: false,
        awaiting_first_board: false,
        reset_streak: 0,
        reset_baseline: None,
        baseline_row: None,
        deferred: None,
        deferred_hero: None,
        deferred_at: None,
        deferred_imported: false,
        progressed_boards: 0,
        baseline_at: None,
        text_fallback_locked: false,
    };
    std::fs::write(
        active_game_path(dir.path()),
        serde_json::to_vec(&stale).unwrap(),
    )
    .unwrap();
    assert!(
        recover_active_game(dir.path()).is_none(),
        "yesterday's unfinished game must not swallow today's captures"
    );
}

#[test]
fn awaiting_first_board_does_not_latch_a_carried_gate() {
    let carried = (GateState::default(), Duration::from_secs(40));
    assert!(
        gate_prev_for_store(Some(carried), true).is_none(),
        "a session with no board of its own does not latch the previous game"
    );
    let own = (GateState::default(), Duration::from_secs(40));
    assert!(gate_prev_for_store(Some(own), false).is_some());
}

#[test]
fn a_recovered_awaiting_session_keeps_the_debounce() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut g = ActiveGame::open_now(
        "await".into(),
        detect::MatchOutcome::Unknown,
        vec!["Busan".into()],
    );
    g.awaiting_first_board = true;
    persist_active_game(dir.path(), Some(&g));
    let st = startup_session(dir.path());
    let opened = st
        .last_game_open
        .expect("startup restores the debounce from opened_at");
    assert_eq!(opened, st.active_game.as_ref().unwrap().opened_at);
    let screen = boundary::StartScreen::HeroBan;
    let debounce = Duration::from_secs(120);
    let now = opened + Duration::from_secs(30);
    assert!(
        start_screen_blocked(
            st.active_game.as_ref(),
            Some(opened),
            now,
            debounce,
            &screen
        ),
        "a ban inside the restored debounce does not split the awaiting session"
    );
    let mut resumed = startup_session(dir.path());
    resumed.last_game_open = None;
    let recovered = startup_session(dir.path());
    resumed.last_game_open = recovered.last_game_open;
    resumed.active_game = recovered.active_game;
    assert!(start_screen_blocked(
        resumed.active_game.as_ref(),
        resumed.last_game_open,
        now,
        debounce,
        &screen,
    ));
    let game = resumed.active_game.as_ref().unwrap();
    let decision = boundary::decide_poll(&boundary::PollInput {
        outcome: game.outcome,
        result: game.result_mark,
        pending_boundary: game.pending_boundary,
        awaiting_first_board: game.awaiting_first_board,
        has_board: game.gate.is_some(),
        reset_streak: game.reset_streak,
        map: game.map.as_deref(),
        map_trusted: game.map_is_trusted(),
        hero: game.hero_auth.accepted_hero.as_deref(),
        signal: None,
        signal_confirmed: false,
        accolade_map: None,
        start_screen: Some(screen),
        block_map_vote: true,
        deferred: game.deferred.is_some(),
        text_fallback_locked: game.text_fallback_locked,
        now,
    });
    assert!(
        !matches!(decision, boundary::PollDecision::Open(_)),
        "the recovered session stays through the ban"
    );
}

#[test]
fn a_suspend_resume_keeps_the_debounce() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut g = ActiveGame::open_now(
        "await".into(),
        detect::MatchOutcome::Unknown,
        vec!["Busan".into()],
    );
    g.awaiting_first_board = true;
    persist_active_game(dir.path(), Some(&g));
    let mut st = startup_session(dir.path());
    st.last_game_open = None;
    st.pending_outcome = Some((detect::MatchOutcome::Victory, Instant::now()));
    resume_after_suspend(&mut st, dir.path());
    assert!(
        st.pending_outcome.is_none(),
        "resume drops the volatile pending outcome"
    );
    let opened = st
        .last_game_open
        .expect("resume restores the debounce from opened_at");
    assert_eq!(opened, st.active_game.as_ref().unwrap().opened_at);
    let screen = boundary::StartScreen::HeroBan;
    let now = opened + Duration::from_secs(30);
    assert!(
        start_screen_blocked(
            st.active_game.as_ref(),
            st.last_game_open,
            now,
            Duration::from_secs(120),
            &screen,
        ),
        "a ban inside the restored debounce does not split"
    );
}

#[test]
fn a_pre_0_4_19_skeleton_with_a_map_is_untrusted_text() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut g = ActiveGame::open_now("old".into(), detect::MatchOutcome::Unknown, Vec::new());
    g.map = Some("Busan".into());
    g.map_source = Some(boundary::MapSource::TopBar);
    persist_active_game(dir.path(), Some(&g));
    let path = active_game_path(dir.path());
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("map_source");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let recovered = recover_active_game(dir.path()).expect("recent skeleton recovers");
    assert_eq!(recovered.map.as_deref(), Some("Busan"));
    assert_eq!(
        recovered.map_source,
        Some(boundary::MapSource::TextFallback),
        "a map with no source is untrusted text"
    );

    g.map = None;
    g.map_source = None;
    persist_active_game(dir.path(), Some(&g));
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("map_source");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let recovered = recover_active_game(dir.path()).unwrap();
    assert!(recovered.map.is_none());
    assert!(
        recovered.map_source.is_none(),
        "a skeleton with no map stays unsourced"
    );
}

#[test]
fn a_restart_mid_game_keeps_the_best_guess() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut g = ActiveGame::open_now("guess".into(), detect::MatchOutcome::Unknown, Vec::new());
    let mut row = test_match("guess", "victory");
    row.suspect_fields = vec!["h".into()];
    g.best_guess = Some(row.clone());
    persist_active_game(dir.path(), Some(&g));
    let recovered = recover_active_game(dir.path()).expect("recent skeleton recovers");
    assert_eq!(recovered.best_guess, Some(row));
}

#[test]
fn a_persisted_text_fallback_lock_survives_recovery() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut g = ActiveGame::open_now("locked".into(), detect::MatchOutcome::Unknown, Vec::new());
    g.map = Some("Dorado".into());
    g.map_source = Some(boundary::MapSource::TextFallback);
    g.text_fallback_locked = true;
    persist_active_game(dir.path(), Some(&g));
    let recovered = recover_active_game(dir.path()).expect("recent skeleton recovers");
    assert!(
        recovered.text_fallback_locked,
        "a lock written to disk must still be set after recovery"
    );
    assert_eq!(recovered.map.as_deref(), Some("Dorado"));
}

/// F252-1: the text-fallback name a banner read replaced survives a
/// restart, so the game's accolade still counts as this game.
#[test]
fn a_replaced_text_fallback_name_survives_recovery() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut g = ActiveGame::open_now("renamed".into(), detect::MatchOutcome::Unknown, Vec::new());
    g.map = Some("Ilios".into());
    g.map_source = Some(boundary::MapSource::TopBar);
    g.replaced_text_fallback_map = Some("Busan".into());
    persist_active_game(dir.path(), Some(&g));
    let recovered = recover_active_game(dir.path()).expect("recent skeleton recovers");
    assert_eq!(
        recovered.replaced_text_fallback_map.as_deref(),
        Some("Busan")
    );
    // an older skeleton without the field reads as no rename
    let path = active_game_path(dir.path());
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("replaced_text_fallback_map");
    std::fs::write(&path, value.to_string()).unwrap();
    let old = recover_active_game(dir.path()).expect("older skeleton recovers");
    assert_eq!(old.replaced_text_fallback_map, None);
}

#[test]
fn an_unmappable_hint_timestamp_keeps_the_text_fallback_lock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut g = ActiveGame::open_now("locked".into(), detect::MatchOutcome::Unknown, Vec::new());
    g.map = Some("Dorado".into());
    g.map_source = Some(boundary::MapSource::TextFallback);
    g.text_fallback_locked = true;
    g.result_mark = Some(ResultMark {
        outcome: detect::MatchOutcome::Defeat,
        confirmed: false,
        seen_at: Instant::now(),
    });
    persist_active_game(dir.path(), Some(&g));
    let path = active_game_path(dir.path());
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let future = Utc::now() + chrono::Duration::hours(48);
    value["result_seen_at"] = serde_json::Value::String(future.to_rfc3339());
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let recovered = recover_active_game(dir.path()).expect("the session still loads");
    assert!(
        recovered.text_fallback_locked,
        "dropping the hint must not drop the lock"
    );
    assert!(recovered.result_mark.is_none());
}

#[test]
fn a_main_0_4_18_skeleton_recovers_as_untrusted_unlocked_text() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A whole-second RFC3339 stamp has no fraction. Recovery must still
    // accept it. Anything older than the unfinished idle bound is stale.
    let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    assert!(
        !now.contains('.'),
        "whole seconds have no fraction, got {now}"
    );
    // The eleven keys a 0.4.18 skeleton actually stored, with a mid-match
    // gate and the three hero-authority keys. Later fields stay absent so
    // a missing default fails this load.
    let skeleton = serde_json::json!({
        "session_id": "old",
        "outcome": "unknown",
        "map": "Busan",
        "map_candidates": [],
        "session_created": true,
        "opened_at": now,
        "last_activity": now,
        "outcome_recorded_at": null,
        "gate": {
            "accepted": {
                "elims": 14,
                "assists": 22,
                "deaths": 6,
                "damage": 2400,
                "healing": 9800,
                "mitigation": 400
            },
            "last_raw": {
                "elims": 14,
                "assists": 22,
                "deaths": 6,
                "damage": 2400,
                "healing": 9800,
                "mitigation": 400
            },
            "down_streak_len": [0, 0, 0, 0, 0, 0],
            "down_streak_last": [0, 0, 0, 0, 0, 0],
            "last_raw_suspect": [false, false, false, false, false, false]
        },
        "last_stats_at": now,
        "hero_auth": {
            "career_ever_ok": true,
            "accepted_hero": "Zenyatta",
            "portrait_pending": null
        }
    });
    assert_eq!(
        skeleton.as_object().expect("object").len(),
        11,
        "a 0.4.18 skeleton has these eleven keys and no later fields"
    );
    assert_eq!(
        skeleton["hero_auth"].as_object().expect("hero_auth").len(),
        3
    );
    std::fs::write(
        active_game_path(dir.path()),
        serde_json::to_vec(&skeleton).unwrap(),
    )
    .unwrap();
    let recovered = recover_active_game(dir.path()).expect("a 0.4.18 skeleton recovers");
    assert_eq!(recovered.map.as_deref(), Some("Busan"));
    assert_eq!(
        recovered.map_source,
        Some(boundary::MapSource::TextFallback),
        "a map with no source is untrusted text"
    );
    assert!(!recovered.text_fallback_locked);
    assert!(recovered.result_mark.is_none());
    assert!(!recovered.pending_boundary);
    assert!(!recovered.awaiting_first_board);
    assert!(recovered.session_created);
    assert!(recovered.last_stats_at.is_some());
    assert_eq!(recovered.gate.map(|gate| gate.accepted.elims), Some(14));
    assert!(recovered.hero_auth.career_ever_ok);
    assert_eq!(
        recovered.hero_auth.accepted_hero.as_deref(),
        Some("Zenyatta")
    );
    assert!(recovered.hero_auth.portrait_pending.is_none());
    let gate = recovered.gate.expect("skeleton gate");
    assert!(
        !gate.low_trust,
        "a pre-0.4.21 skeleton has no low-trust bit, so the latch stays trusted"
    );
    assert!(!gate.unconfirmed.iter().any(|&col| col));
}
