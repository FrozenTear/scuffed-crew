//! `rank` training crops: nothing with consent off, one set of native rank
//! reader regions per game with it on (claimed before the task, released on
//! an error before anything was kept), only from frames the kit took as a
//! rank screen, never chat or overlay text, no mode banner, no empty role
//! line, and a region over the cap or the edge bound never kept.

use std::path::Path;
use std::time::{Duration, Instant};

use image::{DynamicImage, Rgb, RgbImage};
use scuffed_types::api::training_crops::{CropSpot, TrainingCropConsent};
use stat_tracker::report::kit::RankScreenPngs;
use stat_tracker::sync::{RANK_SLOTS, RANK_SPOT_VERSION, UploadKind};

use super::rank_training::*;
use crate::ActiveGame;

fn set_consent(dir: &Path, rank: bool) {
    let consent = TrainingCropConsent {
        hero_icon: true,
        stat_digits: true,
        map_name: true,
        rank,
    };
    let path = dir.join("training_crops").join("consent.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_vec(&consent).unwrap()).unwrap();
    save_label_pack(dir);
}

/// A rank-v1 pack in `dir` holding only the "RANK PROGRESS:" label
/// template, learned from [`rank_frame`]'s stand-in label: what the rank
/// reader's rank screen test needs (the real pack comes from the site).
fn save_label_pack(dir: &Path) {
    use stat_tracker::shadow::rank::templates::ClassTemplate;
    use stat_tracker::shadow::rank::{
        Family, GATE_CLASS, RECOGNIZER_ID, TEMPLATE_DIR, TemplateSet, ink, layout,
    };
    let frame = rank_frame(2560, 1440).to_rgb8();
    let gate = layout::gate_word(&frame).expect("the stand-in label");
    let (w, h, _) = Family::Gate.shape();
    let mut set = TemplateSet {
        recognizer: RECOGNIZER_ID.to_string(),
        ..TemplateSet::default()
    };
    set.families.insert(
        Family::Gate,
        vec![ClassTemplate {
            class: GATE_CLASS.to_string(),
            aspect: ink::aspect(&gate.ink),
            n: 1,
            pooled: false,
            vector: ink::unit(ink::vector(&gate.ink, w, h)),
        }],
    );
    set.save_dir(&dir.join(TEMPLATE_DIR)).unwrap();
}

fn game(session: &str) -> ActiveGame {
    ActiveGame::open_at(
        session.to_string(),
        stat_tracker::detect::MatchOutcome::Unknown,
        Vec::new(),
        Instant::now()
            .checked_sub(Duration::from_secs(5))
            .expect("a past instant"),
    )
}

const BG: Rgb<u8> = Rgb([40, 44, 60]);
const TEXT: Rgb<u8> = Rgb([235, 235, 235]);

fn fill(img: &mut RgbImage, [x0, y0, x1, y1]: [u32; 4], col: Rgb<u8>) {
    for y in y0..y1 {
        for x in x0..x1 {
            img.put_pixel(x, y, col);
        }
    }
}

/// Blocks `w` wide, `gap` apart, ending at `right`, rows `y0..y1`.
fn blocks(img: &mut RgbImage, right: u32, y0: u32, y1: u32, (n, w, gap): (u32, u32, u32)) {
    for k in 0..n {
        let x1 = right - k * (w + gap);
        fill(img, [x1 - w, y0, x1, y1], TEXT);
    }
}

/// The first hero entry at 2560x1440 (scaled by `s`): a name line 14 rows
/// tall over a 21 row SR line, 10 rows apart, both ending at x 2333 like
/// the reader's synthetic screens.
const NAME_ROWS: (u32, u32) = (700, 714);
const SR_ROWS: (u32, u32) = (724, 745);

/// A rank screen stand-in at `w` x `h`: flat background, a few text blocks
/// in the role, tier and bar boxes and the mode banner, and (at 1440p) the
/// first hero entry.
fn rank_frame(w: u32, h: u32) -> DynamicImage {
    let mut img = RgbImage::from_pixel(w, h, BG);
    let s = |v: u32| v * w / 2560;
    fill(&mut img, [s(1190), s(290), s(1300), s(310)], TEXT);
    fill(&mut img, [s(1160), s(900), s(1380), s(942)], TEXT);
    // "RANK PROGRESS:" stand-in: letter strokes with a bar under them.
    for k in 0..8 {
        fill(
            &mut img,
            [s(842 + 16 * k), s(1068), s(850 + 16 * k), s(1086)],
            TEXT,
        );
    }
    fill(&mut img, [s(842), s(1086), s(970), s(1090)], TEXT);
    // The mode banner word: on screen, never cut.
    fill(&mut img, [s(2068), s(150), s(2300), s(171)], TEXT);
    if (w, h) == (2560, 1440) {
        blocks(&mut img, 2333, NAME_ROWS.0, NAME_ROWS.1, (5, 12, 4));
        blocks(&mut img, 2333, SR_ROWS.0, SR_ROWS.1, (4, 14, 4));
    }
    DynamicImage::ImageRgb8(img)
}

/// `rank_frame` with chat text in the chat box's bottom line (x 60 on,
/// y 1258 to 1308 at 1440p): thin bright strokes.
fn with_chat(frame: &DynamicImage) -> DynamicImage {
    let mut img = frame.to_rgb8();
    for x in (70..400).step_by(6) {
        fill(&mut img, [x, 1266, x + 2, 1290], TEXT);
    }
    DynamicImage::ImageRgb8(img)
}

fn noise(w: u32, h: u32) -> RgbImage {
    let mut seed: u32 = 0x9e37_79b9;
    RgbImage::from_fn(w, h, |_, _| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let [a, b, c, _] = seed.to_le_bytes();
        Rgb([a, b, c])
    })
}

