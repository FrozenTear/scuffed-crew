//! Map name from the Tab board's top banner (shadow only).
//!
//! The banner at the top right of a Tab board reads `[mode icon] [MODE] |
//! MAP NAME  TIME:m:ss` in white on a dark bar. The reader:
//!
//! * finds the `|` separator: a thin, tall column of white ink inside the 16:9
//!   game rect that reaches below the glyph baseline (an accented capital such
//!   as `Í` is just as tall but does not),
//! * takes the white ink right of it up to the first gap wider than a capital
//!   (the gap before `TIME:`), as one strip normalised to [`H`] rows,
//! * compares the whole strip against one template per banner spelling of
//!   every map in [`MAPS`]. A template is composed from a glyph atlas; each
//!   glyph may shift up to [`ELASTIC`] px against the strip (kerning), with
//!   neighbouring glyphs moving together. The score is the zero-mean
//!   normalised correlation of the strip with the shifted template.
//! * A read is sure when its score is at least [`FLOOR`] and it leads the
//!   best other map by at least [`MARGIN`]. Anything else is flagged. Brightness
//!   only, no hue: nothing here depends on team colour.
//!
//! The glyph atlas is game art and is not embedded. It comes from the
//! `maps-v1.tar` pack, which pack sync installs into
//! `<data_dir>/templates/maps-v1/` (`atlas.json` plus one 8-bit grey PNG per
//! glyph), the same policy as the hero icons. A member who copied a pack in
//! by hand has it in `<data_dir>/templates/maps/`; that folder is used only
//! when `maps-v1/` is missing or does not load ([`MapDirs`]). Without either
//! the map is not read. [`LiveMaps`] notices a folder that appears or is
//! swapped after the reader started and re-loads it in the background. A map
//! whose banner spelling needs a glyph the atlas lacks gets no template; the
//! floor keeps that from turning into a sure read of another map.
//!
//! The after-match accolade screen prints the same name in the same face at
//! the top left, right of the result word, with `MATCH TIME: m:ss` under it.
//! [`locate_accolade`] cuts that line by its fixed place in the game rect
//! and [`MapTemplates::read_accolade`] scores it against the same templates
//! plus the accolade-only spellings in [`ACCOLADE_ALIASES`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use image::imageops::{self, FilterType};
use image::{ImageBuffer, Luma, RgbImage};
use serde::Deserialize;

use crate::ocr::preprocess::game_rect_16_9;

/// Identifies this recognizer's output. Bump on any change that can move a read.
pub const MAP_RECOGNIZER_ID: &str = "map-v1";
/// Hand-installed template directory under the data dir (the
/// `reader_pack_url` location before alpha.7). The fallback for
/// [`TEMPLATE_SUBDIR_V1`].
pub const TEMPLATE_SUBDIR: &str = "templates/maps";
/// Where pack sync installs `maps-v1.tar` (`packs::PACKS`). Preferred.
pub const TEMPLATE_SUBDIR_V1: &str = "templates/maps-v1";
/// Minimum score for a sure read.
pub const FLOOR: f32 = 0.80;
/// Minimum lead over the best other map for a sure read.
pub const MARGIN: f32 = 0.06;
/// Strip and glyph height after normalisation.
pub const H: usize = 32;
/// Largest per-glyph shift, in normalised px.
pub const ELASTIC: usize = 3;
/// Strip and template widths may differ by this share at most.
const MAX_RATIO: f64 = 0.15;
const STEP_COST: f64 = 0.02;

/// Identifies the accolade map read. Bump on any change that can move one.
pub const ACCOLADE_MAP_RECOGNIZER_ID: &str = "accolade-map-v1";
/// Rows of an [`H`]-row glyph that hold a capital letter, top and bottom
/// edge. The banner strip window is cut around the `|` (15% of its height
/// above it, 12% below), which leaves the capitals of every maps-v1 glyph
/// between these rows. The accolade line has no `|`, so its window is laid
/// out from the capitals with the same proportions.
const CAP_ROWS: (f64, f64) = (8.0, 25.0);
/// Accolade map line search region, as fractions of the 16:9 game rect:
/// right of the result word, from under an overlay's top rows down to the
/// accolade result region's bottom edge, which stops above the nameplate
/// ([`super::result::ACCOLADE_Y1`]). No name is inside it.
const ACCOLADE_X: (f64, f64) = (0.125, 0.45);
const ACCOLADE_Y: (f64, f64) = (0.030, super::result::ACCOLADE_Y1);
/// Where the capitals of the accolade map line sit: top edge and height, as
/// fractions of the game rect height, with the tolerance on each. Measured
/// on native 1080p and 1440p frames (top 4.17%, height 1.48% to 1.53%).
const ACCOLADE_CAP_TOP: (f64, f64) = (0.0417, 0.0030);
const ACCOLADE_CAP_HEIGHT: (f64, f64) = (0.0125, 0.0180);
/// Bottom edge of the row band that holds the line (a `Q` tail adds a row).
const ACCOLADE_BAND_BOTTOM: (f64, f64) = (0.0535, 0.0605);
/// Top edge of that band: the capitals' top, or an accent joined to it.
const ACCOLADE_BAND_TOP: (f64, f64) = (0.0330, 0.0455);
// The region ends above the nameplate and holds the line's rows (checked
// at compile time).
const _: () = assert!(
    ACCOLADE_Y.1 < super::result::NAME_LINE_TOP
        && ACCOLADE_Y.0 < ACCOLADE_BAND_TOP.0
        && ACCOLADE_BAND_BOTTOM.1 < ACCOLADE_Y.1
);

/// Accolade spellings that are not a banner spelling of the map: (text on
/// the accolade screen, [`MapInfo::key`]). `ADLERSBRUNN` is the Halloween
/// label of Eichenwalde, which ocr-v1's table stores as Eichenwalde.
pub const ACCOLADE_ALIASES: &[(&str, &str)] = &[("ADLERSBRUNN", "eichenwalde")];

/// One map: stable key (OverFast API), the name ocr-v1 stores, banner
/// spellings, and its single game mode (`None` when it has several).
#[derive(Debug, PartialEq)]
pub struct MapInfo {
    pub key: &'static str,
    pub name: &'static str,
    pub banner: &'static [&'static str],
    pub mode: Option<&'static str>,
}

