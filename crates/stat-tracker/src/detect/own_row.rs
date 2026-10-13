//! The member's own scoreboard row without name OCR (OCR site T4, alpha.7 O9).
//!
//! ocr-v1 reads every row's name with Tesseract and takes the row whose text
//! matches the configured in-game name, else the highlighted row. Under
//! `reader = "new"` no name is read. The daemon keeps a few crops of the
//! member's own name as it is drawn on the board (the seeds) and compares
//! each row's name area with them. Nothing in this module knows what a name
//! says: a seed is pixels, and a match is a correlation score.
//!
//! * **Geometry.** The name area hangs off the table's measured header bar
//!   and row bands ([`heroes::find_header`], [`heroes::find_rows`]), so it
//!   follows the 5v5 and 6v6 tables through every perk layout. Lengths are
//!   in row pitches from the header's left edge.
//! * **Mask.** Names are white on the team colour. The plane compared is the
//!   whiteness (the lowest channel), stretched between the area's median and
//!   its brightest half percent. A team colour, magenta and yellow included,
//!   has a low channel and stays dark. No luma, no Otsu.
//! * **Seed.** [`SeedStore`] trusts a crop only after three boards from two
//!   games gave crops that match each other. A crop is offered only from a
//!   board whose highlight is clear, whose career panel shows the hero on the
//!   highlighted row, and whose row is not under chat ([`seed_gate`]).
//! * **Match.** A seed slides over each row's name area. A row's score is
//!   the best whole-crop ZNCC, capped by the ZNCC of that window's weakest
//!   glyph-wide chunk, so one changed glyph is enough to fail. The best row
//!   wins when it reaches [`MATCH_FLOOR`] and leads the next row by
//!   [`MATCH_MARGIN`].
//! * **Combine.** [`combine`]: seed and highlight on one row is sure, one of
//!   them alone is used and marked, two different rows is no row.
//!
//! The trusted seeds live in `<data_dir>/private/own-name/seeds.json` (dir
//! 0700, file 0600, written atomically). Only crops confirmed as the member's
//! own name line are ever written: a crop still waiting for confirmation may
//! be another player's name, so it stays in memory and a restart drops it.
//! The file holds a hash of the configured name, never the name, and is
//! dropped when that hash changes. No report bundle, upload or log line
//! reads it.

use std::path::{Path, PathBuf};