/// `png` with zero bytes appended up to `len` (the PNG decoder stops at
/// IEND, so it still reads), as `sync/tests_training.rs` pads one.
fn pad_png(mut png: Vec<u8>, len: usize) -> Vec<u8> {
    assert!(png.len() <= len, "base PNG is already {} bytes", png.len());
    png.resize(len, 0);
    png
}

fn queued(dir: &Path) -> Vec<stat_tracker::sync::QueuedCrop> {
    crate::sync::queued_crops(dir, UploadKind::Rank)
}

fn slots(crops: &[stat_tracker::sync::QueuedCrop]) -> Vec<String> {
    let mut v: Vec<String> = crops
        .iter()
        .map(|c| c.meta.slot_type.clone().unwrap_or_default())
        .collect();
    v.sort();
    v
}

fn sorted(names: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

fn spot(x: u32, y: u32, w: u32, h: u32) -> CropSpot {
    CropSpot { x, y, w, h }
}

fn pngs() -> RankScreenPngs {
    RankScreenPngs {
        outcome: Vec::new(),
        rank: None,
        rank_banner: None,
    }
}

#[tokio::test]
async fn rank_consent_off_cuts_and_keeps_nothing() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false);
    let g = game("rank-off");
    let target = rank_crop_target(dir.path(), Some(&g)).expect("an open game");
    assert!(
        offer_rank_crop(target, &rank_frame(2560, 1440)).is_none(),
        "consent off: nothing is cut"
    );
    // Even crops already cut are not kept with consent off.
    keep_rank_crop(dir.path(), "rank-off", &rank_frame(2560, 1440));
    assert!(queued(dir.path()).is_empty());
    assert!(
        !dir.path()
            .join("training_crops")
            .join("outbox")
            .join("rank")
            .exists()
    );
    // And no consent at all (never read from the site) is off too.
    let fresh = tempfile::tempdir().unwrap();
    let target = rank_crop_target(fresh.path(), Some(&g)).unwrap();
    assert!(offer_rank_crop(target, &rank_frame(2560, 1440)).is_none());
    assert!(queued(fresh.path()).is_empty());
}

#[tokio::test]
async fn rank_consent_on_keeps_one_set_of_native_regions_per_game() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    let g = game("rank-once");
    let target = rank_crop_target(dir.path(), Some(&g)).expect("not offered yet");
    offer_rank_crop(target, &rank_frame(2560, 1440))
        .expect("consent on: cut")
        .await
        .unwrap();
    let kept = queued(dir.path());
    assert_eq!(slots(&kept), sorted(&RANK_SLOTS), "one crop per region");
    let want = [
        ("rank_role", spot(1060, 236, 440, 90)),
        ("rank_tier", spot(1070, 340, 420, 680)),
        ("rank_bar", spot(540, 1048, 1500, 184)),
    ];
    assert!(!slots(&kept).contains(&"rank_banner".to_string()));
    assert_eq!(RANK_SPOT_VERSION, "rank-regions.2");
    for crop in &kept {
        assert_eq!(crate::sync::check_crop(UploadKind::Rank, crop), Ok(()));
        let meta = &crop.meta;
        assert_eq!(meta.screen.as_deref(), Some("rank_update"));
        assert_eq!(meta.spot_version.as_deref(), Some(RANK_SPOT_VERSION));
        assert_eq!(meta.found_confidence, Some(1.0));
        assert_eq!(
            (meta.screen_width, meta.screen_height),
            (Some(2560), Some(1440))
        );
        assert!(meta.hero.is_none());
        assert!(crate::sync::game_token_ok(
            meta.game_token.as_deref().unwrap()
        ));
        // Native scale: the PNG is exactly the region's rectangle.
        let at = meta.found_spot.expect("the region rectangle");
        assert_eq!(crate::sync::png_size(&crop.png), Some((at.w, at.h)));
        let slot = meta.slot_type.as_deref().unwrap();
        if let Some((_, b)) = want.iter().find(|(s, _)| *s == slot) {
            assert_eq!(at, *b, "{slot}");
        }
    }
    // The game is offered: the next rank screen frame of it is not cut.
    assert!(rank_crop_target(dir.path(), Some(&g)).is_none());
    // Another game is.
    let other = game("rank-once-2");
    assert!(rank_crop_target(dir.path(), Some(&other)).is_some());
    // No open game, or one quiet past the rank idle limit: nothing.
    assert!(rank_crop_target(dir.path(), None).is_none());
    let quiet = ActiveGame::open_at(
        "rank-quiet".to_string(),
        stat_tracker::detect::MatchOutcome::Unknown,
        Vec::new(),
        Instant::now()
            .checked_sub(crate::daemon::RANK_OUTCOME_MAX_IDLE + Duration::from_secs(60))
            .unwrap(),
    );
    assert!(rank_crop_target(dir.path(), Some(&quiet)).is_none());
}

