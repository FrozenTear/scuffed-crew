//! A game's rank slot (queue and role) and the newest rank per slot.

use super::*;
use crate::model::{PersonalMatch, QueueTitle, RankText, RankUpdate};
use crate::sync::upload_request;
use chrono::{DateTime, Duration, Utc};
use scuffed_types::api::{RankQueue, RankRole};
use serde_json::Value;

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-12-08T20:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// A stored Competitive game the upload sends: `title` read at 0.93 (a
/// confident read), `size` players a side, and a rank screen with `role`
/// (at 0.9) and `tier`. `None` leaves each out.
fn row(
    title: Option<&str>,
    size: Option<u8>,
    role: Option<&str>,
    tier: Option<&str>,
) -> PersonalMatch {
    let mut m: PersonalMatch = serde_json::from_value(serde_json::json!({
        "hero": "Ana",
        "map_name": "Busan",
        "game_mode": "Control",
        "role": "Support",
        "outcome": "victory",
        "elims": 1, "deaths": 0, "assists": 0,
        "damage": 1, "healing": 0, "mitigation": 0,
        "played_at": "2026-12-08T20:00:00Z",
        "session_id": "g",
    }))
    .expect("a stored row");
    m.queue_title = title.map(|t| QueueTitle {
        value: Some(t.into()),
        confidence: 0.93,
        ..QueueTitle::default()
    });
    m.team_size = size;
    let text = |v: &str| RankText {
        value: v.into(),
        confidence: 0.9,
    };
    let rank = RankUpdate {
        role: role.map(text),
        role_rank: tier.map(text),
        ..RankUpdate::default()
    };
    m.rank_update = (!rank.is_empty()).then_some(rank);
    m
}

fn game_at(m: &PersonalMatch, minutes: i64) -> Game {
    let mut g = Game::from_match(m);
    g.played_at = t0() + Duration::minutes(minutes);
    g
}

/// What the upload body says for the row: `rank.queue.value`,
/// `rank.queue.confidence` and `rank.role.value`, or `None` with no `rank`.
fn sent(m: &PersonalMatch) -> Option<(Value, Value, Value)> {
    let body = serde_json::to_value(upload_request(std::slice::from_ref(m), &[])).unwrap();
    let rank = body["matches"][0].get("rank")?.clone();
    Some((
        rank["queue"]["value"].clone(),
        rank["queue"]["confidence"].clone(),
        rank["role"]["value"].clone(),
    ))
}

/// Title, team size, role line, and the queue the slot should have.
type Case<'a> = (
    Option<&'a str>,
    Option<u8>,
    Option<&'a str>,
    Option<RankQueue>,
);

/// The local slot says exactly what the upload sends, on 5-a-side and
/// 6-a-side (12-player) boards, for titles in any word order, the R1 and R2
/// rules, and games with no Competitive evidence.
#[test]
fn the_slot_is_the_queue_and_role_the_upload_sends() {
    let cases: [Case; 16] = [
        (
            Some("ROLE QUEUE"),
            Some(5),
            Some("Tank"),
            Some(RankQueue::Role5v5),
        ),
        (
            Some("ROLE QUEUE"),
            Some(6),
            Some("Tank"),
            Some(RankQueue::Role6v6),
        ),
        (
            Some("6V6 ROLE QUEUE"),
            Some(6),
            Some("Support"),
            Some(RankQueue::Role6v6),
        ),
        (
            Some("ROLE QUEUE 6V6"),
            None,
            Some("Damage"),
            Some(RankQueue::Role6v6),
        ),
        (
            Some("COMPETITIVE: ROLE QUEUE"),
            Some(6),
            Some("Tank"),
            Some(RankQueue::Role6v6),
        ),
        (
            Some("OPEN QUEUE"),
            Some(6),
            Some("Open"),
            Some(RankQueue::Open6v6),
        ),
        (
            Some("OPEN QUEUE"),
            Some(5),
            Some("Open"),
            Some(RankQueue::Open5v5),
        ),
        (
            Some("COMPETITIVE: 6V6 OPEN QUEUE"),
            None,
            Some("Open"),
            Some(RankQueue::Open6v6),
        ),
        // No title, 12-player board, role icon: role queue 6v6.
        (None, Some(6), Some("Tank"), Some(RankQueue::Role6v6)),
        // R1: an Open line alone decides nothing.
        (None, Some(6), Some("Open"), Some(RankQueue::Unknown)),
        // No size from board or title: unknown, never joins a known queue.
        (
            Some("ROLE QUEUE"),
            None,
            Some("Tank"),
            Some(RankQueue::Unknown),
        ),
        // Board and title sizes disagree: unknown.
        (
            Some("6V6 ROLE QUEUE"),
            Some(5),
            Some("Tank"),
            Some(RankQueue::Unknown),
        ),
        // A title-only rank object still has a slot (queue, no role).
        (Some("ROLE QUEUE"), Some(6), None, Some(RankQueue::Role6v6)),
        // R2: not Competitive, even with a rank screen read.
        (Some("ROLE QUEUE: QUICK PLAY"), Some(6), Some("Tank"), None),
        (
            Some("OPEN QUEUE: 6V6 QUICK PLAY"),
            Some(6),
            Some("Open"),
            None,
        ),
        // R8: a title naming no queue and no rank screen read.
        (Some("COMPETITIVE"), Some(6), None, None),
    ];
    for (title, size, role, want) in cases {
        let m = row(title, size, role, role.map(|_| "GOLD"));
        let slot = Game::from_match(&m).rank_slot;
        let case = format!("{title:?} size={size:?} role={role:?}");
        assert_eq!(slot.map(|s| s.queue), want, "{case}");
        match (slot, sent(&m)) {
            (None, None) => {}
            (Some(s), Some((queue, confidence, role))) => {
                assert_eq!(serde_json::to_value(s.queue).unwrap(), queue, "{case}");
                let conf = s.queue_confidence.map(|c| serde_json::json!(c));
                assert_eq!(conf.unwrap_or(Value::Null), confidence, "{case}");
                let r = s.role.map(|r| serde_json::to_value(r).unwrap());
                assert_eq!(r.unwrap_or(Value::Null), role, "{case}");
            }
            (slot, body) => panic!("{case}: slot {slot:?} but upload {body:?}"),
        }
    }
}

