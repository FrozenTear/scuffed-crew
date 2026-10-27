//! `--fixture full`: a long history for screenshots of the 0.6 Maps and
//! Seasons tabs, built from the spec r1.1 example numbers (the
//! pre-redesign build's real per-map and per-season totals).
//!
//! 857 games: the 28 listed maps, 3 small maps that show "Too few
//! games" (7 to 9 games), 2 tiny maps that fold under "Show" (Hanaoka and
//! Throne of Anubis, under 3 games), and 23 games with no map. The
//! spec had 26 with no map and no tiny maps; 3 of those games moved so
//! the fixture draws the Show row. Seasons 3, 4 and 5 hold 366, 381 and
//! 110 of them with the spec's W-L-D and undecided counts; Seasons 1 and
//! 2 have none. The newest game is Oct 20 00:40 UTC, 49 days before Season 5
//! ends. Which game lands on which day is made up, so per-season map
//! splits are example data.

use chrono::{DateTime, Duration, TimeZone, Utc};
use stat_tracker::storage::Snapshot;
use surrealdb_types::Datetime as SurrealDatetime;

/// Name, games, wins, losses, draws. Undecided is the rest.
const MAPS: &[(&str, u32, u32, u32, u32)] = &[
    ("King's Row", 62, 28, 22, 1),
    ("Neon Junction", 61, 23, 27, 0),
    ("Suravasa", 56, 20, 27, 1),
    ("Rialto", 44, 16, 20, 0),
    ("Aatlis", 43, 20, 15, 0),
    ("Eichenwalde", 40, 13, 20, 0),
    ("Colosseo", 37, 15, 14, 0),
    ("Ilios", 36, 15, 12, 0),
    ("Lijiang Tower", 35, 14, 18, 0),
    ("Circuit Royal", 33, 14, 13, 0),
    ("Hollywood", 28, 12, 11, 2),
    ("Nepal", 28, 11, 13, 0),
    ("Midtown", 25, 6, 15, 0),
    ("Paraiso", 25, 11, 8, 2),
    ("Dorado", 24, 12, 7, 0),
    ("Watchpoint: Gibraltar", 24, 9, 13, 0),
    ("Antarctic Peninsula", 23, 14, 4, 0),
    ("Blizzard World", 22, 8, 11, 0),
    ("Busan", 21, 9, 8, 0),
    ("Havana", 21, 12, 8, 0),
    ("Shambali Monastery", 18, 5, 7, 0),
    ("Route 66", 17, 6, 9, 0),
    ("Oasis", 16, 5, 8, 1),
    ("Esperanca", 15, 5, 6, 0),
    ("New Junk City", 14, 6, 7, 0),
    ("New Queen Street", 14, 3, 8, 0),
    ("Runasapi", 13, 2, 8, 0),
    ("Samoa", 12, 5, 5, 0),
    ("Watchpoint: Grímsvötn", 9, 3, 5, 0),
    ("Numbani", 8, 3, 4, 0),
    ("Junkertown", 7, 3, 3, 0),
    ("Hanaoka", 2, 1, 0, 0),
    ("Throne of Anubis", 1, 0, 1, 0),
    // No map.
    ("", 23, 1, 13, 0),
];

/// Season id, wins, losses, draws, undecided (oldest first).
const QUOTAS: [(&str, u32, u32, u32, u32); 3] = [
    ("season-3", 134, 166, 5, 61),
    ("season-4", 144, 149, 2, 86),
    ("season-5", 52, 55, 0, 3),
];

/// Games with no hero read, every this many decided games.
const NO_HERO_EVERY: usize = 80;

pub fn seasons_json() -> &'static str {
    r#"[
  {"id": "season-1", "name": "Season 1", "starts_at": "2026-02-24T00:00:00Z", "ends_at": "2026-04-21T00:00:00Z", "is_current": false},
  {"id": "season-2", "name": "Season 2", "starts_at": "2026-04-21T00:00:00Z", "ends_at": "2026-06-16T00:00:00Z", "is_current": false},
  {"id": "season-3", "name": "Season 3", "starts_at": "2026-06-16T00:00:00Z", "ends_at": "2026-08-11T00:00:00Z", "is_current": false},
  {"id": "season-4", "name": "Season 4", "starts_at": "2026-08-11T00:00:00Z", "ends_at": "2026-10-06T00:00:00Z", "is_current": false},
  {"id": "season-5", "name": "Season 5", "starts_at": "2026-10-06T00:00:00Z", "ends_at": "2026-12-08T00:00:00Z", "is_current": true}
]"#
}

fn utc(m: u32, d: u32, hh: u32, mm: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, m, d, hh, mm, 0).unwrap()
}

/// Where each season's games are spread: its start to its end, or to the
/// newest game for Season 5.
fn span(season: usize) -> (DateTime<Utc>, DateTime<Utc>) {
    match season {
        0 => (utc(6, 16, 18, 0), utc(8, 10, 23, 0)),
        1 => (utc(8, 11, 18, 0), utc(10, 5, 23, 0)),
        _ => (utc(10, 6, 18, 0), utc(10, 20, 0, 40)),
    }
}

/// Outcome slots 0 to 3: win, loss, draw, undecided.
fn label(slot: usize) -> &'static str {
    ["victory", "defeat", "draw", "unknown"][slot]
}

