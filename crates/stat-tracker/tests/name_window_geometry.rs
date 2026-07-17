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
//! The spans below are hardcoded from the measurement taken on the real
//! 2026-07-16 capture. They must never be derived from the crate's `NAME_COL_*`
//! constants: a test that reads the constant it guards asserts only that the
//! code agrees with itself, and can never fail.

use image::{DynamicImage, Rgb, RgbImage};
use stat_tracker::ocr::preprocess::{crop_name_cell, prepare_name_cell_hard_threshold};

/// Measured on the 2026-07-16 6v6 board (12 rows, dumped row_00.png): the name
/// plate spans ~0.155–0.24 of row width.
const NAME_SPAN_6V6: (f64, f64) = (0.155, 0.24);
/// 5v5 layout: name text at 26–38% of row width. In a 6v6 row this window lands
/// on the E/A/D digits — exactly what OCR read during the incident.
const NAME_SPAN_5V5: (f64, f64) = (0.26, 0.38);

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

#[test]
fn name_window_5v5_still_lands_on_the_5v5_name_column() {
    let cell = crop_name_cell(&synthetic_row(1000, 77), 5);
    assert!(
        count(&cell, MARK_5V5) > 0,
        "5v5 name window drifted off the 5v5 name column"
    );
    assert_eq!(
        count(&cell, MARK_6V6),
        0,
        "5v5 name window slid left onto the 6v6 plate span"
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
