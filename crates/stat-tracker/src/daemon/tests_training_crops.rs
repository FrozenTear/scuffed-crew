//! Hero icon and stat digit training crops from a Tab: cut only from a
//! stored Tab of a known game, only with consent, and never in the way of
//! recording. The cut is the raw card; a `hero_icon` over 64 KiB may be
//! stored as a 171 px downscale (`tests_training_crops_bytecap`).

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

/// Mean luma of the dark gap drawn round a synthetic career panel card:
/// the real gap measured 0 to 17.
const GAP_LUMA: u8 = 12;

/// A textured career panel card pixel, luma about `base - 5` to
/// `base + 45` (the real card measured 50 to 90 and more near its left
/// and top edges in live games).
fn card_pixel(x: u32, y: u32, base: u8) -> Rgb<u8> {
    let v = base.saturating_add(((x * 37 + y * 101 + (x * y) % 13) % 50) as u8);
    Rgb([v, v.saturating_sub(12), v.saturating_add(20)])
}

/// The career panel card [`tab_frame`] draws, `(left, top, side)`, right
/// of the 1920x1080 board crop and clear of its header strip, in a dark
/// gap from x 1585 to 1830 and y 185 to 420. The side is
/// [`CARD_SIDE`] of 1080 px.
const TAB_CARD: (u32, u32, u32) = (1610, 210, 171);

/// A 1920x1080 Tab with the six stat labels at [`REAL_LABELS`].
fn tab_frame() -> DynamicImage {
    tab_frame_with(&REAL_LABELS)
}

/// The width in frame px of the mark [`tab_frame_with`] draws for the
/// `index`th label: 8 px, but 32 px for the fourth, as the real DMG label
/// is the widest of the six.
fn label_width(index: usize) -> u32 {
    if index == 3 { 32 } else { 8 }
}

/// The left edge, in frame px, of the mark [`tab_frame_with`] centres on
/// the `index`th label at `label` (a fraction of the board width).
fn label_start(label: f64, index: usize) -> u32 {
    let (bx, _, bw, _) = ocr::preprocess::crop_scoreboard_rect(1920, 1080);
    bx + (label * f64::from(bw)).round() as u32 - label_width(index) / 2
}

/// A 1920x1080 Tab: every pixel right of the chat strip names its own
/// position, and the scoreboard's header strip is a bright bar with a dark
/// mark centred on each of `labels` (fractions of the board width), the
/// fourth wider than the rest ([`label_width`]). Right of the board, the
/// career panel card [`TAB_CARD`] in a dark gap.
fn tab_frame_with(labels: &[f64]) -> DynamicImage {
    let (w, h) = (1920, 1080);
    let (bx, by, bw, bh) = ocr::preprocess::crop_scoreboard_rect(w, h);
    let header_end = by + (f64::from(bh) * 0.024) as u32;
    let (label_y0, label_y1) = (
        by + (f64::from(bh) * 0.008) as u32,
        by + (f64::from(bh) * 0.022) as u32,
    );
    let spans: Vec<(u32, u32)> = labels
        .iter()
        .enumerate()
        .map(|(i, &c)| (label_start(c, i), label_start(c, i) + label_width(i)))
        .collect();
    let is_label = |x: u32, y: u32| {
        (label_y0..label_y1).contains(&y) && spans.iter().any(|&(s, e)| (s..e).contains(&x))
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
        } else if in_square(TAB_CARD, x, y) {
            card_pixel(x, y, 60)
        } else if (1585..1830).contains(&x) && (185..420).contains(&y) {
            Rgb([GAP_LUMA; 3])
        } else {
            coord_pixel(x, y)
        }
    });
    DynamicImage::ImageRgb8(img)
}

fn in_square((left, top, side): (u32, u32, u32), x: u32, y: u32) -> bool {
    (left..left + side).contains(&x) && (top..top + side).contains(&y)
}

