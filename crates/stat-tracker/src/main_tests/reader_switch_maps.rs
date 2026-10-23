//! Daemon tests: reader_switch_maps. See `mod.rs`.

use super::*;

#[test]
fn a_skipped_capture_report_uses_the_plan_outcome_and_the_callers_career_flag() {
    let plan = boundary::CapturePlan {
        split: false,
        defer: true,
        ignore_row: false,
        skip_store: true,
        clear_hint: false,
        reset_streak: 1,
        reset_baseline: None,
        baseline_row: Some(2),
        refresh_baseline: false,
        count_progress: false,
        stored_outcome: detect::MatchOutcome::Victory,
        deferred_counters: Some(night_counters(2, 1, 0, 100, 40, 10)),
        seal: None,
        close_reason: None,
    };
    let report = skipped_capture_report(
        &plan,
        "sid",
        true,
        HeroAuthState::default(),
        Some("Zenyatta".into()),
        None,
    );
    assert_eq!(report.outcome, detect::MatchOutcome::Victory);
    assert!(report.career_panel);
    assert!(!report.recorded);
    assert!(report.armed_reset);
    assert_eq!(report.held_counters.map(|c| c.elims), Some(2));
    assert_eq!(report.session_id, "sid");
}

#[test]
fn the_confirming_tick_uses_the_carried_map() {
    let now = test_now();
    let mut streak = None;
    let first = resolve_word_tick(
        &mut streak,
        Some((
            detect::MatchOutcome::Victory,
            detect::match_end::OutcomeSource::ResultWord,
        )),
        now,
        Some("Junkertown".into()),
    );
    assert!(!first.signal_confirmed);
    assert_eq!(first.accolade_map.as_deref(), Some("Junkertown"));
    let second = resolve_word_tick(
        &mut streak,
        Some((
            detect::MatchOutcome::Victory,
            detect::match_end::OutcomeSource::ResultWord,
        )),
        now + Duration::from_secs(4),
        None,
    );
    assert!(second.signal_confirmed);
    assert_eq!(
        second.accolade_map.as_deref(),
        Some("Junkertown"),
        "the confirming tick keeps the first read's map"
    );
}

#[test]
fn a_carried_map_requires_the_same_outcome() {
    let now = test_now();
    let mut streak = None;
    note_word_streak(
        &mut streak,
        detect::MatchOutcome::Victory,
        now,
        Some("Junkertown".into()),
    );
    let dropped = note_word_streak(
        &mut streak,
        detect::MatchOutcome::Defeat,
        now + Duration::from_secs(4),
        None,
    );
    assert!(
        dropped.is_none(),
        "a different outcome must not keep the previous map"
    );
}

#[test]
fn a_carried_map_expires_with_the_confirm_window() {
    let now = test_now();
    let mut fresh = None;
    note_word_streak(
        &mut fresh,
        detect::MatchOutcome::Victory,
        now,
        Some("Junkertown".into()),
    );
    assert_eq!(
        note_word_streak(
            &mut fresh,
            detect::MatchOutcome::Victory,
            now + Duration::from_secs(30),
            None,
        )
        .as_deref(),
        Some("Junkertown")
    );
    let mut stale = None;
    note_word_streak(
        &mut stale,
        detect::MatchOutcome::Victory,
        now,
        Some("Junkertown".into()),
    );
    assert!(
        note_word_streak(
            &mut stale,
            detect::MatchOutcome::Victory,
            now + OUTCOME_CONFIRM_WINDOW + Duration::from_secs(1),
            None,
        )
        .is_none(),
        "a map older than the confirm window is not carried"
    );
    let mut edge = None;
    note_word_streak(
        &mut edge,
        detect::MatchOutcome::Victory,
        now,
        Some("Junkertown".into()),
    );
    assert_eq!(
        note_word_streak(
            &mut edge,
            detect::MatchOutcome::Victory,
            now + OUTCOME_CONFIRM_WINDOW,
            None,
        )
        .as_deref(),
        Some("Junkertown"),
        "a map exactly as old as the confirm window is still carried"
    );
    assert_eq!(
        poll_debug_hit(
            Some((
                detect::MatchOutcome::Victory,
                detect::match_end::OutcomeSource::ResultWord,
            )),
            Some((detect::MatchOutcome::Victory, OUTCOME_CONFIRM_WINDOW)),
            OUTCOME_CONFIRM_WINDOW,
        ),
        Some((PollDebugHit::Confirm, detect::MatchOutcome::Victory)),
        "the confirm window's exact edge agrees with the carried map"
    );
}

#[test]
fn a_text_fallback_poll_input_is_not_map_trusted() {
    let now = test_now();
    let mut game =
        ActiveGame::open_at("sid".into(), detect::MatchOutcome::Unknown, Vec::new(), now);
    game.map = Some("Busan".into());
    game.map_source = Some(boundary::MapSource::TextFallback);
    let input = poll_input_from_game(&game, None, false, None, None, false, now);
    assert!(
        !input.map_trusted,
        "a text fallback is absent on the poll path"
    );
    game.map_source = Some(boundary::MapSource::TopBar);
    let input = poll_input_from_game(&game, None, false, None, None, false, now);
    assert!(input.map_trusted);
}

/// `reader = "ocr-v1"` (the default) never reads the banner: the top-bar
/// map is the OCR label, read or not.
#[test]
fn ocr_v1_never_reads_the_banner_for_the_top_bar_map() {
    let called = std::cell::Cell::new(false);
    let read = || {
        called.set(true);
        Some(banner_read("ilios", false))
    };
    let t = choose_top_bar_map(false, read, Some("Busan".into()));
    assert_eq!(t.map.as_deref(), Some("Busan"));
    assert!(!t.from_banner);
    assert_eq!(t.banner, None);
    assert_eq!(choose_top_bar_map(false, read, None).map, None);
    assert!(!called.get(), "the banner was read under ocr-v1");
}

