//! cv-v1: the bans and picks reader. Reads the ban results screen (four
//! banned heroes, the team each ban belongs to, the lobby ban), the hero
//! select picks and the ban vote picks by matching hero icon templates on
//! gradient planes. A crop of the screen goes in; a value and a confidence
//! come out, and a value the screen does not show clearly is `None`, never a
//! guess.
//!
//! Off by default. Tesseract `ocr-v1` stays the default reader for
//! everything it reads today; this module is a second reader for screens
//! ocr-v1 does not read at all, and nothing here runs unless the switch in
//! [`BansPicksSetting`] is `cv-v1` and the templates are installed. The
//! capture wiring (which frames to keep) and the server field are not part
//! of this module; see "What a caller must do" below.
//!
//! # What was measured
//!
//! The method and thresholds come from a prototype run over 33 saved
//! pregame games at 2560x1440. It reads icons with a masked correlation of
//! blurred Sobel gradients (see [`matcher`]) and accepts a read only when
//! the best score is at least [`MIN_SCORE`] and leads the second best by at
//! least [`MIN_MARGIN`].
//!
//! # What a caller must do
//!
//! * Pass the **last** frame of the hero select and of the ban vote, never
//!   the first. The first frame is mid transition (a wipe band, a fade) and
//!   gave false accepts on rows that are not readable. As a second guard a
//!   row under the wipe band reads as `None` anyway ([`BAND_MAX`]).
//! * Pass 2560x1440 frames. Other sizes are refused (`None`), because the
//!   positions, scales and thresholds were measured at that size only.
//! * Install the templates ([`templates`]); without them [`Reader::load`]
//!   fails and the reader is off.
//!
//! # Output
//!
//! [`BansPicks`] carries `bans`, `lobby_ban`, `picks` and
//! `bans_picks_recognizer: "cv-v1"`. A value is a hero pack key, the string
//! `no_preference` when the screen shows the "no preferred hero" icon, or
//! null when it cannot be read.

mod imaging;
mod matcher;
mod templates;

#[cfg(test)]
mod tests;

use std::path::Path;

use image::RgbImage;
use serde::{Deserialize, Serialize};

use imaging::{Gray, Plane, gradient, gray_from_rgb, halve, hsv};
use matcher::{SearchPlan, Window, best_scores};
pub use templates::{
    NO_PREFERENCE_KEY, TEMPLATE_DIR, TEMPLATE_H, TEMPLATE_W, TemplateError, TemplateSet, Use,
    hero_keys,
};

/// The recognizer id written next to the values.
pub const RECOGNIZER_ID: &str = "cv-v1";
/// The value of a row that shows the "no preferred hero" icon.
pub const NO_PREFERENCE: &str = "no_preference";
/// Environment override for the switch: `cv-v1` turns the reader on.
pub const ENV: &str = "STAT_TRACKER_BANS_PICKS_RECOGNIZER";

/// Lowest best score a read may have.
pub const MIN_SCORE: f32 = 0.45;
/// Least lead over the second best a read must have.
pub const MIN_MARGIN: f32 = 0.12;
/// A row with at least this share of wipe-band cyan reads as `None`.
pub const BAND_MAX: f32 = 0.4;
/// The only frame size the reader accepts.
pub const FRAME_W: u32 = 2560;
pub const FRAME_H: u32 = 1440;

/// Whether the cv-v1 reader runs. Off unless set to `cv-v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BansPicksSetting {
    #[default]
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "cv-v1")]
    CvV1,
}

impl BansPicksSetting {
    /// `cv-v1` (any case) is on; `off`, empty and anything else is off.
    pub fn parse(s: &str) -> Self {
        if s.trim().eq_ignore_ascii_case(RECOGNIZER_ID) {
            Self::CvV1
        } else {
            Self::Off
        }
    }

    /// The setting from [`ENV`]; off when it is unset.
    pub fn from_env() -> Self {
        std::env::var(ENV).map_or(Self::Off, |v| Self::parse(&v))
    }

    pub fn is_on(self) -> bool {
        self == Self::CvV1
    }
}

/// What a screen position shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// A hero, by pack key (`wrecking-ball`).
    Hero(String),
    /// The "no preferred hero" icon.
    NoPreference,
}

impl Choice {
    pub fn as_str(&self) -> &str {
        match self {
            Choice::Hero(k) => k,
            Choice::NoPreference => NO_PREFERENCE,
        }
    }
}

impl Serialize for Choice {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Choice {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(if s == NO_PREFERENCE {
            Choice::NoPreference
        } else {
            Choice::Hero(s)
        })
    }
}

