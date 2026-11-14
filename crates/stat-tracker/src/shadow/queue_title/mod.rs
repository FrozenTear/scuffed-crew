//! Queue title reader: reads the queue word mark ("OPEN QUEUE",
//! "ROLE QUEUE") drawn top left on the pregame screens (map vote, hero
//! select, hero ban), so a game record can carry the queue that was played.
//!
//! Off by default. It runs only when [`ENV`] is switched on
//! ([`QueueTitleSetting::parse`]) and a template pack is installed under
//! `<data_dir>/templates/queue-title-v1/` ([`templates`]). The pack is cut
//! from the game's own screens, so it is never committed: it is built
//! locally with [`Learner`] from labelled frames (the ignored bench in
//! `tests.rs` writes one).
//!
//! # How it reads
//!
//! The whole title is matched as one picture, no OCR:
//!
//! 1. Crop a fixed region around the title at the 2560x1440 reference
//!    ([`REF_X`], [`REF_Y_MAPVOTE`], [`REF_Y_SELECT`]), placed in the
//!    frame's 16:9 game rect and scaled by its height / 1440. The
//!    thresholds were measured at 1440p and at a synthetic 1080p only.
//!    Ultrawide and 16:10 frames are placed by the shared game rect
//!    (`frame::geometry::game_rect_16_9`), checked on synthetic frames only:
//!    there is no real ultrawide or 16:10 capture in the tests. Map vote
//!    draws the title 27 px higher than hero select and hero ban.
//! 2. Turn it into an ink plane: the smallest of R, G, B (the title is grey
//!    to white, the backdrop is dark red), with strongly saturated pixels
//!    set to zero. That drops the pink voice chat indicator that overlaps
//!    the lower left of the title on every frame.
//! 3. Shrink to [`TW`] x [`TH`] plus a [`PAD`] margin and scale to the
//!    99th percentile, so fade-dimmed frames look like bright ones.
//! 4. Compare with every class template by normalised cross correlation
//!    over all shifts within the margin. Each class gets two scores at its
//!    best shift: the whole title, and the worst of [`STRIPS`] vertical
//!    strips. The strip score is what turns unknown titles away: two titles
//!    that share "QUEUE" still score well as a whole, but the strips over
//!    the first word do not.
//! 5. Accept the best class only when its whole score is at least
//!    [`MIN_SCORE`], its strip score at least [`MIN_STRIP`], and it leads
//!    the runner-up by [`MIN_MARGIN`]. Otherwise the title is `None`, never
//!    a guess. Competitive pre-game titles read the bare queue name
//!    (`OPEN QUEUE`, `ROLE QUEUE`), which this pack does know. Titles the
//!    pack does not have (arcade, or any other) are expected to come back
//!    `None`, but that is extrapolated from the held-out runs (each known
//!    title left out of the pack in turn), not measured on real arcade
//!    frames: the bench has none.
//!
//! # Thresholds
//!
//! Picked from the private leave-one-session-out bench (63 frames, 15
//! sessions, two titles), at 1440p and at a synthetic Lanczos3 1080p:
//!
//! * correct class, whole score: lowest 0.964 (all three set ups)
//! * correct class, strip score: lowest 0.894
//! * wrong class, and every frame of a title left out of the pack: whole at
//!   most 0.629, strip at most 0.512
//! * lead of the correct class over the other: at least 0.342
//!
//! [`MIN_SCORE`] 0.80 and [`MIN_STRIP`] 0.70 sit near the middle of those
//! gaps (about 0.16 to 0.19 clear on each side), so a dimmer or noisier
//! frame than the set holds still reads, and a title the pack does not know
//! still fails. The strip check carries most of the unknown title
//! rejection: with a one class pack there is no runner-up. [`MIN_MARGIN`]
//! 0.15 is well below the observed lead and only matters when two known
//! titles look alike (say a future "5V5 ROLE QUEUE" next to "ROLE QUEUE").
//!
//! # What a caller must do
//!
//! * Pass a whole frame with its [`Screen`]. Only its 16:9 game rect is
//!   read; a non-16:9 area (ultrawide side bars, 16:10 bands) never is.
//! * Expect `None` for a game rect under 359 px tall (at 359 px the region
//!   crops to exactly 124 x 39, at 358 px to less): below that the region
//!   is smaller than the [`TW`] x [`TH`] template plus its [`PAD`] margin, and
//!   the reader never scales a region up ([`plane`]). Before the game rect
//!   placement such a frame was scaled up and read.
//! * For one value per game, use [`pick`] (hero ban first, then hero
//!   select last) or [`Reader::read_dump_dir`] on a pregame dump folder.
//!
//! The live capture path reads it through `daemon::queue_title` in the
//! binary, which puts the result on the game record
//! (`PersonalMatch::queue_title`) and the upload.

