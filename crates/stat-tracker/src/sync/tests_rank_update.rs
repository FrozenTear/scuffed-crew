//! Tests of the rank update fields in the upload's `rank` object: each
//! field as `{value, confidence}` next to `queue_title` under the one
//! `rank_recognizer`, a field not read left out, no read at all the old
//! body, and the body read by the site's own `resolve_rank`.

use super::tests_wire::row;
use super::wire::{RANK_RECOGNIZER_ID, upload_rank};
use super::*;
use crate::model::{QueueTitle, RankNumber, RankTags, RankText, RankUpdate};
use scuffed_types::api::{RankRole, StatsUploadBody, resolve_rank};
use serde_json::{Value, json};

fn n(value: i64, confidence: f32) -> Option<RankNumber> {
    Some(RankNumber { value, confidence })
}

fn t(value: &str, confidence: f32) -> Option<RankText> {
    Some(RankText {
        value: value.to_string(),
        confidence,
    })
}

fn tags(count: usize) -> Option<RankTags> {
    Some(RankTags {
        value: (0..count).map(|k| format!("Tag {k}")).collect(),
        confidence: 0.7,
    })
}

fn full() -> RankUpdate {
    RankUpdate {
        role: t("Damage", 0.874),
        role_rank: t("Diamond", 0.81),
        division: n(3, 0.9),
        progress_pct: n(-15, 0.92),
        progress_change: n(-29, 0.8),
        change_tags: Some(RankTags {
            value: vec!["Calibration Down".to_string(), "Uphill Battle".to_string()],
            confidence: 0.7,
        }),
        higher_than_pct: n(88, 0.95),
        hero_sr: n(3412, 0.85),
        hero_sr_change: n(-21, 0.75),
        next_mode_hint: t("ROLE QUEUE", 0.66),
    }
}

fn queue(value: &str) -> QueueTitle {
    QueueTitle {
        value: Some(value.to_string()),
        confidence: 0.93,
        ..QueueTitle::default()
    }
}

fn game(rank: Option<RankUpdate>, title: Option<QueueTitle>) -> crate::storage::PersonalMatch {
    let mut game = row("Ana", "Busan", "Control");
    game.rank_update = rank;
    game.queue_title = title;
    game
}

fn body(rank: Option<RankUpdate>, title: Option<QueueTitle>) -> String {
    serde_json::to_string(&upload_request(&[game(rank, title)], &[])).unwrap()
}

fn sent(rank: Option<RankUpdate>, title: Option<QueueTitle>) -> Value {
    let v: Value = serde_json::from_str(&body(rank, title)).unwrap();
    v["matches"][0]["rank"].clone()
}

/// The JSON of one field as the upload sends it (the f32 confidence as its
/// shortest text, as the body has it).
fn f(value: Value, confidence: f32) -> Value {
    let confidence: Value =
        serde_json::from_str(&serde_json::to_string(&confidence).unwrap()).unwrap();
    json!({ "value": value, "confidence": confidence })
}

#[test]
fn a_full_read_sends_every_field_next_to_the_queue_title() {
    let rank = sent(Some(full()), Some(queue("ROLE QUEUE")));
    let want = json!({
        "role": f(json!("Damage"), 0.874),
        "role_rank": f(json!("Diamond"), 0.81),
        "division": f(json!(3), 0.9),
        "progress_pct": f(json!(-15), 0.92),
        "progress_change": f(json!(-29), 0.8),
        "change_tags": f(json!(["Calibration Down", "Uphill Battle"]), 0.7),
        "higher_than_pct": f(json!(88), 0.95),
        "hero_sr": f(json!(3412), 0.85),
        "hero_sr_change": f(json!(-21), 0.75),
        "next_mode_hint": f(json!("ROLE QUEUE"), 0.66),
        "queue_title": f(json!("ROLE QUEUE"), 0.93),
        "rank_recognizer": "rank-v1",
    });
    assert_eq!(rank, want);
    // and the site reads every field back as sent
    let site = resolve_rank(Some(&rank)).expect("a rank object");
    assert_eq!(site.role.value, Some(RankRole::Damage));
    assert_eq!(site.role_rank.value.as_deref(), Some("Diamond"));
    assert_eq!(site.division.value, Some(3));
    assert_eq!(site.progress_pct.value, Some(-15));
    assert_eq!(site.progress_change.value, Some(-29));
    assert_eq!(site.change_tags.value.map(|t| t.len()), Some(2));
    assert_eq!(site.higher_than_pct.value, Some(88));
    assert_eq!(site.hero_sr.value, Some(3412));
    assert_eq!(site.hero_sr_change.value, Some(-21));
    assert_eq!(site.next_mode_hint.value.as_deref(), Some("ROLE QUEUE"));
    assert_eq!(site.queue_title.value.as_deref(), Some("ROLE QUEUE"));
    assert_eq!(site.rank_recognizer.as_deref(), Some("rank-v1"));
    assert_eq!(site.hero_sr.confidence, Some(0.85));
    // the whole body still parses as the site's upload
    let parsed: StatsUploadBody = serde_json::from_str(&body(Some(full()), None)).unwrap();
    assert_eq!(parsed.matches.len(), 1);
}

