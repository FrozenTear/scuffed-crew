//! Name blanking for game reports (`docs/report-bundle.md` section 4).
//!
//! Every region the spec fills on a scoreboard frame lives in this module,
//! in one table each: the player-row name columns ([`NAME_COLUMNS_5V5`],
//! [`NAME_COLUMNS_6V6`]) and the playfield regions ([`SCOREBOARD_REGIONS`]).
//! The blanker, the report kit, and the tests all read these tables.
//!
//! Blanking runs on the full frame first. The scoreboard crop is cut from the
//! blanked frame, so no unblanked pixel is ever written for a report.

use image::{DynamicImage, Rgb, RgbImage};

use crate::ocr::preprocess::{crop_scoreboard_rect, game_rect_16_9, row_slot_height};

/// The one fill colour for every region (section 4).
pub const FILL: Rgb<u8> = Rgb([32, 32, 32]);

/// Header height and team 2 start, as fractions of the scoreboard crop
/// height. Same values the OCR row crop uses (`crop_player_row`).
const HEADER: f64 = 0.025;
const TEAM2_START: f64 = 0.565;

/// Columns of a player row, in thousandths of the scoreboard crop width.
/// Half-open: `[start, end)`. Portrait, name, and digits share edges and
/// cover the full row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowColumns {
    pub portrait_end: u32,
    pub name_start: u32,
    pub name_end: u32,
    pub digits_start: u32,
}

/// 5v5 columns (section 4, first table). No current 5v5 board has been
/// measured, so the blank runs from the portrait edge of the current layout
/// to the old 5v5 digit edge and covers both layouts.
pub const NAME_COLUMNS_5V5: RowColumns = RowColumns {
    portrait_end: 80,
    name_start: 80,
    name_end: 450,
    digits_start: 450,
};

/// 6v6 columns (section 4, second table). Measured on the member's 6v6
/// boards of 2026-10-11/12 at 2560x1440: the portrait ends near 78/1000 and
/// the level badge, rank icon, name, and title run from about 85/1000 to
/// 215/1000.
pub const NAME_COLUMNS_6V6: RowColumns = RowColumns {
    portrait_end: 80,
    name_start: 80,
    name_end: 255,
    digits_start: 255,
};

/// Left edge of the kept portrait cell, in thousandths of the scoreboard crop
/// width: the start of the role bar. Left of it is the game world (QC1-2).
pub const PORTRAIT_START: u32 = 20;

/// Right edge of the stat table (end of the MIT column), in thousandths of
/// the scoreboard crop width. Right of it are the voice icons, the hero card,
/// and the game world, which can hold in-world nameplates.
pub const TABLE_END: u32 = 590;

/// Centre-screen notice band ("YOU WERE ELIMINATED BY <name>", "SAVED BY
/// <name>"), in thousandths of the scoreboard crop height. The notice is drawn
/// over the stat columns, so this band is filled across the whole width, digit
/// cells included. Measured on the member's boards of 2026-10-11: notice rows
/// at 791..831 and the next line below it.
pub const NOTICE_BAND: (u32, u32) = (740, 900);

/// Bumped whenever a table in this module changes. Kits store it, and a kit
/// written under another table is never reported (it is blanked again or
/// refused).
pub const BLANK_TABLE_VERSION: u32 = 7;

/// One playfield region from the "Other name regions" table, in thousandths
/// of the 16:9 playfield.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayfieldRegion {
    pub id: &'static str,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Regions filled on every kept scoreboard frame, besides the name rows.
pub const SCOREBOARD_REGIONS: [PlayfieldRegion; 4] = [
    PlayfieldRegion {
        id: "overlay_top",
        x: 0,
        y: 0,
        w: 1000,
        h: 40,
    },
    PlayfieldRegion {
        id: "overlay_bottom",
        x: 0,
        y: 960,
        w: 1000,
        h: 40,
    },
    PlayfieldRegion {
        id: "kill_feed",
        x: 700,
        y: 40,
        w: 300,
        h: 105,
    },
    PlayfieldRegion {
        id: "chat_below",
        x: 0,
        y: 855,
        w: 1000,
        h: 145,
    },
];

/// Result word on the competitive summary (rank) screen, in thousandths of
/// the playfield. Same box `read_rank_screen_result` reads (word spans x
/// 4-12.5%, y 16-21%). It holds "VICTORY!", "DEFEAT", or "DRAW" over the
/// COMPETITIVE title and nothing else, so it is cut as it is
/// (`crops/outcome.png`). Only the rank screen is used: on the accolade
/// screen the member's own name sits just under the word box, and the
/// banner flash and the centred end title sit over gameplay and nameplates.
pub const OUTCOME_BAND: PlayfieldRegion = PlayfieldRegion {
    id: "outcome",
    x: 10,
    y: 145,
    w: 240,
    h: 80,
};

/// Which name layout to fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamLayout {
    Five,
    Six,
    /// The app could not tell 5v5 from 6v6. Both layouts are filled and the
    /// manifest says `team_size: null` (section 4, "What is dropped").
    Unknown,
}