pub mod templates;

#[cfg(test)]
mod tests;

use std::path::Path;

use image::RgbImage;
use serde::{Deserialize, Serialize};

use super::ncc::Gray;
use crate::frame::geometry::game_rect_16_9;
pub use templates::{Learner, PACK_FILE, TEMPLATE_DIR, TemplateError, TemplateSet};

/// The recognizer id written next to the value (`rank-v1`). Defined in
/// `model` so the upload sends the same id without using the readers.
pub use crate::model::QUEUE_TITLE_RECOGNIZER_ID as RECOGNIZER_ID;
/// Environment switch: `on`, `1`, `true`, `yes` or [`RECOGNIZER_ID`] turns
/// it on. Anything else, or unset, is off.
pub const ENV: &str = "STAT_TRACKER_QUEUE_TITLE_RECOGNIZER";
/// Lowest whole title score a read may have (0.80). Defined in `model` so
/// the upload checks the same bar.
pub use crate::model::QUEUE_TITLE_MIN_SCORE as MIN_SCORE;
/// Lowest score of the worst strip.
pub const MIN_STRIP: f32 = 0.70;
/// Least whole score lead over the runner-up class.
pub const MIN_MARGIN: f32 = 0.15;
/// Vertical strips the strip score is taken over.
pub const STRIPS: usize = 4;

/// Reference frame height the layout was measured at.
pub const FRAME_H: u32 = 1440;
/// Left edge of the title region at the reference.
pub const REF_X: u32 = 40;
/// Top of the title region on the map vote screen.
pub const REF_Y_MAPVOTE: u32 = 60;
/// Top of the title region on hero select and hero ban.
pub const REF_Y_SELECT: u32 = 87;
/// Width of the title region at the reference.
pub const REF_W: u32 = 480;
/// Height of the title region at the reference.
pub const REF_H: u32 = 140;
/// Template width: the region shrunk 4x.
pub const TW: usize = 120;
/// Template height: the region shrunk 4x.
pub const TH: usize = 35;
/// Shift margin in template pixels on each side (8 px at 1440p).
pub const PAD: usize = 2;
/// Reference pixels per template pixel.
const STEP: u32 = REF_W / TW as u32;
/// A pixel with saturation (max - min) / max above this is overlay, not
/// title ink.
const SAT_MAX: f32 = 0.35;
/// Below this 99th percentile the region holds no text worth scaling up.
const MIN_PEAK: f32 = 0.08;

/// Whether the reader runs. Off unless switched on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum QueueTitleSetting {
    #[default]
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "on")]
    On,
}

impl QueueTitleSetting {
    /// `on`, `1`, `true`, `yes` or [`RECOGNIZER_ID`] (any case) is on;
    /// anything else is off.
    pub fn parse(s: &str) -> Self {
        let v = s.trim();
        if ["on", "1", "true", "yes", RECOGNIZER_ID]
            .iter()
            .any(|o| v.eq_ignore_ascii_case(o))
        {
            Self::On
        } else {
            Self::Off
        }
    }

    /// The setting from [`ENV`]; off when it is unset.
    pub fn from_env() -> Self {
        std::env::var(ENV).map_or(Self::default(), |v| Self::parse(&v))
    }