/// One value with how sure the reader is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Read {
    /// `None` when the position cannot be read.
    pub value: Option<Choice>,
    /// 0..1; 0 when `value` is `None`.
    pub confidence: f32,
    /// Best correlation score (diagnostic, also set when `value` is `None`).
    pub score: f32,
    /// Lead of the best over the second best (diagnostic).
    pub margin: f32,
}

impl Read {
    fn unreadable() -> Self {
        Self {
            value: None,
            confidence: 0.0,
            score: 0.0,
            margin: 0.0,
        }
    }
}

/// Which side a ban belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Team {
    Own,
    Enemy,
}

/// One banned hero and the team that banned it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BanRead {
    #[serde(flatten)]
    pub read: Read,
    /// `None` when the bar colour is not clearly one side or the own team
    /// colour is not known.
    pub team: Option<Team>,
}

/// Ban results screen layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// Four bans, no lobby ban.
    Plain,
    /// Four bans lowered, with a lobby ban icon above.
    Lobby,
}

/// The ban results screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BanResults {
    pub layout: Layout,
    /// Four slots, left to right.
    pub bans: Vec<BanRead>,
    /// `Some` in the lobby layout (its value is `None` when unreadable),
    /// `None` in the plain layout.
    pub lobby_ban: Option<Read>,
}

/// Everything cv-v1 reads before a game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BansPicks {
    pub bans: Vec<BanRead>,
    pub lobby_ban: Option<Read>,
    /// The six own-team rows of the hero select, top (the local player) first.
    pub picks: Vec<Read>,
    pub bans_picks_recognizer: String,
}

/// The hero select, with the own-team ring colour the ban results need.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroSelect {
    pub picks: Vec<Read>,
    /// RGB of the own-team ring on the local player's row.
    pub ring: [u8; 3],
}

// ---- geometry, at 2560x1440 ----

const SLOT_X: [i32; 4] = [645, 1068, 1493, 1918];
/// Ban slot centre y: plain layout, lobby layout.
const SLOT_CY: [i32; 2] = [645, 850];
const LOBBY_XY: (i32, i32) = (1160, 452);
const ROW_X: i32 = 125;
const HS_Y: [i32; 6] = [452, 622, 750, 878, 1006, 1134];
const HB_Y: [i32; 6] = [452, 624, 754, 882, 1012, 1142];
const HALF_SLOT: i32 = 115;
const HALF_PROBE: i32 = 135;
const HALF_LOBBY: i32 = 100;
const HALF_ROW: i32 = 100;
/// Margin, in half-size pixels, kept around a computed region so blur and
/// Sobel see the same neighbours they would on the whole frame.
const EDGE: i32 = 4;

/// A gradient plane of part of the half-size frame.
struct Field {
    plane: Plane,
    /// Half-size coordinates of `plane`'s top-left pixel.
    ox: i32,
    oy: i32,
}

impl Field {
    /// The half-size gradient of the native rectangle `x0..x1`, `y0..y1`.
    fn of(img: &RgbImage, x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        let hx0 = (x0 / 2 - EDGE).max(0);
        let hy0 = (y0 / 2 - EDGE).max(0);
        let hx1 = (x1 / 2 + EDGE).min(FRAME_W as i32 / 2);
        let hy1 = (y1 / 2 + EDGE).min(FRAME_H as i32 / 2);
        let (w, h) = ((hx1 - hx0) as usize * 2, (hy1 - hy0) as usize * 2);
        let rect =
            image::imageops::crop_imm(img, hx0 as u32 * 2, hy0 as u32 * 2, w as u32, h as u32)
                .to_image();
        let grey: Gray = halve(&gray_from_rgb(&rect));
        Field {
            plane: gradient(&grey),
            ox: hx0,
            oy: hy0,
        }
    }

    /// Window for a native centre and half size, clipped like the prototype:
    /// `x0 = max(int((cx - half) / 2), 0)`, `x1 = int((cx + half) / 2)`.
    fn window(&self, cx: i32, cy: i32, half: i32) -> Window {
        let x0 = (((cx - half) / 2).max(0) - self.ox).max(0) as usize;
        let y0 = (((cy - half) / 2).max(0) - self.oy).max(0) as usize;
        let x1 = ((cx + half) / 2 - self.ox).max(0) as usize;
        let y1 = ((cy + half) / 2 - self.oy).max(0) as usize;
        Window::cut(&self.plane, x0, y0, x1, y1)
    }
}

