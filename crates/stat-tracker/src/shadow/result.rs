//! VICTORY / DEFEAT from a result frame (shadow only).
//!
//! Two end screens carry the word, both read here from one frame:
//!
//! * **Post-match accolade screen** (layout `A`, player cards): the word top
//!   left in the team colour, the map name and match time to its right.
//! * **Competitive rank screen** (layout `B`, also the defeat screen): white
//!   italic `DEFEAT` / `VICTORY!` under `COMPETITIVE`.
//! * Any further screen plugs in through `layouts.json` in the pack (see
//!   [`ResultTemplates::load_dir`]).
//!
//! Each layout has a fixed region in the 16:9 game rect. The region is turned
//! into one brightness plane (layout A: the brightest channel, so any bright
//! team colour reads alike; layout B: the darkest channel, so only white text
//! survives on the gold or blue background), scaled to a 720p work size, and
//! searched with zero-mean normalised correlation for each word template at
//! three scales. A result is sure when the best word scores at least
//! [`FLOOR`] and leads the other word by at least [`MARGIN`]. Below
//! [`PRESENT`] there is no result word on the frame.
//!
//! The word templates are game art and are not embedded: they load from
//! `<data_dir>/templates/result/{A,B}_{victory,defeat}.png` (8-bit grey at
//! the work scale). DRAW has no template yet (no real frame), so a draw reads
//! as flagged or nothing, never as a sure win or loss.

use std::path::{Path, PathBuf};

use image::RgbImage;
use serde::Deserialize;

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
/// Minimum lead over the other word for a sure read.
pub const MARGIN: f32 = 0.12;
/// Work scale relative to a 1440p game rect.
const WORK: f64 = 0.5;
const SCALES: [f64; 3] = [0.96, 1.0, 1.04];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Victory,
    Defeat,
}

impl Outcome {
    /// The value ocr-v1 stores (`PersonalMatch.outcome`).
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Victory => "victory",
            Outcome::Defeat => "defeat",
        }
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

/// One screen that shows the result word: a fixed region in the 16:9 game
/// rect (fractions x0, y0, x1, y1) and its plane. Templates for layout `id`
/// are `<id>_victory.png` and `<id>_defeat.png`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Layout {
    pub id: String,
    pub roi: [f64; 4],
    pub plane: Plane,
}

impl Layout {
    /// Post-match accolade screen (player cards): the word top left in the
    /// team colour. 52 real 1440p frames.
    pub fn accolade() -> Self {
        Self {
            id: ACCOLADE.into(),
            roi: [0.008, 0.022, 0.165, 0.098],
            plane: Plane::Max,
        }
    }

    /// Competitive rank screen: white italic word under COMPETITIVE. 98 real
    /// 1440p frames.
    pub fn rank_screen() -> Self {
        Self {
            id: RANK_SCREEN.into(),
            roi: [0.016, 0.150, 0.145, 0.235],
            plane: Plane::Min,
        }
    }
}

/// Layout ids of the built-in screens (template file prefixes).
pub const ACCOLADE: &str = "A";
pub const RANK_SCREEN: &str = "B";