/// Current OW2 maps, from the OverFast API `/maps` list (2026-10-09). Names
/// follow ocr-v1's table where it has the map. Banner spellings that differ
/// from the plain upper-case name were checked on real boards (Grímsvötn shows
/// without `WATCHPOINT:`, Gibraltar with it).
pub const MAPS: &[MapInfo] = &[
    MapInfo {
        key: "aatlis",
        name: "Aatlis",
        banner: &["AATLIS"],
        mode: Some("Flashpoint"),
    },
    MapInfo {
        key: "antarctic-peninsula",
        name: "Antarctic Peninsula",
        banner: &["ANTARCTIC PENINSULA"],
        mode: Some("Control"),
    },
    MapInfo {
        key: "anubis",
        name: "Temple of Anubis",
        banner: &["TEMPLE OF ANUBIS"],
        mode: Some("Assault"),
    },
    MapInfo {
        key: "arena-victoriae",
        name: "Arena Victoriae",
        banner: &["ARENA VICTORIAE"],
        mode: Some("Control"),
    },
    MapInfo {
        key: "ayutthaya",
        name: "Ayutthaya",
        banner: &["AYUTTHAYA"],
        mode: Some("Capture the Flag"),
    },
    MapInfo {
        key: "black-forest",
        name: "Black Forest",
        banner: &["BLACK FOREST"],
        mode: Some("Elimination"),
    },
    MapInfo {
        key: "blizzard-world",
        name: "Blizzard World",
        banner: &["BLIZZARD WORLD"],
        mode: Some("Hybrid"),
    },
    MapInfo {
        key: "busan",
        name: "Busan",
        banner: &["BUSAN"],
        mode: Some("Control"),
    },
    MapInfo {
        key: "castillo",
        name: "Castillo",
        banner: &["CASTILLO"],
        mode: Some("Elimination"),
    },
    MapInfo {
        key: "chateau-guillard",
        name: "Château Guillard",
        banner: &["CHÂTEAU GUILLARD"],
        mode: Some("Deathmatch"),
    },
    MapInfo {
        key: "circuit-royal",
        name: "Circuit Royal",
        banner: &["CIRCUIT ROYAL"],
        mode: Some("Escort"),
    },
    MapInfo {
        key: "colosseo",
        name: "Colosseo",
        banner: &["COLOSSEO"],
        mode: Some("Push"),
    },
    MapInfo {
        key: "dorado",
        name: "Dorado",
        banner: &["DORADO"],
        mode: Some("Escort"),
    },
    MapInfo {
        key: "ecopoint-antarctica",
        name: "Ecopoint: Antarctica",
        banner: &["ECOPOINT: ANTARCTICA"],
        mode: Some("Elimination"),
    },
    MapInfo {
        key: "eichenwalde",
        name: "Eichenwalde",
        banner: &["EICHENWALDE"],
        mode: Some("Hybrid"),
    },
    MapInfo {
        key: "esperanca",
        name: "Esperanca",
        banner: &["ESPERANÇA"],
        mode: Some("Push"),
    },
    MapInfo {
        key: "gogadoro",
        name: "Gogadoro",
        banner: &["GOGADORO"],
        mode: Some("Control"),
    },
    MapInfo {
        key: "hanamura",
        name: "Hanamura",
        banner: &["HANAMURA"],
        mode: Some("Assault"),
    },
    MapInfo {
        key: "hanaoka",
        name: "Hanaoka",
        banner: &["HANAOKA"],
        mode: Some("Clash"),
    },
    MapInfo {
        key: "havana",
        name: "Havana",
        banner: &["HAVANA"],
        mode: Some("Escort"),
    },
    MapInfo {
        key: "hollywood",
        name: "Hollywood",
        banner: &["HOLLYWOOD"],
        mode: Some("Hybrid"),
    },
    MapInfo {
        key: "horizon",
        name: "Horizon Lunar Colony",
        banner: &["HORIZON LUNAR COLONY"],
        mode: Some("Assault"),
    },
    MapInfo {
        key: "ilios",
        name: "Ilios",
        banner: &["ILIOS"],
        mode: Some("Control"),
    },
    MapInfo {
        key: "junkertown",
        name: "Junkertown",
        banner: &["JUNKERTOWN"],
        mode: Some("Escort"),
    },
    MapInfo {
        key: "lijiang-tower",
        name: "Lijiang Tower",
        banner: &["LIJIANG TOWER"],
        mode: Some("Control"),
    },
    MapInfo {
        key: "kanezaka",
        name: "Kanezaka",
        banner: &["KANEZAKA"],
        mode: None,
    },
    MapInfo {
        key: "kings-row",
        name: "King's Row",
        banner: &["KING'S ROW"],
        mode: Some("Hybrid"),
    },
    MapInfo {
        key: "malevento",
        name: "Malevento",
        banner: &["MALEVENTO"],
        mode: None,
    },
    MapInfo {
        key: "midtown",
        name: "Midtown",
        banner: &["MIDTOWN"],
        mode: Some("Hybrid"),
    },
    MapInfo {
        key: "necropolis",
        name: "Necropolis",
        banner: &["NECROPOLIS"],
        mode: Some("Elimination"),
    },
    MapInfo {
        key: "nepal",
        name: "Nepal",
        banner: &["NEPAL"],
        mode: Some("Control"),
    },
    MapInfo {
        key: "neon-junction",
        name: "Neon Junction",
        banner: &["NEON JUNCTION"],
        mode: Some("Hybrid"),
    },
    MapInfo {
        key: "new-junk-city",
        name: "New Junk City",
        banner: &["NEW JUNK CITY"],
        mode: Some("Flashpoint"),
    },
    MapInfo {
        key: "new-queen-street",
        name: "New Queen Street",
        banner: &["NEW QUEEN STREET"],
        mode: Some("Push"),
    },
    MapInfo {
        key: "numbani",
        name: "Numbani",
        banner: &["NUMBANI"],
        mode: Some("Hybrid"),
    },
    MapInfo {
        key: "oasis",
        name: "Oasis",
        banner: &["OASIS"],
        mode: Some("Control"),
    },
    MapInfo {
        key: "paraiso",
        name: "Paraiso",
        banner: &["PARAÍSO"],
        mode: Some("Hybrid"),
    },
    MapInfo {
        key: "paris",
        name: "Paris",
        banner: &["PARIS"],
        mode: Some("Assault"),
    },
    MapInfo {
        key: "petra",
        name: "Petra",
        banner: &["PETRA"],
        mode: None,
    },
    MapInfo {
        key: "place-lacroix",
        name: "Place Lacroix",
        banner: &["PLACE LACROIX"],
        mode: Some("Push"),
    },
    MapInfo {
        key: "powder-keg-mine",
        name: "Powder Keg Mine",
        banner: &["POWDER KEG MINE"],
        mode: Some("Payload Race"),
    },
    MapInfo {
        key: "practice-range",
        name: "Practice Range",
        banner: &["PRACTICE RANGE"],
        mode: Some("Practice Range"),
    },
    MapInfo {
        key: "redwood-dam",
        name: "Redwood Dam",
        banner: &["REDWOOD DAM"],
        mode: Some("Push"),
    },
    MapInfo {
        key: "rialto",
        name: "Rialto",
        banner: &["RIALTO"],
        mode: Some("Escort"),
    },
    MapInfo {
        key: "route-66",
        name: "Route 66",
        banner: &["ROUTE 66"],
        mode: Some("Escort"),
    },
    MapInfo {
        key: "runasapi",
        name: "Runasapi",
        banner: &["RUNASAPI"],
        mode: Some("Push"),
    },
    MapInfo {
        key: "samoa",
        name: "Samoa",
        banner: &["SAMOA"],
        mode: Some("Control"),
    },
    MapInfo {
        key: "shambali-monastery",
        name: "Shambali Monastery",
        banner: &["SHAMBALI MONASTERY"],
        mode: Some("Escort"),
    },
    MapInfo {
        key: "suravasa",
        name: "Suravasa",
        banner: &["SURAVASA"],
        mode: Some("Flashpoint"),
    },
    MapInfo {
        key: "thames-district",
        name: "Thames District",
        banner: &["THAMES DISTRICT"],
        mode: Some("Payload Race"),
    },
    MapInfo {
        key: "throne-of-anubis",
        name: "Throne of Anubis",
        banner: &["THRONE OF ANUBIS"],
        mode: Some("Clash"),
    },
    MapInfo {
        key: "volskaya",
        name: "Volskaya Industries",
        banner: &["VOLSKAYA INDUSTRIES"],
        mode: Some("Assault"),
    },
    MapInfo {
        key: "watchpoint-gibraltar",
        name: "Watchpoint: Gibraltar",
        banner: &["WATCHPOINT: GIBRALTAR"],
        mode: Some("Escort"),
    },
    MapInfo {
        key: "watchpoint-grimsvotn",
        name: "Watchpoint: Grímsvötn",
        banner: &["GRÍMSVÖTN", "WATCHPOINT: GRÍMSVÖTN"],
        mode: Some("Escort"),
    },
    MapInfo {
        key: "workshop-chamber",
        name: "Workshop Chamber",
        banner: &["WORKSHOP CHAMBER"],
        mode: Some("Workshop"),
    },
    MapInfo {
        key: "workshop-expanse",
        name: "Workshop Expanse",
        banner: &["WORKSHOP EXPANSE"],
        mode: Some("Workshop"),
    },
    MapInfo {
        key: "workshop-green-screen",
        name: "Workshop Green Screen",
        banner: &["WORKSHOP GREEN SCREEN"],
        mode: Some("Workshop"),
    },
    MapInfo {
        key: "workshop-island",
        name: "Workshop Island",
        banner: &["WORKSHOP ISLAND"],
        mode: Some("Workshop"),
    },
    MapInfo {
        key: "wuxing-university",
        name: "Wuxing University",
        banner: &["WUXING UNIVERSITY"],
        mode: Some("Control"),
    },
];

/// Float grey image, row-major, values 0..1.
#[derive(Debug, Clone, PartialEq)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<f32>,
}

impl Gray {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![0.0; w * h],
        }
    }

    #[inline]
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.px[y * self.w + x]
    }

    /// Bilinear (triangle) resize, area-aware when shrinking. Values must be
    /// in 0..1: the image crate clamps float pixels to that range.
    pub fn resize(&self, w: usize, h: usize) -> Gray {
        if w == self.w && h == self.h {
            return self.clone();
        }
        let Some(buf) = ImageBuffer::<Luma<f32>, Vec<f32>>::from_raw(
            self.w as u32,
            self.h as u32,
            self.px.clone(),
        ) else {
            return Gray::new(w, h);
        };
        let out = imageops::resize(&buf, w.max(1) as u32, h.max(1) as u32, FilterType::Triangle);
        Gray {
            w: w.max(1),
            h: h.max(1),
            px: out.into_raw(),
        }
    }

    fn mean(&self) -> f64 {
        self.px.iter().map(|&v| v as f64).sum::<f64>() / self.px.len().max(1) as f64
    }
}