#[tokio::test]
async fn two_offers_before_the_first_task_completes_keep_one_set() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    let g = game("rank-race");
    // Two poll ticks took their target before either offered (the first
    // tick's task is still running when the second tick starts).
    let first = rank_crop_target(dir.path(), Some(&g)).unwrap();
    let second = rank_crop_target(dir.path(), Some(&g)).unwrap();
    let frame = rank_frame(2560, 1440);
    let task = offer_rank_crop(first, &frame).expect("the first offer cuts");
    assert!(
        offer_rank_crop(second, &frame).is_none(),
        "the game is claimed before the first task completes"
    );
    // A third tick while the task runs gets no target at all.
    assert!(rank_crop_target(dir.path(), Some(&g)).is_none());
    assert!(
        offer_for_tick(
            Some(&pngs()),
            rank_crop_target(dir.path(), Some(&g)),
            &frame
        )
        .is_none()
    );
    task.await.unwrap();
    let kept = queued(dir.path());
    assert_eq!(slots(&kept), sorted(&RANK_SLOTS), "exactly one set");
}

#[tokio::test]
async fn only_a_frame_the_kit_took_as_a_rank_screen_is_offered() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    let g = game("rank-gate");
    let frame = rank_frame(2560, 1440);
    // Any polled frame the report kit did not take as a rank screen (a
    // scoreboard, a Tab, gameplay): no rank screen PNGs, nothing cut, and
    // the game is not claimed.
    for source in [
        None,
        Some(stat_tracker::detect::match_end::OutcomeSource::ResultWord),
        Some(stat_tracker::detect::match_end::OutcomeSource::EndTitle),
    ] {
        let kit = stat_tracker::report::kit::rank_screen_pngs(source, &frame);
        assert!(kit.is_none(), "{source:?}");
        let target = rank_crop_target(dir.path(), Some(&g));
        assert!(offer_for_tick(kit.as_ref(), target, &frame).is_none());
        assert!(rank_crop_target(dir.path(), Some(&g)).is_some());
    }
    assert!(queued(dir.path()).is_empty());
    // A rank screen tick with no game to file it under cuts nothing.
    assert!(offer_for_tick(Some(&pngs()), None, &frame).is_none());
    // A rank screen tick for the game is cut.
    let target = rank_crop_target(dir.path(), Some(&g));
    offer_for_tick(Some(&pngs()), target, &frame)
        .expect("a rank screen frame is offered")
        .await
        .unwrap();
    assert_eq!(slots(&queued(dir.path())), sorted(&RANK_SLOTS));
}

#[tokio::test]
async fn rank_screen_under_the_minimum_size_is_not_cut() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    let g = game("rank-small");
    let target = rank_crop_target(dir.path(), Some(&g)).unwrap();
    assert!(offer_rank_crop(target, &rank_frame(1600, 900)).is_none());
    assert!(queued(dir.path()).is_empty());
    // Once per game: the skip marks the game, so the next rank tick of the
    // same game asks nothing (and logs nothing) again.
    assert!(rank_crop_target(dir.path(), Some(&g)).is_none());
}

#[test]
fn a_lost_rank_crop_warning_names_the_rank_screen_once() {
    let dir = tempfile::tempdir().unwrap();
    crate::sync::warn_crop_lost(dir.path(), UploadKind::Rank);
    let warning = crate::sync::training_warning(dir.path()).unwrap();
    assert_eq!(
        warning,
        "Training crops: a rank screen could not be kept on this PC. Your games still record."
    );
}

#[test]
fn region_sizes_at_1080p_1440p_and_4k() {
    let size = |r: RankRegion, w: u32, h: u32| {
        let s = frame_spot(r.reference_box().unwrap(), w, h).unwrap();
        (s.w, s.h)
    };
    use RankRegion::*;
    let table = [
        (Role, (330, 68), (440, 90), (660, 135)),
        (Tier, (315, 510), (420, 680), (630, 1020)),
        (Bar, (1125, 138), (1500, 184), (2250, 276)),
    ];
    for (r, p1080, p1440, p2160) in table {
        assert_eq!(size(r, 1920, 1080), p1080, "{r:?} 1080p");
        assert_eq!(size(r, 2560, 1440), p1440, "{r:?} 1440p");
        assert_eq!(size(r, 3840, 2160), p2160, "{r:?} 4K");
    }
    // A letterboxed 16:10 frame places the boxes on its 16:9 playfield.
    let b = frame_spot(Role.reference_box().unwrap(), 2560, 1600).unwrap();
    assert_eq!((b.x, b.y, b.w, b.h), (1060, 80 + 236, 440, 90));
    assert_eq!(Sr.reference_box(), None);
    let names: Vec<&str> = RankRegion::ALL.iter().map(|r| r.slot_type()).collect();
    assert_eq!(names, RANK_SLOTS);
}

