//! Demo snapshots so Overview can be shown empty and with data.
//!
//! Written to a temp (or `--data-dir`) `live_snapshot.json` and re-read through
//! `stat_tracker::storage::read_snapshot` so the spike exercises the real API.

use chrono::{TimeZone, Utc};
use stat_tracker::storage::{MatchSession, PersonalMatch, Snapshot};
use surrealdb_types::Datetime as SurrealDatetime;

use crate::cli::FixtureKind;

pub fn snapshot(kind: FixtureKind) -> Snapshot {
    match kind {
        FixtureKind::Empty => Snapshot {
            matches: Vec::new(),
            sessions: Vec::new(),
        },
        FixtureKind::Sample => sample_snapshot(),
    }
}

pub fn sample_seasons_json() -> &'static str {
    r#"[
  {
    "id": "season-16",
    "name": "Season 16",
    "starts_at": "2026-06-24T00:00:00Z",
    "ends_at": "2026-09-01T00:00:00Z",
    "is_current": false
  },
  {
    "id": "season-17",
    "name": "Season 17",
    "starts_at": "2026-09-01T00:00:00Z",
    "ends_at": "2026-12-10T00:00:00Z",
    "is_current": true
  }
]"#
}

fn dt(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> SurrealDatetime {
    SurrealDatetime::from(Utc.with_ymd_and_hms(y, m, d, hh, mm, 0).unwrap())
}

#[allow(clippy::too_many_arguments)]
fn pm(
    session_id: &str,
    hero: &str,
    map: &str,
    role: &str,
    outcome: &str,
    elims: u32,
    deaths: u32,
    assists: u32,
    damage: u32,
    healing: u32,
    mitigation: u32,
    played_at: SurrealDatetime,
) -> PersonalMatch {
    PersonalMatch {
        id: None,
        hero: hero.into(),
        map_name: map.into(),
        game_mode: "competitive".into(),
        role: role.into(),
        outcome: outcome.into(),
        elims,
        deaths,
        assists,
        damage,
        healing,
        mitigation,
        played_at,
        synced: false,
        session_id: session_id.into(),
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
    }
}

fn sample_snapshot() -> Snapshot {
    // Newest first, matching `get_all_matches` / live_snapshot.json.
    // Each session_id is one game (latest_per_game). Tonight is the local day.
    let matches = vec![
        pm(
            "sess-t1",
            "Junker Queen",
            "King's Row",
            "Tank",
            "victory",
            28,
            7,
            9,
            12400,
            0,
            18600,
            dt(2026, 9, 2, 21, 14),
        ),
        pm(
            "sess-t2",
            "Ashe",
            "Busan",
            "Damage",
            "defeat",
            19,
            11,
            4,
            9800,
            0,
            0,
            dt(2026, 9, 2, 20, 51),
        ),
        pm(
            "sess-t3",
            "Ana",
            "Ilios",
            "Support",
            "victory",
            14,
            5,
            16,
            4200,
            11200,
            0,
            dt(2026, 9, 2, 20, 28),
        ),
        pm(
            "sess-s16-1",
            "Reinhardt",
            "Numbani",
            "Tank",
            "victory",
            22,
            8,
            11,
            8100,
            0,
            24100,
            dt(2026, 8, 18, 19, 40),
        ),
        pm(
            "sess-s16-2",
            "Soldier: 76",
            "Havana",
            "Damage",
            "defeat",
            16,
            9,
            5,
            10200,
            800,
            0,
            dt(2026, 8, 18, 19, 12),
        ),
    ];
    let sessions = vec![
        MatchSession {
            session_id: "sess-t1".into(),
            hero: "Junker Queen".into(),
            map_name: "King's Row".into(),
            role: "Tank".into(),
            started_at: dt(2026, 9, 2, 21, 14),
            last_capture_at: dt(2026, 9, 2, 21, 14),
            capture_count: 1,
            final_outcome: "victory".into(),
        },
        MatchSession {
            session_id: "sess-s16-1".into(),
            hero: "Reinhardt".into(),
            map_name: "Numbani".into(),
            role: "Tank".into(),
            started_at: dt(2026, 8, 18, 19, 40),
            last_capture_at: dt(2026, 8, 18, 19, 40),
            capture_count: 1,
            final_outcome: "victory".into(),
        },
    ];
    Snapshot { matches, sessions }
}
