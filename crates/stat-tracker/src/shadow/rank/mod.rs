//! rank-v1: the competitive rank update reader. Reads the screen shown after
//! a competitive game (role rank and division, rank progress, the progress
//! change pill and its tags, "higher ranked than X% of players", the first
//! hero's SR and SR change, and the next mode banner) by cutting each field's
//! text out by colour and matching every glyph or word against templates. A
//! frame goes in; each field comes out as a value with a confidence, and a
//! value the screen does not show clearly is `None`, never a guess.
//!
//! Off by default. The tracker's capture wiring (`daemon::rank` in the
//! binary, through [`game::read_for_game`]) runs the reader only when
//! config `rank_reader` (Read my rank after Comp games) is on, or [`ENV`]
//! overrides it to on ([`RankSetting::for_config`]), and a template pack is
//! installed ([`templates`]); it then reads the rank screen's frames after a
//! game, stores the result on that game and uploads it in the `rank` object.
//! Which frames to read and how reads are combined are the wiring's; see
//! "What a caller must do" below, which also says what the wiring does.
//!
//! # Spellings
//!
//! The strings are the pack's class names, which come from the bench labels:
//!
//! * `role`: [`Role`], spelled as the site's `scuffed_types::api::RankRole`
//!   (`Tank`, `Damage`, `Support`, `Open`; [`game::rank_role`] pins it);
//! * `role_rank`: the tier word, capitalised (`Master`, `Diamond`);
//! * `change_tags`: the tag words as the game writes them, one class per tag
//!   (`Calibration Up` and `Calibration Down` among them; there are no
//!   streak tags). The reader has no cap; the site keeps at most 8 and nulls
//!   a longer list, so the wiring sends no list above 8
//!   ([`crate::model::RANK_MAX_CHANGE_TAGS`]);
//! * `next_mode_hint`: the banner word as its class is named. The shipped
//!   pack names it as the screen writes it (`COMPETITIVE: 6V6 OPEN QUEUE`,
//!   `COMPETITIVE: ROLE QUEUE`). The hint names the next game's queue; the
//!   daemon's rank wiring drops the `COMPETITIVE:` and a team size word
//!   after it (`5V5`, `6V6`) before it hands the hint on, and the hint
//!   only agrees with that game's queue title (`OPEN QUEUE`, `ROLE QUEUE`)
//!   when both reduce to the same key (`model::queue_key`); a spelling that
//!   does not is dropped, never confirmed.
//!
//! `hero_sr` and `hero_sr_change` are the first hero listed. After a hero
//! swap that need not be the hero of the game.
//!
//! # Confidence
//!
//! A field's confidence blends its weakest glyph's score and lead
//! ([`MIN_SCORE`], [`MIN_MARGIN`]): an accepted glyph field is at least about
//! 0.53, an accepted word field about 0.42, and [`Role::Open`] is fixed at
//! [`OPEN_CONFIDENCE`]. The queue title reader instead reports its raw score
//! (bar 0.80). The two scales are not comparable; a caller that wants a bar
//! sets one per field.
//!
//! # How it reads
//!
//! The role (Tank, Damage, Support) is read from the role icon shown left of
//! the role name above the emblem. Open queue screens show no role line, so
//! a role band with no ink, not even faint ink, reads as [`Role::Open`] (at
//! [`OPEN_CONFIDENCE`], below any accepted icon read).
//!
//! Every field has a fixed place at the 2560x1440 reference ([`layout`]),
//! scaled to 1080p by height/1440 (and width/2560). The hero
//! entry is found as the first "name line over a taller SR line" in the
//! right column, and the pill is found by its colour on the bar.
//! Text is turned into an ink plane by colour ([`ink`]): cyan or orange for
//! the progress, yellow for the percentile, white on the pill, grey-white for
//! the hero SR (scaled to its own peak so a chat overlay that dims it does
//! not hide it), green or red for the deltas. The ink is cut into glyphs at
//! empty columns, and each glyph is compared, aspect kept, with the class
//! means of its family by correlation (for single glyphs, the best of the
//! cut and its one pixel shifts). A cut wider than tall that matches nothing
//! is split at its thinnest column and both halves are read (two digits
//! that touch at 1080p). A field is accepted only when every
//! glyph's best score is at least [`MIN_SCORE`] and leads the next class by
//! [`MIN_MARGIN`]; that holds for the pill's `+` and chevrons too, which are
//! matched to the same bar and then left out of the value. Signs come from
//! colour (orange progress, red pill, red SR change), not from a minus glyph.
//! Each value must also be in its field's range (percentages within 100, a
//! division 1 to 5, a hero SR of 3 or 4 digits).
//!
//! # What a caller must do
//!
//! * Pass the **full game client frame**, 16:9, at 1920x1080 or 2560x1440
//!   (height within 2%, [`layout::Scale`]). Not a crop: every box is a
//!   fixed place in the full frame. Any other size (a crop, 16:10, 21:9,
//!   720p, 4K) is [`ReadError::Unsupported`], so a wiring mistake shows up
//!   as such and not as "no rank screen".
//! * Nothing here detects the animation. What is caught: text that is not
//!   drawn yet reads `None`, and a glyph caught mid-fade that matches no
//!   class well makes its field `None`. What is not caught: a frame taken
//!   while the pill or the SR is still counting shows clean digits of a
//!   value that is not final yet, and reads as that value with full
//!   confidence. One read of one frame is never enough to trust a value.
//! * Install a pack ([`templates::TemplateSet::load`]); without it
//!   [`Reader::load`] fails and the reader is off.
//!
//! The tracker's wiring (`daemon::rank` in the binary) does this: it reads
//! every frame the end detector flags as the rank screen, 2560x1440 only,
//! through [`game::read_for_game`], and holds the reads for the open game.
//! While a screen is held, it also reads a frame whose rank-screen word the
//! stability gate held back on a first sighting (the first tick after a
//! focus pause, so the end detector never looked at it); a read there is
//! held like any other, and a frame it does not read is a miss.
//! A single tick that misses the screen is a break between reads; two in a
//! row, another game, or the game closing end the screen. Then each field
//! is kept only when the newest read that has it agrees with the read right
//! before it, at the lower confidence, so a value still counting up or one
//! the screen replaced on its newest read is dropped. What that does not
//! catch: two frames in a row that both show the same value that is not
//! final yet (the count paused on one value for two ticks), and a field
//! whose last agreed pair is followed only by frames that no longer read
//! it (a fade, or a final value the reader cannot read). A field that
//! stops reading while others go on keeps that older pair, so the settled
//! record can mix fields from different moments: reads of division 3 at
//! 95%, division 3 at 95%, then no division at 5% twice settle to
//! division 3 at 5%, a record the screen never showed.
//! [`Reader::is_rank_screen`] (the "RANK PROGRESS:" label only, cheap) is
//! there for a caller that wants to check frames before a full read.
//!
//! A supported frame that does not show the "RANK PROGRESS:" label (another
//! screen, the placement screen) is [`ReadError::NotRankScreen`].

