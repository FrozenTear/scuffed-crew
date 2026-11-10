//! The roster screen (both teams side by side under their white team
//! headers), shown after the ban results and before the hero select. Pixel
//! check only, no OCR. Used only by the opt-in pre-game saves
//! (`debug_save_pregame_screens`); nothing else reads it.
//!
//! All geometry is in shares of the 16:9 game area, so any resolution
//! measures alike. Only the headers and the column edges under them are
//! read, never the player rows, so 5v5 and 6v6 rosters both match.
//!
//! Definitions:
//!
//! * chroma: `max(r, g, b) - min(r, g, b)`;
//! * near-white: `min(r, g, b) >= 150` and chroma `<= 40`. An absolute
//!   floor, low enough that a fading-in roster still counts (see below);
//! * coloured: chroma `> 55`, any hue (team colours and colourblind
//!   filters vary).
//!
//! The rule, both parts must pass:
//!
//! * header: two near-white bars, searched over y 0.15 to 0.31 (a 5v5
//!   header sits at about 0.236 to 0.278, a 6v6 one at about 0.19 to
//!   0.235). The left bar is sampled over x 0.067 to 0.361, the right one
//!   over x 0.64 to 0.93. A row is a bar row when at least 75% of its
//!   samples across that span are near-white; a bar is the longest run of
//!   bar rows. Each bar must be 0.02 to 0.065 tall and their tops at most
//!   0.015 apart. Then six parts are scored in 0 to 1: each bar's
//!   coverage (the mean near-white share of its rows), and how clean
//!   (1 minus the near-white share) the area is above the bars (0.005 to
//!   0.025 over the higher top, both spans), below them (0.005 to 0.025
//!   under the lower bottom, both spans), in the gap between them (x 0.361
//!   to 0.64 at bar height) and outside them (x 0 to 0.067 and 0.93 to 1
//!   at bar height). The header score is the lowest of those, 0 when the
//!   bars are missing, too short or tall, or not level, and must be at
//!   least 0.25. The coverage is 0.75 or more by construction, so the
//!   score really gates the cleanliness: each clean area may be at most
//!   75% near-white. Loose on purpose, since the bars' inner ends and
//!   their labels reach into the gap; a full-width white band, a white
//!   frame or a bright sky edge still fail it.
//! * colour strips: the coloured share of the role strip on each column's
//!   inner edge (x 0.386 to 0.404 and 0.596 to 0.614), on the band from
//!   0.006 to 0.050 under the lower bar's bottom, which the first player
//!   row fills for any team size. The strip score is the lower of the two
//!   and must be at least 0.30. With no bar found it is 0.
//!
//! Fade-in: the roster fades in, every channel scaled by a share `f` of
//! full brightness. Bars drawn at 246 (their lowest channel) stay
//! near-white while `246 f` rounds to 150 or more, so down to `f` of
//! 0.608 (149.5 / 246); the tests pass it at 0.7 and fail it at 0.55.
//! Chroma scales the same way, so a strip of chroma `c` counts while
//! `c f > 55`: a strip of chroma 123 down to about 0.45, under the
//! header's limit, so the header sets it.

use image::RgbImage;

/// The header search rows, as shares of the height.
const HEADER_SCAN: (f32, f32) = (0.15, 0.31);
/// The two bars' sampled spans across.
const BAR_LEFT: (f32, f32) = (0.067, 0.361);
const BAR_RIGHT: (f32, f32) = (0.64, 0.93);
/// A row is a bar row when at least this share of its samples is near-white.
const BAR_ROW_WHITE: f32 = 0.75;
/// A bar's height (shares of the height): a white frame's band is taller.
const BAR_HEIGHT: (f32, f32) = (0.02, 0.065);
/// The two bars' tops are at most this far apart.
const BAR_LEVEL: f32 = 0.015;
/// The clean bands above and below the bars: this far off them, this tall.
/// The offset keeps a blurred bar edge out of them.
const CLEAN_OFFSET: f32 = 0.005;
const CLEAN_HEIGHT: f32 = 0.02;
/// Between the bars, at bar height.
const GAP: (f32, f32) = (0.361, 0.64);
/// Outside the bars, at bar height.
const OUTSIDE_LEFT: (f32, f32) = (0.0, 0.067);
const OUTSIDE_RIGHT: (f32, f32) = (0.93, 1.0);
/// The lowest header score that passes.
const HEADER_MIN: f32 = 0.25;

/// The role strips on the columns' inner edges.
const STRIP_LEFT: (f32, f32) = (0.386, 0.404);
const STRIP_RIGHT: (f32, f32) = (0.596, 0.614);
/// The strip band starts this far under the lower bar and is this tall.
const STRIP_OFFSET: f32 = 0.006;
const STRIP_HEIGHT: f32 = 0.044;
/// A strip pixel is coloured above this chroma.
const STRIP_CHROMA: u8 = 55;
/// The lowest strip score (the lower strip's coloured share) that passes.
const STRIP_MIN: f32 = 0.30;

