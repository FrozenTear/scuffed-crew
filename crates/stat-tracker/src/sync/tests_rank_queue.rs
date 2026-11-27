//! Tests of `rank.queue` on the upload (T15, #654): the five values, a
//! game with no rank screen read still sends a rank object that holds the
//! queue when the title names role or open queue, a failed queue read is
//! `unknown` and never drops the game, and Quick Play / a game with the
//! readers off still omit `rank`. A title that names a mode that is not
//! Competitive omits `rank` even with rank screen fields, and an Open role
//! line decides no queue (#905 R1, R2).

use super::tests_wire::row;
use super::wire::{UploadQueueField, upload_rank};
use super::*;
use crate::model::{QueueTitle, RankNumber, RankTags, RankText, RankUpdate};
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
}

#[test]
fn an_open_role_line_with_no_title_decides_no_queue() {
    // Open is not sent as role without an open queue title, and it does
    // not decide the queue either: the queue is unknown, the other rank
    // fields stay.
    let open = |hero_sr| {
        game(
            None,
            Some(6),
            Some(RankUpdate {
                role: t("Open", 0.5),
                hero_sr,
                ..RankUpdate::default()
            }),
        )
    };
    let rank = sent(&open(n(3412, 0.85)));
    assert_eq!(rank["queue"]["value"], json!("unknown"), "{rank}");
    assert_eq!(rank["queue"]["confidence"], Value::Null, "{rank}");
    assert!(rank.get("role").is_none(), "{rank}");
    assert_eq!(rank["hero_sr"]["value"], json!(3412));
    // The Open line alone is no field the upload sends, so no rank object.
    assert!(upload_rank(&open(None)).is_none());
}

#[test]
fn a_title_naming_a_mode_that_is_not_competitive_sends_no_rank() {
    // With or without a rank screen field, and whatever the size: the
    // game is uploaded with no `rank` key at all (#889).
    let screen = RankUpdate {
        role: t("Tank", 0.9),
        hero_sr: n(3412, 0.85),
        ..RankUpdate::default()
    };
    for word in ["QUICK", "UNRANKED", "ARCADE", "STADIUM", "CUSTOM"] {
        // The game's own order ("ROLE QUEUE: QUICK PLAY", "OPEN QUEUE:
        // 6V6 QUICK PLAY") and the mode word first.
        for title in [
            format!("ROLE QUEUE: {word} PLAY"),
            format!("OPEN QUEUE: 6V6 {word} PLAY"),
            format!("{word} PLAY: ROLE QUEUE"),
            format!("{word}: OPEN QUEUE"),
        ] {
            for size in [Some(5), Some(6), None] {
                for update in [None, Some(screen.clone())] {
                    let m = game(Some(queue(&title)), size, update);
                    assert!(upload_rank(&m).is_none(), "{title} {size:?}");
                    let body = body_of(&m);
                    assert!(!body.contains("rank"), "{title}: {body}");
                    let parsed: StatsUploadBody = serde_json::from_str(&body).unwrap();
                    assert_eq!(parsed.matches.len(), 1, "{title}");
                }
            }
        }
    }
}

#[test]
fn board_and_title_sizes_that_disagree_send_an_unknown_queue() {
    // Both ways round, and a title that names both sizes (#905 R3, R4).
    for (title, size) in [
        ("5V5 ROLE QUEUE", 6),
        ("6V6 OPEN QUEUE", 5),
        ("5V5 6V6 ROLE QUEUE", 6),
    ] {
        let m = game(Some(queue(title)), Some(size), None);
        let rank = sent(&m);
        assert_eq!(rank["queue"]["value"], json!("unknown"), "{title} {size}");
        assert_eq!(rank["queue"]["confidence"], Value::Null, "{title} {size}");
        assert_eq!(rank["queue_title"]["value"], json!(title), "{title}");
        let parsed: StatsUploadBody = serde_json::from_str(&body_of(&m)).unwrap();
        assert_eq!(parsed.matches.len(), 1, "{title}");
    }
}