/// A later Tab that fills in the team size moves the game's slot from
/// unknown to role queue 6v6: the slot is derived, never frozen.
#[test]
fn a_later_team_size_moves_the_slot_off_unknown() {
    let mut m = row(Some("ROLE QUEUE"), None, Some("Tank"), Some("GOLD"));
    let before = Game::from_match(&m).rank_slot.unwrap();
    assert_eq!(before.queue, RankQueue::Unknown);
    assert_eq!(before.queue_confidence, None);
    m.team_size = Some(6);
    let after = Game::from_match(&m).rank_slot.unwrap();
    assert_eq!(after.queue, RankQueue::Role6v6);
    assert_eq!(after.queue_confidence, Some(0.93));
    assert_eq!(after.role, Some(RankRole::Tank));
}

fn tier(g: &Game) -> &str {
    &g.rank_update
        .as_ref()
        .unwrap()
        .role_rank
        .as_ref()
        .unwrap()
        .value
}

/// Ranks are kept per queue and role. The newest rank of a slot replaces
/// only an older rank of that same slot; 6v6 role, 5v5 role, open and
/// unknown queue ranks each stay; a game with no rank screen read or no
/// slot is left out.
#[test]
fn the_newest_rank_is_kept_per_queue_and_role() {
    let games = [
        // 5v5 role Tank, then a newer 5v5 role Tank: the newer one stays.
        game_at(
            &row(Some("ROLE QUEUE"), Some(5), Some("Tank"), Some("SILVER")),
            0,
        ),
        game_at(
            &row(Some("ROLE QUEUE"), Some(5), Some("Tank"), Some("DIAMOND")),
            10,
        ),
        // 6v6 role Tank, newest of all: its own slot.
        game_at(
            &row(Some("ROLE QUEUE"), Some(6), Some("Tank"), Some("GOLD")),
            30,
        ),
        // 6v6 role Support: another slot.
        game_at(
            &row(
                Some("ROLE QUEUE"),
                Some(6),
                Some("Support"),
                Some("PLATINUM"),
            ),
            20,
        ),
        // Open 6v6.
        game_at(
            &row(Some("OPEN QUEUE"), Some(6), Some("Open"), Some("MASTER")),
            5,
        ),
        // Unknown queue, Tank: never joins a known Tank slot.
        game_at(
            &row(Some("ROLE QUEUE"), None, Some("Tank"), Some("BRONZE")),
            40,
        ),
        // Quick Play with a rank read, and a title-only game: left out.
        game_at(
            &row(
                Some("ROLE QUEUE: QUICK PLAY"),
                Some(6),
                Some("Tank"),
                Some("GRANDMASTER"),
            ),
            50,
        ),
        game_at(&row(Some("ROLE QUEUE"), Some(6), None, None), 60),
    ];
    let latest = latest_rank_per_slot(&games);
    let got: Vec<_> = latest
        .iter()
        .map(|g| {
            let s = g.rank_slot.unwrap();
            (s.queue, s.role, tier(g))
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (RankQueue::Unknown, Some(RankRole::Tank), "BRONZE"),
            (RankQueue::Role6v6, Some(RankRole::Tank), "GOLD"),
            (RankQueue::Role6v6, Some(RankRole::Support), "PLATINUM"),
            (RankQueue::Role5v5, Some(RankRole::Tank), "DIAMOND"),
            (RankQueue::Open6v6, Some(RankRole::Open), "MASTER"),
        ]
    );
}

/// Within one slot the order games are listed in does not matter: the
/// newest wins either way, and a tie keeps the one listed first.
#[test]
fn the_newest_of_a_slot_wins_in_any_order() {
    let old = game_at(
        &row(Some("ROLE QUEUE"), Some(6), Some("Tank"), Some("SILVER")),
        0,
    );
    let new = game_at(
        &row(Some("ROLE QUEUE"), Some(6), Some("Tank"), Some("GOLD")),
        10,
    );
    for games in [[old.clone(), new.clone()], [new.clone(), old.clone()]] {
        let latest = latest_rank_per_slot(&games);
        assert_eq!(latest.len(), 1);
        assert_eq!(tier(latest[0]), "GOLD");
    }
    let tie = game_at(
        &row(Some("ROLE QUEUE"), Some(6), Some("Tank"), Some("BRONZE")),
        10,
    );
    assert_eq!(
        tier(latest_rank_per_slot(&[new.clone(), tie.clone()])[0]),
        "GOLD"
    );
    assert_eq!(tier(latest_rank_per_slot(&[tie, new])[0]), "BRONZE");
}
