//! Hero icon and stat digit training crops from a Tab: cut only from a
//! stored Tab of a known game, only with consent, raw and never resized,
//! and never in the way of recording.

use std::path::Path;
use std::time::Duration;

use crate::{main_tests::*, *};
use image::{DynamicImage, Rgb, RgbImage};
use scuffed_types::api::training_crops::TrainingCropConsent;
use stat_tracker::detect::own_row::{OwnRow, Source as OwnRowSource, Trust};
use stat_tracker::shadow::heroes::{HeroPack, HeroRead, PanelRead, RowClass};
use stat_tracker::sync::UploadKind;

fn set_consent(dir: &Path, hero_icon: bool, stat_digits: bool) {
    let consent = TrainingCropConsent {
        hero_icon,
        stat_digits,
        map_name: false,
    };
    let path = dir.join("training_crops").join("consent.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_vec(&consent).unwrap()).unwrap();
}

/// A pixel that names its own frame position (frames up to 2047 x 8191).
fn coord_pixel(x: u32, y: u32) -> Rgb<u8> {
    Rgb([
        (x & 0xff) as u8,
        (((x >> 8) & 0x07) | ((y & 0x1f) << 3)) as u8,
        (y >> 5) as u8,
    ])
}

fn coord_of(p: &Rgb<u8>) -> (u32, u32) {
    let [r, g, b] = p.0;
    (
        u32::from(r) | ((u32::from(g) & 0x07) << 8),
        (u32::from(g) >> 3) | (u32::from(b) << 5),
    )
}

/// Header label centres of E, A, D, DMG, H and MIT as fractions of the
/// board width, from a real 5v5 board (the reader's header fixture, 1664
/// px wide): E/A/D close together, DMG/H/MIT further apart, and E just
/// right of the in-match name column.
const REAL_LABELS: [f64; 6] = [0.3206, 0.3549, 0.3912, 0.4441, 0.5093, 0.5661];

/// A 1920x1080 Tab with the six stat labels at [`REAL_LABELS`].
fn tab_frame() -> DynamicImage {
    tab_frame_with(&REAL_LABELS)
}

/// The left edge, in frame px, of the 8 px mark [`tab_frame_with`] centres
/// on `label` (a fraction of the board width).
fn label_start(label: f64) -> u32 {
    let (bx, _, bw, _) = ocr::preprocess::crop_scoreboard_rect(1920, 1080);
    bx + (label * f64::from(bw)).round() as u32 - 4
}

/// A 1920x1080 Tab: every pixel right of the chat strip names its own
/// position, and the scoreboard's header strip is a bright bar with a dark
/// 8 px mark centred on each of `labels` (fractions of the board width).
fn tab_frame_with(labels: &[f64]) -> DynamicImage {
    let (w, h) = (1920, 1080);
    let (bx, by, bw, bh) = ocr::preprocess::crop_scoreboard_rect(w, h);
    let header_end = by + (f64::from(bh) * 0.024) as u32;
    let (label_y0, label_y1) = (
        by + (f64::from(bh) * 0.008) as u32,
        by + (f64::from(bh) * 0.022) as u32,
    );
    let starts: Vec<u32> = labels.iter().map(|&c| label_start(c)).collect();
    let is_label = |x: u32, y: u32| {
        (label_y0..label_y1).contains(&y) && starts.iter().any(|&s| (s..s + 8).contains(&x))
    };
    let img = RgbImage::from_fn(w, h, |x, y| {
        if x < bx {
            Rgb([40, 40, 40])
        } else if (by..header_end).contains(&y) && (bx..bx + bw).contains(&x) {
            if is_label(x, y) {
                Rgb([16, 16, 16])
            } else {
                Rgb([232, 232, 232])
            }
        } else {
            coord_pixel(x, y)
        }
    });
    DynamicImage::ImageRgb8(img)
}

fn panel(box_px: (usize, usize, usize, usize), hero: &str, score: f32) -> PanelRead {
    PanelRead {
        read: HeroRead {
            row: 0,
            class: RowClass::Hero,
            hero: Some(hero.to_string()),
            best: hero.to_string(),
            score,
            lead: 0.5,
            second: "kiriko".to_string(),
            suspect: false,
        },
        pack: HeroPack::V2,
        box_px,
        rel: (0.0, 0.12, 0.16),
    }
}

/// The new reader's own row verdict naming `row`, at `trust`.
fn own(row: usize, trust: Trust) -> OwnRow {
    OwnRow {
        row: Some(row),
        trust,
        source: OwnRowSource::Both,
    }
}

/// A new reader Tab: its own row verdict names row 1 and is sure, and its
/// hero authority named Ana.
fn source<'a>(panel: Option<&'a PanelRead>, pack_hero: Option<&'a str>) -> TabCropSource<'a> {
    TabCropSource {
        panel,
        pack_hero,
        row: Some(1),
        own_row: Some(own(1, Trust::Sure)),
        team_size: 5,
        hero: Some("Ana"),
    }
}

fn decode(png: &[u8]) -> RgbImage {
    image::load_from_memory(png).unwrap().to_rgb8()
}

async fn run(dir: &Path, session: &str, frame: &DynamicImage, src: TabCropSource<'_>) -> bool {
    match archive_tab_training_crops(dir, session, frame, src) {
        Some(handle) => {
            handle.await.unwrap();
            true
        }
        None => false,
    }
}

#[tokio::test]
async fn a_stored_tab_keeps_the_raw_career_panel_portrait_with_its_hero() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let frame = tab_frame();
    let read = panel((1600, 200, 1800, 400), "ana", 0.97);
    assert!(
        run(
            dir.path(),
            "tc-hero",
            &frame,
            source(Some(&read), Some("ana"))
        )
        .await
    );

    let queued = sync::queued_crops(dir.path(), UploadKind::HeroIcon);
    assert_eq!(queued.len(), 1);
    // The raw panel box: same size, same pixels, nothing resized or
    // preprocessed. Not the hero name title under it.
    let want = frame.crop_imm(1600, 200, 200, 200).to_rgb8();
    assert_eq!(decode(&queued[0].png), want);
    let meta = &queued[0].meta;
    assert_eq!(meta.hero.as_deref(), Some("ana"));
    assert_eq!(meta.slot_type.as_deref(), Some("career_panel"));
    assert_eq!(meta.icon_set_version.as_deref(), Some("heroes-v2"));
    assert!((meta.score.unwrap() - 0.97).abs() < 1e-6);
    assert!((meta.margin.unwrap() - 0.5).abs() < 1e-6);
    assert!(sync::game_token_ok(meta.game_token.as_deref().unwrap()));
    assert_eq!(
        (meta.screen_width, meta.screen_height),
        (Some(1920), Some(1080))
    );
    assert!(meta.found_spot.is_none());
    // No stat digit consent: no stat digit crop.
    assert!(sync::queued_crops(dir.path(), UploadKind::StatDigits).is_empty());

    // One hero icon per hero per game.
    assert!(
        !run(
            dir.path(),
            "tc-hero",
            &frame,
            source(Some(&read), Some("ana"))
        )
        .await
    );
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::HeroIcon).len(),
        1
    );
}

