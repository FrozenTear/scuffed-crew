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

/// R9-2, the KDE tester's 2026-10-22 Route 66 game: DEFEAT was taken
/// mid-match, Tabs inside the 75 s grace window kept showing the board
/// rising, and the first Tab after it (board 10/9) closed the game
/// "superseded by Tab-opened game" and opened a new one. Now that board
/// keeps the game, the false result is cleared on the game and in the
/// store along with the result words behind it (review M1: `reader =
/// "new"` would stamp them on every later row), the probe frame's own
/// VICTORY is not taken (review A3), and the real result comes through
/// the poller's word path.
#[tokio::test]
async fn a_board_still_rising_after_a_false_result_keeps_the_game_and_clears_it() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let synced = std::cell::Cell::new(0);
    let (mut st, t0) = r9_unlatched_game(&ctx, &synced).await;
    assert!(st.tab_probe.is_none(), "the probe is settled");
    let g = st.active_game.as_ref().unwrap();
    assert_eq!(g.session_id, "live", "no new session");
    assert_eq!(
        g.outcome,
        detect::MatchOutcome::Unknown,
        "the false result is cleared, and the probe frame's word is not taken"
    );
    assert!(g.outcome_recorded_at.is_none());
    assert!(g.result_mark.is_none());
    assert!(g.result_evidence.is_empty());
    assert_eq!(g.gate.map(|gate| gate.accepted.elims), Some(10));
    assert_eq!(r9_final_outcome(&ctx, "live").await, "unknown");
    assert_eq!(ctx.store.get_all_sessions().await.unwrap().len(), 1);
    assert_eq!(synced.get(), 0, "nothing closed, nothing uploaded early");
    // A later Tab is an ordinary mid-game Tab of the same game, and its
    // row carries no result either.
    let press = t0 + Duration::from_secs(190);
    assert!(!finished_game_tab_probe(st.active_game.as_ref(), press));
    r9_tab(
        &mut st,
        &ctx,
        press,
        Ok(r9_frame(r9_counters(12, 10, 5200), "Busan")),
        &synced,
    )
    .await;
    assert_eq!(
        r9_rows(&ctx, "live").await,
        vec![
            (6, "unknown".into()),
            (7, "unknown".into()),
            (9, "unknown".into()),
            (9, "unknown".into()),
            (10, "unknown".into()),
            (12, "unknown".into()),
        ]
    );
    // The real end: the poller's two agreeing words record it.
    r9_word(
        &mut st,
        &ctx,
        detect::MatchOutcome::Victory,
        t0 + Duration::from_secs(300),
    )
    .await;
    assert!(
        !st.active_game.as_ref().unwrap().finished(),
        "one word is a hint"
    );
    r9_word(
        &mut st,
        &ctx,
        detect::MatchOutcome::Victory,
        t0 + Duration::from_secs(306),
    )
    .await;
    let g = st.active_game.as_ref().unwrap();
    assert_eq!(g.session_id, "live");
    assert_eq!(g.outcome, detect::MatchOutcome::Victory);
    assert_eq!(r9_final_outcome(&ctx, "live").await, "victory");
    assert!(
        r9_rows(&ctx, "live")
            .await
            .iter()
            .all(|(_, o)| o == "victory"),
        "every row of the game takes the real result"
    );
}

/// R9-2 review round 2 (B #3): the false result's word streak goes with
/// it, so one more misread of the same word does not finish the game
/// again; two new agreeing reads still do.
#[tokio::test]
async fn a_misread_after_the_clear_needs_two_new_reads() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let synced = std::cell::Cell::new(0);
    let (mut st, t0) = r9_unlatched_game(&ctx, &synced).await;
    assert!(st.word_outcome_streak.is_none());
    r9_word(
        &mut st,
        &ctx,
        detect::MatchOutcome::Defeat,
        t0 + Duration::from_secs(155),
    )
    .await;
    assert!(
        !st.active_game.as_ref().unwrap().finished(),
        "one read is a hint"
    );
    r9_word(
        &mut st,
        &ctx,
        detect::MatchOutcome::Defeat,
        t0 + Duration::from_secs(161),
    )
    .await;
    assert_eq!(
        st.active_game.as_ref().unwrap().outcome,
        detect::MatchOutcome::Defeat
    );
}

