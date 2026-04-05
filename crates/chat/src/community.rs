//! Community event builders for custom Nostr event kinds.
//!
//! Custom kinds for gaming community features:
//! - Kind 30400: LFG (Looking For Group) requests
//! - Kind 30401: RSVP to events
//! - Kind 30402: Match results / highlights
//! - Kind 30403: Event announcements
//!
//! All are parameterized replaceable events (30000-39999 range per NIP-01).

use nostr::key::Keys;
use serde::{Deserialize, Serialize};

use crate::nostr::events::{EventBuilder, EventError};
use scuffed_types::nostr::NostrEvent;

/// Custom Nostr event kinds for community features.
pub mod kinds {
    /// LFG (Looking For Group) request.
    pub const LFG: u32 = 30400;
    /// RSVP to a scheduled event.
    pub const RSVP: u32 = 30401;
    /// Match result / highlight.
    pub const MATCH_RESULT: u32 = 30402;
    /// Event announcement.
    pub const EVENT_ANNOUNCEMENT: u32 = 30403;
}

/// LFG request metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LfgRequest {
    /// Game name (e.g., "Overwatch 2").
    pub game: String,
    /// Rank/tier (e.g., "Diamond", "Gold").
    pub rank: Option<String>,
    /// Number of players needed.
    pub players_needed: u8,
    /// When the session starts (ISO 8601 or relative like "now").
    pub start_time: Option<String>,
    /// NIP-29 group ID to post in.
    pub group_id: String,
    /// Freeform description.
    pub description: String,
}

/// Match result summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    /// Game played.
    pub game: String,
    /// "win", "loss", or "draw".
    pub outcome: String,
    /// Score (e.g., "3-1").
    pub score: Option<String>,
    /// NIP-29 group ID.
    pub group_id: String,
    /// Summary / highlights.
    pub summary: String,
}

/// Event announcement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventAnnouncement {
    /// Event title.
    pub title: String,
    /// Event description.
    pub description: String,
    /// When the event starts (ISO 8601).
    pub start_time: String,
    /// NIP-29 group ID.
    pub group_id: String,
    /// "d" tag for replaceable event dedup.
    pub event_id: String,
}

/// Build a LFG event (kind 30400).
pub fn build_lfg_event(keys: &Keys, lfg: &LfgRequest) -> Result<NostrEvent, EventError> {
    let content = serde_json::to_string(lfg).map_err(|e| EventError::SerializationFailed(e.to_string()))?;

    let mut tags = vec![
        vec!["h".into(), lfg.group_id.clone()],
        vec!["d".into(), format!("lfg-{}", chrono::Utc::now().timestamp())],
        vec!["game".into(), lfg.game.clone()],
        vec!["players_needed".into(), lfg.players_needed.to_string()],
    ];

    if let Some(ref rank) = lfg.rank {
        tags.push(vec!["rank".into(), rank.clone()]);
    }
    if let Some(ref start) = lfg.start_time {
        tags.push(vec!["start".into(), start.clone()]);
    }

    let event = EventBuilder::build_custom_event(keys, kinds::LFG as u16, &content, tags)?;
    Ok(EventBuilder::to_relay_event(&event))
}

/// Build a match result event (kind 30402).
pub fn build_match_result_event(
    keys: &Keys,
    result: &MatchResult,
) -> Result<NostrEvent, EventError> {
    let content =
        serde_json::to_string(result).map_err(|e| EventError::SerializationFailed(e.to_string()))?;

    let mut tags = vec![
        vec!["h".into(), result.group_id.clone()],
        vec![
            "d".into(),
            format!("match-{}", chrono::Utc::now().timestamp()),
        ],
        vec!["game".into(), result.game.clone()],
        vec!["outcome".into(), result.outcome.clone()],
    ];

    if let Some(ref score) = result.score {
        tags.push(vec!["score".into(), score.clone()]);
    }

    let event = EventBuilder::build_custom_event(keys, kinds::MATCH_RESULT as u16, &content, tags)?;
    Ok(EventBuilder::to_relay_event(&event))
}

/// Build an event announcement (kind 30403).
pub fn build_event_announcement(
    keys: &Keys,
    announcement: &EventAnnouncement,
) -> Result<NostrEvent, EventError> {
    let content = serde_json::to_string(announcement)
        .map_err(|e| EventError::SerializationFailed(e.to_string()))?;

    let tags = vec![
        vec!["h".into(), announcement.group_id.clone()],
        vec!["d".into(), announcement.event_id.clone()],
        vec!["title".into(), announcement.title.clone()],
        vec!["start".into(), announcement.start_time.clone()],
    ];

    let event =
        EventBuilder::build_custom_event(keys, kinds::EVENT_ANNOUNCEMENT as u16, &content, tags)?;
    Ok(EventBuilder::to_relay_event(&event))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lfg_event_has_correct_kind_and_tags() {
        let keys = Keys::generate();
        let lfg = LfgRequest {
            game: "Overwatch 2".into(),
            rank: Some("Diamond".into()),
            players_needed: 3,
            start_time: Some("2026-04-05T20:00:00Z".into()),
            group_id: "team-alpha".into(),
            description: "Need 3 for ranked".into(),
        };

        let event = build_lfg_event(&keys, &lfg).unwrap();
        assert_eq!(event.kind, kinds::LFG);
        assert_eq!(event.group_id(), Some("team-alpha"));
        assert_eq!(event.tag_value("game"), Some("Overwatch 2"));
        assert_eq!(event.tag_value("players_needed"), Some("3"));
        assert_eq!(event.tag_value("rank"), Some("Diamond"));
    }

    #[test]
    fn match_result_event_structure() {
        let keys = Keys::generate();
        let result = MatchResult {
            game: "Valorant".into(),
            outcome: "win".into(),
            score: Some("13-7".into()),
            group_id: "team-bravo".into(),
            summary: "Clean sweep on Ascent".into(),
        };

        let event = build_match_result_event(&keys, &result).unwrap();
        assert_eq!(event.kind, kinds::MATCH_RESULT);
        assert_eq!(event.tag_value("outcome"), Some("win"));
        assert_eq!(event.tag_value("score"), Some("13-7"));
    }

    #[test]
    fn event_announcement_structure() {
        let keys = Keys::generate();
        let announcement = EventAnnouncement {
            title: "Friday Night Customs".into(),
            description: "Weekly custom games".into(),
            start_time: "2026-04-05T20:00:00Z".into(),
            group_id: "team-alpha".into(),
            event_id: "friday-customs-2026-04-05".into(),
        };

        let event = build_event_announcement(&keys, &announcement).unwrap();
        assert_eq!(event.kind, kinds::EVENT_ANNOUNCEMENT);
        assert_eq!(event.tag_value("title"), Some("Friday Night Customs"));
        assert_eq!(
            event.tag_value("d"),
            Some("friday-customs-2026-04-05")
        );
    }
}
