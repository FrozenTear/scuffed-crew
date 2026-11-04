//! The ban vote screen: the local player's three vote slots and the TEAM
//! BAN SCORE strip, read at 2560x1440 only.
//!
//! The left roster column of this screen shows each player's preferred
//! heroes, not their votes, so it is never read here (the hero select
//! reader reads that column on its own screen). The votes are the three
//! small boxes under the local player's name; the strip at the top shows
//! the team's three most voted heroes with their vote counts.
//!
//! Every part is checked for its shape before it is read: the boxes must
//! have their border at the measured place and nothing border-like just
//! outside it, and the strip must have its red title. A part that does not
//! look like itself is `None`, and inside a part a slot or box that is not
//! clearly one thing is unread. A guess is never stored.
//!
//! # What was measured
//!
//! Over 34 saved ban vote frames (17 games, first and last frame of each),
//! with a Python twin of this file:
//!
//! * The rectangles are the same in every frame ([`OWN_SLOTS`],
//!   [`STRIP_BOXES`], [`STRIP_TITLE`]).
//! * Own slots: 51 skipped, 8 heroes and 43 empty, every one classified
//!   and every hero read as seen. Hero scores at [`Use::VoteSlot`] over the
//!   245 hero slots of all six rows: lowest right score 0.554, lowest
//!   margin 0.178, highest second best 0.464.
//! * Strip: all 102 boxes and counts read right. Lowest right hero score
//!   0.616, lowest margin 0.313, highest second best 0.348.
//! * 609 other saved frames (hero select, ban results, map vote, scoreboards,
//!   loading) and random noise: both parts `None` on every one.

use image::RgbImage;
use serde::{Deserialize, Serialize};

use super::digits::{Number, read_number};
use super::matcher::SearchPlan;
use super::{Choice, FRAME_H, FRAME_W, Field, NO_PREFERENCE_KEY, Read, Reader, Use, ranked};

/// A native rectangle, inclusive: `(x0, y0, x1, y1)`.
type Rect = (u32, u32, u32, u32);

/// The local player's three vote slots, left to right: the outer edge of
/// each box's border. The teammates' slots are at the same x in their own
/// rows (box tops at y 628, 757, 887, 1016 and 1149) and are not read yet.
pub const OWN_SLOTS: [(u32, u32, u32, u32); 3] = [
    (193, 456, 243, 506),
    (255, 456, 304, 506),
    (316, 456, 366, 506),
];
/// The strip's three hero boxes, most votes first.
pub const STRIP_BOXES: [(u32, u32, u32, u32); 3] = [
    (1154, 97, 1229, 171),
    (1242, 97, 1317, 171),
    (1330, 97, 1405, 171),
];
/// The red "TEAM BAN SCORE" title.
pub const STRIP_TITLE: (u32, u32, u32, u32) = (1180, 61, 1380, 76);
/// The rows of the vote counts under the strip boxes.
const COUNT_Y0: u32 = 180;
const COUNT_Y1: u32 = 210;
/// The count window is the box's width less this much on each side.
const COUNT_PAD: u32 = 11;

/// Border thickness of a slot or box, in pixels.
const BORDER: u32 = 3;
/// The ring just outside a box, which must not look like a border.
const OUT_GAP: u32 = 6;
/// Pixels kept off the border when looking at a box's inside.
const INSET: u32 = 4;
/// A slot's border ring must be at least this share red or grey.
const SLOT_BORDER_MIN: f32 = 0.75;
/// The same for a strip box (seen: 0.70 on a frame dimmed by a fade).
const STRIP_BORDER_MIN: f32 = 0.6;
/// The ring outside a box may be at most this share red or grey (seen:
/// 0.0 on every vote frame; a flat red or grey screen fills it).
const OUT_MAX: f32 = 0.25;
/// Share of title pixels that are red (seen: 0.45..0.49).
const TITLE_RED_MIN: f32 = 0.25;
const TITLE_RED_MAX: f32 = 0.75;