/// R9-2 review round 2 (R2-1): the last board's Tab was pressed before
/// the result, but its report landed after it (the poller confirmed the
/// result while the capture ran). The board still counts as from before
/// the result: a rise on the frozen final board past the grace window
/// does not clear the real result.
#[tokio::test]
async fn a_board_pressed_before_the_result_counts_as_before_it_even_if_reported_after() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let synced = std::cell::Cell::new(0);
    let t0 = test_now();
    let mut active = game(detect::MatchOutcome::Unknown, None, t0);
    active.session_id = "live".into();
    active.map = Some("Busan".into());
    active.map_source = Some(boundary::MapSource::TopBar);
    active.session_created = false;
    active.opened_at = t0;
    let mut st = session(Some(active), t0);
    for (at, c) in [(0, r9_counters(6, 6, 3000)), (40, r9_counters(7, 7, 3500))] {
        let press = t0 + Duration::from_secs(at);
        r9_tab(&mut st, &ctx, press, Ok(r9_frame(c, "Busan")), &synced).await;
    }
    // Tab at 70 s; the poller confirms VICTORY at 66 and 72 s while it
    // runs; the report lands at 74 s.
    let press = t0 + Duration::from_secs(70);
    let req = tab_capture_request(&mut st, &ctx.store, &ctx.data_dir, press, || {
        std::future::ready(())
    })
    .await;
    let sid = req.session_id.clone();
    *INJECTED_CAPTURE.lock().unwrap() = Some(r9_frame(r9_counters(9, 8, 4200), "Busan"));
    let result = handle_capture(&ctx, req)
        .await
        .map_err(|e| format!("{e:#}"));
    r9_word(
        &mut st,
        &ctx,
        detect::MatchOutcome::Victory,
        t0 + Duration::from_secs(66),
    )
    .await;
    r9_word(
        &mut st,
        &ctx,
        detect::MatchOutcome::Victory,
        t0 + Duration::from_secs(72),
    )
    .await;
    let landed = t0 + Duration::from_secs(74);
    let sid = settle_tab_probe(
        &mut st,
        &ctx.store,
        &ctx.data_dir,
        sid,
        &result,
        landed,
        || std::future::ready(()),
    )
    .await;
    apply_capture_report(&mut st, &ctx.store, &ctx.data_dir, &sid, result, landed).await;
    let g = st.active_game.as_ref().unwrap();
    assert_eq!(g.outcome, detect::MatchOutcome::Victory);
    assert_eq!(g.last_stats_at, Some(landed), "reported after the result");
    assert_eq!(g.last_board_pressed_at, Some(press), "pressed before it");
    // The frozen final board, opened past the grace window.
    let press = t0 + Duration::from_secs(150);
    assert!(finished_game_tab_probe(st.active_game.as_ref(), press));
    let verdict = r9_tab(
        &mut st,
        &ctx,
        press,
        Ok(r9_frame(r9_counters(10, 9, 4600), "Busan")),
        &synced,
    )
    .await;
    assert_eq!(verdict, Some(boundary::FinishedGameTab::NewGame));
    assert_ne!(st.active_game.as_ref().unwrap().session_id, "live");
    assert_eq!(r9_final_outcome(&ctx, "live").await, "victory");
}

