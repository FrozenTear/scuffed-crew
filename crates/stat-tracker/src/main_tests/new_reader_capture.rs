//! Daemon tests: new_reader_capture. See `mod.rs`.

use super::*;

#[test]
fn new_reader_drop_from_11_to_1_stays_11_and_flags_elims() {
    let mut parsed = test_match("s", "unknown");
    parsed.recognizer = "cv-v5".into();
    parsed.elims = 1;
    parsed.assists = 4;
    parsed.deaths = 2;
    parsed.damage = 3100;
    parsed.healing = 800;
    parsed.mitigation = 400;
    let prior = capture_gate::GateState {
        accepted: capture_gate::Counters {
            elims: 11,
            assists: 4,
            deaths: 2,
            damage: 3000,
            healing: 800,
            mitigation: 400,
        },
        last_raw: capture_gate::Counters {
            elims: 11,
            assists: 4,
            deaths: 2,
            damage: 3000,
            healing: 800,
            mitigation: 400,
        },
        ..capture_gate::GateState::default()
    };
    let clean = [false; capture_gate::GATE_COLS];
    let held = hold_new_reader_stats(
        &mut parsed,
        Some((prior, Duration::from_secs(20))),
        clean,
        clean,
        false,
        true,
        None,
    );
    assert_eq!(
        parsed.elims, 11,
        "a new-reader 1 does not replace a held 11"
    );
    assert!(
        parsed.suspect_fields.iter().any(|field| field == "e"),
        "elims stays flagged: {:?}",
        parsed.suspect_fields
    );
    assert_eq!(held.accepted.elims, 11);
    // The same hold waits on a lone jump, then stores the matching second read.
    parsed.elims = 18;
    parsed.suspect_fields.clear();
    let jumped = hold_new_reader_stats(
        &mut parsed,
        Some((prior, Duration::from_secs(20))),
        clean,
        clean,
        false,
        true,
        None,
    );
    assert_eq!(parsed.elims, 11, "one 18 does not count");
    assert!(parsed.suspect_fields.iter().any(|field| field == "e"));
    parsed.elims = 18;
    parsed.suspect_fields.clear();
    let matched = hold_new_reader_stats(
        &mut parsed,
        Some((jumped.state, Duration::from_secs(20))),
        clean,
        clean,
        false,
        true,
        None,
    );
    assert_eq!(matched.accepted.elims, 18, "the second 18 counts");
    assert_eq!(parsed.elims, 18);
}

#[test]
fn new_reader_replaces_a_fallback_of_40_18_on_one_clean_read() {
    let clean = [false; capture_gate::GATE_COLS];
    let mut parsed = test_match("s", "unknown");
    parsed.elims = 2;
    parsed.assists = 40;
    parsed.deaths = 18;
    parsed.damage = 450;
    parsed.healing = 0;
    parsed.mitigation = 2;
    let first = hold_new_reader_stats(&mut parsed, None, clean, clean, false, false, None);
    assert_eq!((parsed.assists, parsed.deaths), (40, 18));
    assert!(first.state.low_trust);
    assert!(first.state.unconfirmed[1] && first.state.unconfirmed[2]);

    parsed.assists = 0;
    parsed.deaths = 0;
    parsed.damage = 1105;
    parsed.healing = 259;
    parsed.mitigation = 450;
    parsed.suspect_fields.clear();
    let second = hold_new_reader_stats(
        &mut parsed,
        Some((first.state, Duration::from_secs(20))),
        clean,
        clean,
        false,
        true,
        None,
    );
    assert_eq!(
        (parsed.assists, parsed.deaths),
        (0, 0),
        "one clean read replaces the fallback"
    );
    assert!(!second.state.low_trust);

    parsed.assists = 0;
    parsed.deaths = 0;
    parsed.suspect_fields.clear();
    let third = hold_new_reader_stats(
        &mut parsed,
        Some((second.state, Duration::from_secs(20))),
        clean,
        clean,
        false,
        true,
        None,
    );
    assert_eq!((third.accepted.assists, third.accepted.deaths), (0, 0));
    assert_eq!((parsed.assists, parsed.deaths), (0, 0));
    assert!(!third.state.low_trust);
}

