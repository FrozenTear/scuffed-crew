//! Tests of the read, merge and upload path: they start from a reader's
//! board (`shadow`) and end in the save merge ([`super::merge_saved`]) or the
//! upload body (`sync::upload_request`).
//!
//! They used to sit with the reader (`shadow::digits`, `shadow::reader`) and
//! with the uploader (`sync`), so those modules' tests depended on this one,
//! which sits above both. They live here now, next to the merge.

use image::{DynamicImage, Rgb, RgbImage};

use crate::shadow::digits::tests::{sample_values, synth_board};
use crate::shadow::digits::{self, CellRead, RowRead};
use crate::shadow::reader::assemble;
use crate::shadow::result::ResultEvidence;
use crate::shadow::{Reader, ReaderConfig, result_field};
use crate::storage::PersonalMatch;
use crate::sync::upload_request;

// ------------------------------------------------- from shadow::digits

/// Grey row bands in the name strip so the saturation scan sees a pitch.
/// Stat digits stay where `synth_board` drew them.
fn with_row_pitch(board: &DynamicImage, pitch: f64) -> DynamicImage {
    let mut img = board.to_rgb8();
    let (w, h) = img.dimensions();
    let x0 = (w as f64 * 0.06) as u32;
    let x1 = (w as f64 * 0.40) as u32;
    for y in 0..h {
        for x in x0..x1 {
            img.put_pixel(x, y, Rgb([40, 90, 160]));
        }
    }
    let step = pitch * h as f64;
    let mut y = step / 2.0;
    while y + 12.0 < h as f64 * 0.45 {
        for dy in 0..12 {
            for x in x0..x1 {
                img.put_pixel(x, y as u32 + dy, Rgb([150, 150, 150]));
            }
        }
        y += step;
    }
    DynamicImage::ImageRgb8(img)
}

fn frame_with_board(board: &DynamicImage) -> DynamicImage {
    let mut img = RgbImage::from_pixel(2560, 1440, Rgb([0, 0, 0]));
    let src = board.to_rgb8();
    for y in 0..src.height() {
        for x in 0..src.width() {
            img.put_pixel(448 + x, 216 + y, *src.get_pixel(x, y));
        }
    }
    DynamicImage::ImageRgb8(img)
}

