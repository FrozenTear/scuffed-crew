//! Tests of `rank.queue` on the upload (T15, #654): the five values, a
//! game with no rank screen read still sends a rank object that holds the
//! queue when the title names role or open queue, a failed queue read is
//! `unknown` and never drops the game, and Quick Play / a game with the
//! readers off still omit `rank`.

use super::tests_wire::row;
use super::wire::{UploadQueueField, upload_rank};
use super::*;
use crate::model::{QueueTitle, RankNumber, RankText, RankUpdate};
use scuffed_types::api::{RankQueue, StatsUploadBody, resolve_rank};
use serde_json::{Value, json};

fn queue(value: &str) -> QueueTitle {
    QueueTitle {
        value: Some(value.to_string()),
        confidence: 0.93,
        ..QueueTitle::default()
    }
}

fn unsure() -> QueueTitle {
    QueueTitle {
        unsure: true,
        ..QueueTitle::default()
    }
}

fn game(
    title: Option<QueueTitle>,
    team_size: Option<u8>,
    rank: Option<RankUpdate>,
) -> crate::storage::PersonalMatch {
    let mut game = row("Ana", "Busan", "Control");
    game.queue_title = title;
    game.team_size = team_size;
    game.rank_update = rank;
    game
}

fn body_of(m: &crate::storage::PersonalMatch) -> String {
    serde_json::to_string(&upload_request(std::slice::from_ref(m), &[])).unwrap()
}

fn sent(m: &crate::storage::PersonalMatch) -> Value {
    let v: Value = serde_json::from_str(&body_of(m)).unwrap();
    v["matches"][0]["rank"].clone()
}

/// The JSON of one field as the upload sends it (the f32 confidence as its
/// shortest text, as the body has it).
fn f(value: Value, confidence: f32) -> Value {
    let confidence: Value =
        serde_json::from_str(&serde_json::to_string(&confidence).unwrap()).unwrap();
    json!({ "value": value, "confidence": confidence })
}

fn n(value: i64, confidence: f32) -> Option<RankNumber> {
    Some(RankNumber { value, confidence })
}

fn t(value: &str, confidence: f32) -> Option<RankText> {
    Some(RankText {
        value: value.to_string(),
        confidence,
    })
}

#[test]
fn open_queue_with_five_per_team_is_open_5v5() {
    let m = game(Some(queue("OPEN QUEUE")), Some(5), None);
    let rank = sent(&m);
    assert_eq!(rank["queue"], f(json!("open_5v5"), 0.93));
    assert_eq!(rank["queue_title"]["value"], json!("OPEN QUEUE"));
    // No rank screen read: the title names open queue, so the object still
    // holds the queue.
    assert!(rank.get("role").is_none(), "{rank}");
    let site = resolve_rank(Some(&rank)).expect("a rank object");
    assert_eq!(site.queue.value, Some(RankQueue::Open5v5));
    assert_eq!(site.queue_title.value.as_deref(), Some("OPEN QUEUE"));
}

#[test]
fn each_confident_title_and_size_maps_to_its_value() {
    let cases = [
        ("ROLE QUEUE", 5, "role_5v5"),
        ("ROLE QUEUE", 6, "role_6v6"),
        ("OPEN QUEUE", 5, "open_5v5"),
        ("OPEN QUEUE", 6, "open_6v6"),
        ("6V6 ROLE QUEUE", 6, "role_6v6"),
        ("COMPETITIVE: 6V6 OPEN QUEUE", 6, "open_6v6"),
    ];
    for (title, size, want) in cases {
        let rank = sent(&game(Some(queue(title)), Some(size), None));
        assert_eq!(rank["queue"]["value"], json!(want), "{title} {size}");
    }
    // Size can come from the title alone.
    let rank = sent(&game(Some(queue("ROLE QUEUE 6V6")), None, None));
    assert_eq!(rank["queue"]["value"], json!("role_6v6"));
}

#[test]
fn a_failed_queue_read_is_unknown_and_the_game_is_still_uploaded() {
    // A Competitive game (a rank screen field was read) whose title was
    // not seen: queue is unknown, the warning stays on the local title,
    // and the game is in the body.
    let m = game(
        Some(unsure()),
        Some(5),
        Some(RankUpdate {
            hero_sr: n(3412, 0.85),
            ..RankUpdate::default()
        }),
    );
    assert_eq!(
        m.queue_title.as_ref().and_then(|t| t.warning()),
        Some(crate::model::QUEUE_TITLE_WARNING)
    );
    let body = body_of(&m);
    let parsed: StatsUploadBody = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed.matches.len(), 1);
    assert_eq!(parsed.matches[0].entry.hero, "Ana");
    let rank = sent(&m);
    assert_eq!(rank["queue"]["value"], json!("unknown"));
    assert_eq!(rank["queue"]["confidence"], Value::Null);
    assert_eq!(rank["hero_sr"]["value"], json!(3412));
    let site = resolve_rank(Some(&rank)).expect("a rank object");
    assert_eq!(site.queue.value, Some(RankQueue::Unknown));
    assert_eq!(site.queue.confidence, None);
}