#[test]
fn the_new_reader_takes_a_sure_banner_read_over_the_ocr_label() {
    let t = choose_top_bar_map(
        true,
        || Some(banner_read("ilios", false)),
        Some("Busan".into()),
    );
    assert_eq!(t.map.as_deref(), Some("Ilios"));
    assert!(t.from_banner);
    assert_eq!(
        t.banner,
        Some(Some(banner_read("ilios", false))),
        "kept for the board"
    );
    let top = choose_top_bar_map(true, || Some(banner_read("ilios", false)), None).map;
    assert_eq!(top.as_deref(), Some("Ilios"));
    // The banner read is a top-bar read: a session without a map stores
    // it as MapSource::TopBar, and a session map still wins.
    assert_eq!(
        resolved_map(None, top.as_deref(), "Nepal", &[]),
        ("Ilios".to_string(), Some(boundary::MapSource::TopBar))
    );
    assert_eq!(
        resolved_map(Some("Busan"), top.as_deref(), "", &[]),
        ("Busan".to_string(), None)
    );
    // the vote-candidate veto still applies
    let candidates = vec!["Oasis".to_string(), "Busan".to_string()];
    assert_eq!(
        resolved_map(None, top.as_deref(), "Oasis", &candidates),
        ("Oasis".to_string(), Some(boundary::MapSource::TextFallback))
    );
}

/// An abstaining banner (no pack, a flagged read, an unknown name:
/// `shadow::top_bar_map` returns `None`) leaves the OCR label in place.
/// The read itself is still handed on, so the board does not read the
/// frame again.
#[test]
fn an_abstaining_banner_falls_back_to_the_ocr_label() {
    let flagged = choose_top_bar_map(
        true,
        || Some(banner_read("ilios", true)),
        Some("Busan".into()),
    );
    assert_eq!(flagged.map.as_deref(), Some("Busan"));
    assert!(!flagged.from_banner);
    assert_eq!(flagged.banner, Some(Some(banner_read("ilios", true))));
    let no_pack = choose_top_bar_map(true, || None, Some("Busan".into()));
    assert_eq!(no_pack.map.as_deref(), Some("Busan"));
    assert_eq!(
        no_pack.banner,
        Some(None),
        "no pack: the board reads no map either"
    );
    assert_eq!(choose_top_bar_map(true, || None, None).map, None);
}

/// F251-2: a panic in the banner reader is an abstain. The Tab keeps
/// the OCR label, and the board reads the banner again under its own
/// `catch_unwind`.
#[test]
fn a_panicking_banner_reader_falls_back_to_the_ocr_label() {
    let t = choose_top_bar_map(
        true,
        || -> Option<shadow::banner::MapRead> { panic!("banner reader") },
        Some("Busan".into()),
    );
    assert_eq!(t.map.as_deref(), Some("Busan"));
    assert!(!t.from_banner);
    assert_eq!(t.banner, None);
}

/// Both outcome reads take the FINAL SCORE source from the reader switch
/// (alpha.7 O3): the poll tick and the Tab frame's vision pass. The Tab
/// pass no longer reads the scoreboard header (O4) and makes no ocr-v1
/// only outcome call. Whitespace is ignored.
#[test]
fn the_outcome_reads_take_the_final_score_source_from_the_reader_switch() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let full = include_str!("../main.rs");
    let src = full.split("\nmod tests {").next().unwrap();
    let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
    let body = norm(src[at..].split("\n}\n").next().unwrap());
    assert!(
        body.contains("let p4 = detect::match_end::FinalScoreSource::for_reader(use_new_reader);")
    );
    // one end-title read per Tab frame (F257-3)
    assert!(body.contains(
        "let frame_outcome = if allow_banner_recovery { \
         detect::match_end::detect_outcome_with(&img, p4, &titles, &words) } else { \
         detect::match_end::detect_outcome_text_with(&img, p4, &titles) };"
    ));
    assert_eq!(body.matches("detect::match_end::detect_outcome").count(), 2);
    let all = norm(src);
    assert!(all.contains(
        "detect::match_end::detect_outcome_signal_polled( &img, &rgb, &mut stability, \
         detect::match_end::FinalScoreSource::for_reader(use_new_reader), &titles, &words, );"
    ));
    for ocr_only in [
        "detect::match_end::detect_outcome(",
        "detect::match_end::detect_outcome_text(",
        "detect::match_end::detect_outcome_signal(",
        "detect::match_end::detect_outcome_signal_with_rgb(",
    ] {
        assert!(!all.contains(ocr_only), "{ocr_only}");
    }
}

/// `reader = "ocr-v1"` (the default): the accolade map is the title OCR,
/// on a result-word tick only, and the map pack is never read.
#[test]
fn ocr_v1_reads_the_accolade_map_by_ocr_only() {
    use detect::match_end::OutcomeSource as S;
    let pack = || -> shadow::AccoladeRead { panic!("map pack read under ocr-v1") };
    let ocr = || Some("Busan".to_string());
    assert_eq!(
        choose_accolade_map(false, false, Some(S::ResultWord), pack, ocr).as_deref(),
        Some("Busan")
    );
    assert_eq!(
        choose_accolade_map(false, false, Some(S::ResultWord), pack, || None),
        None
    );
    // no result word on the tick, or Tab cell OCR in flight: nothing runs
    let no_ocr = || -> Option<String> { panic!("accolade OCR off a result-word tick") };
    for signal in [None, Some(S::RankScreen), Some(S::EndTitle)] {
        assert_eq!(
            choose_accolade_map(false, false, signal, pack, no_ocr),
            None
        );
    }
    assert_eq!(
        choose_accolade_map(false, true, Some(S::ResultWord), pack, no_ocr),
        None
    );
}