impl TeamLayout {
    pub fn from_team_size(team_size: Option<usize>) -> Option<Self> {
        match team_size {
            Some(5) => Some(Self::Five),
            Some(6) => Some(Self::Six),
            None => Some(Self::Unknown),
            Some(_) => None,
        }
    }

    /// `team_size` for the manifest.
    pub fn team_size(self) -> Option<u32> {
        match self {
            Self::Five => Some(5),
            Self::Six => Some(6),
            Self::Unknown => None,
        }
    }
}

/// Pixel rectangle, half-open on both axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn intersects(&self, o: &Rect) -> bool {
        self.w > 0
            && self.h > 0
            && o.w > 0
            && o.h > 0
            && self.x < o.x + o.w
            && o.x < self.x + self.w
            && self.y < o.y + o.h
            && o.y < self.y + self.h
    }

    fn clipped(self, w: u32, h: u32) -> Rect {
        let x = self.x.min(w);
        let y = self.y.min(h);
        Rect {
            x,
            y,
            w: self.w.min(w - x),
            h: self.h.min(h - y),
        }
    }
}

/// Scale by a fraction in thousandths, then truncate toward zero.
fn frac(len: u32, thousandths: u32) -> u32 {
    (u64::from(len) * u64::from(thousandths) / 1000) as u32
}

/// Row bands `(y, height)` in scoreboard-crop pixels: `team_size` rows per
/// team, full row height, clipped to the crop.
fn row_bands(crop_h: u32, team_size: usize) -> Vec<(u32, u32)> {
    let team1 = (f64::from(crop_h) * HEADER) as u32;
    let team2 = (f64::from(crop_h) * TEAM2_START) as u32;
    let row_h = row_slot_height(crop_h, team_size);
    let mut bands = Vec::with_capacity(team_size * 2);
    for base in [team1, team2] {
        for i in 0..team_size as u32 {
            let y = base + i * row_h;
            if y >= crop_h {
                continue;
            }
            bands.push((y, row_h.min(crop_h - y)));
        }
    }
    bands
}

fn columns(team_size: usize) -> RowColumns {
    if team_size == 6 {
        NAME_COLUMNS_6V6
    } else {
        NAME_COLUMNS_5V5
    }
}

fn layouts(layout: TeamLayout) -> &'static [usize] {
    match layout {
        TeamLayout::Five => &[5],
        TeamLayout::Six => &[6],
        TeamLayout::Unknown => &[5, 6],
    }
}

/// Name rectangles in scoreboard-crop pixels.
pub fn name_rects(crop_w: u32, crop_h: u32, layout: TeamLayout) -> Vec<Rect> {
    let mut out = Vec::new();
    for &ts in layouts(layout) {
        let c = columns(ts);
        let x0 = frac(crop_w, c.name_start);
        let x1 = frac(crop_w, c.name_end);
        for (y, h) in row_bands(crop_h, ts) {
            out.push(Rect {
                x: x0,
                y,
                w: x1 - x0,
                h,
            });
        }
    }
    out
}

/// Portrait and digit rectangles of one team size, in scoreboard-crop
/// pixels. These must stay byte-identical after blanking.
pub fn protected_rects(crop_w: u32, crop_h: u32, team_size: usize) -> Vec<Rect> {
    let c = columns(team_size);
    let portrait_x = frac(crop_w, PORTRAIT_START);
    let portrait_end = frac(crop_w, c.portrait_end).max(portrait_x);
    let digits_x = frac(crop_w, c.digits_start);
    let table_end = frac(crop_w, TABLE_END).max(digits_x);
    let mut out = Vec::new();
    for (y, h) in row_bands(crop_h, team_size) {
        out.push(Rect {
            x: portrait_x,
            y,
            w: portrait_end - portrait_x,
            h,
        });
        out.push(Rect {
            x: digits_x,
            y,
            w: table_end - digits_x,
            h,
        });
    }
    out
}

/// The notice band in scoreboard-crop pixels, full width.
pub fn notice_band(crop_w: u32, crop_h: u32) -> Rect {
    let (y0, y1) = (frac(crop_h, NOTICE_BAND.0), frac(crop_h, NOTICE_BAND.1));
    Rect {
        x: 0,
        y: y0,
        w: crop_w,
        h: y1 - y0,
    }
}

/// What `crops/scoreboard.png` keeps, in scoreboard-crop pixels: the
/// portrait and digit cells of each row, minus the notice band. Everything
/// else is the fill colour.
///
/// Digit cells are kept for `own_row` only (the member's row, index 0..2n
/// with team 1 first, the same index `crop_player_row` takes): the table rows
/// are translucent, and the glyph test cannot prove the world behind a row
/// holds no name for every team colour (QC1-8). The member's row is the only
/// row the tracker reads stats from. Other rows keep only their portrait. With
/// no own row, or an unknown team size, no digit cell is kept.
pub fn kept_rects(
    crop_w: u32,
    crop_h: u32,
    layout: TeamLayout,
    own_row: Option<usize>,
) -> Vec<Rect> {
    let band = notice_band(crop_w, crop_h);
    let sizes: &[usize] = match layout {
        TeamLayout::Five => &[5],
        TeamLayout::Six => &[6],
        // Unknown size: keep only what both layouts protect, so no name
        // column of either layout is kept.
        TeamLayout::Unknown => &[5, 6],
    };
    // A row cell that touches the notice band is not kept at all, so no
    // row keeps half of its digits (QC1-3).
    let own_row = own_row.filter(|_| sizes.len() == 1);
    protected_rects(crop_w, crop_h, sizes[0])
        .into_iter()
        .enumerate()
        // protected_rects gives (portrait, digits) for each row in order.
        .filter(|(i, _)| i % 2 == 0 || own_row == Some(i / 2))
        .map(|(_, r)| r)
        .filter(|r| !r.intersects(&band))
        .filter(|r| sizes.len() == 1 || covered_by_all(*r, crop_w, crop_h, sizes))
        .collect()
}

