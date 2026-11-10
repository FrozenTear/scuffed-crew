//! The bans and picks part of the upload: the stored cv-v1 JSON
//! (`PersonalMatch::bans_picks`, written by the daemon) as the plain-value
//! `bans_picks` object the site's upload takes.
//!
//! The stored JSON keeps every value as `{value, confidence, score, margin}`.
//! The upload sends the values only: a hero pack key, the literal
//! `no_preference` (picks and ban vote) or null. No confidence, score,
//! margin or layout leaves the machine.
//!
//! The reader names its no-preference class `no-preference` (the template
//! file) and writes `no_preference`; the wire accepts only the underscore
//! form, so either spelling is sent as `no_preference`. A ban or the lobby
//! ban is a hero or nothing: `no_preference` there is sent as null.
//!
//! Nothing read, which is no stored result, one that does not parse, or one
//! with no value in it, gives `None`, and the upload omits the key.
//!
//! `ban_vote` is the local player's three ban votes in slot order (a hero
//! key, or null for a skipped, empty or unread slot). Two optional keys
//! ride along when the stored result has them: `ban_votes`, each slot's
//! state (`hero`, `skipped`, `empty` or null) and hero, and `ban_score`,
//! the TEAM BAN SCORE strip (rank, hero, vote count, empty). They are left
//! out when not read, and a server that does not know them ignores them
//! (`scuffed_types::api::resolve_bans_picks` reads only the keys it
//! knows). They never decide whether the `bans_picks` key is sent.

use serde::Serialize;
use serde_json::Value;
// The limits and the no preference literal are the server's, from the shared
// contract, so the two sides cannot drift.
use scuffed_types::{
    BANS_PICKS_MAX_BAN_VOTE, BANS_PICKS_MAX_BANS, BANS_PICKS_MAX_PICKS, BANS_PICKS_NO_PREFERENCE,
};

/// The reader's template name for the same icon.
const NO_PREFERENCE_TEMPLATE: &str = "no-preference";
/// The reader the stored column comes from, when the JSON does not say.
const RECOGNIZER: &str = "cv-v1";

/// One ban: the hero and the side that made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct UploadBan {
    pub hero: Option<String>,
    /// `own`, `enemy` or null.
    pub team: Option<&'static str>,
}

/// One ban vote slot: whose, which, what it showed, and the hero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct UploadVoteSlot {
    /// 0 for the local player.
    pub voter: u8,
    /// 1..=3.
    pub slot: u8,
    /// `hero`, `skipped`, `empty` or null (unread).
    pub state: Option<&'static str>,
    pub hero: Option<String>,
}

/// One box of the TEAM BAN SCORE strip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct UploadBanScore {
    /// 1..=3, most votes first.
    pub rank: u8,
    pub hero: Option<String>,
    pub votes: Option<u32>,
    /// The box showed no hero yet.
    pub empty: bool,
}

/// The `bans_picks` object of one uploaded match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct UploadBansPicks {
    pub bans: Vec<UploadBan>,
    pub lobby_ban: Option<String>,
    pub picks: Vec<Option<String>>,
    pub ban_vote: Vec<Option<String>>,
    /// Optional, newer than the server contract: left out when not read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ban_votes: Option<Vec<UploadVoteSlot>>,
    /// Optional, newer than the server contract: left out when not read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ban_score: Option<Vec<UploadBanScore>>,
    pub recognizer: String,
}

/// At most this many vote slots and strip boxes are sent.
const MAX_VOTE_SLOTS: usize = 18;
const MAX_SCORE_BOXES: usize = 3;

/// The value a stored read carries: its `value` field, or the entry itself
/// when it is already a plain string. Empty is no value.
fn read_value(entry: &Value) -> Option<&str> {
    let raw = match entry {
        Value::Object(read) => read.get("value")?.as_str()?,
        Value::String(plain) => plain,
        _ => return None,
    };
    (!raw.trim().is_empty()).then_some(raw)
}

