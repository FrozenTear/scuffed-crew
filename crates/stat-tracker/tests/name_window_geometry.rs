//! CI regression guard for the 2026-07-16 "games didn't register" incident.
//!
//! The real-frame replay (`player_row_fixtures.rs`) is `#[ignore]`d because its
//! fixtures are copyrighted game captures that must stay local (see
//! `test-data/.gitignore`), so CI never runs it. These tests rebuild the two
//! things the incident actually turned on — the name-column window and the
//! hard-threshold fallback — from synthetic pixels, so CI guards the
//! regression without shipping game art.
//!
//! Synthetic pixels do not prove Tesseract reads real game frames; that is what
//! the local replay is for. Neither test replaces the other.
//!
//! The 5v5 tests guard the 2026-10-12 "own row not found" incident the same
//! way: the single 5v5 window fitted only the centred pre-match table and read
//! the E/A digits or perk icons on every in-match board.
//!
//! The spans below are hardcoded from the measurements taken on the real
//! 2026-07-16 (6v6) and 2026-10-12 (5v5) captures. They must never be derived from the crate's `NAME_COL_*`
//! constants: a test that reads the constant it guards asserts only that the
//! code agrees with itself, and can never fail.

use image::{DynamicImage, Rgb, RgbImage};
use stat_tracker::ocr::preprocess::{
    crop_name_cell, crop_name_cells, prepare_name_cell_hard_threshold,
};

/// Measured on the 2026-07-16 6v6 board (12 rows, dumped row_00.png): the name
/// plate spans ~0.155–0.24 of row width.
///
/// The crate's window is deliberately wider than the plate, so these tests
/// catch a window that leaves the plate — not small drift within it. The
/// incident-sized regression (the 5v5 window on a 6v6 board) is caught; a few
/// percent of drift is not.
const NAME_SPAN_6V6: (f64, f64) = (0.155, 0.24);
/// The pre-2026-10 5v5 window (26–38% of row width). In a 6v6 row it lands on
/// the E/A/D digits — exactly what OCR read during the incident.
const NAME_SPAN_5V5: (f64, f64) = (0.26, 0.38);

// 5v5 spans measured 2026-10-12 on 15 in-match boards (0, 1 and 2 perks) and
// one centred pre-match board, 2560x1440, as fractions of row width. Names
// cover every row of team 1, own row included.

/// In-match board, no perks: names.
const NAME_SPAN_5V5_INGAME: (f64, f64) = (0.106, 0.225);
/// In-match board, no perks: the E digit column starts here.
const E_DIGITS_5V5_INGAME: (f64, f64) = (0.27, 0.30);
/// In-match board with 1 or 2 perks: names (the table shifts left).
const NAME_SPAN_5V5_PERKS: (f64, f64) = (0.097, 0.184);
/// In-match board with perks: the perk icons (1-perk 0.237-0.269, 2-perk
/// 0.214-0.246 and 0.260-0.292).
const PERK_ICONS_5V5: (f64, f64) = (0.214, 0.292);
/// Centred pre-match board: names, and the E digit column after them.
const NAME_SPAN_5V5_CENTRED: (f64, f64) = (0.328, 0.415);
const E_DIGITS_5V5_CENTRED: (f64, f64) = (0.46, 0.49);
/// In-match board with 1 perk: the ult-charge ring left of the name (about;
/// the 2-perk ring sits further left, 0.027-0.054).
const ULT_RING_5V5_PERKS: (f64, f64) = (0.048, 0.078);

const MARK_6V6: Rgb<u8> = Rgb([255, 0, 0]);
const MARK_5V5: Rgb<u8> = Rgb([0, 0, 255]);

/// A synthetic row carrying one marker across each measured span and nothing
/// else, so a crop can be scored by which marker it caught.
fn synthetic_row(w: u32, h: u32) -> DynamicImage {
    let mut img = RgbImage::from_pixel(w, h, Rgb([0, 0, 0]));
    for (span, mark) in [(NAME_SPAN_6V6, MARK_6V6), (NAME_SPAN_5V5, MARK_5V5)] {
        let x0 = (w as f64 * span.0) as u32;
        let x1 = ((w as f64 * span.1) as u32).min(w);
        for x in x0..x1 {
            for y in 0..h {
                img.put_pixel(x, y, mark);
            }
        }
    }
    DynamicImage::ImageRgb8(img)
}

fn count(img: &DynamicImage, mark: Rgb<u8>) -> usize {
    img.to_rgb8().pixels().filter(|p| **p == mark).count()
}

#[test]
fn name_window_6v6_lands_on_the_plate_not_the_digits() {
    let cell = crop_name_cell(&synthetic_row(1000, 77), 6);
    assert!(
        count(&cell, MARK_6V6) > 0,
        "6v6 name window missed the name plate entirely"
    );
    assert_eq!(
        count(&cell, MARK_5V5),
        0,
        "6v6 name window reached into the E/A/D digit column — this is the \
         2026-07-16 regression: the 5v5 window applied to a 6v6 board"
    );
}

/// A row whose every column is painted with its own x, so a crop says exactly
/// which columns it took.
fn ruler_row(w: u32, h: u32) -> DynamicImage {
    let mut img = RgbImage::new(w, h);
    for x in 0..w {
        for y in 0..h {
            img.put_pixel(x, y, Rgb([(x & 0xff) as u8, (x >> 8) as u8, 77]));
        }
    }
    DynamicImage::ImageRgb8(img)
}