/// Best class per template, sorted: `(score, class index)` descending.
fn ranked(win: &Window, classes: &[&matcher::Prepared], plan: SearchPlan) -> Vec<(f32, usize)> {
    let mut out: Vec<(f32, usize)> = best_scores(win, classes, plan)
        .into_iter()
        .enumerate()
        .filter_map(|(i, s)| s.map(|s| (s, i)))
        .collect();
    out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// The reader.
pub struct Reader {
    templates: TemplateSet,
    plan: SearchPlan,
}

impl Reader {
    pub fn new(templates: TemplateSet) -> Self {
        Self {
            templates,
            plan: SearchPlan::FAST,
        }
    }

    /// Load the templates from `<data_dir>/templates/bans-picks-v1/`.
    pub fn load(data_dir: &Path) -> Result<Self, TemplateError> {
        TemplateSet::load(data_dir).map(Self::new)
    }

    /// Score every offset of every template instead of the coarse to fine
    /// search. For measuring the search, not for running.
    pub fn exact(mut self) -> Self {
        self.plan = SearchPlan::EXACT;
        self
    }

    fn read_of(&self, ranked: &[(f32, usize)]) -> Read {
        let Some(&(score, class)) = ranked.first() else {
            return Read::unreadable();
        };
        let second = ranked.get(1).map_or(0.0, |r| r.0);
        let margin = score - second;
        let confidence =
            (0.5 * (score / 0.8).min(1.0) + 0.5 * (margin / 0.3).min(1.0)).clamp(0.0, 1.0);
        let r3 = |v: f32| (v * 1000.0).round() / 1000.0;
        let accepted = score >= MIN_SCORE && margin >= MIN_MARGIN;
        let key = self.templates.key(class);
        let value = accepted.then(|| {
            if key == NO_PREFERENCE_KEY {
                Choice::NoPreference
            } else {
                Choice::Hero(key.to_string())
            }
        });
        Read {
            confidence: if accepted { r3(confidence) } else { 0.0 },
            value,
            score: r3(score),
            margin: r3(margin),
        }
    }

    fn read_window(&self, field: &Field, cx: i32, cy: i32, half: i32, u: Use, np: bool) -> Read {
        let win = field.window(cx, cy, half);
        let classes = self.templates.classes(u, np);
        self.read_of(&ranked(&win, &classes, self.plan))
    }

    /// The ban results screen. `own_ring` is the own-team ring colour from
    /// the hero select ([`HeroSelect::ring`]); without it every `team` is `None`.
    /// `None` when the frame is not 2560x1440.
    pub fn read_ban_results(
        &self,
        img: &RgbImage,
        own_ring: Option<[u8; 3]>,
    ) -> Option<BanResults> {
        if img.dimensions() != (FRAME_W, FRAME_H) {
            return None;
        }
        let field = Field::of(img, 510, 352, 2053, 985);
        // Which layout: the one whose two middle slots match a hero best.
        let probe: Vec<f32> = SLOT_CY
            .iter()
            .map(|&cy| {
                SLOT_X[1..3]
                    .iter()
                    .map(|&x| {
                        let win = field.window(x, cy, HALF_PROBE);
                        let classes = self.templates.classes(Use::Slot, false);
                        ranked(&win, &classes, SearchPlan::PROBE)
                            .first()
                            .map_or(0.0, |r| r.0)
                    })
                    .sum()
            })
            .collect();
        let lobby = probe[1] > probe[0];
        let cy = SLOT_CY[lobby as usize];
        let teams = slot_teams(img, cy, own_ring);
        let bans = SLOT_X
            .iter()
            .zip(teams)
            .map(|(&x, team)| BanRead {
                read: self.read_window(&field, x, cy, HALF_SLOT, Use::Slot, false),
                team,
            })
            .collect();
        let lobby_ban = lobby.then(|| {
            self.read_window(
                &field,
                LOBBY_XY.0,
                LOBBY_XY.1,
                HALF_LOBBY,
                Use::Lobby,
                false,
            )
        });
        Some(BanResults {
            layout: if lobby { Layout::Lobby } else { Layout::Plain },
            bans,
            lobby_ban,
        })
    }

    fn read_rows(&self, img: &RgbImage, ys: &[i32; 6]) -> Option<Vec<Read>> {
        if img.dimensions() != (FRAME_W, FRAME_H) {
            return None;
        }
        let field = Field::of(img, 25, ys[0] - HALF_ROW, 225, ys[5] + HALF_ROW);
        let np = self.templates.has_no_preference();
        Some(
            ys.iter()
                .map(|&y| {
                    if band_share(img, y) >= BAND_MAX {
                        Read::unreadable()
                    } else {
                        self.read_window(&field, ROW_X, y, HALF_ROW, Use::Row, np)
                    }
                })
                .collect(),
        )
    }

    /// The hero select: the six own-team picks and the own-team ring colour.
    /// Pass the last frame of the screen. `None` when the frame is not 2560x1440.
    pub fn read_hero_select(&self, img: &RgbImage) -> Option<HeroSelect> {
        Some(HeroSelect {
            picks: self.read_rows(img, &HS_Y)?,
            ring: self_ring_colour(img),
        })
    }

    /// The six own-team rows of the ban vote screen. Pass the last frame.
    pub fn read_ban_vote(&self, img: &RgbImage) -> Option<Vec<Read>> {
        self.read_rows(img, &HB_Y)
    }

    /// Ban results and hero select picks together, from the ban results
    /// frame and the last hero select frame. `None` when either frame is
    /// not 2560x1440.
    pub fn read(&self, ban_results: &RgbImage, hero_select_last: &RgbImage) -> Option<BansPicks> {
        let hs = self.read_hero_select(hero_select_last)?;
        let br = self.read_ban_results(ban_results, Some(hs.ring))?;
        Some(BansPicks {
            bans: br.bans,
            lobby_ban: br.lobby_ban,
            picks: hs.picks,
            bans_picks_recognizer: RECOGNIZER_ID.to_string(),
        })
    }
}

/// Share of the row's icon area (x 77..173, y +-48) that is wipe-band cyan
/// (HSV H 85..105, S >= 120, V >= 190).
pub fn band_share(img: &RgbImage, y: i32) -> f32 {
    let (x0, x1) = (77u32, 173u32);
    let (y0, y1) = ((y - 48).max(0) as u32, ((y + 48) as u32).min(img.height()));
    let mut n = 0u32;
    let mut cyan = 0u32;
    for py in y0..y1 {
        for px in x0..x1 {
            let [h, s, v] = hsv(img.get_pixel(px, py).0);
            n += 1;
            if (85..=105).contains(&h) && s >= 120 && v >= 190 {
                cyan += 1;
            }
        }
    }
    if n == 0 { 0.0 } else { cyan as f32 / n as f32 }
}

/// The own-team ring colour: the most saturated, brightest pixel of the
/// local player's row frame edge (x 64..70, y 430..475).
pub fn self_ring_colour(img: &RgbImage) -> [u8; 3] {
    let mut best = ([0u8; 3], -1i32);
    for y in 430..475.min(img.height()) {
        for x in 64..70.min(img.width()) {
            let p = img.get_pixel(x, y).0;
            let [_, s, v] = hsv(p);
            let score = s as i32 * v as i32;
            if score > best.1 {
                best = (p, score);
            }
        }
    }
    best.0
}

fn hue_distance(a: u8, b: u8) -> i32 {
    let d = (a as i32 - b as i32).abs();
    d.min(180 - d)
}

/// The team of each ban slot from its ban bar colour against the own-team
/// ring colour. A bar is own when its hue is within 8 of the closest bar and
/// under 25 from the ring, enemy when 40 or more from the ring, otherwise
/// (or when it is grey) unknown.
fn slot_teams(img: &RgbImage, cy: i32, ring: Option<[u8; 3]>) -> [Option<Team>; 4] {
    let Some(ring) = ring else {
        return [None; 4];
    };
    let ring_h = hsv(ring)[0];
    let by = cy + 257;
    let bars: Vec<[u8; 2]> = SLOT_X
        .iter()
        .map(|&x| {
            let mut sum = [0u64; 3];
            let mut n = 0u64;
            for py in (by - 3)..(by + 4) {
                for px in (x - 60)..(x - 20) {
                    let p = img.get_pixel(px as u32, py as u32).0;
                    for c in 0..3 {
                        sum[c] += p[c] as u64;
                    }
                    n += 1;
                }
            }
            let mean = [0, 1, 2].map(|c| (sum[c] / n) as u8);
            let [h, s, _] = hsv(mean);
            [h, s]
        })
        .collect();
    let d: Vec<i32> = bars.iter().map(|b| hue_distance(b[0], ring_h)).collect();
    let nearest = *d.iter().min().unwrap_or(&0);
    let mut out = [None; 4];
    for i in 0..4 {
        if bars[i][1] < 80 {
            continue;
        }
        out[i] = if d[i] <= nearest + 8 && d[i] < 25 {
            Some(Team::Own)
        } else if d[i] >= 40 {
            Some(Team::Enemy)
        } else {
            None
        };
    }
    out
}
