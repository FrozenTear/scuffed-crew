//! `rank` training crops: nothing with consent off, one set of native rank
//! reader regions per game with it on (claimed before the task, released on
//! an error before anything was kept), only from frames the kit took as a
//! rank screen and the reader reads, never chat or overlay text, no mode
//! banner, no SR line, a role line only with its role icon, and a region
//! over the cap or the edge bound never kept.

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
/// in the role (an icon, a gap, the role name), tier and bar boxes and the
/// mode banner, and (at 1440p) the first hero entry with its SR digits.
fn rank_frame(w: u32, h: u32) -> DynamicImage {
    let mut img = RgbImage::from_pixel(w, h, BG);
    let s = |v: u32| v * w / 2560;
    fill(&mut img, [s(1190), s(290), s(1210), s(310)], TEXT);
    fill(&mut img, [s(1230), s(290), s(1300), s(310)], TEXT);
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
    assert!(!slots(&kept).contains(&"rank_sr".to_string()));
    assert_eq!(RANK_SPOT_VERSION, "rank-regions.3");
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
            &frame,
            &frame.to_rgb8()
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
        assert!(offer_for_tick(kit.as_ref(), target, &frame, &frame.to_rgb8()).is_none());
        assert!(rank_crop_target(dir.path(), Some(&g)).is_some());
    }
    assert!(queued(dir.path()).is_empty());
    // A rank screen tick with no game to file it under cuts nothing.
    assert!(offer_for_tick(Some(&pngs()), None, &frame, &frame.to_rgb8()).is_none());
    // A rank screen tick for the game is cut.
    let target = rank_crop_target(dir.path(), Some(&g));
    offer_for_tick(Some(&pngs()), target, &frame, &frame.to_rgb8())
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

#[tokio::test]
async fn a_size_the_reader_does_not_read_claims_the_game_on_the_tick() {
    // Through the poll tick's gate (#752): the size is checked before the
    // label test, so the game is claimed on the first tick (one log line
    // per game) and nothing is cut. Under the minimum, 4K, ultrawide and
    // 16:10.
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    for (w, h) in [
        (1600, 900),
        (1855, 1056),
        (3840, 2160),
        (3440, 1440),
        (2560, 1600),
    ] {
        let g = game(&format!("rank-size-{w}x{h}"));
        // Only the size is checked, so a plain frame will do.
        let frame = DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, BG));
        let target = rank_crop_target(dir.path(), Some(&g));
        assert!(target.is_some(), "{w}x{h}");
        assert!(
            offer_for_tick(Some(&pngs()), target, &frame, &frame.to_rgb8()).is_none(),
            "{w}x{h}"
        );
        assert!(
            rank_crop_target(dir.path(), Some(&g)).is_none(),
            "{w}x{h}: claimed once, no later tick asks again"
        );
    }
    assert!(queued(dir.path()).is_empty());
    // A frame the kit did not take, or with consent off, claims nothing.
    let g = game("rank-size-gate");
    let frame = rank_frame(1600, 900);
    let target = rank_crop_target(dir.path(), Some(&g));
    assert!(offer_for_tick(None, target, &frame, &frame.to_rgb8()).is_none());
    assert!(rank_crop_target(dir.path(), Some(&g)).is_some());
    set_consent(dir.path(), false);
    let target = rank_crop_target(dir.path(), Some(&g));
    assert!(offer_for_tick(Some(&pngs()), target, &frame, &frame.to_rgb8()).is_none());
    assert!(rank_crop_target(dir.path(), Some(&g)).is_some());
    // A size the reader reads goes on to the label test and is cut.
    set_consent(dir.path(), true);
    let frame = rank_frame(2560, 1440);
    let target = rank_crop_target(dir.path(), Some(&g));
    offer_for_tick(Some(&pngs()), target, &frame, &frame.to_rgb8())
        .expect("1440p is offered")
        .await
        .unwrap();
    assert_eq!(slots(&queued(dir.path())), sorted(&RANK_SLOTS));
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
        let s = frame_spot(r.reference_box(), w, h).unwrap();
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
    let b = frame_spot(Role.reference_box(), 2560, 1600).unwrap();
    assert_eq!((b.x, b.y, b.w, b.h), (1060, 80 + 236, 440, 90));
    let names: Vec<&str> = RankRegion::ALL.iter().map(|r| r.slot_type()).collect();
    assert_eq!(names, RANK_SLOTS);
}

