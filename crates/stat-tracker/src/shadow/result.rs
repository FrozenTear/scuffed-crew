//! VICTORY / DEFEAT / DRAW from result frames (shadow only).
//!
//! The regions are ocr-v1's (`detect::match_end`), as fractions of the
//! centred 16:9 game rect, one [`Layout`] each, tagged with a source id:
//!
//! * `accolade`: post-match accolade screen, player cards; region x 0.5-25.5%, y 3.5-8.62%;
//!   plane `max`; real frames: 52 (1440p). ocr-v1's crop runs to y 9.5%, but
//!   the member's nameplate under the result word starts at y 8.7%, so this
//!   region stops above it ([`NAME_LINE_TOP`]). The map and match time right
//!   of the word are in the region; no name is.
//! * `rank_screen`: competitive rank screen (incl. defeat); region x 1-25%, y 14.5-22.5%;
//!   plane `min`; real frames: 98 (1440p).
//! * `end_title`: centred italic end title; region x 32-68%, y 34-52%;
//!   plane `max`. The words-v1 pack widens it with its `layouts.json` (the
//!   current title sits lower, y 43-56.5%) and adds a second layout,
//!   `end_title_plain`. Its templates are rendered from the game font, so
//!   every real frame is held out. Under `reader = "new"` the poller reads
//!   the end-title layouts on their own ([`ResultTemplates::end_titles`],
//!   `detect::match_end`).
//!
//! The `accolade` and `rank_screen` layouts are the poller's two result
//! screens ([`ResultScreen`]). Under `reader = "new"` a sure read of the
//! whole frame whose word was found on one of them is that screen's outcome
//! signal ([`ScreenRead`], `detect::match_end`), in place of ocr-v1's OCR of
//! the same crop. The frame is read in full only after a first look at
//! those two layouts alone found a sure word
//! ([`LoadedResults::read_screens`]).
//! * `tab_header`: post-match Tab board header; region x 30-70%, y 2-24%;
//!   plane `min`; real frames: none.
//!
//! The plane makes the reader colour-blind: `max` (brightest channel) reads
//! a team-coloured word the same in any bright colour, `min` (darkest
//! channel) keeps only white text on a coloured background. The region is
//! scaled to a 720p work size and searched with zero-mean normalised
//! correlation for each word template at three scales. A read names a word
//! when the best template scores at least [`PRESENT`], and is sure when it
//! scores at least [`FLOOR`] and leads the best other word by [`MARGIN`].
//! DRAW has no real frame yet (its words-v1 template is rendered), so a DRAW
//! read that is not sure names no word at all: it abstains instead of
//! flagging the match. This is by design, and it has a cost: with no read,
//! the `result` field is not attempted, so on a real draw that only the
//! rendered template half-matches, ocr-v1's value is stored without a
//! `result` suspect mark (it was marked before).
//!
//! The search is coarse to fine ([`ResultTemplates::read`]): each template
//! is matched at every position of a 2x or 4x box-downscaled region, the
//! best few peaks are refined one pyramid level at a time in a small window,
//! and the score is the full-resolution correlation at the refined position.
//! A score is never higher than the exhaustive search's
//! ([`ResultTemplates::read_exhaustive`]) and equals it whenever the
//! exhaustive best position is found, which tests check on synthetic frames.
//! The exhaustive search over the words-v1 end-title region took 8.6 s per
//! frame.
//!
//! That alone does not make a fast read conservative: the margin is taken
//! against the runner-up, and an under-read runner-up widens it. So a read
//! the fast scores call sure is certified: for every template of another
//! word, an exact bound (the block projection plus Cauchy-Schwarz on the
//! rest, [`bound_at`]) rules out every window, and the windows it cannot
//! rule out are scored exactly. Any window at `s1 - MARGIN` or above flags
//! the read. A sure fast read is therefore a sure exhaustive read of the
//! same word; a word the fast search under-reads can only lower confidence.
//!
//! Like ocr-v1, one sure read is not enough: [`ResultEvidence`] collects every
//! read of one match (poll frames, the accolade screen, the rank screen) and
//! the result is sure only with [`MIN_AGREE`] agreeing sure reads and no read
//! of another word. An accolade read is a second look at the same result,
//! never a new event: the reader creates no events, the caller adds each
//! frame of a match to that match's evidence.
//!
//! Templates are game art and are not embedded. They load from the words-v1
//! pack folder `<data_dir>/templates/words-v1/` ([`WORDS_SUBDIR`], installed
//! by pack sync), else from the old manual folder
//! `<data_dir>/templates/result/` ([`TEMPLATE_SUBDIR`]). Files are
//! `<source>_<word>.png` (8-bit grey at the work scale, `word` one of
//! `victory`, `defeat`, `draw`). A layout without templates reads nothing.
//! Further screens plug in through an optional `layouts.json` in the same
//! directory (see [`ResultTemplates::load_dir`]). [`LiveResults`] re-loads
//! the templates when either folder changes, the way the hero packs do.
//! Only `accolade` and `rank_screen` have real templates so far; DRAW,
//! `tab_header` and 1080p-native accolade frames are untested.
//! No colour flood is read here or anywhere else: the colour-flood banner
//! outcome was removed in 0.5.0-rc.9, and the word reads cover the same
//! screens.
//!
//! The words-v1 pack also ships `p4_final_score.png`, the FINAL SCORE line
//! under the end title ([`FinalScoreTemplate`], region x 36-64%, y 70-84%,
//! plane `min`). It is not a result word: it opens the gate of the
//! end-title OCR in `detect::match_end` in place of ocr-v1's P4 OCR, with
//! the same search and certification against [`FINAL_SCORE_FLOOR`]. Since
//! alpha.7 O2 `reader = "new"` runs that OCR, and so this read, only while
//! no end-title templates are loaded; with them loaded the end title is
//! the template read above and nothing reads this line.
//!
//! The words-v1 folder also holds the phase-screen titles ([`phrase`]),
//! loaded with the result words and read with the same search.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::RgbImage;
use serde::{Deserialize, Serialize};

use super::banner::Gray;
use super::heroes::{DirStamp, dir_stamp};
use crate::ocr::preprocess::game_rect_16_9;

pub mod phrase;

/// Identifies this recognizer's output. Bump on any change that can move a read.
/// Nothing reads it yet: no snapshot, manifest or log carries it, so a field
/// report cannot tell a result-v2 read from a result-v3 one.
pub const RESULT_RECOGNIZER_ID: &str = "result-v3";
/// The old manual template folder under the data dir (the fallback).
pub const TEMPLATE_SUBDIR: &str = "templates/result";
/// The words-v1 pack folder under the data dir (preferred).
pub const WORDS_SUBDIR: &str = "templates/words-v1";
/// Below this, no result word is on the frame.
pub const PRESENT: f32 = 0.50;
/// Minimum score for a sure read.
pub const FLOOR: f32 = 0.70;
/// Minimum lead over the best other word for a sure read.
pub const MARGIN: f32 = 0.12;
/// Agreeing sure reads a match needs before its result is sure (ocr-v1 rule).
pub const MIN_AGREE: usize = 2;
/// Top of the accolade screen's nameplate under the result word, as a
/// fraction of the game rect height. Measured on the 13 real 1440p accolade
/// frames the new reader reads: the word's glyphs end by 8.1% (its template
/// by 8.4%), the nameplate's icon starts at 8.7% to 8.9% and its text at
/// about 9.2%.
pub const NAME_LINE_TOP: f64 = 0.0865;
/// Bottom of the accolade region, above [`NAME_LINE_TOP`]. 74 rows at
/// 1440p, so the 2:1 resample to the work scale is exact. At 1080p and
/// below the gap to the nameplate is under one pixel (row 93 against about
/// 93.96 at 1080p), from u32 truncation; native 1080p accolade frames are
/// unmeasured.
pub const ACCOLADE_Y1: f64 = 0.0862;
/// The accolade nameplate box (x0, y0, x1, y1): its icon and text under the
/// result word, across the width of the accolade region. No layout's region
/// may reach into it ([`ResultTemplates::load_dir`]).
pub const NAMEPLATE: [f64; 4] = [0.0, NAME_LINE_TOP, 0.255, 0.115];
// The accolade region ends above the nameplate (checked at compile time).
const _: () = assert!(ACCOLADE_Y1 < NAME_LINE_TOP && NAMEPLATE[1] == NAME_LINE_TOP);
/// Work scale relative to a 1440p game rect.
const WORK: f64 = 0.5;
const SCALES: [f64; 3] = [0.96, 1.0, 1.04];
/// The coarse search level keeps at least this many template pixels on the
/// template's short side.
const MIN_COARSE_PX: usize = 12;
/// Deepest pyramid level: 4x downscaled.
const MAX_LEVEL: usize = 2;
/// A coarser level is used only when the template keeps at least this
/// share of its per-pixel variance there. Text strokes survive a 4x box
/// filter (the words-v1 templates keep 0.84 to 0.97); fine stripes average
/// out, and a coarse search over them would rank the wrong peaks.
const MIN_KEPT: f64 = 0.8;
/// Coarse peaks refined per template and scale.
const PEAKS: usize = 12;
/// Coarse peaks closer than this (coarse pixels, either axis) are one peak.
const PEAK_SEP: usize = 1;
/// Refine window half-width on the first level below the coarse one, then
/// on every later level (pixels of that level).
const REFINE_FIRST: usize = 3;
const REFINE: usize = 2;

/// Layout id and file stem of the FINAL SCORE line template (P4).
pub const FINAL_SCORE: &str = "p4_final_score";
/// Minimum score for a sure FINAL SCORE read. The words-v1 template scores
/// 0.86 to 0.96 on 12 held-out real FINAL SCORE frames (12 games, 3
/// creators, 1080p and 1440p), 0.90 to 0.93 on the 3 frames it was fit on,
/// and at most 0.39 on 637 frames without the line (2026-10-13).
pub const FINAL_SCORE_FLOOR: f32 = 0.70;

/// Source ids of the built-in layouts.
pub const ACCOLADE: &str = "accolade";
pub const RANK_SCREEN: &str = "rank_screen";
pub const END_TITLE: &str = "end_title";
pub const TAB_HEADER: &str = "tab_header";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Victory,
    Defeat,
    Draw,
}

impl Outcome {
    pub const ALL: [Outcome; 3] = [Outcome::Victory, Outcome::Defeat, Outcome::Draw];

    /// The value ocr-v1 stores (`PersonalMatch.outcome`).
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Victory => "victory",
            Outcome::Defeat => "defeat",
            Outcome::Draw => "draw",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Which channel makes the brightness plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Plane {
    /// Brightest channel: any bright colour reads alike (team-coloured words).
    Max,
    /// Darkest channel: only white or grey survives (white words on colour).
    Min,
}

/// One screen that shows the result word: a region of the 16:9 game rect
/// (fractions x0, y0, x1, y1) and its plane. `id` is the source tag and the
/// template file prefix.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Layout {
    pub id: String,
    pub roi: [f64; 4],
    pub plane: Plane,
}

impl Layout {
    fn new(id: &str, roi: [f64; 4], plane: Plane) -> Self {
        Self {
            id: id.into(),
            roi,
            plane,
        }
    }

    /// The four ocr-v1 regions, the accolade one stopped above the nameplate.
    pub fn builtin() -> Vec<Layout> {
        vec![
            Self::new(ACCOLADE, [0.005, 0.035, 0.255, ACCOLADE_Y1], Plane::Max),
            Self::new(RANK_SCREEN, [0.010, 0.145, 0.250, 0.225], Plane::Min),
            Self::new(END_TITLE, [0.320, 0.340, 0.680, 0.520], Plane::Max),
            Self::new(TAB_HEADER, [0.300, 0.020, 0.700, 0.240], Plane::Min),
        ]
    }

    pub fn by_id(id: &str) -> Option<Layout> {
        Self::builtin().into_iter().find(|l| l.id == id)
    }
}

/// One frame's read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResultRead {
    /// Best word when one is present (also when flagged).
    pub outcome: Option<Outcome>,
    /// Source tag: id of the layout the best word was found in.
    pub source: Option<String>,
    pub score: f32,
    pub margin: f32,
    pub suspect: bool,
}

impl ResultRead {
    /// The word of a sure read: present, at or above [`FLOOR`], and
    /// [`MARGIN`] clear of every other word. `None` for a flagged read and
    /// for a frame that shows no word.
    pub fn sure(&self) -> Option<Outcome> {
        self.outcome.filter(|_| !self.suspect)
    }

    fn nothing(score: f64, margin: f64) -> Self {
        Self {
            outcome: None,
            source: None,
            score: score.max(0.0) as f32,
            margin: margin.max(0.0) as f32,
            suspect: true,
        }
    }
}

// ------------------------------------------------------------------ search

/// A window's variance sum at or below this was flat before [`FLAT_ROUNDING`]
/// and still is: the new rule only adds flat windows.
const FLAT_VAR: f64 = 1e-9;
/// The rounding error of a window's variance sum as [`Level::window`]
/// computes it, per window pixel and per level pixel (the bound is derived
/// there). A window at or under it is flat.
const FLAT_ROUNDING: f64 = 12.0 * f64::EPSILON;

/// One pyramid level of a region: pixels plus integral images (sum and sum
/// of squares) for window statistics. Pixels are in 0..1.
struct Level {
    w: usize,
    h: usize,
    px: Vec<f32>,
    s1: Vec<f64>,
    s2: Vec<f64>,
    /// [`FLAT_ROUNDING`] times this level's pixel count: what the variance
    /// sum of a flat window can be per window pixel ([`Level::window`]).
    flat: f64,
}

impl Level {
    fn new(w: usize, h: usize, px: Vec<f32>) -> Self {
        let mut s1 = vec![0.0f64; (w + 1) * (h + 1)];
        let mut s2 = vec![0.0f64; (w + 1) * (h + 1)];
        for y in 0..h {
            for x in 0..w {
                let v = px[y * w + x] as f64;
                let i = (y + 1) * (w + 1) + x + 1;
                s1[i] = v + s1[i - 1] + s1[i - w - 1] - s1[i - w - 2];
                s2[i] = v * v + s2[i - 1] + s2[i - w - 1] - s2[i - w - 2];
            }
        }
        let flat = FLAT_ROUNDING * (w * h) as f64;
        Self {
            w,
            h,
            px,
            s1,
            s2,
            flat,
        }
    }

    /// Sum of `s` over the `tw` x `th` window at (x, y).
    #[inline]
    fn rect(&self, s: &[f64], x: usize, y: usize, tw: usize, th: usize) -> f64 {
        let a = y * (self.w + 1) + x;
        let b = (y + th) * (self.w + 1) + x;
        s[b + tw] - s[b] - s[a + tw] + s[a]
    }

    /// The pixel sum and the variance sum (`sum((p - mean)^2)`, not divided
    /// by the pixel count) of the `tw` x `th` window at (x, y), or `None`
    /// when the window is flat: nothing in it can match a word.
    ///
    /// A window of one grey value has a variance of 0, but the value
    /// computed here from the f64 integral images is their rounding error,
    /// positive or negative. On a 512 x 230 band of one grey value it
    /// reaches 1e-7 for an end-title window (R3-1). The old test, a fixed
    /// [`FLAT_VAR`], let about a third of those windows through, and
    /// [`ncc_at`] scored them: the rounding error of its f32 dot product
    /// over the square root of that noise. Measured on such bands: up to
    /// 0.38 for a words-v1 end title, 0.63 for a synthetic word of the
    /// tests. So a window is flat when its variance sum is within what that
    /// rounding can be.
    ///
    /// The bound, with `A` this level's pixel count, `n` the window's, and
    /// `u = EPSILON / 2`. An integral-image entry is three rounded additions
    /// of values up to `A`, `2 A` and `A` (pixels are in 0..1, and a square
    /// of an f32 is exact in f64), so it is off by at most `4 u A` from the
    /// same sum of exact entries. The recurrence is linear, so the table is
    /// the exact table of pixels that are each off by that much, and a
    /// window sum, with the three additions of [`Self::rect`], is off by at
    /// most `(4 n + 3) u A`, for the sums and for the squares. The variance
    /// sum `s2 - s1 * s1 / n` is then off by at most that for `s2`, twice
    /// that for `s1` (its error times `2 * mean`, and the mean is at most 1)
    /// and `3 u n` for its own three operations:
    /// `3 (4 n + 3) u A + 3 u n <= 24 u n A = 12 EPSILON n A`
    /// ([`FLAT_ROUNDING`]). A window of one value is therefore always flat.
    ///
    /// Nothing real is that faint. A title one 8-bit grey level off its
    /// background over 2% of the window has a variance sum of `3e-7 n`; the
    /// bound is `3.1e-10 n` on the end-title band (512 x 230) and `2.5e-9 n`
    /// on the largest region there is, the whole 1280 x 720 work rect.
    #[inline]
    fn window(&self, x: usize, y: usize, tw: usize, th: usize) -> Option<(f64, f64)> {
        let n = (tw * th) as f64;
        let sum = self.rect(&self.s1, x, y, tw, th);
        let var = self.rect(&self.s2, x, y, tw, th) - sum * sum / n;
        (var > FLAT_VAR.max(n * self.flat)).then_some((sum, var))
    }
}

/// 2x2 box average, dropping an odd last row or column. Pixel (x, y) of the
/// result covers pixels (2x..2x+2, 2y..2y+2), so a position at one level is
/// twice that position one level down.
fn half(w: usize, h: usize, px: &[f32]) -> (usize, usize, Vec<f32>) {
    let (hw, hh) = (w / 2, h / 2);
    let mut out = Vec::with_capacity(hw * hh);
    for y in 0..hh {
        for x in 0..hw {
            let i = 2 * y * w + 2 * x;
            out.push((px[i] + px[i + 1] + px[i + w] + px[i + w + 1]) * 0.25);
        }
    }
    (hw, hh, out)
}

/// One template at one scale and pyramid level: zero-mean pixels and norm.
#[derive(Debug, Clone)]
struct Kernel {
    w: usize,
    h: usize,
    z: Vec<f32>,
    norm: f64,
}

impl Kernel {
    fn new(w: usize, h: usize, px: &[f32]) -> Option<Self> {
        if w == 0 || h == 0 {
            return None;
        }
        let mean = px.iter().map(|&v| v as f64).sum::<f64>() / (w * h) as f64;
        let z: Vec<f32> = px.iter().map(|&v| (v as f64 - mean) as f32).collect();
        let norm = z.iter().map(|&v| v as f64 * v as f64).sum::<f64>().sqrt();
        (norm > 0.0).then_some(Self { w, h, z, norm })
    }

    /// Per-pixel variance.
    fn var(&self) -> f64 {
        self.norm * self.norm / (self.w * self.h) as f64
    }
}

/// One template at one scale: level 0 is the work scale, level `l` is
/// 2^l times smaller. Only levels whose short side keeps
/// [`MIN_COARSE_PX`] pixels are built.
#[derive(Debug, Clone)]
struct Stack {
    levels: Vec<Kernel>,
    /// The template's block projections for the certification bound
    /// ([`certify`]), coarsest first: `f` = 16, 8, 4, 2 while the template
    /// keeps [`MIN_PROJ_BLOCKS`] blocks on each side.
    projs: Vec<Proj>,
}

/// A template projected onto `f` x `f` blocks: `ct` is each block's mean
/// minus the whole template's mean (`cw` x `ch` blocks, the ones the
/// coarse kernel covers), `res` the norm of what the projection leaves
/// out. With `P` the block projection and `t` the zero-mean template,
/// `|t|^2 = f^2 |ct|^2 + res^2`.
#[derive(Debug, Clone)]
struct Proj {
    f: usize,
    cw: usize,
    ch: usize,
    ct: Vec<f64>,
    res: f64,
}

impl Proj {
    fn new(t: &Gray, top: usize, norm: f64) -> Self {
        let (mut cw, mut ch, mut px) = (t.w, t.h, t.px.clone());
        for _ in 0..top {
            (cw, ch, px) = half(cw, ch, &px);
        }
        let f = 1usize << top;
        let mean = t.px.iter().map(|&v| v as f64).sum::<f64>() / (t.w * t.h) as f64;
        let ct: Vec<f64> = px.iter().map(|&v| v as f64 - mean).collect();
        let proj2 = (f * f) as f64 * ct.iter().map(|&v| v * v).sum::<f64>();
        let res = (norm * norm - proj2).max(0.0).sqrt();
        Self { f, cw, ch, ct, res }
    }
}

impl Stack {
    fn new(t: &Gray) -> Option<Self> {
        Self::with_kept(t, MIN_KEPT)
    }

    /// [`Stack::new`] with the kept-variance rule as an argument (tests).
    fn with_kept(t: &Gray, min_kept: f64) -> Option<Self> {
        let base = Kernel::new(t.w, t.h, &t.px)?;
        let var0 = base.var();
        let mut levels = vec![base];
        let (mut w, mut h, mut px) = (t.w, t.h, t.px.clone());
        while levels.len() <= MAX_LEVEL && (w / 2).min(h / 2) >= MIN_COARSE_PX {
            (w, h, px) = half(w, h, &px);
            match Kernel::new(w, h, &px) {
                Some(k) if k.var() >= min_kept * var0 => levels.push(k),
                _ => break,
            }
        }
        let projs = (1..=4)
            .rev()
            .filter(|&l| (t.w >> l).min(t.h >> l) >= MIN_PROJ_BLOCKS)
            .map(|l| Proj::new(t, l, levels[0].norm))
            .collect();
        Some(Self { levels, projs })
    }
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let (ca, ra) = a.as_chunks::<8>();
    let (cb, rb) = b.as_chunks::<8>();
    for (x, y) in ca.iter().zip(cb) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    let mut s = ((acc[0] + acc[1]) + (acc[2] + acc[3])) + ((acc[4] + acc[5]) + (acc[6] + acc[7]));
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

/// Zero-mean NCC of `k` at (x, y) of `p`; `None` on a flat window
/// ([`Level::window`]).
#[inline]
fn ncc_at(p: &Level, k: &Kernel, x: usize, y: usize) -> Option<f64> {
    let (_, var) = p.window(x, y, k.w, k.h)?;
    let mut num = 0.0f64;
    for ty in 0..k.h {
        let row = &p.px[(y + ty) * p.w + x..][..k.w];
        num += dot(row, &k.z[ty * k.w..][..k.w]) as f64;
    }
    Some(num / (var.sqrt() * k.norm))
}

/// Best score of one search and its multiply-adds (window pixels scored).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Found {
    score: f64,
    macs: usize,
}

const NONE_FOUND: Found = Found {
    score: -1.0,
    macs: 0,
};

/// Every position at level 0.
fn search_full(region: &[Level], st: &Stack) -> Found {
    let (p, k) = (&region[0], &st.levels[0]);
    if k.w > p.w || k.h > p.h {
        return NONE_FOUND;
    }
    let mut out = NONE_FOUND;
    for y in 0..=p.h - k.h {
        for x in 0..=p.w - k.w {
            out.macs += k.w * k.h;
            if let Some(v) = ncc_at(p, k, x, y)
                && v > out.score
            {
                out.score = v;
            }
        }
    }
    out
}

/// Coarse to fine: every position at the coarsest level where the template
/// fits, then the best [`PEAKS`] peaks, each refined level by level in a
/// small window around twice its position. The score is level 0's.
///
/// Peaks that sit close together refine over the same windows. A window is
/// scored once per search (F263-3): its score is the same every time, so
/// the result is the same and only the repeats are saved.
///
/// [`search_fast_skipping`] also leaves out the level-0 windows that
/// cannot reach [`PRESENT`].
fn search_fast(region: &[Level], st: &Stack) -> Found {
    search_fast_with(region, st, PEAKS, REFINE_FIRST, REFINE)
}