#[tokio::test]
async fn without_consent_nothing_is_cut_or_kept() {
    let dir = tempfile::tempdir().unwrap();
    let frame = tab_frame();
    let read = panel((1600, 200, 1800, 400), "ana", 0.97);
    // No consent read yet.
    assert!(
        !run(
            dir.path(),
            "tc-off",
            &frame,
            source(Some(&read), Some("ana"))
        )
        .await
    );
    // Both kinds off.
    set_consent(dir.path(), false, false);
    assert!(
        !run(
            dir.path(),
            "tc-off",
            &frame,
            source(Some(&read), Some("ana"))
        )
        .await
    );
    assert!(!sync::has_queued_crops(dir.path()));
    assert!(!dir.path().join("training_crops").join("outbox").exists());
    assert_eq!(sync::training_warning(dir.path()), None);
}

#[tokio::test]
async fn no_hero_icon_without_an_accepted_panel_hero_or_a_box_in_the_frame() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let frame = tab_frame();
    let good = panel((1600, 200, 1800, 400), "ana", 0.97);
    let weak = panel((1600, 200, 1800, 400), "ana", 0.5);
    let outside = panel((1800, 200, 2000, 400), "ana", 0.97);
    let cases = [
        // ocr-v1: no panel read, no title or template crop in its place.
        source(None, None),
        // No hero authority, or one naming another hero.
        source(Some(&good), None),
        source(Some(&good), Some("kiriko")),
        // Under the panel gate.
        source(Some(&weak), Some("ana")),
        // Box past the frame's right edge.
        source(Some(&outside), Some("ana")),
    ];
    for (i, src) in cases.into_iter().enumerate() {
        assert!(
            !run(dir.path(), &format!("tc-nohero-{i}"), &frame, src).await,
            "case {i}"
        );
    }
    assert!(!sync::has_queued_crops(dir.path()));
}

