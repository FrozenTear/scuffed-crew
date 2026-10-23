//! Daemon tests: ocr_off_switch. See `mod.rs`.

use super::*;

/// With OCR off the vision pass still runs to its end on a Tab frame.
/// Every OCR read is stopped at the gate and comes back empty, and the
/// missing board text is not an error (an error would fail the capture).
#[test]
fn with_ocr_off_a_tab_frame_is_analysed_without_ocr_and_is_not_a_failed_capture() {
    let frame = s1_tab_frame();
    assert!(
        matches!(
            detect::hero_portrait::preflight_scoreboard(&ocr::preprocess::crop_scoreboard(&frame))
                .0,
            detect::hero_portrait::ScoreboardPreflight::Ready(_)
        ),
        "the synthetic frame has to pass the preflight"
    );
    let dir = tempfile::tempdir().unwrap();
    let matcher = Arc::new(detect::hero_portrait::PortraitMatcher::load(dir.path()));
    let off = ocr::gate::scoped(ocr::gate::OcrMode::Off);
    let analysed = analyze_frame(
        frame,
        matcher,
        Some("PLAYER".into()),
        detect::MatchOutcome::Unknown,
        FrameFlags {
            allow_banner_recovery: true,
            session_map_known: false,
            shadow_on: false,
            // `ocr = "off"` implies the new reader
            use_new_reader: true,
        },
        // no own-name seed yet: a first run
        Vec::new(),
        &names::NameSwitch::off(),
    );
    let FrameAnalysisOutcome::Analyzed(a) = analysed else {
        panic!("the frame was rejected before the reads");
    };
    assert!(
        a.rows
            .iter()
            .all(|row| row.name.is_none() && row.stats.is_empty()),
        "no name and no stat cell was read"
    );
    assert_eq!(a.map_panel_raw, None, "no top-bar label was read");
    assert_eq!(a.career_hero, None);
    assert!(a.name_match.is_none());
    assert_eq!(a.frame_outcome, detect::MatchOutcome::Unknown);
    let text = a
        .ocr
        .as_ref()
        .expect("with OCR off, no board text is not a failed capture");
    assert_eq!((text.raw_text.as_str(), text.confidence), ("", 0));
    // The trust gate downstream is the new reader's board. Nothing on
    // this frame can be read, so the board is refused, not handed to OCR.
    let board = a.board.as_ref().expect("the new reader's board");
    assert!(
        reader_apply::board_refusal(board, a.team_size).is_some(),
        "{:?}",
        board.status
    );
    let blocked = off.blocked();
    assert_eq!(blocked, [], "nothing on the Tab pass went to the gate");
    assert!(
        blocked
            .iter()
            .all(|b| !matches!(b.site.entry, "ocr_with" | "ocr_pool" | "tessdata_lang")),
        "every OCR call stops at its entry point: {blocked:?}"
    );
    // The reads that have a replacement, and the two fallbacks, are not
    // attempted at all: the top-bar label (the banner read, O6), the
    // career-panel title (the hero icons, O8) and the full-board text.
    assert!(
        blocked.iter().all(|b| !matches!(
            b.site.entry,
            "recognize_map_label" | "recognize_region" | "recognize"
        )),
        "a replaced read still went to the gate: {blocked:?}"
    );
}