#[test]
fn a_region_outside_the_edge_bound_is_left_out_never_scaled() {
    // At 4K the tier (630x1020) and the bar (2250x276) are outside 1536x720:
    // left out. The role fits, but the reader reads no 4K frame, so it finds
    // no role icon and the role is left out too (no 4K frame is offered).
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
    assert_eq!(cut_region(RankRegion::Role, &rgb), Err(RegionMiss::Blank));
    let crops = rank_crops(&frame).unwrap();
    assert!(crops.is_empty());
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

/// `frame` with a stream chat line's message drawn over the first hero's
/// SR digits: lower case letter strokes as bright as the digits, opaque
/// (`blend` 1) or half blended into what is under them (`blend` 2), rows
/// the digits share, so they join the digits' glyphs.
fn with_chat_over_the_sr(frame: &RgbImage, blend: u16) -> RgbImage {
    let mut img = frame.clone();
    let (y0, y1) = (SR_ROWS.0 + 6, SR_ROWS.1);
    let mut x = 2240;
    let mut k = 0;
    while x + 2 < 2340 {
        if k % 5 != 4 {
            for yy in y0..y1 {
                for xx in x..x + 2 {
                    let p = img.get_pixel(xx, yy).0;
                    let mix =
                        |a: u8, b: u8| ((u16::from(a) * (blend - 1) + u16::from(b)) / blend) as u8;
                    img.put_pixel(
                        xx,
                        yy,
                        Rgb([
                            mix(p[0], TEXT.0[0]),
                            mix(p[1], TEXT.0[1]),
                            mix(p[2], TEXT.0[2]),
                        ]),
                    );
                }
            }
        }
        x += 5;
        k += 1;
    }
    img
}

#[tokio::test]
async fn no_frame_gives_an_sr_crop_even_with_chat_over_the_digits() {
    // No region is the SR line: chat over the SR digits joins their glyphs,
    // so no SR crop is cut at all (fail safe).
    assert_eq!(RankRegion::ALL.len(), 3);
    assert!(!RANK_SLOTS.contains(&"rank_sr"));
    let clean = rank_frame(2560, 1440).to_rgb8();
    // The stand-in shows the hero entry and its SR digits.
    let e = stat_tracker::shadow::rank::layout::first_entry(&clean).expect("the hero entry");
    assert_eq!(e.sr, SR_ROWS);
    let frames = [
        clean.clone(),
        with_chat_over_the_sr(&clean, 1),
        with_chat_over_the_sr(&clean, 2),
        rank_frame(1920, 1080).to_rgb8(),
    ];
    // The hero column (x 2120 to 2342 at 1440p, hero names and SR digits)
    // is in no crop.
    let hero_column = |w: u32| (2120 * w / 2560, 2342 * w / 2560);
    for (k, rgb) in frames.iter().enumerate() {
        let frame = DynamicImage::ImageRgb8(rgb.clone());
        let crops = rank_crops(&frame).unwrap();
        let got: Vec<&str> = crops.iter().map(|c| c.region.slot_type()).collect();
        assert_eq!(got, ["rank_role", "rank_tier", "rank_bar"], "frame {k}");
        let (hx0, hx1) = hero_column(rgb.width());
        for c in &crops {
            let s = c.spot;
            assert!(
                s.x + s.w <= hx0 || s.x >= hx1,
                "frame {k}: {:?} {s:?}",
                c.region
            );
        }
        // Through the whole offer path, into the outbox.
        let dir = tempfile::tempdir().unwrap();
        set_consent(dir.path(), true);
        let g = game(&format!("rank-no-sr-{k}"));
        if let Some(task) = offer_for_tick(
            Some(&pngs()),
            rank_crop_target(dir.path(), Some(&g)),
            &frame,
            rgb,
        ) {
            task.await.unwrap();
        }
        assert!(
            !slots(&queued(dir.path())).contains(&"rank_sr".to_string()),
            "frame {k}"
        );
    }
}

#[test]
fn a_role_band_without_a_role_icon_is_not_cut() {
    use stat_tracker::shadow::rank::layout::{RoleLine, role_icon};
    let names = |rgb: &RgbImage| -> Vec<&'static str> {
        rank_crops(&DynamicImage::ImageRgb8(rgb.clone()))
            .unwrap()
            .iter()
            .map(|c| c.region.slot_type())
            .collect()
    };
    // The open queue screen: no role line above the emblem.
    let mut empty = rank_frame(2560, 1440).to_rgb8();
    fill(&mut empty, [1060, 236, 1500, 326], BG);
    assert_eq!(role_icon(&empty), RoleLine::Empty);
    assert_eq!(cut_region(RankRegion::Role, &empty), Err(RegionMiss::Blank));
    assert_eq!(names(&empty), ["rank_tier", "rank_bar"]);
    // Ink that is not a role line (an overlay line across the band, glow,
    // a fading line): unclear, not cut either.
    let mut unclear = empty.clone();
    fill(&mut unclear, [1100, 290, 1460, 310], TEXT);
    assert_eq!(role_icon(&unclear), RoleLine::Unclear);
    assert_eq!(
        cut_region(RankRegion::Role, &unclear),
        Err(RegionMiss::Blank)
    );
    assert_eq!(names(&unclear), ["rank_tier", "rank_bar"]);
    // A role line with its icon is cut.
    let rgb = rank_frame(2560, 1440).to_rgb8();
    assert!(matches!(role_icon(&rgb), RoleLine::Icon(_)));
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
    assert_eq!(got, ["rank_role", "rank_tier", "rank_bar"]);
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
    assert_eq!(got, ["rank_role", "rank_tier"]);
    let mut both = bottom.to_rgb8();
    for x in (70..400).step_by(6) {
        fill(&mut both, [x, 1200, x + 2, 1220], TEXT);
    }
    assert_eq!(cut_region(RankRegion::Bar, &both), Err(RegionMiss::Chat));
    // No region reaches the chat strip (x 60 to 448).
    for r in RankRegion::ALL {
        assert!(r.reference_box()[0] >= 540, "{r:?}");
    }
}

