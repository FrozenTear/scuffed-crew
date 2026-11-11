//! The upload wire mapping: a stored [`PersonalMatch`] to the site's
//! `StatsUploadEntry`.

use std::ops::{Deref, DerefMut};

use crate::storage::PersonalMatch;

use scuffed_types::api::{RankRole, StatsUploadEntry};
use serde::Serialize;

use super::bans_picks::{UploadBansPicks, upload_bans_picks};

/// One uploaded match: the site's [`StatsUploadEntry`] and the bans and
/// picks the tracker read for the game. The key is left out when nothing
/// was read, so an older server, which ignores a field it does not know,
/// and a game with no read see the same body as before.
#[derive(Debug, Clone, Serialize)]
pub struct UploadEntry {
    #[serde(flatten)]
    entry: StatsUploadEntry,
    #[serde(skip_serializing_if = "Option::is_none")]
    bans_picks: Option<UploadBansPicks>,
    /// What the `rank-v1` readers read for the game ([`upload_rank`]). Left
    /// out when they read nothing, so an older server, a game with the
    /// readers off and a game with no confident read see the same body as
    /// before.
    #[serde(skip_serializing_if = "Option::is_none")]
    rank: Option<UploadRank>,
}

/// One `rank-v1` field of the upload: the value and how sure the reader
/// was (0..1). The site reads it as `RankField`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UploadRankField<T> {
    pub value: T,
    pub confidence: f32,
}

/// The `rank` object of one uploaded match, the one the site reads with
/// `scuffed_types::api::resolve_rank`. The site replaces its stored `rank`
/// with this object on every upload of the game, so every `rank-v1` field
/// goes in this one struct, built by the one function [`upload_rank`]: the
/// rank update screen reader's fields (role, role_rank, division,
/// progress_pct, progress_change, change_tags, higher_than_pct, hero_sr,
/// hero_sr_change, next_mode_hint) are added to this struct and that
/// function, never to a second object or builder, which would wipe this
/// one's fields on the site. Each field is left out when it was not read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UploadRank {
    /// The rank update screen's fields ([`crate::model::RankUpdate`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<UploadRankField<RankRole>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_rank: Option<UploadRankField<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub division: Option<UploadRankField<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_pct: Option<UploadRankField<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_change: Option<UploadRankField<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_tags: Option<UploadRankField<Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub higher_than_pct: Option<UploadRankField<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hero_sr: Option<UploadRankField<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hero_sr_change: Option<UploadRankField<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_mode_hint: Option<UploadRankField<String>>,
    /// The queue read on the pre-game screens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_title: Option<UploadRankField<String>>,
    /// The reader id ([`RANK_RECOGNIZER_ID`]), sent with any field.
    pub rank_recognizer: &'static str,
}

/// The reader id sent as `rank.rank_recognizer`, for the queue title and
/// the rank update alike (`model::RANK_RECOGNIZER_ID`, which both readers
/// re-export as their `RECOGNIZER_ID`).
pub const RANK_RECOGNIZER_ID: &str = crate::model::RANK_RECOGNIZER_ID;