/// A Tab nothing was read from is held. It stores no row, and it does
/// not touch the game that is open: the row already stored, the result
/// and the map all stay as they were. Nothing is written as Unknown.
#[tokio::test]
async fn an_unread_board_is_held_and_leaves_the_open_game_as_it_was() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let mut now = test_now();
    let mut active = game(detect::MatchOutcome::Victory, Some(5), now);
    active.session_id = "s1".into();
    active.map = Some("Busan".into());
    active.map_source = Some(boundary::MapSource::TopBar);
    active.session_created = false;
    let mut st = session(Some(active), now);

    let tabs = [
        (
            o8_analysis(None, Some("zenyatta")),
            Some(o8_board(Some("zenyatta"))),
            true,
        ),
        (s1_unread_analysis(), None, false),
        (s1_unread_analysis(), None, false),
    ];
    let mut stored_after_first = None;
    for (i, (analysis, board, stored)) in tabs.into_iter().enumerate() {
        *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture { analysis, board });
        let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
        let report = handle_capture(&ctx, req)
            .await
            .expect("an unread board is not a failed capture");
        assert_eq!(report.recorded, stored, "tab {i}");
        assert!(!report.split, "tab {i} stays on this game");
        assert_eq!(report.outcome, detect::MatchOutcome::Victory, "tab {i}");
        apply_capture_report(&mut st, &ctx.store, &ctx.data_dir, "s1", Ok(report), now).await;
        let rows = ctx.store.get_session_snapshots("s1").await.unwrap();
        assert_eq!(rows.len(), 1, "tab {i}: only the read board is stored");
        match &stored_after_first {
            None => stored_after_first = Some(rows),
            Some(first) => assert_eq!(&rows, first, "tab {i} changed the stored row"),
        }
        let game = st.active_game.as_ref().expect("the game stays open");
        assert_eq!(game.session_id, "s1");
        assert_eq!(game.outcome, detect::MatchOutcome::Victory);
        assert_eq!(game.map.as_deref(), Some("Busan"));
        now += Duration::from_secs(20);
    }
    let row = &stored_after_first.unwrap()[0];
    assert_eq!(row.hero, "Zenyatta");
    assert_eq!(row.map_name, "Busan");
    assert_eq!((row.elims, row.deaths, row.damage), (10, 2, 3000));
    assert_ne!(row.outcome, "unknown");
}

/// The merge gate for `ocr = "off"`, in two halves.
///
/// 1. While the stat cells or the own row are not read without OCR on
///    this build, off has to be refused, and the refusal names them.
/// 2. The Tab pass with OCR off, alone in a process
///    ([`own_process_tab_pass_with_ocr_off`]): the real `analyze_frame`
///    on synthetic Tabs, with the process-wide mode off, so a read on
///    any thread is seen. No read may reach the gate from a read this
///    build says it does without OCR. A Tab with a lit row is stored
///    with its stats, the own row found by `analyze_frame` itself.
#[test]
fn with_ocr_off_a_board_the_new_reader_reads_is_stored_with_its_stats() {
    use stat_tracker::config::OcrSetting;
    use stat_tracker::ocr_switch::{self, PackReadiness, Read};
    let every_pack = PackReadiness {
        heroes: true,
        maps: true,
        words: true,
        result_words: true,
        phase_titles: true,
        end_titles: true,
    };
    let board_reads = [Read::StatCells, Read::OwnRow];
    if board_reads.iter().any(|read| !read.without_ocr()) {
        let decision = ocr_switch::decide(OcrSetting::Off, &every_pack, &Read::needing_ocr());
        assert_eq!(
            decision.effective,
            OcrSetting::On,
            "no board reader without OCR on this build: off has to be refused"
        );
        let refusal = decision.refusal.expect("the refusal says why");
        for read in board_reads.iter().filter(|read| !read.without_ocr()) {
            assert!(refusal.reads.contains(read), "{read:?} is named");
        }
    }

    let exe = std::env::current_exe().expect("this test binary");
    // Empty HOME and XDG dirs: the process-wide reader of that process
    // loads no pack of this machine, and no tracker file is read.
    let home = tempfile::tempdir().unwrap();
    let run = std::process::Command::new(exe)
        .args([
            "--ignored",
            "--exact",
            "main_tests::ocr_off_switch::own_process_tab_pass_with_ocr_off",
            "--test-threads",
            "1",
            "--nocapture",
        ])
        .env(OWN_PROCESS_ENV, "1")
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env_remove("SCUFFED_TEMPLATE_DIR")
        .output()
        .expect("run the Tab pass in its own process");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        run.status.success() && text.contains("1 passed"),
        "the Tab pass with OCR off, in its own process:\n{text}"
    );
}