/// A `w` x `h` frame of the dark gap luma `gap` with a textured card of
/// `base` luma and up at `card`, `(left, top, side)`.
fn card_frame(w: u32, h: u32, card: (u32, u32, u32), gap: u8, base: u8) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
        if in_square(card, x, y) {
            card_pixel(x, y, base)
        } else {
            Rgb([gap; 3])
        }
    }))
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
    // The raw portrait card the box was snapped to (10 px right of and
    // below the box's corner, its own side, not the box's): same pixels,
    // nothing resized or preprocessed. Not the hero name title under it.
    let (left, top, side) = TAB_CARD;
    let want = frame.crop_imm(left, top, side, side).to_rgb8();
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
async fn the_card_crop_has_one_side_and_over_256_px_is_skipped_never_resized() {
    // The cut is the card's own side, CARD_SIDE of the game rect height,
    // whatever the side of the box the panel read chose (0.1375 to 0.1825
    // of the height). 1440p: 228 px, under the 256 px cap.
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let qhd = card_frame(2560, 1440, (1621, 209, 228), GAP_LUMA, 60);
    for (n, side) in [252, 263].into_iter().enumerate() {
        let read = panel((1598, 207, 1598 + side, 207 + side), "ana", 0.97);
        let session = format!("tc-qhd-{n}");
        assert!(run(dir.path(), &session, &qhd, source(Some(&read), Some("ana"))).await);
    }
    let queued = sync::queued_crops(dir.path(), UploadKind::HeroIcon);
    assert_eq!(queued.len(), 2);
    for crop in &queued {
        assert_eq!(sync::png_size(&crop.png), Some((228, 228)));
        assert_eq!(
            decode(&crop.png),
            qhd.crop_imm(1621, 209, 228, 228).to_rgb8()
        );
    }
    assert_eq!(sync::training_warning(dir.path()), None);

    // 4K: the card's side (341 px) is over the cap, so nothing is cut: no
    // snap, no frame copy and no PNG encode on any Tab.
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), true, false);
    let uhd = card_frame(3840, 2160, (2432, 314, 341), GAP_LUMA, 60);
    let over = panel((2420, 320, 2720, 620), "ana", 0.97);
    assert!(!run(dir.path(), "tc-uhd", &uhd, source(Some(&over), Some("ana"))).await);
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

/// [`snap_to_card`]'s cut, `None` for either miss.
fn snap(frame: &DynamicImage, corner: (u32, u32)) -> Option<(u32, u32, u32)> {
    snap_to_card(frame, corner).ok()
}

/// The worst of the cut's left, top, right and bottom distances from the
/// drawn `card`, in px, or `None` for no cut.
fn placement_error(cut: Option<(u32, u32, u32)>, card: (u32, u32, u32)) -> Option<u32> {
    let (x, y, side) = cut?;
    let (left, top, card_side) = card;
    Some(
        [
            x.abs_diff(left),
            y.abs_diff(top),
            (x + side).abs_diff(left + card_side),
            (y + side).abs_diff(top + card_side),
        ]
        .into_iter()
        .max()
        .unwrap(),
    )
}

/// What [`scene_frame`] draws round and in a card.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Scene {
    /// A textured card in the dark gap.
    Plain,
    /// A dim card rim (luma 40) on the left and top, then a bright art
    /// band (luma 200) 3 to 5 px inside both edges at 1440p, so the
    /// strongest dark to bright rise is inside the card.
    ArtBand,
    /// The Practice Range: a gap of luma 16 (the game world behind the
    /// panel) round a navy card fill of 37 to 39.
    DimGap,
    /// A bright header bar (luma 200) across the card's rows, ending 13 px
    /// left of the card at 1440p.
    HeaderBar,
    /// A half bright anti-aliased first column and row (luma 25) between a
    /// gap of 0 to 6 and a navy fill of 37 to 39, as on real Practice
    /// Range Tabs.
    AntiAliased,
    /// [`Scene::DimGap`] with a half bright first column and row (luma 27).
    AntiAliasedOnDimGap,
    /// A dim fill on a black gap (luma 0): [`DIM_FILL`] in from the left
    /// and top edges, then textured card.
    DimFill,
    /// A 5v5 Tab's table border: a gap of 9, a 3 px line of luma 52 ending
    /// 40 px left of the card at 1440p, then a gap of 7 to a card of 67
    /// and up, as measured on a real Tab whose cut snapped to the line.
    ThinLine,
    /// A UI block of luma 60, 50 px wide, ending 13 px left of the card at
    /// 1440p: the widest a bright element can be in the span searched from
    /// a box 23 px left of the card with the narrowest measured gap.
    SideBlock,
    /// [`Scene::SideBlock`] above the card: a UI block of luma 60 across
    /// the card's columns, 50 px tall, ending 13 px above the card at
    /// 1440p. The top edge is scanned with the same constants as the left,
    /// so the widest bright element its span can meet must fail the
    /// sustain run there too.
    TopBlock,
}

/// Card luma by px in from its left or top edge at 1440p for
/// [`Scene::DimFill`], as measured on a real Practice Range Tab over a
/// black game world: 25 to 29 for 10 px, then rising into the art.
const DIM_FILL: [u8; 20] = [
    25, 29, 26, 28, 28, 28, 29, 29, 29, 29, 31, 34, 35, 35, 37, 39, 40, 41, 50, 57,
];