#[test]
fn blank_rows_and_an_empty_frame_do_not_invent_a_team_size() {
    let reader = crate::shadow::Reader::load(&crate::shadow::ReaderConfig::default());
    let ocr = crate::reader_apply::PolicyRow {
        map: "Busan".into(),
        mode: "Control".into(),
        result: "victory".into(),
        hero: "Ana".into(),
        elims: 10,
        assists: 4,
        deaths: 2,
        damage: 4000,
        healing: 8000,
        mitigation: 100,
    };
    let own = |team| crate::reader_apply::OwnRow::Identified {
        index: 0,
        team_size: team,
    };
    for (team, pitch) in [(5usize, 0.086), (6usize, 0.074)] {
        let zeros = vec![[0u32; 6]; team * 2];
        let drawn = with_row_pitch(&synth_board(&zeros, team, 3), pitch);
        let board = reader.read_board(&frame_with_board(&drawn));
        if board.status == crate::shadow::BoardStatus::Read {
            assert_eq!(board.team_size, Some(team));
        }
        let other = if team == 5 { 6 } else { 5 };
        assert_ne!(
            board.team_size,
            Some(other),
            "a board of zeros must not be read as {other}v{other}: {:?}",
            (board.status, board.team_size)
        );
        let saved = crate::reader_apply::merge_saved(&ocr, own(team), Some(&board));
        for stat in [
            saved.elims,
            saved.assists,
            saved.deaths,
            saved.damage,
            saved.healing,
            saved.mitigation,
        ] {
            let ocr_stat = [
                ocr.elims,
                ocr.assists,
                ocr.deaths,
                ocr.damage,
                ocr.healing,
                ocr.mitigation,
            ];
            assert!(
                stat == 0 || ocr_stat.contains(&stat),
                "blank rows must not store a made-up stat {stat}"
            );
        }
        for name in ["map", "mode", "result", "hero"] {
            assert!(
                !saved.suspect_fields.iter().any(|field| field == name),
                "{name} was not attempted: {:?}",
                saved.suspect_fields
            );
        }
    }

    let shell = {
        let mut img = synth_board(&vec![[0u32; 6]; 10], 5, 3).to_rgb8();
        let (w, h) = img.dimensions();
        for y in 27..h {
            for x in 900..w {
                img.put_pixel(x, y, Rgb([18, 20, 32]));
            }
        }
        with_row_pitch(&DynamicImage::ImageRgb8(img), 0.086)
    };
    let empty_rows = reader.read_board(&frame_with_board(&shell));
    assert_eq!(empty_rows.status, crate::shadow::BoardStatus::NotFound);
    assert_eq!(
        empty_rows.team_size, None,
        "empty stat rows must not guess a team size"
    );
    let saved = crate::reader_apply::merge_saved(&ocr, own(5), Some(&empty_rows));
    assert_eq!(saved.map, ocr.map);
    assert_eq!(saved.mode, ocr.mode);
    assert_eq!(saved.result, ocr.result);
    assert_eq!(saved.hero, ocr.hero);
    assert_eq!(saved.elims, ocr.elims);
    assert_eq!(saved.assists, ocr.assists);
    assert_eq!(saved.deaths, ocr.deaths);
    assert_eq!(saved.damage, ocr.damage);
    assert_eq!(saved.healing, ocr.healing);
    assert_eq!(saved.mitigation, ocr.mitigation);
    assert_eq!(saved.recognizer, scuffed_types::RECOGNIZER_OCR_V1);
    assert!(saved.suspect_fields.is_empty());

    let empty = reader.read_board(&DynamicImage::new_rgb8(2560, 1440));
    assert_eq!(empty.status, crate::shadow::BoardStatus::NotFound);
    assert_eq!(empty.team_size, None, "an empty frame must not guess 5v5");
    let saved = crate::reader_apply::merge_saved(&ocr, own(5), Some(&empty));
    assert_eq!(saved.map, ocr.map);
    assert_eq!(saved.mode, ocr.mode);
    assert_eq!(saved.result, ocr.result);
    assert_eq!(saved.hero, ocr.hero);
    assert_eq!(saved.elims, ocr.elims);
    assert_eq!(saved.damage, ocr.damage);
    assert_eq!(saved.recognizer, scuffed_types::RECOGNIZER_OCR_V1);
    assert!(saved.suspect_fields.is_empty());
}

#[test]
fn empty_reader_config_does_not_flag_unattempted_fields() {
    let reader = crate::shadow::Reader::load(&crate::shadow::ReaderConfig::default());
    assert!(!reader.has_heroes());
    let drawn = with_row_pitch(&synth_board(&sample_values(5), 5, 3), 0.086);
    let board = reader.read_board(&frame_with_board(&drawn));
    assert_eq!(
        (board.status, board.team_size),
        (crate::shadow::BoardStatus::Read, Some(5)),
        "a drawn 5v5 board with no packs"
    );
    let saved = crate::reader_apply::merge_saved(
        &crate::reader_apply::PolicyRow {
            map: "Busan".into(),
            mode: "Control".into(),
            result: "victory".into(),
            hero: "Ana".into(),
            elims: 10,
            assists: 4,
            deaths: 2,
            damage: 4000,
            healing: 8000,
            mitigation: 100,
        },
        crate::reader_apply::OwnRow::Identified {
            index: 0,
            team_size: 5,
        },
        Some(&board),
    );
    assert!(
        saved.suspect_fields.is_empty(),
        "empty ReaderConfig flagged {:?}",
        saved.suspect_fields
    );
}

// ------------------------------------------------- from shadow::reader

/// Copied from `shadow::reader`'s tests: a digit board with `rows` rows.
fn digits_board(rows: usize) -> digits::BoardRead {
    digits::BoardRead {
        rows: (0..rows)
            .map(|r| RowRead {
                cells: std::array::from_fn(|k| CellRead {
                    value: Some((r * 10 + k) as u32),
                    confidence: 0.9,
                    suspect: r == 3 && k == 2,
                }),
            })
            .collect(),
        elapsed_ms: 1,
        header_dimmed: false,
    }
}