/// Run by [`with_ocr_off_a_board_the_new_reader_reads_is_stored_with_its_stats`],
/// alone in a process, so the gate's mode and counters are this test's:
/// a read made on the OCR pool or in `spawn_blocking` is blocked and
/// counted like one made here (a scoped mode would miss it).
#[tokio::test]
#[ignore = "run alone in a process by with_ocr_off_a_board_the_new_reader_reads_is_stored_with_its_stats"]
async fn own_process_tab_pass_with_ocr_off() {
    use stat_tracker::ocr_switch::Read;
    assert_eq!(
        std::env::var(OWN_PROCESS_ENV).as_deref(),
        Ok("1"),
        "this test sets the process-wide OCR mode: run it through its starter"
    );
    ocr::gate::set_mode(ocr::gate::OcrMode::Off);
    assert_eq!(ocr::gate::mode(), ocr::gate::OcrMode::Off);

    let dir = tempfile::tempdir().unwrap();
    let matcher = Arc::new(detect::hero_portrait::PortraitMatcher::load(dir.path()));
    let analysed = analyze_frame(
        s1_tab_frame(),
        matcher,
        Some("PLAYER".into()),
        detect::MatchOutcome::Unknown,
        FrameFlags {
            allow_banner_recovery: true,
            session_map_known: false,
            shadow_on: false,
            // `ocr = "off"` implies the new reader
            use_new_reader: true,
        },
        // no own-name seed yet: a first run
        Vec::new(),
        &names::NameSwitch::off(),
    );
    let FrameAnalysisOutcome::Analyzed(analysis) = analysed else {
        panic!("the synthetic Tab was rejected before the reads");
    };

    // 1. Every call the pass sent to the gate belongs to a read this
    //    build still needs OCR for. A read marked as done without OCR
    //    that still calls OCR fails here, whatever thread made the call.
    //    Every such call is named, not only the first.
    let check_blocked = |when: &str| {
        let mut wrong = Vec::new();
        for b in ocr::gate::blocked_calls() {
            match tab_pass_read(b.site.entry) {
                None => wrong.push(format!(
                    "{} is an entry point with no read: add it to tab_pass_read",
                    b.site
                )),
                Some(read) if read.without_ocr() => wrong.push(format!(
                    "{read:?} is marked as read without OCR, and {} still asked for it \
                     {} time(s)",
                    b.site, b.count
                )),
                Some(_) => {}
            }
        }
        assert!(wrong.is_empty(), "{when}:\n{}", wrong.join("\n"));
    };
    check_blocked("the Tab pass");
    let blocked_for = |read: Read| -> u64 {
        ocr::gate::blocked_calls()
            .iter()
            .filter(|b| tab_pass_read(b.site.entry) == Some(read))
            .map(|b| b.count)
            .sum()
    };
    // The stat cells are read without OCR on this build, so the cell
    // read must not be at the gate at all.
    assert_eq!(
        blocked_for(Read::StatCells),
        0,
        "{:?}",
        ocr::gate::blocked_calls()
    );
    // 2. Nothing of Tesseract was started, on any thread.
    assert_eq!(ocr::gate::leptess_created(), 0, "no LepTess");
    assert_eq!(ocr::gate::tesseract_calls(), 0, "no Tesseract read");
    assert!(!ocr::pool_started(), "no OCR pool");
    assert!(!ocr::tessdata_lang_probed(), "no tessdata lookup");

    // The plain synthetic Tab has no lit row, no seed and no hero icon:
    // no own row, and the name read was not asked for (nothing above
    // went to the gate, so not the name cells either).
    assert_eq!(analysis.player_row_idx, None);
    assert_eq!(
        blocked_for(Read::OwnRow),
        0,
        "{:?}",
        ocr::gate::blocked_calls()
    );

    // 3. A board the new reader read is stored, with its stats, and
    //    nothing of it goes to OCR.
    //
    //    The Tab has a lit row, and the own row is the one the real
    //    pass finds on it, with OCR off. One thing is given, because a
    //    synthetic frame cannot carry it: the digits (the reader's
    //    board, with 10 / 4 / 2 / 3000 / 800 / 400 on the lit row).
    const LIT: usize = 1;
    let matcher = Arc::new(detect::hero_portrait::PortraitMatcher::load(dir.path()));
    let analysed = analyze_frame(
        s1_tab_frame_lit(LIT),
        matcher,
        Some("PLAYER".into()),
        detect::MatchOutcome::Unknown,
        FrameFlags {
            allow_banner_recovery: true,
            session_map_known: false,
            shadow_on: false,
            use_new_reader: true,
        },
        Vec::new(),
        &names::NameSwitch::off(),
    );
    let FrameAnalysisOutcome::Analyzed(analysis) = analysed else {
        panic!("the lit synthetic Tab was rejected before the reads");
    };
    let analysis = *analysis;
    check_blocked("the Tab pass on the lit Tab");
    assert_eq!(
        analysis.player_row_idx,
        Some(LIT),
        "the own row is the pass's own: {:?}",
        analysis.own_row
    );
    assert_eq!(
        analysis.own_row.map(|own| own.source),
        Some(detect::own_row::Source::HighlightOnly)
    );
    assert_eq!(analysis.name_match, None, "no name was read for it");
    let mut board = o8_board(Some("zenyatta"));
    board.team_size = Some(analysis.team_size);
    for field in &mut board.fields {
        if let Some(rest) = field.name.strip_prefix("r0.") {
            field.name = format!("r{LIT}.{rest}");
        }
    }
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let store_dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(store_dir.path(), true).await;
    let (rows, recorded) = o10_tabs_on(&ctx, vec![(analysis, Some(board))], "Busan").await;
    assert_eq!(
        recorded,
        vec![true],
        "with OCR off a board the new reader read has to be stored"
    );
    check_blocked("storing the board");
    assert_eq!(
        ocr::gate::blocked_total(),
        0,
        "nothing of the two passes or of storing was asked of OCR"
    );
    assert_eq!(ocr::gate::leptess_created(), 0);
    assert_eq!(ocr::gate::tesseract_calls(), 0);
    assert!(!ocr::pool_started(), "no OCR pool");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        o10_stats(&rows[0]),
        [10, 4, 2, 3000, 800, 400],
        "the new reader's numbers"
    );
    assert_eq!(rows[0].map_name, "Busan");
}