/// numpy-style percentile (linear interpolation).
pub(crate) fn percentile(v: &mut [f32], p: f64) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    let pos = p / 100.0 * (v.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let f = (pos - lo as f64) as f32;
    v[lo] + (v[hi] - v[lo]) * f
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn ncc(a: &Gray, b: &Gray) -> f64 {
    let (ma, mb) = (a.mean(), b.mean());
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for (&x, &y) in a.px.iter().zip(&b.px) {
        let (x, y) = (x as f64 - ma, y as f64 - mb);
        num += x * y;
        da += x * x;
        db += y * y;
    }
    let d = (da * db).sqrt();
    if d > 0.0 { num / d } else { 0.0 }
}

#[derive(Debug, Clone, Copy)]
struct Run {
    s: usize,
    e: usize,
    t: usize,
    b: usize,
    fill: usize,
}

/// The map-name strip found right of the banner's `|`.
#[derive(Debug, Clone)]
pub struct Located {
    /// [`H`] rows, width scaled with the same factor.
    pub strip: Gray,
    /// Frame pixels of the strip window: x0, y0, x1, y1.
    pub window: (u32, u32, u32, u32),
}

/// Find the banner's map-name strip, or `None` when there is no banner.
pub fn locate(rgb: &RgbImage) -> Option<Located> {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = game_rect_16_9(w, h);
    let (gwf, ghf) = (gw as f64, gh as f64);
    let y0 = gy + (0.026 * ghf) as u32;
    let y1 = (gy + (0.068 * ghf) as u32).min(h);
    let x0 = gx + (0.55 * gwf) as u32;
    let x1 = (gx + (0.99 * gwf) as u32).min(w);
    if y1 <= y0 + 4 || x1 <= x0 + 4 {
        return None;
    }
    let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut band = Gray::new(bw, bh);
    for y in 0..bh {
        for x in 0..bw {
            let p = rgb.get_pixel(x0 + x as u32, y0 + y as u32).0;
            band.px[y * bw + x] = p[0].min(p[1]).min(p[2]) as f32;
        }
    }
    let mut tmp = band.px.clone();
    let bg = percentile(&mut tmp, 30.0);
    let pk = percentile(&mut tmp, 99.7);
    if pk - bg < 50.0 {
        return None;
    }
    let thr = bg + 0.5 * (pk - bg);
    let ink = |x: usize, y: usize| band.at(x, y) > thr;
    let mut runs: Vec<Run> = Vec::new();
    let mut start: Option<usize> = None;
    for x in 0..=bw {
        let any = x < bw && (0..bh).any(|y| ink(x, y));
        match (any, start) {
            (true, None) => start = Some(x),
            (false, Some(s)) => {
                let (mut t, mut b, mut fill) = (usize::MAX, 0, 0);
                for c in s..x {
                    let mut n = 0;
                    for y in 0..bh {
                        if ink(c, y) {
                            n += 1;
                            t = t.min(y);
                            b = b.max(y + 1);
                        }
                    }
                    fill = fill.max(n);
                }
                runs.push(Run {
                    s,
                    e: x,
                    t,
                    b,
                    fill,
                });
                start = None;
            }
            _ => {}
        }
    }
    let mut best: Option<(Run, Vec<Run>, f64)> = None;
    for (i, r) in runs.iter().enumerate() {
        let hh = (r.b - r.t) as f64;
        let thin = ((r.e - r.s) as f64) <= (0.0022 * gwf).max(4.0);
        if !(thin && 0.0150 * ghf <= hh && hh <= 0.026 * ghf && r.fill as f64 >= 0.85 * hh) {
            continue;
        }
        let nb: Vec<f64> = runs[i + 1..(i + 6).min(runs.len())]
            .iter()
            .map(|q| q.b as f64)
            .collect();
        if !nb.is_empty() && (r.b as f64) - median(nb) < 0.08 * hh {
            continue;
        }
        let cap = 0.68 * hh;
        let mut name: Vec<Run> = Vec::new();
        let mut prev_e = r.e;
        for q in &runs[i + 1..] {
            let gap = (q.s - prev_e) as f64;
            let limit = if name.is_empty() {
                1.6 * cap
            } else {
                0.8 * cap
            };
            if gap > limit {
                break;
            }
            if (q.t as f64) < r.t as f64 - 0.35 * hh || (q.b as f64) > r.b as f64 + 0.35 * hh {
                break;
            }
            name.push(*q);
            prev_e = q.e;
        }
        if name.len() >= 2 {
            best = Some((*r, name, hh));
        }
    }
    let (bar, name, hh) = best?;
    let wt = bar.t.saturating_sub((0.15 * hh).round_ties_even() as usize);
    let wb = (bar.b + (0.12 * hh).round_ties_even() as usize).min(bh);
    let (xs, xe) = (name[0].s, name[name.len() - 1].e);
    let mut native = Gray::new(xe - xs, wb - wt);
    for y in wt..wb {
        for x in xs..xe {
            native.px[(y - wt) * (xe - xs) + (x - xs)] =
                ((band.at(x, y) - bg) / (pk - bg)).clamp(0.0, 1.0);
        }
    }
    let s = H as f64 / (wb - wt) as f64;
    let sw = (((xe - xs) as f64 * s).round_ties_even() as usize).max(1);
    Some(Located {
        strip: native.resize(sw, H),
        window: (
            x0 + xs as u32,
            y0 + wt as u32,
            x0 + xe as u32,
            y0 + wb as u32,
        ),
    })
}

/// Maximal runs of `true` in `on`, as half-open ranges.
fn true_runs(on: impl Iterator<Item = bool>) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut start = None;
    let mut n = 0;
    for (i, v) in on.enumerate() {
        match (v, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                runs.push((s, i));
                start = None;
            }
            _ => {}
        }
        n = i + 1;
    }
    if let Some(s) = start {
        runs.push((s, n));
    }
    runs
}

/// Find the map-name line of the after-match accolade screen, or `None`.
///
/// The line is white capitals at a fixed place in the game rect: right of
/// the result word, above `MATCH TIME`. The darkest channel is used, so the
/// team-coloured result word is not ink. The reader:
///
/// * takes the row band of white ink whose edges are where the line's are
///   ([`ACCOLADE_BAND_TOP`], [`ACCOLADE_BAND_BOTTOM`]); the match time under
///   it and a tab row above it are other bands, and a frame with ink through
///   all those rows has no band. Ink that runs off the top of the region (an
///   overlay) is cut away first,
/// * splits the band's ink into glyph runs and keeps the one group of at
///   least two runs with no gap wider than a capital,
/// * takes the capitals' top and bottom as the median over the runs (an
///   accent, an apostrophe or a `Q` tail moves one run, not the median) and
///   checks them against [`ACCOLADE_CAP_TOP`] and [`ACCOLADE_CAP_HEIGHT`],
/// * cuts the strip window around the capitals with the banner's proportions
///   ([`CAP_ROWS`]) and normalises it to [`H`] rows.
///
/// Anything else is `None`: this is the only gate that says "accolade line".
pub fn locate_accolade(rgb: &RgbImage) -> Option<Located> {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = game_rect_16_9(w, h);
    let (gwf, ghf) = (gw as f64, gh as f64);
    let y0 = gy + (ACCOLADE_Y.0 * ghf) as u32;
    let y1 = (gy + (ACCOLADE_Y.1 * ghf) as u32).min(h);
    let x0 = gx + (ACCOLADE_X.0 * gwf) as u32;
    let x1 = (gx + (ACCOLADE_X.1 * gwf) as u32).min(w);
    if y1 <= y0 + 4 || x1 <= x0 + 4 {
        return None;
    }
    let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut band = Gray::new(bw, bh);
    for y in 0..bh {
        for x in 0..bw {
            let p = rgb.get_pixel(x0 + x as u32, y0 + y as u32).0;
            band.px[y * bw + x] = p[0].min(p[1]).min(p[2]) as f32;
        }
    }
    let mut tmp = band.px.clone();
    let bg = percentile(&mut tmp, 30.0);
    let pk = percentile(&mut tmp, 99.7);
    if pk - bg < 50.0 {
        return None;
    }
    let thr = bg + 0.5 * (pk - bg);
    let ink = |x: usize, y: usize| band.at(x, y) > thr;
    // rows of the region as fractions of the game rect height
    let frac = |y: f64| (y0 as f64 + y - gy as f64) / ghf;
    let within = |v: f64, (lo, hi): (f64, f64)| lo <= v && v <= hi;

    let rows = true_runs((0..bh).map(|y| (0..bw).any(|x| ink(x, y))));
    // Ink that runs off the top of the region is not the line's: a
    // performance overlay's last text row ends just above the capitals.
    // Those rows are no glyph's top and stay empty in the strip.
    let clip = match rows.first() {
        Some(&(0, end)) => end,
        _ => 0,
    };
    let (bt, bb) = rows.into_iter().find(|&(t, b)| {
        within(frac(t as f64), ACCOLADE_BAND_TOP) && within(frac(b as f64), ACCOLADE_BAND_BOTTOM)
    })?;
    let rough = (bb - bt) as f64;
    // room for an accent above and a tail below, short of the match time
    let rt = bt
        .saturating_sub((0.40 * rough).round_ties_even() as usize)
        .max(clip);
    let rb = (bb + (0.25 * rough).round_ties_even() as usize).min(bh);
    let cols = true_runs((0..bw).map(|x| (rt..rb).any(|y| ink(x, y))));
    let mut groups: Vec<Vec<(usize, usize)>> = Vec::new();
    for run in cols {
        match groups.last_mut() {
            Some(g) if (run.0 - g[g.len() - 1].1) as f64 <= rough => g.push(run),
            _ => groups.push(vec![run]),
        }
    }
    groups.retain(|g| g.len() >= 2);
    let [name] = groups.as_slice() else {
        return None;
    };
    let (mut tops, mut bots) = (Vec::new(), Vec::new());
    for &(s, e) in name {
        let lit = |y: &usize| (s..e).any(|x| ink(x, *y));
        tops.push((rt..rb).find(lit)? as f64);
        bots.push((rt..rb).rev().find(lit)? as f64 + 1.0);
    }
    let (ct, cb) = (median(tops), median(bots));
    let cap = cb - ct;
    let (top, tol) = ACCOLADE_CAP_TOP;
    if (frac(ct) - top).abs() > tol || !within(cap / ghf, ACCOLADE_CAP_HEIGHT) {
        return None;
    }
    let rows_per_cap = CAP_ROWS.1 - CAP_ROWS.0;
    let wt = (ct - CAP_ROWS.0 / rows_per_cap * cap).round_ties_even() as i64;
    let wb = (cb + (H as f64 - CAP_ROWS.1) / rows_per_cap * cap).round_ties_even() as i64;
    let (xs, xe) = (name[0].0, name[name.len() - 1].1);
    let wh = (wb - wt).max(1) as usize;
    let mut native = Gray::new(xe - xs, wh);
    for (row, y) in (wt..wb).enumerate() {
        // rows outside the region stay empty, like the dark bar around
        // them, and so do the overlay's
        if y < clip as i64 || y as usize >= bh {
            continue;
        }
        for x in xs..xe {
            native.px[row * (xe - xs) + (x - xs)] =
                ((band.at(x, y as usize) - bg) / (pk - bg)).clamp(0.0, 1.0);
        }
    }
    let s = H as f64 / wh as f64;
    let sw = (((xe - xs) as f64 * s).round_ties_even() as usize).max(1);
    Some(Located {
        strip: native.resize(sw, H),
        window: (
            x0 + xs as u32,
            (y0 as i64 + wt).max(0) as u32,
            x0 + xe as u32,
            (y0 as i64 + wb).max(0) as u32,
        ),
    })
}