/// The skip icon, a circle with a slash, centred in the slot: a ring of
/// this radius sampled at this many points, and two diagonals.
const SKIP_RADIUS: f64 = 15.0;
const SKIP_POINTS: usize = 48;
/// Shares of icon pixels (seen 1.0, 1.0 and at most 0.31).
const SKIP_RING_MIN: f32 = 0.85;
const SKIP_SLASH_MIN: f32 = 0.85;
const SKIP_BACKSLASH_MAX: f32 = 0.5;
/// Share of the inside that is icon pixels for a skip.
const SKIP_SHARE_MIN: f32 = 0.10;
const SKIP_SHARE_MAX: f32 = 0.45;
/// An empty slot or box: a flat inside (grey standard deviation, seen at
/// most 2.3) with no icon pixels.
const EMPTY_STD_MAX: f32 = 8.0;
const EMPTY_ICON_MAX: f32 = 0.02;
/// A voted hero: a red border and a varied inside (seen at least 42).
const HERO_RED_MIN: f32 = 0.75;
const HERO_STD_MIN: f32 = 20.0;
/// A strip box is red (a hero) or grey (no hero yet) when this share of
/// its border is.
const BOX_COLOUR_MIN: f32 = 0.6;

/// Lowest best score of a vote slot hero (lowest right one seen: 0.554;
/// highest second best: 0.464).
pub const VOTE_MIN_SCORE: f32 = 0.5;
/// Least lead over the second best (lowest seen: 0.178).
pub const VOTE_MIN_MARGIN: f32 = 0.12;
/// Lowest best score of a strip hero (lowest right one seen: 0.616;
/// highest second best: 0.348).
pub const STRIP_MIN_SCORE: f32 = 0.5;
/// Least lead over the second best (lowest seen: 0.313).
pub const STRIP_MIN_MARGIN: f32 = 0.15;

/// What a vote slot shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotState {
    /// A hero is voted (red border, hero art).
    Hero,
    /// The vote was skipped (circle with a slash).
    Skipped,
    /// Nothing yet (flat inside).
    Empty,
}

/// One ban vote slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoteSlotRead {
    /// Whose slot: 0 is the local player (the top roster row), 1..=5 the
    /// teammates top to bottom. Only 0 is read today.
    pub voter: u8,
    /// 1..=3, left to right (the vote's rank).
    pub slot: u8,
    /// `None` when the slot is none of the states clearly.
    pub state: Option<SlotState>,
    /// The voted hero's pack key. `Some` only when `state` is `Hero` and
    /// the icon matched one hero clearly.
    pub hero: Option<String>,
    /// 0..1 for `hero`; 0 when `hero` is `None`.
    pub confidence: f32,
    /// Best correlation score (diagnostic, 0 when no match was run).
    pub score: f32,
    /// Lead of the best over the second best (diagnostic).
    pub margin: f32,
}

impl VoteSlotRead {
    fn unread(voter: u8, slot: u8) -> Self {
        Self {
            voter,
            slot,
            state: None,
            hero: None,
            confidence: 0.0,
            score: 0.0,
            margin: 0.0,
        }
    }
}

/// One box of the TEAM BAN SCORE strip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BanScoreEntry {
    /// 1..=3, left to right (most votes first).
    pub rank: u8,
    /// The hero's pack key; `None` when the box shows no hero or the icon
    /// did not match one hero clearly.
    pub hero: Option<String>,
    /// The vote count under the box; `None` when it was not read.
    pub votes: Option<u32>,
    /// The box shows no hero yet (grey border, flat inside, a dash for
    /// the count).
    pub empty: bool,
    /// 0..1 for `hero`; 0 when `hero` is `None`.
    pub confidence: f32,
    /// 0..1 for `votes`; 0 when `votes` is `None`.
    pub votes_confidence: f32,
    /// Best correlation score of the hero match (diagnostic).
    pub score: f32,
    /// Lead of the best over the second best (diagnostic).
    pub margin: f32,
}

impl BanScoreEntry {
    fn unread(rank: u8) -> Self {
        Self {
            rank,
            hero: None,
            votes: None,
            empty: false,
            confidence: 0.0,
            votes_confidence: 0.0,
            score: 0.0,
            margin: 0.0,
        }
    }
}

/// The ban vote screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BanVote {
    /// The local player's three slots, in slot order. `None` when the
    /// frame does not show them where they belong.
    pub slots: Option<Vec<VoteSlotRead>>,
    /// The strip's three boxes, in rank order. `None` when the frame does
    /// not show the strip.
    pub score: Option<Vec<BanScoreEntry>>,
}

