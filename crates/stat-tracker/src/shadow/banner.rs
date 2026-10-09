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
//! The glyph atlas is game art and is not embedded. It loads at start from
//! `<data_dir>/templates/maps/` (`atlas.json` plus one 8-bit grey PNG per
//! glyph), the same policy as the hero icons. Without it the map is not read.
//! A map whose banner spelling needs a glyph the atlas lacks gets no template;
//! the floor keeps that from turning into a sure read of another map.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use image::imageops::{self, FilterType};
use image::{ImageBuffer, Luma, RgbImage};
use serde::Deserialize;

use crate::ocr::preprocess::game_rect_16_9;

/// Identifies this recognizer's output. Bump on any change that can move a read.
pub const MAP_RECOGNIZER_ID: &str = "map-v1";
/// Template directory under the data dir.
pub const TEMPLATE_SUBDIR: &str = "templates/maps";
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

/// One map: stable key (OverFast API), the name ocr-v1 stores, banner
/// spellings, and its single game mode (`None` when it has several).
#[derive(Debug)]
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
    /// Banner spellings that need a glyph the atlas lacks.
    pub unrenderable: Vec<&'static str>,
}

impl MapTemplates {
    pub fn dir_in(data_dir: &Path) -> PathBuf {
        data_dir.join(TEMPLATE_SUBDIR)
    }

    /// Load `<dir>/atlas.json` and its glyph PNGs. `None` when missing or bad.
    pub fn load_dir(dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(dir.join("atlas.json")).ok()?;
        let atlas: AtlasFile = serde_json::from_str(&text).ok()?;
        if atlas.height != H {
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
        Self { tpls, unrenderable }
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

    /// Score a located strip against every map.
    pub fn read_strip(&self, strip: &Gray) -> MapRead {
        let mut best = vec![f64::NEG_INFINITY; MAPS.len()];
        for (mi, _, t) in &self.tpls {
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
#[derive(Debug, Clone)]
pub struct MapRead {
    pub map: Option<&'static MapInfo>,
    pub best: Option<&'static MapInfo>,
    pub score: f32,
    pub margin: f32,
    pub suspect: bool,
    pub found_banner: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    /// Synthetic 5x7-ish block glyphs, one distinct bit pattern per letter,
    /// scaled to `H` rows. Not the game font: only the pipeline is under test.
    fn glyph(c: char) -> Gray {
        let w = 12;
        let mut g = Gray::new(w, H);
        let code = c as u32;
        // cap rows 7..25: the bar window adds room above and below, like
        // the real banner (the `|` reaches past the baseline)
        for y in 7..25 {
            for x in 1..w - 1 {
                let cell = ((y - 7) / 3) * 3 + (x - 1) / 4;
                let on = (code.wrapping_mul(2654435761) >> (cell % 29)) & 1 == 1 || x == 1;
                if on {
                    g.px[y * w + x] = 1.0;
                }
            }
        }
        g
    }

    fn atlas() -> HashMap<char, Gray> {
        "ABCDEFGHIJKLMNOPQRSTUVWXYZ':6"
            .chars()
            .map(|c| (c, glyph(c)))
            .collect()
    }

    /// A dark 1920x1080 frame with a banner: `|` then `text` drawn from the
    /// synthetic glyphs at 1080p scale, then a tall `TIME` block.
    fn frame_with_banner(text: &str) -> RgbImage {
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb([20, 24, 40]));
        let glyphs = atlas();
        let t = compose(&glyphs, 2.0, 9.0, text).unwrap();
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
        let t = MapTemplates::compose_all(&atlas(), 2.0, 9.0);
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
        let t = MapTemplates::compose_all(&glyphs, 2.0, 9.0);
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
        let t = MapTemplates::compose_all(&atlas(), 2.0, 9.0);
        let r = t.read(&RgbImage::from_pixel(1920, 1080, Rgb([20, 24, 40])));
        assert!(!r.found_banner && r.map.is_none() && r.suspect);
    }

    #[test]
    fn missing_pack_dir_loads_nothing() {
        assert!(MapTemplates::load_dir(Path::new("/nonexistent/maps")).is_none());
    }

    #[test]
    fn percentile_matches_numpy() {
        let mut v = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&mut v, 50.0), 2.5);
        assert!((percentile(&mut v, 30.0) - 1.9).abs() < 1e-6);
    }
}