use image::{DynamicImage, RgbImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::report::blank::Chat;
use crate::shadow::TabHeroes;
use crate::shadow::heroes::{self, View};

/// Left end of the area a seed is searched in: right of the portrait.
const SEARCH_X0: f64 = 1.45;
/// Left end of the name cell a seed is cut from: right of the ult-charge
/// disc. Measured 2.00 to 2.19 on the 2026-10-13 own-row boards.
const CELL_X0: f64 = 2.07;
/// Right end of both: left of the perk icons, which start at 4.95.
const CELL_X1: f64 = 4.90;
/// Vertical band of a row that can hold the name line. The title under a
/// name starts at 0.72.
const BAND_Y: (f64, f64) = (0.06, 0.74);
/// Row pitches a board can have (1080p 6v6 is 55, 2160p 5v5 is 176).
const ROW_H: (usize, usize) = (36, 220);
/// A seed is compared with boards whose row pitch is within this many px.
const SCALE_TOL: u32 = 2;

/// Whiteness span (brightest half percent minus median) an area needs
/// before it counts as holding text.
const MIN_SPAN: u32 = 40;
/// Normalised whiteness that counts as ink.
const INK: u8 = 128;
/// A column of the name line is solid with this much ink (in row pitches).
const SOLID: f64 = 0.10;
/// Solid columns closer than this are one word.
const WORD_GAP: f64 = 0.35;
/// A word is carried on to the right over thin ink (the foot of an L, the
/// bar of a T) for at most this many row pitches.
const THIN_TAIL: f64 = 0.30;
/// Narrowest and tallest name line a seed may have.
const MIN_NAME_W: f64 = 0.50;
const NAME_H: (f64, f64) = (0.20, 0.50);
/// A row of the name line is dense with this share of the word's width.
const DENSE_ROW: f64 = 0.15;
/// Blank zones a match needs beside the seed: `(width in row pitches, ink
/// share allowed)`. The right one keeps a seed from matching the start of a
/// longer name; the left one applies when the seed's own left side was blank.
const GUARD_RIGHT: (f64, f64) = (0.35, 0.06);
const GUARD_LEFT: (f64, f64) = (0.10, 0.05);
/// Ink share allowed in a new seed's own guards.
const SEED_GUARD_RIGHT: f64 = 0.04;
const SEED_GUARD_LEFT: f64 = 0.02;
/// A window is scored only when its ink count is within this range of the
/// seed's.
const INK_RATIO: (f64, f64) = (0.70, 1.40);
/// Width of the chunks a matched seed is checked in, in row pitches: a
/// little more than one glyph (0.24).
const CHUNK_W: f64 = 0.30;
/// Ink share a chunk of the seed needs to be scored.
const CHUNK_MIN_INK: f64 = 0.10;

/// Score a row needs to be the seed's row: its whole-crop ZNCC and the ZNCC
/// of its weakest glyph-wide chunk both reach this. On the alpha.7 boards
/// (lossless captures) the member's own row scored 0.88 to 1.00, any other
/// row at most 0.43, and the own name with one glyph swapped at most 0.74.
pub const MATCH_FLOOR: f32 = 0.82;
/// Lead over the next best row.
pub const MATCH_MARGIN: f32 = 0.10;
/// ZNCC two crops need, each on the other's row, to count as one name.
pub const AGREE: f32 = 0.85;
/// Boards and games a group of matching crops needs before it is trusted.
pub const SEED_BOARDS: usize = 3;
pub const SEED_GAMES: usize = 2;
/// The highlighted row must be this much brighter than the next brightest
/// row before a board may seed (two lit rows: a cursor hover, a party tint).
/// On the alpha.7 own-row boards a right highlight led by 1.11 to 1.71 (5th
/// percentile 1.24) and the one wrong highlight by 1.18.
pub const SEED_HIGHLIGHT_LEAD: f64 = 1.20;
/// Trusted seeds kept per board scale, and crops kept while waiting.
const TRUSTED_PER_SCALE: usize = 4;
const PENDING_MAX: usize = 12;
/// Crops of a newly trusted group that become seeds.
const PROMOTE: usize = 3;

/// 2: the file holds trusted seeds only. A version 1 file could hold
/// waiting crops as well; it is deleted on open like any other version.
const SEED_FILE_VERSION: u32 = 2;
const PRIVATE_DIR: &str = "private";
const OWN_NAME_DIR: &str = "own-name";
const SEED_FILE: &str = "seeds.json";

fn px(v: f64) -> usize {
    v.round().max(0.0) as usize
}

// ------------------------------------------------------------------ planes

/// One row's name area as normalised whiteness, with summed-area tables.
#[derive(Clone)]
struct Plane {
    w: usize,
    h: usize,
    v: Vec<u8>,
    /// `(w + 1) * (h + 1)` summed-area tables of ink, value and value².
    ink: Vec<u32>,
    sum: Vec<u64>,
    sq: Vec<u64>,
}

impl Plane {
    fn new(w: usize, h: usize, v: Vec<u8>) -> Self {
        debug_assert_eq!(v.len(), w * h);
        let stride = w + 1;
        let mut ink = vec![0u32; stride * (h + 1)];
        let mut sum = vec![0u64; stride * (h + 1)];
        let mut sq = vec![0u64; stride * (h + 1)];
        for y in 0..h {
            let (mut ri, mut rs, mut rq) = (0u32, 0u64, 0u64);
            for x in 0..w {
                let p = v[y * w + x];
                ri += u32::from(p >= INK);
                rs += u64::from(p);
                rq += u64::from(p) * u64::from(p);
                let at = (y + 1) * stride + x + 1;
                ink[at] = ink[at - stride] + ri;
                sum[at] = sum[at - stride] + rs;
                sq[at] = sq[at - stride] + rq;
            }
        }
        Self {
            w,
            h,
            v,
            ink,
            sum,
            sq,
        }
    }

    /// The whiteness of `[x0, x1) x [y0, y1)` of `img`, stretched between
    /// its median and its brightest half percent. `None` when the area is
    /// off the image or holds no text.
    fn from_rgb(img: &RgbImage, x0: usize, x1: usize, y0: usize, y1: usize) -> Option<Self> {
        let (iw, ih) = (img.width() as usize, img.height() as usize);
        if x1 > iw || y1 > ih || x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let raw = img.as_raw();
        let mut white = Vec::with_capacity(w * h);
        let mut hist = [0u32; 256];
        for y in y0..y1 {
            let row = &raw[(y * iw + x0) * 3..(y * iw + x1) * 3];
            for p in row.as_chunks::<3>().0 {
                let m = p[0].min(p[1]).min(p[2]);
                hist[usize::from(m)] += 1;
                white.push(m);
            }
        }
        let n = (w * h) as u32;
        let level = |want: u32| {
            let mut seen = 0u32;
            for (value, &count) in hist.iter().enumerate() {
                seen += count;
                if seen >= want {
                    return value as u32;
                }
            }
            255
        };
        let bg = level(n.div_ceil(2));
        let peak = level(n - n / 200);
        let span = peak.saturating_sub(bg);
        if span < MIN_SPAN {
            return None;
        }
        let v = white
            .into_iter()
            .map(|m| (u32::from(m).saturating_sub(bg) * 255 / span).min(255) as u8)
            .collect();
        Some(Self::new(w, h, v))
    }

    fn table<T: Copy + std::ops::Add<Output = T> + std::ops::Sub<Output = T>>(
        &self,
        t: &[T],
        x0: usize,
        x1: usize,
        y0: usize,
        y1: usize,
    ) -> T {
        let s = self.w + 1;
        t[y1 * s + x1] + t[y0 * s + x0] - t[y0 * s + x1] - t[y1 * s + x0]
    }

    /// Ink px in `[x0, x1) x [y0, y1)`.
    fn ink_in(&self, x0: usize, x1: usize, y0: usize, y1: usize) -> u32 {
        self.table(&self.ink, x0, x1, y0, y1)
    }

    fn ink_share(&self, x0: usize, x1: usize, y0: usize, y1: usize) -> f64 {
        let area = (x1 - x0) * (y1 - y0);
        if area == 0 {
            return 0.0;
        }
        f64::from(self.ink_in(x0, x1, y0, y1)) / area as f64
    }
}

/// A seed's pixels with the sums a ZNCC needs.
struct Tpl<'a> {
    w: usize,
    h: usize,
    px: &'a [u8],
    left_blank: bool,
    ink: f64,
    sum: f64,
    /// `n * sum(t²) - sum(t)²`.
    var: f64,
}

