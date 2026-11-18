//! `rank` training crops: nothing with consent off, one per game with it
//! on, the report bundle's rank region, and an over-cap crop never kept.

use std::path::Path;
use std::time::{Duration, Instant};

use image::{DynamicImage, Rgb, RgbImage};
use scuffed_types::api::training_crops::TrainingCropConsent;
use stat_tracker::sync::UploadKind;

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

/// A rank screen stand-in: flat colour with a few bands, so it encodes
/// small and fast.
fn frame(w: u32, h: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
        Rgb([(x / 64 % 4 * 60) as u8, (y / 64 % 4 * 60) as u8, 90])
    }))
}

/// A frame whose pixels name their own position, to check the cut.
fn coord_frame(w: u32, h: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
        Rgb([
            (x & 0xff) as u8,
            (y & 0xff) as u8,
            ((x >> 8) | ((y >> 8) << 4)) as u8,
        ])
    }))
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

fn queued(dir: &Path) -> Vec<stat_tracker::sync::QueuedCrop> {
    crate::sync::queued_crops(dir, UploadKind::Rank)
}

#[tokio::test]
async fn rank_consent_off_cuts_and_keeps_nothing() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false);
    let g = game("rank-off");
    let target = rank_crop_target(dir.path(), Some(&g)).expect("an open game");
    assert!(
        offer_rank_crop(target, &frame(2560, 1440)).is_none(),
        "consent off: nothing is cut"
    );
    // Even a crop already cut is not kept with consent off.
    keep_rank_crop(dir.path(), "rank-off", &frame(2560, 1440));
    assert!(queued(dir.path()).is_empty());
    assert!(!dir.path().join("training_crops").join("rank").exists());
    // And no consent at all (never read from the site) is off too.
    let fresh = tempfile::tempdir().unwrap();
    let target = rank_crop_target(fresh.path(), Some(&g)).unwrap();
    assert!(offer_rank_crop(target, &frame(2560, 1440)).is_none());
    assert!(queued(fresh.path()).is_empty());
}

#[tokio::test]
async fn rank_consent_on_keeps_one_crop_per_game() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    let g = game("rank-once");
    let target = rank_crop_target(dir.path(), Some(&g)).expect("not offered yet");
    offer_rank_crop(target, &frame(2560, 1440))
        .expect("consent on: cut")
        .await
        .unwrap();
    let kept = queued(dir.path());
    assert_eq!(kept.len(), 1);
    let crop = &kept[0];
    assert_eq!(crate::sync::check_crop(UploadKind::Rank, crop), Ok(()));
    assert_eq!(crop.meta.screen.as_deref(), Some("rank_update"));
    assert_eq!(crop.meta.slot_type.as_deref(), Some("rank_band"));
    assert_eq!(
        (crop.meta.screen_width, crop.meta.screen_height),
        (Some(2560), Some(1440))
    );
    assert!(crop.meta.hero.is_none());
    assert!(crate::sync::game_token_ok(
        crop.meta.game_token.as_deref().unwrap()
    ));
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
async fn rank_screen_under_the_minimum_size_is_not_cut() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    let g = game("rank-small");
    let target = rank_crop_target(dir.path(), Some(&g)).unwrap();
    assert!(offer_rank_crop(target, &frame(1600, 900)).is_none());
    assert!(queued(dir.path()).is_empty());
}

#[test]
fn rank_crop_over_the_byte_cap_is_skipped_never_kept() {
    let cap = UploadKind::Rank.crop_kind().max_crop_bytes();
    // Noise at the floor width does not compress: over the cap.
    let miss = encode_rank_crop(noise(RANK_SHRINK_FLOOR_WIDTH, 480));
    let Err(RankEncodeMiss::OverCap(bytes)) = miss else {
        panic!("noise must be over the cap: {miss:?}");
    };
    assert!(bytes > cap, "{bytes}");

    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true);
    let meta = crate::sync::rank_meta(2560, 1440);
    keep_rank_result(dir.path(), "rank-cap", meta.clone(), miss);
    assert!(
        queued(dir.path()).is_empty(),
        "an over-cap crop is not kept"
    );
    assert!(crate::sync::training_warning(dir.path()).is_none());
    // A PNG over the cap that reached the store is refused there too.
    let big = vec![0u8; cap + 1];
    keep_rank_result(dir.path(), "rank-cap-2", meta, Ok(big));
    assert!(queued(dir.path()).is_empty());
}

#[test]
fn rank_cut_is_the_report_bundles_rank_region() {
    for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160)] {
        let f = coord_frame(w, h);
        let [_, report_rank, _] = stat_tracker::report::blank::rank_screen_bands(&f);
        let band = rank_band(&f);
        assert_eq!(band, report_rank, "{w}x{h}: same pixels as crops/rank.png");
        // The band sits where RANK_BAND says, in thousandths of the 16:9
        // playfield: x 210 + 744, y 83 + 827. Its corner pixel names its
        // own position in the frame.
        let x0 = w * 210 / 1000;
        let y0 = h * 83 / 1000;
        let px = band.get_pixel(0, 0);
        let near = |got: u8, want: u32| (i64::from(got) - i64::from(want & 0xff)).abs() <= 2;
        assert!(near(px[0], x0) && near(px[1], y0), "{w}x{h}: {px:?}");
        assert!(band.width().abs_diff(w * 744 / 1000) <= 2, "{w}x{h}");
        assert!(band.height().abs_diff(h * 827 / 1000) <= 2, "{w}x{h}");
    }
}

#[test]
fn rank_crop_fits_the_kind_edges_with_the_band_aspect() {
    let (max_w, max_h) = UploadKind::Rank.crop_kind().max_edges();
    assert_eq!(fit_box((1904, 1191), (max_w, max_h)), (1151, 720));
    assert_eq!(fit_box((800, 600), (max_w, max_h)), (800, 600));
    assert_eq!(fit_box((3000, 100), (max_w, max_h)), (1536, 51));
    let band = rank_band(&frame(2560, 1440));
    let png = encode_rank_crop(band.clone()).expect("a flat band fits");
    let (pw, ph) = crate::sync::png_size(&png).unwrap();
    assert!(pw <= max_w && ph <= max_h, "{pw}x{ph}");
    let aspect = |a: u32, b: u32| f64::from(a) / f64::from(b);
    assert!((aspect(pw, ph) - aspect(band.width(), band.height())).abs() < 0.01);
}

#[test]
fn the_rank_screen_class_is_the_report_bundles() {
    assert_eq!(
        crate::sync::RANK_SCREEN,
        stat_tracker::report::bundle::RANK_SCREEN_CLASS
    );
}
