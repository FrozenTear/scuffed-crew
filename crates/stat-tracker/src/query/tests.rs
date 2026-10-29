//! Wire format and answers, without a daemon.

use super::proto::*;
use crate::games::aggregate::{GameFilter, aggregate_filtered};
use crate::games::model::{Game, GameOcr, Outcome, Role};

fn game(sid: &str, hero: &str, role: Role, outcome: Outcome, mins: i64) -> Game {
    Game {
        session_id: sid.into(),
        hero: hero.into(),
        map_name: "Busan".into(),
        role,
        outcome,
        elims: 1,
        deaths: 1,
        assists: 1,
        damage: 1,
        healing: 1,
        mitigation: 1,
        played_at: chrono::DateTime::from_timestamp(1_800_000_000 + mins * 60, 0).unwrap(),
        edited: false,
        edited_fields: Vec::new(),
        ocr: GameOcr::default(),
        segments: Vec::new(),
        upload_reject: None,
        suspect_fields: Vec::new(),
        synced: true,
        recognizer: "cv-v8".into(),
    }
}

#[test]
fn requests_round_trip_and_bad_lines_are_refused() {
    let req = Request::Totals {
        filter: GameFilter {
            roles: vec![Role::Support],
            hero: Some("Ana".into()),
            ..GameFilter::default()
        },
    };
    let line = serde_json::to_vec(&RequestLine {
        v: PROTO,
        request: req.clone(),
    })
    .unwrap();
    assert_eq!(parse_request(&line).unwrap(), req);
    assert!(matches!(
        parse_request(b"{\"v\":1,\"q\":\"rev\"}"),
        Ok(Request::Rev)
    ));
    let bad = |b: &[u8]| match parse_request(b) {
        Err(Response::Err { code, .. }) => code,
        other => panic!("{other:?}"),
    };
    assert_eq!(bad(b"not json"), "bad_request");
    assert_eq!(bad(b"{\"v\":1,\"q\":\"drop_tables\"}"), "bad_request");
    assert_eq!(bad(b"{\"v\":2,\"q\":\"rev\"}"), "unsupported_proto");
}

#[test]
fn totals_are_the_pages_own_totals() {
    let games = vec![
        game("a", "Ana", Role::Support, Outcome::Win, 0),
        game("b", "Ana", Role::Support, Outcome::Loss, 10),
        game("c", "Reinhardt", Role::Tank, Outcome::Win, 20),
    ];
    for filter in [
        GameFilter::default(),
        GameFilter {
            roles: vec![Role::Tank],
            ..GameFilter::default()
        },
        GameFilter {
            hero: Some("Ana".into()),
            outcome: Some(Outcome::Win),
            ..GameFilter::default()
        },
    ] {
        let Response::Totals { rev, totals } = answer(
            &Request::Totals {
                filter: filter.clone(),
            },
            7,
            &games,
        ) else {
            panic!("totals");
        };
        assert_eq!(rev, 7);
        assert_eq!(totals, aggregate_filtered(&games, &filter));
        // And they survive the wire.
        let back: Response = serde_json::from_slice(&encode(&Response::Totals {
            rev,
            totals: totals.clone(),
        }))
        .unwrap();
        assert_eq!(back, Response::Totals { rev, totals });
    }
    let Response::Games { games: kept, .. } = answer(
        &Request::Games {
            filter: GameFilter {
                roles: vec![Role::Support],
                ..GameFilter::default()
            },
        },
        7,
        &games,
    ) else {
        panic!("games");
    };
    assert_eq!(kept, games[..2].to_vec());
}

#[test]
fn a_long_data_dir_gets_a_short_socket_path() {
    let short = std::path::Path::new("/home/u/.local/share/scuffed-stat-tracker");
    assert_eq!(super::socket_path(short), short.join(super::SOCKET_FILE));
    let long = std::path::PathBuf::from(format!("/home/u/{}", "x".repeat(120)));
    let p = super::socket_path(&long);
    assert!(p.as_os_str().len() <= 107, "{}", p.display());
    assert_ne!(
        super::socket_path(&long),
        super::socket_path(&long.join("y"))
    );
}
