//! VICTORY / DEFEAT / DRAW from result frames (shadow only).
//!
//! The regions are ocr-v1's (`detect::match_end`), as fractions of the
//! centred 16:9 game rect, one [`Layout`] each, tagged with a source id:
//!
//! * `accolade`: post-match accolade screen, player cards; region x 0.5-25.5%, y 3.5-9.5%;
//!   plane `max`; real frames: 52 (1440p).
//! * `rank_screen`: competitive rank screen (incl. defeat); region x 1-25%, y 14.5-22.5%;
//!   plane `min`; real frames: 98 (1440p).
//! * `end_title`: centred italic end title; region x 32-68%, y 34-52%;
//!   plane `max`; real frames: none.
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
//!
//! Like ocr-v1, one sure read is not enough: [`ResultEvidence`] collects every
//! read of one match (poll frames, the accolade screen, the rank screen) and
//! the result is sure only with [`MIN_AGREE`] agreeing sure reads and no read
//! of another word. An accolade read is a second look at the same result,
//! never a new event: the reader creates no events, the caller adds each
//! frame of a match to that match's evidence.
//!
//! Templates are game art and are not embedded. They load from
//! `<data_dir>/templates/result/<source>_<word>.png` (8-bit grey at the work
//! scale, `word` one of `victory`, `defeat`, `draw`). A layout without
//! templates reads nothing. Further screens plug in through an optional
//! `layouts.json` in the same directory (see [`ResultTemplates::load_dir`]).
//! Only `accolade` and `rank_screen` have real templates so far; DRAW,
//! `end_title`, `tab_header` and 1080p-native accolade frames are untested.
//! The tracker's colour-flood banner (gold or red over y 30-70%) is not read
//! here: it depends on colour, and a word read covers the same screens.

use std::path::{Path, PathBuf};

use image::RgbImage;
use serde::{Deserialize, Serialize};

use super::banner::Gray;
use crate::ocr::preprocess::game_rect_16_9;

/// Identifies this recognizer's output. Bump on any change that can move a read.
pub const RESULT_RECOGNIZER_ID: &str = "result-v1";
/// Template directory under the data dir.
pub const TEMPLATE_SUBDIR: &str = "templates/result";
/// Below this, no result word is on the frame.
pub const PRESENT: f32 = 0.50;
/// Minimum score for a sure read.
pub const FLOOR: f32 = 0.70;
/// Minimum lead over the best other word for a sure read.
pub const MARGIN: f32 = 0.12;
/// Agreeing sure reads a match needs before its result is sure (ocr-v1 rule).
pub const MIN_AGREE: usize = 2;
/// Work scale relative to a 1440p game rect.
const WORK: f64 = 0.5;
const SCALES: [f64; 3] = [0.96, 1.0, 1.04];

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

    /// The four ocr-v1 regions.
    pub fn builtin() -> Vec<Layout> {
        vec![
            Self::new(ACCOLADE, [0.005, 0.035, 0.255, 0.095], Plane::Max),
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

/// Word templates, per layout and outcome.
#[derive(Debug, Clone)]
pub struct ResultTemplates {
    tpls: Vec<(Layout, Outcome, Gray)>,
}

fn region(rgb: &RgbImage, layout: &Layout) -> Option<Gray> {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = game_rect_16_9(w, h);
    let [fx0, fy0, fx1, fy1] = layout.roi;
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
            g.px[y * rw + x] = match layout.plane {
                Plane::Max => p[0].max(p[1]).max(p[2]),
                Plane::Min => p[0].min(p[1]).min(p[2]),
            } as f32
                / 255.0;
        }
    }
    let s = WORK * 1440.0 / gh as f64;
    let nw = ((rw as f64 * s).round_ties_even() as usize).max(1);
    let nh = ((rh as f64 * s).round_ties_even() as usize).max(1);
    Some(g.resize(nw, nh))
}

/// Best zero-mean NCC of `t` slid over `p`.
fn best_ncc(p: &Gray, t: &Gray) -> f64 {
    let mut out = -1.0f64;
    for sc in SCALES {
        let tw = ((t.w as f64 * sc).round_ties_even() as usize).max(1);
        let th = ((t.h as f64 * sc).round_ties_even() as usize).max(1);
        if tw > p.w || th > p.h {
            continue;
        }
        let tt = t.resize(tw, th);
        let n = (tw * th) as f64;
        let tm = tt.px.iter().map(|&v| v as f64).sum::<f64>() / n;
        let tz: Vec<f64> = tt.px.iter().map(|&v| v as f64 - tm).collect();
        let tn = tz.iter().map(|v| v * v).sum::<f64>().sqrt();
        if tn <= 0.0 {
            continue;
        }
        // integral images for window sums
        let (pw, ph) = (p.w, p.h);
        let mut s1 = vec![0.0f64; (pw + 1) * (ph + 1)];
        let mut s2 = vec![0.0f64; (pw + 1) * (ph + 1)];
        for y in 0..ph {
            for x in 0..pw {
                let v = p.at(x, y) as f64;
                let i = (y + 1) * (pw + 1) + x + 1;
                s1[i] = v + s1[i - 1] + s1[i - pw - 1] - s1[i - pw - 2];
                s2[i] = v * v + s2[i - 1] + s2[i - pw - 1] - s2[i - pw - 2];
            }
        }
        let rect = |s: &[f64], x: usize, y: usize| {
            let a = y * (pw + 1) + x;
            let b = (y + th) * (pw + 1) + x;
            s[b + tw] - s[b] - s[a + tw] + s[a]
        };
        for y in 0..=ph - th {
            for x in 0..=pw - tw {
                let sum = rect(&s1, x, y);
                let var = rect(&s2, x, y) - sum * sum / n;
                if var <= 1e-9 {
                    continue;
                }
                let mut num = 0.0;
                for ty in 0..th {
                    let row = &p.px[(y + ty) * pw + x..(y + ty) * pw + x + tw];
                    let trow = &tz[ty * tw..(ty + 1) * tw];
                    for (a, b) in row.iter().zip(trow) {
                        num += *a as f64 * b;
                    }
                }
                let v = num / (var.sqrt() * tn);
                if v > out {
                    out = v;
                }
            }
        }
    }
    out
}