/// R9-2 review round 2 (N1): a result the member sets while the
/// clearing Tab's capture runs is kept.
#[tokio::test]
async fn a_result_set_by_the_member_during_the_probe_is_kept() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let synced = std::cell::Cell::new(0);
    let (mut st, t0) = r9_finished_game(&ctx, detect::MatchOutcome::Defeat, &synced).await;
    r9_tab(
        &mut st,
        &ctx,
        t0 + Duration::from_secs(110),
        Ok(r9_frame(r9_counters(9, 8, 4300), "Busan")),
        &synced,
    )
    .await;
    let press = t0 + Duration::from_secs(150);
    let req = tab_capture_request(&mut st, &ctx.store, &ctx.data_dir, press, || {
        std::future::ready(())
    })
    .await;
    assert!(req.finished_game_probe.is_some());
    let sid = req.session_id.clone();
    // The member sets Victory in the app while the capture runs.
    st.active_game
        .as_mut()
        .unwrap()
        .record_member_outcome(detect::MatchOutcome::Victory);
    ctx.store
        .set_session_outcome("live", "victory")
        .await
        .unwrap();
    *INJECTED_CAPTURE.lock().unwrap() = Some(r9_frame(r9_counters(10, 9, 4600), "Busan"));
    let result = handle_capture(&ctx, req)
        .await
        .map_err(|e| format!("{e:#}"));
    assert_eq!(
        result.as_ref().ok().and_then(|r| r.finished_game_tab),
        Some(boundary::FinishedGameTab::StillPlaying)
    );
    let now = press + Duration::from_secs(1);
    let sid = settle_tab_probe(
        &mut st,
        &ctx.store,
        &ctx.data_dir,
        sid,
        &result,
        now,
        || std::future::ready(()),
    )
    .await;
    apply_capture_report(&mut st, &ctx.store, &ctx.data_dir, &sid, result, now).await;
    let g = st.active_game.as_ref().unwrap();
    assert_eq!(g.session_id, "live");
    assert_eq!(g.outcome, detect::MatchOutcome::Victory);
    assert_eq!(r9_final_outcome(&ctx, "live").await, "victory");
    assert!(
        r9_rows(&ctx, "live")
            .await
            .iter()
            .all(|(_, o)| o == "victory")
    );
}

/// R9-2: a new game on the same map right after a loss. Its board is
/// back near zero: the loss stays, and the Tab opens the next game as
/// before rc.9 (the board stored there, the closed game uploaded).
#[tokio::test]
async fn a_reset_board_on_the_same_map_after_a_loss_is_the_next_game() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let synced = std::cell::Cell::new(0);
    let (mut st, t0) = r9_finished_game(&ctx, detect::MatchOutcome::Defeat, &synced).await;
    let press = t0 + Duration::from_secs(150);
    assert!(finished_game_tab_probe(st.active_game.as_ref(), press));
    let verdict = r9_tab(
        &mut st,
        &ctx,
        press,
        Ok(r9_frame(r9_counters(1, 0, 400), "Busan")),
        &synced,
    )
    .await;
    assert_eq!(verdict, Some(boundary::FinishedGameTab::NewGame));
    let g = st.active_game.as_ref().unwrap();
    assert_ne!(g.session_id, "live");
    assert_eq!(g.opened_at, press, "the new game opens as of the press");
    assert_eq!(g.outcome, detect::MatchOutcome::Unknown);
    let fresh = g.session_id.clone();
    assert_eq!(r9_rows(&ctx, &fresh).await, vec![(1, "unknown".into())]);
    assert_eq!(r9_final_outcome(&ctx, "live").await, "defeat");
    assert!(
        r9_rows(&ctx, "live")
            .await
            .iter()
            .all(|(_, o)| o == "defeat"),
        "the loss stays on the closed game"
    );
    assert_eq!(synced.get(), 1, "the closed game is uploaded at once");
}