/// A `w` x `h` frame with `card`, `(left, top, side)`, drawn as `scene`,
/// px offsets scaled from 1440p to a game rect `u` px high.
fn scene_frame(w: u32, h: u32, card: (u32, u32, u32), scene: Scene, u: u32) -> DynamicImage {
    let (left, top, side) = card;
    let px = |v: f64| (v * f64::from(u) / 1440.0).round() as u32;
    let (band_start, band_end, bar_end) = (px(3.0), px(6.0), left - px(13.0));
    let line = left - px(43.0)..left - px(40.0);
    let block = left - px(63.0)..bar_end;
    let top_block = top - px(63.0)..top - px(13.0);
    let navy = |x: u32, y: u32| Rgb([37 + ((x + y) % 3) as u8; 3]);
    DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
        if in_square(card, x, y) {
            let inset = (x - left).min(y - top);
            match scene {
                Scene::DimGap => navy(x, y),
                Scene::AntiAliased if inset == 0 => Rgb([25; 3]),
                Scene::AntiAliasedOnDimGap if inset == 0 => Rgb([27; 3]),
                Scene::AntiAliased | Scene::AntiAliasedOnDimGap => navy(x, y),
                Scene::DimFill => {
                    let at = (f64::from(inset) * 1440.0 / f64::from(u)) as usize;
                    DIM_FILL
                        .get(at)
                        .map_or_else(|| card_pixel(x, y, 60), |&v| Rgb([v; 3]))
                }
                Scene::ArtBand if inset < band_start => Rgb([40; 3]),
                Scene::ArtBand if inset < band_end => Rgb([200; 3]),
                Scene::ThinLine => card_pixel(x, y, 67),
                _ => card_pixel(x, y, 60),
            }
        } else if scene == Scene::HeaderBar && x < bar_end && (top..top + side).contains(&y) {
            Rgb([200; 3])
        } else if (scene == Scene::SideBlock
            && block.contains(&x)
            && (top..top + side).contains(&y))
            || (scene == Scene::TopBlock
                && top_block.contains(&y)
                && (left..left + side).contains(&x))
        {
            Rgb([60; 3])
        } else {
            match scene {
                Scene::ThinLine if line.contains(&x) && (top..top + side).contains(&y) => {
                    Rgb([52; 3])
                }
                Scene::ThinLine if x < line.start => Rgb([9; 3]),
                Scene::ThinLine => Rgb([7; 3]),
                Scene::DimGap | Scene::AntiAliasedOnDimGap => Rgb([16; 3]),
                Scene::AntiAliased => Rgb([((x + y) % 7) as u8; 3]),
                Scene::DimFill => Rgb([0; 3]),
                _ => Rgb([GAP_LUMA; 3]),
            }
        }
    }))
}

#[test]
fn the_hero_icon_cut_snaps_to_the_card_from_an_off_box() {
    // Box corners at the OFFSETS from the card, scaled by u for other
    // frames.
    // (name, frame size, game rect left and top, u)
    let frames = [
        ("2560x1440", (2560, 1440), (0, 0), 1440),
        ("1920x1080", (1920, 1080), (0, 0), 1080),
        ("1280x720", (1280, 720), (0, 0), 720),
        ("3440x1440 ultrawide", (3440, 1440), (440, 0), 1440),
        ("1920x1200 16:10", (1920, 1200), (0, 60), 1080),
    ];
    let scenes = [
        Scene::Plain,
        Scene::ArtBand,
        Scene::DimGap,
        Scene::HeaderBar,
        Scene::AntiAliased,
        Scene::AntiAliasedOnDimGap,
        Scene::DimFill,
        Scene::ThinLine,
        Scene::SideBlock,
        Scene::TopBlock,
    ];
    for scene in scenes {
        for (name, (w, h), (gx, gy), u) in frames {
            let scale = |v: f64| (v * f64::from(u) / 1440.0).round();
            let card = (
                gx + scale(1621.0) as u32,
                gy + scale(209.0) as u32,
                (CARD_SIDE * f64::from(u)).round() as u32,
            );
            let frame = scene_frame(w, h, card, scene, u);
            for (dx, dy) in OFFSETS {
                let corner = (
                    (i64::from(card.0) + scale(dx as f64) as i64) as u32,
                    (i64::from(card.1) + scale(dy as f64) as i64) as u32,
                );
                let cut = snap(&frame, corner);
                let err = placement_error(cut, card)
                    .unwrap_or_else(|| panic!("{scene:?} {name} offset ({dx}, {dy}): no card"));
                // Exact: a px off is the miss this guards against.
                assert_eq!(
                    err, 0,
                    "{scene:?} {name} offset ({dx}, {dy}): {cut:?} for {card:?}"
                );
            }
        }
    }
}

/// A 2560x1440 frame of the gap luma `gap` with `card` filled at `fill`
/// to `fill + 2`.
fn flat_card_frame(card: (u32, u32, u32), gap: u8, fill: u8) -> DynamicImage {
    DynamicImage::ImageRgb8(RgbImage::from_fn(2560, 1440, |x, y| {
        if in_square(card, x, y) {
            Rgb([fill + ((x + y) % 3) as u8; 3])
        } else {
            Rgb([gap; 3])
        }
    }))
}

