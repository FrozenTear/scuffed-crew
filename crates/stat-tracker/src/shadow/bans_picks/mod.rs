//! cv-v1: the bans and picks reader. Reads the ban results screen (four
//! banned heroes, the team each ban belongs to, the lobby ban), the hero
//! select picks and the ban vote picks by matching hero icon templates on
//! gradient planes. A crop of the screen goes in; a value and a confidence
//! come out, and a value the screen does not show clearly is `None`, never a
//! guess.
//!
//! On by default (FrozenTear's call for alpha.2). Tesseract `ocr-v1` stays
//! the reader for everything it reads today; this module is a second reader
//! for screens ocr-v1 does not read at all. It runs unless the switch in
//! [`BansPicksSetting`] is `off`, and only when the templates are installed. The
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
/// Environment override for the switch: `off` turns the reader off.
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

/// Whether the cv-v1 reader runs. On unless set to `off`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BansPicksSetting {
    #[serde(rename = "off")]
    Off,
    #[default]
    #[serde(rename = "cv-v1")]
    CvV1,
}

impl BansPicksSetting {
    /// `off`, `0`, `false` or `no` (any case) is off; anything else is on.
    pub fn parse(s: &str) -> Self {
        let v = s.trim();
        if ["off", "0", "false", "no"]
            .iter()
            .any(|o| v.eq_ignore_ascii_case(o))
        {
            Self::Off
        } else {
            Self::CvV1
        }
    }

    /// The setting from [`ENV`]; on when it is unset.
    pub fn from_env() -> Self {
        std::env::var(ENV).map_or(Self::default(), |v| Self::parse(&v))
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
    /// `None` when the bars do not split into two colour groups, the own
    /// team ring colour is unknown, unusable or does not clearly pick one
    /// group, or this bar is grey.
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
    /// Same as [`Reader::read_ban_results_rings`] with at most one ring.
    /// `None` when the frame is not 2560x1440.
    pub fn read_ban_results(
        &self,
        img: &RgbImage,
        own_ring: Option<[u8; 3]>,
    ) -> Option<BanResults> {
        self.read_ban_results_rings(img, own_ring.as_slice())
    }

    /// The ban results screen with several own-team ring colours to try, best
    /// source first (see [`slot_teams`]). The hero select ring can be missing
    /// or faded, so the caller also gives the ring of the ban vote screen,
    /// which has it at the same place ([`self_ring_colour`]).
    /// `None` when the frame is not 2560x1440.
    pub fn read_ban_results_rings(&self, img: &RgbImage, rings: &[[u8; 3]]) -> Option<BanResults> {
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
        let teams = slot_teams(img, cy, rings);
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

/// A bar must be at least this saturated to have a colour (grey: no side).
const BAR_SAT_MIN: u8 = 80;
/// Bars of one team are within this many hue steps (of 180) of each other.
const SAME_TEAM_HUE: i32 = 20;
/// The two teams' bars are at least this many hue steps apart, or the
/// colours do not tell the teams apart (a filter, one colour on both).
const TEAMS_APART_HUE: i32 = 30;
/// A ring pixel is a team colour only when this saturated and this bright.
/// A grey or faded ring (a mid fade frame) has a hue that means nothing.
const RING_SAT_MIN: u8 = 80;
const RING_VAL_MIN: u8 = 75;
/// The ring must be this much nearer one team's hue than the other's.
const RING_LEAD_HUE: i32 = 20;
/// And no farther than this from that team's hue.
const RING_NEAR_HUE: i32 = 45;

/// The team of each ban slot. The bars are sorted into two colour groups by
/// how far apart they are, with no colour assumed: the game's team colours
/// are a player setting (blue and red, or yellow and purple, or any other
/// pair). Then the own-team ring colour says which group is own: the group
/// whose hue the ring is clearly nearer to. Each ring is tried in the order
/// given and a ring that cannot say (grey, faded, equally far from both
/// groups, or far from both) is skipped. Rings that do say must agree, or
/// every slot is `None`. A slot whose bar is grey is `None`. When the bars
/// do not form two groups every slot is `None`.
fn slot_teams(img: &RgbImage, cy: i32, rings: &[[u8; 3]]) -> [Option<Team>; 4] {
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
    let hues: [Option<u8>; 4] =
        std::array::from_fn(|i| (bars[i][1] >= BAR_SAT_MIN).then(|| bars[i][0]));
    teams_from_hues(hues, rings)
}

/// [`slot_teams`] from the four bar hues (`None` for a grey bar).
fn teams_from_hues(hues: [Option<u8>; 4], rings: &[[u8; 3]]) -> [Option<Team>; 4] {
    let none = [None; 4];
    // Two groups: the bars like the first coloured bar, and the rest.
    let Some(a) = hues.iter().flatten().next().copied() else {
        return none;
    };
    let in_a = |h: u8| hue_distance(h, a) <= SAME_TEAM_HUE;
    let Some(b) = hues.iter().flatten().copied().find(|&h| !in_a(h)) else {
        return none;
    };
    let in_b = |h: u8| hue_distance(h, b) <= SAME_TEAM_HUE;
    if hue_distance(a, b) < TEAMS_APART_HUE || hues.iter().flatten().any(|&h| !in_a(h) && !in_b(h))
    {
        return none;
    }
    // Each bar goes to the group it is nearer to; a bar equally near both
    // (`Some(None)`) or a grey bar (`None`) has no group.
    let group = hues.map(|h| {
        h.map(|h| {
            let (da, db) = (hue_distance(h, a), hue_distance(h, b));
            (da != db).then_some(da < db)
        })
    });
    // A team has two bars, so neither group may hold more than two. That
    // rules out three against one, whatever the colours.
    let size = |g: bool| group.iter().filter(|&&x| x == Some(Some(g))).count();
    if size(true) > 2 || size(false) > 2 {
        return none;
    }
    // Which group the rings call own (true: the first group).
    let mut verdicts = rings.iter().filter_map(|&ring| {
        let [rh, rs, rv] = hsv(ring);
        if rs < RING_SAT_MIN || rv < RING_VAL_MIN {
            return None;
        }
        let (da, db) = (hue_distance(rh, a), hue_distance(rh, b));
        if (da - db).abs() < RING_LEAD_HUE || da.min(db) > RING_NEAR_HUE {
            return None;
        }
        Some(da < db)
    });
    let Some(own_is_a) = verdicts.next() else {
        return none;
    };
    if verdicts.any(|v| v != own_is_a) {
        return none;
    }
    group.map(|g| {
        g.flatten().map(|in_a| {
            if in_a == own_is_a {
                Team::Own
            } else {
                Team::Enemy
            }
        })
    })
}