/// R9-2: a late look at the same board after a real result (nothing
/// moved on), or one column moved (could be a misread), stays on that
/// game and keeps its result. It is not stored and does not restart the
/// 120 s window (review L2).
#[tokio::test]
async fn the_same_board_past_the_grace_window_keeps_the_game_and_its_result() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let synced = std::cell::Cell::new(0);
    let (mut st, t0) = r9_finished_game(&ctx, detect::MatchOutcome::Victory, &synced).await;
    let last_board = st.active_game.as_ref().unwrap().last_stats_at;
    for (at, c) in [
        (150, r9_counters(9, 8, 4200)),
        (170, r9_counters(10, 8, 4200)),
    ] {
        let verdict = r9_tab(
            &mut st,
            &ctx,
            t0 + Duration::from_secs(at),
            Ok(r9_frame(c, "Busan")),
            &synced,
        )
        .await;
        assert_eq!(verdict, Some(boundary::FinishedGameTab::SameBoard), "{c:?}");
        let g = st.active_game.as_ref().unwrap();
        assert_eq!(g.session_id, "live");
        assert_eq!(g.outcome, detect::MatchOutcome::Victory);
        assert_eq!(g.last_stats_at, last_board, "the window is not restarted");
        assert_eq!(g.gate.map(|gate| gate.accepted.elims), Some(9));
    }
    assert_eq!(
        r9_rows(&ctx, "live").await.len(),
        3,
        "nothing stored past the result"
    );
    assert_eq!(r9_final_outcome(&ctx, "live").await, "victory");
    assert_eq!(ctx.store.get_all_sessions().await.unwrap().len(), 1);
    assert_eq!(synced.get(), 0);
    // The window runs from the game's own last board: past it, the
    // fresh session opens at the press as before.
    let press = t0 + Duration::from_secs(72) + FINISHED_GAME_TAB_MAX_GAP;
    assert!(!finished_game_tab_probe(st.active_game.as_ref(), press));
}

/// R9-2: what cannot be shown to be the same game keeps the rule before
/// rc.9: another map, a misread drop, a Tab whose frame was missed, a
/// finished game whose last board is too old to compare with, and (review
/// H1) the frozen final board of a real result whose last Tab came
/// before the end: it rose through the last fight, and the result stays.
#[tokio::test]
async fn a_finished_game_tab_that_is_not_shown_to_be_the_same_game_opens_the_next() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let missed = format!("{} (no answer in 60 s)", capture::portal::SHOT_UNANSWERED);
    for (case, result, frame) in [
        (
            "another map",
            detect::MatchOutcome::Defeat,
            Ok(r9_frame(r9_counters(10, 9, 4600), "Ilios")),
        ),
        (
            "elims fell",
            detect::MatchOutcome::Defeat,
            Ok(r9_frame(r9_counters(5, 9, 4600), "Busan")),
        ),
        (
            "missed frame",
            detect::MatchOutcome::Defeat,
            Err(missed.as_str()),
        ),
        (
            "final board after a real result",
            detect::MatchOutcome::Victory,
            Ok(r9_frame(r9_counters(10, 9, 4600), "Busan")),
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let synced = std::cell::Cell::new(0);
        let (mut st, t0) = r9_finished_game(&ctx, result, &synced).await;
        let press = t0 + Duration::from_secs(150);
        assert!(
            finished_game_tab_probe(st.active_game.as_ref(), press),
            "{case}"
        );
        r9_tab(&mut st, &ctx, press, frame, &synced).await;
        let g = st.active_game.as_ref().unwrap();
        assert_ne!(g.session_id, "live", "{case}");
        assert_eq!(g.opened_at, press, "{case}");
        assert_eq!(
            r9_final_outcome(&ctx, "live").await,
            result.to_string(),
            "{case}: the result stays"
        );
        assert_eq!(synced.get(), 1, "{case}");
        assert!(st.tab_probe.is_none(), "{case}");
    }

    // The last board is more than 120 s old: no probe, the fresh
    // session opens at the press as before.
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let synced = std::cell::Cell::new(0);
    let (mut st, t0) = r9_finished_game(&ctx, detect::MatchOutcome::Defeat, &synced).await;
    // The last board was taken at `t0 + 71 s` (its report landed then).
    let press = t0 + Duration::from_secs(72) + FINISHED_GAME_TAB_MAX_GAP;
    assert!(!finished_game_tab_probe(st.active_game.as_ref(), press));
    let req = tab_capture_request(&mut st, &ctx.store, &ctx.data_dir, press, || {
        synced.set(synced.get() + 1);
        std::future::ready(())
    })
    .await;
    assert!(req.finished_game_probe.is_none());
    assert!(st.tab_probe.is_none());
    assert_ne!(st.active_game.as_ref().unwrap().session_id, "live");
    assert_eq!(req.session_id, st.active_game.as_ref().unwrap().session_id);
    assert_eq!(synced.get(), 1);
}