/// `reader = "new"` with a map pack: the accolade map is the pack's
/// read, on the same ticks, and the OCR is never run, not even when the
/// pack abstains.
#[test]
fn the_new_reader_reads_the_accolade_map_from_the_pack_and_never_by_ocr() {
    use detect::match_end::OutcomeSource as S;
    let ocr = || -> Option<String> { panic!("accolade OCR under reader = new") };
    assert_eq!(
        choose_accolade_map(
            true,
            false,
            Some(S::ResultWord),
            || accolade_read("Ilios"),
            ocr
        )
        .as_deref(),
        Some("Ilios")
    );
    // an abstain is no map: no fallback
    assert_eq!(
        choose_accolade_map(
            true,
            false,
            Some(S::ResultWord),
            || shadow::AccoladeRead::Abstain,
            ocr
        ),
        None
    );
    // the pack is read on a result-word tick only (a counter, not a
    // panic: a panic in the pack read is caught and would hide a call)
    let reads = std::cell::Cell::new(0);
    let counted = || {
        reads.set(reads.get() + 1);
        accolade_read("Ilios")
    };
    for signal in [None, Some(S::RankScreen), Some(S::EndTitle)] {
        assert_eq!(choose_accolade_map(true, false, signal, counted, ocr), None);
    }
    assert_eq!(
        choose_accolade_map(true, true, Some(S::ResultWord), counted, ocr),
        None
    );
    assert_eq!(
        reads.get(),
        0,
        "the map pack was read off a result-word tick"
    );
}

/// `reader = "new"` with no map pack loaded (F265-3): nothing read the
/// frame, so the OCR reads it, as it does under ocr-v1 and as the top
/// bar, the FINAL SCORE line and the phase titles do without their
/// packs. Once per result-word tick, on those ticks only, and the
/// warning is logged once per process.
#[test]
fn the_new_reader_reads_the_accolade_map_by_ocr_when_no_map_pack_is_loaded() {
    use detect::match_end::OutcomeSource as S;
    let no_pack = || shadow::AccoladeRead::NoPack;
    let calls = std::cell::Cell::new(0);
    let ocr = |map: Option<&str>| {
        let map = map.map(str::to_string);
        let calls = &calls;
        move || {
            calls.set(calls.get() + 1);
            map
        }
    };
    assert_eq!(
        choose_accolade_map(
            true,
            false,
            Some(S::ResultWord),
            no_pack,
            ocr(Some("Busan"))
        )
        .as_deref(),
        Some("Busan")
    );
    assert_eq!(calls.get(), 1, "one OCR read for the tick");
    // the OCR finds no map either: no map
    assert_eq!(
        choose_accolade_map(true, false, Some(S::ResultWord), no_pack, ocr(None)),
        None
    );
    assert_eq!(calls.get(), 2);
    // off a result-word tick, or with Tab cell OCR in flight: nothing runs
    for (outcome_only, signal) in [
        (false, None),
        (false, Some(S::RankScreen)),
        (false, Some(S::EndTitle)),
        (true, Some(S::ResultWord)),
    ] {
        let pack = || -> shadow::AccoladeRead { panic!("pack read off a result-word tick") };
        assert_eq!(
            choose_accolade_map(true, outcome_only, signal, pack, ocr(Some("Busan"))),
            None
        );
    }
    assert_eq!(calls.get(), 2, "no OCR off a result-word tick");
    // the fallback above has logged its warning; it does not log again
    assert!(!warn_accolade_map_ocr_fallback());
    assert!(!warn_accolade_map_ocr_fallback());
}

/// A panic in the accolade map reader is an abstain: the tick keeps its
/// result word (the poll closure does not unwind) and the OCR stays off.
#[test]
fn a_panicking_accolade_reader_is_no_map() {
    use detect::match_end::OutcomeSource as S;
    let map = choose_accolade_map(
        true,
        false,
        Some(S::ResultWord),
        || -> shadow::AccoladeRead { panic!("accolade reader") },
        || -> Option<String> { panic!("accolade OCR under reader = new") },
    );
    assert_eq!(map, None);
}

/// The poll tick reads the accolade map only through
/// `choose_accolade_map`, with the daemon's reader switch, and the
/// member-edit rule reaches `apply_poll_decision` with the same switch.
#[test]
fn the_poll_tick_chooses_the_accolade_map_with_the_reader_switch() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let src = include_str!("../main.rs");
    let at = src.find("\nasync fn run_loop(").expect("run_loop");
    let body = norm(src[at..].split("\n}\n").next().unwrap());
    assert!(
        body.contains(
            "choose_accolade_map( use_new_reader, outcome_only, signal.map(|(_, source)| source), || shadow::read_accolade_map(&rgb), || detect::match_end::read_accolade_map(&img), );"
        ),
        "the accolade map goes through choose_accolade_map with the switch"
    );
    assert_eq!(
        body.matches("read_accolade_map").count(),
        2,
        "one call each"
    );
    assert!(body.contains("let use_new_reader = ctx.use_new_reader;"));
    assert!(body.contains("start_screen, now, new_game_debounce, use_new_reader, ) .await;"));
    let tick = src
        .find("\nasync fn apply_poll_tick(")
        .expect("apply_poll_tick");
    let tick_body = norm(src[tick..].split("\n}\n").next().unwrap());
    assert!(
        tick_body
            .contains("apply_poll_decision(st, store, data_dir, decision, now, use_new_reader)")
    );
}

