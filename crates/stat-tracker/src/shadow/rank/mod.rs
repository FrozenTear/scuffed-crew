//! rank-v1: the competitive rank update reader. Reads the screen shown after
//! a competitive game (role rank and division, rank progress, the progress
//! change pill and its tags, "higher ranked than X% of players", the first
//! hero's SR and SR change, and the next mode banner) by cutting each field's
//! text out by colour and matching every glyph or word against templates. A
//! frame goes in; each field comes out as a value with a confidence, and a
//! value the screen does not show clearly is `None`, never a guess.
//!
//! Off by default. It runs only when [`ENV`] (or the setting it feeds) is
//! switched on ([`RankSetting::parse`]) and a template pack is installed
//! ([`templates`]). The capture wiring (which frame to keep) and any upload
//! field are not part of this module; see "What a caller must do" below.
//!
//! # How it reads
//!
//! Every field has a fixed place at 2560x1440 ([`layout`]) except the hero
//! entry, which is found as the first "name line over a taller SR line" in
//! the right column, and the pill, which is found by its colour on the bar.
//! Text is turned into an ink plane by colour ([`ink`]): cyan or orange for
//! the progress, yellow for the percentile, white on the pill, grey-white for
//! the hero SR (scaled to its own peak so a chat overlay that dims it does
//! not hide it), green or red for the deltas. The ink is cut into glyphs at
//! empty columns, and each glyph is compared, aspect kept, with the class
//! means of its family by correlation. A field is accepted only when every
//! glyph's best score is at least [`MIN_SCORE`] and leads the next class by
//! [`MIN_MARGIN`]. Signs come from colour (orange progress, red pill, red SR
//! change), not from a minus glyph.
//!
//! # What a caller must do
//!
//! * Pass the **last** frame of the rank update, when the pill and the hero
//!   SR have settled. Mid-animation frames have a fading pill or a scaled SR
//!   and read as `None`.
//! * Pass 2560x1440 frames. Other sizes are refused.
//! * Install a pack ([`templates::TemplateSet::load`]); without it
//!   [`Reader::load`] fails and the reader is off.
//!
//! A frame that does not show the "RANK PROGRESS:" label (another screen, the
//! placement screen) reads as `None` as a whole.

pub mod ink;
pub mod layout;
pub mod templates;

#[cfg(test)]
mod tests;

use std::path::Path;

use image::RgbImage;
use serde::{Deserialize, Serialize};

use ink::Glyph;
pub use templates::{
    CHEV_LEFT, CHEV_RIGHT, Family, Learner, TEMPLATE_DIR, TemplateError, TemplateSet,
};

/// The recognizer id written next to the values.
pub const RECOGNIZER_ID: &str = "rank-v1";
/// Environment switch: `rank-v1`, `on`, `1`, `true` or `yes` turns it on.
pub const ENV: &str = "STAT_TRACKER_RANK_RECOGNIZER";
/// Lowest glyph score a read may have.
pub const MIN_SCORE: f32 = 0.70;
/// Least lead over the second best class a glyph must have.
pub const MIN_MARGIN: f32 = 0.08;
/// Lowest score for a word (tag, banner, tier) and the gate label.
pub const MIN_WORD_SCORE: f32 = 0.60;
/// Least lead for a tag or tier word.
pub const MIN_WORD_MARGIN: f32 = 0.05;
/// The class name of the "RANK PROGRESS:" label in [`Family::Gate`].
pub const GATE_CLASS: &str = "rank_progress";
/// The only frame size the reader accepts.
pub const FRAME_W: u32 = 2560;
pub const FRAME_H: u32 = 1440;

/// Whether rank-v1 runs. Off unless switched on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RankSetting {
    #[default]
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "rank-v1")]
    RankV1,
}

impl RankSetting {
    /// `rank-v1`, `on`, `1`, `true` or `yes` (any case) is on; anything else
    /// is off.
    pub fn parse(s: &str) -> Self {
        let v = s.trim();
        if ["rank-v1", "on", "1", "true", "yes"]
            .iter()
            .any(|o| v.eq_ignore_ascii_case(o))
        {
            Self::RankV1
        } else {
            Self::Off
        }
    }

