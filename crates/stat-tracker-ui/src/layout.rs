//! Responsive column counts from available content width (design rev 3).
//!
//! Cards stay at least ~[`MIN_CARD`] wide. Games and maps start at 2–4
//! columns, heroes at 4–6. On an ultrawide window they add a column once a
//! card would pass [`MAX_CARD`] / [`MAX_HERO_CARD`], so a card does not turn
//! into a long empty bar. Tonight is one featured card plus as many compact
//! cards as fit at that minimum. Settings cards stay between
//! [`MIN_SETTINGS_CARD`] and [`MAX_SETTINGS_CARD`] in 1–4 columns (G4).

use crate::theme::{GRID_GAP, PAGE_PAD_X, SIDEBAR_WIDTH};

/// Minimum card width used when deciding how many columns fit.
pub const MIN_CARD: f32 = 300.0;
/// Widest a games / maps / seasons card gets before the grid adds a column.
pub const MAX_CARD: f32 = 640.0;
/// Widest a hero card gets before the grid adds a column.
pub const MAX_HERO_CARD: f32 = 480.0;

/// Sidebar is fixed; only the content pane flexes.
pub fn content_width_for_window(window_width: f32) -> f32 {
    (window_width - SIDEBAR_WIDTH - PAGE_PAD_X * 2.0).max(0.0)
}

/// How many `min_card`-wide cells fit in `available` with `gap` between them.
pub fn columns_fit(available: f32, min_card: f32, gap: f32) -> usize {
    if !available.is_finite() || available <= 0.0 || min_card <= 0.0 {
        return 1;
    }
    let n = ((available + gap) / (min_card + gap)).floor() as usize;
    n.max(1)
}

/// Fewest columns that keep each cell at most `max_card` wide.
pub fn columns_needed(available: f32, max_card: f32, gap: f32) -> usize {
    if !available.is_finite() || available <= 0.0 || max_card <= 0.0 {
        return 1;
    }
    let n = ((available + gap) / (max_card + gap)).ceil() as usize;
    n.max(1)
}

/// Width of one cell when `cols` equal cells share `available`.
pub fn cell_width(available: f32, cols: usize, gap: f32) -> f32 {
    let cols = cols.max(1) as f32;
    ((available - gap * (cols - 1.0)) / cols).max(0.0)
}

/// `cols` cells of `card` plus the gaps between them.
pub fn span(cols: usize, card: f32, gap: f32) -> f32 {
    let cols = cols.max(1) as f32;
    cols * card + (cols - 1.0) * gap
}

/// What fits at `min_card`, clamped to `lo..=hi`, plus extra columns when a
/// card would pass `max_card` (never more than fit at `min_card`).
fn grid_columns(available: f32, min_card: f32, max_card: f32, lo: usize, hi: usize) -> usize {
    let fit = columns_fit(available, min_card, GRID_GAP);
    let needed = columns_needed(available, max_card, GRID_GAP).min(fit);
    fit.clamp(lo, hi).max(needed)
}

pub fn games_columns(available: f32) -> usize {
    grid_columns(available, MIN_CARD, MAX_CARD, 2, 4)
}

/// Maps use the Games clamp: names + win bar need ~300 px, not a hero-grid squeeze.
pub fn maps_columns(available: f32) -> usize {
    grid_columns(available, MIN_CARD, MAX_CARD, 2, 4)
}

/// Seasons reuse the Maps grid: same ~300 px cells.
pub fn seasons_columns(available: f32) -> usize {
    maps_columns(available)
}

/// Settings section cards. Wider than a map tile so URL / token fields keep
/// their cap.
pub const MIN_SETTINGS_CARD: f32 = 480.0;
/// Widest a settings card gets in a multi-column pane. It also sets where the
/// next column appears: 2 → 3 above 1612 px of content (a 1920 window gets 3),
/// 3 → 4 above 2424 px (a 2560 window keeps 3, a 3440 ultrawide gets 4).
pub const MAX_SETTINGS_CARD: f32 = 800.0;
/// One settings column stays readable on a narrow or portrait window.
pub const SETTINGS_SINGLE_MAX: f32 = 720.0;
/// Eight masonry cards: four columns is two cards each. More would leave
/// columns of one card.
pub const MAX_SETTINGS_COLUMNS: usize = 4;