/// Every game as (map, outcome slot), maps dealt round-robin so each
/// season gets a mix.
fn deck() -> Vec<(&'static str, usize)> {
    let mut left: Vec<[u32; 4]> = MAPS
        .iter()
        .map(|&(_, g, w, l, d)| [w, l, d, g - w - l - d])
        .collect();
    let mut out = Vec::new();
    loop {
        let mut dealt = false;
        for (i, counts) in left.iter_mut().enumerate() {
            if let Some(slot) = (0..4).find(|&s| counts[s] > 0) {
                counts[slot] -= 1;
                out.push((MAPS[i].0, slot));
                dealt = true;
            }
        }
        if !dealt {
            return out;
        }
    }
}

/// For each outcome slot, the season of its n-th game, spread in
/// proportion to [`QUOTAS`].
fn season_slots() -> [Vec<usize>; 4] {
    std::array::from_fn(|slot| {
        let mut keyed: Vec<(f64, usize)> = Vec::new();
        for (season, q) in QUOTAS.iter().enumerate() {
            let n = [q.1, q.2, q.3, q.4][slot];
            for j in 0..n {
                keyed.push(((f64::from(j) + 0.5) / f64::from(n), season));
            }
        }
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        keyed.into_iter().map(|(_, s)| s).collect()
    })
}

pub fn snapshot() -> Snapshot {
    let slots = season_slots();
    let mut taken = [0usize; 4];
    let mut per_season: [Vec<(&str, usize)>; 3] = Default::default();
    for (map, slot) in deck() {
        let season = slots[slot][taken[slot]];
        taken[slot] += 1;
        per_season[season].push((map, slot));
    }
    let mut matches = Vec::new();
    let mut decided = 0usize;
    for (season, games) in per_season.iter().enumerate() {
        let (from, to) = span(season);
        let step = (to - from).num_seconds() / games.len().max(1) as i64;
        for (i, &(map, slot)) in games.iter().enumerate() {
            let played = if i + 1 == games.len() {
                to
            } else {
                from + Duration::seconds(step * i as i64)
            };
            let n = matches.len();
            let (hero, role) = match n % 9 {
                4 => ("Orisa", "tank"),
                7 => ("Ana", "support"),
                _ => ("Wrecking Ball", "tank"),
            };
            let hero = if slot < 3 {
                decided += 1;
                if decided.is_multiple_of(NO_HERO_EVERY) {
                    ""
                } else {
                    hero
                }
            } else {
                hero
            };
            matches.push(crate::fixtures::pm(
                &format!("full-{n:04}"),
                hero,
                map,
                role,
                label(slot),
                20,
                7,
                9,
                9_000,
                0,
                11_000,
                SurrealDatetime::from(played),
            ));
        }
    }
    matches.reverse();
    Snapshot {
        matches,
        sessions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::{SeasonWindow, aggregate};
    use crate::model::{Game, Outcome};

    fn games() -> Vec<Game> {
        crate::snapshot::games_from_snapshot(&snapshot())
    }

    #[test]
    fn totals_match_the_spec() {
        let games = games();
        assert_eq!(games.len(), 857);
        let all = aggregate(&games, None, None).record;
        assert_eq!((all.wins, all.losses, all.draws), (330, 370, 7));
        assert_eq!(all.undecided(), 150);
        assert!(games.windows(2).all(|w| w[0].played_at >= w[1].played_at));
        assert_eq!(games[0].played_at, utc(10, 20, 0, 40));
    }

    #[test]
    fn seasons_match_the_spec() {
        let seasons: Vec<scuffed_types::Season> =
            serde_json::from_str(seasons_json()).expect("seasons");
        let games = games();
        let rec = |id: &str| {
            let s = seasons.iter().find(|s| s.id == id).expect(id);
            let w = SeasonWindow {
                starts_at: s.starts_at,
                ends_at: s.ends_at,
            };
            aggregate(&games, Some(w), None).record
        };
        let s5 = rec("season-5");
        assert_eq!((s5.games, s5.wins, s5.losses, s5.draws), (110, 52, 55, 0));
        let s4 = rec("season-4");
        assert_eq!((s4.games, s4.wins, s4.losses, s4.draws), (381, 144, 149, 2));
        let s3 = rec("season-3");
        assert_eq!((s3.games, s3.wins, s3.losses, s3.draws), (366, 134, 166, 5));
        assert_eq!(rec("season-1").games, 0);
    }

    #[test]
    fn maps_match_the_spec_table() {
        let games = games();
        let refs: Vec<&Game> = games.iter().collect();
        let (rows, no_map) = crate::maps_tab::map_rows(&refs);
        assert_eq!(no_map, 23);
        assert_eq!(rows.len(), 33);
        let (_, folded) = crate::maps_tab::split_fold(rows.clone());
        assert_eq!(folded.len(), 2, "the Show row is drawn");
        let too_few = rows
            .iter()
            .filter(|r| r.sample == crate::small_sample::Sample::TooFew)
            .count();
        assert!(too_few >= 3, "Junkertown, Numbani and Grímsvötn at least");
        let kings = rows.iter().find(|r| r.name == "King's Row").expect("row");
        assert_eq!(kings.record.wl_label(), "28-22-1");
        assert_eq!(kings.pct, Some(55));
        assert!(
            games
                .iter()
                .any(|g| g.hero.is_empty() && g.outcome != Outcome::Unknown)
        );
    }
}