impl<'a> Tpl<'a> {
    fn of(seed: &'a Seed) -> Option<Self> {
        let (w, h) = (seed.w as usize, seed.h as usize);
        if w == 0 || h == 0 || seed.px.len() != w * h {
            return None;
        }
        let n = (w * h) as f64;
        let (mut ink, mut sum, mut sq) = (0u32, 0u64, 0u64);
        for &p in &seed.px {
            ink += u32::from(p >= INK);
            sum += u64::from(p);
            sq += u64::from(p) * u64::from(p);
        }
        let var = n * sq as f64 - (sum as f64) * (sum as f64);
        (var > 0.0).then_some(Self {
            w,
            h,
            px: &seed.px,
            left_blank: seed.left_blank,
            ink: f64::from(ink),
            sum: sum as f64,
            var,
        })
    }
}

/// ZNCC of two equal-size pixel blocks, `0` when either is flat.
fn zncc(a: impl Iterator<Item = u8>, b: impl Iterator<Item = u8>) -> f64 {
    let (mut n, mut sa, mut sb, mut saa, mut sbb, mut sab) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
    for (x, y) in a.zip(b) {
        let (x, y) = (f64::from(x), f64::from(y));
        n += 1.0;
        sa += x;
        sb += y;
        saa += x * x;
        sbb += y * y;
        sab += x * y;
    }
    let (va, vb) = (n * saa - sa * sa, n * sbb - sb * sb);
    if va <= 0.0 || vb <= 0.0 {
        return 0.0;
    }
    (n * sab - sa * sb) / (va * vb).sqrt()
}

/// The score of `tpl` on `plane`: the best whole-crop ZNCC among the windows
/// whose guards are blank and whose ink count is near the seed's, capped by
/// that window's weakest glyph-wide chunk. `None` when no window qualifies.
///
/// The whole-crop ZNCC alone cannot tell a long name from the same name with
/// one glyph changed (about 0.87 on the alpha.7 boards). The chunk that holds
/// the changed glyph can: its own ZNCC drops to 0.74 or less, while every
/// chunk of the member's own name stays at 0.88 or more.
fn best_match(tpl: &Tpl<'_>, plane: &Plane, row_h: usize) -> Option<f32> {
    if tpl.w > plane.w || tpl.h > plane.h {
        return None;
    }
    let rh = row_h as f64;
    let (gr, gl) = (px(GUARD_RIGHT.0 * rh), px(GUARD_LEFT.0 * rh));
    let n = (tpl.w * tpl.h) as f64;
    let mut best: Option<(f64, usize, usize)> = None;
    for dy in 0..=plane.h - tpl.h {
        let (y0, y1) = (dy, dy + tpl.h);
        for dx in 0..=plane.w - tpl.w {
            let (x0, x1) = (dx, dx + tpl.w);
            let right = (x1 + gr).min(plane.w);
            if right - x1 >= 2 && plane.ink_share(x1, right, y0, y1) > GUARD_RIGHT.1 {
                continue;
            }
            let left = x0.saturating_sub(gl);
            if tpl.left_blank && x0 - left >= 2 && plane.ink_share(left, x0, y0, y1) > GUARD_LEFT.1
            {
                continue;
            }
            let ink = f64::from(plane.ink_in(x0, x1, y0, y1));
            if ink < INK_RATIO.0 * tpl.ink || ink > INK_RATIO.1 * tpl.ink {
                continue;
            }
            let sum = plane.table(&plane.sum, x0, x1, y0, y1) as f64;
            let sq = plane.table(&plane.sq, x0, x1, y0, y1) as f64;
            let var = n * sq - sum * sum;
            if var <= 0.0 {
                continue;
            }
            let mut cross = 0u64;
            for ty in 0..tpl.h {
                let t = &tpl.px[ty * tpl.w..(ty + 1) * tpl.w];
                let p = &plane.v[(y0 + ty) * plane.w + x0..(y0 + ty) * plane.w + x1];
                cross += u64::from(
                    t.iter()
                        .zip(p)
                        .map(|(&a, &b)| u32::from(a) * u32::from(b))
                        .sum::<u32>(),
                );
            }
            let score = (n * cross as f64 - tpl.sum * sum) / (tpl.var * var).sqrt();
            if best.is_none_or(|(b, _, _)| score > b) {
                best = Some((score, dy, dx));
            }
        }
    }
    let (whole, dy, dx) = best?;
    Some(whole.min(weakest_chunk(tpl, plane, row_h, dy, dx)) as f32)
}

