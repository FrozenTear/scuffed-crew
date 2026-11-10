//! JSON shapes and error codes for `/api/training-crops` (opt-in training
//! crops sent by the stat tracker).
//!
//! Hero icon contributions are the `hero_icon` kind of the same store, so the
//! PNG checks reuse the `HERO_ICON_ERR_*` codes. See
//! `docs/api/training-crops.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::training_crops::{
    CropKind, TRAINING_CROP_GAME_TOKEN_LEN, TRAINING_CROP_LABEL_MAX_LEN,
    TRAINING_CROP_SCORE_ABS_MAX, TRAINING_CROP_SCREEN_MAX_EDGE, TRAINING_CROP_SCREEN_MIN_EDGE,
    TRAINING_CROP_SLOT_INDEX_MAX, TRAINING_CROP_UI_SCALE_MAX, TRAINING_CROP_VERSION_MAX_LEN,
};

/// `?kind=` is missing or not one of the four kinds. 400.
pub const TRAINING_CROP_ERR_UNKNOWN_KIND: &str = "unknown_crop_kind";
/// `?kind=full_screen`. The server does not take whole screens yet, whatever
/// the consent says. Nothing is read or stored. 422.
pub const TRAINING_CROP_ERR_FULL_SCREEN_REFUSED: &str = "full_screen_not_accepted";
/// The caller has not consented to this kind. 403. Nothing is stored.
pub const TRAINING_CROP_ERR_CONSENT_REQUIRED: &str = "crop_consent_required";
/// A `meta` part is not valid JSON, has an unknown field, or a value is out
/// of bounds. 400.
pub const TRAINING_CROP_ERR_INVALID_META: &str = "invalid_crop_meta";
/// The request carried no `meta` and `crop` pair. 400.
pub const TRAINING_CROP_ERR_NO_CROPS: &str = "no_crops";
/// More than `HERO_ICON_MAX_FILES_PER_UPLOAD` crops in one request. 413.
pub const TRAINING_CROP_ERR_TOO_MANY_IN_REQUEST: &str = "too_many_crops_in_request";
/// The upload would take the caller over the cap of this kind. 409.
pub const TRAINING_CROP_ERR_LIMIT_REACHED: &str = "crop_limit_reached";
/// The consent body is not JSON, names an unknown kind (including
/// `full_screen`, whose consent cannot be stored), or a value is not a
/// bool. 400.
pub const TRAINING_CROP_ERR_INVALID_CONSENT: &str = "invalid_consent";
/// Unknown id, malformed id, or a crop the caller may not see. 404.
pub const TRAINING_CROP_ERR_NOT_FOUND: &str = "crop_not_found";

/// A spot the tracker found on a screen size it has no layout for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CropSpot {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Per-crop details, as stored and as sent back. Every field is optional.
///
/// `game_token` is 32 lowercase hex characters the tracker draws at random per
/// game so crops of one game can be grouped. The server never links it to a
/// match.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrainingCropMeta {
    /// Which screen the crop came from, for example `scoreboard`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<String>,
    /// Which kind of slot, for example `ally` or `enemy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_token: Option<String>,
    /// Best match score of the read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Score of the second best match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_up: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_set_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vote_card_filled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked_in: Option<bool>,
    /// Game resolution. Both or neither.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colorblind_filter: Option<String>,
    /// For unknown screen sizes. Both `found_spot` and `found_confidence`, or
    /// neither.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found_spot: Option<CropSpot>,
    /// `0.0..=1.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found_confidence: Option<f64>,
}

/// The `meta` part sent before each `crop` part of
/// `POST /api/training-crops`. Unknown fields are refused.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingCropUploadMeta {
    /// Hero key or display name. Required for `hero_icon`, optional for the
    /// other kinds. Checked by the server against the hero list and stored
    /// as the hero key (`soldier-76`, see `scuffed_types::hero_key`).
    #[serde(default)]
    pub hero: Option<String>,
    #[serde(default)]
    pub screen: Option<String>,
    #[serde(default)]
    pub slot_type: Option<String>,
    #[serde(default)]
    pub slot_index: Option<u32>,
    #[serde(default)]
    pub game_token: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub runner_up: Option<f64>,
    #[serde(default)]
    pub margin: Option<f64>,
    #[serde(default)]
    pub icon_set_version: Option<String>,
    #[serde(default)]
    pub spot_version: Option<String>,
    #[serde(default)]
    pub vote_card_filled: Option<bool>,
    #[serde(default)]
    pub locked_in: Option<bool>,
    #[serde(default)]
    pub screen_width: Option<u32>,
    #[serde(default)]
    pub screen_height: Option<u32>,
    #[serde(default)]
    pub ui_scale: Option<f64>,
    #[serde(default)]
    pub colorblind_filter: Option<String>,
    #[serde(default)]
    pub found_spot: Option<CropSpot>,
    #[serde(default)]
    pub found_confidence: Option<f64>,
}

