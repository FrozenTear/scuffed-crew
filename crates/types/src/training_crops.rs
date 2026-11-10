//! Crop kinds and numeric limits for opt-in training crops
//! (`/api/training-crops`).
//!
//! A training crop is a small PNG the stat tracker cut from a screen it read,
//! sent with the member's per-kind consent so recognizers can be trained on
//! real captures. Hero icon contributions (`/api/hero-icons`) are the
//! `hero_icon` kind of the same store. See `docs/api/training-crops.md`.

use serde::{Deserialize, Serialize};

use crate::hero_icons::{HERO_ICON_MAX_BYTES, HERO_ICON_MAX_EDGE, HERO_ICON_MAX_PER_MEMBER};

/// What a crop shows. Consent, caps and the upload limiter are per kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CropKind {
    /// A hero portrait or icon. Same rows as `/api/hero-icons`.
    HeroIcon,
    /// Digits from a stat panel.
    StatDigits,
    /// The map name banner.
    MapName,
    /// A whole screen. Uploads are refused for now and consent for it cannot
    /// be stored: the consent shapes have no `full_screen` field.
    FullScreen,
}

impl CropKind {
    /// Every kind, in the order used by lists and counts.
    pub const ALL: [CropKind; 4] = [
        CropKind::HeroIcon,
        CropKind::StatDigits,
        CropKind::MapName,
        CropKind::FullScreen,
    ];

    /// The wire and database name.
    pub fn as_str(self) -> &'static str {
        match self {
            CropKind::HeroIcon => "hero_icon",
            CropKind::StatDigits => "stat_digits",
            CropKind::MapName => "map_name",
            CropKind::FullScreen => "full_screen",
        }
    }

    /// Parse the wire name. Exact match only.
    pub fn parse(raw: &str) -> Option<CropKind> {
        CropKind::ALL.into_iter().find(|kind| kind.as_str() == raw)
    }

    /// True when the server stores uploads of this kind.
    pub fn accepts_uploads(self) -> bool {
        self != CropKind::FullScreen
    }

    /// Most stored crops of this kind per member. `0` for a kind that is
    /// never stored.
    pub fn max_per_member(self) -> u32 {
        match self {
            CropKind::HeroIcon => HERO_ICON_MAX_PER_MEMBER,
            CropKind::StatDigits => STAT_DIGITS_MAX_PER_MEMBER,
            CropKind::MapName => MAP_NAME_MAX_PER_MEMBER,
            CropKind::FullScreen => 0,
        }
    }

    /// Largest accepted crop `(width, height)` in pixels. Every edge is at
    /// least `HERO_ICON_MIN_EDGE`.
    ///
    /// `map_name` banners are wide and the tracker does not shrink them, so
    /// that kind takes up to [`MAP_NAME_MAX_WIDTH`] by
    /// [`MAP_NAME_MAX_HEIGHT`]. Every other kind keeps the hero icon bound of
    /// `HERO_ICON_MAX_EDGE` on each edge.
    pub fn max_edges(self) -> (u32, u32) {
        match self {
            CropKind::MapName => (MAP_NAME_MAX_WIDTH, MAP_NAME_MAX_HEIGHT),
            _ => (HERO_ICON_MAX_EDGE, HERO_ICON_MAX_EDGE),
        }
    }

    /// Largest accepted `crop` part in bytes. [`MAP_NAME_MAX_BYTES`] for
    /// `map_name` (its larger pixel bound needs it), `HERO_ICON_MAX_BYTES`
    /// otherwise. The whole request stays capped at
    /// `HERO_ICON_MAX_REQUEST_BYTES` (2 MiB) for every kind.
    pub fn max_crop_bytes(self) -> usize {
        match self {
            CropKind::MapName => MAP_NAME_MAX_BYTES,
            _ => HERO_ICON_MAX_BYTES,
        }
    }
}