#[test]
fn a_region_over_the_byte_cap_is_left_out_the_rest_kept() {
    let cap = UploadKind::Rank.crop_kind().max_crop_bytes();
    // Noise does not compress: noise in the tier (420x680) and the bar
    // (1500x184) is over 384 KiB; the role is not.
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
    assert_eq!(got, ["rank_role"]);
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
/// the regions each readable frame keeps.
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
                rank_label_shows(Path::new(&data), &frame.to_rgb8())
            );
        }
        if row["readable"] != serde_json::Value::Bool(true) {
            continue;
        }
        frames_read += 1;
        let crops = rank_crops(&frame).unwrap();
        let got: Vec<&str> = crops.iter().map(|c| c.region.slot_type()).collect();
        for g in &got {
            *kept.entry(g.to_string()).or_default() += 1;
        }
        println!("{game}/{file}: {got:?}");
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
    assert!(rank_label_shows(dir.path(), &labelled.to_rgb8()));
    // Hero highlight art and the placement screen: the kit took them as a
    // rank screen (`Some(pngs)`), but there is no rank progress label.
    for art in [false, true] {
        let frame = without_label(&labelled, art);
        assert!(!rank_label_shows(dir.path(), &frame.to_rgb8()), "art {art}");
        let target = rank_crop_target(dir.path(), Some(&g));
        assert!(offer_for_tick(Some(&pngs()), target, &frame, &frame.to_rgb8()).is_none());
        assert!(
            rank_crop_target(dir.path(), Some(&g)).is_some(),
            "art {art}: the game is not claimed"
        );
    }
    assert!(queued(dir.path()).is_empty());
    // A later rank update frame of the same game still gives its set.
    let target = rank_crop_target(dir.path(), Some(&g));
    offer_for_tick(Some(&pngs()), target, &labelled, &labelled.to_rgb8())
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
    assert!(!rank_label_shows(dir.path(), &frame.to_rgb8()));
    let target = rank_crop_target(dir.path(), Some(&g));
    assert!(offer_for_tick(Some(&pngs()), target, &frame, &frame.to_rgb8()).is_none());
    assert!(rank_crop_target(dir.path(), Some(&g)).is_some());
    // The reader takes no 4K frame: nothing offered there either, and the
    // game is claimed so the size is logged once.
    let four_k = tempfile::tempdir().unwrap();
    set_consent(four_k.path(), true);
    let target = rank_crop_target(four_k.path(), Some(&g));
    let big = rank_frame(3840, 2160);
    assert!(offer_for_tick(Some(&pngs()), target, &big, &big.to_rgb8()).is_none());
    assert!(rank_crop_target(four_k.path(), Some(&g)).is_none());
    assert!(queued(dir.path()).is_empty() && queued(four_k.path()).is_empty());
}

#[test]
fn the_label_reader_follows_the_pack_file_when_it_is_replaced() {
    use stat_tracker::shadow::rank::{PACK_FILE, TEMPLATE_DIR};
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    let rgb = rank_frame(2560, 1440).to_rgb8();
    assert!(rank_label_shows(dir.path(), &rgb));
    // The pack keeper replaces the pack with one that does not load: the
    // cached reader is not kept for the old file.
    let pack = dir.path().join(TEMPLATE_DIR).join(PACK_FILE);
    std::fs::write(&pack, b"{").unwrap();
    assert!(!rank_label_shows(dir.path(), &rgb));
    let loads = PACK_LOADS.with(|n| n.get());
    assert!(!rank_label_shows(dir.path(), &rgb), "still the bad pack");
    assert_eq!(
        PACK_LOADS.with(|n| n.get()),
        loads,
        "a pack that does not load is not parsed again on the next tick"
    );
    // A good pack again is picked up without a restart.
    save_label_pack(dir.path());
    assert!(rank_label_shows(dir.path(), &rgb));
    // No pack at all: no label.
    std::fs::remove_file(&pack).unwrap();
    assert!(!rank_label_shows(dir.path(), &rgb));
}