/// `reader = "new"`: the accolade map never renames a game whose map a
/// member chose, and never rewrites the mode stored with that choice.
/// The session still takes the accolade map for its boundary checks.
#[tokio::test]
async fn the_accolade_map_never_renames_a_member_map_pick_under_the_new_reader() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut row = test_match("picked", "unknown");
    row.map_name = String::new();
    store.insert_match(row).await.unwrap();
    store
        .edit_match(
            "picked",
            &storage::MatchEdit {
                map_name: Some("King's Row".into()),
                ..storage::MatchEdit::default()
            },
        )
        .await
        .unwrap();
    let before = store.get_session_snapshots("picked").await.unwrap();
    assert_eq!(before[0].game_mode, "Hybrid");

    let g = adopt_accolade_map(&store, dir.path(), "picked", true).await;
    assert_eq!(g.map.as_deref(), Some("Ilios"));
    assert_eq!(g.map_source, Some(boundary::MapSource::Accolade));
    let rows = store.get_session_snapshots("picked").await.unwrap();
    assert_eq!(
        rows[0].map_name, "",
        "the read under the pick is not renamed"
    );
    assert_eq!(rows[0].display_map_name(), "King's Row");
    assert_eq!(rows[0].game_mode, "Hybrid", "the pick's mode is kept");
    // the outcome of the same tick is still written
    assert_eq!(rows[0].outcome, "victory");
}

/// The member-pick rule is per row (F265-1). A pick made mid-game is on
/// the rows that existed then; the board captured after it, the game's
/// newest row, has none. Under `reader = "new"` the accolade map is
/// written to that row, whether it held nothing or a text fallback, and
/// the picked rows keep their read and their mode. One picked row does
/// not stop the write for the session.
#[tokio::test]
async fn the_accolade_map_is_written_to_the_rows_a_member_did_not_choose() {
    for later_map in ["", "Busan"] {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        insert_game_with_a_mid_game_pick(&store, "mixed", later_map).await;
        for row in store.get_session_snapshots("mixed").await.unwrap() {
            storage::append_match_log(dir.path(), &row);
        }
        let fallback = (!later_map.is_empty()).then_some(later_map);
        let g = adopt_accolade_map_over(&store, dir.path(), "mixed", true, fallback).await;
        assert_eq!(
            g.map.as_deref(),
            Some("Ilios"),
            "session map over {later_map:?}"
        );
        assert_eq!(g.map_source, Some(boundary::MapSource::Accolade));

        let rows = store.get_session_snapshots("mixed").await.unwrap();
        for picked in &rows[..2] {
            assert_eq!(picked.map_name, later_map, "the read under the pick stays");
            assert_eq!(picked.display_map_name(), "King's Row");
            assert_eq!(picked.game_mode, "Hybrid", "the pick's mode is kept");
        }
        let newest = &rows[2];
        assert_eq!(
            newest.map_name, "Ilios",
            "the row after the pick, over {later_map:?}"
        );
        assert_eq!(newest.game_mode, "Control");
        assert_eq!(newest.display_map_name(), "Ilios");
        // the game's row is the newest one: it shows the accolade map
        let shown = storage::latest_per_game(rows.iter().rev().cloned().collect());
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].display_map_name(), "Ilios");
        // the outcome of the same tick is written to every row
        assert!(rows.iter().all(|r| r.outcome == "victory"));
        // the match log is rewritten the same way, row for row
        let log = std::fs::read_to_string(dir.path().join("matches.jsonl")).unwrap();
        let logged: Vec<storage::PersonalMatch> = log
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            logged
                .iter()
                .map(|m| (m.map_name.as_str(), m.game_mode.as_str()))
                .collect::<Vec<_>>(),
            [
                (later_map, "Hybrid"),
                (later_map, "Hybrid"),
                ("Ilios", "Control")
            ]
        );
    }
    // ocr-v1 is unchanged: every row is written, the picked ones too
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_game_with_a_mid_game_pick(&store, "mixed-v1", "").await;
    adopt_accolade_map(&store, dir.path(), "mixed-v1", false).await;
    let rows = store.get_session_snapshots("mixed-v1").await.unwrap();
    assert!(
        rows.iter()
            .all(|r| r.map_name == "Ilios" && r.game_mode == "Control")
    );
}

/// The per-row write itself: every row without a member's map choice
/// takes the map, with the mode and the sync queue of `set_session_map`;
/// a session with no choice at all is written by `set_session_map`.
#[tokio::test]
async fn set_session_map_keeping_member_choices_writes_the_other_rows() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    insert_game_with_a_mid_game_pick(&store, "kept", "").await;
    let before = store.get_session_snapshots("kept").await.unwrap();
    let kept = store
        .set_session_map_keeping_member_choices("kept", "Ilios")
        .await
        .unwrap();
    assert_eq!(kept, 2);
    let rows = store.get_session_snapshots("kept").await.unwrap();
    for (was, now) in before.iter().zip(&rows).take(2) {
        assert_eq!(
            (&now.map_name, &now.game_mode, now.sync_rev),
            (&was.map_name, &was.game_mode, was.sync_rev),
            "a picked row is not touched"
        );
    }
    assert_eq!(
        (rows[2].map_name.as_str(), rows[2].game_mode.as_str()),
        ("Ilios", "Control")
    );
    assert_eq!(rows[2].sync_rev, before[2].sync_rev + 1);
    // a second write of the same map changes nothing
    store
        .set_session_map_keeping_member_choices("kept", "Ilios")
        .await
        .unwrap();
    let again = store.get_session_snapshots("kept").await.unwrap();
    assert_eq!(again[2].sync_rev, rows[2].sync_rev);

    // no choice in the session: every row, as `set_session_map`
    let mut plain = test_match("plain", "unknown");
    plain.map_name = String::new();
    store.insert_match(plain).await.unwrap();
    assert_eq!(
        store
            .set_session_map_keeping_member_choices("plain", "Ilios")
            .await
            .unwrap(),
        0
    );
    let rows = store.get_session_snapshots("plain").await.unwrap();
    assert_eq!(
        (rows[0].map_name.as_str(), rows[0].game_mode.as_str()),
        ("Ilios", "Control")
    );
}