/// R9-2 review (M4): a result the member set in the app is never
/// cleared by a Tab, even a rising board taken after it. The GUI's
/// SetOutcome and EditMatch go through `record_member_outcome`; the Tab
/// past the grace window opens the next game as before rc.9.
#[tokio::test]
async fn a_result_the_member_set_is_never_cleared_by_a_tab() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let synced = std::cell::Cell::new(0);
    let (mut st, t0) = r9_finished_game(&ctx, detect::MatchOutcome::Defeat, &synced).await;
    // The member sets Victory by hand at t0 + 80 s (the GUI command's
    // store write and its in-memory effect).
    let g = st.active_game.as_mut().unwrap();
    g.record_member_outcome(detect::MatchOutcome::Victory);
    g.outcome_recorded_at = Some(t0 + Duration::from_secs(80));
    assert!(g.outcome_locked);
    ctx.store
        .set_session_outcome("live", "victory")
        .await
        .unwrap();
    // A rising board inside the edit's grace window, then past it.
    r9_tab(
        &mut st,
        &ctx,
        t0 + Duration::from_secs(120),
        Ok(r9_frame(r9_counters(10, 9, 4600), "Busan")),
        &synced,
    )
    .await;
    let press = t0 + Duration::from_secs(200);
    assert!(should_start_fresh_session(st.active_game.as_ref(), press));
    assert!(!finished_game_tab_probe(st.active_game.as_ref(), press));
    st.active_game.as_mut().unwrap().outcome_locked = false;
    assert!(
        finished_game_tab_probe(st.active_game.as_ref(), press),
        "only the member's lock keeps this Tab from being compared"
    );
    st.active_game.as_mut().unwrap().outcome_locked = true;
    let verdict = r9_tab(
        &mut st,
        &ctx,
        press,
        Ok(r9_frame(r9_counters(11, 10, 5000), "Busan")),
        &synced,
    )
    .await;
    assert_eq!(verdict, None, "no probe");
    assert_ne!(st.active_game.as_ref().unwrap().session_id, "live");
    assert_eq!(r9_final_outcome(&ctx, "live").await, "victory");
    // A result the tracker reads itself takes the lock off.
    let mut read = game(detect::MatchOutcome::Unknown, None, press);
    read.outcome_locked = true;
    read.record_outcome_at(detect::MatchOutcome::Defeat, press);
    assert!(!read.outcome_locked);
}

/// R9-4: `handle_capture`'s arm for a Tab turned away by a paused or
/// unanswering screenshot portal: a quiet missed capture of the open
/// game, not a failed cycle. Nothing is stored and the game is as it was.
#[tokio::test]
async fn a_tab_turned_away_by_a_paused_portal_is_a_missed_capture_of_the_open_game() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let now = test_now();
    let mut active = game(detect::MatchOutcome::Defeat, Some(10), now);
    active.session_id = "open".into();
    active.gate = Some(GateState::default());
    active.last_stats_at = Some(now - Duration::from_secs(20));
    let mut st = session(Some(active), now);
    *INJECTED_GRAB_ERROR.lock().unwrap() = Some(format!(
        "{} (no answer in 60 s)",
        capture::portal::SHOT_UNANSWERED
    ));
    let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
    let report = handle_capture(&ctx, req)
        .await
        .expect("a missed capture, not an error");
    assert!(
        INJECTED_GRAB_ERROR.lock().unwrap().is_none(),
        "one grab, no retry"
    );
    assert!(!report.recorded);
    assert!(!report.board_taken());
    assert!(!report.split && !report.armed_reset);
    assert_eq!(report.session_id, "open");
    assert_eq!(report.outcome, detect::MatchOutcome::Defeat);
    assert!(report.gate_state.is_none());
    assert_eq!(report.finished_game_tab, None);
    assert!(
        !apply_capture_report(&mut st, &ctx.store, &ctx.data_dir, "open", Ok(report), now).await
    );
    let g = st.active_game.as_ref().unwrap();
    assert_eq!(g.session_id, "open");
    assert_eq!(g.outcome, detect::MatchOutcome::Defeat);
    assert_eq!(g.gate, Some(GateState::default()));
    assert!(ctx.store.get_all_matches().await.unwrap().is_empty());
}