/// F261-9. Startup may wait for the reader packs before it decides the
/// OCR switch. A stop that comes during that wait ends it at once, and
/// startup then leaves as after a normal stop: the SIGTERM handler is
/// registered before the wait, the run marker is finished, and the same
/// handler goes on to the run loop.
#[tokio::test]
async fn a_stop_during_the_startup_pack_wait_ends_it_cleanly() {
    let never = std::future::pending::<()>;
    assert!(
        !stopped_while_waiting(Duration::from_millis(30), never(), never()).await,
        "the limit ends the wait"
    );
    assert!(
        !stopped_while_waiting(Duration::from_secs(30), async {}, never()).await,
        "the packs arriving ends the wait"
    );
    let started = Instant::now();
    assert!(
        stopped_while_waiting(Duration::from_secs(30), never(), async {}).await,
        "a stop ends the wait"
    );
    assert!(started.elapsed() < Duration::from_secs(5));

    // The wait is on the pack job's first try, not on the job: `run`
    // returns only when every pack is settled.
    let config = config::Config {
        data_dir: tempfile::tempdir().unwrap().path().to_path_buf(),
        ..config::Config::default()
    };
    let first_try =
        |missing: Vec<&'static str>, next_try_in: Option<Duration>| stat_tracker::packs::FirstTry {
            missing,
            next_try_in,
        };
    let limit = Duration::from_secs(45);
    // Every pack arrived, or the job ended, or its next try is far off
    // or past the limit: go on at once.
    for first in [
        first_try(vec![], None),
        first_try(vec![], Some(Duration::from_secs(5))),
        first_try(vec!["maps-v1"], None),
        first_try(vec!["maps-v1"], Some(Duration::from_secs(60))),
        first_try(vec!["maps-v1"], Some(Duration::from_secs(3600))),
    ] {
        assert!(!keep_waiting_for_packs(&first, limit), "{first:?}");
        let (send, recv) = tokio::sync::oneshot::channel();
        send.send(first.clone()).unwrap();
        let started = Instant::now();
        reader_packs_arrive(limit, recv, &config).await;
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{first:?} does not hold startup"
        );
    }
    // A next try soon and inside what is left: packs can still arrive.
    let soon = first_try(vec!["maps-v1"], Some(Duration::from_secs(10)));
    assert!(keep_waiting_for_packs(&soon, limit));
    assert!(
        !keep_waiting_for_packs(&soon, Duration::from_secs(10)),
        "the next try is not before the wait ends"
    );
    // The job dropped before its first try: nothing to wait for.
    let (send, recv) = tokio::sync::oneshot::channel::<stat_tracker::packs::FirstTry>();
    drop(send);
    reader_packs_arrive(limit, recv, &config).await;

    // The order in `run`, by its source.
    let src = include_str!("../main.rs");
    let at = src.find("\nasync fn run(").expect("run");
    let body = &src[at..];
    let body = &body[..body
        .find("\nasync fn stopped_while_waiting")
        .expect("the helper")];
    let wait = body.find("stopped_while_waiting(").expect("the wait");
    let handler = body
        .find("tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())")
        .expect("the early handler");
    let decide = body.find("Switch::start(&config)").expect("the switch");
    assert!(
        handler < wait && wait < decide,
        "the handler is registered before the wait, and the wait is before the decision"
    );
    let first = body.find("extras.first_try_done()").expect("the first try");
    let spawn = body.find("tokio::spawn(extras.run());").expect("the job");
    assert!(
        first < spawn && spawn < wait,
        "the first-try channel is taken before the job starts, and the wait follows"
    );
    assert!(
        body.contains(".is_some_and(|job| job.starts_with_downloads())"),
        "no wait when the pack job's first move is a wait"
    );
    let stop_branch = &body[body[wait..].find("if stopped {").expect("the stop branch") + wait..];
    let stop_branch = &stop_branch[..stop_branch.find("early_sigterm = Some(sigterm);").unwrap()];
    assert!(
        stop_branch.contains("guard.finish();") && stop_branch.contains("return Ok(());"),
        "a stop during the wait finishes the run marker and returns Ok"
    );
    assert!(
        body.contains("run_loop(ctx, early_sigterm)"),
        "the same handler goes on to the run loop"
    );
}