impl BanVote {
    /// The local player's votes as the flat list of reads the game record
    /// has always kept under `ban_vote`: one per slot, in slot order, the
    /// hero when one was read and an unread value otherwise (skipped,
    /// empty or not clear). Empty when the slots were not read.
    pub fn own_votes(&self) -> Vec<Read> {
        self.slots
            .iter()
            .flatten()
            .filter(|s| s.voter == 0)
            .map(|s| match (&s.state, &s.hero) {
                (Some(SlotState::Hero), Some(key)) => Read {
                    value: Some(Choice::Hero(key.clone())),
                    confidence: s.confidence,
                    score: s.score,
                    margin: s.margin,
                },
                _ => Read {
                    score: s.score,
                    margin: s.margin,
                    ..Read::unreadable()
                },
            })
            .collect()
    }
}

/// A hero match of one box.
struct Matched {
    /// The hero's key, when accepted and not the no-preference class.
    hero: Option<String>,
    /// The best class was the no-preference icon, accepted.
    no_preference: bool,
    confidence: f32,
    score: f32,
    margin: f32,
}

/// How a slot looks, before any match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Look {
    Skipped,
    Empty,
    Hero,
    Unclear,
}

impl Reader {
    /// The ban vote screen: the local player's three votes and the TEAM
    /// BAN SCORE strip. Pass the last frame. `None` only when the frame is
    /// not 2560x1440; a part the frame does not show is `None` inside.
    pub fn read_ban_vote(&self, img: &RgbImage) -> Option<BanVote> {
        if img.dimensions() != (FRAME_W, FRAME_H) {
            return None;
        }
        Some(BanVote {
            slots: self.read_vote_slots(img),
            score: self.read_ban_score(img),
        })
    }

    fn read_vote_slots(&self, img: &RgbImage) -> Option<Vec<VoteSlotRead>> {
        if !OWN_SLOTS.iter().all(|&r| box_in_place(img, r, SLOT_BORDER_MIN)) {
            return None;
        }
        let mut field: Option<Field> = None;
        let mut out = Vec::with_capacity(OWN_SLOTS.len());
        for (i, &r) in OWN_SLOTS.iter().enumerate() {
            let mut read = VoteSlotRead::unread(0, i as u8 + 1);
            match look(img, r) {
                Look::Skipped => read.state = Some(SlotState::Skipped),
                Look::Empty => read.state = Some(SlotState::Empty),
                Look::Hero => {
                    let field = field.get_or_insert_with(|| span_field(img, &OWN_SLOTS));
                    let m =
                        self.match_box(field, r, Use::VoteSlot, VOTE_MIN_SCORE, VOTE_MIN_MARGIN);
                    read.score = m.score;
                    read.margin = m.margin;
                    // The no-preference icon is not a vote: unread.
                    if !m.no_preference {
                        read.state = Some(SlotState::Hero);
                        read.confidence = m.confidence;
                        read.hero = m.hero;
                    }
                }
                Look::Unclear => {}
            }
            out.push(read);
        }
        Some(out)
    }

    fn read_ban_score(&self, img: &RgbImage) -> Option<Vec<BanScoreEntry>> {
        let title = share(img, STRIP_TITLE, is_red);
        let boxes = STRIP_BOXES.iter().all(|&r| box_in_place(img, r, STRIP_BORDER_MIN));
        if !(TITLE_RED_MIN..=TITLE_RED_MAX).contains(&title) || !boxes {
            return None;
        }
        let mut field: Option<Field> = None;
        let mut out = Vec::with_capacity(STRIP_BOXES.len());
        for (i, &r) in STRIP_BOXES.iter().enumerate() {
            let mut entry = BanScoreEntry::unread(i as u8 + 1);
            let ring = ring_shares(img, r, BORDER);
            let number = read_number(img, r.0 + COUNT_PAD, COUNT_Y0, r.2 - COUNT_PAD, COUNT_Y1);
            if ring.red >= BOX_COLOUR_MIN {
                let field = field.get_or_insert_with(|| span_field(img, &STRIP_BOXES));
                let m = self.match_box(field, r, Use::Strip, STRIP_MIN_SCORE, STRIP_MIN_MARGIN);
                entry.hero = m.hero;
                entry.confidence = m.confidence;
                entry.score = m.score;
                entry.margin = m.margin;
                if let Some(Number::Count { value, confidence }) = number {
                    entry.votes = Some(value);
                    entry.votes_confidence = confidence;
                }
            } else if ring.grey >= BOX_COLOUR_MIN {
                let inside = inside_stats(img, r);
                entry.empty = matches!(number, Some(Number::Dash)) && inside.std <= EMPTY_STD_MAX;
            }
            out.push(entry);
        }
        Some(out)
    }