fn covered_by_all(r: Rect, crop_w: u32, crop_h: u32, sizes: &[usize]) -> bool {
    sizes.iter().all(|&ts| {
        name_rects(crop_w, crop_h, layout_for(ts))
            .iter()
            .all(|n| !n.intersects(&r))
    })
}

fn layout_for(ts: usize) -> TeamLayout {
    if ts == 6 {
        TeamLayout::Six
    } else {
        TeamLayout::Five
    }
}

/// A playfield region in frame pixels. Both edges are scaled from the
/// thousandths and truncated (section 4), so a region ending at 1000 reaches
/// the last pixel. Scaling the width on its own would drop it.
fn playfield_rect(r: &PlayfieldRegion, (gx, gy, gw, gh): (u32, u32, u32, u32)) -> Rect {
    let (x0, x1) = (frac(gw, r.x), frac(gw, r.x + r.w));
    let (y0, y1) = (frac(gh, r.y), frac(gh, r.y + r.h));
    Rect {
        x: gx + x0,
        y: gy + y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}

/// The bands outside the playfield (letterbox or pillarbox), in frame pixels.
fn outside_bands(frame_w: u32, frame_h: u32) -> Vec<Rect> {
    let (gx, gy, gw, gh) = game_rect_16_9(frame_w, frame_h);
    let mut out = vec![
        Rect {
            x: 0,
            y: 0,
            w: gx,
            h: frame_h,
        },
        Rect {
            x: gx + gw,
            y: 0,
            w: frame_w.saturating_sub(gx + gw),
            h: frame_h,
        },
        Rect {
            x: 0,
            y: 0,
            w: frame_w,
            h: gy,
        },
        Rect {
            x: 0,
            y: gy + gh,
            w: frame_w,
            h: frame_h.saturating_sub(gy + gh),
        },
    ];
    out.retain(|r| r.w > 0 && r.h > 0);
    out
}

/// The named playfield regions in frame pixels, filtered by id.
fn named_regions(frame_w: u32, frame_h: u32, keep: impl Fn(&str) -> bool) -> Vec<Rect> {
    let pf = game_rect_16_9(frame_w, frame_h);
    SCOREBOARD_REGIONS
        .iter()
        .filter(|r| keep(r.id))
        .map(|r| playfield_rect(r, pf).clipped(frame_w, frame_h))
        .collect()
}

/// Every region a scoreboard frame fills besides the name rows: the
/// playfield regions plus the bands outside the playfield, in frame pixels.
pub fn frame_regions(frame_w: u32, frame_h: u32) -> Vec<Rect> {
    let mut out = named_regions(frame_w, frame_h, |_| true);
    out.extend(outside_bands(frame_w, frame_h));
    out.retain(|r| r.w > 0 && r.h > 0);
    out
}

/// What the `outcome` class fills on a rank screen frame: `overlay_top`,
/// `overlay_bottom`, and the bands outside the playfield.
pub fn outcome_regions(frame_w: u32, frame_h: u32) -> Vec<Rect> {
    let mut out = named_regions(frame_w, frame_h, |id| id.starts_with("overlay_"));
    out.extend(outside_bands(frame_w, frame_h));
    out.retain(|r| r.w > 0 && r.h > 0);
    out
}

fn fill(img: &mut RgbImage, r: Rect) {
    let r = r.clipped(img.width(), img.height());
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            img.put_pixel(x, y, FILL);
        }
    }
}

/// Blank a scoreboard frame (section 4): the playfield regions, the bands
/// outside the playfield, and every name row inside the scoreboard
/// rectangle.
pub fn blank_scoreboard_frame(frame: &DynamicImage, layout: TeamLayout) -> RgbImage {
    let mut img = frame.to_rgb8();
    let (w, h) = img.dimensions();
    for r in frame_regions(w, h) {
        fill(&mut img, r);
    }
    let sb = crop_scoreboard_rect(w, h);
    for r in name_rects(sb.2, sb.3, layout) {
        fill(
            &mut img,
            Rect {
                x: sb.0 + r.x,
                y: sb.1 + r.y,
                w: r.w,
                h: r.h,
            },
        );
    }
    img
}

