//! The Games list row's Map cell in a narrow Map column: the name, the
//! unsure sign, then the chips ([`MapPart`]). Iced 0.14 text has no
//! ellipsis, so the cell measures its parts with the same text layout the
//! row uses (the renderer's paragraph and the bundled fonts) at the width
//! `responsive` gives it. A chip that fits shows unchanged. A chip that
//! does not is shortened to the letters that fit and "…", and the cell
//! gets a tooltip with the map, its mode and every shortened chip in full.
//! Nothing after the name is cut without an ellipsis.

use iced::advanced::graphics::text::Paragraph;
use iced::advanced::text::{self as core_text, Paragraph as _};
use iced::widget::text::{LineHeight, Shaping, Wrapping};
use iced::{Font, Pixels, Size, alignment};

use crate::games_detail::{MAP_CELL_GAP, MAP_WARN_W, MapPart, map_name_max_width};
use crate::theme::{FONT_SEMIBOLD, SIZE_LABEL, SIZE_META};

/// The chip's left and right padding together (`chip_box`'s 8 px a side).
pub const CHIP_PAD_X: f32 = 16.0;
/// The name's wrapping, the text widget's default.
const WRAP: Wrapping = Wrapping::Word;
/// What a shortened chip ends in.
pub const ELLIPSIS: &str = "…";

/// The Map cell as it fits: the parts to draw (chips shortened where
/// needed), the name's width cap, and the tooltip when a chip is
/// shortened or left out.
#[derive(Debug, Clone, PartialEq)]
pub struct MapFit {
    pub parts: Vec<MapPart>,
    pub name_max: Option<f32>,
    pub tip: Option<String>,
    /// The name shortened with "…" when even a wrapped name would leave the
    /// mode chip less than its [`stub`]; None draws the name in full.
    pub name: Option<String>,
}

/// Size of `s` laid out as the text widget lays it out in a `bounds` box.
fn measure(s: &str, size: f32, font: Font, bounds: Size, wrapping: Wrapping) -> Size {
    Paragraph::with_text(core_text::Text {
        content: s,
        bounds,
        size: Pixels(size),
        line_height: LineHeight::default(),
        font,
        align_x: core_text::Alignment::Default,
        align_y: alignment::Vertical::Top,
        shaping: Shaping::default(),
        wrapping,
    })
    .min_bounds()
}

/// A chip's whole width with `label` in it: the one-line label at
/// `SIZE_LABEL` semibold and the padding. A one-line label's width does
/// not depend on the box, so any box far wider than a chip will do.
pub fn chip_width(label: &str) -> f32 {
    let bounds = Size::new(10_000.0, 1_000.0);
    measure(label, SIZE_LABEL, FONT_SEMIBOLD, bounds, Wrapping::None).width + CHIP_PAD_X
}

/// The fewest letters a shortened chip keeps before its "…", so a chip
/// never shows a bare "…".
pub const MIN_KEPT: usize = 2;

/// `full` cut to its first `n` letters and "…". Spaces before the "…" are
/// dropped.
fn kept(full: &str, n: usize) -> String {
    let start: String = full.chars().take(n).collect();
    format!("{}{ELLIPSIS}", start.trim_end())
}

/// The shortest a chip may get: its first [`MIN_KEPT`] letters and "…".
pub fn stub(full: &str) -> String {
    kept(
        full,
        MIN_KEPT.min(full.chars().count().saturating_sub(1)).max(1),
    )
}

