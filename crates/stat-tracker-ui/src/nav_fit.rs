//! How the 0.6 top bar fits a window width.
//!
//! The bar draws with the sizes and gaps below, and [`layout`] adds up the
//! same parts (text widths from the bundled Urbanist advances) to pick the
//! fullest form that fits on one line: tab icons, the long status with
//! "last game", and the full "need a look" chip. It drops the long status
//! first, then the icons, then the chip words. When even that does not fit
//! (from the 720 px window minimum to about 860 px) the bar takes two
//! lines: title and status, then the tabs.

use crate::model::Screen;
use crate::tracker_status::HeaderStatus;

/// Brand size (spec: 16 px, extra bold).
pub const SIZE_BRAND: f32 = 16.0;
/// Tab labels.
pub const SIZE_TAB: f32 = 14.0;
/// Count inside the Games tab pill.
pub const SIZE_BADGE: f32 = 11.0;
/// Status words and time (spec: about 12 px).
pub const SIZE_STATUS: f32 = 12.0;
/// Chip words (spec: 12 px bold).
pub const SIZE_CHIP: f32 = 12.0;

/// Title to the first tab's icon below [`crate::theme::WIDE_GUTTER_MIN`].
pub const TITLE_GAP: f32 = 24.0;
/// Title to the first tab's icon on wide windows (spec p1: 40 px).
pub const TITLE_GAP_WIDE: f32 = 40.0;
/// Side padding inside a tab with icons. With [`TAB_SPACING`] the gap
/// between two tabs' contents is 26 px (spec: about 30, with labels and
/// icons a little narrower), so the spec bar still fits at 1024.
pub const TAB_PAD: f32 = 11.0;
/// Side padding inside a tab without icons.
pub const TAB_PAD_PLAIN: f32 = 8.0;
/// Space between two tabs.
pub const TAB_SPACING: f32 = 4.0;
/// Tab icon box and the gap to its label.
pub const ICON: f32 = 16.0;
pub const ICON_GAP: f32 = 8.0;
/// The Games count pill's side padding. It sits [`ICON_GAP`] after the
/// label.
pub const BADGE_PAD_X: f32 = 9.0;
/// Status dot, the gap to its words, and the gap before the time.
pub const DOT: f32 = 8.0;
pub const DOT_GAP: f32 = 8.0;
pub const TIME_GAP: f32 = 5.0;
/// Status words to the chip.
pub const CHIP_GAP: f32 = 16.0;
/// Chip side padding, warning sign size, and the gaps around the words.
pub const CHIP_PAD_X: f32 = 10.0;
pub const WARN_ICON: f32 = 12.0;
pub const CHIP_GLYPH_GAP: f32 = 4.0;
/// The chip's closing sign.
pub const CHEVRON: &str = "›";
/// Least room kept between the last tab and the status.
pub const MIN_GAP: f32 = 8.0;

/// The form the bar takes at one width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarLayout {
    /// Title, tabs and status on one line; otherwise tabs on a second line.
    pub one_line: bool,
    pub icons: bool,
    /// "Tracker ready · last game 00:40" rather than "Ready".
    pub long_status: bool,
    /// "⚠ 159 ›" rather than "⚠ 159 need a look ›".
    pub compact_chip: bool,
    /// Side gutter, the same as the page under the bar.
    pub gutter: f32,
}

/// Text weight for [`text_width`]. `Heavy` covers semibold to extra bold
/// with the extra bold advances, so it never comes out short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Medium,
    Heavy,
}

/// Advance widths in 1/1000 em of printable ASCII (space to `~`) in the
/// bundled Urbanist Medium and ExtraBold, rounded up.
const MEDIUM: [u16; 95] = [
    240, 198, 274, 768, 596, 686, 711, 136, 344, 344, 356, 580, 222, 440, 198, 373, 599, 291, 556,
    499, 559, 562, 543, 541, 534, 536, 198, 225, 580, 580, 580, 522, 886, 697, 570, 699, 672, 541,
    481, 749, 638, 241, 419, 554, 511, 905, 649, 750, 536, 781, 577, 552, 630, 670, 682, 879, 685,
    626, 646, 290, 373, 290, 427, 620, 432, 608, 609, 510, 609, 559, 346, 598, 545, 211, 230, 509,
    221, 889, 545, 560, 608, 608, 369, 432, 350, 525, 541, 829, 558, 539, 507, 405, 231, 405, 628,
];
const EXTRABOLD: [u16; 95] = [
    240, 240, 306, 797, 609, 792, 718, 156, 398, 398, 365, 580, 270, 440, 241, 422, 607, 352, 581,
    518, 590, 582, 553, 555, 551, 556, 241, 271, 580, 580, 580, 571, 934, 746, 597, 699, 672, 554,
    485, 749, 663, 289, 458, 604, 531, 952, 681, 750, 564, 790, 623, 565, 630, 686, 780, 962, 745,
    683, 646, 338, 422, 339, 508, 620, 561, 625, 626, 510, 626, 560, 375, 622, 565, 259, 251, 550,
    269, 889, 565, 560, 625, 625, 394, 436, 363, 545, 604, 866, 618, 592, 507, 413, 279, 413, 666,
];