/// The lowest ZNCC over the glyph-wide chunks of `tpl` at window `(dy, dx)`
/// of `plane`. Chunks are [`CHUNK_W`] row pitches wide and overlap by half;
/// a chunk of the seed with almost no ink (a gap) is not scored. `1` when
/// the seed is too narrow to cut.
fn weakest_chunk(tpl: &Tpl<'_>, plane: &Plane, row_h: usize, dy: usize, dx: usize) -> f64 {
    let width = px(CHUNK_W * row_h as f64).max(6).min(tpl.w);
    let step = (width / 2).max(1);
    let mut starts: Vec<usize> = (0..=tpl.w - width).step_by(step).collect();
    if starts.last().is_some_and(|&last| last + width < tpl.w) {
        starts.push(tpl.w - width);
    }
    let mut weakest = 1f64;
    for x in starts {
        let seed_px =
            (0..tpl.h).flat_map(|y| tpl.px[y * tpl.w + x..y * tpl.w + x + width].iter().copied());
        let ink = seed_px.clone().filter(|&p| p >= INK).count();
        if (ink as f64) < CHUNK_MIN_INK * (width * tpl.h) as f64 {
            continue;
        }
        let row_px = (0..tpl.h).flat_map(|y| {
            let at = (dy + y) * plane.w + dx + x;
            plane.v[at..at + width].iter().copied()
        });
        weakest = weakest.min(zncc(seed_px, row_px));
    }
    weakest
}

// ------------------------------------------------------------------- seeds

/// One crop of the member's own name line. Pixels only.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Seed {
    /// Row pitch of the board it was cut from (its scale).
    row_h: u32,
    w: u32,
    h: u32,
    #[serde(with = "hex")]
    px: Vec<u8>,
    /// The area left of the crop was blank on its board.
    left_blank: bool,
}

impl std::fmt::Debug for Seed {
    /// Sizes only: a crop is never printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Seed({}x{} @{})", self.w, self.h, self.row_h)
    }
}

impl Seed {
    fn same_scale(&self, row_h: u32) -> bool {
        self.row_h.abs_diff(row_h) <= SCALE_TOL
    }
}

/// A crop offered for seeding, with the name area it came from so two
/// candidates can be matched on each other's row.
///
/// A candidate is not yet known to be the member's own name: if the
/// highlight was wrong it is another player's name line. It lives in memory
/// only. The type cannot be serialised, so no code path can write one.
#[derive(Clone, PartialEq)]
pub struct Candidate {
    seed: Seed,
    area_w: u32,
    area_h: u32,
    area: Vec<u8>,
    /// Opaque game key (a hash of the session id).
    game: String,
}

impl std::fmt::Debug for Candidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Candidate({:?})", self.seed)
    }
}

impl Candidate {
    fn plane(&self) -> Option<Plane> {
        let (w, h) = (self.area_w as usize, self.area_h as usize);
        (w > 0 && h > 0 && self.area.len() == w * h).then(|| Plane::new(w, h, self.area.clone()))
    }

    /// The crop that becomes a seed when this candidate is trusted.
    pub fn seed(&self) -> &Seed {
        &self.seed
    }
}

/// Score of `seed` on `on`'s row.
fn seed_on(seed: &Seed, on: &Candidate) -> f32 {
    if !seed.same_scale(on.seed.row_h) {
        return -1.0;
    }
    let (Some(tpl), Some(plane)) = (Tpl::of(seed), on.plane()) else {
        return -1.0;
    };
    best_match(&tpl, &plane, on.seed.row_h as usize).unwrap_or(-1.0)
}

/// Two candidates show one name: each crop matches on the other's row.
fn agree(a: &Candidate, b: &Candidate) -> bool {
    seed_on(&a.seed, b) >= AGREE && seed_on(&b.seed, a) >= AGREE
}

mod hex {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(DIGITS[usize::from(b >> 4)] as char);
            out.push(DIGITS[usize::from(b & 15)] as char);
        }
        s.serialize_str(&out)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        let nibble = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        let raw = text.as_bytes();
        if raw.len() % 2 != 0 {
            return Err(serde::de::Error::custom("odd hex length"));
        }
        raw.as_chunks::<2>()
            .0
            .iter()
            .map(|pair| match (nibble(pair[0]), nibble(pair[1])) {
                (Some(hi), Some(lo)) => Ok(hi << 4 | lo),
                _ => Err(serde::de::Error::custom("not hex")),
            })
            .collect()
    }
}

// ------------------------------------------------------------------- board

/// The name areas of one board.
pub struct NameBoard {
    row_h: u32,
    /// Plane x where the name cell starts.
    cell_x0: usize,
    rows: Vec<Option<Plane>>,
    /// Row bands `(top, bottom)` in scoreboard crop rows.
    bands: Vec<(usize, usize)>,
}

fn rgb_of<'a>(img: &'a DynamicImage, owned: &'a mut Option<RgbImage>) -> &'a RgbImage {
    match img {
        DynamicImage::ImageRgb8(rgb) => rgb,
        other => owned.insert(other.to_rgb8()),
    }
}