/// Column range `[x0, x1)` a ruler crop came from, as fractions of `w`.
fn window_of(cell: &DynamicImage, w: u32) -> (f64, f64) {
    let rgb = cell.to_rgb8();
    let xs: Vec<u32> = rgb
        .pixels()
        .map(|p| p.0[0] as u32 | ((p.0[1] as u32) << 8))
        .collect();
    let x0 = *xs.iter().min().unwrap();
    let x1 = *xs.iter().max().unwrap() + 1;
    (x0 as f64 / w as f64, x1 as f64 / w as f64)
}

fn covers(win: (f64, f64), span: (f64, f64)) -> bool {
    win.0 <= span.0 && span.1 <= win.1
}

fn overlaps(win: (f64, f64), span: (f64, f64)) -> bool {
    win.0 < span.1 && span.0 < win.1
}

const ROW_W: u32 = 1664;

fn windows_5v5() -> Vec<(f64, f64)> {
    crop_name_cells(&ruler_row(ROW_W, 90), 5)
        .iter()
        .map(|c| window_of(c, ROW_W))
        .collect()
}

#[test]
fn name_window_5v5_ingame_covers_the_names_and_stops_before_the_digits() {
    let wins = windows_5v5();
    assert!(
        wins.iter()
            .any(|&w| covers(w, NAME_SPAN_5V5_INGAME) && !overlaps(w, E_DIGITS_5V5_INGAME)),
        "no 5v5 window holds the in-match names {NAME_SPAN_5V5_INGAME:?} \
         without the E digits {E_DIGITS_5V5_INGAME:?}: {wins:?}"
    );
}

#[test]
fn name_window_5v5_perks_covers_the_names_and_stops_before_the_perk_icons() {
    let wins = windows_5v5();
    assert!(
        wins.iter().any(|&w| covers(w, NAME_SPAN_5V5_PERKS)
            && !overlaps(w, PERK_ICONS_5V5)
            && !overlaps(w, ULT_RING_5V5_PERKS)),
        "no 5v5 window holds the with-perks names {NAME_SPAN_5V5_PERKS:?} \
         without the perk icons {PERK_ICONS_5V5:?} or the ult ring: {wins:?}"
    );
}

#[test]
fn name_window_5v5_centred_covers_the_names_and_stops_before_the_digits() {
    let wins = windows_5v5();
    assert!(
        wins.iter()
            .any(|&w| covers(w, NAME_SPAN_5V5_CENTRED) && !overlaps(w, E_DIGITS_5V5_CENTRED)),
        "no 5v5 window holds the centred names {NAME_SPAN_5V5_CENTRED:?} \
         without the E digits {E_DIGITS_5V5_CENTRED:?}: {wins:?}"
    );
}

#[test]
fn name_windows_5v5_never_reach_a_digit_column() {
    // The in-match windows (left of the centred names) stay clear of the
    // in-match E digits; no window reaches the centred E digits.
    for w in windows_5v5() {
        assert!(
            !overlaps(w, E_DIGITS_5V5_CENTRED),
            "{w:?} reaches the centred E digits"
        );
        if w.0 < NAME_SPAN_5V5_CENTRED.0 - 0.05 {
            assert!(
                !overlaps(w, E_DIGITS_5V5_INGAME),
                "{w:?} reaches the in-match E digits"
            );
        }
    }
}

#[test]
fn name_windows_6v6_stay_one_window() {
    // 6v6 behaviour is pinned: one window, 0.15-0.25 of the row.
    let cells = crop_name_cells(&ruler_row(ROW_W, 75), 6);
    assert_eq!(cells.len(), 1, "6v6 must keep a single name window");
    let (x0, x1) = window_of(&cells[0], ROW_W);
    assert!(
        (x0 - 0.15).abs() < 1e-3 && (x1 - 0.25).abs() < 1e-3,
        "{x0}-{x1}"
    );
    assert_eq!(
        window_of(&crop_name_cell(&ruler_row(ROW_W, 75), 6), ROW_W),
        (x0, x1)
    );
}

#[test]
fn hard_threshold_keeps_glyph_ink_and_drops_the_cosmetic_plate() {
    let (w, h) = (40u32, 20u32);
    let mut img = RgbImage::new(w, h);
    // Cosmetic plate: mid-grey gradient, everywhere below the glyph floor.
    for y in 0..h {
        for x in 0..w {
            let v = (60 + (x * 120 / w)) as u8;
            img.put_pixel(x, y, Rgb([v, v, v]));
        }
    }
    // Glyph ink. Deliberately 210, not pure white: real anti-aliased nameplate
    // glyphs sit just above the floor, so a fixture painted 255 would survive
    // any floor and the assertion below would be vacuous.
    for y in 5..15 {
        for x in 25..35 {
            img.put_pixel(x, y, Rgb([210, 210, 210]));
        }
    }

    let out = prepare_name_cell_hard_threshold(&DynamicImage::ImageRgb8(img));

    // The fallback upscales 4x with a smooth filter; sample well inside each
    // region so interpolated edges don't decide the assertion.
    assert_eq!(
        out.get_pixel(120, 40).0[0],
        0,
        "glyph ink did not survive the hard threshold as black"
    );
    assert_eq!(
        out.get_pixel(40, 40).0[0],
        255,
        "cosmetic plate leaked through the hard threshold as ink — this empties \
         the name and rejects the frame"
    );
}