#[test]
fn a_title_with_no_team_size_is_unknown_and_the_game_is_still_uploaded() {
    let m = game(Some(queue("ROLE QUEUE")), None, None);
    let parsed: StatsUploadBody = serde_json::from_str(&body_of(&m)).unwrap();
    assert_eq!(parsed.matches.len(), 1);
    let rank = sent(&m);
    assert_eq!(rank["queue"]["value"], json!("unknown"));
    assert_eq!(rank["queue"]["confidence"], Value::Null);
    assert_eq!(rank["queue_title"]["value"], json!("ROLE QUEUE"));
}

#[test]
fn no_title_and_six_a_side_uses_the_rank_role_line() {
    let tank = game(
        None,
        Some(6),
        Some(RankUpdate {
            role: t("Tank", 0.9),
            hero_sr: n(3412, 0.85),
            ..RankUpdate::default()
        }),
    );
    assert_eq!(sent(&tank)["queue"]["value"], json!("role_6v6"));
    let open = game(
        None,
        Some(6),
        Some(RankUpdate {
            role: t("Open", 0.5),
            hero_sr: n(3412, 0.85),
            ..RankUpdate::default()
        }),
    );
    // Open is not sent as role without an open title, but it still decides
    // the queue.
    let rank = sent(&open);
    assert_eq!(rank["queue"]["value"], json!("open_6v6"));
    assert!(rank.get("role").is_none(), "{rank}");
    assert_eq!(rank["hero_sr"]["value"], json!(3412));
}

#[test]
fn readers_off_or_a_weak_title_alone_omit_the_rank_object() {
    let off = game(None, Some(5), None);
    assert!(upload_rank(&off).is_none());
    let body = body_of(&off);
    assert!(!body.contains("rank"), "{body}");
    let weak = game(Some(unsure()), Some(6), None);
    assert!(upload_rank(&weak).is_none());
    assert_eq!(body_of(&weak), body);
    // The game is still uploaded; only the rank object is left out.
    let parsed: StatsUploadBody = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed.matches.len(), 1);
}

#[test]
fn upload_rank_struct_carries_the_decided_queue() {
    let m = game(Some(queue("OPEN QUEUE")), Some(5), None);
    let rank = upload_rank(&m).expect("a rank object");
    assert_eq!(
        rank.queue,
        Some(UploadQueueField {
            value: RankQueue::Open5v5,
            confidence: Some(0.93),
        })
    );
}

#[test]
fn a_title_that_does_not_name_role_or_open_omits_the_rank_object() {
    for title in ["QUICK PLAY", "6V6 QUICK PLAY", "ARCADE", "COMPETITIVE"] {
        let m = game(Some(queue(title)), Some(6), None);
        assert!(upload_rank(&m).is_none(), "{title}");
        let body = body_of(&m);
        assert!(!body.contains("rank"), "{title}: {body}");
    }
}

#[test]
fn open_queue_in_any_word_order_drops_an_icon_role() {
    let m = game(
        Some(queue("OPEN QUEUE 6V6")),
        Some(6),
        Some(RankUpdate {
            role: t("Tank", 0.9),
            hero_sr: n(3412, 0.85),
            ..RankUpdate::default()
        }),
    );
    let rank = sent(&m);
    assert_eq!(rank["queue"]["value"], json!("open_6v6"));
    assert!(rank.get("role").is_none(), "{rank}");
    assert_eq!(rank["hero_sr"]["value"], json!(3412));
}

#[test]
fn an_unsure_role_line_does_not_decide_the_queue() {
    let m = game(
        None,
        Some(6),
        Some(RankUpdate {
            role: t("Tank", f32::NAN),
            hero_sr: n(3412, 0.85),
            ..RankUpdate::default()
        }),
    );
    let rank = sent(&m);
    assert_eq!(rank["queue"]["value"], json!("unknown"));
    assert_eq!(rank["queue"]["confidence"], Value::Null);
    assert!(rank.get("role").is_none(), "{rank}");
    assert_eq!(rank["hero_sr"]["value"], json!(3412));
}