/// Without a member's pick the accolade map is written under both
/// readers, and `reader = "ocr-v1"` writes it over a pick as before.
#[tokio::test]
async fn the_accolade_map_is_written_when_no_member_chose_the_map() {
    for use_new_reader in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let mut row = test_match("unpicked", "unknown");
        row.map_name = String::new();
        store.insert_match(row).await.unwrap();
        let g = adopt_accolade_map(&store, dir.path(), "unpicked", use_new_reader).await;
        assert_eq!(g.map.as_deref(), Some("Ilios"));
        let rows = store.get_session_snapshots("unpicked").await.unwrap();
        assert_eq!(rows[0].map_name, "Ilios", "reader new = {use_new_reader}");
        assert_eq!(rows[0].game_mode, "Control");
    }
    // ocr-v1 is unchanged: the accolade map is written under a pick too
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut row = test_match("picked-v1", "unknown");
    row.map_name = String::new();
    store.insert_match(row).await.unwrap();
    store
        .edit_match(
            "picked-v1",
            &storage::MatchEdit {
                map_name: Some("King's Row".into()),
                ..storage::MatchEdit::default()
            },
        )
        .await
        .unwrap();
    adopt_accolade_map(&store, dir.path(), "picked-v1", false).await;
    let rows = store.get_session_snapshots("picked-v1").await.unwrap();
    assert_eq!(rows[0].map_name, "Ilios");
    assert_eq!(rows[0].game_mode, "Control");
    assert_eq!(rows[0].display_map_name(), "King's Row");
}

/// The end title (P3) and the end-of-match wake (P5) go through the
/// reader switch (alpha.7 O2): on the poll tick and in the Tab frame's
/// vision pass. Each reads the frame's titles once, and no daemon call
/// site uses the wake entry point that is hard-wired to OCR. Every
/// broken wire is named. Whitespace is ignored.
#[test]
fn the_end_titles_go_through_the_reader_switch() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let full = include_str!("../main.rs");
    let src = full.split("\nmod tests {").next().unwrap();
    let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
    let body = norm(src[at..].split("\n}\n").next().unwrap());
    let all = norm(src);
    let checks = [
        (
            "the Tab pass makes its titles from the switch",
            body.contains(
                "let titles = detect::match_end::Titles::for_frame(use_new_reader, &img);",
            ),
        ),
        (
            "the Tab pass hands them to both outcome reads",
            body.contains("detect::match_end::detect_outcome_with(&img, p4, &titles, &words)")
                && body.contains("detect::match_end::detect_outcome_text_with(&img, p4, &titles)"),
        ),
        (
            "the poll tick makes its titles from the switch",
            all.contains("let titles = detect::match_end::Titles::for_rgb(use_new_reader, &rgb);"),
        ),
        (
            "the poll tick hands them to the outcome signal",
            all.contains(
                "detect::match_end::detect_outcome_signal_polled( &img, &rgb, &mut stability, \
                 detect::match_end::FinalScoreSource::for_reader(use_new_reader), &titles, \
                 &words, );",
            ),
        ),
        (
            "the poll tick hands them to the wake",
            all.contains("detect::match_end::detect_end_reel_with(&img, &rgb, &titles);")
                && !all.contains("detect::match_end::detect_end_reel("),
        ),
        (
            "titles are made in those two places only",
            all.matches("detect::match_end::Titles::").count() == 2
                && !all.contains("shadow::read_end_title"),
        ),
    ];
    let broken: Vec<&str> = checks.iter().filter(|c| !c.1).map(|c| c.0).collect();
    assert!(broken.is_empty(), "{broken:?}");
}

/// The accolade and rank-screen result words (P1, P2) go through the
/// reader switch (alpha.7 O11): on the poll tick and in the Tab frame's
/// vision pass. Each makes its `ResultWords` from the switch and hands
/// it to the outcome read, and the poll tick gives the read it took to
/// the result evidence. Every broken wire is named. Whitespace is
/// ignored.
#[test]
fn the_result_words_go_through_the_reader_switch() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let full = include_str!("../main.rs");
    let src = full.split("\nmod tests {").next().unwrap();
    let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
    let body = norm(src[at..].split("\n}\n").next().unwrap());
    let all = norm(src);
    // everything but the Tab pass
    let rest = all.replacen(body.as_str(), "", 1);
    assert!(rest.len() < all.len(), "the Tab pass is part of the source");
    let from_switch = "let words = detect::match_end::ResultWords::for_reader(use_new_reader);";
    let checks = [
        (
            "the Tab pass makes its result words from the switch",
            body.matches(from_switch).count() == 1,
        ),
        (
            "the Tab pass hands them to the outcome read",
            body.contains("detect::match_end::detect_outcome_with(&img, p4, &titles, &words)"),
        ),
        (
            "the poll tick makes its result words from the switch",
            rest.matches(from_switch).count() == 1,
        ),
        (
            "the poll tick hands them to the outcome signal",
            all.contains(
                "detect::match_end::detect_outcome_signal_polled( &img, &rgb, &mut stability, \
                 detect::match_end::FinalScoreSource::for_reader(use_new_reader), &titles, \
                 &words, );",
            ),
        ),
        (
            "the poll tick's result evidence is the read the signal took",
            all.contains(
                "let result_read = tick_result_read( use_new_reader, signal.is_some(), \
                 words.into_taken(), || shadow::read_result(&img), );",
            ),
        ),
        (
            "result words are made in those two places only",
            all.matches("ResultWords::").count() == 2,
        ),
        (
            "the frame's result is read in that one place only",
            all.matches("shadow::read_result(").count() == 1
                && !all.contains("shadow::read_result_screens"),
        ),
    ];
    let broken: Vec<&str> = checks.iter().filter(|c| !c.1).map(|c| c.0).collect();
    assert!(broken.is_empty(), "{broken:?}");
}