#[tokio::test]
async fn a_panel_box_over_256_px_is_skipped_never_resized() {
    // The panel search tries sides of 0.145, 0.16 and 0.175 of the height,
    // then a fine pass 0.0075 either side of the best: 0.1375 to 0.1825.
    // 1440p: the largest coarse side (0.175, 252 px) fits.
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let qhd = DynamicImage::ImageRgb8(RgbImage::from_pixel(2560, 1440, Rgb([90, 60, 30])));
    let side = (0.175_f64 * 1440.0).round() as usize;
    let fits = panel((2000, 160, 2000 + side, 160 + side), "ana", 0.97);
    assert!(run(dir.path(), "tc-qhd", &qhd, source(Some(&fits), Some("ana"))).await);
    let queued = sync::queued_crops(dir.path(), UploadKind::HeroIcon);
    assert_eq!(queued.len(), 1);
    assert_eq!(
        sync::png_size(&queued[0].png),
        Some((side as u32, side as u32))
    );
    // 1440p after the fine pass: the largest side (0.1825, 263 px) is over
    // the cap and is skipped, not resized.
    let side = (0.1825_f64 * 1440.0).round() as usize;
    assert_eq!(side, 263);
    let over = panel((2000, 160, 2000 + side, 160 + side), "ana", 0.97);
    assert!(
        run(
            dir.path(),
            "tc-qhd-fine",
            &qhd,
            source(Some(&over), Some("ana"))
        )
        .await
    );
    assert_eq!(
        sync::queued_crops(dir.path(), UploadKind::HeroIcon).len(),
        1,
        "only the 252 px crop"
    );
    assert_eq!(sync::training_warning(dir.path()), None);

    // 4K: the smallest searched side (0.1375 of the height) is over the cap.
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let uhd = DynamicImage::ImageRgb8(RgbImage::from_pixel(3840, 2160, Rgb([90, 60, 30])));
    let side = (0.1375_f64 * 2160.0).round() as usize;
    assert!(side > 256);
    let over = panel((2700, 240, 2700 + side, 240 + side), "ana", 0.97);
    assert!(run(dir.path(), "tc-uhd", &uhd, source(Some(&over), Some("ana"))).await);
    assert!(sync::queued_crops(dir.path(), UploadKind::HeroIcon).is_empty());
    assert!(
        !sync::has_queued_crops(dir.path()),
        "no smaller copy either"
    );
    assert_eq!(
        sync::training_warning(dir.path()),
        None,
        "a cap skip is not an error"
    );
}

