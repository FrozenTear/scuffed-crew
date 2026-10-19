//! ocr-v1 own-row counts for a folder of Tab frames.
//!
//! Each PNG is one frame. The scoreboard is cropped the same way the daemon
//! crops it, the team size is measured, and the row is the one
//! [`crate::parse::match_player_row_by_name`] picks after the name windows
//! are read. That is the ocr-v1 arm of own-row detection.

use std::collections::BTreeMap;
use std::path::Path;

use image::DynamicImage;

use super::preprocess;
use crate::detect::hero_portrait::detect_team_size;
use crate::parse;

/// Counts from one replay. `rows` maps a matched row index to how many
/// frames picked it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OwnRowCounts {
    pub frames: u32,
    pub found: u32,
    pub failures: u32,
    pub rows: BTreeMap<usize, u32>,
}

/// The ocr-v1 own row on an already-cropped scoreboard.
pub fn own_row_on_scoreboard(
    scoreboard: &DynamicImage,
    team_size: usize,
    player_name: &str,
) -> Option<usize> {
    let rows = super::recognize_scoreboard_cells_pre_cropped(scoreboard, team_size);
    parse::match_player_row_by_name(&rows, player_name).map(|found| found.row)
}

/// Crop one Tab frame and run [`own_row_on_scoreboard`].
///
/// `Err` is a frame that has no 5v5 or 6v6 team size. `Ok(None)` was read
/// and did not match the name.
pub fn own_row_on_frame(frame: &DynamicImage, player_name: &str) -> Result<Option<usize>, ()> {
    let board = preprocess::crop_scoreboard(frame);
    let team_size = detect_team_size(&board);
    if team_size != 5 && team_size != 6 {
        return Err(());
    }
    Ok(own_row_on_scoreboard(&board, team_size, player_name))
}

/// Replay every PNG in `dir` (this directory only). A file that does not
/// decode is a failure. Names read from the frames are not returned.
pub fn tally_own_row_dir(dir: &Path, player_name: &str) -> OwnRowCounts {
    let mut counts = OwnRowCounts::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        counts.failures = 1;
        return counts;
    };
    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
        })
        .collect();
    paths.sort();
    for path in paths {
        counts.frames += 1;
        let img = match image::open(&path) {
            Ok(img) => img,
            Err(_) => {
                counts.failures += 1;
                continue;
            }
        };
        match own_row_on_frame(&img, player_name) {
            Ok(Some(row)) => {
                counts.found += 1;
                *counts.rows.entry(row).or_default() += 1;
            }
            Ok(None) => {}
            Err(()) => counts.failures += 1,
        }
    }
    counts
}