/// Fewest columns that keep cards at most [`MAX_SETTINGS_CARD`] wide, never
/// so many that a card drops under [`MIN_SETTINGS_CARD`].
pub fn settings_columns(available: f32) -> usize {
    let fit = columns_fit(available, MIN_SETTINGS_CARD, GRID_GAP);
    columns_needed(available, MAX_SETTINGS_CARD, GRID_GAP)
        .min(fit)
        .clamp(1, MAX_SETTINGS_COLUMNS)
}

/// Preferred window size on a normal landscape display.
pub const WINDOW_PREF_W: f32 = 1280.0;
pub const WINDOW_PREF_H: f32 = 860.0;
/// Smallest window the layout still scrolls in.
pub const WINDOW_MIN_W: f32 = 720.0;
pub const WINDOW_MIN_H: f32 = 480.0;

/// Settings pane width. One column stops at [`SETTINGS_SINGLE_MAX`]. More
/// columns fill the content width until each card reaches
/// [`MAX_SETTINGS_CARD`], which only happens past 3236 px of content (wider
/// than a 3440 window). The pane is left-aligned like every other page.
pub fn settings_pane_max(available: f32) -> f32 {
    if !available.is_finite() || available <= 0.0 {
        return 0.0;
    }
    let cols = settings_columns(available);
    let cap = if cols <= 1 {
        SETTINGS_SINGLE_MAX
    } else {
        span(cols, MAX_SETTINGS_CARD, GRID_GAP)
    };
    available.min(cap)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowFit {
    pub width: f32,
    pub height: f32,
}

/// Fit the GUI window to the display.
///
/// A tall portrait screen (1080x1920) keeps a width that fits and uses the
/// available height. A landscape screen keeps the preferred 1280x860 when
/// that fits.
pub fn fit_window(display_w: f32, display_h: f32) -> WindowFit {
    const MARGIN: f32 = 48.0;
    if !(display_w.is_finite() && display_h.is_finite()) || display_w < 1.0 || display_h < 1.0 {
        return WindowFit {
            width: WINDOW_PREF_W,
            height: WINDOW_PREF_H,
        };
    }
    let max_w = (display_w - MARGIN).max(display_w.min(WINDOW_MIN_W));
    let max_h = (display_h - MARGIN).max(display_h.min(WINDOW_MIN_H));
    let portrait = display_h > display_w;
    WindowFit {
        width: WINDOW_PREF_W.min(max_w),
        height: if portrait {
            max_h
        } else {
            WINDOW_PREF_H.min(max_h)
        },
    }
}

pub fn heroes_columns(available: f32) -> usize {
    grid_columns(available, MIN_CARD, MAX_HERO_CARD, 4, 6)
}

/// Compact cards under the Tonight featured card (one row's worth, then wrap).
pub fn tonight_compact_columns(available: f32) -> usize {
    columns_fit(available, MIN_CARD, GRID_GAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chrome used in `content_width_for_window`: sidebar 168 + page pads 64.
    fn at_window(window: f32) -> (f32, usize, usize, usize, usize, usize, usize) {
        let w = content_width_for_window(window);
        (
            w,
            games_columns(w),
            maps_columns(w),
            seasons_columns(w),
            heroes_columns(w),
            tonight_compact_columns(w),
            settings_columns(w),
        )
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn accept_widths_map_to_column_counts() {
        let (w1280, g1280, m1280, s1280, h1280, t1280, set1280) = at_window(1280.0);
        assert!(
            (w1280 - 1048.0).abs() < f32::EPSILON,
            "1280 content {w1280}"
        );
        assert_eq!(g1280, 3, "1280 games");
        assert_eq!(m1280, 3, "1280 maps");
        assert_eq!(s1280, 3, "1280 seasons");
        assert_eq!(h1280, 4, "1280 heroes");
        assert_eq!(t1280, 3, "1280 tonight compact");
        assert_eq!(set1280, 2, "1280 settings");

        let (w1920, g1920, m1920, s1920, h1920, t1920, set1920) = at_window(1920.0);
        assert!(
            (w1920 - 1688.0).abs() < f32::EPSILON,
            "1920 content {w1920}"
        );
        assert_eq!(g1920, 4, "1920 games");
        assert_eq!(m1920, 4, "1920 maps");
        assert_eq!(s1920, 4, "1920 seasons");
        assert_eq!(h1920, 5, "1920 heroes");
        assert_eq!(t1920, 5, "1920 tonight compact");
        // G4: was 2 (pane capped at 976 px). 1688 px holds three cards.
        assert_eq!(set1920, 3, "1920 settings");

        let (w2560, g2560, m2560, s2560, h2560, t2560, set2560) = at_window(2560.0);
        assert!(
            (w2560 - 2328.0).abs() < f32::EPSILON,
            "2560 content {w2560}"
        );
        assert_eq!(g2560, 4, "2560 games");
        assert_eq!(m2560, 4, "2560 maps");
        assert_eq!(s2560, 4, "2560 seasons");
        assert_eq!(h2560, 6, "2560 heroes");
        assert_eq!(t2560, 7, "2560 tonight compact");
        // G4: was 2 in the left 42% of the pane (the USER's screenshot).
        assert_eq!(set2560, 3, "2560 settings");

        let (w3440, g3440, m3440, s3440, h3440, t3440, set3440) = at_window(3440.0);
        assert!(close(w3440, 3208.0), "3440 content {w3440}");
        // G4: the 4 / 6 caps made 793 px game and map cards and 525 px hero
        // cards on an ultrawide. One more column each keeps them under the max.
        assert_eq!(g3440, 5, "3440 games");
        assert_eq!(m3440, 5, "3440 maps");
        assert_eq!(s3440, 5, "3440 seasons");
        assert_eq!(h3440, 7, "3440 heroes");
        assert_eq!(t3440, 10, "3440 tonight compact");
        assert_eq!(set3440, 4, "3440 settings");
    }

    #[test]
    fn games_start_at_two_to_four_then_grow_past_the_max_card() {
        assert_eq!(games_columns(0.0), 2);
        assert_eq!(games_columns(200.0), 2);
        assert_eq!(games_columns(1048.0), 3);
        assert_eq!(games_columns(1688.0), 4);
        assert_eq!(games_columns(2328.0), 4);
        // Past ~2596 px four cards would each pass MAX_CARD.
        assert_eq!(games_columns(2596.0), 4);
        assert_eq!(games_columns(2700.0), 5);
        assert_eq!(games_columns(3208.0), 5);
    }

    #[test]
    fn maps_start_at_two_to_four_then_grow_past_the_max_card() {
        assert_eq!(maps_columns(0.0), 2);
        assert_eq!(maps_columns(200.0), 2);
        assert_eq!(maps_columns(1048.0), 3);
        assert_eq!(maps_columns(1688.0), 4);
        assert_eq!(maps_columns(2328.0), 4);
        assert_eq!(maps_columns(3208.0), 5);
    }

    #[test]
    fn grid_cards_never_pass_their_max_width() {
        let mut w = 0.0_f32;
        while w <= 6000.0 {
            let g = cell_width(w, games_columns(w), GRID_GAP);
            assert!(g <= MAX_CARD + 0.01, "games card {g} at {w}");
            let m = cell_width(w, maps_columns(w), GRID_GAP);
            assert!(m <= MAX_CARD + 0.01, "maps card {m} at {w}");
            let h = cell_width(w, heroes_columns(w), GRID_GAP);
            assert!(h <= MAX_HERO_CARD + 0.01, "hero card {h} at {w}");
            w += 7.0;
        }
    }

    #[test]
    fn grids_never_shrink_a_card_under_the_min_to_add_a_column() {
        // Extra columns come only from the max-card rule, and that never asks
        // for more than fit at MIN_CARD. Below the lo clamp the old behaviour
        // (2 games / 4 heroes, even if narrow) is unchanged.
        let mut w = 1300.0_f32;
        while w <= 6000.0 {
            let g = cell_width(w, games_columns(w), GRID_GAP);
            assert!(g >= MIN_CARD - 0.01, "games card {g} at {w}");
            let h = cell_width(w, heroes_columns(w), GRID_GAP);
            assert!(h >= MIN_CARD - 0.01, "hero card {h} at {w}");
            w += 7.0;
        }
    }

    #[test]
    fn extra_columns_never_exceed_what_fits_at_the_min_card() {
        // Today's constants cannot hit this (MAX_CARD is over twice MIN_CARD),
        // so pin the rule itself: 900 px holds two 300 px cards, and a 310 px
        // max would ask for three 292 px ones.
        assert_eq!(columns_needed(900.0, 310.0, GRID_GAP), 3);
        assert_eq!(grid_columns(900.0, 300.0, 310.0, 1, 4), 2);
    }

    #[test]
    fn seasons_match_maps_columns() {
        for w in [0.0, 200.0, 1048.0, 1688.0, 3208.0, 10_000.0] {
            assert_eq!(seasons_columns(w), maps_columns(w), "seasons cols at {w}");
        }
    }

    /// G4 replaces `settings_never_above_two`: the 1..=2 clamp is what left
    /// the USER's 2560 window with two 482 px columns.
    #[test]
    fn settings_use_one_to_four_columns() {
        assert_eq!(settings_columns(0.0), 1);
        assert_eq!(settings_columns(200.0), 1);
        assert_eq!(settings_columns(480.0), 1);
        assert_eq!(settings_columns(971.0), 1);
        assert_eq!(settings_columns(972.0), 2);
        assert_eq!(settings_columns(1048.0), 2);
        assert_eq!(settings_columns(1612.0), 2);
        assert_eq!(settings_columns(1613.0), 3);
        assert_eq!(settings_columns(2424.0), 3);
        assert_eq!(settings_columns(2425.0), 4);
        assert_eq!(settings_columns(10_000.0), MAX_SETTINGS_COLUMNS);
    }

    /// (window width, settings columns, settings pane width).
    const SETTINGS_BY_WINDOW: &[(f32, usize, f32)] = &[
        (720.0, 1, 488.0),
        (1280.0, 2, 1048.0),
        (1600.0, 2, 1368.0),
        (1920.0, 3, 1688.0),
        (2560.0, 3, 2328.0),
        (3440.0, 4, 3208.0),
    ];

    #[test]
    fn settings_columns_and_pane_by_window_width() {
        for &(window, cols, pane) in SETTINGS_BY_WINDOW {
            let content = content_width_for_window(window);
            assert_eq!(settings_columns(content), cols, "cols at window {window}");
            let got = settings_pane_max(content);
            assert!(close(got, pane), "pane {got} at window {window}");
        }
        // Portrait 1080x1920: fit_window keeps 1032 px, one 720 px column.
        let fit = fit_window(1080.0, 1920.0);
        let content = content_width_for_window(fit.width);
        assert!(close(content, 800.0), "portrait content {content}");
        assert_eq!(settings_columns(content), 1);
        assert!(close(settings_pane_max(content), SETTINGS_SINGLE_MAX));
    }

    #[test]
    fn settings_at_2560_use_most_of_the_content_width() {
        let content = content_width_for_window(2560.0);
        let pane = settings_pane_max(content);
        assert!(pane >= 0.85 * content, "pane {pane} of {content}");
    }

    #[test]
    fn settings_cards_stay_between_min_and_max() {
        let mut w = 972.0_f32;
        while w <= 6000.0 {
            let cols = settings_columns(w);
            assert!(cols >= 2, "cols {cols} at {w}");
            let card = cell_width(settings_pane_max(w), cols, GRID_GAP);
            assert!(card >= MIN_SETTINGS_CARD - 0.01, "card {card} at {w}");
            assert!(card <= MAX_SETTINGS_CARD + 0.01, "card {card} at {w}");
            w += 5.0;
        }
        let mut w = 1.0_f32;
        while w < 972.0 {
            assert_eq!(settings_columns(w), 1, "at {w}");
            assert!(settings_pane_max(w) <= SETTINGS_SINGLE_MAX, "at {w}");
            w += 5.0;
        }
    }

    #[test]
    fn portrait_1080x1920_uses_the_available_height() {
        let fit = fit_window(1080.0, 1920.0);
        assert!(fit.width <= 1080.0, "width {}", fit.width);
        assert!(fit.width >= 720.0, "width {}", fit.width);
        assert!(fit.height > WINDOW_PREF_H, "height {}", fit.height);
        assert!(fit.height <= 1920.0, "height {}", fit.height);
        assert!(fit.height > 1600.0, "height {}", fit.height);
        let content = content_width_for_window(fit.width);
        assert_eq!(settings_columns(content), 1, "content {content}");
        let pane = settings_pane_max(content);
        assert!(pane <= 720.0, "pane {pane}");
        assert!(pane <= content);
    }

    #[test]
    fn landscape_keeps_the_preferred_window_and_two_settings_columns() {
        let fit = fit_window(1920.0, 1080.0);
        assert!((fit.width - WINDOW_PREF_W).abs() < f32::EPSILON);
        assert!((fit.height - WINDOW_PREF_H).abs() < f32::EPSILON);
        let content = content_width_for_window(fit.width);
        assert_eq!(settings_columns(content), 2);
        // G4: was capped at two 480 px cards (976 px); now the two cards
        // share the whole 1048 px pane.
        let pane = settings_pane_max(content);
        assert!(close(pane, content), "pane {pane} of {content}");
    }

    #[test]
    fn missing_display_uses_the_preferred_window() {
        let fit = fit_window(0.0, 0.0);
        assert_eq!(fit.width, WINDOW_PREF_W);
        assert_eq!(fit.height, WINDOW_PREF_H);
    }

    #[test]
    fn heroes_start_at_four_to_six_then_grow_past_the_max_card() {
        assert_eq!(heroes_columns(0.0), 4);
        assert_eq!(heroes_columns(400.0), 4);
        assert_eq!(heroes_columns(1688.0), 5);
        assert_eq!(heroes_columns(2328.0), 6);
        assert_eq!(heroes_columns(3208.0), 7);
    }

    #[test]
    fn three_hundred_px_is_the_step() {
        // n * 300 + (n-1) * 12 <= available
        assert_eq!(columns_fit(611.0, MIN_CARD, GRID_GAP), 1);
        assert_eq!(columns_fit(612.0, MIN_CARD, GRID_GAP), 2);
        assert_eq!(columns_fit(923.0, MIN_CARD, GRID_GAP), 2);
        assert_eq!(columns_fit(924.0, MIN_CARD, GRID_GAP), 3);
    }

    #[test]
    fn columns_needed_is_the_fewest_under_the_max() {
        // n * 640 + (n-1) * 12 >= available
        assert_eq!(columns_needed(0.0, MAX_CARD, GRID_GAP), 1);
        assert_eq!(columns_needed(640.0, MAX_CARD, GRID_GAP), 1);
        assert_eq!(columns_needed(641.0, MAX_CARD, GRID_GAP), 2);
        assert_eq!(columns_needed(1292.0, MAX_CARD, GRID_GAP), 2);
        assert_eq!(columns_needed(1293.0, MAX_CARD, GRID_GAP), 3);
        assert!(close(span(3, 100.0, GRID_GAP), 324.0));
        assert!(close(cell_width(324.0, 3, GRID_GAP), 100.0));
    }
}