/// Pins today's behaviour (#905 R8, not yet decided): a confident title
/// that names no queue (or both) and no rank screen read sends no rank
/// object, so `rank.queue_title` is not sent for it either.
#[test]
fn a_title_naming_no_queue_and_no_rank_screen_read_omits_the_rank_object() {
    for title in ["COMPETITIVE", "COMPETITIVE: 6V6", "ROLE OPEN QUEUE"] {
        for size in [Some(5), Some(6)] {
            let m = game(Some(queue(title)), size, None);
            assert!(upload_rank(&m).is_none(), "{title} {size:?}");
            assert!(!body_of(&m).contains("rank"), "{title} {size:?}");
        }
    }
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
    for title in ["QUICK PLAY", "6V6 QUICK PLAY", "ARCADE"] {
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

/// Each rank-screen field alone is enough Competitive evidence to send
/// a `rank` object (and so `rank.queue`). The pre-game title is not one
/// of those fields.
#[test]
fn each_rank_screen_field_alone_sends_a_rank_object() {
    let cases: [(&str, RankUpdate); 10] = [
        (
            "role",
            RankUpdate {
                role: t("Tank", 0.9),
                ..RankUpdate::default()
            },
        ),
        (
            "role_rank",
            RankUpdate {
                role_rank: t("Diamond", 0.9),
                ..RankUpdate::default()
            },
        ),
        (
            "division",
            RankUpdate {
                division: n(2, 0.9),
                ..RankUpdate::default()
            },
        ),
        (
            "progress_pct",
            RankUpdate {
                progress_pct: n(45, 0.9),
                ..RankUpdate::default()
            },
        ),
        (
            "progress_change",
            RankUpdate {
                progress_change: n(23, 0.9),
                ..RankUpdate::default()
            },
        ),
        (
            "change_tags",
            RankUpdate {
                change_tags: Some(RankTags {
                    value: vec!["Win".into()],
                    confidence: 0.9,
                }),
                ..RankUpdate::default()
            },
        ),
        (
            "higher_than_pct",
            RankUpdate {
                higher_than_pct: n(80, 0.9),
                ..RankUpdate::default()
            },
        ),
        (
            "hero_sr",
            RankUpdate {
                hero_sr: n(3412, 0.9),
                ..RankUpdate::default()
            },
        ),
        (
            "hero_sr_change",
            RankUpdate {
                hero_sr_change: n(10, 0.9),
                ..RankUpdate::default()
            },
        ),
        (
            "next_mode_hint",
            RankUpdate {
                next_mode_hint: t("6V6", 0.9),
                ..RankUpdate::default()
            },
        ),
    ];
    for (name, update) in cases {
        let rank = upload_rank(&game(None, Some(6), Some(update)))
            .unwrap_or_else(|| panic!("{name}: a rank-screen field alone must send a rank object"));
        assert!(rank.queue.is_some(), "{name}");
    }
    assert!(
        upload_rank(&game(None, Some(6), Some(RankUpdate::default()))).is_none(),
        "an empty rank update is not a rank-screen field"
    );
}

#[test]
fn a_role_line_rank_role_does_not_take_sends_no_role_and_an_unknown_queue() {
    // These are not `RankRole` spellings: they are neither sent as the
    // role nor used to decide the queue.
    for role in ["TANK", " Tank", "Tank "] {
        let m = game(
            None,
            Some(6),
            Some(RankUpdate {
                role: t(role, 0.9),
                hero_sr: n(3412, 0.85),
                ..RankUpdate::default()
            }),
        );
        let rank = sent(&m);
        assert!(rank.get("role").is_none(), "{role}: {rank}");
        assert_eq!(rank["queue"]["value"], json!("unknown"), "{role}");
        assert_eq!(rank["queue"]["confidence"], Value::Null, "{role}");
        assert_eq!(rank["hero_sr"]["value"], json!(3412), "{role}");
    }
}

#[test]
fn a_role_spelling_rank_role_does_not_take_keeps_a_confident_titles_queue() {
    // The role is not sent, but the title still decides the queue, and
    // the other rank fields and the queue title stay (#905 R11).
    for (title, role, want) in [
        ("ROLE QUEUE", "TANK", "role_6v6"),
        ("ROLE QUEUE", " Tank", "role_6v6"),
        ("ROLE QUEUE", "Tank ", "role_6v6"),
        ("ROLE QUEUE", "tank", "role_6v6"),
        ("OPEN QUEUE", " Open", "open_6v6"),
        ("OPEN QUEUE", "OPEN", "open_6v6"),
    ] {
        let m = game(
            Some(queue(title)),
            Some(6),
            Some(RankUpdate {
                role: t(role, 0.9),
                role_rank: t("Diamond", 0.81),
                hero_sr: n(3412, 0.85),
                ..RankUpdate::default()
            }),
        );
        let rank = sent(&m);
        assert!(rank.get("role").is_none(), "{title} {role:?}: {rank}");
        assert_eq!(rank["queue"], f(json!(want), 0.93), "{title} {role:?}");
        assert_eq!(
            rank["queue_title"]["value"],
            json!(title),
            "{title} {role:?}"
        );
        assert_eq!(
            rank["role_rank"]["value"],
            json!("Diamond"),
            "{title} {role:?}"
        );
        assert_eq!(rank["hero_sr"]["value"], json!(3412), "{title} {role:?}");
    }
}