#[test]
fn no_hero_icon_cut_without_a_clear_card_inside_the_frame() {
    let card = (1621, 209, 228);
    let corner = (1598, 207);
    let cases = [
        // No card at all: all gap, or all bright.
        (
            "uniform dark",
            card_frame(2560, 1440, (0, 0, 0), GAP_LUMA, 0),
        ),
        ("uniform bright", card_frame(2560, 1440, (0, 0, 0), 120, 0)),
        // A card that does not stand out: a gap of 20 and a card of 30 to
        // 32, a rise of about 11, under CARD_MIN_STEP.
        ("low contrast", flat_card_frame(card, 20, 30)),
        // A black gap and a side of 21 to 23: a rise of 22, but too dim
        // for a card, under CARD_MIN_LUMA (the dimmest real card side, over
        // a black game world, measured 25 to 29).
        ("dim step", flat_card_frame(card, 0, 21)),
        // A bright card on a surround that is not the dark gap (a real
        // one measured 35 to 40).
        ("grey surround", card_frame(2560, 1440, card, 36, 100)),
    ];
    for (name, frame) in cases {
        assert_eq!(snap(&frame, corner), None, "{name}");
    }
    // A box 0.04 u (58 px) off the card either way: its edges are out of
    // reach, so the search does not wander onto plain gap further out or
    // plain card further in. A dark to bright step in the art within reach
    // can be taken for the edge. Beside a dark gap the far edge check then
    // keeps none (the_far_edge_check_keeps_no_shifted_cut_beside_a_dark_gap).
    // An art step is a right shift: beside the Practice Range navy panel
    // the outer tops still pass it, and the cut is kept unless the panel's
    // end qualifies as a drop (a mid-band of 22 at 1440p).
    let frame = card_frame(2560, 1440, card, GAP_LUMA, 60);
    assert_eq!(snap(&frame, corner), Some(card));
    for far in [(1563, 151), (1679, 267)] {
        assert_eq!(snap(&frame, far), None, "box at {far:?}");
    }
    // The square may end on the frame's right edge, not one px past it
    // (the card's edges are found either way, and a gap past the frame is
    // not required on the right).
    let right = (2400, 209, 228);
    for (w, want) in [(2628, Some(right)), (2627, None)] {
        let frame = card_frame(w, 1440, right, GAP_LUMA, 60);
        assert_eq!(snap(&frame, (2390, 200)), want, "{w} px wide");
    }
    // Under the card the far edge check needs the gap (real Tabs always
    // have one there), so a card on the frame's bottom edge keeps none, and
    // so does one cut past it.
    let bottom = (1621, 1300, 228);
    for h in [1528, 1527] {
        let frame = card_frame(2560, h, bottom, GAP_LUMA, 60);
        assert_eq!(snap(&frame, (1610, 1290)), None, "{h} px high");
    }
    let frame = card_frame(2560, 1540, bottom, GAP_LUMA, 60);
    assert_eq!(snap(&frame, (1610, 1290)), Some(bottom));
}

/// Box corner offsets from the card in 1440p px: the measured misses (22
/// to 23 px left and 2 px up, 4 to 13 px right and 14 to 15 px down),
/// none, and two worse ones.
const OFFSETS: [(i64, i64); 5] = [(-23, -2), (13, 15), (0, 0), (20, -20), (-40, 40)];

#[test]
fn a_painted_card_a_px_off_the_cut_side_still_cuts_at_its_left_and_top() {
    // The cut side is CARD_SIDE of u rounded; a real card can be painted a
    // px shorter (227 px at 1440p on Practice Range Tabs, its first gap row
    // at top + 227) or longer. Left and top still snap exactly, the cut
    // keeps the rounded side, and the far edge check takes a drop within 1
    // px of the cut's edge. 2 px off is past that: none.
    for (name, (w, h), u) in [
        ("2560x1440", (2560, 1440), 1440),
        ("1920x1080", (1920, 1080), 1080),
    ] {
        let scale = |v: f64| (v * f64::from(u) / 1440.0).round();
        let (left, top) = (scale(1621.0) as u32, scale(209.0) as u32);
        let cut_side = (CARD_SIDE * f64::from(u)).round() as u32;
        assert_eq!(cut_side, if u == 1440 { 228 } else { 171 });
        for scene in [Scene::Plain, Scene::DimGap] {
            for drawn in [cut_side - 2, cut_side - 1, cut_side + 1, cut_side + 2] {
                let frame = scene_frame(w, h, (left, top, drawn), scene, u);
                for (dx, dy) in OFFSETS {
                    let corner = (
                        (i64::from(left) + scale(dx as f64) as i64) as u32,
                        (i64::from(top) + scale(dy as f64) as i64) as u32,
                    );
                    let want = (drawn.abs_diff(cut_side) <= 1).then_some((left, top, cut_side));
                    assert_eq!(
                        snap(&frame, corner),
                        want,
                        "{scene:?} {name} card side {drawn}, offset ({dx}, {dy})"
                    );
                }
            }
        }
    }
}