    pub fn is_on(self) -> bool {
        self == Self::On
    }
}

/// The pregame screen a frame comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Screen {
    #[serde(rename = "mapvote")]
    MapVote,
    #[serde(rename = "heroselect")]
    HeroSelect,
    #[serde(rename = "heroban")]
    HeroBan,
}

impl Screen {
    /// The pregame dump name (`mapvote`, `heroselect`, `heroban`).
    pub fn as_str(self) -> &'static str {
        match self {
            Screen::MapVote => "mapvote",
            Screen::HeroSelect => "heroselect",
            Screen::HeroBan => "heroban",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Screen::MapVote, Screen::HeroSelect, Screen::HeroBan]
            .into_iter()
            .find(|x| x.as_str() == s)
    }

    /// Top of the title region at the reference.
    pub fn ref_y(self) -> u32 {
        match self {
            Screen::MapVote => REF_Y_MAPVOTE,
            Screen::HeroSelect | Screen::HeroBan => REF_Y_SELECT,
        }
    }
}

/// The value read, how sure, and where.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field<T> {
    /// `None` when no known title is shown clearly.
    pub value: Option<T>,
    /// 0..1; 0 when `value` is `None`.
    pub confidence: f32,
    /// Frame box `[x0, y0, x1, y1)` of the matched title window.
    pub bbox: Option<[u32; 4]>,
}

impl<T> Field<T> {
    pub fn none() -> Self {
        Self {
            value: None,
            confidence: 0.0,
            bbox: None,
        }
    }
}

/// The queue title of one frame or one game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueTitleRead {
    /// The title as the pack spells it (`OPEN QUEUE`).
    pub queue_title: Field<String>,
    pub queue_title_recognizer: String,
}

impl QueueTitleRead {
    pub fn new(queue_title: Field<String>) -> Self {
        Self {
            queue_title,
            queue_title_recognizer: RECOGNIZER_ID.to_string(),
        }
    }
}

/// Scores of one class at its best shift (diagnostic and bench).
#[derive(Debug, Clone, PartialEq)]
pub struct ClassScore {
    pub class: String,
    /// Whole title correlation.
    pub score: f32,
    /// Worst strip correlation at the same shift.
    pub strip: f32,
    /// Shift in template pixels, 0..=2*PAD.
    pub dx: usize,
    pub dy: usize,
}

/// The title region of `img` as a padded ink plane of
/// `(TW + 2*PAD) x (TH + 2*PAD)`, and the frame rectangle it was cut from
/// (`[x, y, w, h]`). The region is placed in the frame's 16:9 game rect
/// ([`game_rect_16_9`]), so an ultrawide or 16:10 frame is read from its
/// playfield and an exact 16:9 frame is read as before. `None` for a game
/// rect so small that the region would have to be scaled up to the
/// template size: under 359 px tall, where the crop is 124 x 39.
pub fn plane(img: &RgbImage, screen: Screen) -> Option<(Gray, [u32; 4])> {
    let (gx, gy, gw, gh) = game_rect_16_9(img.width(), img.height());
    let s = gh as f32 / FRAME_H as f32;
    let pad = PAD as u32 * STEP;
    let rx = REF_X - pad;
    let ry = screen.ref_y() - pad;
    let rw = REF_W + 2 * pad;
    let rh = REF_H + 2 * pad;
    let x0 = gx + (rx as f32 * s).round() as u32;
    let y0 = gy + (ry as f32 * s).round() as u32;
    let x1 = gx + ((rx + rw) as f32 * s).round() as u32;
    let y1 = gy + ((ry + rh) as f32 * s).round() as u32;
    // Defensive: the reference region lies inside the 2560x1440 rect, so
    // the scaled one lies inside the game rect, and the size check below is
    // what turns a small rect away. Kept so no subtraction can underflow.
    if x1 > gx + gw || y1 > gy + gh || x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (cw, ch) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if cw < TW + 2 * PAD || ch < TH + 2 * PAD {
        return None;
    }
    let mut g = Gray::new(cw, ch);
    for y in 0..ch {
        for x in 0..cw {
            let p = img.get_pixel(x0 + x as u32, y0 + y as u32).0;
            let mx = p[0].max(p[1]).max(p[2]) as f32;
            let mn = p[0].min(p[1]).min(p[2]) as f32;
            let sat = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
            g.px[y * cw + x] = if sat > SAT_MAX { 0.0 } else { mn / 255.0 };
        }
    }
    let mut g = g.resize(TW + 2 * PAD, TH + 2 * PAD);
    let mut sorted = g.px.clone();
    sorted.sort_by(f32::total_cmp);
    let peak = sorted[(sorted.len() - 1) * 99 / 100];
    if peak >= MIN_PEAK {
        for v in &mut g.px {
            *v = (*v / peak).min(1.0);
        }
    }
    Some((g, [x0, y0, x1 - x0, y1 - y0]))
}