#[test]
fn a_region_outside_the_edge_bound_is_left_out_never_scaled() {
    // At 4K the tier (630x1020) and the bar (2250x276) are outside 1536x720:
    // left out, while the role still goes (the role check reads no 4K frame,
    // so it never calls the line empty). The reader reads no 4K frame, so
    // there is no SR line either.
    let frame = rank_frame(3840, 2160);
    let rgb = frame.to_rgb8();
    assert_eq!(
        cut_region(RankRegion::Tier, &rgb),
        Err(RegionMiss::Edges(630, 1020))
    );
    assert_eq!(
        cut_region(RankRegion::Bar, &rgb),
        Err(RegionMiss::Edges(2250, 276))
    );
    assert_eq!(cut_region(RankRegion::Sr, &rgb), Err(RegionMiss::NotFound));
    let crops = rank_crops(&frame).unwrap();
    let got: Vec<&str> = crops.iter().map(|c| c.region.slot_type()).collect();
    assert_eq!(got, ["rank_role"]);
    for c in &crops {
        assert_eq!(
            crate::sync::png_size(&c.png),
            Some((c.spot.w, c.spot.h)),
            "native"
        );
    }
    // A region under the server's 8 px minimum is left out too.
    let small = RgbImage::from_pixel(64, 64, BG);
    assert_eq!(
        encode_region(&small, spot(0, 0, 7, 20)),
        Err(RegionMiss::Edges(7, 20))
    );
}

#[test]
fn the_sr_crop_is_the_readers_sr_line_without_the_hero_name() {
    let frame = rank_frame(2560, 1440);
    let rgb = frame.to_rgb8();
    let s = sr_spot(&rgb).expect("the reader finds the SR line");
    assert_eq!(cut_region(RankRegion::Sr, &rgb).map(|c| c.spot), Ok(s));
    // Two reference rows and columns around the SR digits only: the name
    // line (rows 700 to 714) is above it.
    assert_eq!(s, spot(2333 - 68 - 2, SR_ROWS.0 - 2, 68 + 4, 21 + 4));
    assert!(s.y >= NAME_ROWS.1, "{s:?}");
    // Inside the reader's hero box (x 2120 to 2342).
    assert!(s.x >= 2120 && s.x + s.w <= 2342, "{s:?}");
    // No hero entry (an SR line with no name line over it): no SR crop.
    let mut bare = RgbImage::from_pixel(2560, 1440, BG);
    blocks(&mut bare, 2333, SR_ROWS.0, SR_ROWS.1, (4, 14, 4));
    assert_eq!(sr_spot(&bare), Err(RegionMiss::NotFound));
    assert_eq!(cut_region(RankRegion::Sr, &bare), Err(RegionMiss::NotFound));
    // At 1080p the screen without a hero entry gives the other three.
    let crops = rank_crops(&rank_frame(1920, 1080)).unwrap();
    let got: Vec<&str> = crops.iter().map(|c| c.region.slot_type()).collect();
    assert_eq!(got, ["rank_role", "rank_tier", "rank_bar"]);
}

#[test]
fn an_sr_window_that_cuts_the_digits_short_gives_no_sr_crop() {
    // The name ends at x 2300 while the SR digits run on to x 2333: the
    // SR window (up to the name's end) would cut the number short, so no
    // SR crop at all rather than a part of one.
    let mut img = RgbImage::from_pixel(2560, 1440, BG);
    blocks(&mut img, 2300, NAME_ROWS.0, NAME_ROWS.1, (5, 12, 4));
    blocks(&mut img, 2333, SR_ROWS.0, SR_ROWS.1, (4, 14, 4));
    let e = stat_tracker::shadow::rank::layout::first_entry(&img).expect("the entry");
    assert!(stat_tracker::shadow::rank::layout::sr_cut_short(&img, &e));
    assert!(
        !stat_tracker::shadow::rank::layout::sr_glyphs(&img, &e).is_empty(),
        "the window alone would give a part of the number"
    );
    assert_eq!(sr_spot(&img), Err(RegionMiss::NotFound));
    assert_eq!(cut_region(RankRegion::Sr, &img), Err(RegionMiss::NotFound));
}

/// A stream chat line drawn across the SR row, its left end at `x`: a dark
/// backing strip, a coloured name, then smaller white message letters.
fn with_overlay_line(frame: &RgbImage, x: u32) -> RgbImage {
    let mut img = frame.clone();
    let (y0, y1) = (SR_ROWS.0 - 3, SR_ROWS.1 + 3);
    let x1 = (x + 420).min(2560);
    for yy in y0..y1 {
        for xx in x..x1 {
            let p = img.get_pixel(xx, yy).0;
            img.put_pixel(xx, yy, Rgb(p.map(|v| v / 3)));
        }
    }
    // The name: cyan strokes 14 rows tall.
    let mut cx = x + 6;
    while cx + 3 < x1.min(x + 90) {
        fill(&mut img, [cx, y0 + 6, cx + 3, y0 + 20], Rgb([90, 200, 255]));
        cx += 7;
    }
    // The message: white strokes 11 rows tall, word gaps every fifth.
    let mut k = 0;
    while cx + 2 < x1 {
        if k % 5 != 4 {
            fill(&mut img, [cx, y0 + 9, cx + 2, y0 + 20], TEXT);
        }
        cx += 6;
        k += 1;
    }
    img
}