pub mod game;
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
    CHEV_LEFT, CHEV_RIGHT, Family, Learner, PACK_FILE, SR_ICON, TEMPLATE_DIR, TemplateError,
    TemplateSet,
};

/// The recognizer id written next to the values: the one id the upload
/// sends as `rank.rank_recognizer` for this reader and the queue title
/// reader alike, defined once in `model`.
pub use crate::model::RANK_RECOGNIZER_ID as RECOGNIZER_ID;
/// Environment override of config `rank_reader` for this reader: `rank-v1`,
/// `on`, `1`, `true` or `yes` turns it on, any other non-blank value off.
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
/// Confidence of [`Role::Open`]: it is read from the absence of a role line,
/// not from an icon, so it stays below the lowest confidence an accepted icon
/// read can have (about 0.53, at [`MIN_SCORE`] and [`MIN_MARGIN`]).
pub const OPEN_CONFIDENCE: f32 = 0.5;
/// Reference frame width the layout was measured at. Other 16:9 sizes are
/// scaled from this ([`layout::Scale`]).
pub const FRAME_W: u32 = 2560;
/// Reference frame height the layout was measured at.
pub const FRAME_H: u32 = 1440;

/// Whether rank-v1 runs. Off unless config `rank_reader` or [`ENV`] turns
/// it on ([`Self::for_config`], see the module docs).
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

    /// [`ENV`] as an override: `None` when it is unset or blank, else
    /// [`Self::parse`] of it (on or off).
    pub fn env_override() -> Option<Self> {
        override_of(std::env::var(ENV).ok().as_deref()).map(Self::parse)
    }

    /// The setting the tracker runs: `env` ([`Self::env_override`]) when
    /// set, else config `rank_reader`.
    pub fn for_config(rank_reader: bool, env: Option<Self>) -> Self {
        env.unwrap_or(if rank_reader { Self::RankV1 } else { Self::Off })
    }

    pub fn is_on(self) -> bool {
        self == Self::RankV1
    }
}

