//! Tests of the `bans_picks` object in the upload body: the stored cv-v1
//! JSON as plain values, and the key left out when nothing was read.

use super::bans_picks::{MAX_BAN_VOTE, MAX_BANS, MAX_PICKS, upload_bans_picks};
use super::tests_wire::row;
use super::*;
use serde_json::{Value, json};

/// One stored read, as the daemon writes it.
fn read(value: Option<&str>) -> Value {
    json!({
        "value": value,
        "confidence": if value.is_some() { 0.93 } else { 0.0 },
        "score": 0.81,
        "margin": 0.3,
    })
}

fn stored_ban(value: Option<&str>, team: Option<&str>) -> Value {
    let mut ban = read(value);
    ban["team"] = json!(team);
    ban
}

fn reads(values: &[Option<&str>]) -> Vec<Value> {
    values.iter().map(|v| read(*v)).collect()
}

/// The upload body's first match, for a row that carries `stored`.
fn first_match(stored: Option<&Value>) -> Value {
    let mut game = row("Ana", "Busan", "Control");
    game.bans_picks = stored.map(Value::to_string);
    let body = serde_json::to_string(&upload_request(&[game], &[])).unwrap();
    let body: Value = serde_json::from_str(&body).unwrap();
    body["matches"][0].clone()
}

fn full_read() -> Value {
    json!({
        "layout": "lobby",
        "bans": [
            stored_ban(Some("mauga"), Some("own")),
            stored_ban(Some("wrecking-ball"), Some("enemy")),
            stored_ban(Some("ana"), Some("own")),
            stored_ban(Some("juno"), Some("enemy")),
        ],
        "lobby_ban": read(Some("mei")),
        "picks": reads(&[
            Some("kiriko"), Some("zarya"), Some("ana"), Some("juno"), Some("mercy"), Some("dva"),
        ]),
        "ban_vote": reads(&[
            Some("ana"), Some("zarya"), Some("mercy"), Some("juno"), Some("dva"), Some("mei"),
        ]),
        "bans_picks_recognizer": "cv-v1",
    })
}

#[test]
fn a_full_read_is_sent_as_plain_values() {
    let sent = first_match(Some(&full_read()));
    assert_eq!(
        sent["bans_picks"],
        json!({
            "bans": [
                { "hero": "mauga", "team": "own" },
                { "hero": "wrecking-ball", "team": "enemy" },
                { "hero": "ana", "team": "own" },
                { "hero": "juno", "team": "enemy" },
            ],
            "lobby_ban": "mei",
            "picks": ["kiriko", "zarya", "ana", "juno", "mercy", "dva"],
            "ban_vote": ["ana", "zarya", "mercy", "juno", "dva", "mei"],
            "recognizer": "cv-v1",
        })
    );
    let wire = sent["bans_picks"].to_string();
    for private in [
        "confidence",
        "score",
        "margin",
        "layout",
        "bans_picks_recognizer",
    ] {
        assert!(
            !wire.contains(private),
            "{private} leaves the machine: {wire}"
        );
    }
    assert_eq!(sent["hero"], "Ana", "the rest of the row is unchanged");
}

#[test]
fn a_partial_read_keeps_its_nulls_in_place() {
    let stored = json!({
        "layout": "plain",
        "bans": [
            stored_ban(Some("ana"), None),
            stored_ban(None, Some("enemy")),
            stored_ban(None, None),
            stored_ban(Some("juno"), Some("own")),
        ],
        "lobby_ban": null,
        "picks": reads(&[Some("kiriko"), None, None, Some("juno")]),
        "ban_vote": [],
        "bans_picks_recognizer": "cv-v1",
    });
    let sent = first_match(Some(&stored));
    assert_eq!(
        sent["bans_picks"],
        json!({
            "bans": [
                { "hero": "ana", "team": null },
                { "hero": null, "team": "enemy" },
                { "hero": null, "team": null },
                { "hero": "juno", "team": "own" },
            ],
            "lobby_ban": null,
            "picks": ["kiriko", null, null, "juno"],
            "ban_vote": [],
            "recognizer": "cv-v1",
        })
    );
}

#[test]
fn no_preference_is_sent_with_an_underscore_in_picks_and_the_vote() {
    let stored = json!({
        "bans": [],
        "picks": reads(&[
            Some("kiriko"), Some("no_preference"), Some("no-preference"), None,
        ]),
        "ban_vote": reads(&[
            Some("no-preference"), Some("ana"), Some("no_preference"),
        ]),
        "bans_picks_recognizer": "cv-v1",
    });
    let sent = first_match(Some(&stored));
    assert_eq!(
        sent["bans_picks"]["picks"],
        json!(["kiriko", "no_preference", "no_preference", null])
    );
    assert_eq!(
        sent["bans_picks"]["ban_vote"],
        json!(["no_preference", "ana", "no_preference"])
    );
    let wire = sent["bans_picks"].to_string();
    assert!(!wire.contains("no-preference"), "{wire}");
}

#[test]
fn no_preference_is_never_a_ban() {
    let stored = json!({
        "bans": [
            stored_ban(Some("no_preference"), Some("own")),
            stored_ban(Some("no-preference"), Some("enemy")),
            stored_ban(Some("ana"), Some("enemy")),
        ],
        "lobby_ban": read(Some("no_preference")),
        "picks": [],
        "ban_vote": [],
        "bans_picks_recognizer": "cv-v1",
    });
    let sent = first_match(Some(&stored));
    assert_eq!(
        sent["bans_picks"]["bans"],
        json!([
            { "hero": null, "team": "own" },
            { "hero": null, "team": "enemy" },
            { "hero": "ana", "team": "enemy" },
        ])
    );
    assert_eq!(sent["bans_picks"]["lobby_ban"], Value::Null);
}