/// One words-pack read per poll tick: the read the signal detection took
/// is the tick's result evidence, and the frame is not read again. A
/// tick whose signal came without one (end title, an OCR word) reads
/// the frame as before. No signal, or `reader = "ocr-v1"`:
/// no evidence and no read.
#[test]
fn a_poll_tick_reads_the_frames_result_words_once() {
    use shadow::result::{ACCOLADE, Outcome, RANK_SCREEN, ResultRead};
    let read_of = |source: &str| ResultRead {
        outcome: Some(Outcome::Victory),
        source: Some(source.into()),
        score: 0.9,
        margin: 0.4,
        suspect: false,
    };
    let no_second_read = || -> ResultRead { panic!("the frame was read a second time") };
    let no_read = || -> ResultRead { panic!("the frame was read with no evidence due") };
    // the signal's own read is the evidence
    assert_eq!(
        tick_result_read(true, true, Some(read_of(ACCOLADE)), no_second_read),
        Some(read_of(ACCOLADE))
    );
    // no read was taken: the frame is read here, once
    let reads = std::cell::Cell::new(0);
    let counted = || {
        reads.set(reads.get() + 1);
        read_of(RANK_SCREEN)
    };
    assert_eq!(
        tick_result_read(true, true, None, counted),
        Some(read_of(RANK_SCREEN))
    );
    assert_eq!(reads.get(), 1);
    // no signal: nothing is added, whether or not a read was taken
    assert_eq!(tick_result_read(true, false, None, no_read), None);
    assert_eq!(
        tick_result_read(true, false, Some(read_of(ACCOLADE)), no_read),
        None
    );
    // ocr-v1: never
    assert_eq!(tick_result_read(false, true, None, no_read), None);
    assert_eq!(
        tick_result_read(false, true, Some(read_of(ACCOLADE)), no_read),
        None
    );
    assert_eq!(tick_result_read(false, false, None, no_read), None);
}

/// The accolade map read still starts from a result-word tick under
/// `reader = "new"` (alpha.7 O11 with O5). The new reader's accolade
/// signal carries the source ocr-v1's does (`OutcomeSource::ResultWord`;
/// see `a_sure_words_read_is_the_result_word_signal` in
/// `detect::match_end` and `tests/result_signal_wiring.rs`). The poll
/// tick hands the source of its own signal to `choose_accolade_map`, and
/// that source alone decides whether the map pack is read: a rank-screen
/// word, an end title and no signal read no map.
#[test]
fn a_result_word_tick_still_starts_the_accolade_map_read() {
    use detect::match_end::OutcomeSource as S;
    // The tick's own signal, made with the reader switch's result
    // words, is what the map read is keyed on. Whitespace is ignored.
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let full = include_str!("../main.rs");
    let all = norm(full.split("\nmod tests {").next().unwrap());
    let signal_at = all
        .find("let signal = detect::match_end::detect_outcome_signal_polled(")
        .expect("the poll tick's signal");
    let to_map = all[signal_at..]
        .find(
            "let accolade_map = choose_accolade_map( use_new_reader, outcome_only, \
             signal.map(|(_, source)| source),",
        )
        .expect("the poll tick's accolade map follows its signal and takes its source");
    assert!(all[signal_at..signal_at + to_map].contains("&titles, &words, );"));

    // A result-word signal under the new reader reads the map pack.
    let ocr = || -> Option<String> { panic!("accolade OCR with a map pack loaded") };
    assert_eq!(
        choose_accolade_map(
            true,
            false,
            Some(S::ResultWord),
            || accolade_read("Ilios"),
            ocr
        )
        .as_deref(),
        Some("Ilios")
    );
    // No other source does (a counter, not a panic: a panic in the pack
    // read is caught and would hide a call).
    let reads = std::cell::Cell::new(0);
    for source in [None, Some(S::RankScreen), Some(S::EndTitle)] {
        let counted = || {
            reads.set(reads.get() + 1);
            accolade_read("Ilios")
        };
        assert_eq!(choose_accolade_map(true, false, source, counted, ocr), None);
    }
    assert_eq!(
        reads.get(),
        0,
        "the map pack was read off a result-word tick"
    );
}

/// `reader = "new"` on the poll tick (F263-2, F263-6): a sure end title
/// is nothing but a wake on its first poll, one unconfirmed word (a
/// hint) on its second, and confirmed on its third, the polls ocr-v1
/// needs. One sure read of one frame never reaches the session machine.
#[test]
fn a_sure_end_title_is_a_word_on_its_second_poll_and_confirms_on_its_third() {
    use detect::match_end::{FinalScoreSource, OutcomeSource, Titles};
    use shadow::result::{END_TITLE, Outcome, ResultRead};
    let rgb = image::RgbImage::from_pixel(640, 360, image::Rgb([10, 10, 10]));
    let img = image::DynamicImage::ImageRgb8(rgb.clone());
    let titles = Titles::Words(ResultRead {
        outcome: Some(Outcome::Defeat),
        source: Some(END_TITLE.to_string()),
        score: 0.9,
        margin: 0.4,
        suspect: false,
    });
    let mut stability = detect::stability::FrameStability::default();
    let mut streak = None;
    let t0 = Instant::now();
    let mut ticks = Vec::new();
    for n in 0..3 {
        let signal = detect::match_end::detect_outcome_signal_polled(
            &img,
            &rgb,
            &mut stability,
            FinalScoreSource::Words,
            &titles,
            &detect::match_end::ResultWords::for_reader(true),
        );
        let wake = detect::match_end::detect_end_reel_with(&img, &rgb, &titles);
        let tick = resolve_word_tick(&mut streak, signal, t0 + Duration::from_secs(4 * n), None);
        ticks.push((
            wake,
            signal.map(|s| s.1),
            tick.signal,
            tick.signal_confirmed,
        ));
    }
    let defeat = Some(detect::MatchOutcome::Defeat);
    let title = Some(OutcomeSource::EndTitle);
    assert_eq!(
        ticks,
        vec![
            (true, None, None, false),
            (true, title, defeat, false),
            (true, title, defeat, true),
        ]
    );
}