/// Shadow-first, through the save merge `reader = "new"` uses: a sure
/// result (two agreeing sure reads from the words pack) is stored, a
/// suspect one never is (ocr-v1's result stays and `result` is marked),
/// and frames that read no word leave ocr-v1's result as it was, with no
/// mark.
#[test]
fn only_a_sure_result_from_the_words_pack_is_saved() {
    use crate::reader_apply::{OwnRow, PolicyRow, merge_saved};
    use crate::shadow::result::test_fixtures::{lay, paint, word, write_pack};
    use crate::shadow::result::{ACCOLADE, Outcome, RANK_SCREEN, WORDS_SUBDIR};
    let data = tempfile::tempdir().unwrap();
    let words = data.path().join(WORDS_SUBDIR);
    write_pack(
        &words,
        &[
            ("accolade_victory.png", &word(1, 70, 30)),
            ("accolade_defeat.png", &word(3, 60, 30)),
            ("rank_screen_victory.png", &word(5, 66, 20)),
            ("rank_screen_defeat.png", &word(7, 50, 20)),
        ],
    );
    let reader = Reader::load(&ReaderConfig {
        words_dir: Some(words),
        ..Default::default()
    });
    let blank = || image::RgbImage::from_pixel(2560, 1440, image::Rgb([20, 26, 50]));
    let accolade = |seed: u32, w: usize| {
        let mut img = blank();
        paint(&mut img, &lay(ACCOLADE), &word(seed, w, 30), [255, 220, 0]);
        DynamicImage::ImageRgb8(img)
    };
    let victory = accolade(1, 70);
    let defeat = accolade(3, 60);
    let mut rank_victory = image::RgbImage::from_pixel(2560, 1440, image::Rgb([150, 100, 30]));
    paint(
        &mut rank_victory,
        &lay(RANK_SCREEN),
        &word(5, 66, 20),
        [245, 245, 245],
    );
    let rank_victory = DynamicImage::ImageRgb8(rank_victory);
    let nothing = DynamicImage::ImageRgb8(blank());
    let ocr = PolicyRow {
        map: "Ilios".into(),
        mode: "Control".into(),
        result: "defeat".into(),
        hero: "Ana".into(),
        elims: 1,
        assists: 2,
        deaths: 3,
        damage: 4,
        healing: 5,
        mitigation: 6,
    };
    let own = OwnRow::Identified {
        index: 0,
        team_size: 5,
    };
    let save = |frames: &[&DynamicImage]| {
        let mut e = ResultEvidence::default();
        for f in frames {
            e.add(reader.read_result(f));
        }
        let mut board = assemble(5, Some(&digits_board(10)), None);
        board.set_result(result_field(&e));
        let saved = merge_saved(&ocr, own, Some(&board));
        let marked = saved.suspect_fields.iter().any(|f| f == "result");
        (saved.result, marked)
    };
    assert_eq!(reader.read_result(&victory).outcome, Some(Outcome::Victory));
    assert!(!reader.read_result(&victory).suspect);
    // sure: the accolade and the rank screen agree
    assert_eq!(save(&[&victory, &rank_victory]), ("victory".into(), false));
    // one sure read is suspect: not saved
    assert_eq!(save(&[&victory]), ("defeat".into(), true));
    // a conflicting read is suspect: not saved
    assert_eq!(
        save(&[&victory, &rank_victory, &defeat]),
        ("defeat".into(), true)
    );
    // no word on any frame: ocr-v1's result, unmarked, as before O1
    assert_eq!(save(&[&nothing, &nothing]), ("defeat".into(), false));
    assert_eq!(save(&[]), ("defeat".into(), false));
}

// ---------------------------------------------------------- from sync

/// Copied from `sync`'s tests (the upload row they build on).
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

