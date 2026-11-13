//! The opt-in rank screen crops (`crops/rank.png`, `crops/rank_banner.png`):
//! off by default, only with the box ticked, `rank_update` only, and never a
//! reason for a report to fail.

use image::{DynamicImage, Rgb, RgbImage};
use serde_json::Value;

use super::bundle::{
    self, GameCategory, GameReportInput, RANK_CROP_MAX_BYTES, RANK_CROPS, RANK_SCREEN_CLASS,
    build_game_report,
};
use super::kit;
use super::tests::game::{frame, input, reads, unzip};
use crate::detect::match_end::OutcomeSource;

/// A plain dark 1920x1080 frame: the rank screen detector's word was seen
/// on it and it is not a scoreboard, so it gives the rank screen crops.
fn rank_frame() -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_pixel(1920, 1080, Rgb([24, 20, 18])))
}

/// A data dir with a scoreboard kit for `sess-1`, plus the rank screen
/// crops when `rank_seen`.
fn seeded(rank_seen: bool) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    kit::record_scoreboard(tmp.path(), "sess-1", &frame(), Some(6), &reads(Some(6))).unwrap();
    if rank_seen {
        let pngs = kit::rank_screen_pngs(Some(OutcomeSource::RankScreen), &rank_frame()).unwrap();
        kit::record_rank_screen_pngs(tmp.path(), "sess-1", &pngs).unwrap();
    }
    tmp
}

fn ticked(dir: &std::path::Path) -> GameReportInput {
    GameReportInput {
        rank_included: true,
        ..input(dir, GameCategory::WrongStats)
    }
}

fn manifest(zip: &bundle::ReportZip) -> Value {
    serde_json::from_str(&zip.manifest_json).unwrap()
}

/// `(path, role, screen_class)` of every manifest file.
fn listed(zip: &bundle::ReportZip) -> Vec<(String, String, Value)> {
    manifest(zip)["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["path"].as_str().unwrap().to_string(),
                f["role"].as_str().unwrap().to_string(),
                f["screen_class"].clone(),
            )
        })
        .collect()
}

fn rank_paths(zip: &bundle::ReportZip) -> Vec<String> {
    listed(zip)
        .into_iter()
        .map(|(p, _, _)| p)
        .filter(|p| RANK_CROPS.contains(&p.as_str()))
        .collect()
}

/// Random-looking pixels: a PNG that does not compress.
fn noise_png(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    let img = RgbImage::from_fn(w, h, |_, _| {
        let mut px = [0u8; 3];
        for c in &mut px {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *c = x as u8;
        }
        Rgb(px)
    });
    let mut buf = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .unwrap();
    buf.into_inner()
}

#[test]
fn rank_included_is_off_by_default() {
    // The app's input: a missed game with no kit needs no saved data.
    let tmp = tempfile::tempdir().unwrap();
    let config = crate::config::Config {
        data_dir: tmp.path().to_path_buf(),
        ..crate::config::Config::default()
    };
    let built = GameReportInput::from_config(
        &config,
        "sess-1",
        GameCategory::MissedGame,
        String::new(),
        crate::capture::CaptureBackend::None,
        false,
        Vec::new(),
        chrono::Utc::now(),
    )
    .unwrap();
    assert!(!built.rank_included);
}

#[test]
fn a_kit_from_before_the_rank_crops_loads_without_them() {
    // An older kit folder: scoreboard, outcome and reads, no rank files.
    let tmp = seeded(false);
    let png = kit::rank_outcome_png(Some(OutcomeSource::RankScreen), &rank_frame()).unwrap();
    kit::record_outcome_png(tmp.path(), "sess-1", &png).unwrap();
    let loaded = kit::load(tmp.path(), "sess-1");
    assert!(loaded.scoreboard_png.is_some() && loaded.outcome_png.is_some());
    assert_eq!((loaded.rank_png, loaded.rank_banner_png), (None, None));
    // ticked, the report still builds, with the crops it has
    let zip = build_game_report(&ticked(tmp.path())).unwrap();
    assert!(rank_paths(&zip).is_empty());
    assert_eq!(zip.images.len(), 2);
}

#[test]
fn off_puts_no_rank_crop_in_the_report() {
    let tmp = seeded(true);
    let inp = input(tmp.path(), GameCategory::WrongStats);
    assert!(inp.kit.rank_png.is_some() && inp.kit.rank_banner_png.is_some());
    assert!(!inp.rank_included);
    let zip = build_game_report(&inp).unwrap();
    assert!(rank_paths(&zip).is_empty(), "{:?}", listed(&zip));
    let files = unzip(&zip.bytes);
    for path in RANK_CROPS {
        assert!(!files.contains_key(path), "{path}");
    }
    assert!(
        zip.images
            .iter()
            .all(|i| !RANK_CROPS.contains(&i.path.as_str()))
    );
    // the preview before a build never shows them either
    let preview = bundle::kit_preview_images(&inp.kit);
    assert!(
        preview
            .iter()
            .all(|i| !RANK_CROPS.contains(&i.path.as_str()))
    );
}