#[test]
fn an_overlay_line_across_the_sr_row_keeps_no_sr_crop() {
    let clean = rank_frame(2560, 1440).to_rgb8();
    // The clean row keeps its SR crop.
    assert!(cut_region(RankRegion::Sr, &clean).is_ok());
    let names = |img: &RgbImage| -> Vec<&'static str> {
        rank_crops(&DynamicImage::ImageRgb8(img.clone()))
            .unwrap()
            .iter()
            .map(|c| c.region.slot_type())
            .collect()
    };
    assert_eq!(
        names(&clean),
        ["rank_role", "rank_tier", "rank_bar", "rank_sr"]
    );
    // Six positions, from wholly left of the digits to over all of them:
    // never an SR crop, and the overlay rule (not only a missed line) is
    // what refuses it where the line still reads.
    let mut refused = 0;
    for x in [1880, 1960, 2040, 2120, 2200, 2260] {
        let img = with_overlay_line(&clean, x);
        let got = cut_region(RankRegion::Sr, &img);
        assert!(got.is_err(), "x {x}: {:?}", got.map(|c| c.spot));
        if matches!(got, Err(RegionMiss::Overlay(_))) {
            refused += 1;
        }
        assert!(!names(&img).contains(&"rank_sr"), "x {x}");
    }
    assert!(refused >= 2, "{refused} of 6 refused by the overlay rule");
}

#[test]
fn sr_overlay_rules_refuse_colour_short_glyphs_and_stray_ink() {
    let clean = rank_frame(2560, 1440).to_rgb8();
    let at = sr_spot(&clean).unwrap();
    // A coloured chip inside the box, clear of the digits' rows.
    let mut chip = clean.clone();
    fill(
        &mut chip,
        [at.x + 1, at.y, at.x + 30, at.y + 2],
        Rgb([220, 60, 200]),
    );
    assert!(matches!(
        sr_spot(&chip),
        Err(RegionMiss::Overlay(SrOverlay::Colour(_)))
    ));
    // A glyph just right of the number, inside the SR window: the digits
    // no longer line up with the rightmost glyph, so the reader drops them
    // all (more than an icon).
    let mut right = clean.clone();
    fill(&mut right, [2334, SR_ROWS.0 + 9, 2336, SR_ROWS.1], TEXT);
    assert_eq!(sr_spot(&right), Err(RegionMiss::Overlay(SrOverlay::Shape)));
    // Glyphs that line up with each other but are shorter than the SR line
    // (lower case or smaller text; the line is as tall as a block far left
    // on the same rows).
    let mut short = RgbImage::from_pixel(2560, 1440, BG);
    blocks(&mut short, 2333, NAME_ROWS.0, NAME_ROWS.1, (5, 12, 4));
    fill(&mut short, [2130, SR_ROWS.0, 2140, SR_ROWS.1], TEXT);
    blocks(&mut short, 2333, SR_ROWS.0 + 5, SR_ROWS.1, (4, 14, 4));
    assert_eq!(sr_spot(&short), Err(RegionMiss::Overlay(SrOverlay::Short)));
    // One left of the number, clear of the box: the reader's own line
    // shape drops it, and the box around the digits is still clean.
    let mut left = clean.clone();
    fill(
        &mut left,
        [at.x - 14, SR_ROWS.0 + 9, at.x - 4, SR_ROWS.1],
        TEXT,
    );
    assert_eq!(sr_spot(&left), Ok(at));
    // A grey rank icon taller than the digits, in two parts: the line's
    // rows grow to the icon's, the reader drops both parts, and the digits
    // still make a clean SR crop.
    let mut icon = clean.clone();
    for x in [at.x - 30, at.x - 18] {
        fill(
            &mut icon,
            [x, SR_ROWS.0 - 4, x + 10, SR_ROWS.1 + 4],
            Rgb([200, 200, 205]),
        );
    }
    let got = sr_spot(&icon).expect("an icon is no overlay");
    assert_eq!((got.x, got.w), (at.x, at.w));
    // An overlay strip hiding the digits leaves the icon as the line's
    // only glyph: too tall for a digit, no SR crop of the icon alone.
    let mut hidden = icon.clone();
    fill(
        &mut hidden,
        [at.x - 4, SR_ROWS.0 - 6, 2342, SR_ROWS.1 + 6],
        Rgb([12, 12, 16]),
    );
    assert_eq!(sr_spot(&hidden), Err(RegionMiss::Overlay(SrOverlay::Tall)));
    // Ink between two digits outside their glyph boxes (a stroke the
    // reader's glyph cut did not take, here added after the cut): the
    // rule alone, on the line the reader found.
    use stat_tracker::shadow::rank::layout;
    let e = layout::first_entry(&clean).unwrap();
    let gl = layout::sr_glyphs(&clean, &e);
    let s = layout::Scale::from_img(&clean).unwrap();
    assert_eq!(sr_overlay(&clean, s, &e, &gl, at), None);
    let mut stray = clean.clone();
    let gap = gl[0].bbox[2] + 1;
    fill(
        &mut stray,
        [gap, SR_ROWS.0, gap + 2, SR_ROWS.1],
        Rgb([200, 200, 200]),
    );
    assert_eq!(
        sr_overlay(&stray, s, &e, &gl, at),
        Some(SrOverlay::Stray(2 * 21))
    );
    // The background a little lighter or darker, digits anti-aliased into
    // it: still the clean line.
    let mut shade = clean.clone();
    for y in at.y..at.y + at.h {
        for x in at.x..at.x + at.w {
            let p = shade.get_pixel(x, y).0;
            if p == BG.0 {
                let d = (x - at.x) as u8 / 8;
                shade.put_pixel(x, y, Rgb(p.map(|v| v + d)));
            } else if x % 3 == 0 {
                shade.put_pixel(x, y, Rgb([138, 140, 148]));
            }
        }
    }
    assert_eq!(sr_spot(&shade), Ok(at));
}