/// With classic OCR off the accolade map is the map pack's read or
/// nothing. The OCR read behind a missing map pack is not asked for, so
/// nothing of it reaches the gate
/// ([`stat_tracker::ocr_switch::Read::AccoladeMap`]). With OCR on that
/// fallback is as #265 left it.
#[test]
fn with_ocr_off_the_accolade_map_is_never_asked_of_ocr() {
    use detect::match_end::OutcomeSource;
    use stat_tracker::ocr_switch::Read;
    let frame = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        1920,
        1080,
        image::Rgb([16, 25, 50]),
    ));
    // The daemon's own OCR read as the fallback, on a result-word tick.
    let choose = |pack: shadow::AccoladeRead| {
        choose_accolade_map(
            true,
            false,
            Some(OutcomeSource::ResultWord),
            || pack,
            || detect::match_end::read_accolade_map(&frame),
        )
    };
    let off = ocr::gate::scoped(ocr::gate::OcrMode::Off);
    // That OCR read does reach the gate when it is called.
    assert_eq!(detect::match_end::read_accolade_map(&frame), None);
    assert_eq!(off.blocked_total(), 1, "the frame reaches the OCR read");
    // No map pack, or a pack that abstains: no map, and no OCR call.
    assert_eq!(choose(shadow::AccoladeRead::NoPack), None);
    assert_eq!(choose(shadow::AccoladeRead::Abstain), None);
    assert_eq!(
        off.blocked_total(),
        1,
        "the accolade map was asked of OCR with classic OCR off: {:?}",
        off.blocked()
    );
    assert!(
        Read::AccoladeMap.without_ocr(),
        "the accolade map is read without OCR on this build"
    );
    drop(off);
    // With OCR on, no map pack still falls back to the OCR read.
    let asked = std::cell::Cell::new(false);
    let map = choose_accolade_map(
        true,
        false,
        Some(OutcomeSource::ResultWord),
        || shadow::AccoladeRead::NoPack,
        || {
            asked.set(true);
            Some("Busan".to_string())
        },
    );
    assert!(asked.get(), "with OCR on the fallback runs");
    assert_eq!(map.as_deref(), Some("Busan"));
}