/// [`search_fast`] with its peak count and refine windows as arguments
/// (tests pin each one).
fn search_fast_with(
    region: &[Level],
    st: &Stack,
    n_peaks: usize,
    refine_first: usize,
    refine: usize,
) -> Found {
    search_fast_once(region, st, n_peaks, refine_first, refine, true, &[])
}

/// Block sizes of the projection bounds [`search_fast_skipping`] checks,
/// coarsest first. Measured on 107 in-match frames with the words-v1 end
/// titles (2026-10-13): the 8-block bound puts 96.7% of the level-0 refine
/// windows under [`PRESENT`] and the 4-block bound the rest; they cost
/// about 1.5% and 6% of one exact score.
const SKIP_BLOCKS: [usize; 2] = [8, 4];

/// [`search_fast`] for a read that runs on every poll tick (F263-3): a
/// level-0 refine window is scored only when no bound of `skip`
/// ([`bound_at`], an upper bound on its score) is below [`PRESENT`].
///
/// A window that is left out scores under [`PRESENT`], so a best score at
/// or above [`PRESENT`] is the same score [`search_fast`] returns, and one
/// below it stays below (it may be lower, or -1). The refine path does not
/// change: the levels above 0 are scored as before. The bound has the
/// float error [`certify`] describes (about 2e-4), far from the
/// [`FLOOR`] a sure read needs.
fn search_fast_skipping(region: &[Level], st: &Stack, skip: &[(&Proj, &Phased)]) -> Found {
    search_fast_once(region, st, PEAKS, REFINE_FIRST, REFINE, true, skip)
}

/// [`search_fast_with`]; `once` = false scores a window again every time a
/// peak's refine reaches it (the reference the tests compare with), and
/// `skip` is [`search_fast_skipping`]'s bounds (empty: every refine window
/// is scored).
fn search_fast_once(
    region: &[Level],
    st: &Stack,
    n_peaks: usize,
    refine_first: usize,
    refine: usize,
    once: bool,
    skip: &[(&Proj, &Phased)],
) -> Found {
    let fits = |l: usize| {
        l < region.len()
            && l < st.levels.len()
            && st.levels[l].w <= region[l].w
            && st.levels[l].h <= region[l].h
    };
    let Some(top) = (0..st.levels.len()).rev().find(|&l| fits(l)) else {
        return NONE_FOUND;
    };
    if top == 0 || !fits(0) {
        return search_full(region, st);
    }
    let (p, k) = (&region[top], &st.levels[top]);
    let mut macs = 0;
    let mut coarse: Vec<(f64, usize, usize)> = Vec::new();
    for y in 0..=p.h - k.h {
        for x in 0..=p.w - k.w {
            macs += k.w * k.h;
            if let Some(v) = ncc_at(p, k, x, y) {
                coarse.push((v, x, y));
            }
        }
    }
    coarse.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut peaks: Vec<(usize, usize)> = Vec::with_capacity(n_peaks);
    for &(_, x, y) in &coarse {
        if peaks.len() == n_peaks {
            break;
        }
        if peaks
            .iter()
            .all(|&(px, py)| px.abs_diff(x) > PEAK_SEP || py.abs_diff(y) > PEAK_SEP)
        {
            peaks.push((x, y));
        }
    }
    let mut best = -1.0f64;
    // The windows scored so far in this search: (level, x, y) to its score.
    let mut scored: std::collections::HashMap<(usize, usize, usize), Option<f64>> =
        std::collections::HashMap::new();
    for (mut x, mut y) in peaks {
        let mut score = None;
        for l in (0..top).rev() {
            let (p, k) = (&region[l], &st.levels[l]);
            let r = if l + 1 == top { refine_first } else { refine };
            let (cx, cy) = ((2 * x).min(p.w - k.w), (2 * y).min(p.h - k.h));
            let mut lvl_best: Option<(f64, usize, usize)> = None;
            for yy in cy.saturating_sub(r)..=(cy + r).min(p.h - k.h) {
                for xx in cx.saturating_sub(r)..=(cx + r).min(p.w - k.w) {
                    let key = (l, xx, yy);
                    let v = match scored.get(&key) {
                        Some(&v) if once => v,
                        _ => {
                            // a flat window has no score either way
                            let out = l == 0
                                && skip.iter().any(|(pr, ph)| {
                                    bound_at(p, ph, k, pr, xx, yy)
                                        .is_none_or(|ub| ub + BOUND_SLACK < PRESENT as f64)
                                });
                            let v = if out {
                                None
                            } else {
                                macs += k.w * k.h;
                                ncc_at(p, k, xx, yy)
                            };
                            scored.insert(key, v);
                            v
                        }
                    };
                    if let Some(v) = v
                        && lvl_best.is_none_or(|b| v > b.0)
                    {
                        lvl_best = Some((v, xx, yy));
                    }
                }
            }
            (x, y) = lvl_best.map_or((cx, cy), |b| (b.1, b.2));
            score = lvl_best.map(|b| b.0);
        }
        if let Some(s) = score
            && s > best
        {
            best = s;
        }
    }
    Found { score: best, macs }
}

// ---------------------------------------------------------- certification

/// One phase image of box means, in f64, with its integral images.
struct Means {
    w: usize,
    px: Vec<f64>,
    s1: Vec<f64>,
    s2: Vec<f64>,
}

impl Means {
    fn new(w: usize, h: usize, px: Vec<f64>) -> Self {
        let mut s1 = vec![0.0f64; (w + 1) * (h + 1)];
        let mut s2 = vec![0.0f64; (w + 1) * (h + 1)];
        for y in 0..h {
            for x in 0..w {
                let v = px[y * w + x];
                let i = (y + 1) * (w + 1) + x + 1;
                s1[i] = v + s1[i - 1] + s1[i - w - 1] - s1[i - w - 2];
                s2[i] = v * v + s2[i - 1] + s2[i - w - 1] - s2[i - w - 2];
            }
        }
        Self { w, px, s1, s2 }
    }

    /// Sum of `s` over the `tw` x `th` window at (x, y).
    #[inline]
    fn rect(&self, s: &[f64], x: usize, y: usize, tw: usize, th: usize) -> f64 {
        let a = y * (self.w + 1) + x;
        let b = (y + th) * (self.w + 1) + x;
        s[b + tw] - s[b] - s[a + tw] + s[a]
    }
}

/// Box means of a level-0 region over `f` x `f` squares at every position,
/// split by phase: `phases[ay * f + ax]` holds the means at
/// `(ax + f i, ay + f j)`, so a window's blocks are a contiguous
/// `cw` x `ch` patch of one phase image. The means stay in f64
/// ([`bound_at`] subtracts the window mean from them).
struct Phased {
    f: usize,
    phases: Vec<Means>,
}

impl Phased {
    fn new(p: &Level, f: usize) -> Self {
        let ff = (f * f) as f64;
        let count = |n: usize, a: usize| if n >= f + a { (n - f - a) / f + 1 } else { 0 };
        let mut phases = Vec::with_capacity(f * f);
        for ay in 0..f {
            for ax in 0..f {
                let (pw, ph) = (count(p.w, ax), count(p.h, ay));
                let mut px = Vec::with_capacity(pw * ph);
                for j in 0..ph {
                    for i in 0..pw {
                        px.push(p.rect(&p.s1, ax + f * i, ay + f * j, f, f) / ff);
                    }
                }
                phases.push(Means::new(pw, ph, px));
            }
        }
        Self { f, phases }
    }
}

/// `sum((a_i - m) * b_i)` in f64.
#[inline]
fn dot_centred(a: &[f64], b: &[f64], m: f64) -> f64 {
    let mut acc = [0.0f64; 4];
    let (ca, ra) = a.as_chunks::<4>();
    let (cb, rb) = b.as_chunks::<4>();
    for (x, y) in ca.iter().zip(cb) {
        for i in 0..4 {
            acc[i] += (x[i] - m) * y[i];
        }
    }
    let mut s = (acc[0] + acc[1]) + (acc[2] + acc[3]);
    for (x, y) in ra.iter().zip(rb) {
        s += (x - m) * y;
    }
    s
}

/// An upper bound on the NCC of `k` at (x, y) of `p`, from the block
/// projection. With `P` the projection onto the window's `f` x `f` block
/// means, `<p, t> = <Pp, Pt> + <(I-P)p, (I-P)t> <= <Pp, Pt> + |(I-P)p| |(I-P)t|`
/// (the two parts are orthogonal; Cauchy-Schwarz on the second).
/// `<Pp, Pt>` is a dot product of `cw * ch` block means instead of the
/// whole window. `None` on a flat window, where [`ncc_at`] has no score
/// either.
///
/// The block means are centred on the window mean before the dot product,
/// and the sum is in f64 (F253-11): on a bright, nearly flat window the
/// uncentred f32 sum lost up to 1.2e-3 of the score to cancellation.
#[inline]
fn bound_at(p: &Level, ph: &Phased, k: &Kernel, pr: &Proj, x: usize, y: usize) -> Option<f64> {
    let n = (k.w * k.h) as f64;
    let (sum, var) = p.window(x, y, k.w, k.h)?;
    let m = sum / n;
    let f = ph.f;
    let q = &ph.phases[(y % f) * f + x % f];
    let (ox, oy) = (x / f, y / f);
    let sb = q.rect(&q.s1, ox, oy, pr.cw, pr.ch);
    let sb2 = q.rect(&q.s2, ox, oy, pr.cw, pr.ch);
    let mut db = 0.0f64;
    for j in 0..pr.ch {
        db += dot_centred(
            &q.px[(oy + j) * q.w + ox..][..pr.cw],
            &pr.ct[j * pr.cw..][..pr.cw],
            m,
        );
    }
    let ff = (f * f) as f64;
    let pp = ff * db;
    let proj2 = ff * (sb2 - 2.0 * m * sb + (pr.cw * pr.ch) as f64 * m * m);
    let res_p = (var - proj2).max(0.0).sqrt();
    Some((pp + res_p * pr.res) / (var.sqrt() * k.norm))
}

/// Fewest blocks on a template side for a projection bound.
const MIN_PROJ_BLOCKS: usize = 3;
/// Slack on the bound for float rounding (the f32 region pixels; the
/// projection itself is in f64). It covers the bound's own error, not
/// [`ncc_at`]'s: that score is an uncentred f32 dot and overshoots the exact
/// score by up to about 1.7e-4 on bright, nearly flat windows (F257-5).
const BOUND_SLACK: f64 = 1e-4;
/// Multiply-adds one read may spend on exact scores while certifying.
const CERT_MACS: usize = 300_000_000;

/// Whether a template's exhaustive best reaches `tau`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Cert {
    /// No window scores `tau` or more.
    Below,
    /// A window scores this (`>= tau`).
    Above(f64),
    /// The exact-score budget ran out first.
    Unknown,
}

/// Decide exactly whether any window of `st` scores at least `tau`: a
/// window is ruled out when any bound of the chain ([`bound_at`], one per
/// projection, coarsest and cheapest first) is below `tau`; every other
/// window is scored with [`ncc_at`]. The same answer as [`search_full`]
/// against `tau`, at a small part of its cost when the bounds are tight,
/// up to [`ncc_at`]'s own f32 error (about 1.7e-4 at most, on bright,
/// nearly flat windows; F257-5). Both ways: a window that error lifts over
/// `tau` may be ruled out by its bound, so `certify` says `Below` where
/// [`search_full`] reports a score at `tau`; and a window that is scored
/// exactly can return `Above` on that error. For a result read `Above`
/// only flags it. Without projections the stack is searched in full.
fn certify(
    region: &[Level],
    chain: &[(&Proj, &Phased)],
    st: &Stack,
    tau: f64,
    budget: &mut usize,
) -> Cert {
    let (p, k) = (&region[0], &st.levels[0]);
    if k.w > p.w || k.h > p.h {
        return Cert::Below;
    }
    if chain.is_empty() {
        let full = search_full(region, st);
        *budget = budget.saturating_sub(full.macs);
        return if full.score >= tau {
            Cert::Above(full.score)
        } else {
            Cert::Below
        };
    }
    for y in 0..=p.h - k.h {
        'window_x: for x in 0..=p.w - k.w {
            for (pr, ph) in chain {
                match bound_at(p, ph, k, pr, x, y) {
                    None => continue 'window_x,
                    Some(ub) if ub + BOUND_SLACK < tau => continue 'window_x,
                    Some(_) => {}
                }
            }
            let cost = k.w * k.h;
            if *budget < cost {
                return Cert::Unknown;
            }
            *budget -= cost;
            if let Some(v) = ncc_at(p, k, x, y)
                && v >= tau
            {
                return Cert::Above(v);
            }
        }
    }
    Cert::Below
}

/// A region inside the game rect: finite, `0 <= x0 < x1 <= 1`, same for y.
fn roi_is_valid([x0, y0, x1, y1]: [f64; 4]) -> bool {
    [x0, y0, x1, y1].iter().all(|v| v.is_finite())
        && 0.0 <= x0
        && x0 < x1
        && x1 <= 1.0
        && 0.0 <= y0
        && y0 < y1
        && y1 <= 1.0
}

/// A layout whose region reaches into [`NAMEPLATE`]: cut to end at
/// [`ACCOLADE_Y1`] when it starts above the nameplate, else dropped.
fn keep_off_nameplate(mut l: Layout) -> Option<Layout> {
    let [x0, y0, x1, y1] = l.roi;
    let [nx0, ny0, nx1, ny1] = NAMEPLATE;
    let overlaps = x0 < nx1 && nx0 < x1 && y0 < ny1 && ny0 < y1;
    if !overlaps {
        return Some(l);
    }
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let first = !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed);
    if y0 < ACCOLADE_Y1 {
        if first {
            tracing::warn!(id = %l.id, "result layout reached the accolade nameplate; cut above it");
        }
        l.roi[3] = ACCOLADE_Y1;
        Some(l)
    } else {
        if first {
            tracing::warn!(id = %l.id, "result layout inside the accolade nameplate; dropped");
        }
        None
    }
}

/// The region of a layout at the work scale, with its pyramid.
fn region(rgb: &RgbImage, roi: [f64; 4], plane: Plane) -> Option<Vec<Level>> {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = game_rect_16_9(w, h);
    let [fx0, fy0, fx1, fy1] = roi;
    let x0 = gx + (fx0 * gw as f64) as u32;
    let x1 = (gx + (fx1 * gw as f64) as u32).min(w);
    let y0 = gy + (fy0 * gh as f64) as u32;
    let y1 = (gy + (fy1 * gh as f64) as u32).min(h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (rw, rh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut g = Gray::new(rw, rh);
    for y in 0..rh {
        for x in 0..rw {
            let p = rgb.get_pixel(x0 + x as u32, y0 + y as u32).0;
            g.px[y * rw + x] = match plane {
                Plane::Max => p[0].max(p[1]).max(p[2]),
                Plane::Min => p[0].min(p[1]).min(p[2]),
            } as f32
                / 255.0;
        }
    }
    let s = WORK * 1440.0 / gh as f64;
    let nw = ((rw as f64 * s).round_ties_even() as usize).max(1);
    let nh = ((rh as f64 * s).round_ties_even() as usize).max(1);
    Some(pyramid(g.resize(nw, nh)))
}

/// A work-scale region and its box-downscaled levels.
fn pyramid(g: Gray) -> Vec<Level> {
    let mut levels = vec![Level::new(g.w, g.h, g.px)];
    while levels.len() <= MAX_LEVEL {
        let last = &levels[levels.len() - 1];
        let (hw, hh, px) = half(last.w, last.h, &last.px);
        if hw == 0 || hh == 0 {
            break;
        }
        levels.push(Level::new(hw, hh, px));
    }
    levels
}

// --------------------------------------------------------------- templates

/// One word template, prepared at every search scale.
#[derive(Debug, Clone)]
struct Template {
    layout: Layout,
    outcome: Outcome,
    stacks: Vec<Stack>,
}

/// Word templates, per layout and outcome.
#[derive(Debug, Clone)]
pub struct ResultTemplates {
    tpls: Vec<Template>,
    /// The FINAL SCORE line under the end title (P4), when the folder has
    /// `p4_final_score.png`.
    final_score: Option<FinalScoreTemplate>,
}

/// The largest template PNG [`load_gray`] decodes: the 16:9 game rect at the
/// work scale. A template is a cut of a region of that rect, so a larger
/// file is not a template (F260-7).
const MAX_TEMPLATE_PX: (u32, u32) = (1280, 720);

/// An 8-bit grey PNG as a [`Gray`] in 0..1, or `None` (missing, unreadable,
/// empty, or larger than [`MAX_TEMPLATE_PX`]). The size is read from the
/// header, before any pixel is decoded.
fn load_gray(p: &Path) -> Option<Gray> {
    let (w, h) = image::image_dimensions(p).ok()?;
    if w > MAX_TEMPLATE_PX.0 || h > MAX_TEMPLATE_PX.1 {
        tracing::warn!(file = %p.display(), w, h, "template image too large; left out");
        return None;
    }
    let l = image::open(p).ok()?.to_luma8();
    if l.width() == 0 || l.height() == 0 {
        return None;
    }
    Some(Gray {
        w: l.width() as usize,
        h: l.height() as usize,
        px: l.pixels().map(|p| p.0[0] as f32 / 255.0).collect(),
    })
}

/// One template's stacks at the search scales ([`SCALES`]).
fn scaled_stacks(t: &Gray) -> Vec<Stack> {
    SCALES
        .iter()
        .filter_map(|sc| {
            let tw = ((t.w as f64 * sc).round_ties_even() as usize).max(1);
            let th = ((t.h as f64 * sc).round_ties_even() as usize).max(1);
            Stack::new(&t.resize(tw, th))
        })
        .collect()
}

impl ResultTemplates {
    /// The old manual folder, `<data_dir>/templates/result`.
    pub fn dir_in(data_dir: &Path) -> PathBuf {
        data_dir.join(TEMPLATE_SUBDIR)
    }

    /// The words-v1 pack folder, `<data_dir>/templates/words-v1`.
    pub fn words_dir_in(data_dir: &Path) -> PathBuf {
        data_dir.join(WORDS_SUBDIR)
    }

    /// Load `<source>_{victory,defeat,draw}.png` for the built-in layouts
    /// plus any listed in an optional `<dir>/layouts.json` (a JSON array of
    /// `{"id", "roi": [x0, y0, x1, y1], "plane": "max"|"min"}`; an entry with a
    /// built-in id replaces it). A new result screen plugs in with a layout
    /// entry and its templates, no code change. `None` unless at least one
    /// victory and one defeat template load.
    ///
    /// The file comes from a downloaded pack, so a file that cannot be read
    /// (not UTF-8, not a file, no permission) or does not parse refuses the
    /// folder; only a missing file means no extra layouts. Each entry is
    /// checked:
    /// * an id that is not a plain word id ([`crate::packs::layout::is_word_id`]:
    ///   no `/`, no `..`, not absolute) or a region that is not inside the
    ///   game rect refuses the whole folder (`None`, so the caller falls
    ///   back);
    /// * a region that reaches into the accolade nameplate ([`NAMEPLATE`]) is
    ///   cut to end at [`ACCOLADE_Y1`] when it starts above the nameplate,
    ///   and dropped when it starts inside it. Logged once per process.
    pub fn load_dir(dir: &Path) -> Option<Self> {
        let mut layouts = Layout::builtin();
        let text = match std::fs::read_to_string(dir.join("layouts.json")) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            // Not UTF-8, a directory, no permission: refused like a file that
            // does not parse (F253-14).
            Err(e) => {
                tracing::warn!(error = %e, "result layouts.json unreadable; folder refused");
                return None;
            }
        };
        if let Some(text) = text {
            match serde_json::from_str::<Vec<Layout>>(&text) {
                Ok(extra) => {
                    for l in extra {
                        if !crate::packs::layout::is_word_id(&l.id) || !roi_is_valid(l.roi) {
                            tracing::warn!(
                                dir = %dir.display(),
                                "result layouts.json has an unsafe id or region; folder refused"
                            );
                            return None;
                        }
                        let Some(l) = keep_off_nameplate(l) else {
                            continue;
                        };
                        layouts.retain(|k| k.id != l.id);
                        layouts.push(l);
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "result layouts.json does not parse; folder refused");
                    return None;
                }
            }
        }
        // The FINAL SCORE line is a phrase, not a result word: its layout
        // (built in, or a `p4_final_score` entry above) is taken out of the
        // word layouts and read on its own.
        let final_layout = layouts
            .iter()
            .find(|l| l.id == FINAL_SCORE)
            .cloned()
            .unwrap_or_else(FinalScoreTemplate::builtin_layout);
        layouts.retain(|l| l.id != FINAL_SCORE);
        let mut tpls = Vec::new();
        for layout in layouts {
            for outcome in Outcome::ALL {
                let p = dir.join(format!("{}_{}.png", layout.id, outcome.as_str()));
                if let Some(g) = load_gray(&p) {
                    tpls.push((layout.clone(), outcome, g));
                }
            }
        }
        let final_score = load_gray(&dir.join(format!("{FINAL_SCORE}.png")))
            .and_then(|g| FinalScoreTemplate::new(final_layout, &g));
        Self::from_templates(tpls).map(|t| t.with_final_score(final_score))
    }

    /// These templates with this FINAL SCORE template (or none).
    pub fn with_final_score(mut self, t: Option<FinalScoreTemplate>) -> Self {
        self.final_score = t;
        self
    }

    /// The FINAL SCORE template, when the folder has one.
    pub fn final_score(&self) -> Option<&FinalScoreTemplate> {
        self.final_score.as_ref()
    }

    /// Build from in-memory templates (tests). Same `None` rule as `load_dir`.
    pub fn from_templates(tpls: Vec<(Layout, Outcome, Gray)>) -> Option<Self> {
        let has = |o| tpls.iter().any(|t| t.1 == o);
        if !(has(Outcome::Victory) && has(Outcome::Defeat)) {
            return None;
        }
        let tpls = tpls
            .into_iter()
            .map(|(layout, outcome, t)| Template {
                layout,
                outcome,
                stacks: scaled_stacks(&t),
            })
            .collect();
        Some(Self {
            tpls,
            final_score: None,
        })
    }

    /// The loaded layout with this id (tests).
    #[cfg(test)]
    fn layout(&self, id: &str) -> Option<&Layout> {
        self.tpls.iter().map(|t| &t.layout).find(|l| l.id == id)
    }

    /// Source ids that have at least one template.
    pub fn sources(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.tpls.iter().map(|t| t.layout.id.as_str()).collect();
        v.dedup();
        v
    }

    /// Each template's region, cut once per rect and plane.
    fn regions(&self, rgb: &RgbImage) -> Regions {
        let mut cut: Vec<([f64; 4], Plane, Option<Vec<Level>>)> = Vec::new();
        let mut of = Vec::with_capacity(self.tpls.len());
        for t in &self.tpls {
            let key = (t.layout.roi, t.layout.plane);
            let idx = match cut.iter().position(|r| (r.0, r.1) == key) {
                Some(i) => i,
                None => {
                    cut.push((key.0, key.1, region(rgb, key.0, key.1)));
                    cut.len() - 1
                }
            };
            of.push(idx);
        }
        Regions {
            levels: cut.into_iter().map(|c| c.2).collect(),
            of,
            phased: Vec::new(),
        }
    }

    /// Best score of each template (in load order) and the windows scored.
    #[cfg(test)]
    fn scores(&self, rgb: &RgbImage, exhaustive: bool) -> Vec<Found> {
        self.scores_in(&self.regions(rgb), exhaustive)
    }

    fn scores_in(&self, regions: &Regions, exhaustive: bool) -> Vec<Found> {
        self.scores_with(
            regions,
            if exhaustive {
                Search::Full
            } else {
                Search::Fast
            },
        )
    }

    fn scores_with(&self, regions: &Regions, search: Search) -> Vec<Found> {
        let mut out = Vec::with_capacity(self.tpls.len());
        for (i, t) in self.tpls.iter().enumerate() {
            let ri = regions.of[i];
            let Some(p) = &regions.levels[ri] else {
                out.push(NONE_FOUND);
                continue;
            };
            let mut best = NONE_FOUND;
            for st in &t.stacks {
                let f = match search {
                    Search::Full => search_full(p, st),
                    Search::Fast => search_fast(p, st),
                    Search::Skipping => {
                        let skip: Vec<(&Proj, &Phased)> = st
                            .projs
                            .iter()
                            .filter_map(|pr| {
                                let ph = regions.phased.iter().find(|(k, _)| *k == (ri, pr.f))?;
                                Some((pr, &ph.1))
                            })
                            .collect();
                        search_fast_skipping(p, st, &skip)
                    }
                };
                best.macs += f.macs;
                if f.score > best.score {
                    best.score = f.score;
                }
            }
            out.push(best);
        }
        out
    }

    /// Read the result word from one result frame (any layout), with the
    /// coarse-to-fine search.
    ///
    /// A read the fast scores call sure is then certified: no template of
    /// another word may score `s1 - MARGIN` or more anywhere in its region,
    /// checked exactly with [`certify`]. Since the word's own fast score is
    /// at most its exhaustive score, a sure read here is a sure exhaustive
    /// read of the same word. A read that fails the check, or whose check
    /// runs out of budget, is flagged (an unsure DRAW abstains).
    pub fn read(&self, rgb: &RgbImage) -> ResultRead {
        let mut regions = self.regions(rgb);
        let scores = self.scores_in(&regions, false);
        self.settle(&mut regions, scores, CERT_MACS)
    }

    /// [`Self::read`] behind a cheaper test for "no word on this frame",
    /// for a read that runs on every poll tick (the end title, F263-3).
    ///
    /// The templates are first searched with [`search_fast_skipping`],
    /// which does not score a refine window that a block-projection bound
    /// puts under [`PRESENT`]. Its scores at or above [`PRESENT`] are the
    /// fast search's, and the ones below stay below. So:
    /// * when no template reaches [`PRESENT`], [`Self::read`] names no word
    ///   either, and this returns "no word" without the rest of the work.
    ///   Its score is then some score under [`PRESENT`], not the best one,
    ///   and its margin is 0;
    /// * when one does, this is [`Self::read`] itself, every field: the
    ///   plain fast search, then the certification.
    pub fn read_skipping(&self, rgb: &RgbImage) -> ResultRead {
        let mut regions = self.skip_regions(rgb);
        let quick = self.scores_with(&regions, Search::Skipping);
        let best = quick.iter().map(|f| f.score).fold(-1.0f64, f64::max);
        if best < PRESENT as f64 {
            return ResultRead::nothing(best, 0.0);
        }
        let scores = self.scores_in(&regions, false);
        self.settle(&mut regions, scores, CERT_MACS)
    }

    /// [`Self::regions`] with the phase images [`Search::Skipping`] checks
    /// its bounds on ([`SKIP_BLOCKS`]).
    fn skip_regions(&self, rgb: &RgbImage) -> Regions {
        let mut regions = self.regions(rgb);
        for (i, t) in self.tpls.iter().enumerate() {
            let ri = regions.of[i];
            let Regions { levels, phased, .. } = &mut regions;
            let Some(lv) = &levels[ri] else { continue };
            for f in SKIP_BLOCKS {
                if t.stacks
                    .iter()
                    .any(|st| st.projs.iter().any(|pr| pr.f == f))
                {
                    make_phased(phased, ri, lv, f);
                }
            }
        }
        regions
    }

    /// [`Self::read`] with every window of every region scored: the
    /// reference the fast search is checked against (tests, benches). Slow.
    pub fn read_exhaustive(&self, rgb: &RgbImage) -> ResultRead {
        self.decide(&self.scores_in(&self.regions(rgb), true))
    }

    /// The read for these fast scores, certified when it is sure.
    fn settle(
        &self,
        regions: &mut Regions,
        mut scores: Vec<Found>,
        mut budget: usize,
    ) -> ResultRead {
        let read = self.decide(&scores);
        let Some(top) = read.outcome.filter(|_| !read.suspect) else {
            return read;
        };
        let s1 = self
            .tpls
            .iter()
            .zip(&scores)
            .filter(|(t, _)| t.outcome == top)
            .map(|(_, f)| f.score)
            .fold(-1.0f64, f64::max);
        let tau = s1 - MARGIN as f64;
        for (i, t) in self.tpls.iter().enumerate() {
            if t.outcome == top {
                continue;
            }
            let ri = regions.of[i];
            for st in &t.stacks {
                let cert = {
                    let Regions { levels, phased, .. } = &mut *regions;
                    let Some(lv) = &levels[ri] else { break };
                    for pr in &st.projs {
                        make_phased(phased, ri, lv, pr.f);
                    }
                    let chain: Vec<(&Proj, &Phased)> = st
                        .projs
                        .iter()
                        .filter_map(|pr| {
                            let ph = phased.iter().find(|(k, _)| *k == (ri, pr.f))?;
                            Some((pr, &ph.1))
                        })
                        .collect();
                    certify(lv, &chain, st, tau, &mut budget)
                };
                match cert {
                    Cert::Below => {}
                    Cert::Above(v) => {
                        scores[i].score = scores[i].score.max(v);
                        return flag(self.decide(&scores));
                    }
                    Cert::Unknown => return flag(read),
                }
            }
        }
        read
    }

    fn decide(&self, scores: &[Found]) -> ResultRead {
        let mut best: [(f64, Option<&str>); 3] = [(-1.0, None); 3];
        for (t, f) in self.tpls.iter().zip(scores) {
            let slot = &mut best[t.outcome.index()];
            if f.score > slot.0 {
                *slot = (f.score, Some(&t.layout.id));
            }
        }
        let mut order = Outcome::ALL;
        order.sort_by(|a, b| best[b.index()].0.total_cmp(&best[a.index()].0));
        let (s1, src) = best[order[0].index()];
        let s2 = best[order[1].index()].0;
        if s1 < PRESENT as f64 {
            return ResultRead::nothing(s1, s1 - s2);
        }
        let suspect = !(s1 >= FLOOR as f64 && s1 - s2 >= MARGIN as f64);
        // DRAW has no real frame: only a sure DRAW read names it.
        if order[0] == Outcome::Draw && suspect {
            return ResultRead::nothing(s1, s1 - s2);
        }
        ResultRead {
            outcome: Some(order[0]),
            source: src.map(str::to_string),
            score: s1 as f32,
            margin: (s1 - s2) as f32,
            suspect,
        }
    }
}

/// Which search [`ResultTemplates::scores_with`] runs.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Search {
    /// Every window ([`search_full`]).
    Full,
    /// Coarse to fine ([`search_fast`]).
    Fast,
    /// Coarse to fine, leaving out the refine windows that cannot reach
    /// [`PRESENT`] ([`search_fast_skipping`]). The caller makes the
    /// [`SKIP_BLOCKS`] phase images first.
    Skipping,
}

