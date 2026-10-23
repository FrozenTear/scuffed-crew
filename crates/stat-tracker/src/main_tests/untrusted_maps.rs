//! Daemon tests: untrusted_maps. See `mod.rs`.

use super::*;

#[tokio::test]
async fn a_stat_reset_on_untrusted_guillard_stays_unknown_and_does_not_upload_as_busan() {
    let mut night = Night::new().await;
    night.begin_on("Busan", "Zenyatta");
    night
        .tab_once(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    let busan = night.id();
    night.advance(Duration::from_secs(60));
    night
        .tab_once(
            night_counters(1, 3, 0, 220, 80, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(
        night.id(),
        busan,
        "the first fresh board only arms the reset"
    );
    assert!(night.game().reset_streak >= 1);
    night.advance(Duration::from_secs(20));
    let guillard = parse::match_map_in_text("guillard").expect("text guillard");
    assert_eq!(guillard, "Château Guillard");
    night
        .tab_text(
            night_counters(2, 1, 0, 300, 40, 80),
            "Zenyatta",
            Some(2),
            &guillard,
            NIGHT_CLEAN,
        )
        .await;
    let opened = night.id();
    assert_ne!(
        opened, busan,
        "the second fresh board is the stat reset split"
    );
    assert!(
        night.game().map.is_none(),
        "an untrusted guillard read must not name the new session Busan, got {:?}",
        night.game().map
    );
    let snaps = night.store.get_session_snapshots(&opened).await.unwrap();
    assert!(
        !snaps.is_empty() && snaps.iter().all(|row| row.map_name.is_empty()),
        "the new session is stored with an empty map: {snaps:?}"
    );
    assert!(snaps.iter().all(|row| row.game_mode.is_empty()));
    night
        .store
        .set_session_outcome(&opened, "victory")
        .await
        .unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_upload = std::sync::Arc::clone(&seen);
    try_sync_with(
        &night.store,
        night.dir.path(),
        night.st.settle_closed_unknown.clone(),
        move |matches, _| {
            let seen_upload = std::sync::Arc::clone(&seen_upload);
            async move {
                *seen_upload.lock().unwrap() = matches
                    .into_iter()
                    .map(|row| (row.session_id, row.map_name, row.game_mode))
                    .collect();
                Ok(upload_ok())
            }
        },
    )
    .await;
    let uploaded = seen.lock().unwrap().clone();
    assert!(
        uploaded.is_empty(),
        "the empty map stays local and is not sent as Busan: {uploaded:?}"
    );
    let kept = night.store.get_session_snapshots(&opened).await.unwrap();
    assert!(
        !kept.is_empty()
            && kept
                .iter()
                .all(|row| !row.synced && row.map_name.is_empty() && row.game_mode.is_empty()),
        "the split stays unsynced with no map until a pick: {kept:?}"
    );
}

#[test]
fn an_untrusted_guillard_read_does_not_replace_the_session_map() {
    let (mut staged, mut parsed) = staged_row(
        "busan-keep",
        "Château Guillard",
        Some(boundary::MapSource::TextFallback),
    );
    let reject = prepare_capture_row(
        &mut parsed,
        &mut staged,
        Some("Busan"),
        Some("GAILLARD"),
        "player GAILLARD",
        true,
    );
    assert_eq!(reject, PreparedRow::Store);
    assert_eq!(staged.map_name, "Busan");
    assert_eq!(parsed.map_name, "Busan");
    assert_eq!(parsed.game_mode, "Control");
    assert!(staged.map_source.is_none());
}

#[test]
fn the_adlersbrunn_gate_rejects_a_one_letter_misread_without_both_teams() {
    let (mut staged, mut parsed) =
        staged_row("ew", "Eichenwalde", Some(boundary::MapSource::TopBar));
    assert_eq!(
        prepare_capture_row(
            &mut parsed,
            &mut staged,
            None,
            Some("ADLERSBRUNM"),
            "",
            false,
        ),
        PreparedRow::Drop
    );
    let (mut staged, mut parsed) =
        staged_row("ew", "Eichenwalde", Some(boundary::MapSource::TopBar));
    assert_eq!(
        prepare_capture_row(
            &mut parsed,
            &mut staged,
            None,
            Some("ADLERSBRUNM"),
            "",
            true,
        ),
        PreparedRow::Store
    );
    let (mut staged, mut parsed) =
        staged_row("ew", "Eichenwalde", Some(boundary::MapSource::TopBar));
    assert_eq!(
        prepare_capture_row(
            &mut parsed,
            &mut staged,
            None,
            Some("EICHENWALDE"),
            "",
            false,
        ),
        PreparedRow::Store
    );
    let (mut staged, mut parsed) =
        staged_row("ew", "Eichenwalde", Some(boundary::MapSource::TopBar));
    assert_eq!(
        prepare_capture_row(
            &mut parsed,
            &mut staged,
            None,
            Some("EICHENWALD"),
            "",
            false,
        ),
        PreparedRow::Store,
        "a one-letter Eichenwalde misread is not an Adlersbrunn alias"
    );
}

/// The banner spelling is text ocr-v1's alias check accepts, and it is
/// the label the tracker's map table stores as Eichenwalde.
#[test]
fn the_banner_spelling_of_adlersbrunn_is_an_ocr_v1_alias() {
    let label = shadow::banner::ADLERSBRUNN;
    assert!(parse::is_adlersbrunn_alias(label));
    assert_eq!(
        parse::match_map_in_text(label).as_deref(),
        Some("Eichenwalde")
    );
}

/// With no OCR text, a sure `ADLERSBRUNN` banner read gives what the
/// OCR label gives under ocr-v1: the map is Eichenwalde (Hybrid), a
/// board with stats on both teams is stored, and a board with stats on
/// one team only (Junkenstein) is dropped.
#[test]
fn a_sure_adlersbrunn_banner_read_feeds_the_pve_gate_without_ocr() {
    let key = shadow::banner::ADLERSBRUNN_KEY;
    let (map, row, stored, mode) = adlersbrunn_tab(Some(banner_read(key, false)), true);
    assert_eq!(map.as_deref(), Some("Eichenwalde"));
    assert_eq!(row, PreparedRow::Store);
    assert_eq!((stored.as_str(), mode.as_str()), ("Eichenwalde", "Hybrid"));
    let (map, row, ..) = adlersbrunn_tab(Some(banner_read(key, false)), false);
    assert_eq!(map.as_deref(), Some("Eichenwalde"));
    assert_eq!(row, PreparedRow::Drop, "a Junkenstein board is dropped");

    // the same two boards through ocr-v1's label
    for (both_teams, want) in [(true, PreparedRow::Store), (false, PreparedRow::Drop)] {
        let v1 = choose_top_bar_map(false, || None, parse::match_map_in_text("ADLERSBRUNN"));
        assert_eq!(v1.map.as_deref(), Some("Eichenwalde"));
        let label = top_bar_raw_label(v1.banner.as_ref(), Some("ADLERSBRUNN".into()));
        let (mut staged, mut parsed) =
            staged_row("m1", "Eichenwalde", Some(boundary::MapSource::TopBar));
        let row = prepare_capture_row(
            &mut parsed,
            &mut staged,
            None,
            label.as_deref(),
            "",
            both_teams,
        );
        assert_eq!(row, want);
        assert_eq!(
            (parsed.map_name.as_str(), parsed.game_mode.as_str()),
            ("Eichenwalde", "Hybrid")
        );
    }
}

/// A real Eichenwalde banner is not the label: a board with stats on
/// one team is stored, as ocr-v1 stores it. A flagged `ADLERSBRUNN`
/// read names nothing and drops nothing.
#[test]
fn an_eichenwalde_or_flagged_banner_read_does_not_feed_the_pve_gate() {
    let (map, row, stored, _) = adlersbrunn_tab(Some(banner_read("eichenwalde", false)), false);
    assert_eq!(map.as_deref(), Some("Eichenwalde"));
    assert_eq!(row, PreparedRow::Store);
    assert_eq!(stored, "Eichenwalde");
    let key = shadow::banner::ADLERSBRUNN_KEY;
    let (map, row, stored, _) = adlersbrunn_tab(Some(banner_read(key, true)), false);
    assert_eq!(map, None, "a flagged read is no top-bar map");
    assert_eq!(row, PreparedRow::Store);
    assert_eq!(stored, "");
    let (map, row, ..) = adlersbrunn_tab(None, false);
    assert_eq!((map, row), (None, PreparedRow::Store), "no map pack");
}

/// Only a sure `ADLERSBRUNN` banner read replaces the OCR label. Every
/// other read, and ocr-v1, hand the OCR label on untouched.
#[test]
fn only_a_sure_adlersbrunn_banner_read_becomes_the_raw_label() {
    let key = shadow::banner::ADLERSBRUNN_KEY;
    let sure = Some(banner_read(key, false));
    assert_eq!(
        top_bar_raw_label(Some(&sure), None).as_deref(),
        Some("ADLERSBRUNN")
    );
    assert_eq!(
        top_bar_raw_label(Some(&sure), Some("EICHENWALDE".into())).as_deref(),
        Some("ADLERSBRUNN"),
        "the sure banner read wins over the OCR label, as it does for the map"
    );
    let untouched = [
        Some(Some(banner_read(key, true))),
        Some(Some(banner_read("eichenwalde", false))),
        Some(Some(banner_read("ilios", false))),
        Some(None),
        None,
    ];
    for banner in &untouched {
        for ocr in [
            None,
            Some("ADLERSBRUNN"),
            Some("EICHENWALDE"),
            Some("BUSAN"),
        ] {
            assert_eq!(
                top_bar_raw_label(banner.as_ref(), ocr.map(String::from)).as_deref(),
                ocr,
                "{banner:?}"
            );
        }
    }
    let mut no_banner = banner_read(key, false);
    no_banner.found_banner = false;
    assert_eq!(top_bar_raw_label(Some(&Some(no_banner)), None), None);
}

/// The vision pass hands the label on from the banner it already read,
/// before the board takes that read. Whitespace is ignored.
#[test]
fn the_vision_pass_takes_the_raw_label_from_the_banner_read() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let full = include_str!("../main.rs");
    let src = full.split("\nmod tests {").next().unwrap();
    let at = src.find("\nfn analyze_frame(").expect("analyze_frame");
    let body = norm(src[at..].split("\n}\n").next().unwrap());
    let call = "let map_panel_raw = top_bar_raw_label(top_bar.banner.as_ref(), map_panel_raw);";
    assert_eq!(body.matches(call).count(), 1);
    let chosen = body.find("let top_bar = choose_top_bar_map(").unwrap();
    let built = body.find("FrameAnalysis {").unwrap();
    let at = body.find(call).unwrap();
    assert!(chosen < at && at < built);
    assert!(body[at..].contains(" map_panel_raw, "));
}

#[tokio::test]
async fn a_trusted_guillard_row_is_kept_locally_and_not_uploaded() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let (mut staged, mut parsed) = staged_row(
        "dm-keep",
        "Château Guillard",
        Some(boundary::MapSource::TopBar),
    );
    assert_eq!(
        prepare_capture_row(
            &mut parsed,
            &mut staged,
            None,
            Some("CHATEAU GUILLARD"),
            "",
            true,
        ),
        PreparedRow::Store
    );
    assert_eq!(parsed.map_name, "Château Guillard");
    assert_eq!(parsed.game_mode, "Deathmatch");
    commit_capture_rows(
        &store,
        dir.path(),
        &staged,
        &parsed,
        SurrealDatetime::from(Utc::now()),
    )
    .await
    .unwrap();
    let rows = store.get_session_snapshots("dm-keep").await.unwrap();
    assert_eq!(rows.len(), 1);
    let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let called_upload = std::sync::Arc::clone(&called);
    try_sync_with(&store, dir.path(), None, move |_matches, _| {
        let called_upload = std::sync::Arc::clone(&called_upload);
        async move {
            called_upload.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(upload_ok())
        }
    })
    .await;
    assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
    let rows = store.get_session_snapshots("dm-keep").await.unwrap();
    assert!(rows.iter().all(|row| row.synced));
}

#[tokio::test]
async fn a_deathmatch_row_stays_local_when_the_new_reader_names_another_map() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let (mut staged, mut parsed) = staged_row(
        "dm-new",
        "Château Guillard",
        Some(boundary::MapSource::TopBar),
    );
    assert_eq!(
        prepare_capture_row(
            &mut parsed,
            &mut staged,
            None,
            Some("CHATEAU GUILLARD"),
            "",
            true,
        ),
        PreparedRow::Store
    );
    assert_eq!(parsed.game_mode, "Deathmatch");
    let board = {
        let mut fields = vec![
            shadow::FieldRead {
                name: "map".into(),
                value: Some(shadow::Value::Text("Ilios".into())),
                confidence: 0.95,
                suspect: false,
            },
            shadow::FieldRead {
                name: "mode".into(),
                value: Some(shadow::Value::Text("Control".into())),
                confidence: 0.95,
                suspect: false,
            },
        ];
        fields.push(shadow::FieldRead {
            name: "r0.hero".into(),
            value: Some(shadow::Value::Text("Kiriko".into())),
            confidence: 0.95,
            suspect: false,
        });
        for (suffix, value) in [
            ("e", 21u32),
            ("a", 9),
            ("d", 3),
            ("dmg", 9100),
            ("h", 1200),
            ("mit", 50),
        ] {
            fields.push(shadow::FieldRead {
                name: format!("r0.{suffix}"),
                value: Some(shadow::Value::Int(value)),
                confidence: 0.95,
                suspect: false,
            });
        }
        shadow::BoardRead {
            status: shadow::BoardStatus::Read,
            team_size: Some(5),
            fields,
            elapsed_ms: 1,
            header_dimmed: false,
        }
    };
    let saved = reader_apply::merge_saved(
        &reader_apply::PolicyRow {
            map: parsed.map_name.clone(),
            mode: parsed.game_mode.clone(),
            result: parsed.outcome.clone(),
            hero: parsed.hero.clone(),
            elims: parsed.elims,
            assists: parsed.assists,
            deaths: parsed.deaths,
            damage: parsed.damage,
            healing: parsed.healing,
            mitigation: parsed.mitigation,
        },
        reader_apply::OwnRow::Identified {
            index: 0,
            team_size: 5,
        },
        Some(&board),
    );
    parsed.map_name = saved.map;
    parsed.game_mode = saved.mode;
    parsed.hero = saved.hero;
    parsed.elims = saved.elims;
    parsed.recognizer = saved.recognizer.to_string();
    parsed.suspect_fields = saved.suspect_fields;
    assert_eq!(parsed.map_name, "Château Guillard");
    assert_eq!(parsed.game_mode, "Deathmatch");
    assert!(parsed.suspect_fields.iter().any(|name| name == "map"));
    commit_capture_rows(
        &store,
        dir.path(),
        &staged,
        &parsed,
        SurrealDatetime::from(Utc::now()),
    )
    .await
    .unwrap();
    let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let called_upload = std::sync::Arc::clone(&called);
    try_sync_with(&store, dir.path(), None, move |_matches, _| {
        let called_upload = std::sync::Arc::clone(&called_upload);
        async move {
            called_upload.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(upload_ok())
        }
    })
    .await;
    assert!(
        !called.load(std::sync::atomic::Ordering::SeqCst),
        "reader = new must not upload a Deathmatch row"
    );
}