#[tokio::test]
async fn stat_digit_crops_are_the_own_rows_stat_cells_only() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    let frame = tab_frame();
    assert!(run(dir.path(), "tc-digits", &frame, source(None, None)).await);

    let queued = sync::queued_crops(dir.path(), UploadKind::StatDigits);
    assert_eq!(queued.len(), 6, "one crop per stat column");
    assert!(sync::queued_crops(dir.path(), UploadKind::HeroIcon).is_empty());

    let (bx, by, bw, bh) = ocr::preprocess::crop_scoreboard_rect(1920, 1080);
    let (_, row_y, _, row_h) = ocr::preprocess::player_row_rect(bw, bh, 1, 5).unwrap();
    let (row_top, row_bottom) = (by + row_y, by + row_y + row_h);
    // The labels sit where a real in-match board has them (E near 0.32 of
    // the board), so this is the name window that board's names are in.
    let (name_x, name_w) =
        ocr::preprocess::name_windows(5)[ocr::preprocess::NAME_WINDOW_5V5_INGAME];
    let name_end = bx + (f64::from(bw) * (name_x + name_w)).ceil() as u32;
    let mut cells = Vec::new();
    for crop in &queued {
        let img = decode(&crop.png);
        let (x0, y0) = coord_of(img.get_pixel(0, 0));
        // Raw pixels: each one is the frame's own, in place.
        for (x, y, p) in img.enumerate_pixels() {
            assert_eq!(coord_of(p), (x0 + x, y0 + y));
        }
        // Inside the own row, right of the name window, and a cell, not
        // the whole row.
        assert!(y0 >= row_top && y0 + img.height() <= row_bottom, "row {y0}");
        assert!(
            x0 >= name_end,
            "cell at x {x0} reaches the name column ({name_end})"
        );
        assert!(img.width() < bw / 8, "{} px wide", img.width());
        let meta = &crop.meta;
        assert_eq!(meta.hero.as_deref(), Some("ana"));
        assert_eq!(meta.screen.as_deref(), Some("tab"));
        assert_eq!(
            (meta.screen_width, meta.screen_height),
            (Some(1920), Some(1080))
        );
        assert!(sync::game_token_ok(meta.game_token.as_deref().unwrap()));
        cells.push((x0, img.width(), meta.slot_type.clone().unwrap()));
    }
    // Left to right, each crop carries its own column's name and sits
    // under its own label.
    cells.sort_by_key(|&(x0, _, _)| x0);
    let slots: Vec<&str> = cells.iter().map(|(_, _, slot)| slot.as_str()).collect();
    assert_eq!(slots, sync::STAT_DIGIT_COLUMNS);
    for ((x0, width, slot), label) in cells.iter().zip(REAL_LABELS) {
        let centre = f64::from(x0 * 2 + width) / 2.0;
        let want = f64::from(label_start(label)) + 4.0;
        assert!(
            (centre - want).abs() <= 2.0,
            "{slot} cell centred at {centre}, its label at {want}"
        );
    }

    // One set per game.
    assert!(!run(dir.path(), "tc-digits", &frame, source(None, None)).await);
}

#[tokio::test]
async fn ocr_v1_keeps_no_stat_digit_crops_from_its_row_stand_in() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    let frame = tab_frame();
    // ocr-v1: the row is a name match or the brightest row, and there is
    // no own row verdict, so a hover or a party tint on another row is not
    // told apart. The same Tab with a sure verdict would cut six crops.
    let mut v1 = source(None, None);
    v1.own_row = None;
    v1.hero = None;
    assert!(!run(dir.path(), "tc-v1", &frame, v1).await);
    assert!(!sync::has_queued_crops(dir.path()));
    assert_eq!(stat_digit_row(Some(1), None), None);
    assert_eq!(stat_digit_row(Some(1), Some(own(1, Trust::Sure))), Some(1));
}

