//! Tests of the `queue_title` object in the upload body (`rank-v1`):
//! `{value, confidence}` for a confident read, the key left out otherwise,
//! and a body the site's request type still parses either way.

use super::tests_wire::row;
use super::wire::{UploadQueueTitle, upload_queue_title};
use super::*;
use crate::model::QueueTitle;
use serde_json::{Value, json};

fn body(title: Option<QueueTitle>) -> String {
    let mut game = row("Ana", "Busan", "Control");
    game.queue_title = title;
    serde_json::to_string(&upload_request(&[game], &[])).unwrap()
}

fn read(value: Option<&str>, unsure: bool) -> QueueTitle {
    QueueTitle {
        value: value.map(str::to_string),
        confidence: if value.is_some() { 0.5 } else { 0.0 },
        unsure,
        mode_hint: None,
    }
}

#[test]
fn no_confident_value_sends_exactly_the_old_body() {
    let before = body(None);
    assert!(!before.contains("queue_title"), "{before}");
    // A weak read with nothing confident behind it adds nothing either.
    assert_eq!(body(Some(read(None, true))), before);
    assert_eq!(body(Some(QueueTitle::default())), before);
    assert_eq!(body(Some(read(Some("  "), false))), before);
}

#[test]
fn a_confident_value_is_sent_as_value_and_confidence() {
    let sent: Value = serde_json::from_str(&body(Some(read(Some("OPEN QUEUE"), false)))).unwrap();
    assert_eq!(
        sent["matches"][0]["queue_title"],
        json!({ "value": "OPEN QUEUE", "confidence": 0.5 })
    );
    // A later weak read kept the value: it is still sent, without the
    // warning or a hint (local only).
    let kept = QueueTitle {
        mode_hint: Some("open".into()),
        ..read(Some("OPEN QUEUE"), true)
    };
    assert_eq!(
        upload_queue_title(Some(&kept)),
        Some(UploadQueueTitle {
            value: "OPEN QUEUE".into(),
            confidence: 0.5
        })
    );
}

#[test]
fn the_site_request_type_parses_old_and_new_bodies() {
    let old = body(None);
    let new = body(Some(read(Some("ROLE QUEUE"), false)));
    for text in [&old, &new] {
        let parsed: scuffed_types::api::StatsUploadRequest = serde_json::from_str(text).unwrap();
        assert_eq!(parsed.matches.len(), 1);
        assert_eq!(parsed.matches[0].hero, "Ana");
    }
    // Apart from the new key, the new body is the old one.
    let mut new: Value = serde_json::from_str(&new).unwrap();
    new["matches"][0]
        .as_object_mut()
        .unwrap()
        .remove("queue_title");
    assert_eq!(new, serde_json::from_str::<Value>(&old).unwrap());
}