#[tokio::test]
async fn practice_range_never_uploads_and_is_not_held_for_review() {
    let name = parse::canonical_map("PRACTICE RANGE").expect("practice range");
    assert_eq!(name, "Practice Range");
    assert_eq!(parse::stored_game_mode(&name), "Practice");

    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let (mut staged, mut parsed) = staged_row("practice", &name, Some(boundary::MapSource::TopBar));
    assert_eq!(
        prepare_capture_row(
            &mut parsed,
            &mut staged,
            None,
            Some("PRACTICE RANGE"),
            "",
            true,
        ),
        PreparedRow::Store
    );
    assert_eq!(parsed.map_name, "Practice Range");
    assert_eq!(parsed.game_mode, "Practice");
    assert!(
        parse::review_suspect_fields(&parsed.map_name, &parsed.game_mode, &parsed.hero).is_empty(),
        "a known practice map with a mode is not a blank-map review hold"
    );
    commit_capture_rows(
        &store,
        dir.path(),
        &staged,
        &parsed,
        SurrealDatetime::from(Utc::now()),
    )
    .await
    .unwrap();

    // No open map: do not store a blank row. A blank map uploads, and
    // the review hold would keep it until someone picked a map.
    let (mut fuzzy, mut fuzzy_parsed) = staged_row(
        "practice-fuzzy",
        &name,
        Some(boundary::MapSource::TextFallback),
    );
    assert_eq!(
        prepare_capture_row(
            &mut fuzzy_parsed,
            &mut fuzzy,
            None,
            Some("PRACTICE"),
            "",
            true
        ),
        PreparedRow::Drop
    );
    assert!(fuzzy.map_name.is_empty());

    // An open game keeps its map. The practice misread must not replace it.
    let (mut open, mut open_parsed) =
        staged_row("busan-open", &name, Some(boundary::MapSource::TextFallback));
    assert_eq!(
        prepare_capture_row(
            &mut open_parsed,
            &mut open,
            Some("Busan"),
            Some("PRACTICE RANGE"),
            "",
            true,
        ),
        PreparedRow::Store
    );
    assert_eq!(open_parsed.map_name, "Busan");
    assert_eq!(open_parsed.game_mode, "Control");
    commit_capture_rows(
        &store,
        dir.path(),
        &open,
        &open_parsed,
        SurrealDatetime::from(Utc::now()),
    )
    .await
    .unwrap();

    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_upload = std::sync::Arc::clone(&seen);
    try_sync_with(&store, dir.path(), None, move |matches, _| {
        let seen_upload = std::sync::Arc::clone(&seen_upload);
        async move {
            *seen_upload.lock().unwrap() = matches
                .into_iter()
                .map(|row| (row.session_id, row.map_name, row.game_mode))
                .collect();
            Ok(upload_ok())
        }
    })
    .await;
    let uploaded = seen.lock().unwrap().clone();
    assert_eq!(
        uploaded,
        vec![("busan-open".into(), "Busan".into(), "Control".into())]
    );
    assert!(
        uploaded
            .iter()
            .all(|(_, map, mode)| map != "Practice Range" && mode != "Practice")
    );

    let practice = store.get_session_snapshots("practice").await.unwrap();
    assert_eq!(practice.len(), 1);
    assert!(
        practice[0].synced,
        "practice is kept locally, not left unsynced"
    );
    assert!(practice[0].upload_rejection().is_none());
    assert!(
        parse::review_suspect_fields(
            practice[0].display_map_name(),
            &practice[0].game_mode,
            practice[0].display_hero(),
        )
        .is_empty()
    );
    assert!(
        store
            .get_session_snapshots("practice-fuzzy")
            .await
            .unwrap()
            .is_empty(),
        "an untrusted practice read is not stored as a blank map"
    );
    assert!(store.get_unsynced().await.unwrap().is_empty());

    let again = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let again_upload = std::sync::Arc::clone(&again);
    let second = try_sync_with(&store, dir.path(), None, move |_, _| {
        let again_upload = std::sync::Arc::clone(&again_upload);
        async move {
            again_upload.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(upload_ok())
        }
    })
    .await;
    assert!(matches!(second, sync::SyncAttempt::NoServerCall));
    assert!(!again.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn an_untrusted_guillard_report_does_not_rename_an_open_game() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let now = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_id = "busan-open".into();
    g.map = Some("Busan".into());
    g.map_source = Some(boundary::MapSource::TopBar);
    let mut st = session(Some(g), now);
    let mut report = rejected_report("busan-open");
    report.map = Some("Château Guillard".into());
    report.map_source = Some(boundary::MapSource::TextFallback);
    apply_capture_report(&mut st, &store, dir.path(), "busan-open", Ok(report), now).await;
    assert_eq!(
        st.active_game.as_ref().and_then(|g| g.map.as_deref()),
        Some("Busan")
    );

    let mut empty = game(detect::MatchOutcome::Unknown, None, now);
    empty.session_id = "no-map".into();
    empty.map = None;
    let mut st = session(Some(empty), now);
    let mut fuzzy = rejected_report("no-map");
    fuzzy.map = Some("Château Guillard".into());
    fuzzy.map_source = Some(boundary::MapSource::TextFallback);
    apply_capture_report(&mut st, &store, dir.path(), "no-map", Ok(fuzzy), now).await;
    assert!(st.active_game.as_ref().unwrap().map.is_none());

    let mut fresh = game(detect::MatchOutcome::Unknown, None, now);
    fresh.session_id = "dm-note".into();
    fresh.map = None;
    let mut st = session(Some(fresh), now);
    for _ in 0..2 {
        let mut noted = rejected_report("dm-note");
        noted.map = Some("Château Guillard".into());
        noted.map_source = Some(boundary::MapSource::Accolade);
        apply_capture_report(&mut st, &store, dir.path(), "dm-note", Ok(noted), now).await;
    }
    assert_eq!(
        st.active_game.as_ref().and_then(|g| g.map.as_deref()),
        Some("Château Guillard")
    );
    let text =
        std::fs::read_to_string(dir.path().join("debug").join("unrecorded_games.jsonl")).unwrap();
    assert_eq!(
        text.lines().filter(|line| line.contains("dm-note")).count(),
        1
    );
}