/// An env value that overrides the config: `None` when unset or blank.
/// Shared with the queue title reader's switch and the app's lock line.
pub fn override_of(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

/// How [`pack_warning`] says to turn the rank switch off.
const RANK_OFF: &str = "turn off Read my rank after Comp games";

/// The warning for the rank readers that are on but have no usable pack:
/// rank-v1 (`rank_on`, `<data_dir>/templates/rank-v1/`) and the queue
/// title reader (`queue_on`, `<data_dir>/templates/queue-title-v1/`).
/// Loads each pack the way the tracker does at start. `None` when every
/// reader that is on loads. Games still record either way: a reader with
/// no pack reads nothing.
///
/// The line names what is on as the switch shows it: "Read my rank" only
/// when both readers are on, else the one reader the variables left on
/// (with the rank switch Off, the queue title reader can still be on).
/// It says "that pack" or "those packs" by how many fail, and that the
/// tracker loads packs only at start, so a pack put in place while it runs
/// reads after a restart.
///
/// `rank_env` is true when [`ENV`] decides the rank reader: the switch is
/// then locked, so with the rank reader on the line says to set the
/// variables to off, not to turn the switch off.
pub fn pack_warning(
    data_dir: &Path,
    rank_on: bool,
    queue_on: bool,
    rank_env: bool,
) -> Option<String> {
    use crate::shadow::queue_title;
    let problem = |dir: &str, missing: bool, error: String| {
        if missing {
            format!("the pack in {dir} is not installed")
        } else {
            format!("the pack in {dir} did not load ({error})")
        }
    };
    let mut problems = Vec::new();
    if rank_on && let Err(e) = Reader::load(data_dir) {
        let missing = matches!(e, TemplateError::Missing(_));
        problems.push(problem(TEMPLATE_DIR, missing, e.to_string()));
    }
    if queue_on && let Err(e) = queue_title::Reader::load(data_dir) {
        let missing = matches!(e, queue_title::TemplateError::Missing(_));
        problems.push(problem(queue_title::TEMPLATE_DIR, missing, e.to_string()));
    }
    if problems.is_empty() {
        return None;
    }
    let (lead, off) = match (rank_on, queue_on) {
        (true, true) if rank_env => (
            "Read my rank is on",
            format!("set {ENV} and {} to off", queue_title::ENV),
        ),
        (true, true) => ("Read my rank is on", RANK_OFF.to_string()),
        (true, false) if rank_env => ("The rank screen reader is on", format!("set {ENV} to off")),
        (true, false) => ("The rank screen reader is on", RANK_OFF.to_string()),
        _ => (
            "The queue title reader is on",
            format!("set {} to off", queue_title::ENV),
        ),
    };
    let (what, them) = if problems.len() == 1 {
        ("that pack reads", "the pack")
    } else {
        ("those packs read", "the packs")
    };
    Some(format!(
        "{lead}, but {}. Games still record, without what {what}. The tracker loads packs \
         only at start: install {them}, then restart the tracker, or {off}.",
        problems.join(" and ")
    ))
}

/// The queue role a rank update is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    Tank,
    Damage,
    Support,
    /// Open queue: the screen shows no role line.
    Open,
}

impl Role {
    pub const ICONS: [Role; 3] = [Role::Tank, Role::Damage, Role::Support];