#[tokio::test]
async fn no_stat_digit_crops_without_consent_a_sure_own_row_or_a_clear_row() {
    let dir = tempfile::tempdir().unwrap();
    let frame = tab_frame();
    // Consent off (hero icons on does not count).
    set_consent(dir.path(), true, false);
    assert!(!run(dir.path(), "tc-nodigits-0", &frame, source(None, None)).await);
    set_consent(dir.path(), false, true);
    // A confirmed but suspect own row, one that names another row than the
    // Tab's, or a verdict with no row: another player's cells could be cut.
    let mut suspect = source(None, None);
    suspect.own_row = Some(own(1, Trust::Suspect));
    assert!(!run(dir.path(), "tc-nodigits-1", &frame, suspect).await);
    let mut other = source(None, None);
    other.own_row = Some(own(2, Trust::Sure));
    assert!(!run(dir.path(), "tc-nodigits-2", &frame, other).await);
    let mut no_row = source(None, None);
    no_row.own_row = Some(OwnRow {
        row: None,
        ..own(1, Trust::Sure)
    });
    assert!(!run(dir.path(), "tc-nodigits-3", &frame, no_row).await);
    // An unconfirmed own row with nothing against it is not suspect.
    assert_eq!(
        stat_digit_row(Some(1), Some(own(1, Trust::Unconfirmed))),
        Some(1)
    );
    // Chat lines stacked up over the board: the row may hold a chat line.
    let mut chat = frame.to_rgb8();
    for y in 150..1010 {
        for x in 40..330 {
            let v = if (x / 2) % 2 == 0 { 255 } else { 0 };
            chat.put_pixel(x, y, Rgb([v, v, v]));
        }
    }
    let chat = DynamicImage::ImageRgb8(chat);
    run(dir.path(), "tc-nodigits-4", &chat, source(None, None)).await;
    // No header labels (no chat either): the columns are not guessed.
    let plain = DynamicImage::ImageRgb8(RgbImage::from_fn(1920, 1080, |x, y| {
        if x < 336 {
            Rgb([40, 40, 40])
        } else {
            coord_pixel(x, y)
        }
    }));
    run(dir.path(), "tc-nodigits-5", &plain, source(None, None)).await;
    assert!(!sync::has_queued_crops(dir.path()));
    assert_eq!(sync::training_warning(dir.path()), None);
}

#[tokio::test]
async fn a_stray_mark_left_of_e_costs_the_digit_set_never_a_shifted_label() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    // One more narrow mark in the stat area, left of E: the reader's first
    // six groups would then be the mark and E to H, every window one column
    // off. Seven groups are not the six labels, so nothing is cut.
    let mut labels = vec![0.28];
    labels.extend(REAL_LABELS);
    let frame = tab_frame_with(&labels);
    run(dir.path(), "tc-stray", &frame, source(None, None)).await;
    assert!(!sync::has_queued_crops(dir.path()));
    // Six marks with the wide gaps first are not the E/A/D then DMG/H/MIT
    // pattern, though every gap is one the reader takes.
    let reversed = [0.32, 0.38, 0.44, 0.48, 0.52, 0.56];
    run(
        dir.path(),
        "tc-reversed",
        &tab_frame_with(&reversed),
        source(None, None),
    )
    .await;
    assert!(!sync::has_queued_crops(dir.path()));
}

#[test]
fn training_columns_need_exactly_the_six_labels_in_their_gap_pattern() {
    // A real 5v5 header (1664 px): E A D DMG H MIT, then a wide junk group
    // (mic icon and hero art), which is not a label and does not count.
    let w = 1664;
    let real = [
        (530, 537),
        (586, 595),
        (649, 653),
        (720, 758),
        (846, 849),
        (935, 949),
        (1049, 1669),
    ];
    assert_eq!(
        training_stat_columns(&real, w),
        ocr::preprocess::columns_from_header_groups(&real, w)
    );
    assert!(training_stat_columns(&real, w).is_some());
    // A stray mark left of E: the reader still takes six windows, all one
    // column off. Training takes none.
    let mut stray = vec![(468, 474)];
    stray.extend_from_slice(&real);
    assert!(ocr::preprocess::columns_from_header_groups(&stray, w).is_some());
    assert_eq!(training_stat_columns(&stray, w), None);
    // A label missing and a stray mark in its place: six groups, but the
    // gaps are not narrow, narrow, then wide.
    let swapped = [
        (530, 537),
        (586, 595),
        (680, 686),
        (720, 758),
        (846, 849),
        (935, 949),
    ];
    assert_eq!(training_stat_columns(&swapped, w), None);
    // Five labels.
    assert_eq!(training_stat_columns(&real[..5], w), None);
    assert_eq!(training_stat_columns(&real, 0), None);
}