#[test]
fn ticked_with_a_rank_frame_carries_both_crops_as_rank_update() {
    let tmp = seeded(true);
    let inp = ticked(tmp.path());
    let zip = build_game_report(&inp).unwrap();
    let files = unzip(&zip.bytes);
    assert_eq!(rank_paths(&zip), RANK_CROPS.map(String::from).to_vec());
    for (path, role, class) in listed(&zip) {
        let rank = RANK_CROPS.contains(&path.as_str());
        if path != bundle::LOG_PATH {
            assert_eq!(role, "crop", "{path}");
        }
        // those two paths carry rank_update, and nothing else does
        assert_eq!(class == RANK_SCREEN_CLASS, rank, "{path}: {class}");
    }
    for (path, kept) in RANK_CROPS
        .into_iter()
        .zip([&inp.kit.rank_png, &inp.kit.rank_banner_png])
    {
        assert_eq!(Some(&files[path]), kept.as_ref(), "{path}");
        let img = zip.images.iter().find(|i| i.path == path).unwrap();
        assert_eq!(files[path], img.png);
        assert!(files[path].len() <= RANK_CROP_MAX_BYTES);
    }
    assert_eq!(files.len(), 6, "{:?}", files.keys().collect::<Vec<_>>());
}

#[test]
fn ticked_without_a_rank_frame_builds_without_them() {
    let tmp = seeded(false);
    let zip = build_game_report(&ticked(tmp.path())).unwrap();
    assert!(rank_paths(&zip).is_empty());
    assert_eq!(zip.images.len(), 1);
}

#[test]
fn an_over_cap_or_unreadable_rank_crop_is_left_out_and_the_report_builds() {
    let tmp = seeded(true);
    let mut inp = ticked(tmp.path());
    let banner = inp.kit.rank_banner_png.clone().unwrap();
    // just over the cap, as the site would count it
    inp.kit.rank_png = Some(vec![0u8; RANK_CROP_MAX_BYTES + 1]);
    let zip = build_game_report(&inp).unwrap();
    assert_eq!(rank_paths(&zip), vec!["crops/rank_banner.png".to_string()]);
    assert_eq!(unzip(&zip.bytes)["crops/rank_banner.png"], banner);

    inp.kit.rank_png = Some(b"not a png".to_vec());
    inp.kit.rank_banner_png = Some(Vec::new());
    let zip = build_game_report(&inp).unwrap();
    assert!(rank_paths(&zip).is_empty());
    assert_eq!(zip.images.len(), 2);
}

#[test]
fn rank_crops_that_would_push_the_zip_over_10_mb_are_dropped() {
    let tmp = seeded(true);
    let mut inp = ticked(tmp.path());
    // A large outcome picture (it is not capped per file) leaves no room.
    inp.kit.outcome_png = Some(noise_png(1500, 1500, 1));
    inp.kit.rank_png = Some(noise_png(800, 800, 2));
    inp.kit.rank_banner_png = Some(noise_png(800, 800, 3));
    assert!(inp.kit.rank_png.as_ref().unwrap().len() <= RANK_CROP_MAX_BYTES);
    let zip = build_game_report(&inp).unwrap();
    assert!(rank_paths(&zip).is_empty());
    assert!(zip.bytes.len() <= bundle::MAX_ZIP_BYTES);
    assert_eq!(zip.images.len(), 2);
}

#[test]
fn rank_screen_crops_follow_the_outcome_rule_and_the_edge_cap() {
    // Not the rank screen: no crops at all.
    assert!(kit::rank_screen_pngs(None, &rank_frame()).is_none());
    assert!(kit::rank_screen_pngs(Some(OutcomeSource::ResultWord), &rank_frame()).is_none());
    for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160)] {
        let frame = DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, Rgb([24, 20, 18])));
        let pngs = kit::rank_screen_pngs(Some(OutcomeSource::RankScreen), &frame).unwrap();
        // outcome.png is unchanged by the rank crops
        assert_eq!(
            Some(&pngs.outcome),
            kit::rank_outcome_png(Some(OutcomeSource::RankScreen), &frame).as_ref()
        );
        for png in [pngs.rank.unwrap(), pngs.rank_banner.unwrap()] {
            let img = image::load_from_memory(&png).unwrap();
            assert!(
                img.width().max(img.height()) <= kit::RANK_CROP_MAX_EDGE,
                "{w}x{h}"
            );
            assert!(png.len() <= RANK_CROP_MAX_BYTES);
        }
    }
}

#[test]
fn fit_edge_keeps_small_crops_and_scales_big_ones() {
    let small = RgbImage::new(440, 786);
    assert_eq!(kit::fit_edge(small.clone(), 1280), small);
    let big = kit::fit_edge(RgbImage::new(1904, 1191), 1280);
    assert_eq!(big.dimensions(), (1280, 800));
    let tall = kit::fit_edge(RgbImage::new(10, 4000), 1280);
    assert_eq!(tall.dimensions(), (3, 1280));
}

#[test]
fn a_deleted_game_gets_no_rank_crops() {
    let tmp = seeded(false);
    kit::remove(tmp.path(), "sess-1");
    let pngs = kit::rank_screen_pngs(Some(OutcomeSource::RankScreen), &rank_frame()).unwrap();
    kit::record_rank_screen_pngs(tmp.path(), "sess-1", &pngs).unwrap();
    let dir = kit::session_dir(tmp.path(), "sess-1").unwrap();
    assert!(!dir.join(kit::RANK_PNG).exists());
}