fn label_ok(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= TRAINING_CROP_LABEL_MAX_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

fn version_ok(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= TRAINING_CROP_VERSION_MAX_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'+'))
}

/// Exactly 32 lowercase hex characters.
fn game_token_ok(value: &str) -> bool {
    value.len() == TRAINING_CROP_GAME_TOKEN_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn score_ok(value: f64) -> bool {
    value.is_finite() && value.abs() <= TRAINING_CROP_SCORE_ABS_MAX
}

fn screen_edge_ok(value: u32) -> bool {
    (TRAINING_CROP_SCREEN_MIN_EDGE..=TRAINING_CROP_SCREEN_MAX_EDGE).contains(&value)
}

/// `Some(true)` passes, `Some(false)` fails, `None` passes.
fn optional<T>(value: Option<T>, check: impl Fn(T) -> bool) -> bool {
    value.is_none_or(check)
}

impl TrainingCropUploadMeta {
    /// Bounds and shapes of every field except `hero`, which the server
    /// checks against the hero list. `Err` names the first bad field, for
    /// logs only: the wire answer is always [`TRAINING_CROP_ERR_INVALID_META`].
    pub fn validate(&self) -> Result<(), &'static str> {
        if !optional(self.screen.as_deref(), label_ok) {
            return Err("screen");
        }
        if !optional(self.slot_type.as_deref(), label_ok) {
            return Err("slot_type");
        }
        if !optional(self.slot_index, |v| v <= TRAINING_CROP_SLOT_INDEX_MAX) {
            return Err("slot_index");
        }
        if !optional(self.game_token.as_deref(), game_token_ok) {
            return Err("game_token");
        }
        if !optional(self.score, score_ok) {
            return Err("score");
        }
        if !optional(self.runner_up, score_ok) {
            return Err("runner_up");
        }
        if !optional(self.margin, score_ok) {
            return Err("margin");
        }
        if !optional(self.icon_set_version.as_deref(), version_ok) {
            return Err("icon_set_version");
        }
        if !optional(self.spot_version.as_deref(), version_ok) {
            return Err("spot_version");
        }
        if self.screen_width.is_some() != self.screen_height.is_some() {
            return Err("screen_width");
        }
        if !optional(self.screen_width, screen_edge_ok) {
            return Err("screen_width");
        }
        if !optional(self.screen_height, screen_edge_ok) {
            return Err("screen_height");
        }
        let ui_scale_ok = |v: f64| v.is_finite() && v > 0.0 && v <= TRAINING_CROP_UI_SCALE_MAX;
        if !optional(self.ui_scale, ui_scale_ok) {
            return Err("ui_scale");
        }
        if !optional(self.colorblind_filter.as_deref(), label_ok) {
            return Err("colorblind_filter");
        }
        if self.found_spot.is_some() != self.found_confidence.is_some() {
            return Err("found_spot");
        }
        if !optional(self.found_confidence, |v| (0.0..=1.0).contains(&v)) {
            return Err("found_confidence");
        }
        if let Some(spot) = self.found_spot {
            let max_w = self.screen_width.unwrap_or(TRAINING_CROP_SCREEN_MAX_EDGE);
            let max_h = self.screen_height.unwrap_or(TRAINING_CROP_SCREEN_MAX_EDGE);
            let fits = |start: u32, len: u32, max: u32| {
                len >= 1 && start.checked_add(len).is_some_and(|end| end <= max)
            };
            if !fits(spot.x, spot.w, max_w) || !fits(spot.y, spot.h, max_h) {
                return Err("found_spot");
            }
        }
        Ok(())
    }

    /// The hero field and the stored metadata.
    pub fn split(self) -> (Option<String>, TrainingCropMeta) {
        let meta = TrainingCropMeta {
            screen: self.screen,
            slot_type: self.slot_type,
            slot_index: self.slot_index,
            game_token: self.game_token,
            score: self.score,
            runner_up: self.runner_up,
            margin: self.margin,
            icon_set_version: self.icon_set_version,
            spot_version: self.spot_version,
            vote_card_filled: self.vote_card_filled,
            locked_in: self.locked_in,
            screen_width: self.screen_width,
            screen_height: self.screen_height,
            ui_scale: self.ui_scale,
            colorblind_filter: self.colorblind_filter,
            found_spot: self.found_spot,
            found_confidence: self.found_confidence,
        };
        (self.hero, meta)
    }
}