    /// The template class name (`Tank`, `Damage`, `Support`, `Open`).
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Tank => "Tank",
            Role::Damage => "Damage",
            Role::Support => "Support",
            Role::Open => "Open",
        }
    }

    pub fn from_class(s: &str) -> Option<Self> {
        [Role::Tank, Role::Damage, Role::Support, Role::Open]
            .into_iter()
            .find(|r| r.as_str() == s)
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
    /// Queue role, from the role icon above the emblem.
    pub role: Field<Role>,
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
    /// SR of the first hero listed (not always the game's hero, after a
    /// swap). `None` when the entry found may not be the first one or its
    /// SR may be cut short ([`layout::first_entry`], [`layout::sr_cut_short`]).
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
            ("role", json(&self.role.value)),
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
            ("role", self.role.confidence),
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

/// Why [`Reader::read`] or [`Reader::is_rank_screen`] gave no read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadError {
    /// The frame is not a size the reader takes: it wants the full game
    /// client frame, 16:9, at 1080p or 1440p ([`layout::Scale`]). A crop, a
    /// 16:10 or 21:9 frame or another height lands here. This is a wiring
    /// problem for the caller to report, not a verdict on the screen.
    Unsupported { width: u32, height: u32 },
    /// A supported frame that does not show the "RANK PROGRESS:" label.
    NotRankScreen,
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Unsupported { width, height } => write!(
                f,
                "rank-v1 takes the full 16:9 client frame at 1080p or 1440p, got {width}x{height}"
            ),
            ReadError::NotRankScreen => write!(f, "not a rank update screen"),
        }
    }
}

impl std::error::Error for ReadError {}