/// Near-white: the lowest channel at least this, the chroma at most this.
const WHITE_MIN: u8 = 150;
const WHITE_CHROMA_MAX: u8 = 40;

/// What the roster check measured.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RosterScores {
    /// The header score (the lowest part, 0 with no valid bar pair).
    pub header: f32,
    /// The bars' heights (shares of the height), 0 with none.
    pub bar_left: f32,
    pub bar_right: f32,
    /// The bars' mean near-white share over their rows, 0 with none.
    pub cover_left: f32,
    pub cover_right: f32,
    /// The bars' tops are at most 0.015 apart.
    pub bars_level: bool,
    /// 1 minus the near-white share above, below, between and outside the
    /// bars; 0 unless both bars were found.
    pub clean_above: f32,
    pub clean_below: f32,
    pub clean_gap: f32,
    pub clean_outside: f32,
    /// The coloured share of the left and right role strips.
    pub strip_left: f32,
    pub strip_right: f32,
}

impl RosterScores {
    pub fn header_passes(&self) -> bool {
        self.header >= HEADER_MIN
    }

    /// The strip score: the lower of the two strips.
    pub fn strips(&self) -> f32 {
        self.strip_left.min(self.strip_right)
    }

    pub fn strips_pass(&self) -> bool {
        self.strips() >= STRIP_MIN
    }

    /// Both the header and the strips pass.
    pub fn is_roster(&self) -> bool {
        self.header_passes() && self.strips_pass()
    }

    /// One part passed but the frame is no roster: a turned-down candidate
    /// worth a look. A frame with no part passing is no candidate.
    pub fn near_miss(&self) -> bool {
        (self.header_passes() || self.strips_pass()) && !self.is_roster()
    }

    /// The scores and the verdict, rounded, for a debug line.
    pub fn describe(&self) -> String {
        format!(
            "header {} {:.2} (bars {:.3}/{:.3}, cover {:.2}/{:.2}, level {}, clean above {:.2} below {:.2} gap {:.2} outside {:.2}), strips {} {:.2} ({:.2}, {:.2}), roster {}",
            self.header_passes(),
            self.header,
            self.bar_left,
            self.bar_right,
            self.cover_left,
            self.cover_right,
            self.bars_level,
            self.clean_above,
            self.clean_below,
            self.clean_gap,
            self.clean_outside,
            self.strips_pass(),
            self.strips(),
            self.strip_left,
            self.strip_right,
            self.is_roster(),
        )
    }
}

fn chroma(p: [u8; 3]) -> u8 {
    p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2])
}

fn white(p: [u8; 3]) -> bool {
    p[0].min(p[1]).min(p[2]) >= WHITE_MIN && chroma(p) <= WHITE_CHROMA_MAX
}

fn coloured(p: [u8; 3]) -> bool {
    chroma(p) > STRIP_CHROMA
}

/// A header bar: top and bottom as shares of the height, and its coverage.
#[derive(Clone, Copy)]
struct Bar {
    top: f32,
    bottom: f32,
    coverage: f32,
}

/// The 16:9 game area and share-to-pixel maps.
struct Area {
    gx: u32,
    gy: u32,
    gw: u32,
    gh: u32,
}

impl Area {
    fn x(&self, share: f32) -> u32 {
        self.gx + (share.clamp(0.0, 1.0) * self.gw as f32) as u32
    }

    fn y(&self, share: f32) -> u32 {
        self.gy + (share.clamp(0.0, 1.0) * self.gh as f32) as u32
    }

    /// A pixel row as a share of the height.
    fn row_share(&self, y: u32) -> f32 {
        (y - self.gy) as f32 / self.gh as f32
    }

    /// The share of sampled pixels (every second each way) over all the
    /// boxes (x0, y0, x1, y1) for which `hit` holds; 0 with no sample.
    fn share(&self, rgb: &RgbImage, boxes: &[[f32; 4]], hit: fn([u8; 3]) -> bool) -> f32 {
        let (mut n, mut total) = (0u32, 0u32);
        for b in boxes {
            let (x0, x1) = (self.x(b[0]), self.x(b[2]).min(rgb.width()));
            let (y0, y1) = (self.y(b[1]), self.y(b[3]).min(rgb.height()));
            for y in (y0..y1).step_by(2) {
                for x in (x0..x1).step_by(2) {
                    total += 1;
                    n += u32::from(hit(rgb.get_pixel(x, y).0));
                }
            }
        }
        if total == 0 {
            0.0
        } else {
            n as f32 / total as f32
        }
    }