    /// The setting from [`ENV`]; off when it is unset.
    pub fn from_env() -> Self {
        std::env::var(ENV).map_or(Self::default(), |v| Self::parse(&v))
    }

    pub fn is_on(self) -> bool {
        self == Self::RankV1
    }
}

/// One field: a value, how sure, and where it was read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field<T> {
    /// `None` when the field is not shown or cannot be read.
    pub value: Option<T>,
    /// 0..1; 0 when `value` is `None`.
    pub confidence: f32,
    /// Frame box `[x0, y0, x1, y1)` of the text read (diagnostic).
    pub bbox: Option<[u32; 4]>,
}

impl<T> Field<T> {
    fn none() -> Self {
        Self {
            value: None,
            confidence: 0.0,
            bbox: None,
        }
    }
}

/// Everything rank-v1 reads from one rank update screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankRead {
    /// Tier name as the template pack spells it (`Master`, `Diamond`).
    pub role_rank: Field<String>,
    pub division: Field<u8>,
    /// Rank progress percent, negative under demotion protection.
    pub progress_pct: Field<i32>,
    /// The pill on the bar, signed by its colour.
    pub progress_change: Field<i32>,
    /// Tags under the bar, left to right.
    pub change_tags: Field<Vec<String>>,
    pub higher_than_pct: Field<u8>,
    /// SR of the first hero listed.
    pub hero_sr: Field<u32>,
    /// That hero's SR change, signed by its colour.
    pub hero_sr_change: Field<i32>,
    /// The mode banner text, as the pack spells it.
    pub next_mode_hint: Field<String>,
    pub rank_recognizer: String,
}

fn json<T: Serialize>(v: &Option<T>) -> Option<serde_json::Value> {
    v.as_ref().and_then(|v| serde_json::to_value(v).ok())
}

impl RankRead {
    /// `(field name, value)` for every field, values as JSON.
    pub fn values(&self) -> Vec<(&'static str, Option<serde_json::Value>)> {
        vec![
            ("role_rank", json(&self.role_rank.value)),
            ("division", json(&self.division.value)),
            ("progress_pct", json(&self.progress_pct.value)),
            ("progress_change", json(&self.progress_change.value)),
            ("change_tags", json(&self.change_tags.value)),
            ("higher_than_pct", json(&self.higher_than_pct.value)),
            ("hero_sr", json(&self.hero_sr.value)),
            ("hero_sr_change", json(&self.hero_sr_change.value)),
            ("next_mode_hint", json(&self.next_mode_hint.value)),
        ]
    }

    /// `(field name, confidence)` for the fields that have a value.
    pub fn confidences(&self) -> Vec<(&'static str, f32)> {
        let c = [
            ("role_rank", self.role_rank.confidence),
            ("division", self.division.confidence),
            ("progress_pct", self.progress_pct.confidence),
            ("progress_change", self.progress_change.confidence),
            ("change_tags", self.change_tags.confidence),
            ("higher_than_pct", self.higher_than_pct.confidence),
            ("hero_sr", self.hero_sr.confidence),
            ("hero_sr_change", self.hero_sr_change.confidence),
            ("next_mode_hint", self.next_mode_hint.confidence),
        ];
        self.values()
            .into_iter()
            .zip(c)
            .filter(|((_, v), _)| v.is_some())
            .map(|(_, c)| c)
            .collect()
    }
}

/// Labels of one rank screen, as the bench's JSONL spells them. Used to
/// learn templates ([`Learner::add_frame`]) and to score reads.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Labels {
    pub role_rank: Option<String>,
    pub division: Option<u8>,
    pub progress_pct: Option<i32>,
    pub progress_change: Option<i32>,
    pub change_tags: Option<Vec<String>>,
    pub higher_than_pct: Option<u8>,
    pub hero_sr: Option<u32>,
    pub hero_sr_change: Option<i32>,
    pub next_mode_hint: Option<String>,
}