#[test]
fn an_empty_role_line_is_not_cut() {
    // The open queue screen: no role line above the emblem.
    let mut rgb = rank_frame(2560, 1440).to_rgb8();
    fill(&mut rgb, [1060, 236, 1500, 326], BG);
    use stat_tracker::shadow::rank::layout::{RoleLine, role_icon};
    assert_eq!(role_icon(&rgb), RoleLine::Empty);
    assert_eq!(cut_region(RankRegion::Role, &rgb), Err(RegionMiss::Blank));
    let got: Vec<&str> = rank_crops(&DynamicImage::ImageRgb8(rgb))
        .unwrap()
        .iter()
        .map(|c| c.region.slot_type())
        .collect();
    assert_eq!(got, ["rank_tier", "rank_bar", "rank_sr"]);
    // A role line that is there (or unclear) is cut.
    let rgb = rank_frame(2560, 1440).to_rgb8();
    assert_ne!(role_icon(&rgb), RoleLine::Empty);
    assert!(cut_region(RankRegion::Role, &rgb).is_ok());
}

#[test]
fn the_bar_is_left_out_while_chat_shows_beside_its_rows() {
    use stat_tracker::report::blank::chat_in_rows;
    let clean = rank_frame(2560, 1440);
    assert!(!chat_in_rows(&clean.to_rgb8(), 1048.0, 1232.0));
    // Chat only in the bottom line (y 1258 to 1308), below the bar's box
    // (rows 1048 to 1232): the bar is kept.
    let bottom = with_chat(&clean);
    assert!(chat_in_rows(&bottom.to_rgb8(), 1048.0, 1340.0));
    assert!(!chat_in_rows(&bottom.to_rgb8(), 1048.0, 1232.0));
    assert!(cut_region(RankRegion::Bar, &bottom.to_rgb8()).is_ok());
    let got: Vec<&str> = rank_crops(&bottom)
        .unwrap()
        .iter()
        .map(|c| c.region.slot_type())
        .collect();
    assert_eq!(got, ["rank_role", "rank_tier", "rank_bar", "rank_sr"]);
    // A chat line beside the bar's rows keeps the bar out, with the bottom
    // line empty or not.
    let mut high = clean.to_rgb8();
    for x in (70..400).step_by(6) {
        fill(&mut high, [x, 1110, x + 2, 1130], TEXT);
    }
    assert!(chat_in_rows(&high, 1048.0, 1232.0));
    assert_eq!(cut_region(RankRegion::Bar, &high), Err(RegionMiss::Chat));
    let got: Vec<&str> = rank_crops(&DynamicImage::ImageRgb8(high.clone()))
        .unwrap()
        .iter()
        .map(|c| c.region.slot_type())
        .collect();
    assert_eq!(got, ["rank_role", "rank_tier", "rank_sr"]);
    let mut both = bottom.to_rgb8();
    for x in (70..400).step_by(6) {
        fill(&mut both, [x, 1200, x + 2, 1220], TEXT);
    }
    assert_eq!(cut_region(RankRegion::Bar, &both), Err(RegionMiss::Chat));
    // No region reaches the chat strip (x 60 to 448).
    for r in RankRegion::ALL {
        if let Some(b) = r.reference_box() {
            assert!(b[0] >= 540, "{r:?}");
        }
    }
}

#[test]
fn a_region_over_the_byte_cap_is_left_out_the_rest_kept() {
    let cap = UploadKind::Rank.crop_kind().max_crop_bytes();
    // Noise does not compress: noise in the tier (420x680) and the bar
    // (1500x184) is over 384 KiB; the role and the SR line are not.
    let mut rgb = rank_frame(2560, 1440).to_rgb8();
    for [x0, y0, x1, y1] in [[1070, 340, 1490, 1020], [540, 1048, 2040, 1232]] {
        let n = noise(x1 - x0, y1 - y0);
        image::imageops::replace(&mut rgb, &n, i64::from(x0), i64::from(y0));
    }
    let Err(RegionMiss::OverCap(bytes)) = cut_region(RankRegion::Tier, &rgb) else {
        panic!("noise tier must be over the cap");
    };
    assert!(bytes > cap, "{bytes}");
    assert!(matches!(
        cut_region(RankRegion::Bar, &rgb),
        Err(RegionMiss::OverCap(_))
    ));
    let frame = DynamicImage::ImageRgb8(rgb);
    let got: Vec<&str> = rank_crops(&frame)
        .unwrap()
        .iter()
        .map(|c| c.region.slot_type())
        .collect();
    assert_eq!(got, ["rank_role", "rank_sr"]);
}