    /// Match the icon inside box `r` (the border's outer edge) on the
    /// full-size field. Every offset is scored: the windows are small.
    fn match_box(
        &self,
        field: &Field,
        r: Rect,
        u: Use,
        min_score: f32,
        min_margin: f32,
    ) -> Matched {
        let (x0, y0, x1, y1) = r;
        // The box less its outermost pixel on every side.
        let win = field.native(x0 as i32 + 1, y0 as i32 + 1, x1 as i32, y1 as i32);
        let classes = self.templates.classes(u, true);
        let best = ranked(&win, &classes, SearchPlan::EXACT);
        let r3 = |v: f32| (v * 1000.0).round() / 1000.0;
        let Some(&(score, class)) = best.first() else {
            return Matched {
                hero: None,
                no_preference: false,
                confidence: 0.0,
                score: 0.0,
                margin: 0.0,
            };
        };
        let second = best.get(1).map_or(0.0, |r| r.0);
        let margin = score - second;
        let accepted = score >= min_score && margin >= min_margin;
        let no_preference = accepted && self.templates.key(class) == NO_PREFERENCE_KEY;
        let hero = (accepted && !no_preference).then(|| self.templates.key(class).to_string());
        let confidence =
            (0.5 * (score / 0.8).min(1.0) + 0.5 * (margin / 0.3).min(1.0)).clamp(0.0, 1.0);
        Matched {
            confidence: if hero.is_some() { r3(confidence) } else { 0.0 },
            hero,
            no_preference,
            score: r3(score),
            margin: r3(margin),
        }
    }
}

/// The full-size gradient field over a row of boxes.
fn span_field(img: &RgbImage, boxes: &[Rect; 3]) -> Field {
    let (x0, y0, _, _) = boxes[0];
    let (_, _, x1, y1) = boxes[2];
    Field::full(img, x0 as i32, y0 as i32, x1 as i32 + 1, y1 as i32 + 1)
}

fn spread(p: [u8; 3]) -> (u8, u8) {
    let [r, g, b] = p;
    (r.max(g).max(b), r.min(g).min(b))
}

/// The red of a voted slot, a hero box and the strip title.
fn is_red(p: [u8; 3]) -> bool {
    let [r, g, b] = p;
    r > 120 && r as i32 - g.max(b) as i32 > 80
}

/// The grey of an empty or skipped slot and an empty box.
fn is_grey(p: [u8; 3]) -> bool {
    let (hi, lo) = spread(p);
    hi > 80 && hi - lo < 40
}

/// A pixel of the skip icon: light and without colour (white on the own
/// slots, grey on a teammate's).
fn is_icon(p: [u8; 3]) -> bool {
    let (hi, lo) = spread(p);
    hi >= 90 && hi - lo <= 40
}

/// Shares of a box's border ring (`t` pixels thick) that are red, grey and
/// either.
struct RingShares {
    red: f32,
    grey: f32,
    either: f32,
}

fn ring_shares(img: &RgbImage, (x0, y0, x1, y1): Rect, t: u32) -> RingShares {
    let (mut n, mut red, mut grey, mut either) = (0u32, 0u32, 0u32, 0u32);
    for y in y0..=y1 {
        for x in x0..=x1 {
            if y < y0 + t || y + t > y1 || x < x0 + t || x + t > x1 {
                let p = img.get_pixel(x, y).0;
                let (r, g) = (is_red(p), is_grey(p));
                n += 1;
                red += r as u32;
                grey += g as u32;
                either += (r || g) as u32;
            }
        }
    }
    let of = |k: u32| if n == 0 { 0.0 } else { k as f32 / n as f32 };
    RingShares {
        red: of(red),
        grey: of(grey),
        either: of(either),
    }
}