impl Labels {
    pub const NAMES: [&'static str; 9] = [
        "role_rank",
        "division",
        "progress_pct",
        "progress_change",
        "change_tags",
        "higher_than_pct",
        "hero_sr",
        "hero_sr_change",
        "next_mode_hint",
    ];

    /// `(field name, value)` in [`Labels::NAMES`] order, values as JSON.
    pub fn values(&self) -> Vec<(&'static str, Option<serde_json::Value>)> {
        let v = [
            json(&self.role_rank),
            json(&self.division),
            json(&self.progress_pct),
            json(&self.progress_change),
            json(&self.change_tags),
            json(&self.higher_than_pct),
            json(&self.hero_sr),
            json(&self.hero_sr_change),
            json(&self.next_mode_hint),
        ];
        Self::NAMES.into_iter().zip(v).collect()
    }
}

/// The reader.
pub struct Reader {
    templates: TemplateSet,
}

impl Reader {
    pub fn new(templates: TemplateSet) -> Self {
        Self { templates }
    }

    /// Load the pack from `<data_dir>/templates/rank-v1/`.
    pub fn load(data_dir: &Path) -> Result<Self, TemplateError> {
        TemplateSet::load(data_dir).map(Self::new)
    }

    pub fn templates(&self) -> &TemplateSet {
        &self.templates
    }

    /// Read a rank update screen. `None` when the frame is not 2560x1440 or
    /// does not show the "RANK PROGRESS:" label.
    pub fn read(&self, img: &RgbImage) -> Option<RankRead> {
        if img.dimensions() != (FRAME_W, FRAME_H) {
            return None;
        }
        let t = &self.templates;
        let gate = layout::gate_word(img)?;
        let m = t.best(Family::Gate, &gate.ink, None)?;
        if m.score < MIN_WORD_SCORE {
            return None;
        }

        let (pct, negative) = layout::pct_glyphs(img);
        let pct_allowed: Vec<&str> = DIGITS.iter().copied().chain(["%", "-", "+"]).collect();
        let progress_pct = self.number(Family::Pct, &pct, &pct_allowed, &[], |s| {
            let v: i32 = s.strip_suffix('%')?.parse().ok()?;
            Some(if negative && v > 0 { -v } else { v })
        });

        let higher_allowed: Vec<&str> = DIGITS.iter().copied().chain(["%"]).collect();
        let higher_than_pct = self.number(
            Family::Higher,
            &layout::higher_glyphs(img),
            &higher_allowed,
            &[],
            |s| s.strip_suffix('%')?.parse().ok(),
        );

        let (pill, sign) = layout::pill_glyphs(img);
        let pill_allowed: Vec<&str> = DIGITS
            .iter()
            .copied()
            .chain(["%", "+", CHEV_LEFT, CHEV_RIGHT])
            .collect();
        let progress_change = match sign {
            Some(sign) => self.number(
                Family::Pill,
                &pill,
                &pill_allowed,
                &["+", CHEV_LEFT, CHEV_RIGHT],
                |s| s.strip_suffix('%')?.parse::<i32>().ok().map(|v| v * sign),
            ),
            None => Field::none(),
        };

        let (hero_sr, hero_sr_change) = match layout::first_entry(img) {
            Some(e) => {
                let gl = layout::sr_glyphs(img, &e);
                // keep the rightmost four: the rank icon sits left of them
                let gl = &gl[gl.len().saturating_sub(4)..];
                let sr = self.number(Family::Sr, gl, &DIGITS, &[], |s| {
                    (3..=4).contains(&s.len()).then(|| s.parse().ok()).flatten()
                });
                let (cg, sign) = layout::change_glyphs(img, &e);
                let change = match sign {
                    Some(sign) => self.number(Family::Change, &cg, &DIGITS, &[], |s| {
                        s.parse::<i32>().ok().map(|v| v * sign)
                    }),
                    None => Field::none(),
                };
                (sr, change)
            }
            None => (Field::none(), Field::none()),
        };

        let words = layout::tag_words(img);
        let mut tags = Vec::new();
        let mut tag_conf = 1.0f32;
        for w in &words {
            match t.best(Family::Tag, &w.ink, None) {
                Some(m) if m.score >= MIN_WORD_SCORE && m.margin >= MIN_WORD_MARGIN => {
                    tag_conf = tag_conf.min(confidence(m.score, m.margin));
                    tags.push(m.class);
                }
                _ => {
                    tags.clear();
                    break;
                }
            }
        }
        let change_tags = if tags.is_empty() {
            Field::none()
        } else {
            Field {
                value: Some(tags),
                confidence: r3(tag_conf),
                bbox: union(words.iter().map(|w| w.bbox)),
            }
        };

        let next_mode_hint = layout::banner_word(img)
            .and_then(|g| {
                let m = t.best(Family::Banner, &g.ink, None)?;
                (m.score >= MIN_WORD_SCORE).then(|| Field {
                    confidence: r3(confidence(m.score, m.margin)),
                    value: Some(m.class),
                    bbox: Some(g.bbox),
                })
            })
            .unwrap_or_else(Field::none);

        let (role_rank, division) = match layout::rank_text(img) {
            Some((word, digit)) => {
                let tier = t
                    .best(Family::Tier, &word.ink, None)
                    .filter(|m| m.score >= MIN_WORD_SCORE && m.margin >= MIN_WORD_MARGIN)
                    .map_or_else(Field::none, |m| Field {
                        confidence: r3(confidence(m.score, m.margin)),
                        value: Some(m.class),
                        bbox: Some(word.bbox),
                    });
                let div = t
                    .best(Family::Division, &digit.ink, None)
                    .filter(|m| m.score >= MIN_SCORE && m.margin >= MIN_MARGIN)
                    .and_then(|m| Some((m.class.parse::<u8>().ok()?, m)))
                    .map_or_else(Field::none, |(v, m)| Field {
                        confidence: r3(confidence(m.score, m.margin)),
                        value: Some(v),
                        bbox: Some(digit.bbox),
                    });
                (tier, div)
            }
            None => (Field::none(), Field::none()),
        };

        Some(RankRead {
            role_rank,
            division,
            progress_pct,
            progress_change,
            change_tags,
            higher_than_pct,
            hero_sr,
            hero_sr_change,
            next_mode_hint,
            rank_recognizer: RECOGNIZER_ID.to_string(),
        })
    }