/// The game's `rank` object as the upload sends it: each field only when
/// the reader read it, and `None` (no key, the old body) when no field is
/// set.
///
/// The local `QueueTitle::mode_hint` is not sent as `rank.next_mode_hint`:
/// that site field belongs to the game whose rank screen was read (the
/// banner names the game after it), while the hint is stored on the game
/// after, once its read confirmed it. `rank.next_mode_hint` is the banner
/// word read on this game's own rank screen
/// (`PersonalMatch::rank_update`).
pub(super) fn upload_rank(m: &PersonalMatch) -> Option<UploadRank> {
    let queue_title = m.queue_title.as_ref().and_then(|t| {
        Some(UploadRankField {
            value: t.confident_value()?.to_string(),
            confidence: t.confidence,
        })
    });
    let open_queue = m.queue_title.as_ref().is_some_and(|t| t.is_open_queue());
    let r = m.rank_update.clone().unwrap_or_default();
    let rank = UploadRank {
        role: field(&r.role, |v| rank_role(v, open_queue)),
        role_rank: field(&r.role_rank, |v| text(v)),
        division: field(&r.division, |v| {
            u8::try_from(*v).ok().filter(|d| (1..=5).contains(d))
        }),
        progress_pct: field(&r.progress_pct, |v| i32::try_from(*v).ok()),
        progress_change: field(&r.progress_change, |v| i32::try_from(*v).ok()),
        change_tags: r.change_tags.as_ref().and_then(|t| {
            let ok = t.value.len() <= crate::model::RANK_MAX_CHANGE_TAGS
                && t.value.iter().all(|s| text(s).is_some());
            sure(t.confidence)
                .filter(|_| ok)
                .map(|confidence| UploadRankField {
                    value: t.value.clone(),
                    confidence,
                })
        }),
        higher_than_pct: field(&r.higher_than_pct, |v| {
            u8::try_from(*v).ok().filter(|p| *p <= 100)
        }),
        hero_sr: field(&r.hero_sr, |v| u32::try_from(*v).ok()),
        hero_sr_change: field(&r.hero_sr_change, |v| i32::try_from(*v).ok()),
        next_mode_hint: field(&r.next_mode_hint, |v| text(v)),
        queue_title,
        rank_recognizer: RANK_RECOGNIZER_ID,
    };
    (rank != UploadRank::none()).then_some(rank)
}

impl UploadRank {
    /// No field read: what [`upload_rank`] never sends.
    pub(super) fn none() -> Self {
        Self {
            role: None,
            role_rank: None,
            division: None,
            progress_pct: None,
            progress_change: None,
            change_tags: None,
            higher_than_pct: None,
            hero_sr: None,
            hero_sr_change: None,
            next_mode_hint: None,
            queue_title: None,
            rank_recognizer: RANK_RECOGNIZER_ID,
        }
    }
}

/// A stored rank value with a confidence the site keeps (finite, above 0,
/// at most 1), mapped by `map`; `None` drops the field.
fn field<V: RankValue, T>(
    stored: &Option<V>,
    map: impl Fn(&V::Value) -> Option<T>,
) -> Option<UploadRankField<T>> {
    let s = stored.as_ref()?;
    Some(UploadRankField {
        confidence: sure(s.confidence())?,
        value: map(s.value())?,
    })
}

fn sure(confidence: f32) -> Option<f32> {
    (confidence.is_finite() && confidence > 0.0 && confidence <= 1.0).then_some(confidence)
}

/// Text the site keeps: trimmed, not empty, at most
/// `scuffed_types::api::RANK_STRING_MAX_CHARS` characters.
fn text(v: &str) -> Option<String> {
    let v = v.trim();
    (!v.is_empty() && v.chars().count() <= scuffed_types::api::RANK_STRING_MAX_CHARS)
        .then(|| v.to_string())
}

/// The stored role as the site's [`RankRole`]. `Open` only when the game's
/// queue title names open queue: the reader reads it from a missing role
/// line, which another screen state can show too.
fn rank_role(v: &str, open_queue: bool) -> Option<RankRole> {
    match v {
        "Tank" => Some(RankRole::Tank),
        "Damage" => Some(RankRole::Damage),
        "Support" => Some(RankRole::Support),
        "Open" if open_queue => Some(RankRole::Open),
        _ => None,
    }
}

/// A stored rank field: its value and confidence.
trait RankValue {
    type Value: ?Sized;
    fn value(&self) -> &Self::Value;
    fn confidence(&self) -> f32;
}

impl RankValue for crate::model::RankText {
    type Value = String;
    fn value(&self) -> &String {
        &self.value
    }
    fn confidence(&self) -> f32 {
        self.confidence
    }
}