/// `crops/scoreboard.png`: the scoreboard rectangle cut from the blanked
/// frame.
///
/// Allowlist: the crop starts as the fill colour and only [`kept_rects`]
/// (portrait and digit cells) are copied from the blanked frame. Names,
/// titles, badges, the header, the gaps, the hero card, the game world
/// (in-world nameplates), and the notice band all stay filled.
pub fn blanked_scoreboard(
    frame: &DynamicImage,
    layout: TeamLayout,
    own_row: Option<usize>,
) -> RgbImage {
    let blanked = blank_scoreboard_frame(frame, layout);
    let (x, y, w, h) = crop_scoreboard_rect(blanked.width(), blanked.height());
    let src = image::imageops::crop_imm(&blanked, x, y, w, h).to_image();
    allowlist_crop(&src, layout, own_row)
}

/// The allowlist step of [`blanked_scoreboard`] on an already cut scoreboard
/// crop (the tracker's `debug/accepted` ring holds such crops).
pub fn allowlist_crop(src: &RgbImage, layout: TeamLayout, own_row: Option<usize>) -> RgbImage {
    let (w, h) = src.dimensions();
    let mut out = RgbImage::from_pixel(w, h, FILL);
    // Portrait cells are opaque art and are copied as they are. The table
    // rows behind the digits are translucent, so the game world shows
    // through them, in-world nameplates included (QC1-7). In a digit cell only
    // the digit glyphs are kept: they are drawn near-white on top of the row,
    // while anything behind the row is tinted by its purple or yellow colour.
    let digits_from = frac(w, NAME_COLUMNS_6V6.digits_start);
    for r in kept_rects(w, h, layout, own_row) {
        let r = r.clipped(w, h);
        let digit_cell = r.x >= digits_from;
        // Fail closed (R236-F): the glyph test only separates digits from the
        // world behind the row when the row tint is coloured. A digit cell
        // whose background is near-neutral (a grey team colour, a filter) is
        // not kept at all.
        if digit_cell && !row_tint_is_coloured(src, r) {
            continue;
        }
        for py in r.y..r.y + r.h {
            for px in r.x..r.x + r.w {
                let p = *src.get_pixel(px, py);
                if !digit_cell || is_glyph(p) {
                    out.put_pixel(px, py, p);
                }
            }
        }
    }
    out
}

/// Darkest channel a kept digit-cell pixel may have. White digits and the
/// dim grey zeros (about 172,170,172 on the member's boards) are above it.
pub const GLYPH_MIN: u8 = 120;

/// Widest channel spread a kept digit-cell pixel may have. Glyphs are drawn
/// neutral on top of the row; anything behind the translucent row takes its
/// purple or yellow tint and is far less neutral (QC1-7).
pub const GLYPH_MAX_SPREAD: u8 = 28;

/// Smallest (max - min) / max a digit cell's median colour may have for its
/// glyphs to be kept. The member's purple rows measure about 0.6 (55,20,55)
/// and yellow rows about 0.9; a grey row is near 0.
pub const ROW_TINT_MIN_CHROMA: f32 = 0.3;

/// The median colour of a digit cell is clearly coloured, so anything seen
/// through the row is tinted away from neutral.
pub fn row_tint_is_coloured(src: &RgbImage, r: Rect) -> bool {
    let mut ch: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            let p = src.get_pixel(x, y);
            for (i, v) in p.0.iter().enumerate() {
                ch[i].push(*v);
            }
        }
    }
    if ch[0].is_empty() {
        return false;
    }
    let med: Vec<u8> = ch
        .iter_mut()
        .map(|c| {
            c.sort_unstable();
            c[c.len() / 2]
        })
        .collect();
    let (lo, hi) = (*med.iter().min().unwrap(), *med.iter().max().unwrap());
    hi > 0 && f32::from(hi - lo) / f32::from(hi) >= ROW_TINT_MIN_CHROMA
}

/// A digit glyph pixel: bright and neutral.
pub fn is_glyph(p: Rgb<u8>) -> bool {
    let (lo, hi) = (*p.0.iter().min().unwrap(), *p.0.iter().max().unwrap());
    lo >= GLYPH_MIN && hi - lo <= GLYPH_MAX_SPREAD
}

/// Blank a rank screen frame for the `outcome` class ([`outcome_regions`]).
pub fn blank_outcome_frame(frame: &DynamicImage) -> RgbImage {
    let mut img = frame.to_rgb8();
    let (w, h) = img.dimensions();
    for r in outcome_regions(w, h) {
        fill(&mut img, r);
    }
    img
}