/// Labels of one rank screen, as the bench's JSONL spells them. Used to
/// learn templates ([`Learner::add_frame`]) and to score reads.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Labels {
    /// Absent from the main label file; merged from a separate role file.
    #[serde(default)]
    pub role: Option<Role>,
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
    pub const NAMES: [&'static str; 10] = [
        "role",
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
            json(&self.role),
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

    /// Whether `img` is a rank update screen: the "RANK PROGRESS:" label
    /// only, no field is read. `Ok` holds the label's match score (at least
    /// [`MIN_WORD_SCORE`]). Cheap enough to run on every candidate frame, so
    /// a caller can [`read`](Self::read) once, on the last screen frame.
    pub fn is_rank_screen(&self, img: &RgbImage) -> Result<f32, ReadError> {
        if layout::Scale::from_img(img).is_none() {
            let (width, height) = img.dimensions();
            return Err(ReadError::Unsupported { width, height });
        }
        let gate = layout::gate_word(img).ok_or(ReadError::NotRankScreen)?;
        // only the label's own class: a pack may hold other gate words
        match self.templates.best(Family::Gate, &gate.ink, None) {
            Some(m) if m.class == GATE_CLASS && m.score >= MIN_WORD_SCORE => Ok(r3(m.score)),
            _ => Err(ReadError::NotRankScreen),
        }
    }

    /// Read a rank update screen from the full client frame (see the module
    /// docs for the contract). Fields the screen does not show clearly are
    /// `None`; the whole read is an error when the frame is not a supported
    /// size or not a rank screen ([`ReadError`]).
    pub fn read(&self, img: &RgbImage) -> Result<RankRead, ReadError> {
        self.is_rank_screen(img)?;
        let t = &self.templates;

        let (pct, negative) = layout::pct_glyphs(img);
        let pct_allowed: Vec<&str> = DIGITS.iter().copied().chain(["%", "-", "+"]).collect();
        let progress_pct = self.number(Family::Pct, &pct, &pct_allowed, &[], |s| {
            let v: i32 = s.strip_suffix('%')?.parse().ok()?;
            within(if negative && v > 0 { -v } else { v }, -100..=100)
        });

        let higher_allowed: Vec<&str> = DIGITS.iter().copied().chain(["%"]).collect();
        let higher_than_pct = self.number(
            Family::Higher,
            &layout::higher_glyphs(img),
            &higher_allowed,
            &[],
            |s| within(s.strip_suffix('%')?.parse().ok()?, 0..=100),
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
                |s| within(s.strip_suffix('%')?.parse::<i32>().ok()? * sign, -100..=100),
            ),
            None => Field::none(),
        };

        // a window that may end inside the digits reads no SR, and the
        // change under it is right aligned to the same wrong edge
        let (hero_sr, hero_sr_change) = match layout::first_entry(img) {
            Some(e) if !layout::sr_cut_short(img, &e) => {
                // a rank icon taller than the digits is left out by shape
                // (layout::sr_glyphs); one of digit height is matched as
                // the SR_ICON class and stripped, at the start only.
                // Touching digits are split in number().
                let gl = layout::sr_glyphs(img, &e);
                let sr_allowed: Vec<&str> = DIGITS.iter().copied().chain([SR_ICON]).collect();
                let sr = self.number(Family::Sr, &gl, &sr_allowed, &[SR_ICON], |s| {
                    (3..=4).contains(&s.len()).then(|| s.parse().ok()).flatten()
                });
                let (cg, sign) = layout::change_glyphs(img, &e);
                let change = match sign {
                    Some(sign) => self.number(Family::Change, &cg, &DIGITS, &[], |s| {
                        // the change is at most three digits
                        let v: i32 = s.parse().ok()?;
                        within(v * sign, -999..=999)
                    }),
                    None => Field::none(),
                };
                (sr, change)
            }
            _ => (Field::none(), Field::none()),
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
                (m.score >= MIN_WORD_SCORE && m.margin >= MIN_WORD_MARGIN).then(|| Field {
                    confidence: r3(confidence(m.score, m.margin)),
                    value: Some(m.class),
                    bbox: Some(g.bbox),
                })
            })
            .unwrap_or_else(Field::none);

        let text = layout::rank_text(img);
        let role_rank = text
            .as_ref()
            .and_then(|(word, _)| {
                let m = t
                    .best(Family::Tier, &word.ink, None)
                    .filter(|m| m.score >= MIN_WORD_SCORE && m.margin >= MIN_WORD_MARGIN)?;
                Some(Field {
                    confidence: r3(confidence(m.score, m.margin)),
                    value: Some(m.class),
                    bbox: Some(word.bbox),
                })
            })
            .unwrap_or_else(Field::none);
        // the division digit, and the neighbour emblems' digits (read even
        // when the tier line is not found), each only 1 to 5
        let vote = |f: Family, g: Option<&Glyph>, implied: fn(u8) -> u8| {
            let g = g?;
            let m = t
                .best(f, &g.ink, Some(&DIGITS[1..6]))
                .filter(|m| m.score >= MIN_SCORE && m.margin >= MIN_MARGIN)?;
            let v: u8 = m.class.parse().ok()?;
            Some(Vote {
                division: implied(v),
                confidence: confidence(m.score, m.margin),
                bbox: g.bbox,
            })
        };
        let (left, right) = layout::emblem_digits(img);
        let division = division_from_votes(
            vote(Family::Division, text.as_ref().map(|(_, d)| d), |d| d),
            vote(Family::Emblem, left.as_ref(), |l| (l + 3) % 5 + 1),
            vote(Family::Emblem, right.as_ref(), |r| r % 5 + 1),
        );

        let role = self.role(img);

        Ok(RankRead {
            role,
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

    /// The role icon matched against [`Family::Role`]; an empty role band
    /// ([`layout::RoleLine::Empty`]) is [`Role::Open`] at
    /// [`OPEN_CONFIDENCE`]. A line whose icon matches no role well is `None`.
    fn role(&self, img: &RgbImage) -> Field<Role> {
        match layout::role_icon(img) {
            layout::RoleLine::Empty => Field {
                value: Some(Role::Open),
                confidence: OPEN_CONFIDENCE,
                bbox: None,
            },
            layout::RoleLine::Unclear => Field::none(),
            layout::RoleLine::Icon(g) => self
                .templates
                .best(Family::Role, &g.ink, None)
                .filter(|m| m.score >= MIN_SCORE && m.margin >= MIN_MARGIN)
                .and_then(|m| Some((Role::from_class(&m.class)?, m)))
                .map_or_else(Field::none, |(r, m)| Field {
                    value: Some(r),
                    confidence: r3(confidence(m.score, m.margin)),
                    bbox: Some(g.bbox),
                }),
        }
    }

    /// Read glyphs as one string and parse it. Every cut, the `strip`
    /// classes (`+`, chevrons) included, must score at least [`MIN_SCORE`]
    /// and lead by [`MIN_MARGIN`]. A cut wider than tall that fails is split
    /// at its thinnest column and both halves are held to the same bar (two
    /// touching digits, or a digit merged into the `+`); otherwise the field
    /// is `None`. A `strip` class (`+`, chevrons, the SR rank icon) is left
    /// out of the string, and only at either end (before any digit, or after
    /// the `%`): one in the middle would hide a digit, so it makes the field
    /// `None`.
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
        let best = |g: &Glyph| {
            self.templates
                .best(f, &g.ink, Some(allowed))
                .filter(|m| m.score >= MIN_SCORE && m.margin >= MIN_MARGIN)
        };
        let mut s = String::new();
        let mut conf = 1.0f32;
        for g in gl {
            let ms = match best(g) {
                Some(m) => vec![m],
                None => {
                    let halves = (g.ink.w > g.ink.h).then(|| ink::split_two(g)).flatten();
                    match halves.map(|(a, b)| (best(&a), best(&b))) {
                        Some((Some(a), Some(b))) => vec![a, b],
                        _ => return Field::none(),
                    }
                }
            };
            for m in ms {
                conf = conf.min(confidence(m.score, m.margin));
                if !strip.contains(&m.class.as_str()) {
                    s.push_str(&m.class);
                } else if !(s.is_empty() || s.ends_with('%')) {
                    return Field::none();
                }
            }
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

/// The divisions the two neighbour emblems show for division `d` (1..=5):
/// the one below (left; 5 shows the next tier's 1) and the one above
/// (right; 1 shows the tier above's 5).
pub fn neighbours(d: u8) -> (u8, u8) {
    (d % 5 + 1, (d + 3) % 5 + 1)
}

/// One reading of the division: the big digit, or what a neighbour emblem
/// implies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vote {
    pub division: u8,
    pub confidence: f32,
    pub bbox: [u32; 4],
}

/// The division from the big digit and the two neighbour emblems.
///
/// The big digit alone cannot tell a lookalike from an unseen class: its
/// score is only compared with the classes the pack holds, so a "3" read
/// by a pack without a "3" scores 0.77 against "5" and wins. The emblems
/// are independent glyphs (another size, another place) of other digits,
/// so the division is what the readings agree on:
///
/// * all present readings agree: that value, at the best confidence (one
///   reading alone is the big digit as before; a single emblem alone is too
///   little and is `None`);
/// * two agree and the third differs: the two, at the lower of their two
///   confidences;
/// * otherwise `None`.
pub fn division_from_votes(
    big: Option<Vote>,
    left: Option<Vote>,
    right: Option<Vote>,
) -> Field<u8> {
    let votes: Vec<Vote> = [big, left, right].into_iter().flatten().collect();
    let pick = |v: u8, agree: &[Vote], conf: f32| Field {
        value: Some(v),
        confidence: r3(conf),
        bbox: big
            .filter(|b| b.division == v)
            .map(|b| b.bbox)
            .or_else(|| union(agree.iter().map(|a| a.bbox))),
    };
    if votes.is_empty() || (big.is_none() && votes.len() == 1) {
        return Field::none();
    }
    for v in votes.iter().map(|v| v.division) {
        let agree: Vec<Vote> = votes.iter().copied().filter(|x| x.division == v).collect();
        let best = agree.iter().map(|a| a.confidence).fold(0.0, f32::max);
        let worst = agree.iter().map(|a| a.confidence).fold(1.0, f32::min);
        if agree.len() == votes.len() {
            return pick(v, &agree, best);
        }
        if agree.len() >= 2 {
            return pick(v, &agree, worst);
        }
    }
    Field::none()
}

const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// 0..1 from a score and a lead: half for the score (full at 0.95), half for
/// the lead (full at 0.25).
fn confidence(score: f32, margin: f32) -> f32 {
    (0.5 * (score / 0.95).min(1.0) + 0.5 * (margin / 0.25).min(1.0)).clamp(0.0, 1.0)
}

/// `v` when it is in `r`.
fn within<T: PartialOrd>(v: T, r: std::ops::RangeInclusive<T>) -> Option<T> {
    r.contains(&v).then_some(v)
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