impl NameBoard {
    /// Measure `scoreboard` (the `crop_scoreboard` cut). With the full
    /// `frame`, a table whose header bar starts at the crop's left edge is
    /// re-cut further left first, as the hero reader does
    /// ([`heroes::clipped_header_pad`]). `None` when no header bar or no row
    /// bands show, or the name cells fall off the image.
    pub fn measure(
        frame: Option<&DynamicImage>,
        scoreboard: &DynamicImage,
        team_size: usize,
    ) -> Option<Self> {
        if !(5..=6).contains(&team_size) {
            return None;
        }
        let mut owned = None;
        let rgb = rgb_of(scoreboard, &mut owned);
        let hdr = heroes::find_header(View::rgb(rgb))?;
        if let Some(frame) = frame
            && let Some(pad) =
                heroes::clipped_header_pad(frame.width(), frame.height(), scoreboard, hdr.0)
        {
            let wide = heroes::hero_crop(frame, pad).to_rgb8();
            let board = heroes::find_header(View::rgb(&wide))
                .and_then(|h| Self::from_layout(&wide, h, team_size));
            if board.is_some() {
                return board;
            }
        }
        Self::from_layout(rgb, hdr, team_size)
    }

    fn from_layout(
        rgb: &RgbImage,
        hdr: (usize, usize, usize, usize),
        team_size: usize,
    ) -> Option<Self> {
        let bands = heroes::find_rows(View::rgb(rgb), hdr, team_size);
        if bands.len() != 2 * team_size {
            return None;
        }
        let row_h = bands[0].1.checked_sub(bands[0].0)? + 2;
        if !(ROW_H.0..=ROW_H.1).contains(&row_h) {
            return None;
        }
        let rh = row_h as f64;
        let (x0, x1) = (hdr.0 + px(SEARCH_X0 * rh), hdr.0 + px(CELL_X1 * rh));
        if x1 > rgb.width() as usize {
            return None;
        }
        let rows = bands
            .iter()
            .map(|&(top, _)| {
                Plane::from_rgb(
                    rgb,
                    x0,
                    x1,
                    top + px(BAND_Y.0 * rh),
                    top + px(BAND_Y.1 * rh),
                )
            })
            .collect();
        Some(Self {
            row_h: row_h as u32,
            cell_x0: px((CELL_X0 - SEARCH_X0) * rh),
            rows,
            bands,
        })
    }

    /// Row pitch in px: the board's scale.
    pub fn row_h(&self) -> u32 {
        self.row_h
    }

    /// Rows on the board, both teams.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Bottom of `row`'s band in scoreboard crop rows.
    pub fn row_bottom(&self, row: usize) -> Option<u32> {
        self.bands.get(row).map(|b| b.1 as u32)
    }

    /// The name line of `row` as a seeding candidate. `None` when the row
    /// has no clean name line: no word in the name cell, a word that
    /// reaches the perk icons, ink right after the word, or a line of the
    /// wrong height.
    pub fn candidate(&self, row: usize) -> Option<Candidate> {
        let plane = self.rows.get(row)?.as_ref()?;
        let rh = f64::from(self.row_h);
        let solid = (px(SOLID * rh) as u32).max(3);
        let gap = px(WORD_GAP * rh);
        // The widest word (solid columns with small gaps) in the name cell.
        let mut words: Vec<(usize, usize)> = Vec::new();
        for x in self.cell_x0..plane.w {
            if plane.ink_in(x, x + 1, 0, plane.h) < solid {
                continue;
            }
            match words.last_mut() {
                Some(word) if x - word.1 <= gap => word.1 = x + 1,
                _ => words.push((x, x + 1)),
            }
        }
        let &(x0, x1) = words.iter().max_by_key(|w| w.1 - w.0)?;
        if ((x1 - x0) as f64) < MIN_NAME_W * rh || x1 + 1 >= plane.w {
            return None;
        }
        // The name line: the tallest run of rows dense across that word.
        let dense = (((x1 - x0) as f64 * DENSE_ROW) as u32).max(2);
        let (mut line, mut run) = ((0, 0), None);
        for y in 0..=plane.h {
            let on = y < plane.h && plane.ink_in(x0, x1, y, y + 1) >= dense;
            match (on, run) {
                (true, None) => run = Some(y),
                (false, Some(start)) => {
                    if y - start > line.1 - line.0 {
                        line = (start, y);
                    }
                    run = None;
                }
                _ => {}
            }
        }
        let height = (line.1 - line.0) as f64;
        if height < NAME_H.0 * rh || height > NAME_H.1 * rh {
            return None;
        }
        let (y0, y1) = (line.0.saturating_sub(1), (line.1 + 1).min(plane.h));
        // The last glyph may end in strokes too thin to be solid columns.
        let tail = (x1 + px(THIN_TAIL * rh)).min(plane.w);
        let mut x1 = x1;
        while x1 < tail && plane.ink_in(x1, x1 + 1, y0, y1) > 0 {
            x1 += 1;
        }
        if x1 + 1 >= plane.w {
            return None;
        }
        let right = (x1 + px(GUARD_RIGHT.0 * rh)).min(plane.w);
        if plane.ink_share(x1, right, y0, y1) > SEED_GUARD_RIGHT {
            return None;
        }
        let left = x0.saturating_sub(px(GUARD_LEFT.0 * rh));
        let left_blank = x0 - left >= 2 && plane.ink_share(left, x0, y0, y1) <= SEED_GUARD_LEFT;
        let mut crop = Vec::with_capacity((x1 - x0) * (y1 - y0));
        for y in y0..y1 {
            crop.extend_from_slice(&plane.v[y * plane.w + x0..y * plane.w + x1]);
        }
        Some(Candidate {
            seed: Seed {
                row_h: self.row_h,
                w: (x1 - x0) as u32,
                h: (y1 - y0) as u32,
                px: crop,
                left_blank,
            },
            area_w: plane.w as u32,
            area_h: plane.h as u32,
            area: plane.v.clone(),
            game: String::new(),
        })
    }