/// F261-1. With classic OCR off the co-op event board check works
/// from its two sources without OCR
/// ([`stat_tracker::ocr_switch::Read::PveBoard`]): the word from a sure
/// `ADLERSBRUNN` banner read (#271) and the teams from the new reader's
/// board (#266), through `handle_capture`.
///
/// * A board with stats on one team only (Junkenstein) is not stored.
/// * A board with stats on both teams is stored, as Eichenwalde.
/// * The limit: the word exists only on a sure banner read. When the
///   banner is read unsure the Tab has no top-bar map and the check does
///   not fire, so a co-op board is stored in the open game. With OCR on
///   the label OCR is a second chance to read the word.
#[tokio::test]
async fn with_ocr_off_a_coop_board_is_not_stored_and_a_two_team_adlersbrunn_game_is() {
    use stat_tracker::ocr_switch::Read;
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    // One Tab as the pass with OCR off hands it over: no label OCR and
    // no board text, the rows unread, and the top bar from the banner
    // read alone.
    let tab = |banner_suspect: bool, both_teams: bool| {
        let banner = Some(banner_read(shadow::banner::ADLERSBRUNN_KEY, banner_suspect));
        let top = choose_top_bar_map(true, || banner, None);
        let mut analysis = s1_unread_analysis();
        analysis.rows = ocr::unread_rows(5);
        analysis.player_row_idx = Some(0);
        analysis.pack_hero = Some("ana".into());
        analysis.map_panel_raw = top_bar_raw_label(top.banner.as_ref(), None);
        analysis.map_from_panel = top.map;
        analysis.map_from_banner = top.from_banner;
        let mut board = o8_board(Some("ana"));
        if both_teams {
            for suffix in reader_apply::STAT_SUFFIXES {
                board.fields.push(shadow::FieldRead {
                    name: format!("r7.{suffix}"),
                    value: Some(shadow::Value::Int(3)),
                    confidence: 0.9,
                    suspect: false,
                });
            }
        }
        (analysis, Some(board))
    };
    let off = ocr::gate::scoped(ocr::gate::OcrMode::Off);
    let run = |tab: (FrameAnalysis, Option<shadow::BoardRead>)| async move {
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        o10_tabs_on(&ctx, vec![tab], "Eichenwalde").await
    };

    // The word arrives without OCR.
    let (sure, _) = tab(false, true);
    assert_eq!(
        sure.map_panel_raw.as_deref(),
        Some(shadow::banner::ADLERSBRUNN)
    );
    assert_eq!(sure.map_from_panel.as_deref(), Some("Eichenwalde"));

    let (rows, recorded) = run(tab(false, true)).await;
    assert_eq!(recorded, vec![true], "two teams: a real game");
    assert_eq!(rows[0].map_name, "Eichenwalde");

    let (rows, recorded) = run(tab(false, false)).await;
    assert_eq!(recorded, vec![false], "one team only: the co-op board");
    assert!(rows.is_empty());

    // The limit. An unsure banner read says no word and gives no map.
    let (unsure, _) = tab(true, false);
    assert_eq!(unsure.map_panel_raw, None);
    assert_eq!(unsure.map_from_panel, None);
    let (_, recorded) = run(tab(true, false)).await;
    assert_eq!(
        recorded,
        vec![true],
        "the limit: without a sure banner read the co-op board is not told apart"
    );

    assert_eq!(off.blocked(), [], "none of it asked anything of OCR");
    assert!(
        Read::PveBoard.without_ocr(),
        "the check has both its sources without OCR on this build"
    );
}