/// Per-kind consent. Every kind is off until the member turns it on.
///
/// `hero_icon` is the same flag as `icon_contribution_opt_in` on
/// `/api/hero-icons/opt-in`. There is no `full_screen` field: whole screens
/// are not accepted, so their consent cannot be stored or reported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainingCropConsent {
    pub hero_icon: bool,
    pub stat_digits: bool,
    pub map_name: bool,
}

impl TrainingCropConsent {
    /// Consent for `kind`. Always false for `full_screen`.
    pub fn get(&self, kind: CropKind) -> bool {
        match kind {
            CropKind::HeroIcon => self.hero_icon,
            CropKind::StatDigits => self.stat_digits,
            CropKind::MapName => self.map_name,
            CropKind::FullScreen => false,
        }
    }

    pub fn any(&self) -> bool {
        CropKind::ALL.into_iter().any(|kind| self.get(kind))
    }
}

/// `PUT /api/training-crops/consent` (session only). Only the kinds given
/// change. Unknown fields, `full_screen` among them, are refused (400
/// `invalid_consent`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingCropConsentUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero_icon: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stat_digits: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_name: Option<bool>,
}

/// One stored crop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingCropEntry {
    /// 32 lowercase hex characters, made by the server.
    pub id: String,
    pub kind: CropKind,
    /// Hero key (`soldier-76`, see `scuffed_types::hero_key`). Always set
    /// for `hero_icon`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero: Option<String>,
    /// Stored PNG size after the server re-encoded it.
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    /// RFC 3339 UTC.
    pub uploaded_at: DateTime<Utc>,
    /// `scuffed-stat-tracker/<semver>` version from the upload's User-Agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracker_version: Option<String>,
    pub meta: TrainingCropMeta,
    /// `/api/training-crops/{id}/image`. Needs a session; never public.
    pub thumbnail_url: String,
}

/// `201` from `POST /api/training-crops`. The crops stored by this request,
/// in the order they were sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingCropUploadResponse {
    pub kind: CropKind,
    pub crops: Vec<TrainingCropEntry>,
}

/// `DELETE /api/training-crops/mine` and `DELETE /api/training-crops/{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingCropsDeleted {
    pub deleted: u32,
}

/// Stored count of one kind for one member.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingCropKindCount {
    pub kind: CropKind,
    pub count: u32,
    /// The member's consent for this kind right now. Always true in an
    /// officer's list.
    pub consented: bool,
    /// Cap for this kind.
    pub max: u32,
}

/// Stored count of one kind at one game resolution. `screen_width` and
/// `screen_height` are absent for crops sent without a resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingCropResolutionCount {
    pub kind: CropKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_height: Option<u32>,
    pub count: u32,
}

/// `GET /api/training-crops/mine` (session only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MyTrainingCrops {
    pub consent: TrainingCropConsent,
    /// One row per stored kind (`full_screen` is never stored).
    pub kinds: Vec<TrainingCropKindCount>,
    /// Newest first, paged with the usual cursor.
    pub crops: Vec<TrainingCropEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// One crop in the admin list (`GET /api/training-crops`).
///
/// `uploaded_at` is cut down to the start of its UTC hour here, so a crop
/// cannot be matched to a game by its upload time.
///
/// Every stored kind of every active member is listed, so an admin can find
/// any crop's id to delete it. A crop whose kind the owner does not consent
/// to right now is `hidden`: only its id, kind, size, dimensions and hour are
/// sent (no `hero`, `tracker_version` or `meta` fields), and its image answers
/// 404.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaffTrainingCrop {
    pub member_id: String,
    /// Empty when the member record has no name.
    pub display_name: String,
    /// True when the owner's consent for this kind is off.
    pub hidden: bool,
    #[serde(flatten)]
    pub crop: TrainingCropEntry,
}