/// A read that must not be sure: flagged, or nothing for a DRAW (only a
/// sure DRAW is named).
fn flag(mut r: ResultRead) -> ResultRead {
    if r.outcome == Some(Outcome::Draw) {
        return ResultRead::nothing(r.score as f64, r.margin as f64);
    }
    r.suspect = true;
    r
}

// ------------------------------------------------------------- FINAL SCORE

/// What one frame says about the FINAL SCORE line (P4).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum FinalScoreRead {
    /// A window scores [`FINAL_SCORE_FLOOR`] or more: the line is on the
    /// frame. The only read that is a signal.
    Sure(f32),
    /// The best window scores [`PRESENT`] or more but no window reaches the
    /// floor, or the exact check ran out of budget. Not a signal.
    Suspect(f32),
    /// The fast search found no window at [`PRESENT`]. Not a signal.
    Absent(f32),
}

impl FinalScoreRead {
    /// Only a sure read says the FINAL SCORE line is on the frame.
    pub fn is_signal(self) -> bool {
        matches!(self, FinalScoreRead::Sure(_))
    }

    fn of(score: f64) -> Self {
        let s = score.max(0.0) as f32;
        if score >= FINAL_SCORE_FLOOR as f64 {
            FinalScoreRead::Sure(s)
        } else if score >= PRESENT as f64 {
            FinalScoreRead::Suspect(s)
        } else {
            FinalScoreRead::Absent(s)
        }
    }
}

/// The words-v1 `p4_final_score` template: the FINAL SCORE line under the
/// end title, which the end-title gate takes as end-of-match evidence when
/// the title itself is too faint (ocr-v1's P4 reads it with Tesseract).
#[derive(Debug, Clone)]
pub struct FinalScoreTemplate {
    layout: Layout,
    stacks: Vec<Stack>,
}

impl FinalScoreTemplate {
    /// The pack's region for the line (words-v1 `words.json`): x 36-64%,
    /// y 70-84%, plane `min` (white text).
    pub fn builtin_layout() -> Layout {
        Layout::new(FINAL_SCORE, [0.36, 0.70, 0.64, 0.84], Plane::Min)
    }

    /// A template at the work scale, or `None` when it is flat.
    pub fn new(layout: Layout, t: &Gray) -> Option<Self> {
        let stacks = scaled_stacks(t);
        (!stacks.is_empty()).then_some(Self { layout, stacks })
    }

    /// Read one frame with the coarse-to-fine search. A fast score at the
    /// floor is a real window's score, so it is sure. A fast score between
    /// [`PRESENT`] and the floor may be an under-read, so [`certify`]
    /// decides exactly whether any window reaches the floor: a window that
    /// does makes the read sure, and a check that runs out of budget leaves
    /// it suspect. So a sure read here is a sure exhaustive read, and the
    /// reverse holds whenever the fast score is at [`PRESENT`] (both up to
    /// [`ncc_at`]'s f32 error at the floor, see [`certify`]).
    ///
    /// Cost note (F257-6): a suspect fast score certifies with the full
    /// [`CERT_MACS`] budget, and the bound pass is not budgeted (F253-13).
    /// On the poll tick that can repeat on every stable tick. No frame of
    /// the 652 measured lands in the suspect band (others at most 0.39,
    /// the line at least 0.86), so it has never run on a real frame.
    pub fn read(&self, rgb: &RgbImage) -> FinalScoreRead {
        self.read_with_budget(rgb, CERT_MACS)
    }

    fn read_with_budget(&self, rgb: &RgbImage, budget: usize) -> FinalScoreRead {
        let Some(levels) = region(rgb, self.layout.roi, self.layout.plane) else {
            return FinalScoreRead::Absent(0.0);
        };
        let fast = self
            .stacks
            .iter()
            .map(|st| search_fast(&levels, st).score)
            .fold(-1.0f64, f64::max);
        self.settle(&levels, fast, budget)
    }

    /// The read for this fast score, certified against the floor when it is
    /// suspect (tests hand it an under-read score).
    fn settle(&self, levels: &[Level], fast: f64, mut budget: usize) -> FinalScoreRead {
        let read = FinalScoreRead::of(fast);
        if !matches!(read, FinalScoreRead::Suspect(_)) {
            return read;
        }
        let tau = FINAL_SCORE_FLOOR as f64;
        let mut phased: Vec<((usize, usize), Phased)> = Vec::new();
        for st in &self.stacks {
            for pr in &st.projs {
                make_phased(&mut phased, 0, levels, pr.f);
            }
            let chain: Vec<(&Proj, &Phased)> = st
                .projs
                .iter()
                .filter_map(|pr| {
                    let ph = phased.iter().find(|(k, _)| *k == (0, pr.f))?;
                    Some((pr, &ph.1))
                })
                .collect();
            match certify(levels, &chain, st, tau, &mut budget) {
                Cert::Below => {}
                Cert::Above(v) => return FinalScoreRead::Sure(v as f32),
                Cert::Unknown => return read,
            }
        }
        read
    }

    /// [`Self::read`] with every window scored (tests, benches). Slow.
    pub fn read_exhaustive(&self, rgb: &RgbImage) -> FinalScoreRead {
        let Some(levels) = region(rgb, self.layout.roi, self.layout.plane) else {
            return FinalScoreRead::Absent(0.0);
        };
        FinalScoreRead::of(
            self.stacks
                .iter()
                .map(|st| search_full(&levels, st).score)
                .fold(-1.0f64, f64::max),
        )
    }
}

/// The regions of one frame: one per distinct rect and plane, the region
/// index of each template, and box-mean phase images made on demand.
struct Regions {
    levels: Vec<Option<Vec<Level>>>,
    of: Vec<usize>,
    phased: Vec<((usize, usize), Phased)>,
}

/// Make the phase images of region `ri` at block size `f`, once.
fn make_phased(cache: &mut Vec<((usize, usize), Phased)>, ri: usize, levels: &[Level], f: usize) {
    if !cache.iter().any(|(k, _)| *k == (ri, f)) {
        cache.push(((ri, f), Phased::new(&levels[0], f)));
    }
}

// ------------------------------------------------------------ live reload

/// Where the result templates live: the words-v1 pack folder first, then
/// the old manual folder. `None` skips that folder.
#[derive(Debug, Clone, Default)]
pub struct ResultDirs {
    pub words: Option<PathBuf>,
    pub legacy: Option<PathBuf>,
}

/// Which folder the loaded templates came from (fixed names, logged).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ResultPack {
    #[serde(rename = "words-v1")]
    WordsV1,
    #[serde(rename = "result")]
    Legacy,
}

impl ResultPack {
    pub fn as_str(self) -> &'static str {
        match self {
            ResultPack::WordsV1 => "words-v1",
            ResultPack::Legacy => "result",
        }
    }
}

/// Loaded templates and the folder they came from.
#[derive(Debug)]
pub struct LoadedResults {
    pub templates: ResultTemplates,
    pub pack: ResultPack,
    /// The end-title templates of `templates` on their own
    /// ([`ResultTemplates::end_titles`]), cut once per load.
    pub end_titles: Option<ResultTemplates>,
    /// The result screens `templates` can name
    /// ([`ResultTemplates::screens`]), found once per load.
    pub screens: ResultScreens,
    /// The result screens' templates of `templates` on their own
    /// ([`ResultTemplates::screen_words`]), cut once per load: the first
    /// look of [`Self::read_screens`].
    pub screen_words: Option<ResultTemplates>,
    /// The phase-screen titles, from the words-v1 folder only (the old
    /// folder has none), whichever folder the result words came from.
    pub phrases: Option<phrase::PhraseTemplates>,
}

impl LoadedResults {
    /// One load: the word templates of `pack` with their end titles cut
    /// out, and the phase-screen titles of the words-v1 folder.
    pub fn new(
        templates: ResultTemplates,
        pack: ResultPack,
        phrases: Option<phrase::PhraseTemplates>,
    ) -> Self {
        let end_titles = templates.end_titles();
        let screens = templates.screens();
        let screen_words = templates.screen_words();
        Self {
            templates,
            pack,
            end_titles,
            screens,
            screen_words,
            phrases,
        }
    }

    /// One frame's result read for the poller's result screens (accolade,
    /// rank screen), in two looks.
    ///
    /// The first look reads the result screens' own templates
    /// ([`Self::screen_words`]): two small regions. Only when it is sure of
    /// a word is the frame read in full ([`ResultTemplates::read`], every
    /// layout, #253's result read), and that second read is the answer.
    ///
    /// The first look loses no word. A full read that is sure of a word
    /// found on a result screen has its leading template among the result
    /// screens' templates, with the same score, and every rival there is a
    /// rival in the full read too. So the first look is sure of that word
    /// whenever the full read is, and this names exactly the words the full
    /// read alone would name on a result screen. What it saves is the full
    /// read, most of which is the end-title region, on every frame that
    /// shows no result-screen word.
    pub fn read_screens(&self, rgb: &RgbImage) -> ScreenRead {
        let seen = self
            .screen_words
            .as_ref()
            .is_some_and(|words| words.read(rgb).sure().is_some());
        ScreenRead {
            read: seen.then(|| self.templates.read(rgb)),
            screens: self.screens,
        }
    }
}

type Stamp = [DirStamp; 2];

impl ResultDirs {
    pub fn in_data_dir(data_dir: &Path) -> Self {
        Self {
            words: Some(ResultTemplates::words_dir_in(data_dir)),
            legacy: Some(ResultTemplates::dir_in(data_dir)),
        }
    }

    fn stamp(&self) -> Stamp {
        [
            dir_stamp(self.words.as_deref()),
            dir_stamp(self.legacy.as_deref()),
        ]
    }

    /// Templates from the first folder that loads: words-v1, then the old
    /// manual folder.
    pub fn load(&self) -> Option<LoadedResults> {
        let phrases = self
            .words
            .as_deref()
            .and_then(phrase::PhraseTemplates::load_dir);
        let words = self.words.as_deref().and_then(ResultTemplates::load_dir);
        if let Some(templates) = words {
            return Some(LoadedResults::new(templates, ResultPack::WordsV1, phrases));
        }
        let legacy = self.legacy.as_deref().and_then(ResultTemplates::load_dir);
        legacy.map(|templates| LoadedResults::new(templates, ResultPack::Legacy, phrases))
    }

    /// Load, or `None` when a folder changed while it loaded (a swap ran
    /// meanwhile; the caller retries later). `during` runs between the first
    /// stamp and the load (tests use it to swap a folder in).
    fn load_stable(&self, during: impl FnOnce()) -> Option<(Option<LoadedResults>, Stamp)> {
        let before = self.stamp();
        during();
        let loaded = self.load();
        (self.stamp() == before).then_some((loaded, before))
    }
}

/// The new reader's [`ResultTemplates`], re-loaded when a template folder
/// changes ([`super::heroes::LiveHeroSet`]'s rules).
///
/// Pack sync installs words-v1 in the background after the reader is set
/// up. Each result read checks the two folders' stamps (two `stat` calls).
/// On a change it starts one background load and keeps reading with the
/// templates it has, so a poll tick never waits on a load. A load that saw
/// a folder change while it ran is thrown away and retried on a later read,
/// so the templates never mix files from before and after a swap.
#[derive(Debug, Default)]
pub struct LiveResults {
    inner: Arc<LiveInner>,
}

type Current = (Option<Arc<LoadedResults>>, Stamp);

#[derive(Debug, Default)]
struct LiveInner {
    dirs: ResultDirs,
    current: std::sync::RwLock<Current>,
    loader: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    #[cfg(test)]
    hooks: TestHooks,
}

/// Test probes on the reload thread: a spawn count, a delay before the
/// load, and a step run between the first stamp and the load.
#[cfg(test)]
#[derive(Default)]
struct TestHooks {
    spawns: std::sync::atomic::AtomicUsize,
    delay_ms: std::sync::atomic::AtomicU64,
    during: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

#[cfg(test)]
impl std::fmt::Debug for TestHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestHooks")
    }
}

impl LiveInner {
    /// What runs between the reload's first stamp and its load: nothing,
    /// except in tests.
    fn before_load(&self) -> impl FnOnce() + use<> {
        #[cfg(test)]
        {
            use std::sync::atomic::Ordering;
            let ms = self.hooks.delay_ms.load(Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            let during = self.hooks.during.lock().unwrap().take();
            move || {
                if let Some(f) = during {
                    f();
                }
            }
        }
        #[cfg(not(test))]
        || {}
    }

    fn install(&self, (loaded, stamp): (Option<LoadedResults>, Stamp)) {
        tracing::info!(
            results = loaded.is_some(),
            result_pack = loaded.as_ref().map(|l| l.pack.as_str()),
            result_sources = ?loaded.as_ref().map(|l| l.templates.sources()),
            end_titles = ?loaded
                .as_ref()
                .and_then(|l| l.end_titles.as_ref())
                .map(|t| t.sources()),
            phrases = ?loaded.as_ref().and_then(|l| l.phrases.as_ref()).map(|p| p.ids()),
            "new reader result words loaded"
        );
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = (loaded.map(Arc::new), stamp);
    }
}

impl LiveResults {
    /// Load now (reader setup). A folder that keeps changing during the
    /// first tries leaves the templates empty; the next read loads again.
    pub fn load(dirs: ResultDirs) -> Self {
        let inner = LiveInner {
            dirs,
            ..Default::default()
        };
        let first = (0..3).find_map(|_| inner.dirs.load_stable(|| {}));
        // An all-zero stamp never matches a real one: the next read reloads.
        inner.install(first.unwrap_or((None, [Some((0, 0, 0, 0)); 2])));
        Self {
            inner: inner.into(),
        }
    }

    /// The templates for this read. Starts a background reload when a
    /// folder changed since they were loaded.
    pub fn current(&self) -> Option<Arc<LoadedResults>> {
        let (loaded, stamp) = self
            .inner
            .current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if self.inner.dirs.stamp() != stamp {
            self.start_reload();
        }
        loaded
    }

    fn start_reload(&self) {
        let Ok(mut loader) = self.inner.loader.try_lock() else {
            return;
        };
        if loader.as_ref().is_some_and(|h| !h.is_finished()) {
            return;
        }
        let inner = Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("result-words-reload".into())
            .spawn(move || {
                let during = inner.before_load();
                match inner.dirs.load_stable(during) {
                    Some(loaded) => inner.install(loaded),
                    None => tracing::debug!("result words changed while loading, retry later"),
                }
            });
        match spawned {
            Ok(handle) => {
                #[cfg(test)]
                self.inner
                    .hooks
                    .spawns
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                *loader = Some(handle);
            }
            Err(e) => tracing::warn!(error = %e, "result words reload not started"),
        }
    }

    /// Reload threads started so far (tests).
    #[cfg(test)]
    pub(crate) fn reload_spawns(&self) -> usize {
        self.inner
            .hooks
            .spawns
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Make each reload wait `ms` before it loads (tests).
    #[cfg(test)]
    pub(crate) fn set_load_delay_ms(&self, ms: u64) {
        self.inner
            .hooks
            .delay_ms
            .store(ms, std::sync::atomic::Ordering::SeqCst);
    }

    /// Run `f` inside the next reload, between its first stamp and its load
    /// (tests).
    #[cfg(test)]
    pub(crate) fn during_next_load(&self, f: impl FnOnce() + Send + 'static) {
        *self.inner.hooks.during.lock().unwrap() = Some(Box::new(f));
    }

    /// Wait for a reload in flight (tests).
    #[cfg(test)]
    pub(crate) fn wait_reload(&self) {
        let handle = self.inner.loader.lock().unwrap().take();
        if let Some(h) = handle {
            h.join().unwrap();
        }
    }
}

// --------------------------------------------------------------- end title

/// Is `id` an end-title layout: [`END_TITLE`] itself, or a variant of it
/// named `end_title_<name>` (words-v1 ships `end_title_plain`)?
pub fn is_end_title(id: &str) -> bool {
    id.strip_prefix(END_TITLE)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('_'))
}

impl ResultTemplates {
    /// The end-title templates alone: the centred VICTORY / DEFEAT / DRAW
    /// over the world at the end of a match ([`is_end_title`] layouts). The
    /// poller reads these on their own (`detect::match_end`, P3 under
    /// `reader = "new"`): the accolade and rank-screen words are other
    /// screens, and a read of them must not lend the end title a margin.
    ///
    /// `None` unless a VICTORY and a DEFEAT end-title template are loaded,
    /// because a sure read is a lead over the best other word.
    pub fn end_titles(&self) -> Option<ResultTemplates> {
        // only the end-title templates are copied (F263-10); the FINAL
        // SCORE template stays with the whole set, which is what reads it
        let tpls: Vec<Template> = self
            .tpls
            .iter()
            .filter(|t| is_end_title(&t.layout.id))
            .cloned()
            .collect();
        let has = |o| tpls.iter().any(|t| t.outcome == o);
        (has(Outcome::Victory) && has(Outcome::Defeat)).then_some(ResultTemplates {
            tpls,
            final_score: None,
        })
    }
}

// ---------------------------------------------------------- result screens

/// A post-match screen that prints the result word top-left and stays up
/// for many seconds: the poller's two word signals (`detect::match_end`,
/// `OutcomeSource::ResultWord` and `OutcomeSource::RankScreen`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultScreen {
    /// The accolade screen ([`ACCOLADE`] layouts).
    Accolade,
    /// The competitive rank-update screen ([`RANK_SCREEN`] layouts).
    RankScreen,
}

