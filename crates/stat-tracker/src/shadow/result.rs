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
//!   plane `max`; real frames: none. The words-v1 pack widens it with its
//!   `layouts.json` (the current title sits lower, y 43-56.5%).
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
//! flagging the match.
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
//! `end_title`, `tab_header` and 1080p-native accolade frames are untested.
//! The tracker's colour-flood banner (gold or red over y 30-70%) is not read
//! here: it depends on colour, and a word read covers the same screens.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::RgbImage;
use serde::{Deserialize, Serialize};

use super::banner::Gray;
use super::heroes::{DirStamp, dir_stamp};
use crate::ocr::preprocess::game_rect_16_9;

/// Identifies this recognizer's output. Bump on any change that can move a read.
pub const RESULT_RECOGNIZER_ID: &str = "result-v2";
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
const MIN_KEPT: f64 = 0.6;
/// Coarse peaks refined per template and scale.
const PEAKS: usize = 6;
/// Coarse peaks closer than this (coarse pixels, either axis) are one peak.
const PEAK_SEP: usize = 1;
/// Refine window half-width on the first level below the coarse one, then
/// on every later level (pixels of that level).
const REFINE_FIRST: usize = 3;
const REFINE: usize = 2;

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
            Self::new(ACCOLADE, [0.005, 0.035, 0.255, 0.0862], Plane::Max),
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

/// One pyramid level of a region: pixels plus integral images (sum and sum
/// of squares) for window statistics.
struct Level {
    w: usize,
    h: usize,
    px: Vec<f32>,
    s1: Vec<f64>,
    s2: Vec<f64>,
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
        Self { w, h, px, s1, s2 }
    }

    /// Sum of `s` over the `tw` x `th` window at (x, y).
    #[inline]
    fn rect(&self, s: &[f64], x: usize, y: usize, tw: usize, th: usize) -> f64 {
        let a = y * (self.w + 1) + x;
        let b = (y + th) * (self.w + 1) + x;
        s[b + tw] - s[b] - s[a + tw] + s[a]
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
}