/// R9-4, the mirror of the final pre-tag review case: a refused board
/// with no outcome on attempt 0, an unsure team size carrying VICTORY on
/// the retry. `tab_outcome_rank` ranks any analysed board above an
/// unsure team size whatever the outcomes, so the refused board is kept
/// and the unsure frame's VICTORY is not.
#[tokio::test]
async fn a_refused_board_without_an_outcome_still_outranks_an_unsure_victory() {
    use stat_tracker::tab_retry::{TAB_RETRY_WAITS, TabFlight, with_retries};
    let refused = || FrameAnalysisOutcome::Analyzed(Box::new(o8_analysis(None, None)));
    let unsure_victory = || FrameAnalysisOutcome::TeamSizeUncertain {
        outcome: detect::MatchOutcome::Victory,
        frame: image::DynamicImage::new_rgb8(4, 4),
        scan: detect::hero_portrait::RowScan {
            dip_count: 0,
            median_pitch: None,
            spectral_pitch: None,
        },
    };
    assert!(tab_outcome_rank(&refused(), true) > tab_outcome_rank(&unsure_victory(), true));
    let (best, retries) = with_retries(
        &TabFlight::new(Instant::now()),
        &TAB_RETRY_WAITS,
        |outcome| tab_result(outcome, true),
        |outcome| tab_outcome_rank(outcome, true),
        |outcome| tab_retry_reason(outcome, true),
        |attempt| {
            let outcome = if attempt == 0 {
                refused()
            } else {
                unsure_victory()
            };
            async move { Ok::<_, ()>(outcome) }
        },
        |_| async {},
    )
    .await
    .unwrap();
    // The kept board is a refused one, so the retries go on; every
    // unsure grab ranks below it.
    assert_eq!(retries, TAB_RETRY_WAITS.len());
    match best {
        FrameAnalysisOutcome::Analyzed(analysis) => {
            assert!(reader_refusal(&analysis, true).is_some());
            assert_eq!(analysis.frame_outcome, detect::MatchOutcome::Unknown);
        }
        _ => panic!("the refused board is kept"),
    }
}

/// R9-3: a refused end-of-match board carrying VICTORY, then a retry
/// whose screen grab fails: the capture keeps that board and its
/// outcome instead of failing the whole cycle.
#[tokio::test]
async fn a_failed_retry_grab_keeps_the_refused_board_and_its_outcome() {
    use stat_tracker::tab_retry::{TAB_RETRY_WAITS, TabFlight, with_retries};
    let refused_victory = || {
        let mut analysis = o8_analysis(None, None);
        analysis.frame_outcome = detect::MatchOutcome::Victory;
        FrameAnalysisOutcome::Analyzed(Box::new(analysis))
    };
    let (best, retries) = with_retries(
        &TabFlight::new(Instant::now()),
        &TAB_RETRY_WAITS,
        |outcome| tab_result(outcome, true),
        |outcome| tab_outcome_rank(outcome, true),
        |outcome| tab_retry_reason(outcome, true),
        |attempt| {
            let outcome = refused_victory();
            async move {
                if attempt == 0 {
                    Ok(outcome)
                } else {
                    Err(anyhow::anyhow!("screen capture failed"))
                }
            }
        },
        |_| async {},
    )
    .await
    .expect("the held board, not the retry's error");
    assert_eq!(retries, 1);
    match best {
        FrameAnalysisOutcome::Analyzed(analysis) => {
            assert!(reader_refusal(&analysis, true).is_some());
            assert_eq!(analysis.frame_outcome, detect::MatchOutcome::Victory);
        }
        _ => panic!("the refused board with its outcome is kept"),
    }
}