/// `crops/outcome.png`: the result word box, cut from the blanked rank
/// screen frame (section 3). Call it only for a rank-screen outcome
/// (`OutcomeSource::RankScreen`) on this same frame.
pub fn outcome_band(frame: &DynamicImage) -> RgbImage {
    let blanked = blank_outcome_frame(frame);
    let (w, h) = blanked.dimensions();
    let r = playfield_rect(&OUTCOME_BAND, game_rect_16_9(w, h)).clipped(w, h);
    image::imageops::crop_imm(&blanked, r.x, r.y, r.w, r.h).to_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZES: [(u32, u32); 5] = [
        (1920, 1080),
        (2560, 1440),
        (3440, 1440),
        (1920, 1200),
        (1280, 720),
    ];

    /// A frame where every pixel is unique-ish and never equals FILL.
    fn noise_frame(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
            Rgb([
                (x % 200 + 40) as u8,
                (y % 200 + 40) as u8,
                ((x * 7 + y * 13) % 200 + 40) as u8,
            ])
        }))
    }

    #[test]
    fn tables_match_the_spec() {
        assert_eq!(FILL, Rgb([32, 32, 32]));
        assert_eq!(
            (
                NAME_COLUMNS_5V5.portrait_end,
                NAME_COLUMNS_5V5.name_start,
                NAME_COLUMNS_5V5.name_end,
                NAME_COLUMNS_5V5.digits_start
            ),
            (80, 80, 450, 450)
        );
        assert_eq!(
            (
                NAME_COLUMNS_6V6.portrait_end,
                NAME_COLUMNS_6V6.name_start,
                NAME_COLUMNS_6V6.name_end,
                NAME_COLUMNS_6V6.digits_start
            ),
            (80, 80, 255, 255)
        );
        let ids: Vec<_> = SCOREBOARD_REGIONS
            .iter()
            .map(|r| (r.id, r.x, r.y, r.w, r.h))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("overlay_top", 0, 0, 1000, 40),
                ("overlay_bottom", 0, 960, 1000, 40),
                ("kill_feed", 700, 40, 300, 105),
                ("chat_below", 0, 855, 1000, 145),
            ]
        );
        // The doc table must hold the same numbers.
        let doc = include_str!("../../docs/report-bundle.md");
        for line in [
            "| Name (blank) | 80/1000 | 450/1000 |",
            "| Name (blank) | 80/1000 | 255/1000 |",
            "| `overlay_top` | every kept frame | 0 | 0 | 1000 | 40 | no |",
            "| `kill_feed` | scoreboard | 700 | 40 | 300 | 105 | no |",
            "| `chat_below` | scoreboard | 0 | 855 | 1000 | 145 | no |",
            "RGB 32, 32, 32",
        ] {
            assert!(doc.contains(line), "spec line missing: {line}");
        }
    }

    #[test]
    fn no_name_rect_touches_a_portrait_or_digit_rect() {
        for (w, h) in SIZES {
            let (_, _, cw, ch) = crop_scoreboard_rect(w, h);
            for (ts, layout) in [(5, TeamLayout::Five), (6, TeamLayout::Six)] {
                let names = name_rects(cw, ch, layout);
                assert_eq!(names.len(), ts * 2, "{w}x{h} {ts}");
                for p in protected_rects(cw, ch, ts) {
                    for n in &names {
                        assert!(!n.intersects(&p), "{w}x{h} {ts}v{ts}: {n:?} vs {p:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn every_name_pixel_is_filled_and_protected_pixels_are_untouched() {
        for (w, h) in SIZES {
            let frame = noise_frame(w, h);
            let orig_sb = crop_scoreboard_rect(w, h);
            let orig = frame.to_rgb8();
            for (ts, layout) in [(5, TeamLayout::Five), (6, TeamLayout::Six)] {
                let own = Some(1);
                let out = blanked_scoreboard(&frame, layout, own);
                assert_eq!(out.dimensions(), (orig_sb.2, orig_sb.3));
                for n in name_rects(orig_sb.2, orig_sb.3, layout) {
                    for y in n.y..n.y + n.h {
                        for x in n.x..n.x + n.w {
                            assert_eq!(*out.get_pixel(x, y), FILL, "{w}x{h} {ts} name px {x},{y}");
                        }
                    }
                }
                let kept = kept_rects(orig_sb.2, orig_sb.3, layout, own);
                let digits_from = frac(orig_sb.2, NAME_COLUMNS_6V6.digits_start);
                let orig_crop = image::imageops::crop_imm(
                    &blank_scoreboard_frame(&frame, layout),
                    orig_sb.0,
                    orig_sb.1,
                    orig_sb.2,
                    orig_sb.3,
                )
                .to_image();
                for p in &kept {
                    let coloured = row_tint_is_coloured(&orig_crop, *p);
                    for y in p.y..p.y + p.h {
                        for x in p.x..p.x + p.w {
                            let src = *orig.get_pixel(orig_sb.0 + x, orig_sb.1 + y);
                            let got = *out.get_pixel(x, y);
                            if p.x < digits_from || (coloured && is_glyph(src)) {
                                assert_eq!(got, src, "{w}x{h} {ts} kept px {x},{y}");
                            } else {
                                assert_eq!(got, FILL, "{w}x{h} {ts} non-glyph px {x},{y}");
                            }
                        }
                    }
                }
                // Allowlist: every other pixel is the fill colour.
                for y in 0..out.height() {
                    for x in 0..out.width() {
                        let inside = kept
                            .iter()
                            .any(|r| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h);
                        if !inside {
                            assert_eq!(*out.get_pixel(x, y), FILL, "{w}x{h} {ts} px {x},{y}");
                        }
                    }
                }
                // Kept cells are only portrait and digit cells left of the
                // table edge, never in the notice band.
                let band = notice_band(orig_sb.2, orig_sb.3);
                let table_end = frac(orig_sb.2, TABLE_END);
                for r in &kept {
                    assert!(
                        !r.intersects(&band),
                        "{w}x{h} {ts} {r:?} in the notice band"
                    );
                    assert!(
                        r.x + r.w <= table_end,
                        "{w}x{h} {ts} {r:?} right of the table"
                    );
                }
            }
        }
    }

    #[test]
    fn notices_nameplates_and_the_left_strip_are_filled() {
        // QC1-1 / QC1-2 at the measured spots, on a 2560x1440 frame: a notice
        // across the stat columns (crop y 791..831 per mille), a nameplate in
        // the gap by the hero card (x 600..700, y 500..540), one at the right
        // edge, and world pixels left of the role bar.
        let (w, h) = (2560, 1440);
        let (sx, sy, cw, ch) = crop_scoreboard_rect(w, h);
        let mut img = noise_frame(w, h).to_rgb8();
        let white = Rgb([255, 255, 255]);
        let mut paint = |x0: u32, x1: u32, y0: u32, y1: u32| {
            for y in frac(ch, y0)..frac(ch, y1) {
                for x in frac(cw, x0)..frac(cw, x1) {
                    img.put_pixel(sx + x, sy + y, white);
                }
            }
        };
        paint(364, 631, 791, 831);
        paint(600, 700, 500, 540);
        paint(950, 1000, 0, 60);
        paint(0, PORTRAIT_START, 100, 400);
        let out = blanked_scoreboard(&DynamicImage::ImageRgb8(img), TeamLayout::Six, Some(0));
        for (x0, x1, y0, y1) in [
            (364, 631, 791, 831),
            (600, 700, 500, 540),
            (950, 1000, 0, 60),
            (0, PORTRAIT_START, 100, 400),
        ] {
            for y in frac(ch, y0)..frac(ch, y1) {
                for x in frac(cw, x0)..frac(cw, x1) {
                    assert_eq!(
                        *out.get_pixel(x, y),
                        FILL,
                        "px {x},{y} in ({x0},{x1},{y0},{y1})"
                    );
                }
            }
        }
        // Every kept cell starts at the role bar or right of it, and a row
        // that touches the band keeps nothing.
        let band = notice_band(cw, ch);
        for r in kept_rects(cw, ch, TeamLayout::Six, Some(0)) {
            assert!(r.x >= frac(cw, PORTRAIT_START), "{r:?}");
            assert!(!r.intersects(&band), "{r:?}");
        }
    }

    /// Alpha-blend `top` over `under` with `alpha` in 0..=255.
    fn blend(under: Rgb<u8>, top: Rgb<u8>, alpha: u16) -> Rgb<u8> {
        let mix =
            |u: u8, t: u8| ((u16::from(t) * alpha + u16::from(u) * (255 - alpha)) / 255) as u8;
        Rgb([
            mix(under.0[0], top.0[0]),
            mix(under.0[1], top.0[1]),
            mix(under.0[2], top.0[2]),
        ])
    }

    #[test]
    fn world_text_behind_a_translucent_row_is_flattened_and_digits_stay() {
        // QC1-7: a nameplate (red, white) and a white world label behind the
        // purple team-1 and yellow team-2 rows, plus a white digit and a dim
        // zero drawn on top. Each case is the member's own row.
        let (cw, ch) = (1664, 1007);
        let purple = Rgb([110, 20, 120]);
        let yellow = Rgb([200, 160, 20]);
        let digits_from = frac(cw, NAME_COLUMNS_6V6.digits_start);
        let world = [
            Rgb([255, 255, 255]),
            Rgb([255, 60, 60]),
            Rgb([200, 200, 200]),
        ];
        for (own, tint) in [(0usize, purple), (6, yellow)] {
            let mut src = RgbImage::from_pixel(cw, ch, Rgb([40, 40, 40]));
            let cell = *kept_rects(cw, ch, TeamLayout::Six, Some(own))
                .iter()
                .find(|r| r.x >= digits_from)
                .expect("own row digit cell");
            for y in cell.y..cell.y + cell.h {
                for x in cell.x..cell.x + cell.w {
                    let behind = if x < cell.x + cell.w / 2 {
                        world[(x as usize) % 3]
                    } else {
                        Rgb([40, 40, 40])
                    };
                    src.put_pixel(x, y, blend(behind, tint, 180));
                }
            }
            for y in cell.y + 5..cell.y + 15 {
                for x in cell.x + cell.w - 20..cell.x + cell.w - 10 {
                    src.put_pixel(x, y, Rgb([255, 255, 255]));
                }
                for x in cell.x + cell.w - 40..cell.x + cell.w - 30 {
                    src.put_pixel(x, y, Rgb([172, 170, 172]));
                }
            }
            let out = allowlist_crop(&src, TeamLayout::Six, Some(own));
            for y in cell.y..cell.y + cell.h {
                for x in cell.x..cell.x + cell.w / 2 {
                    assert_eq!(*out.get_pixel(x, y), FILL, "world text kept at {x},{y}");
                }
            }
            assert_eq!(
                *out.get_pixel(cell.x + cell.w - 15, cell.y + 10),
                Rgb([255, 255, 255])
            );
            assert_eq!(
                *out.get_pixel(cell.x + cell.w - 35, cell.y + 10),
                Rgb([172, 170, 172])
            );
        }
    }

    #[test]
    fn only_the_own_row_keeps_digit_cells() {
        // QC1-8: other rows keep their portrait only, whatever the colours.
        let (cw, ch) = (1664, 1007);
        let digits_from = frac(cw, NAME_COLUMNS_6V6.digits_start);
        for layout in [TeamLayout::Five, TeamLayout::Six] {
            let n = if layout == TeamLayout::Six { 12 } else { 10 };
            for own in 0..n {
                let digits: Vec<_> = kept_rects(cw, ch, layout, Some(own))
                    .into_iter()
                    .filter(|r| r.x >= digits_from)
                    .collect();
                assert!(digits.len() <= 1, "{layout:?} own {own}: {digits:?}");
            }
            let none: Vec<_> = kept_rects(cw, ch, layout, None)
                .into_iter()
                .filter(|r| r.x >= digits_from)
                .collect();
            assert!(
                none.is_empty(),
                "{layout:?} without an own row kept {none:?}"
            );
        }
        assert!(
            kept_rects(cw, ch, TeamLayout::Unknown, Some(0))
                .iter()
                .all(|r| r.x < digits_from),
            "unknown team size kept a digit cell"
        );
        // A white nameplate behind a grey row on ANOTHER row is filled even
        // though the glyph and tint tests would pass it (the QC1-8 forge).
        let grey = Rgb([90, 90, 92]);
        let mut src = RgbImage::from_pixel(cw, ch, Rgb([60, 120, 220]));
        let other = protected_rects(cw, ch, 6)[2 * 3 + 1];
        for y in other.y + 2..other.y + 12 {
            for x in other.x + 10..other.x + 80 {
                src.put_pixel(x, y, Rgb([235, 235, 235]));
            }
        }
        let _ = grey;
        let out = allowlist_crop(&src, TeamLayout::Six, Some(0));
        for y in other.y..other.y + other.h {
            for x in other.x..other.x + other.w {
                assert_eq!(*out.get_pixel(x, y), FILL, "{x},{y}");
            }
        }
    }

    #[test]
    fn a_neutral_row_tint_keeps_no_digit_cell() {
        // R236-F: white world text behind a grey row stays neutral and bright,
        // so the glyph test alone would keep it. The cell is dropped instead.
        let (cw, ch) = (1664, 1007);
        let grey = Rgb([90, 90, 92]);
        let mut src = RgbImage::from_pixel(cw, ch, grey);
        let digits_from = frac(cw, NAME_COLUMNS_6V6.digits_start);
        let cell = *kept_rects(cw, ch, TeamLayout::Six, Some(0))
            .iter()
            .find(|r| r.x >= digits_from)
            .unwrap();
        for y in cell.y + 2..cell.y + 12 {
            for x in cell.x + 10..cell.x + 60 {
                src.put_pixel(x, y, blend(Rgb([255, 255, 255]), grey, 60));
            }
        }
        assert!(!row_tint_is_coloured(&src, cell));
        let out = allowlist_crop(&src, TeamLayout::Six, Some(0));
        for y in cell.y..cell.y + cell.h {
            for x in cell.x..cell.x + cell.w {
                assert_eq!(*out.get_pixel(x, y), FILL, "{x},{y}");
            }
        }
        // The member's real row colours pass.
        for tint in [Rgb([55, 20, 55]), Rgb([64, 31, 50]), Rgb([200, 160, 20])] {
            let img = RgbImage::from_pixel(cw, ch, tint);
            assert!(row_tint_is_coloured(&img, cell), "{tint:?}");
        }
    }

    #[test]
    fn unknown_team_size_keeps_nothing_in_either_name_column() {
        for (w, h) in SIZES {
            let (_, _, cw, ch) = crop_scoreboard_rect(w, h);
            let kept = kept_rects(cw, ch, TeamLayout::Unknown, Some(0));
            for layout in [TeamLayout::Five, TeamLayout::Six] {
                for n in name_rects(cw, ch, layout) {
                    assert!(kept.iter().all(|k| !k.intersects(&n)), "{w}x{h} {n:?}");
                }
            }
        }
    }

    #[test]
    fn name_columns_cover_every_row_from_the_first_to_the_last() {
        // Rows are contiguous inside each team: a name that spans a row
        // boundary is still covered.
        for (w, h) in SIZES {
            let (_, _, cw, ch) = crop_scoreboard_rect(w, h);
            for (ts, layout) in [(5usize, TeamLayout::Five), (6, TeamLayout::Six)] {
                let names = name_rects(cw, ch, layout);
                for team in names.chunks(ts) {
                    for pair in team.windows(2) {
                        assert_eq!(pair[0].y + pair[0].h, pair[1].y, "{w}x{h} {ts}");
                    }
                }
            }
        }
    }

    #[test]
    fn unknown_team_size_fills_both_layouts() {
        let (w, h) = (1920, 1080);
        let frame = noise_frame(w, h);
        let (_, _, cw, ch) = crop_scoreboard_rect(w, h);
        let out = blanked_scoreboard(&frame, TeamLayout::Unknown, Some(0));
        for layout in [TeamLayout::Five, TeamLayout::Six] {
            for n in name_rects(cw, ch, layout) {
                assert_eq!(*out.get_pixel(n.x, n.y), FILL);
                assert_eq!(*out.get_pixel(n.x + n.w - 1, n.y + n.h - 1), FILL);
            }
        }
        assert_eq!(TeamLayout::Unknown.team_size(), None);
        assert_eq!(TeamLayout::from_team_size(Some(7)), None);
    }

    #[test]
    fn frame_regions_and_bands_are_filled() {
        // Ultrawide: pillarbox bands plus the playfield regions.
        let (w, h) = (3440, 1440);
        let frame = noise_frame(w, h);
        let out = blank_scoreboard_frame(&frame, TeamLayout::Five);
        let (gx, _, gw, _) = game_rect_16_9(w, h);
        assert_eq!(*out.get_pixel(0, h / 2), FILL);
        assert_eq!(*out.get_pixel(gx - 1, h / 2), FILL);
        assert_eq!(*out.get_pixel(gx + gw, h / 2), FILL);
        assert_eq!(*out.get_pixel(gx + gw / 2, 0), FILL); // overlay_top
        assert_eq!(*out.get_pixel(gx + gw - 1, h * 100 / 1000), FILL); // kill_feed
        assert_eq!(*out.get_pixel(gx + 1, h - 1), FILL); // chat_below / overlay_bottom
        // The middle of the playfield, outside every region, is untouched.
        assert_ne!(*out.get_pixel(gx + gw / 2, h / 2 - h / 20), FILL);
    }

    #[test]
    fn regions_ending_at_the_edge_reach_the_last_pixel() {
        for (w, h) in SIZES {
            let frame = noise_frame(w, h);
            let out = blank_scoreboard_frame(&frame, TeamLayout::Five);
            for x in 0..w {
                assert_eq!(*out.get_pixel(x, h - 1), FILL, "{w}x{h} bottom row x={x}");
                assert_eq!(*out.get_pixel(x, 0), FILL, "{w}x{h} top row x={x}");
            }
        }
    }

    /// QA harness on real frames, not in CI: `SCUFFED_QA_FRAMES=<dir of full
    /// frames> SCUFFED_QA_OUT=<private dir> cargo test -p scuffed-stat-tracker
    /// --lib real_frames_blank -- --ignored`. Writes each frame's blanked
    /// scoreboard (both team-size layouts) for a person to look at.
    #[test]
    #[ignore]
    fn real_frames_blank_contact_sheet() {
        let src = std::env::var("SCUFFED_QA_FRAMES").expect("set SCUFFED_QA_FRAMES");
        let out =
            std::path::PathBuf::from(std::env::var("SCUFFED_QA_OUT").expect("set SCUFFED_QA_OUT"));
        std::fs::create_dir_all(&out).unwrap();
        for entry in std::fs::read_dir(src).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "png") {
                continue;
            }
            let Ok(img) = image::open(&path) else {
                continue;
            };
            let stem = path.file_stem().unwrap().to_string_lossy().to_string();
            let own_row = std::env::var("SCUFFED_QA_OWN_ROW")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(Some(0));
            // A scoreboard crop (debug/accepted) is about 1.65:1; a frame is wider.
            let ratio = img.width() as f64 / img.height().max(1) as f64;
            let is_crop = (ratio - 1.651).abs() < 0.02;
            for (tag, layout) in [("5", TeamLayout::Five), ("6", TeamLayout::Six)] {
                let blanked = if is_crop {
                    allowlist_crop(&img.to_rgb8(), layout, own_row)
                } else {
                    blanked_scoreboard(&img, layout, own_row)
                };
                blanked
                    .save(out.join(format!("{stem}_{tag}v{tag}.png")))
                    .unwrap();
            }
        }
    }

    #[test]
    fn outcome_box_is_clear_of_every_outcome_class_fill() {
        // The spec says blanking never paints the word box on a rank frame.
        for (w, h) in SIZES {
            let frame = noise_frame(w, h);
            let pf = game_rect_16_9(w, h);
            let word = playfield_rect(&OUTCOME_BAND, pf);
            for reg in SCOREBOARD_REGIONS
                .iter()
                .filter(|r| r.id.starts_with("overlay_"))
            {
                assert!(
                    !playfield_rect(reg, pf).intersects(&word),
                    "{w}x{h} {}",
                    reg.id
                );
            }
            let band = outcome_band(&frame);
            let orig = frame.to_rgb8();
            for y in 0..band.height() {
                for x in 0..band.width() {
                    assert_eq!(band.get_pixel(x, y), orig.get_pixel(word.x + x, word.y + y));
                }
            }
            // The overlay bars are filled on the blanked rank frame.
            assert_eq!(*blank_outcome_frame(&frame).get_pixel(pf.0 + 1, pf.1), FILL);
        }
    }

    #[test]
    fn outcome_band_is_the_detector_band() {
        let frame = noise_frame(2560, 1440);
        let band = outcome_band(&frame);
        assert_eq!(band.dimensions(), (615, 116));
        assert_eq!(band.get_pixel(0, 0), frame.to_rgb8().get_pixel(25, 208));
    }
}