/// One row of `GET /api/training-crops/members` (officer or admin), sent as
/// a `CursorResponse` page sorted by display name, then member id.
///
/// Officers see only the kinds a member consents to, and only members with
/// at least one consented kind. Admins also see kinds that are hidden
/// (consent off) while crops of that kind are still stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingCropMemberCounts {
    pub member_id: String,
    /// Empty when the member record has no name.
    pub display_name: String,
    /// Sum of `kinds[].count`.
    pub total: u32,
    pub kinds: Vec<TrainingCropKindCount>,
    /// The same crops by kind and game resolution, largest count first.
    pub resolutions: Vec<TrainingCropResolutionCount>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> TrainingCropUploadMeta {
        TrainingCropUploadMeta {
            hero: Some("ana".into()),
            screen: Some("scoreboard".into()),
            slot_type: Some("ally".into()),
            slot_index: Some(4),
            game_token: Some("0123456789abcdef0123456789abcdef".into()),
            score: Some(0.93),
            runner_up: Some(0.41),
            margin: Some(0.52),
            icon_set_version: Some("2026.11.1".into()),
            spot_version: Some("v3+1080p".into()),
            vote_card_filled: Some(true),
            locked_in: Some(false),
            screen_width: Some(1920),
            screen_height: Some(1080),
            ui_scale: Some(100.0),
            colorblind_filter: Some("deuteranopia".into()),
            found_spot: Some(CropSpot {
                x: 10,
                y: 20,
                w: 64,
                h: 64,
            }),
            found_confidence: Some(0.8),
        }
    }

    fn bad(edit: impl FnOnce(&mut TrainingCropUploadMeta)) -> &'static str {
        let mut meta = full();
        edit(&mut meta);
        meta.validate().expect_err("should be refused")
    }

    #[test]
    fn empty_and_full_meta_pass() {
        TrainingCropUploadMeta::default().validate().unwrap();
        full().validate().unwrap();
    }

    #[test]
    fn labels_are_bounded_and_plain() {
        assert_eq!(bad(|m| m.screen = Some(String::new())), "screen");
        assert_eq!(bad(|m| m.screen = Some("a".repeat(33))), "screen");
        assert_eq!(bad(|m| m.slot_type = Some("ally team".into())), "slot_type");
        assert_eq!(
            bad(|m| m.colorblind_filter = Some("é".into())),
            "colorblind_filter"
        );
        let mut ok = full();
        ok.screen = Some("a".repeat(32));
        ok.validate().unwrap();
    }

    #[test]
    fn slot_index_bound() {
        assert_eq!(bad(|m| m.slot_index = Some(64)), "slot_index");
        let mut ok = full();
        ok.slot_index = Some(63);
        ok.validate().unwrap();
    }

    #[test]
    fn game_token_is_exactly_32_lowercase_hex() {
        for token in [
            "short",
            "0123456789abcdef0123456789abcde",   // 31
            "0123456789abcdef0123456789abcdef0", // 33
            "0123456789ABCDEF0123456789ABCDEF",  // upper case
            "0123456789abcdef0123456789abcdeg",  // not hex
            // A UUID with dashes and an RFC 3339 timestamp.
            "3f2a9c0e-6b1d-4e8f-9a7c-5b3d1e0f",
            "2026-11-10T18:04:05.123456Z00000",
            "17312592451234567890123456789012x",
        ] {
            assert_eq!(bad(|m| m.game_token = Some(token.into())), "game_token");
        }
        let mut ok = full();
        ok.game_token = Some("ffffffffffffffffffffffffffffffff".into());
        ok.validate().unwrap();
    }

    #[test]
    fn scores_must_be_finite_and_bounded() {
        assert_eq!(bad(|m| m.score = Some(f64::NAN)), "score");
        assert_eq!(bad(|m| m.runner_up = Some(f64::INFINITY)), "runner_up");
        assert_eq!(bad(|m| m.margin = Some(-1000.5)), "margin");
        let mut ok = full();
        ok.margin = Some(-1000.0);
        ok.validate().unwrap();
    }

    #[test]
    fn versions_are_bounded() {
        assert_eq!(
            bad(|m| m.icon_set_version = Some(String::new())),
            "icon_set_version"
        );
        assert_eq!(
            bad(|m| m.spot_version = Some("v".repeat(65))),
            "spot_version"
        );
        assert_eq!(bad(|m| m.spot_version = Some("v 1".into())), "spot_version");
    }

    #[test]
    fn resolution_needs_both_edges_in_range() {
        assert_eq!(bad(|m| m.screen_height = None), "screen_width");
        assert_eq!(bad(|m| m.screen_width = Some(319)), "screen_width");
        assert_eq!(bad(|m| m.screen_height = Some(16385)), "screen_height");
    }

    #[test]
    fn ui_scale_is_above_zero_and_at_most_200() {
        assert_eq!(bad(|m| m.ui_scale = Some(0.0)), "ui_scale");
        assert_eq!(bad(|m| m.ui_scale = Some(200.5)), "ui_scale");
        assert_eq!(bad(|m| m.ui_scale = Some(f64::NAN)), "ui_scale");
        let mut ok = full();
        ok.ui_scale = Some(200.0);
        ok.validate().unwrap();
    }

    #[test]
    fn found_spot_needs_confidence_and_must_fit() {
        assert_eq!(bad(|m| m.found_confidence = None), "found_spot");
        assert_eq!(bad(|m| m.found_spot = None), "found_spot");
        assert_eq!(bad(|m| m.found_confidence = Some(1.01)), "found_confidence");
        assert_eq!(
            bad(|m| m.found_confidence = Some(-0.01)),
            "found_confidence"
        );
        assert_eq!(
            bad(|m| m.found_spot = Some(CropSpot {
                x: 1900,
                y: 0,
                w: 64,
                h: 64
            })),
            "found_spot"
        );
        assert_eq!(
            bad(|m| m.found_spot = Some(CropSpot {
                x: 0,
                y: 0,
                w: 0,
                h: 64
            })),
            "found_spot"
        );
        assert_eq!(
            bad(|m| m.found_spot = Some(CropSpot {
                x: u32::MAX,
                y: 0,
                w: 2,
                h: 2
            })),
            "found_spot"
        );
        // No resolution: the spot is bounded by the largest screen edge.
        let mut unknown = full();
        unknown.screen_width = None;
        unknown.screen_height = None;
        unknown.found_spot = Some(CropSpot {
            x: 3000,
            y: 1500,
            w: 64,
            h: 64,
        });
        unknown.validate().unwrap();
    }

    #[test]
    fn unknown_fields_are_refused() {
        let parsed = serde_json::from_str::<TrainingCropUploadMeta>(r#"{"kind":"map_name"}"#);
        assert!(parsed.is_err());
        let parsed = serde_json::from_str::<TrainingCropUploadMeta>(
            r#"{"found_spot":{"x":1,"y":1,"w":1,"h":1,"z":1}}"#,
        );
        assert!(parsed.is_err());
        let parsed: TrainingCropUploadMeta = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, TrainingCropUploadMeta::default());
        let parsed = serde_json::from_str::<TrainingCropUploadMeta>(r#"{"slot_index":-1}"#);
        assert!(parsed.is_err(), "negative slot index");
    }

    #[test]
    fn split_keeps_every_field() {
        let (hero, meta) = full().split();
        assert_eq!(hero.as_deref(), Some("ana"));
        assert_eq!(meta.screen.as_deref(), Some("scoreboard"));
        assert_eq!(meta.found_confidence, Some(0.8));
        assert_eq!(meta.screen_width, Some(1920));
    }

    #[test]
    fn consent_update_refuses_unknown_kinds() {
        let parsed = serde_json::from_str::<TrainingCropConsentUpdate>(r#"{"replays":true}"#);
        assert!(parsed.is_err());
        // full_screen consent cannot be stored.
        let parsed = serde_json::from_str::<TrainingCropConsentUpdate>(r#"{"full_screen":true}"#);
        assert!(parsed.is_err());
        let parsed = serde_json::from_str::<TrainingCropConsentUpdate>(r#"{"full_screen":false}"#);
        assert!(parsed.is_err());
        let parsed: TrainingCropConsentUpdate =
            serde_json::from_str(r#"{"map_name":true}"#).unwrap();
        assert_eq!(parsed.map_name, Some(true));
        assert_eq!(parsed.hero_icon, None);
    }

    #[test]
    fn consent_has_no_full_screen_field() {
        let all = TrainingCropConsent {
            hero_icon: true,
            stat_digits: true,
            map_name: true,
        };
        assert!(!all.get(CropKind::FullScreen));
        let json = serde_json::to_value(all).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "hero_icon": true, "stat_digits": true, "map_name": true })
        );
    }
}