    /// Read glyphs as one string (classes in `strip` are matched but left
    /// out) and parse it. Any glyph under [`MIN_SCORE`] or [`MIN_MARGIN`]
    /// makes the whole field `None`.
    fn number<T>(
        &self,
        f: Family,
        gl: &[Glyph],
        allowed: &[&str],
        strip: &[&str],
        parse: impl Fn(&str) -> Option<T>,
    ) -> Field<T> {
        if gl.is_empty() {
            return Field::none();
        }
        let mut s = String::new();
        let mut conf = 1.0f32;
        for g in gl {
            let Some(m) = self.templates.best(f, &g.ink, Some(allowed)) else {
                return Field::none();
            };
            if strip.contains(&m.class.as_str()) {
                continue;
            }
            if m.score < MIN_SCORE || m.margin < MIN_MARGIN {
                return Field::none();
            }
            conf = conf.min(confidence(m.score, m.margin));
            s.push_str(&m.class);
        }
        match parse(&s) {
            Some(v) => Field {
                value: Some(v),
                confidence: r3(conf),
                bbox: union(gl.iter().map(|g| g.bbox)),
            },
            None => Field::none(),
        }
    }
}

const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// 0..1 from a score and a lead: half for the score (full at 0.95), half for
/// the lead (full at 0.25).
fn confidence(score: f32, margin: f32) -> f32 {
    (0.5 * (score / 0.95).min(1.0) + 0.5 * (margin / 0.25).min(1.0)).clamp(0.0, 1.0)
}

fn r3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

fn union(boxes: impl Iterator<Item = [u32; 4]>) -> Option<[u32; 4]> {
    boxes.reduce(|a, b| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    })
}
