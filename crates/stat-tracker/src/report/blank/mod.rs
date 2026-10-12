//! Name blanking for game reports (`docs/report-bundle.md` section 4).
//!
//! `crops/scoreboard.png` is an allowlist. It starts as [`FILL`], and only
//! two kinds of pixel are copied from the frame, each from a structure
//! measured on this board ([`board`]) and checked by an independent cue:
//! * the portrait cell of a row (role bar and hero portrait), when its fine
//!   detail matches the same row on an earlier capture of the game
//!   ([`PortraitMemory`]), nothing is drawn over its bar, out of it to either
//!   side, across its edge or over the separators above and below it, and no
//!   chat can be over it ([`portrait`], [`chat`]);
//! * the digit glyphs of a row whose six cells the reader copy reads as
//!   well-formed numbers and every glyph gate accepts, on one flat colour per
//!   cell that equals no source pixel ([`cells`]).
//!
//! Names, titles, badges, the header, the hero card, notices, nameplates and
//! the world all stay fill. A measurement that fails keeps nothing of the
//! board, the row, or the portrait it belongs to. Every kept row is read back
//! from the output before it is accepted.
//!
//! The frame is never written: the crop is cut and blanked in memory before
//! any byte reaches the report kit.

mod board;
mod cells;
mod chat;
mod letters;
#[cfg(test)]
mod port_check;
mod portrait;
mod reader;
#[cfg(test)]
pub(crate) mod synth;
#[cfg(test)]
mod tests_measured;

pub use chat::{Chat, chat_over_board};

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use image::{DynamicImage, Rgb, RgbImage};

use crate::ocr::preprocess::{crop_scoreboard_rect, game_rect_16_9};

/// The one fill colour (section 4).
pub const FILL: Rgb<u8> = Rgb([32, 32, 32]);

/// Bumped whenever what the blanker keeps can change. Kits store it, and a
/// kit written by another version is never reported.
///
/// * 9: measured geometry, glyph gates on the reader copy, portrait gates,
///   chat over the board (2026-10-12 redesign).
/// * 10: a portrait is copied only when it matches the same row on an
///   earlier capture of the game; a role icon and the separators over the
///   portrait must show; portraits only at the 1440p and 1080p scales.
pub const BLANK_TABLE_VERSION: u32 = 10;

/// An earlier capture is a reference for the portrait check only when it is
/// at least this old: an overlay that shows on both captures has to stay
/// pixel-identical for this long.
pub const REF_MIN_AGE: chrono::Duration = chrono::Duration::seconds(60);
/// ... and at most this old (one game).
pub const REF_MAX_AGE: chrono::Duration = chrono::Duration::minutes(30);
/// Captures a [`PortraitMemory`] keeps.
const MEMORY_BOARDS: usize = 8;

/// Portrait cells of the earlier captures of one game, kept in memory only
/// (never written) as references for the portrait content check. Each cell
/// is the unblanked `[L, pe)` cell of a measured row, so the memory lives as
/// long as the daemon holds the game and no longer.
#[derive(Default)]
pub struct PortraitMemory {
    seen: VecDeque<Seen>,
}

struct Seen {
    at: DateTime<Utc>,
    crop_w: u32,
    /// `(team, idx, cell)`.
    cells: Vec<(usize, usize, RgbImage)>,
}

impl PortraitMemory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cells of the same row, of the same size, captured between
    /// [`REF_MIN_AGE`] and [`REF_MAX_AGE`] before `at`.
    fn references<'a>(
        &'a self,
        at: DateTime<Utc>,
        crop_w: u32,
        team: usize,
        idx: usize,
        dims: (u32, u32),
    ) -> impl Iterator<Item = &'a RgbImage> + 'a {
        self.seen
            .iter()
            .filter(move |b| {
                b.crop_w == crop_w && at - b.at >= REF_MIN_AGE && at - b.at <= REF_MAX_AGE
            })
            .flat_map(|b| b.cells.iter())
            .filter(move |(t, i, c)| *t == team && *i == idx && c.dimensions() == dims)
            .map(|(_, _, c)| c)
    }

    fn remember(&mut self, seen: Seen) {
        self.seen.retain(|b| (seen.at - b.at).abs() <= REF_MAX_AGE);
        self.seen.push_back(seen);
        while self.seen.len() > MEMORY_BOARDS {
            self.seen.pop_front();
        }
    }
}