#[tokio::test]
async fn a_png_over_the_cap_that_reached_the_store_is_refused_there() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    let cap = UploadKind::Rank.crop_kind().max_crop_bytes();
    let frame = rank_frame(2560, 1440);
    let mut crops = rank_crops(&frame).unwrap();
    crops.truncate(1);
    // A real PNG padded past the cap: refused for its size, not as a
    // broken PNG.
    let over = pad_png(crops[0].png.clone(), cap + 1);
    let mut meta = crate::sync::rank_meta(2560, 1440, "rank_role", crops[0].spot);
    meta.game_token = Some(stat_tracker::sync::random_token().unwrap());
    let queued_crop = stat_tracker::sync::QueuedCrop {
        id: "0000000000001-00000000".into(),
        meta,
        png: over.clone(),
    };
    assert_eq!(
        crate::sync::check_crop(UploadKind::Rank, &queued_crop),
        Err(stat_tracker::sync::CropRefusal::TooLarge(cap + 1))
    );
    crops[0].png = over;
    keep_rank_result(dir.path(), "rank-cap", (2560, 1440), Ok(crops));
    assert!(queued(dir.path()).is_empty());
    assert!(crate::sync::training_warning(dir.path()).is_none());
}

#[tokio::test]
async fn an_encode_or_write_error_releases_the_game_for_a_later_frame() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    let g = game("rank-retry");
    // Claimed the way a tick claims it, then the encode fails.
    super::training_crops::mark_offered(super::training_crops::offer_key(
        "rank-retry",
        UploadKind::Rank,
        "",
    ));
    assert!(rank_crop_target(dir.path(), Some(&g)).is_none());
    keep_rank_result(dir.path(), "rank-retry", (2560, 1440), Err(EncodeFailed));
    assert!(crate::sync::training_warning(dir.path()).is_some());
    assert!(
        rank_crop_target(dir.path(), Some(&g)).is_some(),
        "an encode error releases the game"
    );

    // A write error before any region was kept releases it too: the kind's
    // folder is a file.
    let broken = tempfile::tempdir().unwrap();
    set_consent(broken.path(), true);
    let rank_dir = broken
        .path()
        .join("training_crops")
        .join("outbox")
        .join("rank");
    std::fs::create_dir_all(rank_dir.parent().unwrap()).unwrap();
    std::fs::write(&rank_dir, b"x").unwrap();
    let g2 = game("rank-write");
    let target = rank_crop_target(broken.path(), Some(&g2)).unwrap();
    let frame = rank_frame(2560, 1440);
    offer_rank_crop(target, &frame).unwrap().await.unwrap();
    assert!(crate::sync::training_warning(broken.path()).is_some());
    assert!(rank_crop_target(broken.path(), Some(&g2)).is_some());
    // Fixed, the next frame keeps one set.
    std::fs::remove_file(&rank_dir).unwrap();
    let target = rank_crop_target(broken.path(), Some(&g2)).unwrap();
    offer_rank_crop(target, &frame).unwrap().await.unwrap();
    assert_eq!(slots(&queued(broken.path())), sorted(&RANK_SLOTS));
    assert!(rank_crop_target(broken.path(), Some(&g2)).is_none());

    // No region fitting at all keeps the claim: a retry would spend the
    // encodes again for the same result.
    let g3 = game("rank-none-fit");
    let target = rank_crop_target(dir.path(), Some(&g3)).unwrap();
    super::training_crops::mark_offered(super::training_crops::offer_key(
        &target.session,
        UploadKind::Rank,
        "",
    ));
    keep_rank_result(dir.path(), "rank-none-fit", (2560, 1440), Ok(Vec::new()));
    assert!(rank_crop_target(dir.path(), Some(&g3)).is_none());
}

#[tokio::test]
async fn a_write_error_after_a_region_was_kept_keeps_the_claim() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    let crops = rank_crops(&rank_frame(2560, 1440)).unwrap();
    assert!(crops.len() >= 2);
    let claim = |session: &str| {
        super::training_crops::mark_offered(super::training_crops::offer_key(
            session,
            UploadKind::Rank,
            "",
        ));
    };
    // The first region is written, the second write fails: the game stays
    // claimed, so no later frame sends the first region again.
    let g = game("rank-half");
    claim("rank-half");
    let mut writes = Vec::new();
    keep_rank_result_with(
        dir.path(),
        "rank-half",
        (2560, 1440),
        Ok(crops.clone()),
        |meta, _| {
            writes.push(meta.slot_type.clone().unwrap_or_default());
            (writes.len() == 1).then_some(1)
        },
    );
    assert_eq!(writes.len(), crops.len(), "every region is tried once");
    assert!(
        rank_crop_target(dir.path(), Some(&g)).is_none(),
        "a region was kept: the claim stays"
    );
    // Every write fails: nothing was kept, the claim is released.
    let g2 = game("rank-none-written");
    claim("rank-none-written");
    keep_rank_result_with(
        dir.path(),
        "rank-none-written",
        (2560, 1440),
        Ok(crops),
        |_, _| None,
    );
    assert!(rank_crop_target(dir.path(), Some(&g2)).is_some());
}

#[test]
fn the_rank_screen_class_is_the_report_bundles() {
    assert_eq!(
        crate::sync::RANK_SCREEN,
        stat_tracker::report::bundle::RANK_SCREEN_CLASS
    );
}