    /// The longest run of bar rows in [`HEADER_SCAN`] over the span `xs`,
    /// `None` with no bar row.
    fn bar(&self, rgb: &RgbImage, xs: (f32, f32)) -> Option<Bar> {
        let (x0, x1) = (self.x(xs.0), self.x(xs.1).min(rgb.width()));
        let (y0, y1) = (
            self.y(HEADER_SCAN.0),
            self.y(HEADER_SCAN.1).min(rgb.height()),
        );
        // (top, last row, sum of the rows' white shares).
        let mut best: Option<(u32, u32, f32)> = None;
        let mut run: Option<(u32, f32)> = None;
        for y in y0..y1 {
            let (mut n, mut total) = (0u32, 0u32);
            for x in (x0..x1).step_by(2) {
                total += 1;
                n += u32::from(white(rgb.get_pixel(x, y).0));
            }
            let row = if total == 0 {
                0.0
            } else {
                n as f32 / total as f32
            };
            if row >= BAR_ROW_WHITE {
                let entry = run.get_or_insert((y, 0.0));
                entry.1 += row;
                let (top, sum) = *entry;
                if best.is_none_or(|(t, last, _)| y - top > last - t) {
                    best = Some((top, y, sum));
                }
            } else {
                run = None;
            }
        }
        // A bar of one row is one row high, not none.
        best.map(|(t, last, sum)| Bar {
            top: self.row_share(t),
            bottom: self.row_share(last + 1),
            coverage: sum / (last + 1 - t) as f32,
        })
    }
}

/// Measure the roster parts on `rgb`.
pub fn scores(rgb: &RgbImage) -> RosterScores {
    let (w, h) = rgb.dimensions();
    let (gx, gy, gw, gh) = crate::frame::geometry::game_rect_16_9(w, h);
    if gw < 16 || gh < 16 {
        return RosterScores::default();
    }
    let area = Area { gx, gy, gw, gh };
    let left = area.bar(rgb, BAR_LEFT);
    let right = area.bar(rgb, BAR_RIGHT);
    let mut s = RosterScores::default();
    if let Some(b) = left {
        s.bar_left = b.bottom - b.top;
        s.cover_left = b.coverage;
    }
    if let Some(b) = right {
        s.bar_right = b.bottom - b.top;
        s.cover_right = b.coverage;
    }
    if let (Some(l), Some(r)) = (left, right) {
        s.bars_level = (l.top - r.top).abs() <= BAR_LEVEL;
        let (top, bottom) = (l.top.min(r.top), l.bottom.max(r.bottom));
        let clean = |boxes: &[[f32; 4]]| 1.0 - area.share(rgb, boxes, white);
        let (a0, a1) = (top - CLEAN_OFFSET - CLEAN_HEIGHT, top - CLEAN_OFFSET);
        let (b0, b1) = (bottom + CLEAN_OFFSET, bottom + CLEAN_OFFSET + CLEAN_HEIGHT);
        s.clean_above = clean(&[
            [BAR_LEFT.0, a0, BAR_LEFT.1, a1],
            [BAR_RIGHT.0, a0, BAR_RIGHT.1, a1],
        ]);
        s.clean_below = clean(&[
            [BAR_LEFT.0, b0, BAR_LEFT.1, b1],
            [BAR_RIGHT.0, b0, BAR_RIGHT.1, b1],
        ]);
        s.clean_gap = clean(&[[GAP.0, top, GAP.1, bottom]]);
        s.clean_outside = clean(&[
            [OUTSIDE_LEFT.0, top, OUTSIDE_LEFT.1, bottom],
            [OUTSIDE_RIGHT.0, top, OUTSIDE_RIGHT.1, bottom],
        ]);
        let tall = |h: f32| (BAR_HEIGHT.0..=BAR_HEIGHT.1).contains(&h);
        if tall(s.bar_left) && tall(s.bar_right) && s.bars_level {
            s.header = [
                s.cover_left,
                s.cover_right,
                s.clean_above,
                s.clean_below,
                s.clean_gap,
                s.clean_outside,
            ]
            .into_iter()
            .fold(1.0, f32::min);
        }
    }
    // The strips hang under the lower bar; with no bar there is nowhere
    // to look, and a strip-only pass on gameplay would be noise.
    let bottom = [left, right]
        .into_iter()
        .flatten()
        .map(|b| b.bottom)
        .reduce(f32::max);
    if let Some(bottom) = bottom {
        let (y0, y1) = (bottom + STRIP_OFFSET, bottom + STRIP_OFFSET + STRIP_HEIGHT);
        let strip = |xs: (f32, f32)| area.share(rgb, &[[xs.0, y0, xs.1, y1]], coloured);
        s.strip_left = strip(STRIP_LEFT);
        s.strip_right = strip(STRIP_RIGHT);
    }
    s
}

/// Whether `rgb` looks like the roster screen ([`RosterScores::is_roster`]).
pub fn is_roster_screen(rgb: &RgbImage) -> bool {
    scores(rgb).is_roster()
}

#[cfg(test)]
#[path = "tests_roster_rule.rs"]
mod tests_roster_rule;