#[test]
fn upload_json_names_recognizer_on_every_row() {
    use crate::reader_apply::{OwnRow, PolicyRow, merge_saved, own_row};
    use crate::shadow::digits::RECOGNIZER_ID;
    use crate::shadow::{BoardRead, BoardStatus, FieldRead, Value};

    fn snap() -> PolicyRow {
        PolicyRow {
            map: "Busan".into(),
            mode: "Control".into(),
            result: "victory".into(),
            hero: "Ana".into(),
            elims: 10,
            assists: 4,
            deaths: 2,
            damage: 4000,
            healing: 8000,
            mitigation: 100,
        }
    }

    fn text(name: &str, value: &str) -> FieldRead {
        FieldRead {
            name: name.into(),
            value: Some(Value::Text(value.into())),
            confidence: 0.95,
            suspect: false,
        }
    }

    fn number(name: &str, value: u32) -> FieldRead {
        FieldRead {
            name: name.into(),
            value: Some(Value::Int(value)),
            confidence: 0.95,
            suspect: false,
        }
    }

    fn board(header_dimmed: bool) -> BoardRead {
        let mut fields = vec![
            text("map", "Ilios"),
            text("mode", "Control"),
            text("result", "victory"),
            text("r0.hero", "wrecking-ball"),
        ];
        for (suffix, value) in [
            ("e", 21u32),
            ("a", 9),
            ("d", 3),
            ("dmg", 9100),
            ("h", 1200),
            ("mit", 50),
        ] {
            fields.push(number(&format!("r0.{suffix}"), value));
        }
        BoardRead {
            status: BoardStatus::Read,
            team_size: Some(6),
            fields,
            elapsed_ms: 1,
            header_dimmed,
        }
    }

    let read = board(false);
    let fallback = merge_saved(&snap(), OwnRow::Uncertain, Some(&read));
    let named = own_row(Some("Ada"), None, Some(0), 6);
    let kept = merge_saved(&snap(), named, Some(&read));
    let mut agreeing = snap();
    agreeing.map = "Ilios".into();
    let clean = merge_saved(&agreeing, named, Some(&read));

    assert_eq!(fallback.recognizer, scuffed_types::RECOGNIZER_OCR_V1);
    assert!(fallback.suspect_fields.is_empty());
    assert_eq!(kept.recognizer, RECOGNIZER_ID);
    assert_eq!(kept.hero, "Wrecking Ball");
    assert_eq!(kept.suspect_fields, vec!["map".to_string()]);
    assert_eq!(clean.recognizer, RECOGNIZER_ID);
    assert!(clean.suspect_fields.is_empty());

    let saved = [fallback, kept, clean];
    let mut rows = Vec::new();
    for (index, saved) in saved.iter().enumerate() {
        let mut match_row = row(&saved.hero, &saved.map, &saved.mode);
        match_row.session_id = format!("sess-{index}");
        match_row.elims = saved.elims;
        match_row.recognizer = saved.recognizer.to_string();
        match_row.suspect_fields = saved.suspect_fields.clone();
        rows.push(match_row);
    }

    let request = upload_request(&rows, &[]);
    assert_eq!(request.matches.len(), rows.len());
    let json = serde_json::to_value(&request).unwrap();
    let matches = json["matches"].as_array().expect("matches array");
    assert_eq!(matches.len(), saved.len());
    for (entry, saved) in matches.iter().zip(&saved) {
        let recognizer = entry
            .get("recognizer")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("recognizer omitted or not a string: {entry}"));
        assert_eq!(recognizer, saved.recognizer);
        assert_ne!(recognizer, "");
        match entry.get("suspect_fields") {
            None => assert!(
                saved.suspect_fields.is_empty(),
                "suspect_fields dropped: {entry}"
            ),
            Some(list) => {
                let names: Vec<String> = list
                    .as_array()
                    .unwrap_or_else(|| panic!("suspect_fields is not an array: {entry}"))
                    .iter()
                    .map(|name| {
                        name.as_str()
                            .unwrap_or_else(|| panic!("suspect name is not a string: {entry}"))
                            .to_string()
                    })
                    .collect();
                assert_eq!(names, saved.suspect_fields);
            }
        }
    }
    assert_eq!(matches[0]["recognizer"], "ocr-v1");
    assert!(matches[0].get("suspect_fields").is_none());
    assert_eq!(matches[1]["recognizer"], "cv-v8");
    assert_eq!(matches[1]["suspect_fields"], serde_json::json!(["map"]));
    assert_eq!(matches[1]["hero"], "Wrecking Ball");
    assert_eq!(matches[2]["recognizer"], "cv-v8");
    assert!(matches[2].get("suspect_fields").is_none());
    let raw = serde_json::to_string(&request).unwrap();
    assert!(!raw.contains("\"recognizer\":null"));
    assert!(raw.contains("\"recognizer\":\"ocr-v1\""));
    assert!(raw.contains("\"recognizer\":\"cv-v8\""));

    // The same agreeing board, with the header found only on the dimmed
    // strip. The upload carries that mark as own-row suspects. A clean
    // board of the same read lists none.
    let dimmed = merge_saved(&agreeing, named, Some(&board(true)));
    assert_eq!(dimmed.recognizer, RECOGNIZER_ID);
    assert!(
        saved[2].suspect_fields.is_empty(),
        "the clean board stays unmarked"
    );
    let mut dimmed_row = row(&dimmed.hero, &dimmed.map, &dimmed.mode);
    dimmed_row.session_id = "sess-dimmed".into();
    dimmed_row.elims = dimmed.elims;
    dimmed_row.recognizer = dimmed.recognizer.to_string();
    dimmed_row.suspect_fields = dimmed.suspect_fields.clone();
    let dimmed_json = serde_json::to_value(upload_request(&[dimmed_row], &[])).unwrap();
    let names = dimmed_json["matches"][0]["suspect_fields"]
        .as_array()
        .expect("dimmed row sends suspect_fields");
    for name in ["hero", "e", "a", "d", "dmg", "h", "mit"] {
        assert!(names.iter().any(|n| n == name), "{name} missing: {names:?}");
    }
}