impl ResultScreen {
    pub const ALL: [ResultScreen; 2] = [ResultScreen::Accolade, ResultScreen::RankScreen];

    /// The built-in layout id of this screen.
    pub fn id(self) -> &'static str {
        match self {
            ResultScreen::Accolade => ACCOLADE,
            ResultScreen::RankScreen => RANK_SCREEN,
        }
    }

    /// Is `id` a layout of this screen: [`Self::id`] itself, or a variant
    /// of it named `<id>_<name>` (the rule [`is_end_title`] uses)?
    pub fn has_layout(self, id: &str) -> bool {
        id.strip_prefix(self.id())
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('_'))
    }
}

/// The result screens a set of templates can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResultScreens {
    pub accolade: bool,
    pub rank_screen: bool,
}

impl ResultScreens {
    pub fn has(self, screen: ResultScreen) -> bool {
        match screen {
            ResultScreen::Accolade => self.accolade,
            ResultScreen::RankScreen => self.rank_screen,
        }
    }
}

impl ResultTemplates {
    /// Can these templates name the word on `screen`? Only with a VICTORY
    /// and a DEFEAT template of that screen's layouts: with one word alone
    /// the screen could only ever say that word.
    pub fn reads_screen(&self, screen: ResultScreen) -> bool {
        let has = |o| {
            self.tpls
                .iter()
                .any(|t| t.outcome == o && screen.has_layout(&t.layout.id))
        };
        has(Outcome::Victory) && has(Outcome::Defeat)
    }

    /// The result screens these templates can name.
    pub fn screens(&self) -> ResultScreens {
        ResultScreens {
            accolade: self.reads_screen(ResultScreen::Accolade),
            rank_screen: self.reads_screen(ResultScreen::RankScreen),
        }
    }

    /// The templates of the result screens alone (every layout of a
    /// [`ResultScreen`]): the first look of [`LoadedResults::read_screens`].
    /// `None` when they can name no screen.
    pub fn screen_words(&self) -> Option<ResultTemplates> {
        // only the result screens' templates are copied
        let tpls: Vec<Template> = self
            .tpls
            .iter()
            .filter(|t| ResultScreen::ALL.iter().any(|s| s.has_layout(&t.layout.id)))
            .cloned()
            .collect();
        let only = ResultTemplates {
            tpls,
            final_score: None,
        };
        let screens = only.screens();
        (screens.accolade || screens.rank_screen).then_some(only)
    }
}

/// One frame's result read for the poller's result screens
/// ([`LoadedResults::read_screens`]), with the screens the templates behind
/// it can name. Under `reader = "new"` this is the poller's accolade and
/// rank-screen outcome signal, and its read is the frame's result evidence
/// (`detect::match_end::ResultWords`).
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenRead {
    /// [`ResultTemplates::read`] of the frame (every layout, certified),
    /// made only when the result screens' own templates found a sure word.
    /// `None`: they did not, and the frame was not read in full.
    pub read: Option<ResultRead>,
    pub screens: ResultScreens,
}

impl ScreenRead {
    /// The word of a sure read that was found on `screen`. `None` for a
    /// flagged read, for a frame that shows no word, and for a sure word
    /// found on another screen's layout (the end title, the other result
    /// screen).
    pub fn word_on(&self, screen: ResultScreen) -> Option<Outcome> {
        let read = self.read.as_ref()?;
        let on_screen = read
            .source
            .as_deref()
            .is_some_and(|id| screen.has_layout(id));
        read.sure().filter(|_| on_screen)
    }
}

// ---------------------------------------------------------------- evidence

/// Every result read of one match, from any source.
#[derive(Debug, Clone, Default)]
pub struct ResultEvidence {
    reads: Vec<ResultRead>,
}

/// What a match's evidence says.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResultDecision {
    pub outcome: Option<Outcome>,
    pub confidence: f32,
    pub suspect: bool,
    /// Sure reads that agree with `outcome`.
    pub agreeing: usize,
    /// Source tags of the reads behind `outcome`, deduplicated.
    pub sources: Vec<String>,
}

impl ResultEvidence {
    /// Add one frame's read. A read that names no word is ignored.
    pub fn add(&mut self, read: ResultRead) {
        if read.outcome.is_some() {
            self.reads.push(read);
        }
    }

    pub fn len(&self) -> usize {
        self.reads.len()
    }

    pub fn is_empty(&self) -> bool {
        self.reads.is_empty()
    }

    /// Most-read word; sure with [`MIN_AGREE`] agreeing sure reads and no read
    /// (sure or flagged) of another word.
    pub fn decide(&self) -> ResultDecision {
        let count = |o: Outcome, sure_only: bool| {
            self.reads
                .iter()
                .filter(|r| r.outcome == Some(o) && (!sure_only || !r.suspect))
                .count()
        };
        let Some(top) = Outcome::ALL
            .into_iter()
            .filter(|&o| count(o, false) > 0)
            .max_by_key(|&o| (count(o, true), count(o, false)))
        else {
            return ResultDecision {
                outcome: None,
                confidence: 0.0,
                suspect: true,
                agreeing: 0,
                sources: Vec::new(),
            };
        };
        let agreeing = count(top, true);
        let conflict = self.reads.iter().any(|r| r.outcome != Some(top));
        let mut sources: Vec<String> = Vec::new();
        for r in self.reads.iter().filter(|r| r.outcome == Some(top)) {
            if let Some(s) = &r.source
                && !sources.contains(s)
            {
                sources.push(s.clone());
            }
        }
        let confidence = self
            .reads
            .iter()
            .filter(|r| r.outcome == Some(top))
            .map(|r| r.score)
            .fold(0.0, f32::max);
        ResultDecision {
            outcome: Some(top),
            confidence,
            suspect: conflict || agreeing < MIN_AGREE,
            agreeing,
            sources,
        }
    }
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    //! Synthetic word templates and frames. Not game art.
    use super::*;
    use image::Rgb;

    /// Synthetic word: vertical bars in a word-specific rhythm.
    pub(crate) fn word(seed: u32, w: usize, h: usize) -> Gray {
        let mut g = Gray::new(w, h);
        for y in 2..h - 2 {
            for x in 0..w {
                let on = (x / (2 + seed as usize)).is_multiple_of(2)
                    || (y + x * seed as usize).is_multiple_of(11);
                if on {
                    g.px[y * w + x] = 0.9;
                }
            }
        }
        g
    }

    pub(crate) fn lay(id: &str) -> Layout {
        Layout::by_id(id).unwrap()
    }

    /// Paint a work-scale word into a 2560x1440 frame at (x0, y0), doubled.
    pub(crate) fn paint_at(img: &mut RgbImage, x0: u32, y0: u32, t: &Gray, colour: [u8; 3]) {
        for y in 0..t.h * 2 {
            for x in 0..t.w * 2 {
                if t.at(x / 2, y / 2) > 0.4 {
                    img.put_pixel(x0 + x as u32, y0 + y as u32, Rgb(colour));
                }
            }
        }
    }

    /// Paint a work-scale word into a 2560x1440 frame inside a layout's region.
    pub(crate) fn paint(img: &mut RgbImage, layout: &Layout, t: &Gray, colour: [u8; 3]) {
        let [fx0, fy0, _, _] = layout.roi;
        let (x0, y0) = ((fx0 * 2560.0) as u32 + 20, (fy0 * 1440.0) as u32 + 10);
        paint_at(img, x0, y0, t, colour);
    }

    /// Write `<id>_<word>.png` files the way a pack ships them.
    pub(crate) fn write_pack(dir: &Path, files: &[(&str, &Gray)]) {
        std::fs::create_dir_all(dir).unwrap();
        for (name, g) in files {
            let img = image::GrayImage::from_fn(g.w as u32, g.h as u32, |x, y| {
                image::Luma([(g.at(x as usize, y as usize) * 255.0) as u8])
            });
            img.save(dir.join(name)).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::*;
    use super::*;
    use image::Rgb;

    fn templates() -> ResultTemplates {
        ResultTemplates::from_templates(vec![
            (lay(ACCOLADE), Outcome::Victory, word(1, 70, 30)),
            (lay(ACCOLADE), Outcome::Defeat, word(3, 60, 30)),
            (lay(RANK_SCREEN), Outcome::Victory, word(5, 66, 20)),
            (lay(RANK_SCREEN), Outcome::Defeat, word(7, 50, 20)),
        ])
        .unwrap()
    }

    fn sure(o: Outcome, src: &str) -> ResultRead {
        ResultRead {
            outcome: Some(o),
            source: Some(src.into()),
            score: 0.9,
            margin: 0.4,
            suspect: false,
        }
    }

    #[test]
    fn regions_are_ocr_v1s_except_the_accolade_name_line() {
        let roi = |id: &str| lay(id).roi;
        // ocr-v1 crops the accolade word to y 9.5%; the nameplate starts at 8.7%
        assert_eq!(roi(ACCOLADE), [0.005, 0.035, 0.255, 0.0862]);
        assert_eq!(roi(RANK_SCREEN), [0.010, 0.145, 0.250, 0.225]);
        assert_eq!(roi(END_TITLE), [0.320, 0.340, 0.680, 0.520]);
        assert_eq!(roi(TAB_HEADER), [0.300, 0.020, 0.700, 0.240]);
    }

    #[test]
    fn accolade_word_reads_in_any_team_colour() {
        let t = templates();
        for colour in [[255, 220, 0], [150, 20, 160], [40, 200, 255]] {
            let mut img = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
            paint(&mut img, &lay(ACCOLADE), &word(3, 60, 30), colour);
            let r = t.read(&img);
            assert_eq!(r.outcome, Some(Outcome::Defeat), "{colour:?}");
            assert_eq!(r.source.as_deref(), Some(ACCOLADE));
            assert!(!r.suspect, "{colour:?}: {r:?}");
        }
    }

    #[test]
    fn rank_screen_defeat_reads() {
        let t = templates();
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([150, 100, 30]));
        paint(
            &mut img,
            &lay(RANK_SCREEN),
            &word(7, 50, 20),
            [245, 245, 245],
        );
        let r = t.read(&img);
        assert_eq!(r.outcome, Some(Outcome::Defeat));
        assert_eq!(r.source.as_deref(), Some(RANK_SCREEN));
        assert!(!r.suspect);
    }

    #[test]
    fn same_screen_reads_at_1080p() {
        let t = templates();
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([150, 100, 30]));
        paint(
            &mut img,
            &lay(RANK_SCREEN),
            &word(7, 50, 20),
            [245, 245, 245],
        );
        let small =
            image::imageops::resize(&img, 1920, 1080, image::imageops::FilterType::Triangle);
        let r = t.read(&small);
        assert_eq!(r.outcome, Some(Outcome::Defeat), "{r:?}");
        assert!(!r.suspect, "{r:?}");
    }

    fn with_draw() -> ResultTemplates {
        ResultTemplates::from_templates(vec![
            (lay(ACCOLADE), Outcome::Victory, word(1, 70, 30)),
            (lay(ACCOLADE), Outcome::Defeat, word(3, 60, 30)),
            (lay(ACCOLADE), Outcome::Draw, word(6, 50, 30)),
        ])
        .unwrap()
    }

    #[test]
    fn draw_template_reads_draw() {
        let t = with_draw();
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
        paint(&mut img, &lay(ACCOLADE), &word(6, 50, 30), [230, 230, 230]);
        let r = t.read(&img);
        assert_eq!(r.outcome, Some(Outcome::Draw), "{r:?}");
        assert!(!r.suspect, "{r:?}");
        assert_eq!(Outcome::Draw.as_str(), "draw");
    }

    /// A DRAW that is present but not sure names no word: it abstains, and
    /// so adds nothing to (and flags nothing in) the match's evidence. The
    /// same read for VICTORY is kept, flagged.
    #[test]
    fn an_unsure_draw_abstains() {
        let t = with_draw();
        let found = |s: [f64; 3]| -> Vec<Found> {
            s.iter().map(|&score| Found { score, macs: 1 }).collect()
        };
        // templates are victory, defeat, draw in load order
        let draw = t.decide(&found([0.40, 0.30, 0.62]));
        assert_eq!(draw.outcome, None, "{draw:?}");
        assert!(draw.suspect);
        let mut e = ResultEvidence::default();
        e.add(sure(Outcome::Victory, ACCOLADE));
        e.add(sure(Outcome::Victory, RANK_SCREEN));
        e.add(draw);
        assert!(!e.decide().suspect, "an abstain flags nothing");
        // a close DRAW also abstains
        assert_eq!(t.decide(&found([0.75, 0.30, 0.80])).outcome, None);
        // a sure DRAW still reads
        let sure_draw = t.decide(&found([0.40, 0.30, 0.90]));
        assert_eq!(
            (sure_draw.outcome, sure_draw.suspect),
            (Some(Outcome::Draw), false)
        );
        // an unsure VICTORY is kept, flagged
        let v = t.decide(&found([0.62, 0.30, 0.40]));
        assert_eq!((v.outcome, v.suspect), (Some(Outcome::Victory), true));
    }

    // ------------------------------------------ the end title on its own (O2)

    fn end_title_plain() -> Layout {
        Layout::new("end_title_plain", lay(END_TITLE).roi, Plane::Max)
    }

    /// Accolade words plus the end titles words-v1 ships: VICTORY, DEFEAT,
    /// DRAW and a second VICTORY variant.
    fn with_end_titles() -> ResultTemplates {
        ResultTemplates::from_templates(vec![
            (lay(ACCOLADE), Outcome::Victory, word(1, 70, 30)),
            (lay(ACCOLADE), Outcome::Defeat, word(3, 60, 30)),
            (lay(END_TITLE), Outcome::Victory, word(2, 90, 34)),
            (lay(END_TITLE), Outcome::Defeat, word(4, 80, 34)),
            (lay(END_TITLE), Outcome::Draw, word(6, 60, 34)),
            (end_title_plain(), Outcome::Victory, word(5, 90, 34)),
        ])
        .unwrap()
    }