/// The brand at [`SIZE_BRAND`], measured with kerning from the bundled
/// ExtraBold and rounded up.
pub const BRAND_WIDTH: f32 = 116.0;

/// A tab label at [`SIZE_TAB`] in ExtraBold (the width every label
/// reserves), measured with kerning and rounded up to half a pixel.
pub fn tab_label_width(screen: Screen) -> f32 {
    match screen {
        Screen::Overview => 62.5,
        Screen::Games => 45.5,
        Screen::Heroes => 43.5,
        Screen::Maps => 36.0,
        Screen::Seasons => 52.0,
        Screen::Settings => 51.5,
    }
}

/// Width of `s` at `size` px, from the advances above. "·" and "›" are
/// measured too; any other character counts as a wide letter.
pub fn text_width(s: &str, size: f32, weight: Weight) -> f32 {
    let table = match weight {
        Weight::Medium => &MEDIUM,
        Weight::Heavy => &EXTRABOLD,
    };
    let units: u32 = s
        .chars()
        .map(|c| match c {
            ' '..='~' => u32::from(table[c as usize - 0x20]),
            '·' => 221,
            '›' => 400,
            _ => 1000,
        })
        .sum();
    units as f32 / 1000.0 * size
}

/// Width of the six tabs, from the first label (its left padding dropped,
/// so the tabs start where the title gap ends) to the last tab's right
/// padding. Tab labels reserve their bold width.
pub fn tabs_width(icons: bool, count: usize) -> f32 {
    let pad = if icons { TAB_PAD } else { TAB_PAD_PLAIN };
    let screens = Screen::all();
    let mut total = pad * (2 * screens.len() - 1) as f32;
    total += TAB_SPACING * (screens.len() - 1) as f32;
    for screen in screens {
        if icons {
            total += ICON + ICON_GAP;
        }
        total += tab_label_width(screen);
        if screen == Screen::Games && count > 0 {
            total += ICON_GAP + badge_width(count);
        }
    }
    total
}

/// The Games tab's count pill.
pub fn badge_width(count: usize) -> f32 {
    text_width(&count.to_string(), SIZE_BADGE, Weight::Heavy) + 2.0 * BADGE_PAD_X
}

/// Dot, words, and the time when there is one.
pub fn status_width(label: &str, time: Option<&str>) -> f32 {
    DOT + DOT_GAP
        + text_width(label, SIZE_STATUS, Weight::Medium)
        + time.map_or(0.0, |t| {
            TIME_GAP + text_width(t, SIZE_STATUS, Weight::Heavy)
        })
}

/// The "need a look" chip, border included.
pub fn chip_width(count: usize, compact: bool) -> f32 {
    2.0 * CHIP_PAD_X
        + WARN_ICON
        + 2.0 * CHIP_GLYPH_GAP
        + text_width(
            &crate::nav::chip_text(count, compact),
            SIZE_CHIP,
            Weight::Heavy,
        )
        + text_width(CHEVRON, SIZE_CHIP, Weight::Heavy)
}

/// Status plus, with games to look at, the chip.
fn right_width(
    status: HeaderStatus,
    last_game: Option<&str>,
    count: usize,
    long: bool,
    compact: bool,
) -> f32 {
    let (label, time) = crate::nav::bar_status(status, last_game, long);
    let chip = if count > 0 {
        CHIP_GAP + chip_width(count, compact)
    } else {
        0.0
    };
    status_width(&label, time.as_deref()) + chip
}

/// Width the one-line bar needs at window `width` in the form given.
pub fn one_line_width(
    width: f32,
    form: BarLayout,
    status: HeaderStatus,
    last_game: Option<&str>,
    count: usize,
) -> f32 {
    let title_gap = if crate::theme::page_gutter(width) > TITLE_GAP {
        TITLE_GAP_WIDE
    } else {
        TITLE_GAP
    };
    2.0 * form.gutter
        + BRAND_WIDTH
        + title_gap
        + tabs_width(form.icons, count)
        + MIN_GAP
        + right_width(
            status,
            last_game,
            count,
            form.long_status,
            form.compact_chip,
        )
}

/// First line of the two-line bar: title, room, status and chip.
pub fn first_line_width(gutter: f32, right: f32) -> f32 {
    2.0 * gutter + BRAND_WIDTH + MIN_GAP + right
}

