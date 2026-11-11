//! Hero icon and stat digit training crops from a Tab: cut only from a
//! stored Tab of a known game, only with consent, raw and never resized,
//! and never in the way of recording.

use std::path::Path;
use std::time::Duration;

use crate::{main_tests::*, *};
use image::{DynamicImage, Rgb, RgbImage};
use scuffed_types::api::training_crops::TrainingCropConsent;
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

/// A 1920x1080 Tab: every pixel right of the chat strip names its own
/// position, and the scoreboard's header strip is a bright bar with the six
/// stat labels as dark marks, so the header columns are found.
fn tab_frame() -> DynamicImage {
    let (w, h) = (1920, 1080);
    let (bx, by, bw, bh) = ocr::preprocess::crop_scoreboard_rect(w, h);
    let header_end = by + (f64::from(bh) * 0.024) as u32;
    let (label_y0, label_y1) = (
        by + (f64::from(bh) * 0.008) as u32,
        by + (f64::from(bh) * 0.022) as u32,
    );
    let origin = bx + (f64::from(bw) * 0.5) as u32;
    let pitch = (f64::from(bw) * 0.05) as u32;
    let is_label = |x: u32, y: u32| {
        (label_y0..label_y1).contains(&y)
            && (0..6).any(|i| (origin + i * pitch..origin + i * pitch + 8).contains(&x))
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

fn source<'a>(panel: Option<&'a PanelRead>, pack_hero: Option<&'a str>) -> TabCropSource<'a> {
    TabCropSource {
        panel,
        pack_hero,
        own_row: Some(1),
        own_row_suspect: false,
        team_size: 5,
        hero: "Ana",
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
    // 1440p: the largest searched side (0.175 of the height) still fits.
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

    // 4K: the smallest searched side (0.145 of the height) is over the cap.
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let uhd = DynamicImage::ImageRgb8(RgbImage::from_pixel(3840, 2160, Rgb([90, 60, 30])));
    let side = (0.145_f64 * 2160.0).round() as usize;
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
    let name_end = ocr::preprocess::name_windows(5)
        .iter()
        .map(|(x, w)| bx + (f64::from(bw) * (x + w)).ceil() as u32)
        .max()
        .unwrap();
    let mut slots = Vec::new();
    let mut lefts = Vec::new();
    for crop in &queued {
        let img = decode(&crop.png);
        let (x0, y0) = coord_of(img.get_pixel(0, 0));
        // Raw pixels: each one is the frame's own, in place.
        for (x, y, p) in img.enumerate_pixels() {
            assert_eq!(coord_of(p), (x0 + x, y0 + y));
        }
        // Inside the own row, right of every name window, and a cell, not
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
        slots.push(meta.slot_type.clone().unwrap());
        lefts.push(x0);
    }
    slots.sort();
    let mut want: Vec<String> = sync::STAT_DIGIT_COLUMNS
        .iter()
        .map(|s| s.to_string())
        .collect();
    want.sort();
    assert_eq!(slots, want);
    lefts.sort();
    lefts.dedup();
    assert_eq!(lefts.len(), 6, "six different columns");

    // One set per game.
    assert!(!run(dir.path(), "tc-digits", &frame, source(None, None)).await);
}

#[tokio::test]
async fn no_stat_digit_crops_without_consent_a_sure_own_row_or_a_clear_row() {
    let dir = tempfile::tempdir().unwrap();
    let frame = tab_frame();
    // Consent off (hero icons on does not count).
    set_consent(dir.path(), true, false);
    assert!(!run(dir.path(), "tc-nodigits-0", &frame, source(None, None)).await);
    set_consent(dir.path(), false, true);
    // No own row, or a suspect one: another player's cells could be cut.
    let mut none = source(None, None);
    none.own_row = None;
    assert!(!run(dir.path(), "tc-nodigits-1", &frame, none).await);
    let mut suspect = source(None, None);
    suspect.own_row_suspect = true;
    assert!(!run(dir.path(), "tc-nodigits-2", &frame, suspect).await);
    // Chat lines stacked up over the board: the row may hold a chat line.
    let mut chat = frame.to_rgb8();
    for y in 150..1010 {
        for x in 40..330 {
            let v = if (x / 2) % 2 == 0 { 255 } else { 0 };
            chat.put_pixel(x, y, Rgb([v, v, v]));
        }
    }
    let chat = DynamicImage::ImageRgb8(chat);
    assert!(!run(dir.path(), "tc-nodigits-3", &chat, source(None, None)).await);
    // No header labels (no chat either): the columns are not guessed.
    let plain = DynamicImage::ImageRgb8(RgbImage::from_fn(1920, 1080, |x, y| {
        if x < 336 {
            Rgb([40, 40, 40])
        } else {
            coord_pixel(x, y)
        }
    }));
    assert!(!run(dir.path(), "tc-nodigits-4", &plain, source(None, None)).await);
    assert!(!sync::has_queued_crops(dir.path()));
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
    let now = test_now();
    let mut active = game(detect::MatchOutcome::Unknown, None, now);
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

/// A Tab of [`tab_frame`] with a career panel read for Ana at `box_px`.
fn panel_tab(box_px: (usize, usize, usize, usize)) -> FrameAnalysis {
    let mut analysis = o8_analysis(None, Some("ana"));
    analysis.frame = tab_frame();
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