#[test]
fn nothing_read_leaves_the_key_out() {
    let unreadable = json!({
        "layout": "plain",
        "bans": [stored_ban(None, None), stored_ban(None, Some("own"))],
        "lobby_ban": read(None),
        "picks": reads(&[None, None, None, None, None, None]),
        "ban_vote": reads(&[None, None]),
        "bans_picks_recognizer": "cv-v1",
    });
    let only_the_id = json!({ "bans_picks_recognizer": "cv-v1" });
    let empty_lists = json!({
        "bans": [], "lobby_ban": null, "picks": [], "ban_vote": [],
        "bans_picks_recognizer": "cv-v1",
    });
    let not_an_object = json!(["ana"]);
    let none_at_all = first_match(None);
    let without_key = none_at_all.as_object().unwrap().clone();
    assert!(!without_key.contains_key("bans_picks"), "{none_at_all}");
    for stored in [
        unreadable,
        only_the_id,
        empty_lists,
        not_an_object,
        json!({}),
    ] {
        let sent = first_match(Some(&stored));
        assert_eq!(
            sent.as_object().unwrap().keys().collect::<Vec<_>>(),
            without_key.keys().collect::<Vec<_>>(),
            "no key for {stored}"
        );
        assert!(!sent.to_string().contains("bans_picks"), "{sent}");
    }
    let mut broken = row("Ana", "Busan", "Control");
    broken.bans_picks = Some("{not json".to_string());
    let body = serde_json::to_string(&upload_request(&[broken], &[])).unwrap();
    assert!(!body.contains("bans_picks"), "{body}");
}

#[test]
fn a_row_with_no_read_uploads_the_same_bytes_as_before() {
    let plain = row("Ana", "Busan", "Control");
    let mut unread = plain.clone();
    unread.bans_picks = Some(json!({ "bans_picks_recognizer": "cv-v1" }).to_string());
    let deleted = vec!["gone-1".to_string()];
    let new_body = upload_request(&[plain.clone()], &deleted);
    assert_eq!(new_body.matches.len(), 1, "the row is uploaded");
    // The body the tracker sent before this change: the site's own
    // `StatsUploadRequest`, built from the same entries. This pins that
    // `#[serde(flatten)]` on `UploadEntry` leaves the bytes unchanged.
    let old_body = scuffed_types::api::StatsUploadRequest {
        matches: new_body.matches.iter().map(|m| (**m).clone()).collect(),
        deleted_sessions: deleted.clone(),
    };
    let a = serde_json::to_string(&new_body).unwrap();
    assert_eq!(a, serde_json::to_string(&old_body).unwrap(), "same bytes as the old request");
    let b = serde_json::to_string(&upload_request(&[unread], &deleted)).unwrap();
    assert_eq!(a, b, "a recognizer id with no read adds nothing");
}

#[test]
fn the_lists_are_capped_at_4_6_and_6() {
    assert_eq!((MAX_BANS, MAX_PICKS, MAX_BAN_VOTE), (4, 6, 6));
    let names: Vec<String> = (0..9).map(|i| format!("hero-{i}")).collect();
    let some: Vec<Option<&str>> = names.iter().map(|n| Some(n.as_str())).collect();
    let stored = json!({
        "bans": (0..9).map(|i| stored_ban(Some(&names[i]), Some("own"))).collect::<Vec<_>>(),
        "lobby_ban": read(Some("mei")),
        "picks": reads(&some),
        "ban_vote": reads(&some),
        "bans_picks_recognizer": "cv-v1",
    });
    let sent = first_match(Some(&stored));
    let bp = &sent["bans_picks"];
    assert_eq!(bp["bans"].as_array().unwrap().len(), 4);
    assert_eq!(bp["picks"].as_array().unwrap().len(), 6);
    assert_eq!(bp["ban_vote"].as_array().unwrap().len(), 6);
    assert_eq!(bp["bans"][3]["hero"], "hero-3", "the first four stay");
    assert_eq!(bp["picks"][5], "hero-5");
    assert_eq!(bp["ban_vote"][5], "hero-5");
}

#[test]
fn the_recognizer_is_cv_v1_unless_the_stored_id_is_another_usable_one() {
    let mut stored = full_read();
    stored
        .as_object_mut()
        .unwrap()
        .remove("bans_picks_recognizer");
    assert_eq!(
        upload_bans_picks(Some(&stored.to_string()))
            .unwrap()
            .recognizer,
        "cv-v1"
    );
    stored["bans_picks_recognizer"] = json!("Not A Valid Id!");
    assert_eq!(
        upload_bans_picks(Some(&stored.to_string()))
            .unwrap()
            .recognizer,
        "cv-v1"
    );
    stored["bans_picks_recognizer"] = json!("cv-v2");
    assert_eq!(
        upload_bans_picks(Some(&stored.to_string()))
            .unwrap()
            .recognizer,
        "cv-v2"
    );
}

#[test]
fn an_older_server_that_ignores_the_key_still_decodes_the_body() {
    let mut game = row("Ana", "Busan", "Control");
    game.bans_picks = Some(full_read().to_string());
    let body = serde_json::to_string(&upload_request(&[game], &[])).unwrap();
    assert!(body.contains("\"bans_picks\""), "{body}");
    let older: scuffed_types::api::StatsUploadRequest = serde_json::from_str(&body).unwrap();
    assert_eq!(older.matches.len(), 1);
    assert_eq!(older.matches[0].hero, "Ana");
    assert_eq!(older.matches[0].map_name, "Busan");
}
