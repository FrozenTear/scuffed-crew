//! Tests of the upload wire mapping: `upload_request` and `row_needs_review`.

use super::*;

fn played() -> surrealdb_types::Datetime {
    surrealdb_types::Datetime::from(
        chrono::DateTime::parse_from_rfc3339("2026-07-01T20:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
    )
}

fn row(hero: &str, map: &str, mode: &str) -> PersonalMatch {
    PersonalMatch {
        id: None,
        hero: hero.into(),
        map_name: map.into(),
        game_mode: mode.into(),
        role: "Support".into(),
        outcome: "victory".into(),
        elims: 4,
        deaths: 1,
        assists: 2,
        damage: 1000,
        healing: 4000,
        mitigation: 0,
        played_at: played(),
        synced: false,
        sync_rev: 0,
        upload_reject: None,
        session_id: "sess-hold".into(),
        corrected_hero: None,
        corrected_role: None,
        corrected_map_name: None,
        corrected_outcome: None,
        corrected_elims: None,
        corrected_deaths: None,
        corrected_assists: None,
        corrected_damage: None,
        corrected_healing: None,
        corrected_mitigation: None,
        edited_fields: Vec::new(),
        edited_at: None,
        heroes_played: Vec::new(),
        segment_resolutions: Vec::new(),
        recognizer: String::new(),
        suspect_fields: Vec::new(),
        name_recognizer: None,
    }
}

fn body_of(rows: &[PersonalMatch]) -> String {
    serde_json::to_string(&upload_request(rows, &[])).unwrap()
}

#[test]
fn no_map_read_never_sends_an_empty_map_name() {
    let missed = row("Ana", "", "");
    assert_eq!(
        crate::parse::review_suspect_fields(
            missed.display_map_name(),
            &missed.game_mode,
            missed.display_hero()
        ),
        vec!["map", "mode"]
    );
    assert!(row_needs_review(&missed));
    let unrecognised = row("Ana", "Not a map", "");
    assert!(row_needs_review(&unrecognised));
    let ready = row("Ana", "Busan", "Control");
    let body = body_of(&[missed, unrecognised, ready]);
    assert!(!body.contains("\"map_name\":\"\""));
    assert!(!body.contains("Not a map"));
    let parsed: scuffed_types::api::StatsUploadRequest = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed.matches.len(), 1);
    assert_eq!(parsed.matches[0].map_name, "Busan");
    assert_eq!(parsed.matches[0].game_mode, "Control");
    assert_eq!(parsed.matches[0].hero, "Ana");
}

#[test]
fn empty_game_mode_and_unknown_hero_are_not_sent() {
    let mode_only = row("Ana", "Busan", "");
    let mode_body = body_of(&[mode_only]);
    assert!(!mode_body.contains("\"game_mode\":\"\""));
    let mode_parsed: scuffed_types::api::StatsUploadRequest =
        serde_json::from_str(&mode_body).unwrap();
    assert_eq!(mode_parsed.matches.len(), 1);
    assert_eq!(mode_parsed.matches[0].game_mode, "Control");
    assert_eq!(mode_parsed.matches[0].map_name, "Busan");

    let blank_mode = row("Ana", "", "");
    let unknown_hero = row("Unknown", "Busan", "");
    let lower = row("unknown", "Ilios", "Control");
    let body = body_of(&[blank_mode, unknown_hero, lower]);
    assert!(
        upload_request(&[row("Unknown", "Busan", "")], &[])
            .matches
            .is_empty()
    );
    assert!(!body.contains("\"game_mode\":\"\""));
    assert!(!body.contains("\"map_name\":\"\""));
    assert!(!body.contains("Unknown"));
    assert!(!body.contains("unknown"));
    assert!(!body.contains("Ilios"));
    let parsed: scuffed_types::api::StatsUploadRequest = serde_json::from_str(&body).unwrap();
    assert!(parsed.matches.is_empty());
}

#[test]
fn picked_map_and_hero_clear_the_hold() {
    let mut held = row("Unknown", "", "");
    assert_eq!(
        crate::parse::review_suspect_fields(
            held.display_map_name(),
            &held.game_mode,
            held.display_hero()
        ),
        vec!["map", "mode", "hero"]
    );
    assert!(upload_request(&[held.clone()], &[]).matches.is_empty());

    held.corrected_map_name = Some("King's Row".into());
    held.game_mode = crate::parse::stored_game_mode("King's Row");
    held.corrected_hero = Some("Ana".into());
    held.corrected_role = Some(crate::parse::guess_role_public("Ana"));
    assert!(!row_needs_review(&held));
    let parsed = upload_request(&[held], &[]);
    assert_eq!(parsed.matches.len(), 1);
    assert_eq!(parsed.matches[0].map_name, "King's Row");
    assert_eq!(parsed.matches[0].game_mode, "Hybrid");
    assert_eq!(parsed.matches[0].hero, "Ana");
    assert_eq!(parsed.matches[0].role, "Support");
    let body = serde_json::to_string(&parsed).unwrap();
    assert!(!body.contains("\"map_name\":\"\""));
    assert!(!body.contains("\"game_mode\":\"\""));
    assert!(!body.contains("Unknown"));
}

fn upload_name_recognizer(id: Option<&str>) -> serde_json::Value {
    let mut match_row = row("Ana", "Busan", "Control");
    match_row.name_recognizer = id.map(str::to_string);
    let raw = serde_json::to_string(&upload_request(&[match_row], &[])).unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn upload_serializes_names_v1_when_the_pack_named_the_row() {
    let body = upload_name_recognizer(Some("names-v1"));
    assert_eq!(body["matches"][0]["name_recognizer"], "names-v1");
}

#[test]
fn upload_serializes_ocr_v1_when_the_row_fell_back_to_tesseract() {
    let body = upload_name_recognizer(Some("ocr-v1"));
    assert_eq!(body["matches"][0]["name_recognizer"], "ocr-v1");
}

#[test]
fn upload_omits_name_recognizer_when_no_name_was_read() {
    let body = upload_name_recognizer(None);
    assert!(
        body["matches"][0].get("name_recognizer").is_none(),
        "{body}"
    );
    let raw = serde_json::to_string(&body).unwrap();
    assert!(!raw.contains("name_recognizer"), "{raw}");
    assert!(!raw.contains("\"name_recognizer\":null"), "{raw}");
}