// ---------------------------------------------------------------- templates

#[derive(Deserialize)]
struct AtlasFile {
    height: usize,
    gap: f64,
    space: f64,
    glyphs: HashMap<String, String>,
}

/// One composed banner spelling: glyphs at nominal x positions.
#[derive(Debug, Clone)]
pub struct Tpl {
    parts: Vec<(i64, Gray)>,
    wd: usize,
}

impl Tpl {
    fn render(&self, offs: &[i64]) -> Gray {
        let mut out = Gray::new(self.wd, H);
        for (i, (px, g)) in self.parts.iter().enumerate() {
            let ix = px + offs.get(i).copied().unwrap_or(0);
            let s0 = ix.max(0);
            let e = (ix + g.w as i64).min(self.wd as i64);
            for x in s0..e {
                let gxp = (x - ix) as usize;
                for y in 0..H {
                    let o = &mut out.px[y * self.wd + x as usize];
                    *o = o.max(g.at(gxp, y));
                }
            }
        }
        out
    }

    /// Elastic score of `strip` against this template; -1 when the widths
    /// are too far apart to be the same word.
    pub fn score(&self, strip: &Gray) -> f64 {
        let (wo, wt) = (strip.w as f64, self.wd as f64);
        if self.wd == 0 || (wo / wt - 1.0).abs() > MAX_RATIO {
            return -1.0;
        }
        let o = strip.resize(self.wd, H);
        let d = ELASTIC as i64;
        let k = 2 * ELASTIC + 1;
        let n = self.parts.len();
        if n == 0 {
            return -1.0;
        }
        let mut loc = vec![vec![-1.0f64; k]; n];
        for (i, (px, g)) in self.parts.iter().enumerate() {
            let gm = g.mean();
            let gn =
                g.px.iter()
                    .map(|&v| (v as f64 - gm).powi(2))
                    .sum::<f64>()
                    .sqrt();
            let gn = if gn > 0.0 { gn } else { 1.0 };
            for (j, off) in (-d..=d).enumerate() {
                let s0 = px + off;
                let e = s0 + g.w as i64;
                if s0 < 0 || e > self.wd as i64 {
                    continue;
                }
                let s0 = s0 as usize;
                let mut wm = 0.0;
                for y in 0..H {
                    for x in 0..g.w {
                        wm += o.at(s0 + x, y) as f64;
                    }
                }
                wm /= (g.w * H) as f64;
                let (mut num, mut wn) = (0.0, 0.0);
                for y in 0..H {
                    for x in 0..g.w {
                        let wv = o.at(s0 + x, y) as f64 - wm;
                        num += (g.at(x, y) as f64 - gm) * wv;
                        wn += wv * wv;
                    }
                }
                let wn = if wn > 0.0 { wn.sqrt() } else { 1.0 };
                loc[i][j] = num / (gn * wn);
            }
        }
        let mut best = loc[0].clone();
        let mut back = vec![vec![0usize; k]; n];
        for i in 1..n {
            let mut nb = vec![-1e9f64; k];
            for j in 0..k {
                for kk in [j as i64 - 1, j as i64, j as i64 + 1] {
                    if kk < 0 || kk >= k as i64 {
                        continue;
                    }
                    let kk = kk as usize;
                    let v = best[kk] - STEP_COST * (j as f64 - kk as f64).abs();
                    if v > nb[j] {
                        nb[j] = v;
                        back[i][j] = kk;
                    }
                }
            }
            for j in 0..k {
                best[j] = nb[j] + loc[i][j];
            }
        }
        let mut j = 0;
        for c in 1..k {
            if best[c] > best[j] {
                j = c;
            }
        }
        let mut path = vec![j];
        for i in (1..n).rev() {
            j = back[i][j];
            path.push(j);
        }
        path.reverse();
        let offs: Vec<i64> = path.iter().map(|&p| p as i64 - d).collect();
        ncc(&o, &self.render(&offs))
    }
}

/// Accented capital to its base letter, for atlas lookups.
fn base_char(c: char) -> char {
    match c {
        'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => 'A',
        'É' | 'È' | 'Ê' | 'Ë' => 'E',
        'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
        'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'O',
        'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
        'Ç' => 'C',
        'Ñ' => 'N',
        '’' => '\'',
        c => c,
    }
}

/// Composed templates for every map the atlas can spell.
#[derive(Debug, Clone)]
pub struct MapTemplates {
    /// (index into [`MAPS`], banner spelling, template)
    tpls: Vec<(usize, &'static str, Tpl)>,
    /// The same for [`ACCOLADE_ALIASES`]. Only the accolade read scores
    /// these; a banner read is the same with or without them.
    accolade: Vec<(usize, &'static str, Tpl)>,
    /// Banner spellings that need a glyph the atlas lacks.
    pub unrenderable: Vec<&'static str>,
}

impl MapTemplates {
    /// The hand-installed folder, `<data_dir>/templates/maps`.
    pub fn dir_in(data_dir: &Path) -> PathBuf {
        data_dir.join(TEMPLATE_SUBDIR)
    }

    /// The pack sync folder, `<data_dir>/templates/maps-v1`.
    pub fn dir_v1_in(data_dir: &Path) -> PathBuf {
        data_dir.join(TEMPLATE_SUBDIR_V1)
    }

    /// Load `<dir>/atlas.json` and its glyph PNGs. `None` when missing or bad.
    ///
    /// Every glyph file `atlas.json` names must be one `g<4 lowercase
    /// hex>.png` file in `dir`, the maps-v1 layout pack sync allows
    /// (`packs::is_glyph_name`). A name with a `/`, a `..`, an absolute path
    /// or any other shape refuses the whole pack, so [`MapDirs`] falls back
    /// to the next folder and no image outside the pack is ever decoded.
    pub fn load_dir(dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(dir.join("atlas.json")).ok()?;
        let atlas: AtlasFile = serde_json::from_str(&text).ok()?;
        if atlas.height != H {
            return None;
        }
        if !atlas
            .glyphs
            .values()
            .all(|f| crate::packs::is_glyph_name(f))
        {
            tracing::warn!(
                folder = ?dir.file_name(),
                "map pack atlas.json names a file outside the pack layout; pack not used"
            );
            return None;
        }
        let mut glyphs: HashMap<char, Gray> = HashMap::new();
        for (k, file) in &atlas.glyphs {
            let Some(c) = k.chars().next() else { continue };
            let Ok(img) = image::open(dir.join(file)) else {
                continue;
            };
            let l = img.to_luma8();
            if l.height() as usize != H || l.width() == 0 {
                continue;
            }
            glyphs.insert(
                c,
                Gray {
                    w: l.width() as usize,
                    h: H,
                    px: l.pixels().map(|p| p.0[0] as f32 / 255.0).collect(),
                },
            );
        }
        if glyphs.is_empty() {
            return None;
        }
        Some(Self::compose_all(&glyphs, atlas.gap, atlas.space))
    }

    /// Build templates from glyphs (tests use synthetic glyphs).
    pub fn compose_all(glyphs: &HashMap<char, Gray>, gap: f64, space: f64) -> Self {
        let mut tpls = Vec::new();
        let mut unrenderable = Vec::new();
        for (mi, m) in MAPS.iter().enumerate() {
            for &v in m.banner {
                match compose(glyphs, gap, space, v) {
                    Some(t) => tpls.push((mi, v, t)),
                    None => unrenderable.push(v),
                }
            }
        }
        let accolade = ACCOLADE_ALIASES
            .iter()
            .filter_map(|&(text, key)| {
                let mi = MAPS.iter().position(|m| m.key == key)?;
                Some((mi, text, compose(glyphs, gap, space, text)?))
            })
            .collect();
        Self {
            tpls,
            accolade,
            unrenderable,
        }
    }

    pub fn template_count(&self) -> usize {
        self.tpls.len()
    }

    /// Read one frame's banner.
    pub fn read(&self, rgb: &RgbImage) -> MapRead {
        match locate(rgb) {
            Some(l) => self.read_strip(&l.strip),
            None => MapRead {
                map: None,
                best: None,
                score: 0.0,
                margin: 0.0,
                suspect: true,
                found_banner: false,
            },
        }
    }

    /// Read one frame's accolade map line ([`locate_accolade`]), with the
    /// banner's floor and margin. `found_banner` is "the line was found".
    pub fn read_accolade(&self, rgb: &RgbImage) -> MapRead {
        match locate_accolade(rgb) {
            Some(l) => self.read_accolade_strip(&l.strip),
            None => MapRead {
                map: None,
                best: None,
                score: 0.0,
                margin: 0.0,
                suspect: true,
                found_banner: false,
            },
        }
    }

    /// Score a located banner strip against every map.
    pub fn read_strip(&self, strip: &Gray) -> MapRead {
        self.read_strip_of(strip, self.tpls.iter())
    }

    /// Score a located accolade strip against every map, the
    /// [`ACCOLADE_ALIASES`] spellings included.
    pub fn read_accolade_strip(&self, strip: &Gray) -> MapRead {
        self.read_strip_of(strip, self.tpls.iter().chain(&self.accolade))
    }

    fn read_strip_of<'a>(
        &self,
        strip: &Gray,
        tpls: impl Iterator<Item = &'a (usize, &'static str, Tpl)>,
    ) -> MapRead {
        let mut best = vec![f64::NEG_INFINITY; MAPS.len()];
        for (mi, _, t) in tpls {
            best[*mi] = best[*mi].max(t.score(strip));
        }
        let mut order: Vec<usize> = (0..MAPS.len()).filter(|&i| best[i].is_finite()).collect();
        order.sort_by(|&a, &b| best[b].total_cmp(&best[a]));
        let s1 = order.first().map_or(-1.0, |&i| best[i]);
        let s2 = order.get(1).map_or(-1.0, |&i| best[i]);
        let found = s1 > 0.0;
        let sure = s1 >= FLOOR as f64 && s1 - s2 >= MARGIN as f64;
        let top = order.first().map(|&i| &MAPS[i]);
        MapRead {
            map: if found { top } else { None },
            best: top,
            score: s1.max(0.0) as f32,
            margin: (s1 - s2).max(0.0) as f32,
            suspect: !sure,
            found_banner: true,
        }
    }
}