    /// Each row's best score over the seeds of this board's scale. A row
    /// with no name area, or no window a seed may be compared with, scores
    /// `-1`.
    pub fn scores(&self, seeds: &[Seed]) -> Vec<f32> {
        let tpls: Vec<Tpl<'_>> = seeds
            .iter()
            .filter(|s| s.same_scale(self.row_h))
            .filter_map(Tpl::of)
            .collect();
        self.rows
            .iter()
            .map(|plane| {
                let Some(plane) = plane else {
                    return -1.0;
                };
                tpls.iter()
                    .filter_map(|t| best_match(t, plane, self.row_h as usize))
                    .fold(-1.0, f32::max)
            })
            .collect()
    }

    /// A seed of this board's scale exists.
    pub fn seeded(&self, seeds: &[Seed]) -> bool {
        seeds.iter().any(|s| s.same_scale(self.row_h))
    }
}

/// The row a set of scores names: the best one, when it reaches
/// [`MATCH_FLOOR`] and leads every other row by [`MATCH_MARGIN`].
pub fn seed_row(scores: &[f32]) -> Option<usize> {
    let (row, &best) = scores
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))?;
    let next = scores
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != row)
        .map(|(_, &s)| s)
        .fold(-1.0, f32::max);
    (best >= MATCH_FLOOR && best - next >= MATCH_MARGIN).then_some(row)
}

// ----------------------------------------------------------------- combine

/// How far the own row can be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// The seed and the highlight name the same row.
    Sure,
    /// The highlight alone, on a board no seed exists for yet. This is
    /// the row the new reader used before seeds existed.
    Unconfirmed,
    /// One of the two fired where both could have. The row is used and its
    /// fields are marked suspect.
    Suspect,
}

/// Which evidence named the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Both,
    SeedOnly,
    HighlightOnly,
    /// Seed and highlight name different rows: no row.
    Disagree,
    Neither,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Both => "both",
            Source::SeedOnly => "seed",
            Source::HighlightOnly => "highlight",
            Source::Disagree => "disagree",
            Source::Neither => "none",
        }
    }
}

/// The own row of one board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnRow {
    pub row: Option<usize>,
    pub trust: Trust,
    pub source: Source,
}

impl OwnRow {
    /// The row is used, and its fields are marked suspect.
    pub fn suspect(&self) -> bool {
        self.row.is_some() && self.trust == Trust::Suspect
    }
}

/// Combine the seed match with the highlight. `seeded` says a seed of the
/// board's scale exists, so the seed had its say.
///
/// * both on one row: that row, sure;
/// * both, on different rows: no row (the capture is held);
/// * the seed alone: its row, suspect;
/// * the highlight alone: its row, suspect when a seed exists and
///   unconfirmed when none does;
/// * neither: no row.
///
/// No case falls back to a name read.
pub fn combine(seed: Option<usize>, highlight: Option<usize>, seeded: bool) -> OwnRow {
    let (row, trust, source) = match (seed, highlight) {
        (Some(s), Some(h)) if s == h => (Some(s), Trust::Sure, Source::Both),
        (Some(_), Some(_)) => (None, Trust::Suspect, Source::Disagree),
        (Some(s), None) => (Some(s), Trust::Suspect, Source::SeedOnly),
        (None, Some(h)) if seeded => (Some(h), Trust::Suspect, Source::HighlightOnly),
        (None, Some(h)) => (Some(h), Trust::Unconfirmed, Source::HighlightOnly),
        (None, None) => (None, Trust::Suspect, Source::Neither),
    };
    OwnRow { row, trust, source }
}

/// The highlighted row, with how far it leads the next brightest row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Highlight {
    pub row: usize,
    /// Brightness of the row over the next brightest row's.
    pub lead: f64,
}

/// The highlighted row's name line, cut for seeding, and where it was.
#[derive(Debug, Clone)]
pub struct RowCandidate {
    pub candidate: Candidate,
    pub row: usize,
    /// Bottom of the row in scoreboard crop rows.
    pub row_bottom: u32,
}

/// One board's own-row read.
#[derive(Debug, Clone)]
pub struct OwnRowRead {
    pub own: OwnRow,
    /// Best and next best row score against the seeds (`-1` without one).
    pub best: f32,
    pub next: f32,
    /// The highlighted row's name line, when the highlight is clear enough
    /// to seed from and no seed matched that row. The caller still has to
    /// pass [`seed_gate`].
    pub candidate: Option<RowCandidate>,
}

impl OwnRowRead {
    /// The highlight alone, unconfirmed: what the new reader used before
    /// seeds existed.
    pub fn highlight_only(highlight: Option<usize>) -> Self {
        Self {
            own: combine(None, highlight, false),
            best: -1.0,
            next: -1.0,
            candidate: None,
        }
    }
}