/// Whether box `r` is where it belongs: a border ring at least `min`
/// red or grey, and the ring [`OUT_GAP`] pixels outside it not.
fn box_in_place(img: &RgbImage, r: Rect, min: f32) -> bool {
    let (x0, y0, x1, y1) = r;
    let out = (x0 - OUT_GAP, y0 - OUT_GAP, x1 + OUT_GAP, y1 + OUT_GAP);
    ring_shares(img, r, BORDER).either >= min && ring_shares(img, out, BORDER).either <= OUT_MAX
}

/// Share of the pixels of `r` that pass `test`.
fn share(img: &RgbImage, (x0, y0, x1, y1): Rect, test: fn([u8; 3]) -> bool) -> f32 {
    let mut n = 0u32;
    let mut k = 0u32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            n += 1;
            k += test(img.get_pixel(x, y).0) as u32;
        }
    }
    if n == 0 { 0.0 } else { k as f32 / n as f32 }
}

/// The inside of a box: grey standard deviation and share of icon pixels.
struct Inside {
    std: f32,
    icon: f32,
}

fn inside_stats(img: &RgbImage, (x0, y0, x1, y1): Rect) -> Inside {
    let (mut n, mut sum, mut sq, mut icon) = (0f64, 0f64, 0f64, 0u32);
    for y in (y0 + INSET)..=(y1 - INSET) {
        for x in (x0 + INSET)..=(x1 - INSET) {
            let p = img.get_pixel(x, y).0;
            // The grey of `imaging::gray_from_rgb`.
            let g = ((p[2] as u32 * 1868 + p[1] as u32 * 9617 + p[0] as u32 * 4899 + 8192) >> 14)
                as f64;
            n += 1.0;
            sum += g;
            sq += g * g;
            icon += is_icon(p) as u32;
        }
    }
    let mean = sum / n;
    Inside {
        std: (sq / n - mean * mean).max(0.0).sqrt() as f32,
        icon: icon as f32 / n as f32,
    }
}

/// The pixel nearest to `(x, y)`.
fn at(img: &RgbImage, x: f64, y: f64) -> [u8; 3] {
    img.get_pixel((x + 0.5).floor() as u32, (y + 0.5).floor() as u32)
        .0
}

/// Shares of icon pixels on the skip icon's ring, on its slash (lower left
/// to upper right) and on the other diagonal.
fn skip_marks(img: &RgbImage, (x0, y0, x1, y1): Rect) -> (f32, f32, f32) {
    let (cx, cy) = ((x0 + x1) as f64 / 2.0, (y0 + y1) as f64 / 2.0);
    let ring = (0..SKIP_POINTS)
        .filter(|&i| {
            let a = 2.0 * std::f64::consts::PI * i as f64 / SKIP_POINTS as f64;
            is_icon(at(img, cx + SKIP_RADIUS * a.cos(), cy + SKIP_RADIUS * a.sin()))
        })
        .count() as f32
        / SKIP_POINTS as f32;
    let steps: Vec<f64> = (0..13).map(|i| -9.0 + 1.5 * i as f64).collect();
    let on = |dx: f64| {
        steps
            .iter()
            .filter(|&&k| is_icon(at(img, cx + dx * k, cy + k)))
            .count() as f32
            / steps.len() as f32
    };
    (ring, on(-1.0), on(1.0))
}

/// How a slot looks. A skip is checked first: its icon is bright on a
/// flat inside, so it would never pass as empty, and a hero's art does not
/// draw a full ring with a slash.
fn look(img: &RgbImage, r: Rect) -> Look {
    let inside = inside_stats(img, r);
    let (ring, slash, backslash) = skip_marks(img, r);
    if ring >= SKIP_RING_MIN
        && slash >= SKIP_SLASH_MIN
        && backslash <= SKIP_BACKSLASH_MAX
        && (SKIP_SHARE_MIN..=SKIP_SHARE_MAX).contains(&inside.icon)
    {
        return Look::Skipped;
    }
    if inside.std <= EMPTY_STD_MAX && inside.icon <= EMPTY_ICON_MAX {
        return Look::Empty;
    }
    if ring_shares(img, r, BORDER).red >= HERO_RED_MIN && inside.std >= HERO_STD_MIN {
        return Look::Hero;
    }
    Look::Unclear
}