/// A region of the 16:9 playfield, in thousandths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayfieldRegion {
    pub id: &'static str,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Bars filled on a rank screen frame before the outcome box is cut.
pub const OVERLAY_REGIONS: [PlayfieldRegion; 2] = [
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

/// The team size the capture path reported, for the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamLayout {
    Five,
    Six,
    /// The app could not tell 5v5 from 6v6 (`team_size: null`).
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

/// A playfield region in frame pixels. Both edges are scaled from the
/// thousandths and truncated, so a region ending at 1000 reaches the last
/// pixel.
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

/// What the `outcome` class fills on a rank screen frame: the overlay bars
/// and the bands outside the playfield.
pub fn outcome_regions(frame_w: u32, frame_h: u32) -> Vec<Rect> {
    let pf = game_rect_16_9(frame_w, frame_h);
    let mut out: Vec<Rect> = OVERLAY_REGIONS
        .iter()
        .map(|r| playfield_rect(r, pf).clipped(frame_w, frame_h))
        .collect();
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

/// What happened to one row of the measured grid.
#[derive(Debug, Clone, PartialEq)]
pub struct RowVerdict {
    /// 0 for team 1 (top), 1 for team 2.
    pub team: usize,
    pub idx: usize,
    pub digits: Result<(), String>,
    pub portrait: Result<(), String>,
}

/// One blanked scoreboard and what it kept.
#[derive(Debug, Clone)]
pub struct BlankedBoard {
    pub image: RgbImage,
    /// Why the whole board is fill, when it is.
    pub board: Result<(), String>,
    pub rows: Vec<RowVerdict>,
}

impl BlankedBoard {
    fn nothing(image: RgbImage, why: impl Into<String>) -> Self {
        BlankedBoard {
            image,
            board: Err(why.into()),
            rows: Vec::new(),
        }
    }

    /// Rows whose numbers were kept.
    pub fn digit_rows(&self) -> usize {
        self.rows.iter().filter(|r| r.digits.is_ok()).count()
    }

    /// Rows whose portrait cell was kept.
    pub fn portraits(&self) -> usize {
        self.rows.iter().filter(|r| r.portrait.is_ok()).count()
    }

    /// Anything besides fill: a board worth sending.
    pub fn kept_anything(&self) -> bool {
        self.digit_rows() + self.portraits() > 0
    }
}

/// `crops/scoreboard.png` from a full frame with no earlier capture: no
/// portrait is kept.
pub fn blanked_scoreboard(frame: &DynamicImage, team_size: Option<usize>) -> BlankedBoard {
    blanked_scoreboard_seen(
        frame,
        team_size,
        &mut PortraitMemory::new(),
        DateTime::<Utc>::UNIX_EPOCH,
    )
}

/// `crops/scoreboard.png` from a full frame captured at `at`: the
/// scoreboard crop, blanked, with chat read from the rest of the frame and
/// portraits checked against the game's earlier captures in `memory`, which
/// then remembers this one.
pub fn blanked_scoreboard_seen(
    frame: &DynamicImage,
    team_size: Option<usize>,
    memory: &mut PortraitMemory,
    at: DateTime<Utc>,
) -> BlankedBoard {
    let rgb = frame.to_rgb8();
    let (x, y, w, h) = crop_scoreboard_rect(rgb.width(), rgb.height());
    let crop = image::imageops::crop_imm(&rgb, x, y, w, h).to_image();
    blank_crop_seen(&crop, team_size, chat_over_board(&rgb), memory, at)
}

/// The allowlist on a cut scoreboard crop with no earlier capture: no
/// portrait is kept.
pub fn blank_crop(src: &RgbImage, team_size: Option<usize>, chat: Chat) -> BlankedBoard {
    blank_crop_seen(
        src,
        team_size,
        chat,
        &mut PortraitMemory::new(),
        DateTime::<Utc>::UNIX_EPOCH,
    )
}

/// The allowlist on a cut scoreboard crop captured at `at`. `chat` is what
/// the full frame showed; with [`Chat::Unknown`] every row the chat box could
/// reach keeps no portrait. `memory` holds the game's earlier captures and
/// remembers this one's portrait cells.
pub fn blank_crop_seen(
    src: &RgbImage,
    team_size: Option<usize>,
    chat: Chat,
    memory: &mut PortraitMemory,
    at: DateTime<Utc>,
) -> BlankedBoard {
    let (w, h) = src.dimensions();
    let mut out = RgbImage::from_pixel(w, h, FILL);
    let Some(team) = team_size else {
        return BlankedBoard::nothing(out, "team size unknown");
    };
    let b = match board::measure(src, team) {
        Ok(b) => b,
        Err(e) => return BlankedBoard::nothing(out, e.0),
    };
    let s = b.s;
    let bars: Vec<Option<Result<[f64; 3], &'static str>>> = b
        .rows
        .iter()
        .map(|r| r.verified.then(|| portrait::bar_colour(src, b.l, r, s)))
        .collect();
    // Team tint: the row's own opaque bar, or (when something covers it)
    // the weakest measured bar of its team, 2+ needed; otherwise none.
    let tint: Vec<Option<f64>> = b
        .rows
        .iter()
        .zip(&bars)
        .map(|(r, bar)| match bar {
            None => None,
            Some(Ok(c)) => Some(portrait::sat(*c)),
            Some(Err(_)) => {
                let sats: Vec<f64> = b
                    .rows
                    .iter()
                    .zip(&bars)
                    .filter(|(o, _)| o.team == r.team)
                    .filter_map(|(_, ob)| match ob {
                        Some(Ok(c)) => Some(portrait::sat(*c)),
                        _ => None,
                    })
                    .collect();
                Some(if sats.len() >= 2 {
                    sats.iter().copied().fold(f64::INFINITY, f64::min)
                } else {
                    0.0
                })
            }
        })
        .collect();
    let with_bar: Vec<&board::Row> = b
        .rows
        .iter()
        .zip(&bars)
        .filter(|(_, c)| matches!(c, Some(Ok(_))))
        .map(|(r, _)| r)
        .collect();
    let pe = portrait::portrait_edge(src, b.l, &with_bar, s);
    let sal_x0 = b.l.saturating_sub(3);
    let sal_x1 = (i64::from(b.l) + board::pyround(36.0 * s)).min(i64::from(w)) as u32;
    let sal = portrait::salient_map(src, sal_x0, sal_x1);
    let sal_w = sal_x1.saturating_sub(sal_x0) as usize;
    let digits = cells::digit_rows(src, &b, &tint);
    let native = cells::native_scale(s);
    let mut rows = Vec::with_capacity(b.rows.len());
    let mut painted: Vec<(usize, cells::KeptRow)> = Vec::new();
    let mut seen = Seen {
        at,
        crop_w: w,
        cells: Vec::new(),
    };
    for (ri, ((r, bar), dig)) in b.rows.iter().zip(&bars).zip(digits).enumerate() {
        // 1 px (x s) in from the fitted row edges: the grid is fitted to
        // +-1 px, so the cell never takes a separator or the next row
        let cell = match (bar, &pe) {
            (Some(Ok(_)), Ok(pe)) => portrait::cell_rect(src, b.l, *pe, r, s).map(|c| {
                (
                    c,
                    image::imageops::crop_imm(src, c.x, c.y, c.w, c.h).to_image(),
                )
            }),
            _ => None,
        };
        let portrait = match (bar, &pe, &cell) {
            (None, _, _) => Err("row not verified".to_string()),
            (Some(Err(e)), _, _) => Err(format!("no portrait: {e}")),
            (_, Err(e), _) => Err(format!("no portrait edge: {}", e.0)),
            (_, _, None) => Err("portrait cell outside the crop".to_string()),
            (Some(Ok(_)), Ok(_), Some(_)) if !native => {
                Err(format!("scale {s:.3} has no measured portraits"))
            }
            (Some(Ok(c)), Ok(pe), Some((_, img))) => {
                portrait::verdict(src, &sal, sal_x0, sal_w, b.l, r, s)
                    .and_then(|()| portrait::surroundings(src, b.l, *pe, r, s, *c))
                    .and_then(|()| portrait::separators(src, b.l, *pe, r, s))
                    .map_err(str::to_string)
                    .and_then(|()| {
                        if chat.covers(r.bot, h) {
                            Err("chat may cover it".to_string())
                        } else {
                            Ok(())
                        }
                    })
                    .and_then(|()| {
                        let mut refs = memory
                            .references(at, w, r.team, r.idx, img.dimensions())
                            .peekable();
                        if refs.peek().is_none() {
                            return Err("no earlier capture of this row".to_string());
                        }
                        let best = refs
                            .map(|e| portrait::residual(img, e, s))
                            .min()
                            .unwrap_or(usize::MAX);
                        if portrait::residual_ok(best, s) {
                            Ok(())
                        } else {
                            Err(format!("differs from the earlier capture ({best} px)"))
                        }
                    })
            }
        };
        if let Some((c, img)) = cell {
            if portrait.is_ok() {
                for y in 0..c.h {
                    for x in 0..c.w {
                        out.put_pixel(c.x + x, c.y + y, *img.get_pixel(x, y));
                    }
                }
            }
            seen.cells.push((r.team, r.idx, img));
        }
        let digits = match dig {
            Err(why) => Err(why),
            Ok(kept) => {
                for c in &kept.cells {
                    for y in r.top..r.bot.min(h) {
                        for x in c.x0..c.x1 {
                            out.put_pixel(x, y, Rgb(c.paint));
                        }
                    }
                    for &(x, y) in &c.pixels {
                        out.put_pixel(x, y, *src.get_pixel(x, y));
                    }
                }
                painted.push((ri, kept));
                Ok(())
            }
        };
        rows.push(RowVerdict {
            team: r.team,
            idx: r.idx,
            digits,
            portrait,
        });
    }
    // Read every kept row back from the output; a row that reads otherwise
    // keeps nothing. The digit bands lie inside their rows, so one row's
    // repaint cannot change another row's read.
    if !painted.is_empty() {
        let n = cells::output_ink(&out);
        for (ri, kept) in &painted {
            let r = &b.rows[*ri];
            if !cells::reads_back(&n, &b, r, kept) {
                for c in &kept.cells {
                    for y in r.top..r.bot.min(h) {
                        for x in c.x0..c.x1 {
                            out.put_pixel(x, y, FILL);
                        }
                    }
                }
                rows[*ri].digits = Err("kept cells do not read back".to_string());
            }
        }
    }
    memory.remember(seen);
    BlankedBoard {
        image: out,
        board: Ok(()),
        rows,
    }
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
    fn a_frame_that_is_no_scoreboard_keeps_nothing() {
        for (w, h) in SIZES {
            let b = blanked_scoreboard(&noise_frame(w, h), Some(6));
            assert!(b.board.is_err(), "{w}x{h}");
            assert!(!b.kept_anything());
            assert!(b.image.pixels().all(|p| *p == FILL), "{w}x{h}");
        }
    }

    #[test]
    fn the_spec_describes_this_blanker() {
        // section 4 of the spec states these numbers; change both together
        let doc = include_str!("../../../docs/report-bundle.md");
        for line in [
            "RGB 32, 32, 32",
            "(`BLANK_TABLE_VERSION`, 10 for this revision)",
            "a channel more than 30 from its 5x5 median",
            "at most 0.45 of the text height in ink",
            "x 60 to 448, y 1250 to 1340",
            "the portrait's right edge shows on at least 40% of the row's pixel rows",
            "at least a minute earlier",
            "more than 20 at more than 20 px",
        ] {
            assert!(doc.contains(line), "spec line missing: {line}");
        }
        assert_eq!(BLANK_TABLE_VERSION, 10);
        assert_eq!(FILL, Rgb([32, 32, 32]));
    }

    #[test]
    fn team_layouts() {
        assert_eq!(TeamLayout::Unknown.team_size(), None);
        assert_eq!(TeamLayout::from_team_size(Some(7)), None);
        assert_eq!(TeamLayout::from_team_size(Some(6)), Some(TeamLayout::Six));
    }

    #[test]
    fn outcome_box_is_clear_of_every_outcome_class_fill() {
        // The spec says blanking never paints the word box on a rank frame.
        for (w, h) in SIZES {
            let frame = noise_frame(w, h);
            let pf = game_rect_16_9(w, h);
            let word = playfield_rect(&OUTCOME_BAND, pf);
            for reg in &OVERLAY_REGIONS {
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