    fn dark_frame() -> RgbImage {
        RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]))
    }

    #[test]
    fn end_title_layouts_are_end_title_and_its_variants() {
        for id in [END_TITLE, "end_title_plain", "end_title_x"] {
            assert!(is_end_title(id), "{id}");
        }
        for id in [
            ACCOLADE,
            RANK_SCREEN,
            TAB_HEADER,
            "end_titles",
            "end_titleplain",
            "xend_title",
            "end",
            "",
        ] {
            assert!(!is_end_title(id), "{id}");
        }
    }

    /// The end-title read sees the end-title templates only: another
    /// screen's word neither names the result nor takes the margin away.
    #[test]
    fn the_end_title_is_read_on_its_own() {
        let all = with_end_titles();
        let titles = all.end_titles().expect("end-title templates");
        assert_eq!(titles.sources(), vec![END_TITLE, "end_title_plain"]);

        // an accolade DEFEAT and an end-title VICTORY on one frame
        let mut both = dark_frame();
        paint(&mut both, &lay(ACCOLADE), &word(3, 60, 30), [255, 220, 0]);
        paint(&mut both, &lay(END_TITLE), &word(2, 90, 34), [150, 20, 160]);
        let whole = all.read(&both);
        assert_eq!(whole.sure(), None, "two words: {whole:?}");
        let r = titles.read(&both);
        assert_eq!(r.sure(), Some(Outcome::Victory), "{r:?}");
        assert_eq!(r.source.as_deref(), Some(END_TITLE));

        // the accolade word alone is no end title
        let mut accolade = dark_frame();
        paint(
            &mut accolade,
            &lay(ACCOLADE),
            &word(3, 60, 30),
            [255, 220, 0],
        );
        assert_eq!(all.read(&accolade).sure(), Some(Outcome::Defeat));
        let r = titles.read(&accolade);
        assert_eq!((r.outcome, r.suspect), (None, true), "{r:?}");

        // every end-title template reads, in any colour
        for (layout, seed, w, want) in [
            (lay(END_TITLE), 4, 80, Outcome::Defeat),
            (lay(END_TITLE), 6, 60, Outcome::Draw),
            (end_title_plain(), 5, 90, Outcome::Victory),
        ] {
            let mut img = dark_frame();
            paint(&mut img, &layout, &word(seed, w, 34), [40, 200, 255]);
            let r = titles.read(&img);
            assert_eq!(r.sure(), Some(want), "{r:?}");
            assert_eq!(r.source.as_deref(), Some(layout.id.as_str()));
        }
    }

    /// Only a sure end-title read names a word, and an unsure DRAW names
    /// nothing at all.
    #[test]
    fn an_unsure_end_title_is_not_sure() {
        let titles = with_end_titles().end_titles().unwrap();
        let found = |s: [f64; 4]| -> Vec<Found> {
            s.iter().map(|&score| Found { score, macs: 1 }).collect()
        };
        // templates are victory, defeat, draw, plain victory in load order
        let sure = titles.decide(&found([0.85, 0.30, 0.20, 0.40]));
        assert_eq!(sure.sure(), Some(Outcome::Victory));
        // under the floor
        let low = titles.decide(&found([0.65, 0.30, 0.20, 0.40]));
        assert_eq!((low.outcome, low.sure()), (Some(Outcome::Victory), None));
        // no lead over the best other word
        let close = titles.decide(&found([0.85, 0.80, 0.20, 0.40]));
        assert_eq!(close.sure(), None, "{close:?}");
        // a second VICTORY template is the same word, not a rival
        let twice = titles.decide(&found([0.85, 0.30, 0.20, 0.84]));
        assert_eq!(twice.sure(), Some(Outcome::Victory), "{twice:?}");
        // DRAW: sure or nothing
        let draw = titles.decide(&found([0.40, 0.30, 0.90, 0.20]));
        assert_eq!(draw.sure(), Some(Outcome::Draw));
        let unsure_draw = titles.decide(&found([0.75, 0.30, 0.80, 0.20]));
        assert_eq!((unsure_draw.outcome, unsure_draw.sure()), (None, None));
        // nothing on the frame
        let none = titles.decide(&found([0.20, 0.25, 0.10, 0.15]));
        assert_eq!((none.outcome, none.sure()), (None, None));
    }

    /// A sure read is a lead over the best other word, so the end titles
    /// need a VICTORY and a DEFEAT template of their own.
    #[test]
    fn end_titles_need_a_victory_and_a_defeat() {
        assert!(templates().end_titles().is_none(), "no end-title template");
        let one_word = ResultTemplates::from_templates(vec![
            (lay(ACCOLADE), Outcome::Victory, word(1, 70, 30)),
            (lay(ACCOLADE), Outcome::Defeat, word(3, 60, 30)),
            (lay(END_TITLE), Outcome::Victory, word(2, 90, 34)),
            (end_title_plain(), Outcome::Victory, word(5, 90, 34)),
            (lay(END_TITLE), Outcome::Draw, word(6, 60, 34)),
        ])
        .unwrap();
        assert!(one_word.end_titles().is_none());
    }

    /// Each load cuts the end titles out of the folder it loaded.
    #[test]
    fn a_load_carries_its_end_titles() {
        let data = tempfile::tempdir().unwrap();
        let words = data.path().join(WORDS_SUBDIR);
        write_pack(
            &words,
            &[
                ("accolade_victory.png", &word(1, 70, 30)),
                ("accolade_defeat.png", &word(3, 60, 30)),
            ],
        );
        let dirs = ResultDirs::in_data_dir(data.path());
        assert!(dirs.load().unwrap().end_titles.is_none());
        write_pack(
            &words,
            &[
                ("end_title_victory.png", &word(2, 90, 34)),
                ("end_title_defeat.png", &word(4, 80, 34)),
            ],
        );
        let loaded = dirs.load().unwrap();
        assert_eq!(loaded.pack, ResultPack::WordsV1);
        assert_eq!(loaded.end_titles.unwrap().sources(), vec![END_TITLE]);
    }

    // ------------------------------------------------- result screens (O11)

    #[test]
    fn result_screen_layouts_are_the_id_and_its_variants() {
        use ResultScreen::{Accolade, RankScreen};
        assert_eq!((Accolade.id(), RankScreen.id()), (ACCOLADE, RANK_SCREEN));
        for id in [ACCOLADE, "accolade_wide", "accolade_x"] {
            assert!(Accolade.has_layout(id), "{id}");
            assert!(!RankScreen.has_layout(id), "{id}");
        }
        for id in [RANK_SCREEN, "rank_screen_wide"] {
            assert!(RankScreen.has_layout(id), "{id}");
            assert!(!Accolade.has_layout(id), "{id}");
        }
        for id in [
            END_TITLE,
            "end_title_plain",
            TAB_HEADER,
            "accolades",
            "accoladewide",
            "xaccolade",
            "rank_screens",
            "rank_title",
            "rank",
            "",
        ] {
            for screen in ResultScreen::ALL {
                assert!(!screen.has_layout(id), "{screen:?} {id}");
            }
        }
    }

    /// A screen is named only with a VICTORY and a DEFEAT template of its
    /// own layouts. A DRAW template, or another screen's words, do not count.
    #[test]
    fn a_result_screen_needs_a_victory_and_a_defeat() {
        let both = templates().screens();
        assert_eq!((both.accolade, both.rank_screen), (true, true));
        for screen in ResultScreen::ALL {
            assert!(both.has(screen));
            assert!(!ResultScreens::default().has(screen));
        }
        let accolade_only = with_end_titles().screens();
        assert_eq!(
            (accolade_only.accolade, accolade_only.rank_screen),
            (true, false)
        );
        assert!(accolade_only.has(ResultScreen::Accolade));
        assert!(!accolade_only.has(ResultScreen::RankScreen));
        // one word of a screen, however many other templates
        let one_word = ResultTemplates::from_templates(vec![
            (lay(ACCOLADE), Outcome::Victory, word(1, 70, 30)),
            (lay(ACCOLADE), Outcome::Draw, word(6, 50, 30)),
            (lay(RANK_SCREEN), Outcome::Defeat, word(7, 50, 20)),
            (lay(RANK_SCREEN), Outcome::Draw, word(6, 50, 20)),
            (lay(END_TITLE), Outcome::Victory, word(2, 90, 34)),
            (lay(END_TITLE), Outcome::Defeat, word(4, 80, 34)),
        ])
        .unwrap();
        assert_eq!(one_word.screens(), ResultScreens::default());
        // a variant layout counts for its screen
        let variant = ResultTemplates::from_templates(vec![
            (lay(ACCOLADE), Outcome::Victory, word(1, 70, 30)),
            (
                Layout::new("accolade_wide", lay(ACCOLADE).roi, Plane::Max),
                Outcome::Defeat,
                word(3, 60, 30),
            ),
        ])
        .unwrap();
        assert!(variant.reads_screen(ResultScreen::Accolade));
        assert!(!variant.reads_screen(ResultScreen::RankScreen));
    }

    /// A screen's word is the word of a sure read found on that screen's
    /// layout: not a flagged word, not no word, not another layout's word.
    #[test]
    fn a_screens_word_is_a_sure_read_found_on_that_screen() {
        use ResultScreen::{Accolade, RankScreen};
        let screens = templates().screens();
        let on = |read: ResultRead| {
            let r = ScreenRead {
                read: Some(read),
                screens,
            };
            (r.word_on(Accolade), r.word_on(RankScreen))
        };
        for o in Outcome::ALL {
            assert_eq!(on(sure(o, ACCOLADE)), (Some(o), None));
            assert_eq!(on(sure(o, RANK_SCREEN)), (None, Some(o)));
            assert_eq!(on(sure(o, "accolade_wide")), (Some(o), None));
            assert_eq!(on(sure(o, END_TITLE)), (None, None));
            assert_eq!(on(sure(o, TAB_HEADER)), (None, None));
            for src in [ACCOLADE, RANK_SCREEN] {
                let flagged = ResultRead {
                    suspect: true,
                    ..sure(o, src)
                };
                assert_eq!(on(flagged), (None, None), "{o:?} {src}");
            }
            let no_source = ResultRead {
                source: None,
                ..sure(o, ACCOLADE)
            };
            assert_eq!(on(no_source), (None, None));
        }
        assert_eq!(on(ResultRead::nothing(0.2, 0.1)), (None, None));
        let unread = ScreenRead {
            read: None,
            screens,
        };
        assert_eq!(
            (unread.word_on(Accolade), unread.word_on(RankScreen)),
            (None, None)
        );
    }

    /// Every layout the words pack ships: both result screens and the end
    /// titles.
    fn loaded_with_end_titles() -> LoadedResults {
        let all = ResultTemplates::from_templates(vec![
            (lay(ACCOLADE), Outcome::Victory, word(1, 70, 30)),
            (lay(ACCOLADE), Outcome::Defeat, word(3, 60, 30)),
            (lay(RANK_SCREEN), Outcome::Victory, word(5, 66, 20)),
            (lay(RANK_SCREEN), Outcome::Defeat, word(7, 50, 20)),
            (lay(END_TITLE), Outcome::Victory, word(2, 90, 34)),
            (lay(END_TITLE), Outcome::Defeat, word(4, 80, 34)),
        ])
        .unwrap();
        LoadedResults::new(all, ResultPack::WordsV1, None)
    }

    fn accolade_victory() -> RgbImage {
        let mut img = dark_frame();
        paint(&mut img, &lay(ACCOLADE), &word(1, 70, 30), [255, 220, 0]);
        img
    }

    fn rank_defeat() -> RgbImage {
        let mut img = dark_frame();
        paint(
            &mut img,
            &lay(RANK_SCREEN),
            &word(7, 50, 20),
            [245, 245, 245],
        );
        img
    }

    /// The first look is the result screens' templates alone.
    #[test]
    fn the_screen_words_are_the_result_screens_templates() {
        let loaded = loaded_with_end_titles();
        let words = loaded.screen_words.as_ref().expect("screen words");
        assert_eq!(words.sources(), vec![ACCOLADE, RANK_SCREEN]);
        assert_eq!(loaded.templates.sources().len(), 3);
        // end titles alone name no result screen
        let titles = loaded.end_titles.as_ref().unwrap();
        assert!(titles.screen_words().is_none());
        // one word of each screen names none either
        let one_word = ResultTemplates::from_templates(vec![
            (lay(ACCOLADE), Outcome::Victory, word(1, 70, 30)),
            (lay(RANK_SCREEN), Outcome::Defeat, word(7, 50, 20)),
        ])
        .unwrap();
        assert!(one_word.screen_words().is_none());
        // one whole screen is enough, and the other screen's lone word stays
        let accolade = ResultTemplates::from_templates(vec![
            (lay(ACCOLADE), Outcome::Victory, word(1, 70, 30)),
            (lay(ACCOLADE), Outcome::Defeat, word(3, 60, 30)),
            (lay(RANK_SCREEN), Outcome::Defeat, word(7, 50, 20)),
            (lay(END_TITLE), Outcome::Victory, word(2, 90, 34)),
        ])
        .unwrap();
        let words = accolade.screen_words().unwrap();
        assert_eq!(words.sources(), vec![ACCOLADE, RANK_SCREEN]);
    }

    /// The real reads: a word painted on each screen is that screen's word
    /// and not the other's, and the read behind it is the frame's full
    /// result read.
    #[test]
    fn a_frame_read_names_the_screen_its_word_is_on() {
        use ResultScreen::{Accolade, RankScreen};
        let loaded = loaded_with_end_titles();
        let on = |r: &ScreenRead| (r.word_on(Accolade), r.word_on(RankScreen));

        let accolade = accolade_victory();
        let r = loaded.read_screens(&accolade);
        assert_eq!(on(&r), (Some(Outcome::Victory), None), "{r:?}");
        assert_eq!(r.read, Some(loaded.templates.read(&accolade)));
        assert_eq!(r.screens, loaded.screens);

        let rank = rank_defeat();
        let r = loaded.read_screens(&rank);
        assert_eq!(on(&r), (None, Some(Outcome::Defeat)), "{r:?}");
        assert_eq!(r.read, Some(loaded.templates.read(&rank)));

        // a VICTORY on one screen and a DEFEAT on the other: no sure word
        let mut two = accolade.clone();
        paint(
            &mut two,
            &lay(RANK_SCREEN),
            &word(7, 50, 20),
            [245, 245, 245],
        );
        let r = loaded.read_screens(&two);
        assert_eq!(on(&r), (None, None), "{r:?}");
        assert_eq!(r.read, None, "the first look was not sure: no full read");
    }

    /// A frame with no sure word on a result screen is not read in full:
    /// a blank frame, and a frame that shows only an end title, which the
    /// full read would name.
    #[test]
    fn a_frame_without_a_result_screen_word_is_not_read_in_full() {
        let loaded = loaded_with_end_titles();
        let r = loaded.read_screens(&dark_frame());
        assert_eq!(r.read, None);

        let mut title = dark_frame();
        paint(
            &mut title,
            &lay(END_TITLE),
            &word(4, 80, 34),
            [150, 20, 160],
        );
        assert_eq!(
            loaded.templates.read(&title).sure(),
            Some(Outcome::Defeat),
            "the full read names the end title"
        );
        assert_eq!(loaded.read_screens(&title).read, None);

        // lit texture in both result regions that is no word (not a flat
        // plane, so the first look has something to score)
        let mut busy = dark_frame();
        paint(&mut busy, &lay(ACCOLADE), &blobs(3, 110, 30), [255, 220, 0]);
        paint(
            &mut busy,
            &lay(RANK_SCREEN),
            &glyphs(5, 100, 34),
            [245, 245, 245],
        );
        let first = loaded.screen_words.as_ref().unwrap().read(&busy);
        assert!(first.score > 0.0, "the texture is scored: {first:?}");
        assert_eq!(first.sure(), None, "{first:?}");
        assert_eq!(loaded.read_screens(&busy).read, None);

        // no result-screen templates at all: never read
        let titles_only = LoadedResults::new(
            loaded.end_titles.clone().unwrap(),
            ResultPack::WordsV1,
            None,
        );
        assert_eq!(titles_only.screens, ResultScreens::default());
        assert_eq!(titles_only.read_screens(&accolade_victory()).read, None);
    }

    /// The full read has the last word. An accolade VICTORY with an
    /// end-title DEFEAT on the same frame passes the first look, and the
    /// full read, which sees both words, is not sure: no word.
    #[test]
    fn the_full_read_decides_after_the_first_look() {
        use ResultScreen::{Accolade, RankScreen};
        let loaded = loaded_with_end_titles();
        let mut both = accolade_victory();
        paint(&mut both, &lay(END_TITLE), &word(4, 80, 34), [150, 20, 160]);
        let first = loaded.screen_words.as_ref().unwrap().read(&both);
        assert_eq!(first.sure(), Some(Outcome::Victory), "{first:?}");
        let r = loaded.read_screens(&both);
        let full = loaded.templates.read(&both);
        assert_eq!(full.sure(), None, "two words: {full:?}");
        assert_eq!(r.read, Some(full));
        assert_eq!((r.word_on(Accolade), r.word_on(RankScreen)), (None, None));
    }

    /// The first look loses no word: on every frame, the two looks name
    /// what the full read alone names on each result screen.
    #[test]
    fn the_first_look_loses_no_word_of_the_full_read() {
        let loaded = loaded_with_end_titles();
        let mut frames = vec![dark_frame(), accolade_victory(), rank_defeat()];
        for (layout, seed, w, h, colour) in [
            (ACCOLADE, 3, 60, 30, [40, 200, 255]),
            (RANK_SCREEN, 5, 66, 20, [245, 245, 245]),
            (END_TITLE, 2, 90, 34, [150, 20, 160]),
            (END_TITLE, 4, 80, 34, [255, 220, 0]),
        ] {
            let mut img = dark_frame();
            paint(&mut img, &lay(layout), &word(seed, w, h), colour);
            // each word alone, and with every word painted before it
            let mut stacked = frames.last().unwrap().clone();
            paint(&mut stacked, &lay(layout), &word(seed, w, h), colour);
            frames.push(img);
            frames.push(stacked);
        }
        let mut named = 0;
        for (i, frame) in frames.iter().enumerate() {
            let full = ScreenRead {
                read: Some(loaded.templates.read(frame)),
                screens: loaded.screens,
            };
            let two_looks = loaded.read_screens(frame);
            for screen in ResultScreen::ALL {
                assert_eq!(
                    two_looks.word_on(screen),
                    full.word_on(screen),
                    "frame {i} {screen:?}"
                );
                named += usize::from(full.word_on(screen).is_some());
            }
        }
        assert!(named >= 4, "the frames name a word on a screen: {named}");
    }

    /// Each load says which result screens the folder it loaded can name.
    #[test]
    fn a_load_carries_its_result_screens() {
        let data = tempfile::tempdir().unwrap();
        let words = data.path().join(WORDS_SUBDIR);
        write_pack(
            &words,
            &[
                ("accolade_victory.png", &word(1, 70, 30)),
                ("accolade_defeat.png", &word(3, 60, 30)),
                ("rank_screen_victory.png", &word(5, 66, 20)),
            ],
        );
        let dirs = ResultDirs::in_data_dir(data.path());
        let loaded = dirs.load().unwrap();
        assert_eq!(
            (loaded.screens.accolade, loaded.screens.rank_screen),
            (true, false)
        );
        assert_eq!(loaded.screen_words.unwrap().screens(), loaded.screens);
        write_pack(&words, &[("rank_screen_defeat.png", &word(7, 50, 20))]);
        let loaded = dirs.load().unwrap();
        assert_eq!(
            loaded.screens,
            ResultScreens {
                accolade: true,
                rank_screen: true
            }
        );
        assert_eq!(loaded.screen_words.unwrap().sources().len(), 2);
    }

    #[test]
    fn blank_frame_has_no_result() {
        let r = templates().read(&RgbImage::from_pixel(1920, 1080, Rgb([20, 26, 50])));
        assert_eq!(r.outcome, None);
        assert!(r.suspect);
    }

    #[test]
    fn needs_victory_and_defeat_to_load() {
        let one = vec![(lay(ACCOLADE), Outcome::Victory, word(1, 20, 10))];
        assert!(ResultTemplates::from_templates(one).is_none());
        assert!(ResultTemplates::load_dir(Path::new("/nonexistent/result")).is_none());
    }

    #[test]
    fn one_sure_read_is_not_enough() {
        let mut e = ResultEvidence::default();
        e.add(sure(Outcome::Victory, RANK_SCREEN));
        let d = e.decide();
        assert_eq!(d.outcome, Some(Outcome::Victory));
        assert!(d.suspect);
        e.add(sure(Outcome::Victory, RANK_SCREEN));
        assert!(!e.decide().suspect);
    }

    #[test]
    fn accolade_read_is_a_second_look_at_the_same_result() {
        let mut e = ResultEvidence::default();
        e.add(sure(Outcome::Defeat, RANK_SCREEN));
        e.add(sure(Outcome::Defeat, ACCOLADE));
        let d = e.decide();
        assert_eq!(
            (d.outcome, d.suspect, d.agreeing),
            (Some(Outcome::Defeat), false, 2)
        );
        assert_eq!(
            d.sources,
            vec![RANK_SCREEN.to_string(), ACCOLADE.to_string()]
        );
    }

    #[test]
    fn any_conflicting_read_flags_the_match() {
        let mut e = ResultEvidence::default();
        e.add(sure(Outcome::Victory, RANK_SCREEN));
        e.add(sure(Outcome::Victory, RANK_SCREEN));
        let mut odd = sure(Outcome::Defeat, ACCOLADE);
        odd.suspect = true;
        e.add(odd);
        let d = e.decide();
        assert_eq!(d.outcome, Some(Outcome::Victory));
        assert!(d.suspect);
        // a frame without a word adds nothing
        let mut e2 = ResultEvidence::default();
        e2.add(ResultRead::nothing(0.1, 0.0));
        assert!(e2.is_empty());
        assert_eq!(e2.decide().outcome, None);
    }

    #[test]
    fn a_new_screen_plugs_in_through_layouts_json() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(
            dir.path(),
            &[
                ("cards_victory.png", &word(1, 70, 30)),
                ("cards_defeat.png", &word(3, 60, 30)),
            ],
        );
        std::fs::write(
            dir.path().join("layouts.json"),
            r#"[{"id": "cards", "roi": [0.40, 0.40, 0.60, 0.50], "plane": "max"}]"#,
        )
        .unwrap();
        let t = ResultTemplates::load_dir(dir.path()).expect("plugged-in layout loads");
        assert_eq!(t.sources(), vec!["cards"]);
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
        let c = Layout {
            id: "cards".into(),
            roi: [0.40, 0.40, 0.60, 0.50],
            plane: Plane::Max,
        };
        paint(&mut img, &c, &word(1, 70, 30), [200, 40, 40]);
        let r = t.read(&img);
        assert_eq!(r.outcome, Some(Outcome::Victory));
        assert_eq!(r.source.as_deref(), Some("cards"));
    }

    // ------------------------------------------------ name line (O1 item 5)

    /// A synthetic name line: dense short strokes, unlike any word template.
    fn name_line(img: &mut RgbImage, gh: f64, colour: [u8; 3]) {
        let y0 = (NAME_LINE_TOP * gh) as u32;
        let y1 = (0.113 * gh) as u32;
        let gw = gh * 16.0 / 9.0;
        for y in y0..y1 {
            for x in (0.027 * gw) as u32..(0.088 * gw) as u32 {
                if (x / 3 + y / 5).is_multiple_of(2) {
                    img.put_pixel(x, y, Rgb(colour));
                }
            }
        }
    }

    /// The accolade region stops above the member's name line (measured on
    /// real frames): a name drawn there leaves the region's pixels and every
    /// accolade score exactly as they were, at 1440p and 1080p. ocr-v1's crop
    /// would hold the name's top rows.
    #[test]
    fn the_accolade_region_excludes_the_name_line() {
        let a = lay(ACCOLADE);
        assert!(a.roi[3] < NAME_LINE_TOP, "{:?}", a.roi);
        let t = templates();
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
        // the word where real frames have it: y 4.0-8.4%
        paint_at(&mut img, 64, 58, &word(1, 70, 30), [255, 220, 0]);
        let before = t.scores(&img, true);
        let read = t.read(&img);
        assert_eq!(read.outcome, Some(Outcome::Victory), "{read:?}");
        for colour in [[255, 255, 255], [255, 220, 0]] {
            let mut named = img.clone();
            name_line(&mut named, 1440.0, colour);
            assert_eq!(t.scores(&named, true), before, "{colour:?}");
            assert_eq!(t.read(&named), read);
        }
        // the region's pixels are untouched by the name, and the old ocr-v1
        // crop (to 9.5%) would hold it
        let px = |img: &RgbImage, roi| region(img, roi, Plane::Max).unwrap()[0].px.clone();
        let mut named = img.clone();
        name_line(&mut named, 1440.0, [255, 255, 255]);
        assert_eq!(px(&named, a.roi), px(&img, a.roi));
        let old = [0.005, 0.035, 0.255, 0.095];
        assert_ne!(px(&named, old), px(&img, old));
        // the same at 1080p
        let (small, small_named) = (
            image::imageops::resize(&img, 1920, 1080, image::imageops::FilterType::Nearest),
            image::imageops::resize(&named, 1920, 1080, image::imageops::FilterType::Nearest),
        );
        let mut small_named2 = small.clone();
        name_line(&mut small_named2, 1080.0, [255, 255, 255]);
        assert_eq!(px(&small_named2, a.roi), px(&small, a.roi));
        assert_ne!(px(&small_named, old), px(&small, old));
    }

    /// F253-4: a pack's `layouts.json` cannot bring the nameplate back. An
    /// `accolade` override that runs to ocr-v1's 9.5% is cut at
    /// [`ACCOLADE_Y1`]; a layout that starts inside the nameplate is dropped.
    #[test]
    fn a_layouts_json_cannot_reach_the_nameplate() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(
            dir.path(),
            &[
                ("accolade_victory.png", &word(1, 70, 30)),
                ("accolade_defeat.png", &word(3, 60, 30)),
                ("cards_victory.png", &word(5, 40, 12)),
                ("cards_defeat.png", &word(7, 40, 12)),
            ],
        );
        std::fs::write(
            dir.path().join("layouts.json"),
            r#"[{"id": "accolade", "roi": [0.005, 0.035, 0.255, 0.095], "plane": "max"},
                {"id": "cards", "roi": [0.02, 0.09, 0.20, 0.112], "plane": "max"}]"#,
        )
        .unwrap();
        let t = ResultTemplates::load_dir(dir.path()).expect("loads, clamped");
        assert_eq!(
            t.layout(ACCOLADE).unwrap().roi,
            [0.005, 0.035, 0.255, ACCOLADE_Y1]
        );
        assert!(t.layout("cards").is_none(), "{:?}", t.sources());
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
        paint_at(&mut img, 64, 58, &word(1, 70, 30), [255, 220, 0]);
        let mut named = img.clone();
        name_line(&mut named, 1440.0, [255, 255, 255]);
        assert_eq!(t.scores(&named, true), t.scores(&img, true));
        assert_eq!(t.read(&img).outcome, Some(Outcome::Victory));
    }

    /// F253-5: layout ids are file stems. One that could leave the pack
    /// folder (or is not a words-v1 word id), or a region outside the game
    /// rect, refuses the folder, and the reader falls back to the old one.
    #[test]
    fn a_hostile_layouts_json_refuses_the_folder() {
        let long = "x".repeat(65);
        let bad_ids = [
            "../evil",
            "/tmp/evil",
            "a/b",
            "..",
            ".",
            "Evil",
            "",
            &long,
            "e vil",
            "e\\\\vil",
        ];
        let bad_rois = [
            "[0.5, 0.5, 0.4, 0.6]",
            "[-0.1, 0.4, 0.2, 0.6]",
            "[0.0, 0.0, 1.5, 1.0]",
            "[0.4, 0.6, 0.6, 0.6]",
        ];
        let cases = bad_ids
            .iter()
            .map(|id| format!(r#"[{{"id": "{id}", "roi": [0.4, 0.4, 0.6, 0.5], "plane": "max"}}]"#))
            .chain([r#"[{"id": "cards", "roi": [0.4, 0.4], "plane": "max"}]"#.to_string()])
            .chain(
                bad_rois
                    .iter()
                    .map(|roi| format!(r#"[{{"id": "cards", "roi": {roi}, "plane": "max"}}]"#)),
            );
        for (k, json) in cases.enumerate() {
            let data = tempfile::tempdir().unwrap();
            let words = data.path().join(WORDS_SUBDIR);
            pack(&words, 1);
            // what `../evil` would open, one level above the pack folder
            write_pack(
                &data.path().join("templates"),
                &[
                    ("evil_victory.png", &word(9, 70, 30)),
                    ("evil_defeat.png", &word(4, 60, 30)),
                ],
            );
            std::fs::write(words.join("layouts.json"), &json).unwrap();
            assert!(
                ResultTemplates::load_dir(&words).is_none(),
                "case {k}: {json}"
            );
            pack(&data.path().join(TEMPLATE_SUBDIR), 5);
            let live = LiveResults::load(ResultDirs::in_data_dir(data.path()));
            assert_eq!(
                reads_victory(&live, 5),
                (Some(ResultPack::Legacy), true),
                "case {k}"
            );
        }
        // the same folder with a plain id loads
        let dir = tempfile::tempdir().unwrap();
        pack(dir.path(), 1);
        std::fs::write(
            dir.path().join("layouts.json"),
            r#"[{"id": "end_title_2", "roi": [0.4, 0.4, 0.6, 0.5], "plane": "max"}]"#,
        )
        .unwrap();
        assert!(ResultTemplates::load_dir(dir.path()).is_some());
    }

    // ---------------------------------------------- fast search (O1 item 2)

    /// The kept-variance rule the parity tests build stacks with: looser
    /// than [`MIN_KEPT`], so the synthetic glyphs reach the 4x level.
    const TEST_KEPT: f64 = 0.6;

    /// Deterministic xorshift.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, m: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % m.max(1)
        }
    }

    /// A work-scale region `rw` x `rh` of noise on a random background,
    /// with `t` drawn at a random spot and size (one of the search scales)
    /// and a second, different word beside it as a distractor.
    fn random_region(rng: &mut Rng, t: &Gray, other: &Gray, rw: usize, rh: usize) -> Gray {
        let bg = rng.next(40) as f32 / 255.0;
        let mut g = Gray::new(rw, rh);
        for v in g.px.iter_mut() {
            *v = bg + rng.next(30) as f32 / 255.0;
        }
        let ink = 0.55 + rng.next(45) as f32 / 100.0;
        let word_t = t;
        let mut draw = |t: &Gray, rng: &mut Rng| {
            let sc = if std::ptr::eq(t, word_t) {
                1.0
            } else {
                SCALES[rng.next(3) as usize]
            };
            let tw = ((t.w as f64 * sc).round() as usize).min(rw);
            let th = ((t.h as f64 * sc).round() as usize).min(rh);
            let s = t.resize(tw, th);
            let (x0, y0) = (
                rng.next((rw - tw) as u64) as usize,
                rng.next((rh - th) as u64) as usize,
            );
            for y in 0..th {
                for x in 0..tw {
                    let a = s.at(x, y) / 0.9;
                    let p = &mut g.px[(y0 + y) * rw + x0 + x];
                    *p = (*p * (1.0 - a) + ink * a).min(1.0);
                }
            }
        };
        draw(other, rng);
        draw(t, rng);
        g
    }

    /// Template sizes of the real packs at the work scale (accolade, rank
    /// screen) and two that build the 4x level (end-title shape, smaller so
    /// the exhaustive reference stays quick in a debug build).
    const SIZES: [(usize, usize, usize, usize); 4] = [
        (149, 32, 230, 37),
        (131, 39, 230, 57),
        (120, 50, 230, 90),
        (100, 48, 210, 100),
    ];

    /// Synthetic italic "letters" of thick strokes, like title text (the
    /// bar rhythm of [`word`] aliases at 4x and is harder than any real
    /// word; [`fast_search_matches_on_thin_bars`] covers it too).
    fn glyphs(seed: u64, w: usize, h: usize) -> Gray {
        let mut rng = Rng(seed.wrapping_mul(0x2545_f491_4f6c_dd1d) | 1);
        let mut g = Gray::new(w, h);
        let n = (w * 10 / (h * 6)).max(3);
        let cw = w / n;
        let t = (h / 6).max(2);
        for i in 0..n {
            let x0 = i * cw + cw / 8;
            let x1 = (i + 1) * cw - cw / 8;
            let strokes = 2 + rng.next(3);
            for _ in 0..strokes {
                let kind = rng.next(5);
                for y in 2..h - 2 {
                    let skew = (h - y) / 5;
                    for x in x0..x1 {
                        let on = match kind {
                            0 => x < x0 + t,
                            1 => x + t >= x1,
                            2 => y < 2 + t,
                            3 => y + t + 2 >= h,
                            _ => (y as isize - h as isize / 2).unsigned_abs() < t / 2 + 1,
                        };
                        let xs = (x + skew).min(w - 1);
                        if on {
                            g.px[y * w + xs] = 0.9;
                        }
                    }
                }
            }
        }
        g
    }

    /// One parity case: `t` and a distractor in a noisy region; both
    /// templates searched both ways. Returns (present, deep) counts.
    fn parity_case(
        rng: &mut Rng,
        case: usize,
        t: &Gray,
        other: &Gray,
        rw: usize,
        rh: usize,
    ) -> (usize, usize) {
        let p = pyramid(random_region(rng, t, other, rw, rh));
        let (mut present, mut deep) = (0, 0);
        for tpl in [t, other] {
            // a looser kept rule than production, so these synthetic words
            // reach the 4x level: the search must be exact on any stack
            let st = Stack::with_kept(tpl, TEST_KEPT).unwrap();
            deep += usize::from(st.levels.len() == 3);
            let (fast, full) = (search_fast(&p, &st), search_full(&p, &st));
            assert!(
                fast.score <= full.score + 1e-9,
                "case {case}: {fast:?} > {full:?}"
            );
            if full.score >= PRESENT as f64 {
                present += 1;
                assert!(
                    (fast.score - full.score).abs() < 1e-9,
                    "case {case}: fast {fast:?} full {full:?}"
                );
            }
        }
        (present, deep)
    }

    /// The fast search never scores above the exhaustive one, and finds the
    /// exhaustive best score whenever that score is a present word. Checked
    /// for the word's own template and for another word's template (a
    /// cross match), at every template size the packs use.
    #[test]
    fn fast_search_matches_the_exhaustive_search() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let (mut present, mut deep) = (0, 0);
        for case in 0..16 {
            let (tw, th, rw, rh) = SIZES[case % SIZES.len()];
            let t = glyphs(case as u64, tw, th);
            let other = glyphs(case as u64 + 100, tw * 9 / 10, th);
            let (p, d) = parity_case(&mut rng, case, &t, &other, rw, rh);
            present += p;
            deep += d;
        }
        assert!(present >= 16, "each case has its word present: {present}");
        assert!(deep >= 16, "the 4x level is exercised: {deep}");
    }

    /// The same check with thin-bar words, which average out at 4x: their
    /// pyramid stops early ([`MIN_KEPT`]), and the search stays exact.
    #[test]
    fn fast_search_matches_on_thin_bars() {
        let mut rng = Rng(0x5851_f42d_4c95_7f2d);
        let (mut present, mut deep) = (0, 0);
        for case in 0..12 {
            let (tw, th, rw, rh) = SIZES[case % SIZES.len()];
            let seed = 1 + (case as u32 % 9);
            let t = word(seed, tw, th);
            let other = word(seed % 9 + 2, tw * 9 / 10, th);
            let (p, d) = parity_case(&mut rng, case, &t, &other, rw, rh);
            present += p;
            deep += d;
        }
        assert!(present >= 12, "{present}");
        // glyph words of these sizes build 16 deep stacks in 16 cases
        assert!(deep < 8, "thin bars keep too little at 4x: {deep}");
        assert_eq!(
            Stack::with_kept(&glyphs(1, 120, 50), TEST_KEPT)
                .unwrap()
                .levels
                .len(),
            3
        );
    }

    /// A blocky copy of the word (each coarse pixel of the template blown up
    /// to a square, like a low-resolution or heavily compressed title) is
    /// the template itself on the coarse level and beats the sharp, noisy
    /// word there; at full resolution the sharp word wins. The fast search
    /// refines more than the best coarse peak, so it still finds it.
    #[test]
    fn fast_search_looks_past_a_coarse_decoy() {
        let mut rng = Rng(4242);
        for (k, &(tw, th, rw, rh)) in SIZES.iter().enumerate() {
            let t = glyphs(k as u64 + 50, tw, th);
            let st = Stack::new(&t).unwrap();
            let top = st.levels.len() - 1;
            let f = 1usize << top;
            let (mut cw, mut ch, mut cpx) = (t.w, t.h, t.px.clone());
            for _ in 0..top {
                (cw, ch, cpx) = half(cw, ch, &cpx);
            }
            let (rw, rh) = (
                (rw + tw + 4 * f).next_multiple_of(f),
                (rh + 4 * f).next_multiple_of(f),
            );
            let mut g = Gray::new(rw, rh);
            // the sharp word, with pixel noise, on the coarse grid
            let (sx, sy) = (2 * f, 2 * f);
            for y in 0..th {
                for x in 0..tw {
                    g.px[(sy + y) * rw + sx + x] = t.at(x, y) * 0.8 + rng.next(60) as f32 / 255.0;
                }
            }
            // the blocky copy, right of it, on the coarse grid
            let (dx, dy) = ((sx + tw + 2 * f).next_multiple_of(f), f);
            for y in 0..ch * f {
                for x in 0..cw * f {
                    g.px[(dy + y) * rw + dx + x] = cpx[(y / f) * cw + x / f] * 0.8;
                }
            }
            let p = pyramid(g);
            let coarse =
                |x: usize, y: usize| ncc_at(&p[top], &st.levels[top], x / f, y / f).unwrap();
            assert!(
                coarse(dx, dy) > coarse(sx, sy),
                "{k}: the decoy leads on the coarse level"
            );
            let (fast, full) = (search_fast(&p, &st), search_full(&p, &st));
            let sharp = ncc_at(&p[0], &st.levels[0], sx, sy).unwrap();
            assert!(sharp > ncc_at(&p[0], &st.levels[0], dx, dy).unwrap(), "{k}");
            assert!((full.score - sharp).abs() < 1e-9, "{k}: {full:?} {sharp}");
            assert!(
                (fast.score - full.score).abs() < 1e-9,
                "{k}: fast {fast:?} full {full:?}"
            );
        }
    }

    // ------------------------------------------- the kernel's exact bits (V1)

    /// A dot product one element at a time, in a chosen order: products go
    /// to `lanes` sums by index modulo `lanes` over the whole chunks, the
    /// lane sums fold in pairs (`(0 + 1) + (2 + 3)`, and so on up) or one
    /// after the other, and the leftover elements are added one by one.
    /// `fused` rounds each product and its addition once, as a fused
    /// multiply-add instruction does.
    fn dot_in_order(a: &[f32], b: &[f32], lanes: usize, pairs: bool, fused: bool) -> f32 {
        let step = |s: f32, x: f32, y: f32| if fused { x.mul_add(y, s) } else { s + x * y };
        let whole = a.len() / lanes * lanes;
        let mut lane = vec![0.0f32; lanes];
        for i in 0..whole {
            lane[i % lanes] = step(lane[i % lanes], a[i], b[i]);
        }
        if pairs {
            while lane.len() > 1 {
                lane = lane.chunks(2).map(|c| c[0] + c[1]).collect();
            }
        }
        let mut s = lane.iter().skip(1).fold(lane[0], |s, v| s + v);
        for i in whole..a.len() {
            s = step(s, a[i], b[i]);
        }
        s
    }

    /// [`dot`]'s order: 8 lanes, folded in pairs, no fused multiply-add.
    fn dot_reference(a: &[f32], b: &[f32]) -> f32 {
        dot_in_order(a, b, 8, true, false)
    }

    /// [`ncc_at`]'s score from [`dot_reference`]: the rows' sums added in
    /// f64, top row first, over the root of the window's variance sum times
    /// the kernel's norm.
    fn ncc_reference(p: &Level, k: &Kernel, x: usize, y: usize) -> f64 {
        let n = (k.w * k.h) as f64;
        let sum = p.rect(&p.s1, x, y, k.w, k.h);
        let var = p.rect(&p.s2, x, y, k.w, k.h) - sum * sum / n;
        let mut num = 0.0f64;
        for ty in 0..k.h {
            let row = &p.px[(y + ty) * p.w + x..][..k.w];
            num += dot_reference(row, &k.z[ty * k.w..][..k.w]) as f64;
        }
        num / (var.sqrt() * k.norm)
    }

    /// A score is a sum of floats, so the order of the additions is part
    /// of it, and thresholds are compared with the last bit. This pins the
    /// order: [`dot`] returns the bits of the plain loop of
    /// [`dot_reference`] on every length up to 100, with values that
    /// cancel, overflow to infinity or underflow. A faster `dot` (wider
    /// registers, several rows at a time) has to pass this unchanged, so
    /// that no read can move.
    ///
    /// The inputs tell the orders apart: the same sum in 4 or 16 lanes,
    /// folded one lane after the other, added left to right, or with fused
    /// multiply-adds gives other bits on many of them, and the test checks
    /// that it does.
    #[test]
    fn dot_is_the_sum_in_one_fixed_order() {
        let mut rng = Rng(0x0d07_0d07);
        let mut value = |kind: u64| -> f32 {
            let u = rng.next(1 << 24) as f32 / (1 << 24) as f32;
            match kind {
                // a pixel, a zero-mean template value
                0 => u,
                1 => u - 0.5,
                // 8-bit values
                2 => (rng.next(256) as f32) / 255.0,
                // big and small magnitudes, both signs
                3 => (u - 0.5) * [1e-20, 1e-6, 1.0, 1e6, 1e20][rng.next(5) as usize],
                // the largest values: sums overflow to infinity
                4 => [f32::MAX, -f32::MAX, 1.0, 0.0][rng.next(4) as usize],
                // the smallest: products underflow
                _ => [f32::MIN_POSITIVE, 1e-30, -1e-30, 1.0][rng.next(4) as usize],
            }
        };
        let same = |x: f32, y: f32| x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan());
        // other orders: 4 lanes, 16 lanes, lanes folded one after the
        // other, left to right (1 lane), fused multiply-adds
        let others = [
            (4, true, false),
            (16, true, false),
            (8, false, false),
            (1, true, false),
            (8, true, true),
        ];
        let mut differ = [0usize; 5];
        let mut checked = 0;
        for len in 0..=100 {
            for kind in 0..36u64 {
                let a: Vec<f32> = (0..len).map(|_| value(kind / 6)).collect();
                let b: Vec<f32> = (0..len).map(|_| value(kind % 6)).collect();
                let (got, want) = (dot(&a, &b), dot_reference(&a, &b));
                assert!(
                    same(got, want),
                    "len {len} kind {kind}: {got:e} is not {want:e}"
                );
                checked += 1;
                for (n, &(lanes, pairs, fused)) in differ.iter_mut().zip(&others) {
                    *n += usize::from(!same(dot_in_order(&a, &b, lanes, pairs, fused), want));
                }
            }
        }
        assert_eq!(checked, 101 * 36);
        // measured: 1140, 1001, 728, 1486 and 603 of the 3636 inputs
        assert!(differ.iter().all(|&n| n > 300), "{differ:?}");
    }

    /// [`ncc_at`] is [`ncc_reference`] to the last bit wherever it gives a
    /// score: templates 1 pixel wide and 1 pixel high, widths around the
    /// chunk of 8, the pack shapes, on noise, on planes of the largest
    /// value and on low-contrast planes.
    #[test]
    fn ncc_at_is_the_reference_kernel_bit_for_bit() {
        let mut rng = Rng(0x5eed_0b17);
        let shapes = [
            (1, 1),
            (1, 24),
            (24, 1),
            (7, 5),
            (8, 8),
            (9, 3),
            (15, 4),
            (16, 6),
            (17, 9),
            (31, 12),
            (33, 2),
            (64, 20),
            (86, 25),
            (149, 32),
        ];
        let mut scored = 0;
        for &(tw, th) in &shapes {
            let mut t = Gray::new(tw, th);
            for v in t.px.iter_mut() {
                *v = rng.next(256) as f32 / 255.0;
            }
            // a 1 x 1 template is flat: no kernel
            let Some(k) = Kernel::new(tw, th, &t.px) else {
                assert_eq!((tw, th), (1, 1));
                continue;
            };
            let (rw, rh) = (tw + 9, th + 7);
            for kind in 0..6 {
                let mut g = Gray::new(rw, rh);
                for v in g.px.iter_mut() {
                    *v = match kind {
                        0 => rng.next(256) as f32 / 255.0,
                        1 => rng.next(1 << 24) as f32 / (1 << 24) as f32,
                        // the largest value, with a few black pixels
                        2 => f32::from(rng.next(50) != 0),
                        // two neighbouring bright grey levels
                        3 => (242 + rng.next(2)) as f32 / 255.0,
                        // dark, a few bright pixels
                        4 => f32::from(rng.next(40) == 0),
                        // sub-grey-level noise on a bright plane
                        _ => 0.9 + 1e-3 * rng.next(256) as f32 / 255.0,
                    };
                }
                let p = Level::new(rw, rh, g.px);
                for y in 0..=rh - th {
                    for x in 0..=rw - tw {
                        if let Some(v) = ncc_at(&p, &k, x, y) {
                            let want = ncc_reference(&p, &k, x, y);
                            assert_eq!(
                                v.to_bits(),
                                want.to_bits(),
                                "{tw}x{th} kind {kind} ({x},{y}): {v} is not {want}"
                            );
                            scored += 1;
                        }
                    }
                }
            }
        }
        // measured: 5691
        assert!(scored > 5_000, "{scored}");
    }

    // ------------------------------------------- certification (F253-3)

    /// The projection bound is never below the exact score, at every
    /// window, for 2x and 4x stacks, glyph and thin-bar words.
    #[test]
    fn the_certification_bound_is_never_below_the_score() {
        let mut rng = Rng(31337);
        let mut checked = 0;
        for case in 0..8 {
            let (tw, th) = [(60, 24), (100, 48)][case % 2];
            let t = if case % 4 < 2 {
                glyphs(case as u64 + 200, tw, th)
            } else {
                word(1 + case as u32, tw, th)
            };
            let other = glyphs(case as u64 + 300, tw * 9 / 10, th);
            let p = pyramid(random_region(&mut rng, &t, &other, tw + 40, th + 24));
            for tpl in [&t, &other] {
                let st = Stack::new(tpl).unwrap();
                assert!(!st.projs.is_empty());
                for pr in &st.projs {
                    let ph = Phased::new(&p[0], pr.f);
                    let k = &st.levels[0];
                    for y in 0..=p[0].h - k.h {
                        for x in 0..=p[0].w - k.w {
                            if let (Some(ub), Some(v)) =
                                (bound_at(&p[0], &ph, k, pr, x, y), ncc_at(&p[0], k, x, y))
                            {
                                assert!(
                                    ub + 1e-6 >= v,
                                    "case {case} f {} ({x},{y}): bound {ub} < {v}",
                                    pr.f
                                );
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(checked > 10_000, "{checked}");
    }

    /// `certify` gives the exhaustive search's answer against any
    /// threshold: `Above` exactly when the exhaustive best reaches it.
    #[test]
    fn certify_agrees_with_the_exhaustive_search() {
        let mut rng = Rng(271828);
        for case in 0..8 {
            let (tw, th, rw, rh) = SIZES[case % SIZES.len()];
            let t = glyphs(case as u64 + 400, tw, th);
            let other = glyphs(case as u64 + 500, tw * 9 / 10, th);
            let p = pyramid(random_region(&mut rng, &t, &other, rw, rh));
            for tpl in [&t, &other] {
                let st = Stack::new(tpl).unwrap();
                let phs: Vec<Phased> = st.projs.iter().map(|pr| Phased::new(&p[0], pr.f)).collect();
                let chain: Vec<(&Proj, &Phased)> = st.projs.iter().zip(&phs).collect();
                let full = search_full(&p, &st).score;
                for tau in [0.3, 0.5, full - 0.02, full + 1e-3] {
                    let mut budget = usize::MAX;
                    let c = certify(&p, &chain, &st, tau, &mut budget);
                    match c {
                        Cert::Above(v) => assert!(full >= tau && v >= tau && v <= full + 1e-9),
                        Cert::Below => assert!(full < tau, "case {case} tau {tau}: full {full}"),
                        Cert::Unknown => panic!("unbounded budget"),
                    }
                }
                // no budget: any window that needs an exact score stops it
                if full >= 0.5 {
                    assert_eq!(certify(&p, &chain, &st, full - 0.02, &mut 0), Cert::Unknown);
                }
            }
        }
    }

    /// Two present words, VICTORY and DEFEAT, in one region: the exhaustive
    /// read is suspect (no margin). If the fast search under-read DEFEAT,
    /// its scores alone would say sure; the certification finds DEFEAT and
    /// flags the read. With no budget for exact scores it is flagged too.
    #[test]
    fn an_under_read_runner_up_never_makes_a_read_sure() {
        let title = Layout::new(END_TITLE, [0.40, 0.40, 0.62, 0.56], Plane::Max);
        let (a, b) = (glyphs(601, 100, 48), glyphs(602, 90, 48));
        let t = ResultTemplates::from_templates(vec![
            (title.clone(), Outcome::Victory, a.clone()),
            (title.clone(), Outcome::Defeat, b.clone()),
        ])
        .unwrap();
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
        paint_at(&mut img, 1050, 600, &a, [240, 240, 240]);
        paint_at(&mut img, 1290, 650, &b, [240, 240, 240]);
        let exhaustive = t.read_exhaustive(&img);
        assert!(exhaustive.suspect, "{exhaustive:?}");
        let mut regions = t.regions(&img);
        let fast = t.scores_in(&regions, false);
        assert!(
            fast[1].score > 0.9,
            "the fast search finds DEFEAT: {fast:?}"
        );
        // the same frame, as if the fast search had missed DEFEAT
        let mut under = fast.clone();
        under[1].score = 0.2;
        let uncertified = t.decide(&under);
        assert!(
            !uncertified.suspect,
            "the fast scores alone: {uncertified:?}"
        );
        let r = t.settle(&mut regions, under.clone(), CERT_MACS);
        assert!(r.suspect, "{r:?}");
        assert!(r.margin < MARGIN, "{r:?}");
        let r = t.settle(&mut regions, under, 0);
        assert!(r.suspect, "no budget: {r:?}");
        // DEFEAT gone: the read is sure and stays sure
        let mut one = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
        paint_at(&mut one, 1050, 600, &a, [240, 240, 240]);
        let r = t.read(&one);
        assert!(r.outcome == Some(Outcome::Victory) && !r.suspect, "{r:?}");
        assert_eq!(r, t.read_exhaustive(&one));
    }

    /// Random blocks of ink, a third kind of synthetic word.
    fn blobs(seed: u64, w: usize, h: usize) -> Gray {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9) | 1);
        let mut g = Gray::new(w, h);
        for _ in 0..(w / 6).max(4) {
            let (bw, bh) = (
                2 + rng.next(10) as usize,
                2 + rng.next((h / 2) as u64) as usize,
            );
            let (x0, y0) = (
                rng.next((w - bw) as u64) as usize,
                rng.next((h - bh) as u64) as usize,
            );
            for y in y0..y0 + bh {
                for x in x0..x0 + bw {
                    g.px[y * w + x] = 0.9;
                }
            }
        }
        g
    }

    /// Parity stress, run by hand: `O1_STRESS_CASES` (600), `O1_STRESS_SEED`,
    /// `O1_PEAKS` and `O1_KEPT` (the shipped values) in a release test build.
    /// Random word kind (glyphs, thin bars, blobs), template size, region
    /// size, scale, noise, ink, 1 to 3 distractors, sometimes a blocky decoy;
    /// three templates at the three search scales per case. Prints the
    /// present matches the fast search missed, by kept variance.
    #[test]
    #[ignore]
    fn fast_search_stress() {
        let env = |k: &str| std::env::var(k).ok();
        let n: usize = env("O1_STRESS_CASES")
            .and_then(|v| v.parse().ok())
            .unwrap_or(600);
        let seed: u64 = env("O1_STRESS_SEED")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let peaks: usize = env("O1_PEAKS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(PEAKS);
        let kept: f64 = env("O1_KEPT")
            .and_then(|v| v.parse().ok())
            .unwrap_or(MIN_KEPT);
        let mut rng = Rng(seed.wrapping_mul(0x2545_f491_4f6c_dd1d) | 1);
        let (mut present, mut miss, mut miss_hi, mut stacks) = (0usize, 0usize, 0usize, 0usize);
        let make = |kind: u64, sd: u64, w: usize, h: usize| match kind {
            0 => glyphs(sd, w, h),
            1 => word(1 + (sd % 9) as u32, w, h),
            _ => blobs(sd, w, h),
        };
        for case in 0..n {
            let kind = rng.next(3);
            let (tw, th) = (60 + rng.next(121) as usize, 24 + rng.next(37) as usize);
            let base = seed * 100_000 + case as u64 * 10;
            let tpls = [
                make(kind, base, tw, th),
                make(rng.next(3), base + 1, tw * 9 / 10, th),
                make(rng.next(3), base + 2, tw * 11 / 10, th),
            ];
            let (rw, rh) = (
                tw * 12 / 10 + 8 + rng.next(tw as u64) as usize,
                th * 12 / 10 + 6 + rng.next(th as u64) as usize,
            );
            let mut g = Gray::new(rw, rh);
            let (bg, amp) = (rng.next(60) as f32 / 255.0, 1 + rng.next(60));
            for v in g.px.iter_mut() {
                *v = bg + rng.next(amp) as f32 / 255.0;
            }
            let mut draw = |t: &Gray, rng: &mut Rng, blocky: bool| {
                let sc = SCALES[rng.next(3) as usize];
                let w = ((t.w as f64 * sc).round() as usize).min(rw);
                let h = ((t.h as f64 * sc).round() as usize).min(rh);
                let mut st = t.resize(w, h);
                if blocky {
                    let f = 4;
                    for y in 0..h {
                        for x in 0..w {
                            st.px[y * w + x] =
                                t.at(((x / f) * f).min(t.w - 1), ((y / f) * f).min(t.h - 1));
                        }
                    }
                }
                let ink = 0.45 + rng.next(55) as f32 / 100.0;
                let (x0, y0) = (
                    rng.next((rw - w + 1) as u64) as usize,
                    rng.next((rh - h + 1) as u64) as usize,
                );
                for y in 0..h {
                    for x in 0..w {
                        let a = (st.at(x, y) / 0.9).min(1.0);
                        let p = &mut g.px[(y0 + y) * rw + x0 + x];
                        *p = (*p * (1.0 - a) + ink * a).min(1.0);
                    }
                }
            };
            for _ in 0..1 + rng.next(3) {
                let k = 1 + rng.next(2) as usize;
                draw(&tpls[k], &mut rng, false);
            }
            if rng.next(3) == 0 {
                draw(&tpls[0], &mut rng, true);
            }
            draw(&tpls[0], &mut rng, false);
            let p = pyramid(g);
            for tpl in &tpls {
                for sc in SCALES {
                    let (w, h) = (
                        ((tpl.w as f64 * sc).round_ties_even() as usize).max(1),
                        ((tpl.h as f64 * sc).round_ties_even() as usize).max(1),
                    );
                    let Some(st) = Stack::with_kept(&tpl.resize(w, h), kept) else {
                        continue;
                    };
                    stacks += 1;
                    let fast = search_fast_with(&p, &st, peaks, REFINE_FIRST, REFINE);
                    let full = search_full(&p, &st);
                    assert!(fast.score <= full.score + 1e-9);
                    if full.score >= PRESENT as f64 {
                        present += 1;
                        if full.score - fast.score > 1e-9 {
                            miss += 1;
                            let k = st.levels.last().unwrap().var() / st.levels[0].var();
                            if k >= 0.84 {
                                miss_hi += 1;
                            }
                            eprintln!(
                                "MISS case {case} kind {kind} levels {} kept {k:.2} full {:.3} fast {:.3}",
                                st.levels.len(),
                                full.score,
                                fast.score
                            );
                        }
                    }
                }
            }
        }
        eprintln!(
            "STRESS seed {seed} cases {n} stacks {stacks} peaks {peaks} kept {kept}: present {present} miss {miss} (kept>=0.84: {miss_hi})"
        );
    }

    /// F253-2: `MIN_KEPT` decides the depth. This glyph word keeps between
    /// 0.6 and 0.8 of its variance at 4x: the shipped rule stops it at 2x,
    /// where the stress runs found no misses, and a 0.6 rule would not.
    #[test]
    fn min_kept_stops_a_word_that_blurs_at_4x() {
        let g = glyphs(1, 120, 50);
        let (mut w, mut h, mut px) = (g.w, g.h, g.px.clone());
        for _ in 0..2 {
            (w, h, px) = half(w, h, &px);
        }
        let kept =
            Kernel::new(w, h, &px).unwrap().var() / Kernel::new(g.w, g.h, &g.px).unwrap().var();
        assert!((0.6..0.8).contains(&kept), "{kept}");
        assert_eq!(Stack::new(&g).unwrap().levels.len(), 2);
        assert_eq!(Stack::with_kept(&g, 0.6).unwrap().levels.len(), 3);
    }

    /// A sharp, noisy word and `n` blocky decoys (each the template itself
    /// on the coarse level) in one region. Returns the region and stack.
    fn decoy_region(seed: u64, n: usize) -> (Vec<Level>, Stack) {
        let mut rng = Rng(seed);
        let t = glyphs(seed + 70, 100, 48);
        let st = Stack::new(&t).unwrap();
        let top = st.levels.len() - 1;
        let f = 1usize << top;
        let (mut cw, mut ch, mut cpx) = (t.w, t.h, t.px.clone());
        for _ in 0..top {
            (cw, ch, cpx) = half(cw, ch, &cpx);
        }
        let step = (t.w + 3 * f).next_multiple_of(f);
        let (rw, rh) = ((n + 1) * step + 4 * f, (t.h + 4 * f).next_multiple_of(f));
        let mut g = Gray::new(rw, rh);
        let (sx, sy) = (2 * f, 2 * f);
        for y in 0..t.h {
            for x in 0..t.w {
                g.px[(sy + y) * rw + sx + x] = t.at(x, y) * 0.8 + rng.next(60) as f32 / 255.0;
            }
        }
        for d in 0..n {
            let (dx, dy) = (sx + (d + 1) * step, f + f * (d % 2));
            for y in 0..ch * f {
                for x in 0..cw * f {
                    g.px[(dy + y) * rw + dx + x] = cpx[(y / f) * cw + x / f] * 0.8;
                }
            }
        }
        (pyramid(g), st)
    }

    /// F253-2: with 8 decoys ahead of the sharp word on the coarse level
    /// (each takes one peak), 6 or 8 peaks do not reach it; the shipped 12 do.
    #[test]
    fn fast_search_refines_past_eight_decoys() {
        for seed in [11, 12, 13] {
            let (p, st) = decoy_region(seed, 8);
            let full = search_full(&p, &st).score;
            let fast = search_fast(&p, &st).score;
            assert!(
                (fast - full).abs() < 1e-9,
                "{seed}: fast {fast} full {full}"
            );
            for peaks in [6, 8] {
                let few = search_fast_with(&p, &st, peaks, REFINE_FIRST, REFINE).score;
                assert!(few < full - 1e-6, "{seed}: {peaks} peaks {few} full {full}");
            }
        }
    }

    /// One region and its two 3-level stacks, from a seed (the cases below
    /// were found by a search over seeds).
    fn refine_case(seed: u64) -> (Vec<Level>, Vec<Stack>) {
        let mut rng = Rng(seed * 7919 + 1);
        let (tw, th) = [(100, 48), (120, 50), (140, 64)][(seed % 3) as usize];
        let t = if seed.is_multiple_of(2) {
            glyphs(seed, tw, th)
        } else {
            blobs(seed, tw, th)
        };
        let other = glyphs(seed + 9999, tw * 9 / 10, th);
        let p = pyramid(random_region(&mut rng, &t, &other, tw + 60, th + 40));
        let stacks = [&t, &other]
            .into_iter()
            .filter_map(|g| Stack::with_kept(g, TEST_KEPT))
            .filter(|st| st.levels.len() == 3)
            .collect();
        (p, stacks)
    }

    /// F253-9: the refine windows are load-bearing. Seed 5 needs the ±3
    /// window on the first finer level (±2 misses it), seed 1116 needs ±2
    /// on the last (±1 misses it, with ±3 first), and seed 1449 misses with
    /// ±1 on both. The shipped windows find all three.
    #[test]
    fn the_refine_windows_are_pinned() {
        for (seed, narrow) in [(5, (2, 2)), (1116, (3, 1)), (1449, (1, 1))] {
            let (p, stacks) = refine_case(seed);
            let mut missed = false;
            for st in &stacks {
                let full = search_full(&p, st).score;
                let fast = search_fast(&p, st).score;
                assert!(
                    (fast - full).abs() < 1e-9,
                    "{seed}: fast {fast} full {full}"
                );
                let n = search_fast_with(&p, st, PEAKS, narrow.0, narrow.1).score;
                missed |= full >= PRESENT as f64 && n < full - 1e-6;
            }
            assert!(missed, "{seed}: the narrower windows {narrow:?} miss");
        }
    }

    /// Frames without a word: the fast search finds nothing above the
    /// exhaustive search.
    #[test]
    fn fast_search_finds_nothing_more_on_noise() {
        let mut rng = Rng(77);
        for (k, &(tw, th, rw, rh)) in SIZES.iter().enumerate() {
            let mut g = Gray::new(rw, rh);
            for v in g.px.iter_mut() {
                *v = rng.next(256) as f32 / 255.0;
            }
            let p = pyramid(g);
            let st = Stack::new(&word(k as u32 + 1, tw, th)).unwrap();
            let (fast, full) = (search_fast(&p, &st), search_full(&p, &st));
            assert!(fast.score <= full.score + 1e-9, "{k}: {fast:?} {full:?}");
            assert!(full.score < PRESENT as f64, "{k}: {full:?}");
        }
    }

    /// Whole frames at 1440p and 1080p: the fast read is the exhaustive read.
    #[test]
    fn fast_reads_are_the_exhaustive_reads() {
        let title = Layout::new(END_TITLE, [0.43, 0.43, 0.55, 0.53], Plane::Max);
        let t = ResultTemplates::from_templates(vec![
            (lay(ACCOLADE), Outcome::Victory, word(1, 149, 32)),
            (lay(ACCOLADE), Outcome::Defeat, word(3, 123, 31)),
            (title.clone(), Outcome::Victory, word(9, 100, 48)),
            (title.clone(), Outcome::Defeat, word(4, 90, 48)),
        ])
        .unwrap();
        let accolade = lay(ACCOLADE);
        let frames = [
            (&accolade, 1, 149, 32, [255, 220, 0], 90, 60, false),
            (&accolade, 3, 123, 31, [150, 20, 160], 300, 52, true),
            (&title, 9, 100, 48, [240, 240, 240], 1150, 640, true),
            (&title, 4, 90, 48, [40, 200, 255], 1120, 650, false),
        ];
        for (k, (layout, seed, w, h, colour, x0, y0, shrink)) in frames.into_iter().enumerate() {
            let mut img = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
            paint_at(&mut img, x0, y0, &word(seed, w, h), colour);
            if shrink {
                img = image::imageops::resize(
                    &img,
                    1920,
                    1080,
                    image::imageops::FilterType::Triangle,
                );
            }
            let r = t.read(&img);
            assert_eq!(r, t.read_exhaustive(&img), "frame {k}");
            assert!(r.outcome.is_some() && !r.suspect, "frame {k}: {r:?}");
            assert_eq!(r.source.as_deref(), Some(layout.id.as_str()), "frame {k}");
        }
    }

    /// The point of the fast search: on the words-v1 end-title region with
    /// end-title-sized templates it does about 1% of the exhaustive work
    /// (the exhaustive count is the window count times the template area).
    #[test]
    fn fast_search_does_a_small_fraction_of_the_work() {
        let wide = Layout::new(END_TITLE, [0.30, 0.30, 0.70, 0.62], Plane::Max);
        let t = glyphs(4, 279, 100);
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([30, 30, 40]));
        paint_at(&mut img, 900, 640, &t, [240, 240, 240]);
        let p = region(&img, wide.roi, wide.plane).unwrap();
        let st = Stack::new(&t).unwrap();
        assert_eq!(st.levels.len(), 3);
        let (k, r) = (&st.levels[0], &p[0]);
        let full = (r.w - k.w + 1) * (r.h - k.h + 1) * k.w * k.h;
        let fast = search_fast(&p, &st);
        assert!(fast.macs * 50 < full, "fast {} full {full}", fast.macs);
        assert!(fast.score > 0.95, "{fast:?}");
    }

    /// Regions for the search tests below: six small ones (the template
    /// builds one refine level there) and two of the words-v1 end-title
    /// shape (two refine levels). Each has `t` and `other` drawn into
    /// noise, except the last, which has `t` alone in its top-left corner:
    /// there the refine windows of the two levels have the same
    /// coordinates, so a score kept under a position alone would be handed
    /// to the wrong level.
    fn search_cases() -> Vec<(Vec<Level>, [Gray; 3])> {
        let mut rng = Rng(0x2633);
        let mut out = Vec::new();
        let deep = (279, 100, 512, 230);
        for (n, (tw, th, rw, rh)) in [(6, SIZES[2]), (1, deep)] {
            let words = [glyphs(4, tw, th), glyphs(7, tw, th), glyphs(11, tw, th)];
            for _ in 0..n {
                let g = random_region(&mut rng, &words[0], &words[1], rw, rh);
                out.push((pyramid(g), words.clone()));
            }
        }
        let (tw, th, rw, rh) = deep;
        let words = [glyphs(4, tw, th), glyphs(7, tw, th), glyphs(11, tw, th)];
        let mut g = Gray::new(rw, rh);
        for v in g.px.iter_mut() {
            *v = 0.1 + rng.next(30) as f32 / 255.0;
        }
        for y in 0..th {
            for x in 0..tw {
                let a = words[0].at(x, y) / 0.9;
                let p = &mut g.px[(5 + y) * rw + 6 + x];
                *p = (*p * (1.0 - a) + 0.9 * a).min(1.0);
            }
        }
        out.push((pyramid(g), words));
        out
    }

    /// A window is scored once per search (F263-3). The score is the one a
    /// search gets that scores a window again every time a peak's refine
    /// reaches it, bit for bit: for the word in the region, for the other
    /// word in it and for a word that is not there. The work is never more,
    /// and it is less where peaks sit close together.
    #[test]
    fn a_window_is_scored_once_per_search() {
        let (mut all, mut saved, mut deep) = (0, 0, 0);
        for (i, (p, words)) in search_cases().iter().enumerate() {
            for (k, word) in words.iter().enumerate() {
                let st = Stack::new(word).unwrap();
                deep += usize::from(st.levels.len() == 3);
                let again = search_fast_once(p, &st, PEAKS, REFINE_FIRST, REFINE, false, &[]);
                let once = search_fast_once(p, &st, PEAKS, REFINE_FIRST, REFINE, true, &[]);
                assert_eq!(
                    once.score.to_bits(),
                    again.score.to_bits(),
                    "region {i}, word {k}"
                );
                assert_eq!(once, search_fast(p, &st), "region {i}, word {k}");
                assert!(once.macs <= again.macs, "region {i}, word {k}");
                all += again.macs;
                saved += again.macs - once.macs;
            }
        }
        assert!(deep > 0, "a search with two refine levels is covered");
        assert!(saved > 0, "close peaks share windows: {saved} of {all}");
    }

    /// The skipping search (F263-3) leaves out only windows that cannot
    /// reach `PRESENT`. A best score at or above `PRESENT` is the fast
    /// search's score bit for bit, also for a word that is only partly
    /// there; a score below it stays below; and the work is less.
    #[test]
    fn the_skipping_search_keeps_every_score_at_present() {
        let (mut kept, mut partial, mut below) = (0, 0, 0);
        let (mut fast_macs, mut skip_macs) = (0, 0);
        for (i, (p, words)) in search_cases().iter().enumerate() {
            // words that are `t` on the left and another word on the right
            let (t, other) = (&words[0], &words[1]);
            let mixed = [25, 40, 55, 70].map(|pc| {
                let mut g = t.clone();
                for y in 0..g.h {
                    for x in g.w * pc / 100..g.w {
                        g.px[y * g.w + x] = other.px[y * g.w + x];
                    }
                }
                g
            });
            let phased: Vec<Phased> = SKIP_BLOCKS.iter().map(|&f| Phased::new(&p[0], f)).collect();
            for (k, word) in words.iter().chain(&mixed).enumerate() {
                let st = Stack::new(word).unwrap();
                let skip: Vec<(&Proj, &Phased)> = st
                    .projs
                    .iter()
                    .filter_map(|pr| Some((pr, phased.iter().find(|ph| ph.f == pr.f)?)))
                    .collect();
                assert_eq!(skip.len(), SKIP_BLOCKS.len(), "both bounds are checked");
                let fast = search_fast(p, &st);
                let skipping = search_fast_skipping(p, &st, &skip);
                if fast.score >= PRESENT as f64 {
                    assert_eq!(
                        skipping.score.to_bits(),
                        fast.score.to_bits(),
                        "region {i}, word {k}"
                    );
                    kept += 1;
                    partial += usize::from(fast.score < 0.9);
                } else {
                    assert!(skipping.score <= fast.score, "region {i}, word {k}");
                    below += 1;
                }
                assert!(skipping.macs <= fast.macs, "region {i}, word {k}");
                fast_macs += fast.macs;
                skip_macs += skipping.macs;
            }
        }
        assert!(
            kept > 0 && partial > 0 && below > 0,
            "kept {kept}, partial {partial}, below {below}"
        );
        assert!(skip_macs < fast_macs, "{skip_macs} of {fast_macs}");
    }

    /// `read_skipping` is `read`, every field, wherever a template reaches
    /// `PRESENT`: a sure word, a word that is only partly there, two words
    /// at once. A frame with no word reads nothing either way, and the
    /// skipping read scores fewer windows to find that out.
    #[test]
    fn the_skipping_read_is_the_read_wherever_a_word_is_present() {
        let titles = with_end_titles().end_titles().unwrap();
        // no word: a flat frame, and one with texture all over
        let textured = RgbImage::from_fn(2560, 1440, |x, y| {
            let n = ((x.wrapping_mul(37) ^ y.wrapping_mul(17)) % 97) as u8;
            Rgb([20 + n, 26 + n / 2, 50 + n / 3])
        });
        let mut frames = vec![dark_frame(), textured];
        for (layout, seed, w, colour) in [
            (lay(END_TITLE), 2, 90, [150, 20, 160]),
            (lay(END_TITLE), 4, 80, [40, 200, 255]),
            (lay(END_TITLE), 6, 60, [255, 220, 0]),
            (end_title_plain(), 5, 90, [240, 240, 240]),
        ] {
            let mut img = dark_frame();
            paint(&mut img, &layout, &word(seed, w, 34), colour);
            frames.push(img.clone());
            // the right part of the word painted over
            let [fx0, fy0, _, _] = layout.roi;
            let (x0, y0) = ((fx0 * 2560.0) as u32 + 20, (fy0 * 1440.0) as u32 + 10);
            for cut in [w as u32, w as u32 * 3 / 2] {
                let mut part = img.clone();
                for y in y0..y0 + 68 {
                    for x in x0 + cut..x0 + 2 * w as u32 {
                        part.put_pixel(x, y, Rgb([20, 26, 50]));
                    }
                }
                frames.push(part);
            }
        }
        // an end-title VICTORY and DEFEAT on one frame
        let mut both = dark_frame();
        paint(&mut both, &lay(END_TITLE), &word(2, 90, 34), [150, 20, 160]);
        paint_at(&mut both, 1300, 600, &word(4, 80, 34), [150, 20, 160]);
        frames.push(both);

        let (mut sure, mut flagged, mut nothing, mut saved) = (0, 0, 0, 0);
        for (k, img) in frames.iter().enumerate() {
            let (read, skipping) = (titles.read(img), titles.read_skipping(img));
            let work = |f: Vec<Found>| f.iter().map(|f| f.macs).sum::<usize>();
            let fast = work(titles.scores_in(&titles.regions(img), false));
            let quick = work(titles.scores_with(&titles.skip_regions(img), Search::Skipping));
            if read.score >= PRESENT {
                assert_eq!(skipping, read, "frame {k}");
                assert!(quick <= fast, "frame {k}: {quick} of {fast}");
                sure += usize::from(read.sure().is_some());
                flagged += usize::from(read.outcome.is_some() && read.suspect);
            } else {
                assert_eq!(read.outcome, None, "frame {k}");
                assert_eq!(
                    skipping,
                    ResultRead {
                        outcome: None,
                        source: None,
                        score: skipping.score,
                        margin: 0.0,
                        suspect: true,
                    },
                    "frame {k}"
                );
                assert!(skipping.score <= read.score, "frame {k}");
                assert!(quick <= fast, "frame {k}: {quick} of {fast}");
                saved += fast - quick;
                nothing += 1;
            }
        }
        assert!(
            sure >= 4 && nothing >= 2,
            "sure {sure}, flagged {flagged}, nothing {nothing}"
        );
        assert!(saved > 0, "no word: fewer windows are scored");
    }

    // ------------------------------------------------ folders and reload

    fn pack(dir: &Path, victory_seed: u32) {
        write_pack(
            dir,
            &[
                ("accolade_victory.png", &word(victory_seed, 70, 30)),
                ("accolade_defeat.png", &word(3, 60, 30)),
            ],
        );
    }

    /// A frame with `word(seed)` on the accolade screen.
    fn accolade_frame(seed: u32) -> RgbImage {
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
        paint(&mut img, &lay(ACCOLADE), &word(seed, 70, 30), [255, 220, 0]);
        img
    }

    /// Which pack the live templates came from, and whether they read
    /// `word(seed)` as VICTORY.
    fn reads_victory(live: &LiveResults, seed: u32) -> (Option<ResultPack>, bool) {
        let cur = live.current();
        let ok = cur.as_ref().is_some_and(|l| {
            let r = l.templates.read(&accolade_frame(seed));
            r.outcome == Some(Outcome::Victory) && !r.suspect
        });
        (cur.map(|l| l.pack), ok)
    }

    #[test]
    fn words_v1_is_preferred_over_the_old_folder() {
        let data = tempfile::tempdir().unwrap();
        pack(&data.path().join(WORDS_SUBDIR), 1);
        pack(&data.path().join(TEMPLATE_SUBDIR), 5);
        let live = LiveResults::load(ResultDirs::in_data_dir(data.path()));
        assert_eq!(reads_victory(&live, 1), (Some(ResultPack::WordsV1), true));
        assert!(!reads_victory(&live, 5).1, "old folder not used");
    }

    #[test]
    fn the_old_folder_is_the_fallback() {
        let data = tempfile::tempdir().unwrap();
        pack(&data.path().join(TEMPLATE_SUBDIR), 5);
        let live = LiveResults::load(ResultDirs::in_data_dir(data.path()));
        assert_eq!(reads_victory(&live, 5), (Some(ResultPack::Legacy), true));
        // a words-v1 folder without a DEFEAT template does not load either
        let data = tempfile::tempdir().unwrap();
        write_pack(
            &data.path().join(WORDS_SUBDIR),
            &[("accolade_victory.png", &word(1, 70, 30))],
        );
        pack(&data.path().join(TEMPLATE_SUBDIR), 5);
        let live = LiveResults::load(ResultDirs::in_data_dir(data.path()));
        assert_eq!(reads_victory(&live, 5), (Some(ResultPack::Legacy), true));
        // neither folder: no templates
        let empty = tempfile::tempdir().unwrap();
        let live = LiveResults::load(ResultDirs::in_data_dir(empty.path()));
        assert!(live.current().is_none());
    }

    /// Install words-v1 the way pack sync does: staging folder, then swap.
    fn swap_in_words(data: &Path, victory_seed: u32) {
        let staging = data.join(format!("templates/.words-v1-staging-{victory_seed}"));
        pack(&staging, victory_seed);
        crate::packs::layout::swap_dir(&staging, &data.join(WORDS_SUBDIR)).unwrap();
    }

    #[test]
    fn words_v1_is_picked_up_after_it_is_swapped_in() {
        let data = tempfile::tempdir().unwrap();
        pack(&data.path().join(TEMPLATE_SUBDIR), 5);
        let live = LiveResults::load(ResultDirs::in_data_dir(data.path()));
        assert_eq!(reads_victory(&live, 5), (Some(ResultPack::Legacy), true));
        live.wait_reload();
        assert_eq!(live.reload_spawns(), 0, "no change, no reload");
        swap_in_words(data.path(), 1);
        // this read keeps the templates it has; the load runs aside
        assert_eq!(reads_victory(&live, 5), (Some(ResultPack::Legacy), true));
        live.wait_reload();
        assert_eq!(reads_victory(&live, 1), (Some(ResultPack::WordsV1), true));
        // a newer words-v1 over the installed one
        swap_in_words(data.path(), 7);
        live.current();
        live.wait_reload();
        assert_eq!(reads_victory(&live, 7), (Some(ResultPack::WordsV1), true));
        assert_eq!(live.reload_spawns(), 2);
    }

    /// F253-8: a change to the old `templates/result/` folder alone is
    /// picked up too (the stamp covers both folders).
    #[test]
    fn a_new_legacy_folder_is_picked_up() {
        let data = tempfile::tempdir().unwrap();
        let legacy = data.path().join(TEMPLATE_SUBDIR);
        pack(&legacy, 5);
        let live = LiveResults::load(ResultDirs::in_data_dir(data.path()));
        assert_eq!(reads_victory(&live, 5), (Some(ResultPack::Legacy), true));
        let staging = data.path().join("templates/.result-staging");
        pack(&staging, 7);
        crate::packs::layout::swap_dir(&staging, &legacy).unwrap();
        live.current();
        live.wait_reload();
        assert_eq!(reads_victory(&live, 7), (Some(ResultPack::Legacy), true));
        assert_eq!(live.reload_spawns(), 1);
    }

    /// A slow load never holds up a read, and only one load runs at a time.
    #[test]
    fn a_reload_never_blocks_a_read() {
        let data = tempfile::tempdir().unwrap();
        pack(&data.path().join(TEMPLATE_SUBDIR), 5);
        let live = LiveResults::load(ResultDirs::in_data_dir(data.path()));
        live.set_load_delay_ms(400);
        swap_in_words(data.path(), 1);
        let t0 = std::time::Instant::now();
        for _ in 0..50 {
            assert_eq!(
                live.current().map(|l| l.pack),
                Some(ResultPack::Legacy),
                "load still in flight"
            );
        }
        assert!(t0.elapsed() < std::time::Duration::from_millis(300));
        assert_eq!(live.reload_spawns(), 1);
        live.wait_reload();
        assert_eq!(reads_victory(&live, 1), (Some(ResultPack::WordsV1), true));
    }

    /// A load that saw a swap is thrown away (never a mix of two packs), and
    /// a later read loads again.
    #[test]
    fn a_load_that_saw_a_swap_is_discarded() {
        let data = tempfile::tempdir().unwrap();
        pack(&data.path().join(TEMPLATE_SUBDIR), 5);
        let live = LiveResults::load(ResultDirs::in_data_dir(data.path()));
        swap_in_words(data.path(), 1);
        let path = data.path().to_path_buf();
        live.during_next_load(move || swap_in_words(&path, 7));
        assert_eq!(reads_victory(&live, 5).0, Some(ResultPack::Legacy));
        live.wait_reload();
        // nothing was installed; this read keeps the old folder and retries
        assert_eq!(reads_victory(&live, 5), (Some(ResultPack::Legacy), true));
        live.wait_reload();
        assert_eq!(reads_victory(&live, 7), (Some(ResultPack::WordsV1), true));
        assert_eq!(live.reload_spawns(), 2);
    }

    // ------------------------------------- bound precision (F253-11, F253-12)

    /// Exact NCC of `k` at (x, y) of `p`, all in f64.
    fn ncc64(p: &Level, k: &Kernel, x: usize, y: usize) -> f64 {
        let n = (k.w * k.h) as f64;
        let win = |tx: usize, ty: usize| p.px[(y + ty) * p.w + x + tx] as f64;
        let mut mean = 0.0;
        for ty in 0..k.h {
            for tx in 0..k.w {
                mean += win(tx, ty);
            }
        }
        mean /= n;
        let (mut num, mut pp, mut tt) = (0.0, 0.0, 0.0);
        for ty in 0..k.h {
            for tx in 0..k.w {
                let (a, b) = (win(tx, ty) - mean, k.z[ty * k.w + tx] as f64);
                num += a * b;
                pp += a * a;
                tt += b * b;
            }
        }
        num / (pp.sqrt() * tt.sqrt())
    }

    /// The window `m + s (P t + lambda (I - P) t)`, with `t` the zero-mean
    /// template and `P` the stack's coarsest block projection. Its residual
    /// is `lambda s (I - P) t` for that projection and every finer one, so
    /// Cauchy-Schwarz is an equality and the bound equals the score.
    fn tight_window(st: &Stack, m: f64, s: f64, lambda: f64) -> Level {
        let (k, pr) = (&st.levels[0], &st.projs[0]);
        let mut px = vec![0.0f32; k.w * k.h];
        for y in 0..k.h {
            for x in 0..k.w {
                let (bx, by) = (x / pr.f, y / pr.f);
                let pt = if bx < pr.cw && by < pr.ch {
                    pr.ct[by * pr.cw + bx]
                } else {
                    0.0
                };
                let z = k.z[y * k.w + x] as f64;
                px[y * k.w + x] = (m + s * (pt + lambda * (z - pt))) as f32;
            }
        }
        Level::new(k.w, k.h, px)
    }

    /// Glyph templates whose sides are multiples of 16, so every projection
    /// (f = 16, 8, 4, 2) tiles them with no leftover strip.
    fn tiled_stacks() -> Vec<Stack> {
        [(96, 48, 801), (128, 48, 802), (160, 64, 803)]
            .iter()
            .map(|&(w, h, seed)| Stack::new(&glyphs(seed, w, h)).unwrap())
            .collect()
    }

    /// On a tight window the bound is the score, to float precision, at
    /// every projection: a bound that is a little too low (a residual norm
    /// 1% short, a negative slack) is caught here.
    #[test]
    fn the_bound_is_exact_on_a_tight_window() {
        let mut checked = 0;
        for st in tiled_stacks() {
            assert_eq!(st.projs.len(), 4, "f = 16, 8, 4, 2");
            assert_eq!(st.projs[0].f, 16);
            let k = &st.levels[0];
            for (m, s, lambda) in [
                (0.2, 0.3, 1.0),
                (0.5, 0.2, 0.4),
                (0.3, 0.25, 2.5),
                (0.8, 0.05, 0.7),
            ] {
                let p = tight_window(&st, m, s, lambda);
                let v = ncc64(&p, k, 0, 0);
                for pr in &st.projs {
                    let ub = bound_at(&p, &Phased::new(&p, pr.f), k, pr, 0, 0).unwrap();
                    assert!(
                        (ub - v).abs() < 1e-6,
                        "f {} m {m} s {s} lambda {lambda}: bound {ub} score {v}",
                        pr.f
                    );
                    assert!(ub + BOUND_SLACK >= v, "f {}: {ub} < {v}", pr.f);
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 48);
    }

    /// Bright, nearly flat windows (amplitude down to 1e-4 of full scale,
    /// not quantized), and faint 8-bit copies: the bound stays at or above
    /// the exact score within [`BOUND_SLACK`]. An uncentred f32 projection
    /// lost up to 1.2e-3 here (F253-11).
    #[test]
    fn the_bound_holds_on_near_flat_windows() {
        let mut worst = f64::INFINITY;
        for st in tiled_stacks() {
            let k = &st.levels[0];
            let mut windows = Vec::new();
            for m in [0.6, 0.9, 0.97] {
                // sub-grey-level ghosts (1e-4 is 1/40 of a grey level); below
                // about 1e-5 the window variance itself is lost to rounding in
                // the integral images, for ncc_at as much as for the bound
                for s in [1e-3, 3e-4, 1e-4] {
                    for lambda in [0.5, 1.0, 2.0] {
                        windows.push(tight_window(&st, m, s, lambda));
                    }
                }
                // faint 8-bit copies: a few grey levels of contrast
                for step in [2.0, 3.0, 6.0] {
                    let mut p = tight_window(&st, m, step / 255.0, 1.0);
                    for v in p.px.iter_mut() {
                        *v = (*v * 255.0).round() / 255.0;
                    }
                    windows.push(Level::new(p.w, p.h, p.px));
                }
            }
            for p in &windows {
                let v = ncc64(p, k, 0, 0);
                for pr in &st.projs {
                    let Some(ub) = bound_at(p, &Phased::new(p, pr.f), k, pr, 0, 0) else {
                        continue;
                    };
                    worst = worst.min(ub - v);
                }
            }
        }
        assert!(
            worst + BOUND_SLACK >= 0.0,
            "bound below the score by {}",
            -worst
        );
    }

    // ------------------------------------------------ flat windows (R3-1)

    /// The exact variance sum `sum((p - mean)^2)` of a window, in f64.
    fn var64(p: &Level, k: &Kernel, x: usize, y: usize) -> f64 {
        let n = (k.w * k.h) as f64;
        let win = |tx: usize, ty: usize| p.px[(y + ty) * p.w + x + tx] as f64;
        let mut mean = 0.0;
        for ty in 0..k.h {
            for tx in 0..k.w {
                mean += win(tx, ty);
            }
        }
        mean /= n;
        let mut pp = 0.0;
        for ty in 0..k.h {
            for tx in 0..k.w {
                pp += (win(tx, ty) - mean) * (win(tx, ty) - mean);
            }
        }
        pp
    }

    /// The variance sum at or under which `p` calls a `k`-sized window flat.
    fn flat_line(p: &Level, k: &Kernel) -> f64 {
        FLAT_VAR.max((k.w * k.h) as f64 * p.flat)
    }

    /// The same level under the rule before the fix: flat at `FLAT_VAR` only.
    fn old_rule(mut p: Level) -> Level {
        p.flat = 0.0;
        p
    }

    /// R3-1: a band of one grey value has no window to score, at any pyramid
    /// level, for the end-title shape and a phrase shape, at every third
    /// grey level. Before the fix most of these windows were scored (their
    /// variance from the integral images is rounding noise above 1e-9), and
    /// the score was noise too. The old rule is run beside the new one and
    /// must show both, so this test cannot pass on bands that were never a
    /// problem.
    #[test]
    fn a_band_of_one_grey_value_has_no_window_to_score() {
        let (mut leaked, mut worst) = (0usize, 0.0f64);
        for (tw, th, rw, rh, seed) in [(279, 100, 512, 230, 911), (120, 27, 358, 90, 912)] {
            let st = Stack::with_kept(&glyphs(seed, tw, th), TEST_KEPT).unwrap();
            assert!(!st.projs.is_empty());
            for g in (1..255u32).step_by(3) {
                let band = || {
                    let mut b = Gray::new(rw, rh);
                    b.px.fill(g as f32 / 255.0);
                    b
                };
                let p = pyramid(band());
                let old: Vec<Level> = pyramid(band()).into_iter().map(old_rule).collect();
                for (l, k) in st.levels.iter().enumerate() {
                    let ph = (l == 0).then(|| Phased::new(&p[0], st.projs[0].f));
                    // the old rule's scored window with the least variance
                    let mut least: Option<(f64, usize, usize)> = None;
                    for y in 0..=p[l].h - k.h {
                        for x in 0..=p[l].w - k.w {
                            assert_eq!(p[l].window(x, y, k.w, k.h), None, "grey {g} level {l}");
                            if let Some(ph) = &ph {
                                assert_eq!(bound_at(&p[0], ph, k, &st.projs[0], x, y), None);
                            }
                            if let Some((_, var)) = old[l].window(x, y, k.w, k.h) {
                                leaked += 1;
                                if least.is_none_or(|b| var < b.0) {
                                    least = Some((var, x, y));
                                }
                            }
                        }
                    }
                    if let Some((_, x, y)) = least {
                        worst = worst.max(ncc_at(&old[l], k, x, y).unwrap().abs());
                    }
                    assert_eq!(search_full(&p[l..], &st_at(&st, l)).score, -1.0);
                }
                assert_eq!(search_fast(&p, &st).score, -1.0, "grey {g}");
            }
        }
        // measured: 943,008 windows, the worst at 0.633
        assert!(
            leaked > 500_000,
            "the old rule scored {leaked} flat windows"
        );
        assert!(worst > 0.5, "and the worst of them scored {worst}");
    }

    /// `st` from level `l` down, as a stack of its own (to search one level
    /// in full).
    fn st_at(st: &Stack, l: usize) -> Stack {
        Stack {
            levels: st.levels[l..].to_vec(),
            projs: Vec::new(),
        }
    }

    /// The property behind the fix, on thousands of windows: `ncc_at` is
    /// the exact score (all in f64, centred) wherever it gives one, and it
    /// gives none exactly on the windows that are flat. Random planes and
    /// the edge cases: one grey value, black, white, one odd pixel, one
    /// grey level of contrast, the last-bit jitter a resize leaves on a
    /// flat area, a flat half beside noise, values that are not 8-bit,
    /// 1-pixel-wide and 1-pixel-high templates, widths around the dot
    /// product's chunk of 8.
    #[test]
    fn ncc_at_is_the_exact_score_or_none() {
        let mut rng = Rng(0x51ab_f1a7);
        let shapes = [
            (1, 24),
            (24, 1),
            (7, 5),
            (8, 8),
            (9, 9),
            (15, 4),
            (16, 6),
            (17, 3),
            (31, 12),
            (64, 20),
            (100, 33),
        ];
        // (windows scored, windows flat, largest error where the window has
        // real contrast, largest error anywhere)
        let (mut scored, mut flat, mut err_real, mut err_any) = (0usize, 0usize, 0.0f64, 0.0f64);
        let mut check = |p: &Level, k: &Kernel, x: usize, y: usize| {
            let n = (k.w * k.h) as f64;
            let (pp, line) = (var64(p, k, x, y), flat_line(p, k));
            match ncc_at(p, k, x, y) {
                None => {
                    // flat: the exact variance is within the rounding bound
                    // of the line
                    assert!(
                        pp <= 2.0 * line,
                        "({x},{y}) {}x{}: {pp} is not flat",
                        k.w,
                        k.h
                    );
                    flat += 1;
                }
                Some(v) => {
                    assert!(
                        pp > 0.0,
                        "({x},{y}) {}x{}: a window of one value scored {v}",
                        k.w,
                        k.h
                    );
                    let e = (v - ncc64(p, k, x, y)).abs();
                    err_any = err_any.max(e);
                    // one 8-bit grey level over 2% of the window, or more
                    if pp >= 3e-7 * n {
                        err_real = err_real.max(e);
                    }
                    scored += 1;
                }
            }
            // a window with twice the line's variance is always scored
            if pp > 2.0 * line {
                assert!(
                    ncc_at(p, k, x, y).is_some(),
                    "({x},{y}): {pp} was called flat"
                );
            }
        };
        for (si, &(tw, th)) in shapes.iter().enumerate() {
            // a template of 8-bit values (the packs') or of any f32
            let mut t = Gray::new(tw, th);
            for v in t.px.iter_mut() {
                *v = if si % 2 == 0 {
                    rng.next(256) as f32 / 255.0
                } else {
                    rng.next(1 << 24) as f32 / (1 << 24) as f32
                };
            }
            let k = Kernel::new(tw, th, &t.px).unwrap();
            let (rw, rh) = (tw + 13, th + 11);
            for kind in 0..12 {
                for rep in 0..4 {
                    let c = [0.0, 1.0, 242.0 / 255.0, 128.0 / 255.0, 37.0 / 255.0]
                        [rng.next(5) as usize];
                    let c = if rep == 3 {
                        rng.next(256) as f32 / 255.0
                    } else {
                        c
                    };
                    let mut g = Gray::new(rw, rh);
                    g.px.fill(c);
                    let step = if c > 0.5 { -1.0 } else { 1.0 } / 255.0;
                    match kind {
                        // one grey value
                        0 => {}
                        // noise of 8-bit values
                        1 => {
                            g.px.iter_mut()
                                .for_each(|v| *v = rng.next(256) as f32 / 255.0)
                        }
                        // any f32 in 0..1
                        2 => {
                            g.px.iter_mut()
                                .for_each(|v| *v = rng.next(1 << 24) as f32 / (1 << 24) as f32)
                        }
                        // one odd pixel, one grey level off
                        3 => {
                            let i = rng.next((rw * rh) as u64) as usize;
                            g.px[i] += step;
                        }
                        // the template at one grey level of contrast
                        4 => {
                            for y in 0..th {
                                for x in 0..tw {
                                    if t.at(x, y) > 0.5 {
                                        g.px[(y + 5) * rw + x + 6] += step;
                                    }
                                }
                            }
                        }
                        // the last-bit jitter of a resized flat area
                        5 => g.px.iter_mut().for_each(|v| {
                            let bits = v.to_bits();
                            *v = f32::from_bits(match rng.next(3) {
                                0 => bits.saturating_sub(1),
                                1 if *v < 1.0 => bits + 1,
                                _ => bits,
                            });
                        }),
                        // a flat half beside noise
                        6 => {
                            for y in 0..rh {
                                for x in rw / 2..rw {
                                    g.px[y * rw + x] = rng.next(256) as f32 / 255.0;
                                }
                            }
                        }
                        // white with a few black pixels, black with a few white
                        7 => {
                            g.px.fill(if rep % 2 == 0 { 1.0 } else { 0.0 });
                            for _ in 0..3 {
                                let i = rng.next((rw * rh) as u64) as usize;
                                g.px[i] = if rep % 2 == 0 { 0.0 } else { 1.0 };
                            }
                        }
                        // low-contrast noise: two neighbouring grey levels
                        8 => {
                            g.px.iter_mut()
                                .for_each(|v| *v = c + step * rng.next(2) as f32)
                        }
                        // a ramp, one grey level end to end
                        9 => {
                            for y in 0..rh {
                                for x in 0..rw {
                                    g.px[y * rw + x] = c + step * x as f32 / rw as f32;
                                }
                            }
                        }
                        // the template itself, full contrast and faint
                        10 => {
                            let a = [1.0, 0.01, 1e-3, 1e-4][rep];
                            for y in 0..th {
                                for x in 0..tw {
                                    g.px[(y + 3) * rw + x + 4] = 0.5 + a * (t.at(x, y) - 0.5);
                                }
                            }
                        }
                        // sub-grey-level noise
                        _ => {
                            g.px.iter_mut()
                                .for_each(|v| *v = 0.9 + 1e-3 * rng.next(256) as f32 / 255.0)
                        }
                    }
                    for v in g.px.iter_mut() {
                        *v = v.clamp(0.0, 1.0);
                    }
                    let p = Level::new(rw, rh, g.px);
                    for y in 0..=rh - th {
                        for x in 0..=rw - tw {
                            check(&p, &k, x, y);
                        }
                    }
                }
            }
        }
        // the end-title shape in its band: a word one grey level off a
        // bright flat band, and the flat stretch beside it
        let t = glyphs(921, 279, 100);
        let k = Kernel::new(t.w, t.h, &t.px).unwrap();
        let mut g = Gray::new(512, 230);
        g.px.fill(242.0 / 255.0);
        for y in 0..t.h {
            for x in 0..t.w {
                if t.at(x, y) > 0.5 {
                    g.px[(y + 125) * 512 + x + 220] = 243.0 / 255.0;
                }
            }
        }
        let p = Level::new(512, 230, g.px);
        for (x, y) in [(220, 125), (219, 125), (220, 124), (233, 130), (150, 80)] {
            check(&p, &k, x, y);
        }
        for (x, y) in [(0, 0), (100, 20), (233, 25)] {
            assert_eq!(ncc_at(&p, &k, x, y), None, "({x},{y}) is beside the word");
            check(&p, &k, x, y);
        }
        let on = ncc_at(&p, &k, 220, 125).expect("a word one grey level off its band is not flat");
        assert!(on > 0.99, "{on}");
        // measured: 58,008 scored and 30,704 flat; errors 6.8e-5 and 2.4e-4
        assert!(
            scored > 50_000 && flat > 25_000,
            "scored {scored}, flat {flat}"
        );
        assert!(
            err_real < 5e-4,
            "error on a window with real contrast: {err_real}"
        );
        assert!(err_any < 2e-3, "error on any scored window: {err_any}");
    }

    /// The certification threshold is `s1 - MARGIN`, exactly: a runner-up
    /// whose exhaustive best is 0.002 above it flags the read, one 0.002
    /// below it does not.
    #[test]
    fn the_certification_threshold_is_pinned() {
        let title = Layout::new(END_TITLE, [0.40, 0.40, 0.62, 0.56], Plane::Max);
        let (a, b) = (glyphs(611, 100, 48), glyphs(612, 90, 48));
        let t = ResultTemplates::from_templates(vec![
            (title.clone(), Outcome::Victory, a),
            (title, Outcome::Defeat, b.clone()),
        ])
        .unwrap();
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
        paint_at(&mut img, 1290, 650, &b, [240, 240, 240]);
        let mut regions = t.regions(&img);
        // DEFEAT's exhaustive best: the runner-up the certification must find
        let r = t.scores_in(&regions, true)[1].score;
        assert!(r > 0.5, "{r}");
        for (gap, flagged) in [(-0.002, true), (0.002, false)] {
            // VICTORY's fast score puts the threshold `gap` above DEFEAT's
            // best; DEFEAT itself under-read
            let s1 = r + MARGIN as f64 + gap;
            let mut scores = t.scores_in(&regions, false);
            scores[0].score = s1;
            scores[1].score = 0.2;
            assert!(!t.decide(&scores).suspect, "the fast scores alone are sure");
            let read = t.settle(&mut regions, scores, CERT_MACS);
            assert_eq!(read.suspect, flagged, "gap {gap}: {read:?}");
            assert_eq!(read.outcome, Some(Outcome::Victory));
        }
    }

    /// F260-7: a template PNG larger than the work-scale game rect is not
    /// decoded.
    #[test]
    fn an_oversized_template_png_is_not_loaded() {
        let d = tempfile::tempdir().unwrap();
        let save = |name: &str, w: u32, h: u32| {
            let p = d.path().join(name);
            image::GrayImage::from_pixel(w, h, image::Luma([200]))
                .save(&p)
                .unwrap();
            p
        };
        assert!(load_gray(&save("ok.png", 1280, 720)).is_some());
        assert!(load_gray(&save("wide.png", 1281, 4)).is_none());
        assert!(load_gray(&save("tall.png", 4, 721)).is_none());
        assert!(load_gray(&save("huge.png", 5000, 5000)).is_none());
        assert_eq!(MAX_TEMPLATE_PX, (1280, 720));
    }

    /// A layouts.json that cannot be read refuses the folder like one that
    /// does not parse (F253-14); a missing one does not.
    #[test]
    fn an_unreadable_layouts_json_refuses_the_folder() {
        let bad = tempfile::tempdir().unwrap();
        pack(bad.path(), 1);
        std::fs::write(bad.path().join("layouts.json"), [0xff, 0xfe, b'[', b']']).unwrap();
        assert!(ResultTemplates::load_dir(bad.path()).is_none(), "not UTF-8");
        let dir = tempfile::tempdir().unwrap();
        pack(dir.path(), 1);
        std::fs::create_dir(dir.path().join("layouts.json")).unwrap();
        assert!(
            ResultTemplates::load_dir(dir.path()).is_none(),
            "a directory"
        );
        let none = tempfile::tempdir().unwrap();
        pack(none.path(), 1);
        assert!(ResultTemplates::load_dir(none.path()).is_some(), "no file");
    }

    // ------------------------------------------------- FINAL SCORE (O3)

    /// A synthetic FINAL SCORE line at the work scale (the words-v1 one is
    /// 123 x 28).
    fn final_line() -> Gray {
        glyphs(701, 123, 28)
    }

    fn final_template() -> FinalScoreTemplate {
        FinalScoreTemplate::new(FinalScoreTemplate::builtin_layout(), &final_line()).unwrap()
    }

    /// A 1440p frame with `t` painted white inside the FINAL SCORE region.
    fn final_frame(t: &Gray) -> RgbImage {
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
        paint_at(&mut img, 1102, 1060, t, [235, 235, 235]);
        img
    }

    #[test]
    fn the_final_score_line_reads_sure() {
        let fs = final_template();
        let r = fs.read(&final_frame(&final_line()));
        assert!(matches!(r, FinalScoreRead::Sure(s) if s > 0.9), "{r:?}");
        assert!(r.is_signal());
        assert!(fs.read_exhaustive(&final_frame(&final_line())).is_signal());
        let blank = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
        assert!(matches!(fs.read(&blank), FinalScoreRead::Absent(_)));
        // other text in the region is not the line
        let other = fs.read(&final_frame(&glyphs(702, 123, 28)));
        assert!(!other.is_signal(), "{other:?}");
    }

    /// Half the line's glyphs: present, below the floor. Not a signal.
    #[test]
    fn a_partial_final_score_line_is_not_a_signal() {
        let line = final_line();
        let other = glyphs(703, 123, 28);
        let mut half = line.clone();
        for y in 0..half.h {
            for x in half.w / 2..half.w {
                half.px[y * half.w + x] = other.px[y * other.w + x];
            }
        }
        let fs = final_template();
        let r = fs.read(&final_frame(&half));
        assert!(matches!(r, FinalScoreRead::Suspect(_)), "{r:?}");
        assert!(!r.is_signal());
    }

    /// A fast score in the suspect band is certified against the floor: the
    /// line on the frame makes it sure, a frame without it stays suspect,
    /// and a check without budget stays suspect.
    #[test]
    fn an_under_read_final_score_line_is_certified() {
        let fs = final_template();
        let levels = |img: &RgbImage| {
            region(img, fs.layout.roi, fs.layout.plane).expect("region inside the frame")
        };
        let on = levels(&final_frame(&final_line()));
        assert!(fs.settle(&on, 0.6, CERT_MACS).is_signal(), "certified");
        assert!(
            matches!(fs.settle(&on, 0.6, 0), FinalScoreRead::Suspect(_)),
            "no budget"
        );
        let off = levels(&final_frame(&glyphs(704, 123, 28)));
        assert!(
            matches!(fs.settle(&off, 0.6, CERT_MACS), FinalScoreRead::Suspect(_)),
            "no window at the floor"
        );
        assert!(matches!(
            fs.settle(&off, 0.3, CERT_MACS),
            FinalScoreRead::Absent(_)
        ));
    }

    /// `p4_final_score.png` loads with the words; a `p4_final_score` entry
    /// in layouts.json moves its region.
    #[test]
    fn the_final_score_template_loads_from_the_pack() {
        let dir = tempfile::tempdir().unwrap();
        pack(dir.path(), 1);
        assert!(
            ResultTemplates::load_dir(dir.path())
                .unwrap()
                .final_score()
                .is_none()
        );
        write_pack(dir.path(), &[("p4_final_score.png", &final_line())]);
        let t = ResultTemplates::load_dir(dir.path()).unwrap();
        assert!(
            t.final_score()
                .unwrap()
                .read(&final_frame(&final_line()))
                .is_signal()
        );
        std::fs::write(
            dir.path().join("layouts.json"),
            r#"[{"id": "p4_final_score", "roi": [0.05, 0.2, 0.35, 0.4], "plane": "min"}]"#,
        )
        .unwrap();
        let t = ResultTemplates::load_dir(dir.path()).unwrap();
        let fs = t.final_score().unwrap();
        assert!(
            !fs.read(&final_frame(&final_line())).is_signal(),
            "old region"
        );
        let mut moved = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
        paint_at(&mut moved, 300, 400, &final_line(), [235, 235, 235]);
        assert!(fs.read(&moved).is_signal(), "new region");
    }
}