#[test]
fn the_far_edge_check_keeps_no_shifted_cut_beside_a_dark_gap() {
    // Both scenes draw a dark gap right of the card, as round a card with
    // a gap on every side. Real Practice Range Tabs have the navy panel
    // there instead, where the right drop catches no left shift. A 30 px
    // left patch that is tall or partial then returns OuterTop
    // (an_outer_top_off_the_cut_keeps_no_30_px_left_shift_beside_the_navy_panel).
    let card = (1621, 209, 228);
    let corner = (1598, 207);
    let in_rows = |y: u32| (209..209 + 228).contains(&y);
    // A gap of 16 on every side of a navy card of 37 to 39, and a lit patch
    // (luma 45) in the card's rows from 30 px left of the card up to it.
    // The left edge lands on the patch's start (1591), 30 px off; the
    // card's own right edge then drops to the gap 30 px past the cut's.
    let lit = DynamicImage::ImageRgb8(RgbImage::from_fn(2560, 1440, |x, y| {
        if in_square(card, x, y) {
            Rgb([37 + ((x + y) % 3) as u8; 3])
        } else if (1591..1621).contains(&x) && in_rows(y) {
            Rgb([45; 3])
        } else {
            Rgb([16; 3])
        }
    }));
    // A live game: a gap of 6 and card art of 15 for the card's left 20
    // px, then 80. The true edge rises only 9, under CARD_MIN_STEP, so the
    // left edge lands on the step inside the art (1641), 20 px off; the
    // card's right edge drops to the gap 20 px inside the cut, with gap
    // from there to past the cut's edge.
    let dark_art = DynamicImage::ImageRgb8(RgbImage::from_fn(2560, 1440, |x, y| {
        if in_square(card, x, y) {
            Rgb([if x < 1641 { 15 } else { 80 }; 3])
        } else {
            Rgb([6; 3])
        }
    }));
    for (name, frame) in [("lit world patch", lit), ("dark art band", dark_art)] {
        assert_eq!(snap(&frame, corner), None, "{name}");
    }
}

/// What [`range_frame`] draws over the Practice Range surround, px at
/// 1440p.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RangePatch {
    /// The surround alone.
    Bare,
    /// Dark portrait art (luma 15) 24 to 31 px inside the card's right
    /// edge, with navy between it and the edge, as on 3 of the 55 Tabs.
    DarkArtInside,
    /// A lit patch of the game world (luma 45) in the card's columns, 30
    /// px tall, up to the card's top.
    LitAbove,
    /// A lit patch of the game world (luma 45) in the card's rows, 30 px
    /// wide, up to the card's left.
    LitLeft,
    /// [`RangePatch::LitLeft`] from 40 px above the card's top to 60 px
    /// past its bottom.
    LitLeftTall,
    /// [`RangePatch::LitLeft`] from 20 to 170 px below the card's top.
    LitLeftPartial,
}

/// A `w` x `h` frame with `card`, `(left, top, side)`, in the surround
/// measured on 55 Practice Range Tabs at 2560x1440, px offsets scaled from
/// 1440p to a game rect `u` px high, plus `patch`: a navy card of 37 to
/// 39 painted a px short (first gap row at `top + side - 1`), under it a
/// gap of 17 to 21 across its full width, right of it in its rows the navy
/// panel (37 to 39) for 40 px then 24, and the game world (16) elsewhere.
/// No gap right of the card.
fn range_frame(w: u32, h: u32, card: (u32, u32, u32), u: u32, patch: RangePatch) -> DynamicImage {
    range_frame_past(w, h, card, u, patch, 24)
}

/// [`range_frame`] with the band past the navy panel at luma `past` (22 to
/// 24 on 38 of the 55 Tabs, about 29 on the rest).
fn range_frame_past(
    w: u32,
    h: u32,
    card: (u32, u32, u32),
    u: u32,
    patch: RangePatch,
    past: u8,
) -> DynamicImage {
    range_frame_around(w, h, card, u, patch, (past, 16), 30.0)
}