/// The unpadded centre of a padded plane: what a template is learned from.
pub fn centre(g: &Gray) -> Vec<f32> {
    window(g, PAD, PAD)
}

fn window(g: &Gray, dx: usize, dy: usize) -> Vec<f32> {
    let mut v = Vec::with_capacity(TW * TH);
    for y in 0..TH {
        let row = (y + dy) * g.w + dx;
        v.extend_from_slice(&g.px[row..row + TW]);
    }
    v
}

/// Normalised cross correlation over the columns `c0..c1` of two
/// `TW x TH` planes. 0 when either side is flat.
fn ncc_cols(a: &[f32], b: &[f32], c0: usize, c1: usize) -> f32 {
    let n = ((c1 - c0) * TH) as f32;
    let (mut sa, mut sb) = (0.0f32, 0.0f32);
    for y in 0..TH {
        for x in c0..c1 {
            sa += a[y * TW + x];
            sb += b[y * TW + x];
        }
    }
    let (ma, mb) = (sa / n, sb / n);
    let (mut ab, mut aa, mut bb) = (0.0f32, 0.0f32, 0.0f32);
    for y in 0..TH {
        for x in c0..c1 {
            let p = a[y * TW + x] - ma;
            let q = b[y * TW + x] - mb;
            ab += p * q;
            aa += p * p;
            bb += q * q;
        }
    }
    let d = (aa * bb).sqrt();
    if d < 1e-6 { 0.0 } else { ab / d }
}

/// Whole and worst strip score of a window against a template.
fn score_window(w: &[f32], t: &[f32]) -> (f32, f32) {
    let whole = ncc_cols(w, t, 0, TW);
    let sw = TW / STRIPS;
    let strip = (0..STRIPS)
        .map(|i| ncc_cols(w, t, i * sw, (i + 1) * sw))
        .fold(f32::MAX, f32::min);
    (whole, strip)
}