#[test]
fn a_partial_read_sends_only_the_fields_read() {
    let partial = RankUpdate {
        progress_pct: n(31, 0.9),
        hero_sr: n(2481, 0.8),
        ..RankUpdate::default()
    };
    let rank = sent(Some(partial), None);
    assert_eq!(
        rank,
        json!({
            "progress_pct": f(json!(31), 0.9),
            "hero_sr": f(json!(2481), 0.8),
            "rank_recognizer": "rank-v1",
        })
    );
    // no null keys
    assert!(!body(Some(full()), None).contains("null"));
}

#[test]
fn no_rank_update_sends_exactly_the_old_body() {
    // With the reader off the game has no rank update: the body is the one
    // the tracker sent before this field existed, with or without a queue
    // title.
    let old = r#"{"matches":[{"session_id":"sess-hold","hero":"Ana","map_name":"Busan","game_mode":"Control","role":"Support","outcome":"victory","elims":4,"deaths":1,"assists":2,"damage":1000,"healing":4000,"mitigation":0,"played_at":"2026-07-01T20:00:00Z","edited":false,"recognizer":"ocr-v1"}],"deleted_sessions":[]}"#;
    assert_eq!(body(None, None), old);
    assert_eq!(body(Some(RankUpdate::default()), None), old);
    let with_title = body(None, Some(queue("OPEN QUEUE")));
    assert_eq!(
        sent(None, Some(queue("OPEN QUEUE"))),
        json!({ "queue_title": f(json!("OPEN QUEUE"), 0.93), "rank_recognizer": "rank-v1" })
    );
    assert_eq!(
        body(Some(RankUpdate::default()), Some(queue("OPEN QUEUE"))),
        with_title
    );
    assert!(upload_rank(&game(None, None)).is_none());
}

#[test]
fn open_is_sent_only_when_the_queue_title_says_open_queue() {
    let open = RankUpdate {
        role: t("Open", 0.5),
        hero_sr: n(3412, 0.85),
        ..RankUpdate::default()
    };
    for title in [
        None,
        Some(queue("ROLE QUEUE")),
        // a weak title is no title
        Some(QueueTitle {
            confidence: 0.5,
            ..queue("OPEN QUEUE")
        }),
    ] {
        let rank = sent(Some(open.clone()), title.clone());
        assert!(rank.get("role").is_none(), "{title:?}: {rank}");
        assert_eq!(
            rank["hero_sr"]["value"], 3412,
            "{title:?}: the rest is sent"
        );
    }
    for title in ["OPEN QUEUE", "Open Queue", "COMPETITIVE: OPEN QUEUE"] {
        let rank = sent(Some(open.clone()), Some(queue(title)));
        assert_eq!(rank["role"], f(json!("Open"), 0.5), "{title}");
    }
    // a role from an icon goes with no title or a role queue title
    let tank = RankUpdate {
        role: t("Tank", 0.9),
        hero_sr: n(3412, 0.85),
        ..RankUpdate::default()
    };
    assert_eq!(
        sent(Some(tank.clone()), None)["role"],
        f(json!("Tank"), 0.9)
    );
    assert_eq!(
        sent(Some(tank.clone()), Some(queue("ROLE QUEUE")))["role"],
        f(json!("Tank"), 0.9)
    );
    // never with an open queue title: open queue screens have no role line
    for role in ["Tank", "Damage", "Support"] {
        let icon = RankUpdate {
            role: t(role, 0.9),
            ..tank.clone()
        };
        let rank = sent(Some(icon), Some(queue("OPEN QUEUE")));
        assert!(rank.get("role").is_none(), "{role}: {rank}");
        assert_eq!(rank["hero_sr"]["value"], 3412, "{role}: the rest is sent");
    }
    // a spelling the site does not know is not sent
    let odd = RankUpdate {
        role: t("Healer", 0.9),
        ..RankUpdate::default()
    };
    assert_eq!(sent(Some(odd), None), Value::Null);
}