/// [`range_frame_past`] with the game world at luma `world` and a left-hand
/// world patch `left_width` px wide at 1440p (LitLeft, LitLeftTall,
/// LitLeftPartial).
fn range_frame_around(
    w: u32,
    h: u32,
    card: (u32, u32, u32),
    u: u32,
    patch: RangePatch,
    (past, world): (u8, u8),
    left_width: f64,
) -> DynamicImage {
    let (left, top, side) = card;
    let px = |v: f64| (v * f64::from(u) / 1440.0).round() as u32;
    let (right, bottom) = (left + side, top + side - 1);
    let navy = |x: u32, y: u32| Rgb([37 + ((x + y) % 3) as u8; 3]);
    DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
        let in_cols = (left..right).contains(&x);
        let in_rows = (top..bottom).contains(&y);
        let over = match patch {
            RangePatch::Bare => false,
            RangePatch::DarkArtInside => {
                in_rows && (right - px(31.0)..right - px(24.0)).contains(&x)
            }
            RangePatch::LitAbove => in_cols && (top - px(30.0)..top).contains(&y),
            RangePatch::LitLeft => in_rows && (left - px(left_width)..left).contains(&x),
            RangePatch::LitLeftTall => {
                (top - px(40.0)..bottom + px(60.0)).contains(&y)
                    && (left - px(left_width)..left).contains(&x)
            }
            RangePatch::LitLeftPartial => {
                (top + px(20.0)..top + px(170.0)).contains(&y)
                    && (left - px(left_width)..left).contains(&x)
            }
        };
        let patch_luma = if patch == RangePatch::DarkArtInside {
            15
        } else {
            45
        };
        if over {
            Rgb([patch_luma; 3])
        } else if in_cols && in_rows {
            navy(x, y)
        } else if in_cols && y >= bottom {
            Rgb([17 + ((x + y) % 5) as u8; 3])
        } else if in_rows && (right..right + px(40.0)).contains(&x) {
            navy(x, y)
        } else if in_rows && x >= right {
            Rgb([past; 3])
        } else {
            Rgb([world; 3])
        }
    }))
}

/// The measured Practice Range card (x 1604, y 209, side 228 at 1440p)
/// scaled to a game rect `u` px high, with the rounded cut side.
fn range_card(u: u32) -> (u32, u32, u32) {
    let scale = |v: f64| (v * f64::from(u) / 1440.0).round() as u32;
    (
        scale(1604.0),
        scale(209.0),
        (CARD_SIDE * f64::from(u)).round() as u32,
    )
}

const RANGE_FRAMES: [(&str, (u32, u32), u32); 2] = [
    ("2560x1440", (2560, 1440), 1440),
    ("1920x1080", (1920, 1080), 1080),
];

#[test]
fn the_practice_range_surround_cuts_at_the_card_and_drops_a_vertical_shift() {
    for (name, (w, h), u) in RANGE_FRAMES {
        let card = range_card(u);
        let (left, top, side) = card;
        let scale = |v: f64| (v * f64::from(u) / 1440.0).round() as i64;
        let at = |(dx, dy): (i64, i64)| {
            (
                (i64::from(left) + scale(dx as f64)) as u32,
                (i64::from(top) + scale(dy as f64)) as u32,
            )
        };
        // The right scan finds no drop on the navy panel, or only the dark
        // art inside the cut with navy between it and the cut's edge: both
        // keep the cut.
        for patch in [RangePatch::Bare, RangePatch::DarkArtInside] {
            let frame = range_frame(w, h, card, u, patch);
            for offset in OFFSETS {
                assert_eq!(
                    snap(&frame, at(offset)),
                    Some(card),
                    "{patch:?} {name} offset {offset:?}"
                );
            }
        }
        // A lit patch above the card: the top edge lands on its start, and
        // the required bottom drop is the card's, 30 px under the cut's.
        let frame = range_frame(w, h, card, u, RangePatch::LitAbove);
        let shifted = (left, top - scale(30.0) as u32, side);
        assert_eq!(
            snap_to_card(&frame, at((-23, -2))),
            Err(SnapMiss::FarEdge {
                cut: shifted,
                right: None,
                bottom: Some(top + side - 1),
            }),
            "{name}"
        );
    }
}

#[test]
fn the_navy_panel_end_past_the_right_reach_keeps_the_exact_cut() {
    // The band past the navy panel at 22, the low end of the measured 22
    // to 24: window means of navy 38 vs 22 rise 16, at CARD_MIN_STEP, so
    // the panel's end, 40 px right of the card, can qualify as a drop.
    // CARD_RIGHT_SEARCH stops the right scan short of it, so the exact cut
    // is kept. The inversion with the old 0.03 u reach is at 1440p (FarEdge
    // with right 1872). At 1080p the same band rises just under 16, so the
    // old reach already kept the cut.
    for (name, (w, h), u) in RANGE_FRAMES {
        let card = range_card(u);
        let (left, top, _) = card;
        let scale = |v: f64| (v * f64::from(u) / 1440.0).round() as i64;
        let frame = range_frame_past(w, h, card, u, RangePatch::Bare, 22);
        for (dx, dy) in OFFSETS {
            let corner = (
                (i64::from(left) + scale(dx as f64)) as u32,
                (i64::from(top) + scale(dy as f64)) as u32,
            );
            assert_eq!(
                snap_to_card(&frame, corner),
                Ok(card),
                "{name} offset ({dx}, {dy})"
            );
        }
    }
}