/// The poll tick confirms the vote, ban and select screens with the
/// source the reader switch picks (alpha.7 O7), and calls no ocr-v1
/// only phase read. Whitespace is ignored.
#[test]
fn the_poll_tick_takes_the_phase_source_from_the_reader_switch() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let full = include_str!("../main.rs");
    let src = norm(full.split("\nmod tests {").next().unwrap());
    assert!(src.contains(
        "detect::match_start::detect_phase_polled( &img, &rgb, &mut stability, \
         detect::match_start::PhaseSource::for_reader(use_new_reader), )"
    ));
    for ocr_only in [
        "detect::match_start::detect_phase(",
        "detect::match_start::detect_phase_with_rgb(",
        "PhaseSource::Ocr",
    ] {
        assert!(!src.contains(ocr_only), "{ocr_only}");
    }
}

/// The vision pass reads the banner only through `choose_top_bar_map`,
/// with the daemon's reader switch, and hands the read to the board
/// read. Whitespace is ignored, so a reflow does not break this;
/// hard-wiring the switch or a second banner read does.
#[test]
fn the_vision_pass_chooses_the_top_bar_map_with_the_reader_switch() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let src = include_str!("../main.rs");
    let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
    let body = norm(src[at..].split("\n}\n").next().unwrap());
    assert!(
        body.contains("choose_top_bar_map( use_new_reader, || shadow::read_banner(&img),"),
        "the top-bar map goes through choose_top_bar_map with the switch"
    );
    assert_eq!(
        body.matches("shadow::read_banner").count(),
        1,
        "one banner read"
    );
    // ...one hero icon read (O8) and one board read (O10), which reuses
    // both instead of reading again: the only other shadow reads
    assert_eq!(
        body.matches("shadow::read_heroes").count(),
        1,
        "one hero read"
    );
    assert!(body.contains(
        "let board = read_tab_board(use_new_reader, || { shadow::read_board_with(&img, top_bar.banner, tab_heroes) });"
    ));
    assert_eq!(body.matches("shadow::read_").count(), 3);
    assert!(body.contains("map_from_banner: top_bar.from_banner,"));
    let hc = src
        .find("\nasync fn handle_capture(")
        .expect("handle_capture");
    let hc_body = norm(src[hc..].split("\n}\n").next().unwrap());
    assert!(hc_body.contains("use_new_reader: ctx.use_new_reader,"));
    assert!(hc_body.contains(
        "if ctx.use_new_reader { req.member_map_edit = session_has_member_map_edit(&ctx.store, &req).await; }"
    ));
    assert!(hc_body.contains("panel_from_banner: map_from_banner,"));
    assert!(
        hc_body
            .contains("apply_new_reader( &mut parsed, &req.result_evidence, board_row, own_row_suspect, board, pack_hero.as_deref(), );")
    );
    // the stored report carries the text-fallback replacement
    assert!(hc_body.contains("map_replaces_text_fallback: staged.replaced_text_fallback,"));
    // apply_new_reader reads nothing off the frame: it has no frame
    let an = src
        .find("\nfn apply_new_reader(")
        .expect("apply_new_reader");
    let an_body = norm(src[an..].split("\n}\n").next().unwrap());
    assert_eq!(an_body.matches("shadow::read_").count(), 0);
    assert_eq!(an_body.matches("ocr::").count(), 0);
}

/// T5 (O8): the vision pass reads the hero authority only through
/// `read_own_hero` with the daemon's reader switch, and `handle_capture`
/// resolves it with the switch. Hard-wiring either, or reading the career
/// panel outside `read_own_hero`, breaks this.
#[test]
fn the_hero_authority_goes_through_the_reader_switch() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let src = include_str!("../main.rs");
    let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
    let body = norm(src[at..].split("\n}\n").next().unwrap());
    assert!(body.contains("} = read_own_hero( use_new_reader, || {"));
    assert_eq!(
        body.matches("crop_career_hero").count(),
        1,
        "one career read"
    );
    // the icons are read once, behind the switch, before the own row
    // (O9b: the career hero's row is one of its voices), and that one
    // read is the hero authority. The image is the portrait crop: the
    // 5v5 bottom pad, or the unpadded crop for 6v6.
    assert!(body.contains(
        "let tab_heroes = read_tab_heroes(use_new_reader, || { shadow::read_heroes(&img, &portrait_board, team_size) });"
    ));
    assert!(body.contains("tab_heroes, row_idx, );"));
    assert!(
        body.find("read_tab_heroes(").unwrap() < body.find("} = find_own_row(").unwrap(),
        "the icons are read before the own row"
    );
    assert!(body.contains("career_hero, pack_hero, tab_heroes,"));
    let hc = src
        .find("\nasync fn handle_capture(")
        .expect("handle_capture");
    let hc_body = norm(src[hc..].split("\n}\n").next().unwrap());
    assert!(hc_body.contains(
        "if ctx.use_new_reader { hero_auth::resolve_hero_new_reader( pack_hero.as_deref(), portrait, &hero_auth, parse::canonical_hero, ) } else { hero_auth::resolve_hero( career_hero.as_deref(), portrait, &parsed.hero, &hero_auth, parse::canonical_hero, ) };"
    ));
}