impl RankValue for crate::model::RankNumber {
    type Value = i64;
    fn value(&self) -> &i64 {
        &self.value
    }
    fn confidence(&self) -> f32 {
        self.confidence
    }
}

impl Deref for UploadEntry {
    type Target = StatsUploadEntry;

    fn deref(&self) -> &StatsUploadEntry {
        &self.entry
    }
}

impl DerefMut for UploadEntry {
    fn deref_mut(&mut self) -> &mut StatsUploadEntry {
        &mut self.entry
    }
}

/// The body `upload_matches` posts: the site's `StatsUploadRequest` with
/// [`UploadEntry`] matches.
#[derive(Debug, Clone, Serialize)]
pub struct UploadRequest {
    pub matches: Vec<UploadEntry>,
    pub deleted_sessions: Vec<String>,
}

/// True when this row must stay on the machine until the member picks
/// a map or hero. See [`crate::parse::review_suspect_fields`].
pub fn row_needs_review(row: &PersonalMatch) -> bool {
    !crate::parse::review_suspect_fields(row.display_map_name(), &row.game_mode, row.display_hero())
        .is_empty()
}

/// The JSON body `upload_matches` posts. A pure function so tests can pin
/// the exact bytes (for example, that shadow mode leaves them unchanged).
///
/// A row whose map is empty or unrecognised, whose sent mode would be
/// empty, or whose hero is `Unknown` is left out. The server stores a
/// required string for `map_name` and `hero`, so an empty map and the
/// literal `Unknown` are what showed up in stats. Null and omitted `hero`
/// both fail decode, so the match is omitted instead of either of those.
pub fn upload_request(matches: &[PersonalMatch], deleted_sessions: &[String]) -> UploadRequest {
    let entries: Vec<UploadEntry> = matches
        .iter()
        .filter(|m| !row_needs_review(m))
        // Upload the effective (corrected-if-present, else OCR) values so
        // server aggregates and the leaderboard reflect manual fixes, and
        // flag edited rows for the site badge. The immutable OCR reads stay
        // local; the transparency detail lives in the tracker GUI.
        .filter_map(|m| {
            let hero = m.display_hero().to_string();
            let map_name = m.display_map_name().to_string();
            let game_mode = crate::parse::uploaded_game_mode(&map_name, &m.game_mode);
            if crate::parse::upload_identity_blank(&map_name, &game_mode, &hero) {
                return None;
            }
            let entry = StatsUploadEntry {
                session_id: m.session_id.clone(),
                hero,
                map_name,
                game_mode,
                role: m.display_role().to_string(),
                outcome: m.display_outcome().to_string(),
                elims: m.display_elims(),
                deaths: m.display_deaths(),
                assists: m.display_assists(),
                damage: m.display_damage(),
                healing: m.display_healing(),
                mitigation: m.display_mitigation(),
                played_at: chrono::DateTime::<chrono::Utc>::from(m.played_at),
                edited: m.is_edited(),
                // A string on every row, including a full fallback's ocr-v1.
                // None is what an older client omits; this client does not.
                recognizer: Some(m.stored_recognizer().to_string()),
                suspect_fields: crate::suspect::upload_suspect_fields(&m.suspect_fields),
                // None omits the key. An older server that ignores
                // name_recognizer still accepts the rest of the row.
                name_recognizer: m
                    .name_recognizer
                    .as_deref()
                    .filter(|id| !id.is_empty())
                    .map(str::to_string),
            };
            Some(UploadEntry {
                entry,
                // Plain values only, None (no key) when nothing was read.
                bans_picks: upload_bans_picks(m.bans_picks.as_deref()),
                // Every rank-v1 field, or no key.
                rank: upload_rank(m),
            })
        })
        .collect();
    UploadRequest {
        matches: entries,
        deleted_sessions: deleted_sessions.to_vec(),
    }
}