fn compose(glyphs: &HashMap<char, Gray>, gap: f64, space: f64, text: &str) -> Option<Tpl> {
    let mut parts = Vec::new();
    let mut x = 0.0f64;
    for c in text.chars() {
        if c == ' ' {
            x += space - gap;
            continue;
        }
        let g = glyphs.get(&c).or_else(|| glyphs.get(&base_char(c)))?;
        parts.push((x.round_ties_even() as i64, g.clone()));
        x += g.w as f64 + gap;
    }
    let wd = (x - gap).round_ties_even().max(0.0) as usize;
    Some(Tpl { parts, wd })
}

/// One banner read. `map` names the best map when a strip was scored, even
/// when flagged; `suspect` is false only for a sure read.
#[derive(Debug, Clone, PartialEq)]
pub struct MapRead {
    pub map: Option<&'static MapInfo>,
    pub best: Option<&'static MapInfo>,
    pub score: f32,
    pub margin: f32,
    pub suspect: bool,
    pub found_banner: bool,
}

// ---------------------------------------------------------------- live pack

/// Which folder the map templates came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapPack {
    /// `templates/maps-v1/`, installed by pack sync.
    V1,
    /// `templates/maps/`, copied in by hand.
    Legacy,
}

impl MapPack {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "maps-v1",
            Self::Legacy => "maps",
        }
    }
}

/// The two map pack folders, in order of preference.
#[derive(Debug, Clone, Default)]
pub struct MapDirs {
    pub v1: Option<PathBuf>,
    pub legacy: Option<PathBuf>,
}

/// Identity of one pack folder: device, inode and mtime, or `None` when it
/// is missing. Pack sync swaps a folder in with `renameat2(RENAME_EXCHANGE)`
/// or a rename, so a new pack gives the live path a new inode. Adding or
/// removing a file in a hand-kept folder moves its mtime.
type DirStamp = Option<(u64, u64, i64, i64)>;
type MapStamp = [DirStamp; 2];

fn dir_stamp(dir: Option<&Path>) -> DirStamp {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(dir?).ok()?;
    m.is_dir()
        .then(|| (m.dev(), m.ino(), m.mtime(), m.mtime_nsec()))
}

/// Loaded templates and the folder they came from.
#[derive(Debug, Clone)]
pub struct LoadedMaps {
    pub templates: MapTemplates,
    pub pack: MapPack,
}

impl MapDirs {
    pub fn in_data_dir(data_dir: &Path) -> Self {
        Self {
            v1: Some(MapTemplates::dir_v1_in(data_dir)),
            legacy: Some(MapTemplates::dir_in(data_dir)),
        }
    }

    /// maps-v1 when it loads, else the hand-installed folder, else nothing.
    pub fn load(&self) -> Option<LoadedMaps> {
        if let Some(templates) = self.v1.as_deref().and_then(MapTemplates::load_dir) {
            return Some(LoadedMaps {
                templates,
                pack: MapPack::V1,
            });
        }
        if self.v1.as_deref().is_some_and(Path::exists) {
            tracing::warn!("maps-v1 pack folder does not load; trying templates/maps");
        }
        let templates = self.legacy.as_deref().and_then(MapTemplates::load_dir)?;
        Some(LoadedMaps {
            templates,
            pack: MapPack::Legacy,
        })
    }

    fn stamp(&self) -> MapStamp {
        [
            dir_stamp(self.v1.as_deref()),
            dir_stamp(self.legacy.as_deref()),
        ]
    }

    /// Load, or `None` when a folder changed while it loaded (a swap ran
    /// meanwhile; the caller retries later). `during` runs between the first
    /// stamp and the load (tests use it to swap a folder in).
    fn load_stable(&self, during: impl FnOnce()) -> Option<(Option<LoadedMaps>, MapStamp)> {
        let before = self.stamp();
        during();
        let maps = self.load();
        (self.stamp() == before).then_some((maps, before))
    }
}

/// The new reader's map templates, re-loaded when a map pack folder changes.
///
/// The same rules as [`super::heroes::LiveHeroSet`]: pack sync installs
/// maps-v1 in the background after the reader is set up. Each read checks
/// the two folders' stamps (two `stat` calls). On a change it starts one
/// background load and keeps reading with the templates it has, so capture
/// never waits on a load. A load that saw a folder change while it ran is
/// thrown away and retried on a later read, so the templates never mix
/// files from before and after a swap.
#[derive(Debug, Default)]
pub struct LiveMaps {
    inner: std::sync::Arc<LiveInner>,
}

type Current = (Option<std::sync::Arc<LoadedMaps>>, MapStamp);

#[derive(Debug, Default)]
struct LiveInner {
    dirs: MapDirs,
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

    fn install(&self, (maps, stamp): (Option<LoadedMaps>, MapStamp)) {
        tracing::info!(
            maps = maps.is_some(),
            map_pack = maps.as_ref().map(|m| m.pack.as_str()),
            map_templates = maps.as_ref().map(|m| m.templates.template_count()),
            "new reader map pack loaded"
        );
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = (maps.map(Into::into), stamp);
    }
}

