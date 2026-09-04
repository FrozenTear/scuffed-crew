//! Responsive column counts from available content width (design rev 3).
//!
//! Cards stay at least ~[`MIN_CARD`] wide. Games and maps clamp 2–4 columns,
//! heroes 4–6. Tonight is one featured card plus as many compact cards as fit
//! at that minimum.

use crate::theme::{GRID_GAP, PAGE_PAD_X, SIDEBAR_WIDTH};

/// Minimum card width used when deciding how many columns fit.
pub const MIN_CARD: f32 = 300.0;

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

pub fn games_columns(available: f32) -> usize {
    columns_fit(available, MIN_CARD, GRID_GAP).clamp(2, 4)
}

/// Maps use the Games clamp: names + win bar need ~300 px, not a hero-grid squeeze.
pub fn maps_columns(available: f32) -> usize {
    columns_fit(available, MIN_CARD, GRID_GAP).clamp(2, 4)
}

/// Seasons reuse the Maps grid: same ~300 px cells, 2–4 columns.
pub fn seasons_columns(available: f32) -> usize {
    maps_columns(available)
}

/// Settings section cards. Wider than a map tile so URL / token fields keep
/// their cap; still 1–2 columns so the pane is not one ultrawide stack.
pub const MIN_SETTINGS_CARD: f32 = 480.0;

pub fn settings_columns(available: f32) -> usize {
    columns_fit(available, MIN_SETTINGS_CARD, GRID_GAP).clamp(1, 2)
}

pub fn heroes_columns(available: f32) -> usize {
    columns_fit(available, MIN_CARD, GRID_GAP).clamp(4, 6)
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
        assert_eq!(set1920, 2, "1920 settings");

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
        assert_eq!(set2560, 2, "2560 settings");
    }

    #[test]
    fn games_never_below_two_or_above_four() {
        assert_eq!(games_columns(0.0), 2);
        assert_eq!(games_columns(200.0), 2);
        assert_eq!(games_columns(10_000.0), 4);
    }

    #[test]
    fn maps_never_below_two_or_above_four() {
        assert_eq!(maps_columns(0.0), 2);
        assert_eq!(maps_columns(200.0), 2);
        assert_eq!(maps_columns(10_000.0), 4);
        assert_eq!(maps_columns(1048.0), 3);
        assert_eq!(maps_columns(1688.0), 4);
    }

    #[test]
    fn seasons_match_maps_columns() {
        for w in [0.0, 200.0, 1048.0, 1688.0, 10_000.0] {
            assert_eq!(seasons_columns(w), maps_columns(w), "seasons cols at {w}");
        }
    }

    #[test]
    fn settings_never_above_two() {
        assert_eq!(settings_columns(0.0), 1);
        assert_eq!(settings_columns(200.0), 1);
        assert_eq!(settings_columns(480.0), 1);
        assert_eq!(settings_columns(972.0), 2);
        assert_eq!(settings_columns(1048.0), 2);
        assert_eq!(settings_columns(10_000.0), 2);
    }

    #[test]
    fn heroes_never_below_four_or_above_six() {
        assert_eq!(heroes_columns(0.0), 4);
        assert_eq!(heroes_columns(400.0), 4);
        assert_eq!(heroes_columns(10_000.0), 6);
    }

    #[test]
    fn three_hundred_px_is_the_step() {
        // n * 300 + (n-1) * 12 <= available
        assert_eq!(columns_fit(611.0, MIN_CARD, GRID_GAP), 1);
        assert_eq!(columns_fit(612.0, MIN_CARD, GRID_GAP), 2);
        assert_eq!(columns_fit(923.0, MIN_CARD, GRID_GAP), 2);
        assert_eq!(columns_fit(924.0, MIN_CARD, GRID_GAP), 3);
    }
}