#[test]
fn an_outer_top_off_the_cut_keeps_no_30_px_left_shift_beside_the_navy_panel() {
    // A 30 px (23 px at 1080p) lit patch left of the card, taller than it
    // or over only part of its rows: the left edge lands on the patch's
    // start, the navy panel gives no right drop and the bottom holds, but
    // over the cut's outer left columns the patch and the world above it
    // give no top step at the card's top. Narrower than about 26 px at
    // 1440p (19 at 1080p) is still kept
    // (a_narrow_tall_world_patch_keeps_a_horizontal_shift).
    for (name, (w, h), u) in RANGE_FRAMES {
        let card = range_card(u);
        let (left, top, side) = card;
        let scale = |v: f64| (v * f64::from(u) / 1440.0).round() as u32;
        let corner = (left - scale(23.0), top - scale(2.0));
        let shifted = (left - scale(30.0), top, side);
        for patch in [RangePatch::LitLeftTall, RangePatch::LitLeftPartial] {
            let frame = range_frame(w, h, card, u, patch);
            let got = snap_to_card(&frame, corner);
            assert!(
                matches!(got, Err(SnapMiss::OuterTop { cut, .. }) if cut == shifted),
                "{patch:?} {name}: {got:?}"
            );
        }
    }
}

/// One residual of the outer top check: a 30 px lit patch of the game
/// world that spans exactly the card's rows (its top within 1 px of the
/// card's) gives the cut's outer left columns a top step at the card's
/// top, so a cut shifted left onto it still passes and is kept, even
/// though 30 px is past the narrow-patch bound. Pinned so a future guard
/// can be pointed here.
#[test]
fn a_world_patch_on_exactly_the_card_rows_keeps_a_horizontal_shift() {
    for (name, (w, h), u) in RANGE_FRAMES {
        let card = range_card(u);
        let (left, top, side) = card;
        let scale = |v: f64| (v * f64::from(u) / 1440.0).round() as u32;
        let frame = range_frame(w, h, card, u, RangePatch::LitLeft);
        let corner = (left - scale(23.0), top - scale(2.0));
        // 30 px left of the card at 1440p (23 px at 1080p), not None.
        let shifted = (left - scale(30.0), top, side);
        assert_eq!(snap(&frame, corner), Some(shifted), "{name}");
    }
}

/// Another residual: a tall lit patch 25 px wide at 1440p, under the
/// about 26 px bound, still keeps the cut shifted left by that width. The
/// outer-left band starts side/16 (14 px) into the cut, so it still sees
/// the card's top.
#[test]
fn a_narrow_tall_world_patch_keeps_a_horizontal_shift() {
    let (w, h, u) = (2560, 1440, 1440);
    let card = range_card(u);
    let (left, top, side) = card;
    let frame = range_frame_around(w, h, card, u, RangePatch::LitLeftTall, (24, 16), 25.0);
    let corner = (left - 23, top - 2);
    assert_eq!(
        snap(&frame, corner),
        Some((left - 25, top, side)),
        "25 px tall patch at 1440p"
    );
}

/// Not kept: a true card under a soft navy rim over its outer column
/// bands, as on 5 of the 15 live 5v5 Tabs that reached the outer top
/// check (a gap of 21 to 31 rising 8 to 15 into the card at its top).
/// Outer caps loose enough to keep them (a gap of 32 and a step of 8,
/// checked at the cut's top) also keep the 30 px left shifts in
/// an_outer_top_off_the_cut_keeps_no_30_px_left_shift_beside_the_navy_panel,
/// so the outer bands keep the near-edge caps. Also a world band over the
/// outer left only, brighter about 9 px above the top (31, then 29 for the
/// last 5 rows) over darker rows above it (about 6), as on one of those 5:
/// its outer left steps at the band, 9 px above the top (200 at 1440p,
/// as measured). Pinned so a future guard can be pointed here.
#[test]
fn a_soft_outer_rim_still_drops_a_true_card() {
    let (w, h, u) = (2560, 1440, 1440);
    let card = range_card(u);
    let (left, top, side) = card;
    let bare = range_frame(w, h, card, u, RangePatch::Bare);
    let outer = |x: u32| x < left + side / 4 || x >= left + side * 3 / 4;
    for (rim, band) in [(23, false), (30, false), (16, true)] {
        let frame = DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
            let over = (top - 43..top).contains(&y) && (left..left + side).contains(&x);
            if over && band && x < left + side / 4 {
                Rgb([match y {
                    y if y < top - 9 => 6,
                    y if y < top - 5 => 31,
                    _ => 29,
                }; 3])
            } else if over && outer(x) {
                Rgb([rim; 3])
            } else {
                *bare.as_rgb8().unwrap().get_pixel(x, y)
            }
        }));
        let got = snap_to_card(&frame, (left - 23, top - 2));
        assert!(
            matches!(got, Err(SnapMiss::OuterTop { cut, .. }) if cut == card),
            "rim {rim} band {band}: {got:?}"
        );
        if band {
            let tops = [Some(top - 9), Some(top)];
            assert_eq!(got, Err(SnapMiss::OuterTop { cut: card, tops }));
        }
    }
}