impl LiveMaps {
    /// Load now (reader setup). A folder that keeps changing during the
    /// first tries leaves it empty; the next read loads it again.
    pub fn load(dirs: MapDirs) -> Self {
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

    /// The templates for this read. Starts a background reload when a pack
    /// folder changed since they were loaded.
    pub fn current(&self) -> Option<std::sync::Arc<LoadedMaps>> {
        let (maps, stamp) = self
            .inner
            .current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if self.inner.dirs.stamp() != stamp {
            self.start_reload();
        }
        maps
    }

    fn start_reload(&self) {
        let Ok(mut loader) = self.inner.loader.try_lock() else {
            return;
        };
        if loader.as_ref().is_some_and(|h| !h.is_finished()) {
            return;
        }
        let inner = std::sync::Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("map-pack-reload".into())
            .spawn(move || {
                let during = inner.before_load();
                match inner.dirs.load_stable(during) {
                    Some(loaded) => inner.install(loaded),
                    None => tracing::debug!("map pack changed while loading, retry later"),
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
            Err(e) => tracing::warn!(error = %e, "map pack reload not started"),
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

/// Synthetic glyph packs for tests. No game art.
#[cfg(test)]
pub(crate) mod test_fixtures {
    use super::*;
    use image::{GrayImage, Rgb};

    /// Synthetic 5x7-ish block glyphs, one distinct bit pattern per letter,
    /// scaled to `H` rows. Not the game font: only the pipeline is under test.
    pub(crate) fn glyph(c: char) -> Gray {
        // cap rows 7..25: the bar window adds room above and below, like
        // the real banner (the `|` reaches past the baseline)
        glyph_rows(c, 7)
    }

    /// [`glyph`] with the capital between rows `top` and 25. The accolade
    /// fixtures use 8, where maps-v1 keeps it ([`CAP_ROWS`]).
    pub(crate) fn glyph_rows(c: char, top: usize) -> Gray {
        let w = 12;
        let mut g = Gray::new(w, H);
        let code = c as u32;
        for y in top..25 {
            for x in 1..w - 1 {
                let cell = ((y - top) / 3) * 3 + (x - 1) / 4;
                let on = (code.wrapping_mul(2654435761) >> (cell % 29)) & 1 == 1 || x == 1;
                if on {
                    g.px[y * w + x] = 1.0;
                }
            }
        }
        g
    }

    pub(crate) const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ':6";
    pub(crate) const GAP: f64 = 2.0;
    pub(crate) const SPACE: f64 = 9.0;

    pub(crate) fn atlas() -> HashMap<char, Gray> {
        LETTERS.chars().map(|c| (c, glyph(c))).collect()
    }

    /// Write a synthetic pack (`atlas.json` plus one PNG per glyph) the way
    /// maps-v1 lays it out, with every glyph in `letters`.
    pub(crate) fn write_pack(dir: &Path, letters: &str) {
        write_pack_rows(dir, letters, 7);
    }

    /// [`write_pack`] with the [`accolade_atlas`] glyphs.
    pub(crate) fn write_accolade_pack(dir: &Path, letters: &str) {
        write_pack_rows(dir, letters, 8);
    }

    fn write_pack_rows(dir: &Path, letters: &str, top: usize) {
        std::fs::create_dir_all(dir).unwrap();
        let mut entries = Vec::new();
        for c in letters.chars() {
            let g = glyph_rows(c, top);
            let file = format!("g{:04x}.png", c as u32);
            let img = GrayImage::from_fn(g.w as u32, H as u32, |x, y| {
                image::Luma([(g.at(x as usize, y as usize) * 255.0).round() as u8])
            });
            img.save(dir.join(&file)).unwrap();
            entries.push(format!(
                "{}: {}",
                serde_json::json!(c.to_string()),
                serde_json::json!(file)
            ));
        }
        let json = format!(
            "{{\"height\": {H}, \"gap\": {GAP}, \"space\": {SPACE}, \"glyphs\": {{{}}}}}",
            entries.join(", ")
        );
        std::fs::write(dir.join("atlas.json"), json).unwrap();
    }

    /// Glyphs with the capital where maps-v1 keeps it, for accolade frames.
    pub(crate) fn accolade_atlas() -> HashMap<char, Gray> {
        LETTERS.chars().map(|c| (c, glyph_rows(c, 8))).collect()
    }

    /// What an accolade frame holds besides the map line.
    #[derive(Clone, Copy)]
    pub(crate) struct Accolade {
        pub size: (u32, u32),
        /// Left edge of the map line, as a fraction of the frame width
        /// (17.4% after VICTORY, 15.5% after DEFEAT).
        pub x: f64,
        /// Top edge of the capitals, as a fraction of the frame height.
        pub cap_top: f64,
        /// Height of the capitals, as a fraction of the frame height.
        pub cap: f64,
        /// White accent marks above the first and third glyph, joined to
        /// them (`Í`, `Ö`).
        pub accents: bool,
        /// The grey `MATCH TIME` line under the map.
        pub match_time: bool,
        /// A performance overlay's white text across the top of the frame,
        /// down to this fraction of the frame height.
        pub overlay_to: Option<f64>,
    }

    impl Accolade {
        pub(crate) fn at(size: (u32, u32)) -> Self {
            Self {
                size,
                x: 0.174,
                cap_top: 0.0417,
                cap: 0.015,
                accents: false,
                match_time: true,
                overlay_to: None,
            }
        }
    }

    /// A dark frame with an accolade header: a team-coloured result word
    /// block at the left, `text` in white from the [`accolade_atlas`] glyphs
    /// with capitals `cap` of the frame height tall, and the match time line.
    pub(crate) fn frame_with_accolade(text: &str, a: Accolade) -> RgbImage {
        let (w, h) = a.size;
        let mut img = RgbImage::from_pixel(w, h, Rgb([12, 14, 24]));
        let (wf, hf) = (w as f64, h as f64);
        // result word: tall, bright, team blue. Its darkest channel is dark.
        for y in (0.036 * hf) as u32..(0.085 * hf) as u32 {
            for x in (0.01 * wf) as u32..(0.145 * wf) as u32 {
                img.put_pixel(x, y, Rgb([20, 190, 255]));
            }
        }
        let t = compose(&accolade_atlas(), GAP, SPACE, text).unwrap();
        let tpl = t.render(&vec![0; t.parts.len()]);
        // 17 glyph rows hold the capital
        let s = a.cap * hf / 17.0;
        let (tw, th) = (
            (tpl.w as f64 * s).round() as u32,
            (H as f64 * s).round() as u32,
        );
        let scaled = tpl.resize(tw as usize, th as usize);
        let sx = (a.x * wf) as u32;
        let top = (a.cap_top * hf - 8.0 * s).round() as u32;
        for y in 0..th {
            for x in 0..tw {
                let v = scaled.at(x as usize, y as usize).clamp(0.0, 1.0);
                let c = (12.0 + 240.0 * v) as u8;
                img.put_pixel(sx + x, top + y, Rgb([c, c, c.max(24)]));
            }
        }
        if a.accents {
            let cap_top = (a.cap_top * hf).round() as u32;
            for (px, g) in [&t.parts[0], &t.parts[2]] {
                let gx = sx + ((*px as f64 + 3.0) * s) as u32;
                let gw = ((g.w as f64 - 6.0) * s) as u32;
                for y in cap_top - (5.5 * s) as u32..cap_top {
                    for x in gx..gx + gw {
                        img.put_pixel(x, y, Rgb([250, 250, 250]));
                    }
                }
            }
        }
        if let Some(to) = a.overlay_to {
            for y in (0.022 * hf) as u32..(to * hf) as u32 {
                for x in 0..w {
                    if x % 11 < 6 {
                        img.put_pixel(x, y, Rgb([255, 255, 255]));
                    }
                }
            }
        }
        if a.match_time {
            for y in (0.0667 * hf) as u32..(0.0815 * hf) as u32 {
                for x in sx..sx + (0.11 * wf) as u32 {
                    if (x - sx) % 14 < 9 {
                        img.put_pixel(x, y, Rgb([160, 164, 172]));
                    }
                }
            }
        }
        img
    }

    /// A dark 1920x1080 frame with a banner: `|` then `text` drawn from the
    /// synthetic glyphs at 1080p scale, then a tall `TIME` block.
    pub(crate) fn frame_with_banner(text: &str) -> RgbImage {
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb([20, 24, 40]));
        let glyphs = atlas();
        let t = compose(&glyphs, GAP, SPACE, text).unwrap();
        let tpl = t.render(&vec![0; t.parts.len()]);
        // bar window: 1080p bar is 20 px tall at y 35; strip window adds 15%/12%
        let (bar_t, bar_h) = (35u32, 20u32);
        let wt = bar_t - 3;
        let win_h = bar_h + 3 + 2;
        let s = win_h as f64 / H as f64;
        let x0 = 1600u32;
        for y in bar_t..bar_t + bar_h {
            for x in x0..x0 + 2 {
                img.put_pixel(x, y, Rgb([235, 235, 235]));
            }
        }
        let sx = x0 + 10;
        let w = (tpl.w as f64 * s).round() as u32;
        let scaled = tpl.resize(w as usize, win_h as usize);
        for y in 0..win_h {
            for x in 0..w {
                let v = scaled.at(x as usize, y as usize).clamp(0.0, 1.0);
                let c = (20.0 + 215.0 * v) as u8;
                img.put_pixel(sx + x, wt + y, Rgb([c, c, c.max(40)]));
            }
        }
        let tx = sx + w + 20;
        for y in 32..57 {
            for x in tx..tx + 60 {
                if (x - tx) % 14 < 9 {
                    img.put_pixel(x, y, Rgb([240, 240, 240]));
                }
            }
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::{
        Accolade, GAP, LETTERS, SPACE, accolade_atlas, atlas, frame_with_accolade,
        frame_with_banner, write_pack,
    };
    use super::*;
    use image::Rgb;

    const P1080: (u32, u32) = (1920, 1080);
    const P1440: (u32, u32) = (2560, 1440);

    fn accolade_templates() -> MapTemplates {
        MapTemplates::compose_all(&accolade_atlas(), GAP, SPACE)
    }

    #[test]
    fn synthetic_accolade_reads_back_sure_at_1080_and_1440() {
        let t = accolade_templates();
        for size in [P1080, P1440] {
            for name in ["NEPAL", "KING'S ROW", "NEW QUEEN STREET", "ILIOS"] {
                // after VICTORY and after the shorter DEFEAT
                for x in [0.174, 0.155] {
                    let a = Accolade {
                        x,
                        ..Accolade::at(size)
                    };
                    let r = t.read_accolade(&frame_with_accolade(name, a));
                    assert!(r.found_banner, "{name} {size:?}: line not found");
                    let m = r.map.expect("a map");
                    assert!(m.banner.contains(&name), "{name}: read {}", m.key);
                    assert!(!r.suspect, "{name}: score {} margin {}", r.score, r.margin);
                }
            }
        }
    }

    /// The strip window is laid out from the capitals. An accent joined to
    /// two of the glyphs moves those two runs' tops, not the line's.
    #[test]
    fn accents_above_the_accolade_line_do_not_move_the_window() {
        let t = accolade_templates();
        for size in [P1080, P1440] {
            let plain = Accolade::at(size);
            let accented = Accolade {
                accents: true,
                ..plain
            };
            let a = locate_accolade(&frame_with_accolade("SURAVASA", plain)).unwrap();
            let b = locate_accolade(&frame_with_accolade("SURAVASA", accented)).unwrap();
            assert_eq!(a.window, b.window, "{size:?}");
            let r = t.read_accolade(&frame_with_accolade("SURAVASA", accented));
            assert_eq!(r.map.map(|m| m.key), Some("suravasa"));
            assert!(!r.suspect, "score {} margin {}", r.score, r.margin);
        }
    }

    /// An overlay's text row ends just above the capitals (the member's
    /// 1440p frames: ink from the region's top edge down to 3.75%). It is
    /// not part of the line: the window and the read are the same without it.
    #[test]
    fn an_overlay_above_the_accolade_line_is_cut_away() {
        let t = accolade_templates();
        for size in [P1080, P1440] {
            let plain = Accolade::at(size);
            let overlaid = Accolade {
                overlay_to: Some(0.0375),
                ..plain
            };
            let a = locate_accolade(&frame_with_accolade("NEPAL", plain)).unwrap();
            let b = locate_accolade(&frame_with_accolade("NEPAL", overlaid)).expect("line found");
            assert_eq!(a.window, b.window, "{size:?}");
            // the overlay moves the white level a little, nothing else
            assert_eq!((a.strip.w, a.strip.h), (b.strip.w, b.strip.h));
            let worst = a
                .strip
                .px
                .iter()
                .zip(&b.strip.px)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0f32, f32::max);
            assert!(worst < 0.05, "{size:?}: overlay ink in the strip ({worst})");
            let r = t.read_accolade(&frame_with_accolade("NEPAL", overlaid));
            assert_eq!(r.map.map(|m| m.key), Some("nepal"));
            assert!(!r.suspect, "score {} margin {}", r.score, r.margin);
            // an overlay that reaches the capitals leaves no clean line
            let joined = Accolade {
                overlay_to: Some(0.0430),
                ..plain
            };
            assert!(locate_accolade(&frame_with_accolade("NEPAL", joined)).is_none());
        }
    }

    /// White capitals of the right size in the wrong rows are not the map
    /// line: the competitive progression screen has a tab row there.
    #[test]
    fn a_line_off_the_accolade_rows_is_not_located() {
        let t = accolade_templates();
        for size in [P1080, P1440] {
            for cap_top in [0.0350, 0.0470] {
                let a = Accolade {
                    cap_top,
                    match_time: false,
                    ..Accolade::at(size)
                };
                let img = frame_with_accolade("NEPAL", a);
                assert!(locate_accolade(&img).is_none(), "{size:?} {cap_top}");
                let r = t.read_accolade(&img);
                assert!(!r.found_banner && r.map.is_none() && r.suspect);
            }
        }
    }

    /// Capitals of another size in the line's rows are not the map line.
    #[test]
    fn a_line_of_another_size_is_not_located() {
        for size in [P1080, P1440] {
            for (cap_top, cap) in [(0.0380, 0.0220), (0.0430, 0.0105)] {
                let a = Accolade {
                    cap_top,
                    cap,
                    ..Accolade::at(size)
                };
                assert!(
                    locate_accolade(&frame_with_accolade("NEPAL", a)).is_none(),
                    "{size:?} {cap}"
                );
            }
        }
    }

    /// The match time alone, a frame with no ink, and ink through every row
    /// of the region are not a map line.
    #[test]
    fn frames_without_the_map_line_are_not_located() {
        let blank = RgbImage::from_pixel(1920, 1080, Rgb([12, 14, 24]));
        assert!(locate_accolade(&blank).is_none());
        let mut only_time = frame_with_accolade("NEPAL", Accolade::at(P1080));
        for y in 30..70 {
            for x in 300..900 {
                only_time.put_pixel(x, y, Rgb([12, 14, 24]));
            }
        }
        assert!(locate_accolade(&only_time).is_none());
        let mut flooded = frame_with_accolade("NEPAL", Accolade::at(P1080));
        for y in 30..95 {
            for x in 260..270 {
                flooded.put_pixel(x, y, Rgb([250, 250, 250]));
            }
        }
        assert!(locate_accolade(&flooded).is_none(), "no row band");
    }

    /// Two groups of glyphs on the line (something white beside the name)
    /// is not a clean line: abstain rather than pick one.
    #[test]
    fn two_groups_on_the_accolade_line_abstain() {
        let mut img = frame_with_accolade("NEPAL", Accolade::at(P1080));
        // a second word 200 px right of the name, in the same rows
        let second = frame_with_accolade(
            "OASIS",
            Accolade {
                x: 0.174 + 0.12,
                ..Accolade::at(P1080)
            },
        );
        for y in 40..64 {
            for x in 500..900 {
                img.put_pixel(x, y, *second.get_pixel(x, y));
            }
        }
        assert!(locate_accolade(&img).is_none());
    }

    /// A Tab banner is not an accolade line and the reverse.
    #[test]
    fn banner_and_accolade_frames_do_not_read_as_each_other() {
        let t = MapTemplates::compose_all(&atlas(), GAP, SPACE);
        let banner = frame_with_banner("NEPAL");
        assert!(!t.read_accolade(&banner).found_banner);
        let accolade = frame_with_accolade("NEPAL", Accolade::at(P1080));
        assert!(!accolade_templates().read(&accolade).found_banner);
    }

    /// `ADLERSBRUNN` on the accolade screen is Eichenwalde. The banner read
    /// has no such spelling and is the same with or without the alias.
    #[test]
    fn the_accolade_alias_reads_as_its_map_and_the_banner_ignores_it() {
        assert!(
            ACCOLADE_ALIASES
                .iter()
                .all(|(_, key)| MAPS.iter().any(|m| m.key == *key)),
            "every alias names a map"
        );
        let t = accolade_templates();
        let r = t.read_accolade(&frame_with_accolade("ADLERSBRUNN", Accolade::at(P1080)));
        assert_eq!(r.map.map(|m| m.key), Some("eichenwalde"));
        assert!(!r.suspect, "score {} margin {}", r.score, r.margin);

        let banners: usize = MAPS.iter().map(|m| m.banner.len()).sum();
        let b = MapTemplates::compose_all(&atlas(), GAP, SPACE);
        assert_eq!(
            b.template_count(),
            banners,
            "aliases are not banner templates"
        );
        let on_banner = b.read(&frame_with_banner("ADLERSBRUNN"));
        assert!(on_banner.found_banner && on_banner.suspect);
    }

    /// The strip window of a located line stays above the accolade
    /// nameplate (the region itself is checked at compile time).
    #[test]
    fn the_accolade_region_stays_off_the_nameplate() {
        use crate::shadow::result::NAME_LINE_TOP;
        let img = frame_with_accolade("SHAMBALI MONASTERY", Accolade::at(P1440));
        let l = locate_accolade(&img).unwrap();
        let (_, y0, _, y1) = l.window;
        assert!((y1 as f64) < NAME_LINE_TOP * 1440.0 && (y0 as f64) > 0.03 * 1440.0);
    }

    #[test]
    fn every_map_has_a_banner_spelling_and_unique_key() {
        let mut keys: Vec<_> = MAPS.iter().map(|m| m.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), MAPS.len());
        assert!(MAPS.iter().all(|m| !m.banner.is_empty()));
        assert!(MAPS.len() >= 59);
    }

    #[test]
    fn synthetic_banner_reads_back_sure() {
        let t = MapTemplates::compose_all(&atlas(), GAP, SPACE);
        for name in ["NEPAL", "KING'S ROW", "SHAMBALI MONASTERY", "ILIOS"] {
            let img = frame_with_banner(name);
            let r = t.read(&img);
            assert!(r.found_banner, "{name}: banner not found");
            let m = r.map.expect("a map");
            assert!(m.banner.contains(&name), "{name}: read {}", m.key);
            assert!(!r.suspect, "{name}: score {} margin {}", r.score, r.margin);
        }
    }

    #[test]
    fn missing_true_map_is_flagged_not_sure() {
        let mut glyphs = atlas();
        // drop 'P': NEPAL (and others with P) can no longer be spelled
        glyphs.remove(&'P');
        let t = MapTemplates::compose_all(&glyphs, GAP, SPACE);
        assert!(t.unrenderable.contains(&"NEPAL"));
        let r = t.read(&frame_with_banner("NEPAL"));
        assert!(
            r.suspect,
            "read {:?} sure without a NEPAL template",
            r.map.map(|m| m.key)
        );
    }

    #[test]
    fn no_banner_on_a_blank_frame() {
        let t = MapTemplates::compose_all(&atlas(), GAP, SPACE);
        let r = t.read(&RgbImage::from_pixel(1920, 1080, Rgb([20, 24, 40])));
        assert!(!r.found_banner && r.map.is_none() && r.suspect);
    }

    #[test]
    fn missing_pack_dir_loads_nothing() {
        assert!(MapTemplates::load_dir(Path::new("/nonexistent/maps")).is_none());
    }

    fn sure_key(maps: &LoadedMaps, text: &str) -> Option<&'static str> {
        let r = maps.templates.read(&frame_with_banner(text));
        (!r.suspect).then(|| r.map.map(|m| m.key)).flatten()
    }

    /// Without `P`, NEPAL has no template: a frame of it is not a sure read.
    fn no_p() -> String {
        LETTERS.replace('P', "")
    }

    #[test]
    fn a_written_pack_loads_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        write_pack(dir.path(), LETTERS);
        let t = MapTemplates::load_dir(dir.path()).expect("synthetic pack loads");
        assert!(t.unrenderable.is_empty(), "{:?}", t.unrenderable);
        let r = t.read(&frame_with_banner("NEPAL"));
        assert_eq!(r.map.map(|m| m.key), Some("nepal"));
        assert!(!r.suspect);
    }

    #[test]
    fn maps_v1_is_preferred_over_the_hand_installed_folder() {
        let data = tempfile::tempdir().unwrap();
        write_pack(&MapTemplates::dir_v1_in(data.path()), LETTERS);
        write_pack(&MapTemplates::dir_in(data.path()), &no_p());
        let maps = MapDirs::in_data_dir(data.path()).load().unwrap();
        assert_eq!(maps.pack, MapPack::V1);
        assert_eq!(sure_key(&maps, "NEPAL"), Some("nepal"));
    }

    #[test]
    fn the_hand_installed_folder_is_the_fallback() {
        let data = tempfile::tempdir().unwrap();
        write_pack(&MapTemplates::dir_in(data.path()), &no_p());
        let dirs = MapDirs::in_data_dir(data.path());
        let maps = dirs.load().unwrap();
        assert_eq!(maps.pack, MapPack::Legacy);
        assert_eq!(sure_key(&maps, "ILIOS"), Some("ilios"));
        assert_eq!(sure_key(&maps, "NEPAL"), None, "the legacy pack has no P");

        // a maps-v1 folder that does not load is skipped, not used empty
        let v1 = MapTemplates::dir_v1_in(data.path());
        std::fs::create_dir_all(&v1).unwrap();
        std::fs::write(v1.join("atlas.json"), "not json").unwrap();
        assert_eq!(dirs.load().unwrap().pack, MapPack::Legacy);

        assert!(
            MapDirs::in_data_dir(Path::new("/nonexistent"))
                .load()
                .is_none()
        );
    }

    /// F251-7: an atlas that points a glyph anywhere but a `g<hex>.png` file
    /// in the pack refuses the whole pack, and the fallback folder is used.
    #[test]
    fn a_hostile_atlas_path_refuses_the_pack() {
        let data = tempfile::tempdir().unwrap();
        let v1 = MapTemplates::dir_v1_in(data.path());
        write_pack(&MapTemplates::dir_in(data.path()), LETTERS);
        // an image outside the pack that would load as a glyph
        let outside = data.path().join("outside.png");
        image::GrayImage::from_pixel(12, H as u32, image::Luma([255]))
            .save(&outside)
            .unwrap();
        let hostile = [
            outside.to_str().unwrap().to_string(),
            "../outside.png".into(),
            "sub/g0041.png".into(),
            "../maps/g0041.png".into(),
            "G0041.png".into(),
            "g0041.PNG".into(),
            "g041.png".into(),
            "g00411.png".into(),
            "g0041.png.png".into(),
            "".into(),
        ];
        for bad in hostile {
            write_pack(&v1, LETTERS);
            let text = std::fs::read_to_string(v1.join("atlas.json")).unwrap();
            let mut atlas: serde_json::Value = serde_json::from_str(&text).unwrap();
            atlas["glyphs"]["A"] = serde_json::json!(bad);
            std::fs::write(v1.join("atlas.json"), atlas.to_string()).unwrap();
            assert!(MapTemplates::load_dir(&v1).is_none(), "{bad:?} accepted");
            let maps = MapDirs::in_data_dir(data.path()).load().unwrap();
            assert_eq!(maps.pack, MapPack::Legacy, "{bad:?}");
        }
        // the same pack with its own names loads
        write_pack(&v1, LETTERS);
        assert!(MapTemplates::load_dir(&v1).is_some());
    }

    /// Install `templates/maps-v1` the way pack sync does: build it in a
    /// staging folder, then swap it into place.
    fn swap_in_v1(data: &Path, letters: &str, tag: &str) {
        let staging = data.join(format!("templates/.maps-v1-staging-{tag}"));
        write_pack(&staging, letters);
        crate::packs::swap_dir(&staging, &MapTemplates::dir_v1_in(data)).unwrap();
    }

    #[test]
    fn the_live_pack_picks_up_maps_v1_after_it_is_swapped_in() {
        let data = tempfile::tempdir().unwrap();
        let live = LiveMaps::load(MapDirs::in_data_dir(data.path()));
        assert!(live.current().is_none());
        live.wait_reload();
        assert!(live.current().is_none(), "nothing installed");
        swap_in_v1(data.path(), LETTERS, "a");
        // this read keeps what it has; the load runs aside
        assert!(live.current().is_none());
        live.wait_reload();
        let maps = live.current().expect("loaded after the swap");
        assert_eq!(maps.pack, MapPack::V1);
        assert_eq!(sure_key(&maps, "NEPAL"), Some("nepal"));
    }

    #[test]
    fn an_updated_maps_v1_replaces_the_old_one() {
        let data = tempfile::tempdir().unwrap();
        swap_in_v1(data.path(), &no_p(), "old");
        let live = LiveMaps::load(MapDirs::in_data_dir(data.path()));
        let old = live.current().unwrap();
        assert_eq!(sure_key(&old, "NEPAL"), None);
        swap_in_v1(data.path(), LETTERS, "new");
        live.current();
        live.wait_reload();
        let new = live.current().unwrap();
        assert!(!std::sync::Arc::ptr_eq(&old, &new), "re-loaded");
        assert_eq!(sure_key(&new, "NEPAL"), Some("nepal"));
    }

    #[test]
    fn a_hand_installed_pack_that_appears_later_is_picked_up() {
        let data = tempfile::tempdir().unwrap();
        let live = LiveMaps::load(MapDirs::in_data_dir(data.path()));
        assert!(live.current().is_none());
        write_pack(&MapTemplates::dir_in(data.path()), LETTERS);
        live.current();
        live.wait_reload();
        assert_eq!(live.current().unwrap().pack, MapPack::Legacy);
    }

    #[test]
    fn only_one_map_reload_runs_at_a_time() {
        let data = tempfile::tempdir().unwrap();
        let live = LiveMaps::load(MapDirs::in_data_dir(data.path()));
        live.set_load_delay_ms(400);
        swap_in_v1(data.path(), LETTERS, "a");
        for _ in 0..50 {
            assert!(live.current().is_none(), "load still in flight");
        }
        assert_eq!(live.reload_spawns(), 1);
        live.wait_reload();
        assert!(live.current().is_some());
        assert_eq!(live.reload_spawns(), 1, "stamp matches again: no reload");
    }

    /// A load that saw a swap while it ran is thrown away (never a
    /// half-swapped pack), and a later read loads again.
    #[test]
    fn the_map_reload_discards_a_load_that_saw_a_swap() {
        let data = tempfile::tempdir().unwrap();
        swap_in_v1(data.path(), &no_p(), "old");
        let live = LiveMaps::load(MapDirs::in_data_dir(data.path()));
        swap_in_v1(data.path(), &no_p(), "mid");
        let path = data.path().to_path_buf();
        live.during_next_load(move || swap_in_v1(&path, LETTERS, "new"));
        let first = live.current().unwrap();
        live.wait_reload();
        // nothing was installed; this read keeps the old pack and retries
        let kept = live.current().unwrap();
        assert!(
            std::sync::Arc::ptr_eq(&first, &kept),
            "the racing load was dropped"
        );
        live.wait_reload();
        let maps = live.current().unwrap();
        assert_eq!(
            sure_key(&maps, "NEPAL"),
            Some("nepal"),
            "the pack after the swap"
        );
        assert_eq!(live.reload_spawns(), 2);
    }

    #[test]
    fn a_map_load_that_sees_a_swap_is_thrown_away() {
        let data = tempfile::tempdir().unwrap();
        let dirs = MapDirs::in_data_dir(data.path());
        assert!(
            dirs.load_stable(|| swap_in_v1(data.path(), LETTERS, "a"))
                .is_none(),
            "a folder changed while loading"
        );
        let (maps, _) = dirs.load_stable(|| {}).expect("stable");
        assert_eq!(maps.unwrap().pack, MapPack::V1);
    }

    #[test]
    fn percentile_matches_numpy() {
        let mut v = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&mut v, 50.0), 2.5);
        assert!((percentile(&mut v, 30.0) - 1.9).abs() < 1e-6);
    }
}