// ---- through `handle_capture`

/// One Tab of `analysis` and `board` on the open game `session_id`.
/// Returns whether it was recorded, and the rows stored for the game.
async fn record_tab(
    ctx: &DaemonCtx,
    session_id: &str,
    analysis: FrameAnalysis,
    board: shadow::BoardRead,
) -> (bool, usize) {
    record_tab_holding(ctx, session_id, analysis, board, None).await
}

/// [`record_tab`] on a game that already holds `held` as its accepted
/// hero (the way a split carries the last game's hero over).
async fn record_tab_holding(
    ctx: &DaemonCtx,
    session_id: &str,
    analysis: FrameAnalysis,
    board: shadow::BoardRead,
    held: Option<&str>,
) -> (bool, usize) {
    let now = test_now();
    let mut active = game(detect::MatchOutcome::Unknown, None, now);
    active.hero_auth.accepted_hero = held.map(str::to_string);
    active.session_id = session_id.into();
    active.map = Some("Busan".into());
    active.map_source = Some(boundary::MapSource::TopBar);
    active.session_created = false;
    let mut st = session(Some(active), now);
    *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture {
        analysis,
        board: Some(board),
    });
    let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
    let report = handle_capture(ctx, req).await.unwrap();
    let recorded = report.recorded;
    apply_capture_report(
        &mut st,
        &ctx.store,
        &ctx.data_dir,
        session_id,
        Ok(report),
        now,
    )
    .await;
    let rows = ctx.store.get_session_snapshots(session_id).await.unwrap();
    (recorded, rows.len())
}

/// A new reader Tab of [`tab_frame`]: its own row verdict is sure of row 0
/// (the Tab's row), and its hero authority is `pack`.
fn sure_tab(pack: Option<&str>) -> FrameAnalysis {
    let mut analysis = o8_analysis(None, pack);
    analysis.frame = tab_frame();
    analysis.own_row = Some(own(0, Trust::Sure));
    analysis
}

/// [`sure_tab`] with a career panel read for Ana at `box_px`.
fn panel_tab(box_px: (usize, usize, usize, usize)) -> FrameAnalysis {
    let mut analysis = sure_tab(Some("ana"));
    analysis.career_portrait = Some(panel(box_px, "ana", 0.97));
    analysis
}

fn sure_board() -> shadow::BoardRead {
    o10_board([sure(12), sure(5), sure(3), sure(3500), sure(700), sure(300)])
}

/// Wait for the blocking task a Tab started: until `done`, at most 5 s.
async fn wait_for(done: impl Fn() -> bool) -> bool {
    for _ in 0..200 {
        if done() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    done()
}

#[tokio::test]
async fn a_rejected_tab_gives_no_hero_icon_or_stat_digit_crop() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    set_consent(dir.path(), true, true);
    let refused = shadow::reader::not_found(shadow::BoardStatus::NotFound);
    let (recorded, rows) = record_tab(
        &ctx,
        "tc-rejected",
        panel_tab((1600, 200, 1800, 400)),
        refused,
    )
    .await;
    assert!(!recorded);
    assert_eq!(rows, 0);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!sync::has_queued_crops(dir.path()));
}