/// The fullest bar form that fits `width`.
pub fn layout(
    width: f32,
    status: HeaderStatus,
    last_game: Option<&str>,
    count: usize,
) -> BarLayout {
    let width = if width.is_finite() { width } else { 0.0 };
    let gutter = crate::theme::page_gutter(width);
    let icons_allowed = width >= crate::nav::TAB_ICONS_MIN;
    let long_allowed = width >= crate::nav::STATUS_LONG_MIN;
    let form = |icons, long_status, compact_chip| BarLayout {
        one_line: true,
        icons,
        long_status,
        compact_chip,
        gutter,
    };
    let tries = [
        form(icons_allowed, long_allowed, false),
        form(icons_allowed, false, false),
        form(false, false, false),
        form(false, false, true),
    ];
    if let Some(fit) = tries
        .into_iter()
        .find(|f| one_line_width(width, *f, status, last_game, count) <= width)
    {
        return fit;
    }
    let full = right_width(status, last_game, count, false, false);
    BarLayout {
        one_line: false,
        icons: false,
        long_status: false,
        compact_chip: first_line_width(gutter, full) > width,
        gutter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker_status::ALL;

    /// Window widths from the 720 minimum to wide, with every breakpoint
    /// and the pixel under it.
    fn widths() -> Vec<f32> {
        let mut out: Vec<f32> = (720..=1700).step_by(10).map(|w| w as f32).collect();
        for edge in [
            crate::nav::TAB_ICONS_MIN,
            crate::nav::STATUS_LONG_MIN,
            crate::theme::WIDE_GUTTER_MIN,
        ] {
            out.push(edge - 1.0);
            out.push(edge);
        }
        out.extend([1024.0, 1280.0, 1366.0, 1920.0]);
        out
    }

    #[test]
    fn widths_follow_the_fonts() {
        // Measured from the bundled TTFs with kerning; the table sums
        // may only come out the same or a little wider.
        let cases = [
            ("Scuffed Tracker", SIZE_BRAND, Weight::Heavy, 115.5),
            ("Overview", SIZE_TAB, Weight::Heavy, 62.3),
            ("Ready", 13.0, Weight::Medium, 36.7),
            ("Tracker ready · last game", 13.0, Weight::Medium, 147.7),
        ];
        for (s, size, weight, measured) in cases {
            let w = text_width(s, size, weight);
            assert!(w >= measured - 0.5 && w <= measured * 1.04, "{s}: {w}");
        }
        // The measured constants sit just under the table sums.
        let brand = text_width(crate::nav::BRAND, SIZE_BRAND, Weight::Heavy);
        assert!(BRAND_WIDTH <= brand && BRAND_WIDTH >= brand * 0.95);
        for screen in Screen::all() {
            let sum = text_width(screen.label(), SIZE_TAB, Weight::Heavy);
            let w = tab_label_width(screen);
            assert!(w <= sum && w >= sum * 0.95, "{screen:?}: {w} vs {sum}");
        }
    }

    #[test]
    fn the_chosen_bar_fits_every_width_status_and_count() {
        let times = [None, Some("00:40"), Some("Oct 18, 23:14")];
        for width in widths() {
            for status in ALL {
                for time in times {
                    for count in [0, 1, 12, 159] {
                        let fit = layout(width, status, time, count);
                        if fit.one_line {
                            let need = one_line_width(width, fit, status, time, count);
                            assert!(need <= width, "{width} {status:?} {count}: {need}");
                        } else {
                            let right = right_width(status, time, count, false, fit.compact_chip);
                            assert!(
                                first_line_width(fit.gutter, right) <= width,
                                "{width} {status:?} {count}"
                            );
                            assert!(
                                2.0 * fit.gutter + tabs_width(false, count) <= width,
                                "tabs at {width}"
                            );
                        }
                        assert_eq!(fit.gutter, crate::theme::page_gutter(width));
                    }
                }
            }
        }
    }

    #[test]
    fn spec_widths_keep_the_spec_bar() {
        // Spec p8: one line with icons, "Ready" and a 3-digit chip at 1024.
        let at_1024 = layout(1024.0, HeaderStatus::Waiting, Some("00:40"), 159);
        assert!(at_1024.one_line && at_1024.icons && !at_1024.compact_chip);
        assert!(!at_1024.long_status);
        // Spec p1: the long status with the time at 1600.
        let at_1600 = layout(1600.0, HeaderStatus::Waiting, Some("00:40"), 159);
        assert!(at_1600.one_line && at_1600.icons && at_1600.long_status);
        assert!(!at_1600.compact_chip);
        // The long status from the breakpoint, with a 3-digit chip.
        let at_long = layout(
            crate::nav::STATUS_LONG_MIN,
            HeaderStatus::Waiting,
            Some("00:40"),
            159,
        );
        assert!(at_long.long_status && at_long.icons);
        // No icons under the spec's 960 step, one line down to about 860.
        assert!(!layout(959.0, HeaderStatus::Waiting, None, 0).icons);
        assert!(layout(900.0, HeaderStatus::Waiting, Some("00:40"), 1).one_line);
        // The window minimum takes two lines.
        assert!(!layout(720.0, HeaderStatus::Waiting, Some("00:40"), 159).one_line);
    }

    #[test]
    fn the_bar_drops_the_long_status_before_the_icons() {
        for width in widths() {
            let fit = layout(
                width,
                HeaderStatus::TokenRejected,
                Some("Oct 18, 23:14"),
                159,
            );
            if fit.long_status {
                assert!(fit.icons || width < crate::nav::TAB_ICONS_MIN);
            }
            if fit.compact_chip && fit.one_line {
                assert!(!fit.icons && !fit.long_status);
            }
        }
    }
}
