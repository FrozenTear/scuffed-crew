//! The upload wire mapping: a stored [`PersonalMatch`] to the site's
//! `StatsUploadEntry`.

use crate::storage::PersonalMatch;

use scuffed_types::api::{StatsUploadEntry, StatsUploadRequest};

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
pub fn upload_request(
    matches: &[PersonalMatch],
    deleted_sessions: &[String],
) -> StatsUploadRequest {
    let entries: Vec<StatsUploadEntry> = matches
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
            Some(StatsUploadEntry {
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
            })
        })
        .collect();
    StatsUploadRequest {
        matches: entries,
        deleted_sessions: deleted_sessions.to_vec(),
    }
}