#[tokio::test]
async fn an_over_cap_panel_box_still_records_the_tab_and_its_digits() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    set_consent(dir.path(), true, true);
    // 300 px: over the 256 px cap.
    let (recorded, rows) = record_tab(
        &ctx,
        "tc-overcap",
        panel_tab((1500, 100, 1800, 400)),
        sure_board(),
    )
    .await;
    assert!(recorded, "the Tab records whatever happens to its crops");
    assert_eq!(rows, 1);
    // The hero icon is handled before the digits on the same thread.
    let digits = || sync::queued_crops(dir.path(), UploadKind::StatDigits).len() == 6;
    assert!(wait_for(digits).await);
    assert!(sync::queued_crops(dir.path(), UploadKind::HeroIcon).is_empty());
    assert_eq!(sync::training_warning(dir.path()), None);
    // This Tab's hero authority named the hero, so the digits carry it.
    for crop in sync::queued_crops(dir.path(), UploadKind::StatDigits) {
        assert_eq!(crop.meta.hero.as_deref(), Some("ana"));
    }
}

#[tokio::test]
async fn a_held_hero_is_never_written_into_stat_digit_meta() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    set_consent(dir.path(), false, true);
    // The game holds Kiriko (a split can carry the last game's hero into
    // the new game's first Tab), and this Tab's hero authority abstains.
    let (recorded, rows) = record_tab_holding(
        &ctx,
        "tc-held",
        sure_tab(None),
        sure_board(),
        Some("Kiriko"),
    )
    .await;
    assert!(recorded);
    assert_eq!(rows, 1);
    let stored = ctx.store.get_session_snapshots("tc-held").await.unwrap();
    assert_eq!(stored[0].hero, "Kiriko", "the held hero is stored");
    let digits = || sync::queued_crops(dir.path(), UploadKind::StatDigits).len() == 6;
    assert!(wait_for(digits).await);
    for crop in sync::queued_crops(dir.path(), UploadKind::StatDigits) {
        assert_eq!(crop.meta.hero, None, "not this Tab's read");
    }
}

#[tokio::test]
async fn an_accepted_ocr_v1_tab_keeps_no_stat_digit_crops() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), false).await;
    set_consent(dir.path(), true, true);
    // ocr-v1: a row stand-in (name match or brightest row), no own row
    // verdict. The new reader's sure row on this same frame keeps six.
    let mut analysis = o8_analysis(Some("Ana"), None);
    analysis.frame = tab_frame();
    assert_eq!(analysis.player_row_idx, Some(0));
    assert!(analysis.own_row.is_none());
    let (recorded, rows) = record_tab(&ctx, "tc-v1-tab", analysis, sure_board()).await;
    assert!(recorded, "ocr-v1 records the Tab");
    assert_eq!(rows, 1);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(sync::queued_crops(dir.path(), UploadKind::StatDigits).is_empty());
    assert!(!sync::has_queued_crops(dir.path()));
}

#[tokio::test]
async fn a_crop_that_cannot_be_written_warns_and_the_tab_still_records() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    set_consent(dir.path(), true, true);
    // The hero icon outbox cannot be made: a file sits where it goes.
    let outbox = dir.path().join("training_crops").join("outbox");
    std::fs::create_dir_all(&outbox).unwrap();
    std::fs::write(outbox.join("hero_icon"), b"not a folder").unwrap();
    let (recorded, rows) = record_tab(
        &ctx,
        "tc-writefail",
        panel_tab((1600, 200, 1800, 400)),
        sure_board(),
    )
    .await;
    assert!(recorded);
    assert_eq!(rows, 1);
    let digits = || sync::queued_crops(dir.path(), UploadKind::StatDigits).len() == 6;
    assert!(wait_for(digits).await, "the other kind is still kept");
    let warning = sync::training_warning(dir.path()).expect("the member is told");
    assert!(warning.contains("hero icons"), "{warning}");
    assert!(sync::queued_crops(dir.path(), UploadKind::HeroIcon).is_empty());
}