/// Most `stat_digits` crops one member may have stored at the same time.
pub const STAT_DIGITS_MAX_PER_MEMBER: u32 = 1000;

/// Most `map_name` crops one member may have stored at the same time.
pub const MAP_NAME_MAX_PER_MEMBER: u32 = 200;

/// Widest accepted `map_name` crop, in pixels. Map name banners are wide
/// and the tracker sends them unscaled: a native 4K banner is 1036 px wide,
/// so this leaves room above that.
pub const MAP_NAME_MAX_WIDTH: u32 = 1536;

/// Tallest accepted `map_name` crop, in pixels.
pub const MAP_NAME_MAX_HEIGHT: u32 = 256;

/// Largest accepted `map_name` crop part, in bytes (256 KiB). The request
/// limit of 2 MiB still applies, so a request holds fewer of the largest
/// banners than the 20-pair maximum.
pub const MAP_NAME_MAX_BYTES: usize = 256 * 1024;

/// Largest accepted `meta` part, in bytes (4 KiB).
pub const TRAINING_CROP_META_MAX_BYTES: usize = 4 * 1024;

/// Longest accepted short label (`screen`, `slot_type`).
pub const TRAINING_CROP_LABEL_MAX_LEN: usize = 32;

/// Longest accepted version string (`icon_set_version`, `spot_version`).
pub const TRAINING_CROP_VERSION_MAX_LEN: usize = 64;

/// Exact length of a `game_token`: 32 lowercase hex characters (128 random
/// bits made by the tracker per game). Timestamps, UUIDs with dashes and
/// other shapes are refused, so the token cannot carry when or which game.
pub const TRAINING_CROP_GAME_TOKEN_LEN: usize = 32;

/// Highest accepted `slot_index`.
pub const TRAINING_CROP_SLOT_INDEX_MAX: u32 = 63;

/// Largest absolute value accepted for `score`, `runner_up` and `margin`.
pub const TRAINING_CROP_SCORE_ABS_MAX: f64 = 1000.0;

/// Smallest accepted screen width or height, in pixels.
pub const TRAINING_CROP_SCREEN_MIN_EDGE: u32 = 320;

/// Largest accepted screen width or height, in pixels. Also bounds the
/// found spot.
pub const TRAINING_CROP_SCREEN_MAX_EDGE: u32 = 16384;

/// Largest accepted `ui_scale`. The lower bound is exclusive zero, the same
/// range the tracker's report bundle uses.
pub const TRAINING_CROP_UI_SCALE_MAX: f64 = 200.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_names_round_trip() {
        for kind in CropKind::ALL {
            assert_eq!(CropKind::parse(kind.as_str()), Some(kind));
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
        }
        assert_eq!(CropKind::parse("Hero_Icon"), None);
        assert_eq!(CropKind::parse(""), None);
    }

    #[test]
    fn only_full_screen_is_refused_and_capped_at_zero() {
        assert!(!CropKind::FullScreen.accepts_uploads());
        assert_eq!(CropKind::FullScreen.max_per_member(), 0);
        for kind in [CropKind::HeroIcon, CropKind::StatDigits, CropKind::MapName] {
            assert!(kind.accepts_uploads());
            assert!(kind.max_per_member() > 0);
        }
        assert_eq!(
            CropKind::HeroIcon.max_per_member(),
            HERO_ICON_MAX_PER_MEMBER
        );
    }

    #[test]
    fn only_map_name_gets_the_wide_bound() {
        assert_eq!(CropKind::MapName.max_edges(), (1536, 256));
        assert_eq!(CropKind::MapName.max_crop_bytes(), 256 * 1024);
        for kind in [CropKind::HeroIcon, CropKind::StatDigits] {
            assert_eq!(kind.max_edges(), (HERO_ICON_MAX_EDGE, HERO_ICON_MAX_EDGE));
            assert_eq!(kind.max_crop_bytes(), HERO_ICON_MAX_BYTES);
        }
    }
}