/// Every class's scores at its best shift, best whole score first.
pub fn rank(templates: &TemplateSet, g: &Gray) -> Vec<ClassScore> {
    let windows: Vec<(usize, usize, Vec<f32>)> = (0..=2 * PAD)
        .flat_map(|dy| (0..=2 * PAD).map(move |dx| (dx, dy)))
        .map(|(dx, dy)| (dx, dy, window(g, dx, dy)))
        .collect();
    let mut out: Vec<ClassScore> = templates
        .classes
        .iter()
        .map(|t| {
            let mut best = ClassScore {
                class: t.class.clone(),
                score: f32::MIN,
                strip: f32::MIN,
                dx: PAD,
                dy: PAD,
            };
            for (dx, dy, w) in &windows {
                let (score, strip) = score_window(w, &t.vector);
                if score > best.score {
                    best = ClassScore {
                        score,
                        strip,
                        dx: *dx,
                        dy: *dy,
                        ..best
                    };
                }
            }
            best
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

/// The accepted class of a ranking, or `None` when the best class misses
/// [`MIN_SCORE`], [`MIN_STRIP`] or [`MIN_MARGIN`].
pub fn accept(ranked: &[ClassScore]) -> Option<&ClassScore> {
    let best = ranked.first()?;
    let second = ranked.get(1).map_or(0.0, |c| c.score.max(0.0));
    (best.score >= MIN_SCORE && best.strip >= MIN_STRIP && best.score - second >= MIN_MARGIN)
        .then_some(best)
}

/// The queue title reader: a loaded template pack.
#[derive(Debug, Clone)]
pub struct Reader {
    pub templates: TemplateSet,
}

impl Reader {
    pub fn new(templates: TemplateSet) -> Self {
        Self { templates }
    }

    /// Load the pack from `<data_dir>/templates/queue-title-v1/`.
    pub fn load(data_dir: &Path) -> Result<Self, TemplateError> {
        TemplateSet::load(data_dir).map(Self::new)
    }

    /// The gated entry point: a reader only when [`ENV`] is on and a pack
    /// loads. Off, or no pack, is `None` and nothing runs.
    pub fn from_env(data_dir: &Path) -> Option<Self> {
        Self::for_setting(QueueTitleSetting::from_env(), data_dir)
    }

    pub fn for_setting(setting: QueueTitleSetting, data_dir: &Path) -> Option<Self> {
        if !setting.is_on() {
            return None;
        }
        match Self::load(data_dir) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!("queue title reader on but not loaded: {e}");
                None
            }
        }
    }

    /// Read the title of one frame.
    pub fn read(&self, img: &RgbImage, screen: Screen) -> QueueTitleRead {
        QueueTitleRead::new(self.read_field(img, screen))
    }

    pub fn read_field(&self, img: &RgbImage, screen: Screen) -> Field<String> {
        let Some((g, [x0, y0, cw, ch])) = plane(img, screen) else {
            return Field::none();
        };
        let ranked = rank(&self.templates, &g);
        let Some(best) = accept(&ranked) else {
            return Field::none();
        };
        let fx = cw as f32 / g.w as f32;
        let fy = ch as f32 / g.h as f32;
        let bx = |d: usize| x0 + (d as f32 * fx).round() as u32;
        let by = |d: usize| y0 + (d as f32 * fy).round() as u32;
        Field {
            value: Some(best.class.clone()),
            confidence: best.score.clamp(0.0, 1.0),
            bbox: Some([bx(best.dx), by(best.dy), bx(best.dx + TW), by(best.dy + TH)]),
        }
    }

    /// One value for a game from its pregame dump folder
    /// (`<screen>_<first|last>.png`), by [`POLICY`]. Missing or unreadable
    /// files are skipped.
    pub fn read_dump_dir(&self, dir: &Path) -> QueueTitleRead {
        for (screen, frame) in POLICY {
            let p = dir.join(format!("{}_{frame}.png", screen.as_str()));
            let Ok(img) = image::open(&p) else {
                continue;
            };
            let f = self.read_field(&img.to_rgb8(), screen);
            if f.value.is_some() {
                return QueueTitleRead::new(f);
            }
        }
        QueueTitleRead::new(Field::none())
    }
}

/// The frames a game's title is taken from, in order: hero ban (most
/// frames, never caught mid fade), then the last hero select frame. Map
/// vote and the first hero select frame are left out: they are the ones
/// caught in the screen fade.
pub const POLICY: [(Screen, &str); 3] = [
    (Screen::HeroBan, "last"),
    (Screen::HeroBan, "first"),
    (Screen::HeroSelect, "last"),
];

/// The first read with a value, in [`POLICY`] order. `reads` holds
/// `(screen, "first" | "last", field)` for whatever frames were read.
pub fn pick<'a>(reads: &'a [(Screen, &str, Field<String>)]) -> Option<&'a Field<String>> {
    POLICY.iter().find_map(|(s, fr)| {
        reads
            .iter()
            .find(|(rs, rf, f)| rs == s && rf == fr && f.value.is_some())
            .map(|(_, _, f)| f)
    })
}