/// One result read.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultRead {
    /// Best word when one is present (also when flagged).
    pub outcome: Option<Outcome>,
    /// Id of the layout the best word was found in.
    pub layout: Option<String>,
    pub score: f32,
    pub margin: f32,
    pub suspect: bool,
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
    let s = WORK * gh as f64 / 1440.0;
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

    /// Load `<id>_{victory,defeat}.png` for the built-in layouts plus any
    /// listed in an optional `<dir>/layouts.json` (a JSON array of
    /// `{"id", "roi": [x0, y0, x1, y1], "plane": "max"|"min"}`; an entry with a
    /// built-in id replaces it). New result screens plug in with a layout
    /// entry and two templates, no code change. `None` unless at least one
    /// victory and one defeat template load.
    pub fn load_dir(dir: &Path) -> Option<Self> {
        let mut layouts = vec![Layout::accolade(), Layout::rank_screen()];
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
            for (outcome, word) in [(Outcome::Victory, "victory"), (Outcome::Defeat, "defeat")] {
                let p = dir.join(format!("{}_{word}.png", layout.id));
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

    /// Read the result word from one result frame (accolade screen, rank
    /// screen, or any plugged-in layout).
    pub fn read(&self, rgb: &RgbImage) -> ResultRead {
        let mut best: [(f64, Option<&str>); 2] = [(-1.0, None), (-1.0, None)];
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
            let slot = &mut best[usize::from(*o == Outcome::Defeat)];
            if s > slot.0 {
                *slot = (s, Some(&layout.id));
            }
        }
        let (v, d) = (best[0], best[1]);
        let (win, s1, s2) = if v.0 >= d.0 {
            ((Outcome::Victory, v.1), v.0, d.0)
        } else {
            ((Outcome::Defeat, d.1), d.0, v.0)
        };
        if s1 < PRESENT as f64 {
            return ResultRead {
                outcome: None,
                layout: None,
                score: s1.max(0.0) as f32,
                margin: (s1 - s2).max(0.0) as f32,
                suspect: true,
            };
        }
        ResultRead {
            outcome: Some(win.0),
            layout: win.1.map(str::to_string),
            score: s1 as f32,
            margin: (s1 - s2) as f32,
            suspect: !(s1 >= FLOOR as f64 && s1 - s2 >= MARGIN as f64),
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

    fn templates() -> ResultTemplates {
        ResultTemplates::from_templates(vec![
            (Layout::accolade(), Outcome::Victory, word(1, 70, 30)),
            (Layout::accolade(), Outcome::Defeat, word(3, 60, 30)),
            (Layout::rank_screen(), Outcome::Victory, word(5, 66, 20)),
            (Layout::rank_screen(), Outcome::Defeat, word(7, 50, 20)),
        ])
        .unwrap()
    }

    /// Paint a work-scale word into a 2560x1440 frame at a layout's region.
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

    #[test]
    fn accolade_word_reads_in_any_team_colour() {
        let t = templates();
        for colour in [[255, 220, 0], [150, 20, 160], [40, 200, 255]] {
            let mut img = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
            paint(&mut img, &Layout::accolade(), &word(3, 60, 30), colour);
            let r = t.read(&img);
            assert_eq!(r.outcome, Some(Outcome::Defeat), "{colour:?}");
            assert!(!r.suspect, "{colour:?}: {r:?}");
        }
    }

    #[test]
    fn rank_screen_defeat_reads() {
        let t = templates();
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([150, 100, 30]));
        paint(
            &mut img,
            &Layout::rank_screen(),
            &word(7, 50, 20),
            [245, 245, 245],
        );
        let r = t.read(&img);
        assert_eq!(r.outcome, Some(Outcome::Defeat));
        assert_eq!(r.layout.as_deref(), Some(RANK_SCREEN));
        assert!(!r.suspect);
    }

    #[test]
    fn blank_frame_has_no_result() {
        let r = templates().read(&RgbImage::from_pixel(1920, 1080, Rgb([20, 26, 50])));
        assert_eq!(r.outcome, None);
        assert!(r.suspect);
    }

    #[test]
    fn needs_both_words_to_load() {
        assert!(
            ResultTemplates::from_templates(vec![(
                Layout::accolade(),
                Outcome::Victory,
                word(1, 20, 10)
            )])
            .is_none()
        );
        assert!(ResultTemplates::load_dir(Path::new("/nonexistent/result")).is_none());
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
        save("C_victory.png", &word(1, 70, 30));
        save("C_defeat.png", &word(3, 60, 30));
        std::fs::write(
            dir.join("layouts.json"),
            r#"[{"id": "C", "roi": [0.40, 0.40, 0.60, 0.50], "plane": "max"}]"#,
        )
        .unwrap();
        let t = ResultTemplates::load_dir(&dir).expect("plugged-in layout loads");
        let mut img = RgbImage::from_pixel(2560, 1440, Rgb([20, 26, 50]));
        let c = Layout {
            id: "C".into(),
            roi: [0.40, 0.40, 0.60, 0.50],
            plane: Plane::Max,
        };
        paint(&mut img, &c, &word(1, 70, 30), [200, 40, 40]);
        let r = t.read(&img);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(r.outcome, Some(Outcome::Victory));
        assert_eq!(r.layout.as_deref(), Some("C"));
        assert!(!r.suspect, "{r:?}");
    }
}