/// Text for the replay example. Counts only: no player name.
pub fn format_own_row_counts(counts: &OwnRowCounts) -> String {
    let mut out = format!(
        "frames {}\nfound {}\nfailures {}\n",
        counts.frames, counts.found, counts.failures
    );
    for (row, n) in &counts.rows {
        out.push_str(&format!("row {row} {n}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
    use image::{Rgb, RgbImage};

    const PLAYER: &str = "FrozenTear";
    const NEAR_MISS: &str = "FrozenTea";

    /// Subset of DejaVu Sans Bold. It has the Latin letters these names
    /// use and no digits, so the decoys are name-shaped rather than
    /// `Name#1234`. Licence: `fixtures/map-label/LICENSE`.
    const SANS_BOLD: &[u8] = include_bytes!("../../fixtures/map-label/DejaVuSans-Bold.subset.ttf");

    fn counts_text_has_no_decoy() {
        let counts = OwnRowCounts {
            frames: 4,
            found: 3,
            failures: 1,
            rows: BTreeMap::from([(4, 3)]),
        };
        let text = format_own_row_counts(&counts);
        assert_eq!(text, "frames 4\nfound 3\nfailures 1\nrow 4 3\n");
        assert!(
            !text
                .split_whitespace()
                .any(|word| word == PLAYER || word == NEAR_MISS)
        );
    }

    #[test]
    fn replay_counts_print_only_counts() {
        counts_text_has_no_decoy();
    }

    #[test]
    fn a_bad_png_is_a_failure_and_other_files_are_skipped() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("notes.txt"), b"not a frame").unwrap();
        std::fs::write(dir.path().join("broken.png"), b"not a png").unwrap();
        let counts = tally_own_row_dir(dir.path(), PLAYER);
        assert_eq!(counts.frames, 1);
        assert_eq!(counts.found, 0);
        assert_eq!(counts.failures, 1);
        assert!(counts.rows.is_empty());
        let text = format_own_row_counts(&counts);
        assert!(!text.contains("notes"));
        assert!(!text.contains("broken"));
        assert!(!text.split_whitespace().any(|word| word == NEAR_MISS));
    }

    /// 6v6 name window, matching [`preprocess::crop_name_cells`].
    fn name_cell_rect(w: u32, row_h: u32, row_top: u32) -> (u32, u32, u32, u32) {
        let x = (w as f64 * 0.15) as u32;
        let cell_w = (w as f64 * 0.10) as u32;
        let pad_y = (row_h as f64 * 0.15) as u32;
        let y = row_top + pad_y;
        let cell_h = row_h - pad_y * 2;
        (x, y, cell_w, cell_h)
    }

    fn row_top(h: u32, team_size: usize, row: usize) -> (u32, u32) {
        let team1 = (h as f64 * 0.025) as u32;
        let team2 = (h as f64 * 0.565) as u32;
        let row_h = (team2 - team1) / (team_size as u32 + 1);
        let top = if row < team_size {
            team1 + row as u32 * row_h
        } else {
            team2 + (row - team_size) as u32 * row_h
        };
        (top, row_h)
    }

    fn draw_name(img: &mut RgbImage, font: &FontRef<'_>, text: &str, rect: (u32, u32, u32, u32)) {
        let (x0, y0, box_w, box_h) = rect;
        let mut scale = PxScale::from(box_h as f32 * 0.72);
        let width_of = |scale: PxScale| {
            let scaled = font.as_scaled(scale);
            text.chars()
                .map(|ch| scaled.h_advance(font.glyph_id(ch)))
                .sum::<f32>()
        };
        let limit = box_w as f32 - 6.0;
        if width_of(scale) > limit {
            scale = PxScale::from(scale.y * limit / width_of(scale));
        }
        let scaled = font.as_scaled(scale);
        let mut cursor = x0 as f32 + 3.0;
        let baseline = y0 as f32 + (box_h as f32 - scaled.height()) / 2.0 + scaled.ascent();
        for ch in text.chars() {
            let id = font.glyph_id(ch);
            assert_ne!(id.0, 0, "font has no glyph for {ch}");
            let mut glyph = id.with_scale(scale);
            glyph.position = point(cursor, baseline);
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|dx, dy, cov| {
                    if cov < 0.45 {
                        return;
                    }
                    let x = (bounds.min.x + dx as f32).round() as i32;
                    let y = (bounds.min.y + dy as f32).round() as i32;
                    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
                        return;
                    }
                    img.put_pixel(x as u32, y as u32, Rgb([248, 248, 248]));
                });
            }
            cursor += scaled.h_advance(id);
        }
    }

    /// Synthetic 6v6 scoreboard crop. Row 4 is the player. Row 9 is the
    /// near-miss. The other rows are decoys.
    fn synthetic_tab_board() -> DynamicImage {
        let (w, h) = (2200, 1200);
        let mut img = RgbImage::from_pixel(w, h, Rgb([18, 20, 28]));
        let font = FontRef::try_from_slice(SANS_BOLD).expect("bundled sans bold");
        let names = [
            "QuartzLane",
            "MinaOwl",
            "NimbleFox",
            "PebbleHut",
            PLAYER,
            "SableWren",
            "CinderVale",
            "MossKite",
            "AmberLyn",
            NEAR_MISS,
            "RioVale",
            "KiteMoss",
        ];
        for (row, name) in names.iter().enumerate() {
            let (top, row_h) = row_top(h, 6, row);
            draw_name(&mut img, &font, name, name_cell_rect(w, row_h, top));
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn ocr_v1_own_row_picks_the_rendered_name_not_the_near_miss() {
        let board = synthetic_tab_board();
        let row = own_row_on_scoreboard(&board, 6, PLAYER);
        assert_eq!(row, Some(4));
    }
}