#[test]
fn more_than_eight_change_tags_are_not_sent() {
    for (count, kept) in [(1, true), (8, true), (9, false), (20, false)] {
        let rank = RankUpdate {
            change_tags: tags(count),
            hero_sr: n(3412, 0.85),
            ..RankUpdate::default()
        };
        let sent = sent(Some(rank), None);
        assert_eq!(sent.get("change_tags").is_some(), kept, "{count}: {sent}");
        assert_eq!(sent["hero_sr"]["value"], 3412, "{count}");
    }
}

#[test]
fn values_the_site_would_null_are_left_out() {
    let bad = RankUpdate {
        division: n(6, 0.9),
        higher_than_pct: n(101, 0.9),
        hero_sr: n(-1, 0.9),
        progress_pct: n(31, f32::NAN),
        progress_change: n(5, 0.0),
        role_rank: t(&"x".repeat(65), 0.9),
        next_mode_hint: t("  ", 0.9),
        hero_sr_change: n(12, 1.5),
        ..RankUpdate::default()
    };
    assert_eq!(sent(Some(bad), None), Value::Null);
}

#[test]
fn the_upload_names_the_one_rank_recognizer() {
    assert_eq!(RANK_RECOGNIZER_ID, "rank-v1");
    assert_eq!(RANK_RECOGNIZER_ID, crate::model::RANK_RECOGNIZER_ID);
    assert_eq!(RANK_RECOGNIZER_ID, crate::model::QUEUE_TITLE_RECOGNIZER_ID);
    assert!(scuffed_types::api::is_recognizer_id(RANK_RECOGNIZER_ID));
    assert_eq!(
        crate::model::RANK_MAX_CHANGE_TAGS,
        scuffed_types::api::RANK_MAX_CHANGE_TAGS
    );
}

#[test]
fn change_tags_are_sent_trimmed_and_checked_as_sent() {
    let long = "y".repeat(scuffed_types::api::RANK_STRING_MAX_CHARS);
    let rank = RankUpdate {
        change_tags: Some(RankTags {
            value: vec!["  Uphill Battle ".to_string(), format!(" {long} ")],
            confidence: 0.7,
        }),
        ..RankUpdate::default()
    };
    let sent = sent(Some(rank), None);
    assert_eq!(
        sent["change_tags"],
        f(json!(["Uphill Battle", long]), 0.7),
        "{sent}"
    );
}

#[test]
fn a_games_own_banner_word_is_never_its_queue() {
    // the game's own rank screen banner says OPEN QUEUE: that names the
    // next game's queue, so it neither fills this game's queue title nor
    // lets an Open role through
    let rank = RankUpdate {
        role: t("Open", 0.5),
        next_mode_hint: t("OPEN QUEUE", 0.75),
        ..RankUpdate::default()
    };
    let none = sent(Some(rank.clone()), None);
    assert!(none.get("queue_title").is_none(), "{none}");
    assert!(none.get("role").is_none(), "{none}");
    assert_eq!(none["next_mode_hint"], f(json!("OPEN QUEUE"), 0.75));
    let role = sent(Some(rank), Some(queue("ROLE QUEUE")));
    assert_eq!(role["queue_title"], f(json!("ROLE QUEUE"), 0.93), "{role}");
    assert!(role.get("role").is_none(), "{role}");
}

#[test]
fn a_rank_changes_the_upload_only_when_a_field_is_sent() {
    let plain = game(None, None);
    assert!(super::rank_update_changes_upload(&plain, &full()));
    // an Open role with no open queue title, and a value the site would
    // null: nothing the upload sends changes
    let dropped = RankUpdate {
        role: t("Open", 0.5),
        division: n(6, 0.9),
        ..RankUpdate::default()
    };
    assert!(!super::rank_update_changes_upload(&plain, &dropped));
    let open = game(None, Some(queue("OPEN QUEUE")));
    assert!(super::rank_update_changes_upload(&open, &dropped));
}