#[tokio::test]
async fn handle_capture_new_reader_keeps_11_when_the_next_read_is_1() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    fn stat_rows(elims: u32) -> Vec<ocr::RowOcrResult> {
        let values = [
            elims.to_string(),
            "4".into(),
            "2".into(),
            "3000".into(),
            "800".into(),
            "400".into(),
        ];
        (0..10)
            .map(|_| ocr::RowOcrResult {
                name: Some(ocr::CellOcrResult {
                    value: "PLAYER".into(),
                    confidence: 80,
                    suspect: false,
                }),
                name_alternates: Vec::new(),
                stats: values
                    .iter()
                    .map(|value| ocr::CellOcrResult {
                        value: value.clone(),
                        confidence: 90,
                        suspect: false,
                    })
                    .collect(),
                mean_confidence: 90,
            })
            .collect()
    }
    fn analysis_for(elims: u32) -> FrameAnalysis {
        let frame = image::DynamicImage::new_rgb8(64, 64);
        FrameAnalysis {
            frame_outcome: detect::MatchOutcome::Unknown,
            ocr: Ok(ocr::OcrResult {
                raw_text: "Ana\nBusan".into(),
                confidence: 80,
            }),
            rows: stat_rows(elims),
            ocr_fallback: false,
            portrait_hero: None,
            career_hero: None,
            pack_hero: None,
            board: None,
            map_from_panel: Some("Busan".into()),
            map_panel_raw: Some("BUSAN".into()),
            map_from_banner: false,
            scoreboard: std::sync::Arc::new(frame.clone()),
            player_row_idx: Some(0),
            name_match: None,
            name_recognizer: None,
            own_row: None,
            own_name_candidate: None,
            own_seed_verdict: detect::own_row::Verdict::Silent,
            team_size: 5,
            frame,
            shadow_input: None,
        }
    }
    fn board_for(elims: u32) -> shadow::BoardRead {
        let mut fields = vec![
            shadow::FieldRead {
                name: "map".into(),
                value: Some(shadow::Value::Text("Busan".into())),
                confidence: 0.9,
                suspect: false,
            },
            shadow::FieldRead {
                name: "r0.hero".into(),
                value: Some(shadow::Value::Text("Ana".into())),
                confidence: 0.9,
                suspect: false,
            },
        ];
        for (suffix, value) in [
            ("e", elims),
            ("a", 4),
            ("d", 2),
            ("dmg", 3000),
            ("h", 800),
            ("mit", 400),
        ] {
            fields.push(shadow::FieldRead {
                name: format!("r0.{suffix}"),
                value: Some(shadow::Value::Int(value)),
                confidence: 0.9,
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
    }

    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let ctx = DaemonCtx {
        backend: capture::CaptureBackend::None,
        store: store.clone(),
        sync_client: None,
        player_name: None,
        capture_output: None,
        auto_detect: config::AutoDetectConfig::default(),
        finished_game_close: Duration::from_secs(90),
        game_process_names: Vec::new(),
        portrait_matcher: std::sync::Arc::new(detect::hero_portrait::PortraitMatcher::load(
            dir.path(),
        )),
        collect_portraits: false,
        dump_poll_frames: false,
        debug_ocr: false,
        save_pregame_screens: false,
        data_dir: dir.path().to_path_buf(),
        empty_map_reads: std::sync::atomic::AtomicUsize::new(0),
        shadow: None,
        own_name: None,
        names: names::NameSwitch::off(),
        use_new_reader: true,
    };
    let mut now = test_now();
    let mut active = game(detect::MatchOutcome::Unknown, None, now);
    active.session_id = "held-11".into();
    active.map = Some("Busan".into());
    active.map_source = Some(boundary::MapSource::TopBar);
    active.session_created = false;
    let mut st = session(Some(active), now);

    for elims in [11u32, 1] {
        *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture {
            analysis: analysis_for(elims),
            board: Some(board_for(elims)),
        });
        let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
        let report = handle_capture(&ctx, req).await.unwrap();
        assert!(report.recorded, "elims read {elims} was not stored");
        assert!(!report.split, "elims read {elims} must stay on this game");
        apply_capture_report(
            &mut st,
            &ctx.store,
            &ctx.data_dir,
            "held-11",
            Ok(report),
            now,
        )
        .await;
        now += Duration::from_secs(20);
    }

    let saved = ctx.store.get_session_snapshots("held-11").await.unwrap();
    let elims: Vec<u32> = saved.iter().map(|row| row.elims).collect();
    assert_eq!(elims, vec![11, 11], "a read of 1 must not replace 11");
}

/// O1: the same capture, with a sure result in the match's word
/// evidence, stores that result only with `reader = "new"`. With the
/// default `ocr-v1` the stored row is ocr-v1's, whatever the evidence.
#[tokio::test]
async fn result_evidence_is_saved_only_with_the_new_reader() {
    use shadow::result::{ACCOLADE, Outcome, RANK_SCREEN, ResultRead};
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let sure = |src: &str| ResultRead {
        outcome: Some(Outcome::Defeat),
        source: Some(src.into()),
        score: 0.97,
        margin: 0.6,
        suspect: false,
    };
    let rows = || -> Vec<ocr::RowOcrResult> {
        (0..10)
            .map(|_| ocr::RowOcrResult {
                name: Some(ocr::CellOcrResult {
                    value: "PLAYER".into(),
                    confidence: 80,
                    suspect: false,
                }),
                name_alternates: Vec::new(),
                stats: ["7", "4", "2", "3000", "800", "400"]
                    .iter()
                    .map(|value| ocr::CellOcrResult {
                        value: (*value).into(),
                        confidence: 90,
                        suspect: false,
                    })
                    .collect(),
                mean_confidence: 90,
            })
            .collect()
    };
    let analysis = || {
        let frame = image::DynamicImage::new_rgb8(64, 64);
        FrameAnalysis {
            frame_outcome: detect::MatchOutcome::Unknown,
            ocr: Ok(ocr::OcrResult {
                raw_text: "Ana\nBusan".into(),
                confidence: 80,
            }),
            rows: rows(),
            ocr_fallback: false,
            portrait_hero: None,
            career_hero: None,
            pack_hero: None,
            board: None,
            map_from_panel: Some("Busan".into()),
            map_panel_raw: Some("BUSAN".into()),
            scoreboard: std::sync::Arc::new(frame.clone()),
            player_row_idx: Some(0),
            name_match: None,
            name_recognizer: None,
            own_row: None,
            own_name_candidate: None,
            own_seed_verdict: detect::own_row::Verdict::Silent,
            team_size: 5,
            frame,
            shadow_input: None,
            map_from_banner: false,
        }
    };
    let mut fields = vec![shadow::FieldRead {
        name: "result".into(),
        value: None,
        confidence: 0.0,
        suspect: true,
    }];
    for (suffix, value) in [
        ("e", 7),
        ("a", 4),
        ("d", 2),
        ("dmg", 3000),
        ("h", 800),
        ("mit", 400),
    ] {
        fields.push(shadow::FieldRead {
            name: format!("r0.{suffix}"),
            value: Some(shadow::Value::Int(value)),
            confidence: 0.9,
            suspect: false,
        });
    }
    let board = shadow::BoardRead {
        status: shadow::BoardStatus::Read,
        team_size: Some(5),
        fields,
        elapsed_ms: 1,
        header_dimmed: false,
    };
    let mut stored = Vec::new();
    for use_new_reader in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::LocalStore::open(dir.path()).await.unwrap();
        let ctx = DaemonCtx {
            backend: capture::CaptureBackend::None,
            store: store.clone(),
            sync_client: None,
            player_name: None,
            capture_output: None,
            auto_detect: config::AutoDetectConfig::default(),
            finished_game_close: Duration::from_secs(90),
            game_process_names: Vec::new(),
            portrait_matcher: std::sync::Arc::new(detect::hero_portrait::PortraitMatcher::load(
                dir.path(),
            )),
            collect_portraits: false,
            dump_poll_frames: false,
            debug_ocr: false,
            save_pregame_screens: false,
            data_dir: dir.path().to_path_buf(),
            empty_map_reads: std::sync::atomic::AtomicUsize::new(0),
            shadow: None,
            own_name: None,
            names: names::NameSwitch::off(),
            use_new_reader,
        };
        let now = test_now();
        let mut active = game(detect::MatchOutcome::Unknown, None, now);
        active.session_id = "o1-result".into();
        active.map = Some("Busan".into());
        active.map_source = Some(boundary::MapSource::TopBar);
        active.session_created = false;
        active.result_evidence.add(sure(RANK_SCREEN));
        active.result_evidence.add(sure(ACCOLADE));
        let mut st = session(Some(active), now);
        *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture {
            analysis: analysis(),
            board: Some(board.clone()),
        });
        let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
        let report = handle_capture(&ctx, req).await.unwrap();
        assert!(report.recorded);
        apply_capture_report(
            &mut st,
            &ctx.store,
            &ctx.data_dir,
            "o1-result",
            Ok(report),
            now,
        )
        .await;
        let saved = ctx.store.get_session_snapshots("o1-result").await.unwrap();
        assert_eq!(saved.len(), 1);
        stored.push((saved[0].outcome.clone(), saved[0].recognizer.clone()));
    }
    assert_ne!(
        stored[0].0, "defeat",
        "ocr-v1 ignores the evidence: {stored:?}"
    );
    assert_eq!(stored[0].1, scuffed_types::RECOGNIZER_OCR_V1);
    assert_eq!(stored[1].0, "defeat", "{stored:?}");
    assert_eq!(stored[1].1, shadow::digits::RECOGNIZER_ID);
}