#[test]
fn mixed_row_uploads_cv_v6_when_any_stat_came_from_the_new_reader() {
    use crate::reader_apply::{OwnRow, PolicyRow, merge_saved};
    use crate::shadow::digits::RECOGNIZER_ID;
    use crate::shadow::{BoardRead, BoardStatus, FieldRead, Value};

    fn snap() -> PolicyRow {
        PolicyRow {
            map: "Busan".into(),
            mode: "Control".into(),
            result: "victory".into(),
            hero: "Ana".into(),
            elims: 10,
            assists: 4,
            deaths: 2,
            damage: 4000,
            healing: 8000,
            mitigation: 100,
        }
    }

    fn text(name: &str, value: &str, suspect: bool) -> FieldRead {
        FieldRead {
            name: name.into(),
            value: Some(Value::Text(value.into())),
            confidence: if suspect { 0.2 } else { 0.95 },
            suspect,
        }
    }

    fn number(name: &str, value: u32, suspect: bool) -> FieldRead {
        FieldRead {
            name: name.into(),
            value: Some(Value::Int(value)),
            confidence: if suspect { 0.2 } else { 0.95 },
            suspect,
        }
    }

    fn board(elims_suspect: bool, header_dimmed: bool) -> BoardRead {
        BoardRead {
            status: BoardStatus::Read,
            team_size: Some(6),
            elapsed_ms: 1,
            fields: vec![
                text("map", "Busan", false),
                text("mode", "Control", false),
                text("result", "defeat", true),
                text("r0.hero", "tracer", true),
                number("r0.e", 21, elims_suspect),
                number("r0.a", 1, true),
                number("r0.d", 99, true),
                number("r0.dmg", 1, true),
                number("r0.h", 1, true),
                number("r0.mit", 1, true),
            ],
            header_dimmed,
        }
    }

    let own = OwnRow::Identified {
        index: 0,
        team_size: 6,
    };
    let policy = snap();
    let mixed = merge_saved(&policy, own, Some(&board(false, false)));
    assert_eq!(
        mixed.elims, 21,
        "the confident elim came from the new reader"
    );
    assert_eq!(
        mixed.deaths, policy.deaths,
        "a suspect death stays on ocr-v1"
    );
    assert_eq!(mixed.hero, policy.hero);
    assert_eq!(mixed.result, policy.result);
    assert_eq!(mixed.recognizer, RECOGNIZER_ID);
    assert!(mixed.suspect_fields.iter().any(|name| name == "d"));
    assert!(!mixed.suspect_fields.iter().any(|name| name == "e"));

    let all_fallback = merge_saved(&policy, own, Some(&board(true, false)));
    assert_eq!(all_fallback.elims, policy.elims);
    assert_eq!(all_fallback.deaths, policy.deaths);
    assert_eq!(all_fallback.hero, policy.hero);
    assert_eq!(all_fallback.recognizer, scuffed_types::RECOGNIZER_OCR_V1);
    assert!(all_fallback.suspect_fields.iter().any(|name| name == "e"));

    let mut rows = Vec::new();
    for (session, saved) in [("sess-mixed", &mixed), ("sess-fallback", &all_fallback)] {
        let mut match_row = row(&saved.hero, &saved.map, &saved.mode);
        match_row.session_id = session.into();
        match_row.outcome = saved.result.clone();
        match_row.elims = saved.elims;
        match_row.assists = saved.assists;
        match_row.deaths = saved.deaths;
        match_row.damage = saved.damage;
        match_row.healing = saved.healing;
        match_row.mitigation = saved.mitigation;
        match_row.recognizer = saved.recognizer.to_string();
        match_row.suspect_fields = saved.suspect_fields.clone();
        rows.push(match_row);
    }

    let request = upload_request(&rows, &[]);
    let json = serde_json::to_value(&request).unwrap();
    let matches = json["matches"].as_array().expect("matches array");
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0]["recognizer"], "cv-v8");
    assert_eq!(matches[0]["elims"], 21);
    assert_eq!(matches[0]["deaths"], policy.deaths);
    assert_eq!(matches[0]["hero"], "Ana");
    let suspects = matches[0]["suspect_fields"]
        .as_array()
        .expect("mixed row sends suspect_fields");
    assert!(suspects.iter().any(|name| name == "d"));
    assert!(suspects.iter().all(|name| name != "e"));
    assert_eq!(matches[1]["recognizer"], "ocr-v1");
    assert_eq!(matches[1]["elims"], policy.elims);
    assert_eq!(matches[1]["deaths"], policy.deaths);
    assert!(matches[1]["recognizer"].is_string());
    assert!(matches[0]["recognizer"].is_string());

    // Every cv field fell back, and the hero cell was not read. The
    // dimmed header still marks the own row, and the upload carries it.
    let mut dimmed_board = board(true, true);
    if let Some(hero) = dimmed_board.fields.iter_mut().find(|f| f.name == "r0.hero") {
        *hero = FieldRead {
            name: "r0.hero".into(),
            value: None,
            confidence: 0.0,
            suspect: true,
        };
    }
    let dimmed_fallback = merge_saved(&policy, own, Some(&dimmed_board));
    assert_eq!(dimmed_fallback.recognizer, scuffed_types::RECOGNIZER_OCR_V1);
    assert_eq!(dimmed_fallback.hero, policy.hero);
    let mut dimmed_row = row(
        &dimmed_fallback.hero,
        &dimmed_fallback.map,
        &dimmed_fallback.mode,
    );
    dimmed_row.session_id = "sess-dimmed-fallback".into();
    dimmed_row.recognizer = dimmed_fallback.recognizer.to_string();
    dimmed_row.suspect_fields = dimmed_fallback.suspect_fields.clone();
    let dimmed_json = serde_json::to_value(upload_request(&[dimmed_row], &[])).unwrap();
    assert_eq!(dimmed_json["matches"][0]["recognizer"], "ocr-v1");
    let names = dimmed_json["matches"][0]["suspect_fields"]
        .as_array()
        .expect("dimmed ocr-v1 row sends suspect_fields");
    assert!(
        names.iter().any(|name| name == "hero"),
        "dimmed ocr-v1 fallback dropped the own-row mark: {names:?}"
    );
}