/// A pick or ban vote entry: a hero key or `no_preference`.
fn choice(entry: &Value) -> Option<String> {
    read_value(entry).map(|raw| match raw {
        BANS_PICKS_NO_PREFERENCE | NO_PREFERENCE_TEMPLATE => BANS_PICKS_NO_PREFERENCE.to_string(),
        hero => hero.to_string(),
    })
}

/// A ban or the lobby ban: a hero key only.
fn hero(entry: &Value) -> Option<String> {
    choice(entry).filter(|value| value != BANS_PICKS_NO_PREFERENCE)
}

fn entries<'a>(stored: &'a Value, key: &str, max: usize) -> &'a [Value] {
    let all = stored
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    &all[..all.len().min(max)]
}

/// A small whole number field.
fn small(entry: &Value, key: &str) -> Option<u8> {
    entry.get(key)?.as_u64().and_then(|n| u8::try_from(n).ok())
}

/// The stored `ban_votes` list, or `None` when it is missing or null.
fn vote_slots(stored: &Value) -> Option<Vec<UploadVoteSlot>> {
    let all = stored.get("ban_votes")?.as_array()?;
    Some(
        all.iter()
            .take(MAX_VOTE_SLOTS)
            .filter_map(|e| {
                Some(UploadVoteSlot {
                    voter: small(e, "voter")?,
                    slot: small(e, "slot")?,
                    state: match e.get("state").and_then(Value::as_str) {
                        Some("hero") => Some("hero"),
                        Some("skipped") => Some("skipped"),
                        Some("empty") => Some("empty"),
                        _ => None,
                    },
                    hero: e.get("hero").and_then(hero),
                })
            })
            .collect(),
    )
}

/// The stored `ban_score` list, or `None` when it is missing or null.
fn score_boxes(stored: &Value) -> Option<Vec<UploadBanScore>> {
    let all = stored.get("ban_score")?.as_array()?;
    Some(
        all.iter()
            .take(MAX_SCORE_BOXES)
            .filter_map(|e| {
                Some(UploadBanScore {
                    rank: small(e, "rank")?,
                    hero: e.get("hero").and_then(hero),
                    votes: e
                        .get("votes")
                        .and_then(Value::as_u64)
                        .and_then(|n| u32::try_from(n).ok()),
                    empty: e.get("empty").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .collect(),
    )
}

fn ban(entry: &Value) -> UploadBan {
    UploadBan {
        hero: hero(entry),
        team: match entry.get("team").and_then(Value::as_str) {
            Some("own") => Some("own"),
            Some("enemy") => Some("enemy"),
            _ => None,
        },
    }
}

/// The upload object for a stored result, or `None` when nothing was read.
pub(super) fn upload_bans_picks(stored: Option<&str>) -> Option<UploadBansPicks> {
    let stored: Value = serde_json::from_str(stored?).ok()?;
    if !stored.is_object() {
        return None;
    }
    let out = UploadBansPicks {
        bans: entries(&stored, "bans", BANS_PICKS_MAX_BANS)
            .iter()
            .map(ban)
            .collect(),
        lobby_ban: stored.get("lobby_ban").and_then(hero),
        picks: entries(&stored, "picks", BANS_PICKS_MAX_PICKS)
            .iter()
            .map(choice)
            .collect(),
        ban_vote: entries(&stored, "ban_vote", BANS_PICKS_MAX_BAN_VOTE)
            .iter()
            .map(choice)
            .collect(),
        ban_votes: vote_slots(&stored),
        ban_score: score_boxes(&stored),
        recognizer: stored
            .get("bans_picks_recognizer")
            .and_then(Value::as_str)
            .filter(|id| scuffed_types::is_recognizer_id(id))
            .unwrap_or(RECOGNIZER)
            .to_string(),
    };
    let read_something = out.bans.iter().any(|b| b.hero.is_some())
        || out.lobby_ban.is_some()
        || out.picks.iter().any(Option::is_some)
        || out.ban_vote.iter().any(Option::is_some);
    read_something.then_some(out)
}