/// Read the own row of one board: match `seeds` on every row, combine with
/// `highlight`, and cut a seeding candidate from a clearly highlighted row
/// that no seed matched. `frame` is the full frame when there is one (a
/// saved board crop has none).
pub fn read_own_row(
    frame: Option<&DynamicImage>,
    scoreboard: &DynamicImage,
    team_size: usize,
    highlight: Option<Highlight>,
    seeds: &[Seed],
) -> OwnRowRead {
    let lit = highlight.map(|h| h.row);
    let Some(board) = NameBoard::measure(frame, scoreboard, team_size) else {
        // No name area to compare. With seeds in hand the highlight is
        // still only one of two voices, so its row is marked like any other
        // highlight a seed did not confirm.
        return OwnRowRead {
            own: combine(None, lit, !seeds.is_empty()),
            ..OwnRowRead::highlight_only(lit)
        };
    };
    let scores = board.scores(seeds);
    let mut sorted = scores.clone();
    sorted.sort_by(|a, b| b.total_cmp(a));
    let own = combine(seed_row(&scores), lit, board.seeded(seeds));
    let candidate = highlight
        .filter(|h| own.source == Source::HighlightOnly && h.lead >= SEED_HIGHLIGHT_LEAD)
        .and_then(|h| {
            Some(RowCandidate {
                candidate: board.candidate(h.row)?,
                row: h.row,
                row_bottom: board.row_bottom(h.row)?,
            })
        });
    OwnRowRead {
        own,
        best: sorted.first().copied().unwrap_or(-1.0),
        next: sorted.get(1).copied().unwrap_or(-1.0),
        candidate,
    }
}

/// Whether a board may offer its highlighted row's crop as a seed.
///
/// * `chat`: what the full frame says about chat over the board. A row the
///   chat stack reaches, or an unknown chat state, refuses.
/// * `heroes`: the hero icons of the Tab. The career panel must show a hero:
///   that panel is drawn only on the member's own live board, not on a
///   replay, a spectated board or a post-match table. The highlighted row's
///   icon must not name another hero. It may be unread (an ult ring or a
///   skull over the portrait), the way the hero authority accepts one read
///   beside an abstain ([`TabHeroes::authority`]).
pub fn seed_gate(
    row: usize,
    row_bottom: u32,
    crop_h: u32,
    chat: Chat,
    heroes: Option<&TabHeroes>,
) -> bool {
    if chat == Chat::Unknown || chat.covers(row_bottom, crop_h) {
        return false;
    }
    let Some(panel) = heroes.and_then(|h| h.panel_hero()) else {
        return false;
    };
    heroes
        .and_then(|h| h.own_hero(row))
        .is_none_or(|own| own == panel)
}

// ------------------------------------------------------------------- store

/// What [`SeedStore::offer`] did with a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// A trusted seed already matches it.
    Known,
    /// Kept, waiting for more boards or another game.
    Pending,
    /// It completed a group of [`SEED_BOARDS`] matching crops from
    /// [`SEED_GAMES`] games: the group is trusted now.
    Promoted,
}

/// What is on disk: the trusted seeds, and nothing else. A file with any
/// other key (the waiting crops an earlier draft of this file held) does not
/// parse, and a file that does not parse is deleted on open.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeedFile {
    version: u32,
    /// SHA-256 of the configured in-game name. Never the name.
    name_hash: String,
    trusted: Vec<Seed>,
}

/// The member's own-name seeds.
///
/// * `trusted`: crops confirmed as the member's own name line (three
///   matching crops from two games). In memory and in
///   `<data_dir>/private/own-name/seeds.json`.
/// * `pending`: crops still waiting for that confirmation. In memory only,
///   never written: an unconfirmed crop may be another player's name. A
///   restart of the tracker drops them, so the three crops from two games
///   have to come within one run.
pub struct SeedStore {
    /// `None` keeps the trusted seeds in memory only too.
    dir: Option<PathBuf>,
    name_hash: String,
    trusted: Vec<Seed>,
    pending: Vec<Candidate>,
}