/// Local bench on labelled rank screens (never in CI): set
/// `SCUFFED_RANK_BENCH` to the labels JSONL (`game`, `file`, `readable`)
/// and `SCUFFED_RANK_FRAMES` to the folder holding `<game>/<file>`. Prints
/// the regions each readable frame keeps and why an SR line was refused.
#[test]
#[ignore = "needs local labelled rank screens"]
fn bench_rank_training_regions() {
    let (Ok(jsonl), Ok(frames)) = (
        std::env::var("SCUFFED_RANK_BENCH"),
        std::env::var("SCUFFED_RANK_FRAMES"),
    ) else {
        return;
    };
    let mut kept = std::collections::BTreeMap::<String, usize>::new();
    let mut frames_read = 0;
    for line in std::fs::read_to_string(jsonl).unwrap().lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let (game, file) = (row["game"].as_str().unwrap(), row["file"].as_str().unwrap());
        let Ok(frame) = image::open(Path::new(&frames).join(game).join(file)) else {
            continue;
        };
        // `SCUFFED_RANK_DATA`: a data folder holding a rank-v1 pack, for
        // the rank label test on every labelled frame.
        if let Ok(data) = std::env::var("SCUFFED_RANK_DATA") {
            println!(
                "{game}/{file}: screen {} label {}",
                row["screen"],
                rank_label_shows(Path::new(&data), &frame)
            );
        }
        if row["readable"] != serde_json::Value::Bool(true) {
            continue;
        }
        frames_read += 1;
        let rgb = frame.to_rgb8();
        let crops = rank_crops(&frame).unwrap();
        let got: Vec<&str> = crops.iter().map(|c| c.region.slot_type()).collect();
        for g in &got {
            *kept.entry(g.to_string()).or_default() += 1;
        }
        let sr = sr_spot(&rgb);
        println!("{game}/{file}: {got:?} sr {sr:?}");
        if let (Err(RegionMiss::Overlay(_)), Some(e)) =
            (sr, stat_tracker::shadow::rank::layout::first_entry(&rgb))
        {
            let boxes = |gl: Vec<stat_tracker::shadow::rank::ink::Glyph>| -> Vec<[u32; 4]> {
                gl.iter().map(|g| g.bbox).collect()
            };
            println!(
                "  line {:?} glyphs {:?} cluster {:?}",
                e.sr,
                boxes(stat_tracker::shadow::rank::layout::sr_glyphs(&rgb, &e)),
                boxes(stat_tracker::shadow::rank::layout::sr_cluster(&rgb, &e)),
            );
        }
    }
    println!("{frames_read} frames, kept {kept:?}");
}

/// `frame` with the "RANK PROGRESS:" label box (x 840-975, y 1066-1092)
/// holding `label`: nothing (the placement screen), or bright art (a hero
/// highlight).
fn without_label(frame: &DynamicImage, art: bool) -> DynamicImage {
    let mut img = frame.to_rgb8();
    fill(&mut img, [830, 1060, 985, 1098], BG);
    if art {
        for y in 1062..1096 {
            for x in 832..983 {
                let v = 120 + ((x * 7 + y * 13) % 120) as u8;
                img.put_pixel(x, y, Rgb([v, v, v.saturating_sub(10)]));
            }
        }
    }
    DynamicImage::ImageRgb8(img)
}

#[tokio::test]
async fn a_frame_without_the_rank_label_offers_nothing_and_keeps_the_game_open() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    let g = game("rank-label");
    let labelled = rank_frame(2560, 1440);
    assert!(rank_label_shows(dir.path(), &labelled));
    // Hero highlight art and the placement screen: the kit took them as a
    // rank screen (`Some(pngs)`), but there is no rank progress label.
    for art in [false, true] {
        let frame = without_label(&labelled, art);
        assert!(!rank_label_shows(dir.path(), &frame), "art {art}");
        let target = rank_crop_target(dir.path(), Some(&g));
        assert!(offer_for_tick(Some(&pngs()), target, &frame).is_none());
        assert!(
            rank_crop_target(dir.path(), Some(&g)).is_some(),
            "art {art}: the game is not claimed"
        );
    }
    assert!(queued(dir.path()).is_empty());
    // A later rank update frame of the same game still gives its set.
    let target = rank_crop_target(dir.path(), Some(&g));
    offer_for_tick(Some(&pngs()), target, &labelled)
        .expect("the labelled frame is offered")
        .await
        .unwrap();
    assert_eq!(slots(&queued(dir.path())), sorted(&RANK_SLOTS));
    assert!(rank_crop_target(dir.path(), Some(&g)).is_none());
}

#[tokio::test]
async fn no_rank_pack_or_a_4k_frame_offers_nothing() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    let g = game("rank-no-pack");
    // No pack: the label cannot be checked, so nothing is offered.
    std::fs::remove_dir_all(dir.path().join(stat_tracker::shadow::rank::TEMPLATE_DIR)).unwrap();
    let frame = rank_frame(2560, 1440);
    assert!(!rank_label_shows(dir.path(), &frame));
    let target = rank_crop_target(dir.path(), Some(&g));
    assert!(offer_for_tick(Some(&pngs()), target, &frame).is_none());
    assert!(rank_crop_target(dir.path(), Some(&g)).is_some());
    // The reader takes no 4K frame: nothing offered there either.
    let four_k = tempfile::tempdir().unwrap();
    set_consent(four_k.path(), true);
    let target = rank_crop_target(four_k.path(), Some(&g));
    assert!(offer_for_tick(Some(&pngs()), target, &rank_frame(3840, 2160)).is_none());
    assert!(queued(dir.path()).is_empty() && queued(four_k.path()).is_empty());
}
