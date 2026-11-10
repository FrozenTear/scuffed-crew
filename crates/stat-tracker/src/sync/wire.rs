//! The upload wire mapping: a stored [`PersonalMatch`] to the site's
//! `StatsUploadEntry`.

use std::ops::{Deref, DerefMut};

use crate::storage::PersonalMatch;

use scuffed_types::api::StatsUploadEntry;
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
    /// The queue read on the pre-game screens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_title: Option<UploadRankField<String>>,
    /// The reader id ([`RANK_RECOGNIZER_ID`]), sent with any field.
    pub rank_recognizer: &'static str,
}

/// The reader id sent as `rank.rank_recognizer`: the queue title reader's
/// `RECOGNIZER_ID` (`shadow::queue_title`, defined in `model`).
pub const RANK_RECOGNIZER_ID: &str = crate::model::QUEUE_TITLE_RECOGNIZER_ID;

/// The game's `rank` object as the upload sends it: each field only when
/// the reader was confident, and `None` (no key, the old body) when no
/// field is set.
///
/// The local `QueueTitle::mode_hint` is not sent as `rank.next_mode_hint`:
/// that site field belongs to the game whose rank screen was read (the
/// banner names the game after it), while the hint is stored on the game
/// after, once its read confirmed it. The rank reader's wiring sends the
/// banner word on its own game.
pub(super) fn upload_rank(m: &PersonalMatch) -> Option<UploadRank> {
    let queue_title = m.queue_title.as_ref().and_then(|t| {
        Some(UploadRankField {
            value: t.confident_value()?.to_string(),
            confidence: t.confidence,
        })
    });
    queue_title.as_ref()?;
    Some(UploadRank {
        queue_title,
        rank_recognizer: RANK_RECOGNIZER_ID,
    })
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