impl ResultTemplates {
    pub fn dir_in(data_dir: &Path) -> PathBuf {
        data_dir.join(TEMPLATE_SUBDIR)
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
        (has(Outcome::Victory) && has(Outcome::Defeat)).then_some(Self { tpls })
    }

    /// Source ids that have at least one template.
    pub fn sources(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.tpls.iter().map(|t| t.0.id.as_str()).collect();
        v.dedup();
        v
    }

    /// Read the result word from one result frame (any layout).
    pub fn read(&self, rgb: &RgbImage) -> ResultRead {
        let mut best: [(f64, Option<&str>); 3] = [(-1.0, None); 3];
        let mut regions: Vec<(&str, Option<Gray>)> = Vec::new();
        for (layout, o, t) in &self.tpls {
            let idx = match regions.iter().position(|r| r.0 == layout.id) {
                Some(i) => i,
                None => {
                    regions.push((&layout.id, region(rgb, layout)));
                    regions.len() - 1
                }
            };
            let Some(p) = &regions[idx].1 else { continue };
            let s = best_ncc(p, t);
            let slot = &mut best[o.index()];
            if s > slot.0 {
                *slot = (s, Some(&layout.id));
            }
        }
        let mut order = Outcome::ALL;
        order.sort_by(|a, b| best[b.index()].0.total_cmp(&best[a.index()].0));
        let (s1, src) = best[order[0].index()];
        let s2 = best[order[1].index()].0;
        if s1 < PRESENT as f64 {
            return ResultRead::nothing(s1, s1 - s2);
        }
        ResultRead {
            outcome: Some(order[0]),
            source: src.map(str::to_string),
            score: s1 as f32,
            margin: (s1 - s2) as f32,
            suspect: !(s1 >= FLOOR as f64 && s1 - s2 >= MARGIN as f64),
        }
    }
}

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
mod tests {
    use super::*;
    use image::Rgb;

    /// Synthetic word: vertical bars in a word-specific rhythm. Not game art.
    fn word(seed: u32, w: usize, h: usize) -> Gray {
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

    fn lay(id: &str) -> Layout {
        Layout::by_id(id).unwrap()
    }

    fn templates() -> ResultTemplates {
        ResultTemplates::from_templates(vec![
            (lay(ACCOLADE), Outcome::Victory, word(1, 70, 30)),
            (lay(ACCOLADE), Outcome::Defeat, word(3, 60, 30)),
            (lay(RANK_SCREEN), Outcome::Victory, word(5, 66, 20)),
            (lay(RANK_SCREEN), Outcome::Defeat, word(7, 50, 20)),
        ])
        .unwrap()
    }

    /// Paint a work-scale word into a 2560x1440 frame inside a layout's region.
    fn paint(img: &mut RgbImage, layout: &Layout, t: &Gray, colour: [u8; 3]) {
        let [fx0, fy0, _, _] = layout.roi;
        let (x0, y0) = ((fx0 * 2560.0) as u32 + 20, (fy0 * 1440.0) as u32 + 10);
        for y in 0..t.h * 2 {
            for x in 0..t.w * 2 {
                if t.at(x / 2, y / 2) > 0.4 {
                    img.put_pixel(x0 + x as u32, y0 + y as u32, Rgb(colour));
                }
            }
        }
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
    fn regions_are_ocr_v1s() {
        let roi = |id: &str| lay(id).roi;
        assert_eq!(roi(ACCOLADE), [0.005, 0.035, 0.255, 0.095]);
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

    #[test]
    fn draw_template_reads_draw() {
        let mut v = vec![
            (lay(ACCOLADE), Outcome::Victory, word(1, 70, 30)),
            (lay(ACCOLADE), Outcome::Defeat, word(3, 60, 30)),
        ];
        v.push((lay(ACCOLADE), Outcome::Draw, word(6, 50, 30)));
        let t = ResultTemplates::from_templates(v).unwrap();
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
        paint(&mut img, &lay(ACCOLADE), &word(6, 50, 30), [230, 230, 230]);
        let r = t.read(&img);
        assert_eq!(r.outcome, Some(Outcome::Draw), "{r:?}");
        assert_eq!(Outcome::Draw.as_str(), "draw");
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
        let dir = std::env::temp_dir().join(format!("result-pack-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let save = |name: &str, g: &Gray| {
            let img = image::GrayImage::from_fn(g.w as u32, g.h as u32, |x, y| {
                image::Luma([(g.at(x as usize, y as usize) * 255.0) as u8])
            });
            img.save(dir.join(name)).unwrap();
        };
        save("cards_victory.png", &word(1, 70, 30));
        save("cards_defeat.png", &word(3, 60, 30));
        std::fs::write(
            dir.join("layouts.json"),
            r#"[{"id": "cards", "roi": [0.40, 0.40, 0.60, 0.50], "plane": "max"}]"#,
        )
        .unwrap();
        let t = ResultTemplates::load_dir(&dir).expect("plugged-in layout loads");
        std::fs::remove_dir_all(&dir).ok();
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
}