#[test]
fn a_dark_run_ending_on_the_card_is_not_a_gap() {
    // A box 43 px into the card (the reach at 1440p) still finds its left
    // edge. At 44 px the first dark run is 5 gap columns and the card's
    // first column: its mean is under CARD_GAP_MAX_LUMA, but it is not
    // gap, so no edge is found rather than one a px into the card.
    let card = (1621, 209, 228);
    let plain = card_frame(2560, 1440, card, GAP_LUMA, 60);
    let dim = scene_frame(2560, 1440, card, Scene::DimGap, 1440);
    for (name, frame) in [("plain", plain), ("Practice Range", dim)] {
        assert_eq!(snap(&frame, (1664, 209)), Some(card), "{name}");
        assert_eq!(snap(&frame, (1665, 209)), None, "{name}");
    }
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
    for (i, ((x0, width, slot), label)) in cells.iter().zip(REAL_LABELS).enumerate() {
        let centre = f64::from(x0 * 2 + width) / 2.0;
        let want = f64::from(label_start(label, i)) + f64::from(label_width(i)) / 2.0;
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
    // A suspect own row, one that names another row than the Tab's, or a
    // verdict with no row: another player's cells could be cut.
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
    // An unconfirmed own row (a strong highlight before any seed, or a
    // name fallback) is not suspect, but nothing confirmed it: none.
    for (n, voice) in [OwnRowSource::HighlightOnly, OwnRowSource::NameFallback]
        .into_iter()
        .enumerate()
    {
        let unconfirmed = OwnRow {
            source: voice,
            ..own(1, Trust::Unconfirmed)
        };
        assert!(!unconfirmed.suspect());
        assert_eq!(stat_digit_row(Some(1), Some(unconfirmed)), None);
        let mut tab = source(None, None);
        tab.own_row = Some(unconfirmed);
        let session = format!("tc-nodigits-unconfirmed-{n}");
        assert!(!run(dir.path(), &session, &frame, tab).await);
    }
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
    // E missing and a mark past MIT: shifted one column right, every gap
    // still one the reader takes and narrow before wide, but the fourth
    // group is H, not the widest (DMG).
    let shifted = [
        (586, 595),
        (649, 653),
        (720, 758),
        (846, 849),
        (935, 949),
        (1040, 1060),
    ];
    assert!(ocr::preprocess::columns_from_header_groups(&shifted, w).is_some());
    assert_eq!(training_stat_columns(&shifted, w), None);
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
async fn a_hero_icon_miss_still_records_the_tab_and_its_digits() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    set_consent(dir.path(), true, true);
    // A box 110 px left of and above the card: no card edge within reach.
    let (recorded, rows) = record_tab(
        &ctx,
        "tc-nocard",
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

/// The regression for an unconfirmed own row: the new reader stores the
/// Tab on a strong highlight that no seed has confirmed yet (and on a name
/// fallback), unmarked, but that row is not known to be the member's, so
/// it gives no stat digit crop. The same Tab with a sure verdict keeps six
/// ([`a_hero_icon_miss_still_records_the_tab_and_its_digits`]).
#[tokio::test]
async fn an_accepted_tab_with_an_unconfirmed_own_row_keeps_no_stat_digit_crops() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    for (n, voice) in [OwnRowSource::HighlightOnly, OwnRowSource::NameFallback]
        .into_iter()
        .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        set_consent(dir.path(), false, true);
        let mut analysis = sure_tab(Some("ana"));
        analysis.own_row = Some(OwnRow {
            source: voice,
            ..own(0, Trust::Unconfirmed)
        });
        let session = format!("tc-unconfirmed-{n}");
        let (recorded, rows) = record_tab(&ctx, &session, analysis, sure_board()).await;
        assert!(recorded, "an unconfirmed own row still records the Tab");
        assert_eq!(rows, 1);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            sync::queued_crops(dir.path(), UploadKind::StatDigits).is_empty(),
            "{voice:?}"
        );
        assert_eq!(sync::training_warning(dir.path()), None);
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

#[path = "tests_training_crops_bytecap.rs"]
mod bytecap;
#[path = "tests_training_crops_miss.rs"]
mod miss;
#[path = "tests_training_crops_top_band.rs"]
mod top_band;