/// Hash of the configured in-game name, as stored next to the seeds. A name
/// that is not set hashes like an empty one.
pub fn name_hash(configured_name: Option<&str>) -> String {
    let name = configured_name.unwrap_or("").trim().to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(b"scuffed-own-name-v1\0");
    hasher.update(name.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Opaque key for one game, from its session id.
pub fn game_key(session_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"scuffed-own-name-game\0");
    hasher.update(session_id.as_bytes());
    hasher.finalize()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `<data_dir>/private/own-name`.
pub fn seed_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(PRIVATE_DIR).join(OWN_NAME_DIR)
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
}

/// Create `path` as a real directory with mode 0700. `false` when it is, or
/// sits behind, something else (a symlink, a file).
fn private_dir(path: &Path) -> bool {
    if std::fs::symlink_metadata(path).is_err() {
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        if builder.create(path).is_err() {
            return false;
        }
    }
    if !is_real_dir(path) {
        return false;
    }
    crate::fs_mode::tighten_private_dir(path);
    true
}

impl SeedStore {
    /// Seeds kept in memory only (tests, and a data dir that cannot hold
    /// the private folder).
    pub fn in_memory(configured_name: Option<&str>) -> Self {
        Self {
            dir: None,
            name_hash: name_hash(configured_name),
            trusted: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// Open the trusted seeds under `data_dir`. A file written for another
    /// configured name, another file version, or one that does not parse
    /// (which includes a file with waiting crops in it) is deleted and the
    /// store starts empty. Nothing is created until a group of crops is
    /// trusted. The waiting list always starts empty.
    pub fn open(data_dir: &Path, configured_name: Option<&str>) -> Self {
        let dir = seed_dir(data_dir);
        let mut store = Self::in_memory(configured_name);
        let private = data_dir.join(PRIVATE_DIR);
        let exists = std::fs::symlink_metadata(&private).is_ok();
        if exists && (!is_real_dir(&private) || !is_real_dir_or_absent(&dir)) {
            tracing::warn!("own-row seed folder is not a plain folder; seeds stay in memory");
            return store;
        }
        store.dir = Some(dir.clone());
        let file = dir.join(SEED_FILE);
        let Ok(text) = std::fs::read_to_string(&file) else {
            return store;
        };
        match serde_json::from_str::<SeedFile>(&text) {
            Ok(saved)
                if saved.version == SEED_FILE_VERSION && saved.name_hash == store.name_hash =>
            {
                store.trusted = saved.trusted;
            }
            Ok(_) => {
                tracing::info!("configured in-game name changed: own-row seeds cleared");
                let _ = std::fs::remove_file(&file);
            }
            Err(_) => {
                tracing::warn!("own-row seed file unreadable: cleared");
                let _ = std::fs::remove_file(&file);
            }
        }
        store
    }

    /// The trusted seeds.
    pub fn trusted(&self) -> &[Seed] {
        &self.trusted
    }

    /// Crops waiting for more boards or another game.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Take a crop from a board that passed [`seed_gate`], seen in the game
    /// `game` ([`game_key`]).
    ///
    /// A crop a trusted seed already matches changes nothing. Any other is
    /// kept in memory, and once [`SEED_BOARDS`] kept crops from
    /// [`SEED_GAMES`] games match each other, that group is trusted and the
    /// trusted seeds are written. A waiting crop is never written.
    pub fn offer(&mut self, mut candidate: Candidate, game: &str) -> Offer {
        if self
            .trusted
            .iter()
            .any(|seed| seed_on(seed, &candidate) >= AGREE)
        {
            return Offer::Known;
        }
        candidate.game = game.to_string();
        let mut group: Vec<usize> = (0..self.pending.len())
            .filter(|&i| agree(&self.pending[i], &candidate))
            .collect();
        self.pending.push(candidate);
        group.push(self.pending.len() - 1);
        let mut games: Vec<&str> = group
            .iter()
            .map(|&i| self.pending[i].game.as_str())
            .collect();
        games.sort_unstable();
        games.dedup();
        let offer = if group.len() >= SEED_BOARDS && games.len() >= SEED_GAMES {
            let row_h = self.pending[group[0]].seed.row_h;
            // The newest crops of the group become seeds; the whole group
            // leaves the waiting list.
            let first = group.len().saturating_sub(PROMOTE);
            let seeds: Vec<Seed> = group[first..]
                .iter()
                .map(|&i| self.pending[i].seed.clone())
                .collect();
            let mut index = 0;
            self.pending.retain(|_| {
                index += 1;
                !group.contains(&(index - 1))
            });
            self.trusted.extend(seeds);
            // The oldest seeds of this scale make room.
            while self.trusted.iter().filter(|s| s.same_scale(row_h)).count() > TRUSTED_PER_SCALE {
                let Some(oldest) = self.trusted.iter().position(|s| s.same_scale(row_h)) else {
                    break;
                };
                self.trusted.remove(oldest);
            }
            Offer::Promoted
        } else {
            if self.pending.len() > PENDING_MAX {
                let extra = self.pending.len() - PENDING_MAX;
                self.pending.drain(..extra);
            }
            Offer::Pending
        };
        if offer == Offer::Promoted {
            self.save();
        }
        offer
    }

    /// Forget every seed and delete the file.
    pub fn clear(&mut self) {
        self.trusted.clear();
        self.pending.clear();
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_file(dir.join(SEED_FILE));
        }
    }

    /// Write the trusted seeds, and only those: a 0600 temp file in the 0700
    /// folder, then a rename. A failure keeps the seeds in memory and logs
    /// no path and no content.
    fn save(&self) {
        let Some(dir) = &self.dir else {
            return;
        };
        let written = (|| -> std::io::Result<()> {
            let private = dir
                .parent()
                .ok_or_else(|| std::io::Error::other("no parent"))?;
            if !private_dir(private) || !private_dir(dir) {
                return Err(std::io::Error::other("not a plain folder"));
            }
            let body = serde_json::to_vec(&SeedFile {
                version: SEED_FILE_VERSION,
                name_hash: self.name_hash.clone(),
                trusted: self.trusted.clone(),
            })
            .map_err(std::io::Error::other)?;
            let tmp = dir.join(format!("{SEED_FILE}.tmp"));
            let _ = std::fs::remove_file(&tmp);
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            {
                use std::io::Write as _;
                let mut file = options.open(&tmp)?;
                file.write_all(&body)?;
                file.sync_all()?;
            }
            crate::fs_mode::tighten_private_file(&tmp);
            std::fs::rename(&tmp, dir.join(SEED_FILE))
        })();
        if let Err(error) = written {
            tracing::warn!(kind = ?error.kind(), "own-row seeds not saved");
        }
    }
}

fn is_real_dir_or_absent(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_err() || is_real_dir(path)
}

#[cfg(test)]
#[path = "own_row_tests.rs"]
mod tests;