/// F251-1: the rule for a sure banner read against the session map.
#[test]
fn a_banner_read_replaces_only_an_open_text_fallback_map() {
    let (g, now) = request_with_text_fallback();
    let req = build_capture_request(&g, false, now);
    let ilios = banner_facts("Ilios", true);
    assert_eq!(
        banner_replaces_text_fallback(&req, &ilios, false).as_deref(),
        Some("Ilios")
    );
    // the top-bar OCR, the same map, and a split never replace
    assert_eq!(
        banner_replaces_text_fallback(&req, &banner_facts("Ilios", false), false),
        None
    );
    assert_eq!(
        banner_replaces_text_fallback(&req, &banner_facts("busan", true), false),
        None
    );
    assert_eq!(banner_replaces_text_fallback(&req, &ilios, true), None);
    // a source-less map counts as text, like ActiveGame::map_is_trusted
    let mut no_source = request_with_text_fallback().0;
    no_source.map_source = None;
    let req_none = build_capture_request(&no_source, false, now);
    assert!(banner_replaces_text_fallback(&req_none, &ilios, false).is_some());

    type Edit = fn(&mut ActiveGame);
    let closed: [(&str, Edit); 9] = [
        ("top bar", |g| {
            g.map_source = Some(boundary::MapSource::TopBar)
        }),
        ("accolade", |g| {
            g.map_source = Some(boundary::MapSource::Accolade)
        }),
        ("outcome", |g| g.outcome = detect::MatchOutcome::Victory),
        ("pending boundary", |g| g.pending_boundary = true),
        ("lock", |g| g.text_fallback_locked = true),
        ("reset streak", |g| g.reset_streak = 1),
        ("carried board", |g| g.deferred = Some(Counters::default())),
        ("candidates", |g| {
            g.map_candidates = vec!["Oasis".into(), "Busan".into()]
        }),
        ("hint", |g| {
            g.result_mark = Some(boundary::ResultMark {
                outcome: detect::MatchOutcome::Defeat,
                confirmed: false,
                seen_at: test_now(),
            })
        }),
    ];
    for (what, edit) in closed {
        let mut h = request_with_text_fallback().0;
        edit(&mut h);
        let req = build_capture_request(&h, false, now);
        assert_eq!(
            banner_replaces_text_fallback(&req, &ilios, false),
            None,
            "{what}"
        );
    }
    let mut voted = request_with_text_fallback().0;
    voted.map_candidates = vec!["Ilios".into(), "Busan".into()];
    let req = build_capture_request(&voted, false, now);
    assert!(
        banner_replaces_text_fallback(&req, &ilios, false).is_some(),
        "a candidate passes"
    );

    // F252-3: a member's own map choice is never renamed
    let mut edited = build_capture_request(&request_with_text_fallback().0, false, now);
    edited.member_map_edit = true;
    assert_eq!(banner_replaces_text_fallback(&edited, &ilios, false), None);

    // F252-2: the last board must be at most STAT_SPLIT_MIN_GAP old
    // the game's own clock: test_now() moves between calls
    let (g, at) = request_with_text_fallback();
    let at_bound = build_capture_request(&g, false, at + STAT_SPLIT_MIN_GAP);
    assert!(banner_replaces_text_fallback(&at_bound, &ilios, false).is_some());
    let late = build_capture_request(
        &g,
        false,
        at + STAT_SPLIT_MIN_GAP + Duration::from_millis(1),
    );
    assert_eq!(
        banner_replaces_text_fallback(&late, &ilios, false),
        None,
        "gap"
    );
    let mut no_board = request_with_text_fallback().0;
    no_board.gate = None;
    no_board.last_stats_at = None;
    let req = build_capture_request(&no_board, false, now);
    assert_eq!(
        banner_replaces_text_fallback(&req, &ilios, false),
        None,
        "no board"
    );
}

#[test]
fn vote_candidates_constrain_but_never_become_the_map() {
    let candidates = vec!["Oasis".to_string(), "Busan".to_string()];

    // No trusted read yet: candidates alone must NOT pick a map, the
    // vote winner is unknowable, and detected_maps[0] used to be wrong
    // ~2/3 of the time.
    assert_eq!(resolve_map(None, None, "", &candidates), "");

    // Session's confirmed map always wins.
    assert_eq!(
        resolve_map(Some("Busan"), Some("Oasis"), "Ilios", &candidates),
        "Busan"
    );

    // Panel read accepted when it is a candidate...
    assert_eq!(resolve_map(None, Some("Busan"), "", &candidates), "Busan");
    // ...vetoed when it isn't (misread), falling back to a plausible
    // text read, or to nothing.
    assert_eq!(
        resolve_map(None, Some("Ilios"), "Oasis", &candidates),
        "Oasis"
    );
    assert_eq!(resolve_map(None, Some("Ilios"), "Nepal", &candidates), "");

    // Without candidates, panel > text (unchanged behavior).
    assert_eq!(resolve_map(None, Some("Ilios"), "Nepal", &[]), "Ilios");
    assert_eq!(resolve_map(None, None, "Nepal", &[]), "Nepal");

    let (name, source) = resolved_map(Some("Busan"), Some("Busan"), "Ilios", &[]);
    assert_eq!(name, "Busan");
    assert_eq!(
        source,
        Some(boundary::MapSource::TopBar),
        "a later top bar that agrees upgrades the stored source"
    );
    assert_eq!(
        resolved_map(Some("Busan"), None, "Busan", &[]).1,
        None,
        "full-board text that agrees does not upgrade the source"
    );
}