impl Stack {
    fn new(t: &Gray) -> Option<Self> {
        let base = Kernel::new(t.w, t.h, &t.px)?;
        let var0 = base.var();
        let mut levels = vec![base];
        let (mut w, mut h, mut px) = (t.w, t.h, t.px.clone());
        while levels.len() <= MAX_LEVEL && (w / 2).min(h / 2) >= MIN_COARSE_PX {
            (w, h, px) = half(w, h, &px);
            match Kernel::new(w, h, &px) {
                Some(k) if k.var() >= MIN_KEPT * var0 => levels.push(k),
                _ => break,
            }
        }
        Some(Self { levels })
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

/// Zero-mean NCC of `k` at (x, y) of `p`; `None` on a flat window.
#[inline]
fn ncc_at(p: &Level, k: &Kernel, x: usize, y: usize) -> Option<f64> {
    let n = (k.w * k.h) as f64;
    let sum = p.rect(&p.s1, x, y, k.w, k.h);
    let var = p.rect(&p.s2, x, y, k.w, k.h) - sum * sum / n;
    if var <= 1e-9 {
        return None;
    }
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
fn search_fast(region: &[Level], st: &Stack) -> Found {
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
    let mut peaks: Vec<(usize, usize)> = Vec::with_capacity(PEAKS);
    for &(_, x, y) in &coarse {
        if peaks.len() == PEAKS {
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
    for (mut x, mut y) in peaks {
        let mut score = None;
        for l in (0..top).rev() {
            let (p, k) = (&region[l], &st.levels[l]);
            let r = if l + 1 == top { REFINE_FIRST } else { REFINE };
            let (cx, cy) = ((2 * x).min(p.w - k.w), (2 * y).min(p.h - k.h));
            let mut lvl_best: Option<(f64, usize, usize)> = None;
            for yy in cy.saturating_sub(r)..=(cy + r).min(p.h - k.h) {
                for xx in cx.saturating_sub(r)..=(cx + r).min(p.w - k.w) {
                    macs += k.w * k.h;
                    if let Some(v) = ncc_at(p, k, xx, yy)
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
    pub fn load_dir(dir: &Path) -> Option<Self> {
        let mut layouts = Layout::builtin();
        if let Ok(text) = std::fs::read_to_string(dir.join("layouts.json")) {
            match serde_json::from_str::<Vec<Layout>>(&text) {
                Ok(extra) => {
                    for l in extra {
                        layouts.retain(|k| k.id != l.id);
                        layouts.push(l);
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "result layouts.json unreadable, built-ins only")
                }
            }
        }
        let mut tpls = Vec::new();
        for layout in layouts {
            for outcome in Outcome::ALL {
                let p = dir.join(format!("{}_{}.png", layout.id, outcome.as_str()));
                let Ok(img) = image::open(&p) else { continue };
                let l = img.to_luma8();
                if l.width() == 0 || l.height() == 0 {
                    continue;
                }
                tpls.push((
                    layout.clone(),
                    outcome,
                    Gray {
                        w: l.width() as usize,
                        h: l.height() as usize,
                        px: l.pixels().map(|p| p.0[0] as f32 / 255.0).collect(),
                    },
                ));
            }
        }
        Self::from_templates(tpls)
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
                stacks: SCALES
                    .iter()
                    .filter_map(|sc| {
                        let tw = ((t.w as f64 * sc).round_ties_even() as usize).max(1);
                        let th = ((t.h as f64 * sc).round_ties_even() as usize).max(1);
                        Stack::new(&t.resize(tw, th))
                    })
                    .collect(),
            })
            .collect();
        Some(Self { tpls })
    }

    /// Source ids that have at least one template.
    pub fn sources(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.tpls.iter().map(|t| t.layout.id.as_str()).collect();
        v.dedup();
        v
    }

    /// Best score of each template (in load order) and the windows scored.
    fn scores(&self, rgb: &RgbImage, exhaustive: bool) -> Vec<Found> {
        let mut regions: Vec<([f64; 4], Plane, Option<Vec<Level>>)> = Vec::new();
        let mut out = Vec::with_capacity(self.tpls.len());
        for t in &self.tpls {
            let key = (t.layout.roi, t.layout.plane);
            let idx = match regions.iter().position(|r| (r.0, r.1) == key) {
                Some(i) => i,
                None => {
                    regions.push((key.0, key.1, region(rgb, key.0, key.1)));
                    regions.len() - 1
                }
            };
            let Some(p) = &regions[idx].2 else {
                out.push(NONE_FOUND);
                continue;
            };
            let mut best = NONE_FOUND;
            for st in &t.stacks {
                let f = if exhaustive {
                    search_full(p, st)
                } else {
                    search_fast(p, st)
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
    pub fn read(&self, rgb: &RgbImage) -> ResultRead {
        self.decide(&self.scores(rgb, false))
    }

    /// [`Self::read`] with every window of every region scored: the
    /// reference the fast search is checked against (tests, benches). Slow.
    pub fn read_exhaustive(&self, rgb: &RgbImage) -> ResultRead {
        self.decide(&self.scores(rgb, true))
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
        let words = self.words.as_deref().and_then(ResultTemplates::load_dir);
        if let Some(templates) = words {
            return Some(LoadedResults {
                templates,
                pack: ResultPack::WordsV1,
            });
        }
        let legacy = self.legacy.as_deref().and_then(ResultTemplates::load_dir);
        legacy.map(|templates| LoadedResults {
            templates,
            pack: ResultPack::Legacy,
        })
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

    // ---------------------------------------------- fast search (O1 item 2)

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
            let st = Stack::new(tpl).unwrap();
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
        assert_eq!(Stack::new(&glyphs(1, 120, 50)).unwrap().levels.len(), 3);
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
        crate::packs::swap_dir(&staging, &data.join(WORDS_SUBDIR)).unwrap();
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
}
