//! Tests of the `rank` object in the upload body (`rank-v1`):
//! `{"queue_title": {value, confidence}, "rank_recognizer": "rank-v1"}`
//! for a confident read, the key left out otherwise, and the body read by
//! the site's own parse (`StatsUploadBody` and `resolve_rank`).

use super::tests_wire::row;
use super::wire::{RANK_RECOGNIZER_ID, UploadRank, UploadRankField, upload_rank};
use super::*;
use crate::model::QueueTitle;
use scuffed_types::api::{StatsUploadBody, resolve_rank};
use serde_json::{Value, json};

fn game(title: Option<QueueTitle>) -> crate::storage::PersonalMatch {
    let mut game = row("Ana", "Busan", "Control");
    game.queue_title = title;
    game
}

fn body(title: Option<QueueTitle>) -> String {
    serde_json::to_string(&upload_request(&[game(title)], &[])).unwrap()
}

fn read(value: Option<&str>, confidence: f32) -> QueueTitle {
    QueueTitle {
        value: value.map(str::to_string),
        confidence,
        unsure: value.is_none(),
        mode_hint: None,
    }
}

#[test]
fn no_confident_value_sends_exactly_the_old_body() {
    let before = body(None);
    assert!(!before.contains("rank"), "{before}");
    assert!(!before.contains("queue_title"), "{before}");
    for weak in [
        read(None, 0.0),
        QueueTitle::default(),
        read(Some("  "), 0.93),
        // Below the reader's bar, out of range or not a number: not sent.
        read(
            Some("OPEN QUEUE"),
            crate::model::QUEUE_TITLE_MIN_SCORE - 0.01,
        ),
        read(Some("OPEN QUEUE"), 0.5),
        read(Some("OPEN QUEUE"), f32::NAN),
        read(Some("OPEN QUEUE"), 1.5),
    ] {
        assert_eq!(body(Some(weak.clone())), before, "{weak:?}");
    }
}

#[test]
fn a_confident_value_is_sent_in_the_rank_object() {
    let sent: Value = serde_json::from_str(&body(Some(read(Some("OPEN QUEUE"), 0.93)))).unwrap();
    let m = &sent["matches"][0];
    assert!(m.get("queue_title").is_none(), "no top-level key: {m}");
    assert_eq!(m["rank"]["queue_title"]["value"], json!("OPEN QUEUE"));
    let confidence = m["rank"]["queue_title"]["confidence"].as_f64().unwrap();
    assert!((confidence - 0.93).abs() < 1e-6, "{confidence}");
    assert_eq!(m["rank"]["rank_recognizer"], json!("rank-v1"));
    assert_eq!(
        m["rank"].as_object().unwrap().len(),
        2,
        "only the fields that were read: {m}"
    );
    // Exactly at the bar is sent; the warning and a hint are local only.
    let at_bar = QueueTitle {
        mode_hint: Some("open".into()),
        ..read(Some("OPEN QUEUE"), crate::model::QUEUE_TITLE_MIN_SCORE)
    };
    assert_eq!(
        upload_rank(&game(Some(at_bar))),
        Some(UploadRank {
            queue_title: Some(UploadRankField {
                value: "OPEN QUEUE".into(),
                confidence: crate::model::QUEUE_TITLE_MIN_SCORE,
            }),
            rank_recognizer: RANK_RECOGNIZER_ID,
        })
    );
    assert_eq!(RANK_RECOGNIZER_ID, crate::model::QUEUE_TITLE_RECOGNIZER_ID);
}

#[test]
fn the_site_parse_reads_the_queue_title_and_the_rank_recognizer() {
    let old = body(None);
    let new = body(Some(read(Some("ROLE QUEUE"), 0.93)));
    let parsed: StatsUploadBody = serde_json::from_str(&new).unwrap();
    assert_eq!(parsed.matches.len(), 1);
    assert_eq!(parsed.matches[0].entry.hero, "Ana");
    let rank = resolve_rank(parsed.matches[0].rank.as_ref()).expect("a rank");
    assert_eq!(rank.queue_title.value.as_deref(), Some("ROLE QUEUE"));
    let confidence = rank.queue_title.confidence.expect("a confidence");
    assert!((confidence - 0.93).abs() < 1e-6, "{confidence}");
    assert_eq!(rank.rank_recognizer.as_deref(), Some("rank-v1"));
    // Nothing else is claimed.
    assert_eq!(rank.next_mode_hint.value, None);
    assert_eq!(rank.role.value, None);
    // The old body has no rank for the site.
    let parsed: StatsUploadBody = serde_json::from_str(&old).unwrap();
    assert_eq!(parsed.matches[0].rank, None);
    assert_eq!(resolve_rank(parsed.matches[0].rank.as_ref()), None);
    // Apart from the rank object, the new body is the old one.
    let mut new: Value = serde_json::from_str(&new).unwrap();
    new["matches"][0].as_object_mut().unwrap().remove("rank");
    assert_eq!(new, serde_json::from_str::<Value>(&old).unwrap());
}

#[test]
fn a_hint_alone_is_not_sent_as_the_next_mode_hint() {
    let hint_only = QueueTitle {
        mode_hint: Some("OPEN QUEUE".into()),
        ..read(None, 0.0)
    };
    assert_eq!(upload_rank(&game(Some(hint_only))), None);
    let sent = body(Some(QueueTitle {
        mode_hint: Some("open".into()),
        ..read(Some("OPEN QUEUE"), 0.93)
    }));
    assert!(!sent.contains("next_mode_hint"), "{sent}");
    assert!(!sent.contains("\"open\""), "{sent}");
}