/// The longest start of `full` that fits `room` with "…" after it, and
/// never fewer than [`MIN_KEPT`] letters ([`stub`]).
fn shorten(full: &str, room: f32) -> String {
    let len = full.chars().count();
    let with = |n: usize| kept(full, n);
    // Widths grow with the letters kept, so search for the most that fit.
    let floor = MIN_KEPT.min(len.saturating_sub(1)).max(1);
    let (mut lo, mut hi) = (floor, len.saturating_sub(1).max(floor));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if chip_width(&with(mid)) <= room {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    with(lo)
}

/// The longest start of `name` that, with "…" after it, wraps within `cap`
/// and fits `height`; at least one letter.
fn shorten_name(name: &str, font: Font, cap: f32, height: f32) -> String {
    let fits = |n: usize| {
        let m = measure(
            &kept(name, n),
            SIZE_META,
            font,
            Size::new(cap, 1_000.0),
            WRAP,
        );
        m.height <= height && m.width <= cap
    };
    let (mut lo, mut hi) = (1, name.chars().count().saturating_sub(1).max(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    kept(name, lo)
}

/// The Map cell `cell` wide and `height` tall holding `parts`, the name in
/// `font`. `mode` is the map's mode chip, which the tooltip always names.
///
/// Iced lays a row out by taking every gap off first, then giving each
/// child in order the width the earlier ones left, so the name (capped
/// with the sign, [`map_name_max_width`]) and the sign come first and the
/// chips share what is left. A name that would leave less than a
/// [`stub`] chip (two letters and "…") for each chip wraps a little sooner
/// instead, as long as it still fits the row's height. Each chip leaves
/// room for a stub of every chip after it. When not even that fits, the
/// last chips are left out whole (the tooltip names them) and the last
/// chip shown ends in "…". When even the mode chip's stub does not fit
/// beside the wrapped name, the name is shortened with "…" instead. No
/// chip is ever a bare "…".
pub fn fit(
    cell: f32,
    height: f32,
    name: &str,
    font: Font,
    parts: &[MapPart],
    mode: Option<&str>,
) -> MapFit {
    let chips: Vec<&str> = parts
        .iter()
        .filter_map(|p| match p {
            MapPart::Chip(c) => Some(c.as_str()),
            _ => None,
        })
        .collect();
    let fixed: Vec<MapPart> = parts
        .iter()
        .filter(|p| !matches!(p, MapPart::Chip(_)))
        .cloned()
        .collect();
    let warn = fixed.contains(&MapPart::Warn);
    let stubs: Vec<f32> = chips.iter().map(|c| chip_width(&stub(c))).collect();
    for shown in (0..=chips.len()).rev() {
        let mut with: Vec<MapPart> = fixed.clone();
        with.extend(chips[..shown].iter().map(|c| MapPart::Chip(c.to_string())));
        let gaps = MAP_CELL_GAP * with.len().saturating_sub(1) as f32;
        let sign = if warn { MAP_WARN_W } else { 0.0 };
        // The widest the row lets the name be, with the sign's cap.
        let name_room = map_name_max_width(cell, &with).unwrap_or((cell - gaps).max(0.0));
        let reserve: f32 = stubs[..shown].iter().sum();
        let lay = |room: f32| measure(name, SIZE_META, font, Size::new(room, height), WRAP);
        let tall = |room: f32| measure(name, SIZE_META, font, Size::new(room, 1_000.0), WRAP);
        let mut name_max = map_name_max_width(cell, &with);
        let mut short_name = None;
        let mut name_w = lay(name_room).width;
        if shown > 0 && cell - gaps - name_w - sign < reserve {
            // Cap the name to leave the chips their ellipsis, if it still
            // fits the row's height wrapped at that cap.
            // A word wider than the cap would run past it.
            let cap = (cell - gaps - sign - reserve).max(0.0);
            let need = tall(cap);
            name_max = Some(cap);
            if need.height > height || need.width > cap {
                if shown > 1 {
                    continue;
                }
                let short = shorten_name(name, font, cap, height);
                name_w = measure(&short, SIZE_META, font, Size::new(cap, height), WRAP).width;
                short_name = Some(short);
            } else {
                name_w = lay(cap).width;
            }
        }
        let mut left = cell - gaps - name_w - sign;
        let mut out = fixed.clone();
        let mut cut = Vec::new();
        for (i, full) in chips[..shown].iter().enumerate() {
            let room = left - stubs[i + 1..shown].iter().sum::<f32>();
            let last_before_dropped = i + 1 == shown && shown < chips.len();
            let label = if chip_width(full) <= room && !last_before_dropped {
                full.to_string()
            } else {
                cut.push(*full);
                shorten(full, room)
            };
            left -= chip_width(&label);
            out.push(MapPart::Chip(label));
        }
        cut.extend(&chips[shown..]);
        let tip = (!cut.is_empty() || short_name.is_some()).then(|| {
            let mut words = vec![name.to_string()];
            words.extend(mode.map(str::to_string));
            words.extend(
                cut.iter()
                    .filter(|c| Some(**c) != mode)
                    .map(|c| c.to_string()),
            );
            words.join(", ")
        });
        return MapFit {
            parts: out,
            name_max,
            tip,
            name: short_name,
        };
    }
    unreachable!("no chips always fits")
}
