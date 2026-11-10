//! Season-window aggregation, pure function, UI crate only (design §6).
//!
//! A game belongs to a window when `starts_at <= played_at < ends_at` (UTC),
//! matching `GET /api/public/seasons` / the website.

use chrono::{DateTime, Datelike, Local, NaiveDate, Utc};

use crate::games::model::{Game, Outcome, Role, RoleFilter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SeasonWindow {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

impl SeasonWindow {
    pub fn contains(self, played_at: DateTime<Utc>) -> bool {
        self.starts_at <= played_at && played_at < self.ends_at
    }
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct GameFilter {
    pub window: Option<SeasonWindow>,
    pub roles: Vec<Role>,
    pub hero: Option<String>,
    pub map: Option<String>,
    pub outcome: Option<Outcome>,
}

impl GameFilter {
    pub fn from_header(window: Option<SeasonWindow>, roles: RoleFilter) -> Self {
        Self {
            window,
            roles: roles.selected_roles(),
            hero: None,
            map: None,
            outcome: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Record {
    pub games: usize,
    pub wins: usize,
    pub losses: usize,
    pub draws: usize,
}

impl Record {
    pub fn win_rate(&self) -> f32 {
        let decided = self.wins + self.losses + self.draws;
        if decided == 0 {
            0.0
        } else {
            self.wins as f32 / decided as f32
        }
    }

    pub fn win_rate_pct(&self) -> f32 {
        self.win_rate() * 100.0
    }

    /// `4-1` or `4-1-1` when draws are present (Games session header).
    pub fn wl_label(&self) -> String {
        if self.draws > 0 {
            format!("{}-{}-{}", self.wins, self.losses, self.draws)
        } else {
            format!("{}-{}", self.wins, self.losses)
        }
    }

    pub fn games_label(&self) -> String {
        if self.games == 1 {
            "1 game".into()
        } else {
            format!("{} games", self.games)
        }
    }

    /// Games whose outcome is not win/loss/draw. They count in `games` but
    /// not in [`Self::win_rate`] (decided-only, same as P0/P1).
    pub fn undecided(&self) -> usize {
        self.games
            .saturating_sub(self.wins + self.losses + self.draws)
    }

    /// Seasons-screen line: `62 games · 10-15-0 · 37 undecided`.
    /// The undecided tail is omitted when every game is decided.
    pub fn season_line(&self) -> String {
        let base = format!(
            "{} · {}-{}-{}",
            self.games_label(),
            self.wins,
            self.losses,
            self.draws
        );
        match self.undecided() {
            0 => base,
            1 => format!("{base} · 1 undecided"),
            n => format!("{base} · {n} undecided"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HeroAgg {
    pub hero: String,
    pub role: Role,
    pub record: Record,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MapAgg {
    pub map_name: String,
    pub record: Record,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RoleAgg {
    pub role: Role,
    pub record: Record,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Aggregates {
    pub record: Record,
    pub heroes: Vec<HeroAgg>,
    pub maps: Vec<MapAgg>,
    pub roles: Vec<RoleAgg>,
}

/// One sitting of games (daemon session window: 30-minute idle gap).
/// Header uses the local date of the oldest game (`Sat Aug 30 · 4-1`).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionGroup {
    pub heading: String,
    pub started_on: NaiveDate,
    pub record: Record,
    pub games: Vec<Game>,
}

/// Matches `Config::session_window_secs` default (30 minutes).
pub const DEFAULT_SESSION_GAP: chrono::TimeDelta = chrono::TimeDelta::seconds(1800);

/// A window or gap of `secs` seconds as a `TimeDelta`. A value too big for
/// one (a hand-edited config) saturates instead of panicking.
pub fn secs_delta(secs: u64) -> chrono::TimeDelta {
    i64::try_from(secs)
        .ok()
        .and_then(chrono::TimeDelta::try_seconds)
        .unwrap_or(chrono::TimeDelta::MAX)
}

/// The Session window setting as the gap that splits sittings. 0 turns the
/// window off: each game is its own session.
pub fn session_window_gap(window_secs: u64) -> chrono::TimeDelta {
    if window_secs == 0 {
        chrono::TimeDelta::MIN
    } else {
        secs_delta(window_secs)
    }
}

/// The one sitting rule the Games and Overview tabs share: `older` is in
/// `newer`'s sitting when no more than `gap` passes from `older`'s end
/// (its last Tab read) to `newer`'s start (its first Tab read). A row with
/// no Tab reads kept counts as one moment, `played_at`.
pub fn same_sitting(newer: &Game, older: &Game, gap: chrono::TimeDelta) -> bool {
    newer.started_at() - older.ended_at() <= gap
}

/// Filter + aggregate. `window = None` is all time. `role = None` keeps every role.
pub fn aggregate(games: &[Game], window: Option<SeasonWindow>, role: Option<Role>) -> Aggregates {
    aggregate_filtered(
        games,
        &GameFilter {
            window,
            roles: role.into_iter().collect(),
            ..GameFilter::default()
        },
    )
}

/// [`aggregate_filtered`] for games already filtered, so a page that
/// needs both the games and their totals filters once.
pub fn aggregate_games(games: &[&Game]) -> Aggregates {
    aggregate_rows(games)
}

pub fn aggregate_filtered(games: &[Game], filter: &GameFilter) -> Aggregates {
    aggregate_rows(&filter_games(games, filter))
}

pub fn filter_games<'a>(games: &'a [Game], filter: &GameFilter) -> Vec<&'a Game> {
    games
        .iter()
        .filter(|g| filter.window.is_none_or(|w| w.contains(g.played_at)))
        .filter(|g| filter.roles.is_empty() || filter.roles.contains(&g.role))
        .filter(|g| {
            filter
                .hero
                .as_ref()
                .is_none_or(|h| hero_key(&g.hero) == hero_key(h))
        })
        .filter(|g| filter.map.as_ref().is_none_or(|m| map_matches(g, m)))
        .filter(|g| filter.outcome.is_none_or(|o| g.outcome == o))
        .collect()
}

/// The name one hero is grouped under: the reader's name, so a row an
/// older picker saved as "Lucio" or "Torbjorn" joins Lúcio or Torbjörn.
/// A name that is not a known hero stays as it is.
///
/// It runs for every game on each Games and Overview redraw, so a name
/// already in the reader's form returns at once, and any other name is
/// looked up once per thread and cached.
pub fn hero_name(name: &str) -> &str {
    if scuffed_types::HERO_NAMES.iter().any(|h| h.display == name) {
        return name;
    }
    thread_local! {
        static SEEN: std::cell::RefCell<std::collections::HashMap<String, Option<&'static str>>> =
            Default::default();
    }
    let known = SEEN.with_borrow_mut(|seen| match seen.get(name) {
        Some(known) => *known,
        None => {
            let known = scuffed_types::hero_key_to_name(name);
            seen.insert(name.to_string(), known);
            known
        }
    });
    known.unwrap_or(name)
}

/// The heroes the Hero dropdown offers, once each under the name the
/// stats use ([`hero_key`]), so "soldier_76" and "Soldier: 76" are one
/// choice and picking it matches both.
pub fn distinct_heroes(games: &[&Game]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for g in games {
        let key = hero_key(&g.hero);
        if !g.hero.is_empty() && !out.contains(&key) {
            out.push(key);
        }
    }
    out.sort();
    out
}

/// The Map dropdown's one choice for every game with no known map: empty,
/// or a raw read that is not a map ("Ilio5"). The rows show those as "No
/// map", so the choice says that and finds them all ([`map_matches`]).
pub const NO_MAP: &str = "No map";

/// Whether `name` is a map the rows show by name: not empty, and a known
/// map. Any other read shows as [`NO_MAP`].
pub fn map_is_shown(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && crate::parse::map_is_known(name)
}

/// Whether game `g` matches the Map filter `map`: [`NO_MAP`] matches every
/// game whose map is not shown by name, any other choice its own map.
pub fn map_matches(g: &Game, map: &str) -> bool {
    if map == NO_MAP {
        !map_is_shown(&g.map_name)
    } else {
        g.map_name == map
    }
}

/// The Map dropdown's choices: each known map once, A to Z, then
/// [`NO_MAP`] when any game has no known map. A raw read that is not a map
/// is never offered as if it were one.
pub fn distinct_maps(games: &[&Game]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut no_map = false;
    for g in games {
        if !map_is_shown(&g.map_name) {
            no_map = true;
        } else if !out.iter().any(|m| m == &g.map_name) {
            out.push(g.map_name.clone());
        }
    }
    out.sort();
    if no_map {
        out.push(NO_MAP.to_string());
    }
    out
}

/// Group newest-first games into sittings: a game continues the current
/// sitting when it ended within `gap` of the start of the oldest game
/// already in that sitting ([`same_sitting`], the Games tab's rule too).
/// Midnight does not split a sitting. Header date is the oldest game's
/// local day.
pub fn group_by_session_gap(games: &[&Game], gap: chrono::TimeDelta) -> Vec<SessionGroup> {
    let mut groups: Vec<SessionGroup> = Vec::new();
    for g in games {
        let continues = groups
            .last()
            .and_then(|grp| grp.games.last())
            .is_some_and(|prev| same_sitting(prev, g, gap));
        if continues {
            if let Some(grp) = groups.last_mut() {
                bump_outcome(&mut grp.record, g.outcome);
                grp.record.games += 1;
                grp.games.push((*g).clone());
            }
        } else {
            let mut record = Record {
                games: 1,
                wins: 0,
                losses: 0,
                draws: 0,
            };
            bump_outcome(&mut record, g.outcome);
            groups.push(SessionGroup {
                heading: String::new(),
                started_on: g.played_at.with_timezone(&Local).date_naive(),
                record,
                games: vec![(*g).clone()],
            });
        }
    }
    for grp in &mut groups {
        if let Some(oldest) = grp.games.last() {
            grp.started_on = oldest.played_at.with_timezone(&Local).date_naive();
        }
        grp.heading = format!("{} · {}", format_day(grp.started_on), grp.record.wl_label());
    }
    groups
}

fn format_day(day: NaiveDate) -> String {
    format!(
        "{} {} {}",
        weekday_short(day.weekday().num_days_from_sunday()),
        month_short(day.month()),
        day.day()
    )
}

fn weekday_short(from_sunday: u32) -> &'static str {
    match from_sunday {
        0 => "Sun",
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        _ => "Sat",
    }
}

fn month_short(m: u32) -> &'static str {
    match m {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        _ => "Dec",
    }
}

fn aggregate_rows(filtered: &[&Game]) -> Aggregates {
    let mut record = Record {
        games: filtered.len(),
        wins: 0,
        losses: 0,
        draws: 0,
    };
    let mut hero_map: Vec<(String, Role, Record)> = Vec::new();
    let mut map_map: Vec<(String, Record)> = Vec::new();
    let mut role_map: Vec<(Role, Record)> = Vec::new();

    for g in filtered {
        bump_outcome(&mut record, g.outcome);
        bump_hero(&mut hero_map, g);
        if !g.map_name.is_empty() {
            bump_map(&mut map_map, g);
        }
        bump_role(&mut role_map, g);
    }

    hero_map.sort_by(|a, b| b.2.games.cmp(&a.2.games).then_with(|| a.0.cmp(&b.0)));
    map_map.sort_by(|a, b| b.1.games.cmp(&a.1.games).then_with(|| a.0.cmp(&b.0)));
    let role_order = [Role::Tank, Role::Damage, Role::Support];
    role_map.sort_by_key(|(r, _)| role_order.iter().position(|o| o == r).unwrap_or(99));

    Aggregates {
        record,
        heroes: hero_map
            .into_iter()
            .map(|(hero, role, record)| HeroAgg { hero, role, record })
            .collect(),
        maps: map_map
            .into_iter()
            .map(|(map_name, record)| MapAgg { map_name, record })
            .collect(),
        roles: role_map
            .into_iter()
            .map(|(role, record)| RoleAgg { role, record })
            .collect(),
    }
}

fn bump_outcome(r: &mut Record, outcome: Outcome) {
    match outcome {
        Outcome::Win => r.wins += 1,
        Outcome::Loss => r.losses += 1,
        Outcome::Draw => r.draws += 1,
        Outcome::Unknown => {}
    }
}

/// The name a hero's games count under: the catalog spelling, so
/// "soldier_76" and "Soldier: 76" are one hero. An unknown or blank name
/// stays as stored.
///
/// This runs for every game on every redraw, so a name that is already a
/// display name [`canonical_hero`] returns (almost every stored name)
/// skips its allocating scan of the hero table. The fast path checks
/// `HERO_NAMES`, not `HEROES`: `HEROES` spells "Lucio" and "Torbjorn",
/// which `canonical_hero` turns into "Lúcio" and "Torbjörn".
///
/// [`canonical_hero`]: crate::parse::canonical_hero
pub fn hero_key(name: &str) -> String {
    let t = name.trim();
    if let Some(known) = scuffed_types::HERO_NAMES.iter().find(|h| h.display == t) {
        known.display.to_string()
    } else if crate::parse::hero_is_unknown_label(name) {
        name.to_string()
    } else {
        crate::parse::canonical_hero(t)
    }
}

fn bump_hero(rows: &mut Vec<(String, Role, Record)>, g: &Game) {
    let key = hero_key(&g.hero);
    if let Some((_, _, rec)) = rows.iter_mut().find(|(h, _, _)| h == &key) {
        rec.games += 1;
        bump_outcome(rec, g.outcome);
    } else {
        let mut rec = Record {
            games: 1,
            wins: 0,
            losses: 0,
            draws: 0,
        };
        bump_outcome(&mut rec, g.outcome);
        rows.push((key, g.role, rec));
    }
}

fn bump_map(rows: &mut Vec<(String, Record)>, g: &Game) {
    if let Some((_, rec)) = rows.iter_mut().find(|(n, _)| n == &g.map_name) {
        rec.games += 1;
        bump_outcome(rec, g.outcome);
    } else {
        let mut rec = Record {
            games: 1,
            wins: 0,
            losses: 0,
            draws: 0,
        };
        bump_outcome(&mut rec, g.outcome);
        rows.push((g.map_name.clone(), rec));
    }
}

fn bump_role(rows: &mut Vec<(Role, Record)>, g: &Game) {
    if let Some((_, rec)) = rows.iter_mut().find(|(r, _)| *r == g.role) {
        rec.games += 1;
        bump_outcome(rec, g.outcome);
    } else {
        let mut rec = Record {
            games: 1,
            wins: 0,
            losses: 0,
            draws: 0,
        };
        bump_outcome(&mut rec, g.outcome);
        rows.push((g.role, rec));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::model::GameOcr;
    use chrono::TimeZone;

    fn game(hero: &str, role: Role, outcome: Outcome, at: DateTime<Utc>) -> Game {
        game_on("King's Row", hero, role, outcome, at)
    }

    fn game_on(map: &str, hero: &str, role: Role, outcome: Outcome, at: DateTime<Utc>) -> Game {
        Game {
            session_id: format!("s-{hero}-{at}"),
            hero: hero.into(),
            map_name: map.into(),
            role,
            outcome,
            elims: 12,
            deaths: 4,
            assists: 6,
            damage: 8000,
            healing: 0,
            mitigation: 0,
            played_at: at,
            edited: false,
            edited_fields: Vec::new(),
            ocr: GameOcr::default(),
            segments: Vec::new(),
            upload_reject: None,
            suspect_fields: Vec::new(),
            synced: false,
            recognizer: String::new(),
            bans_picks: None,
            bans_picks_status: None,
            queue_title: None,
        }
    }

    fn ts(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    fn ts_h(y: i32, m: u32, d: u32, hh: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, hh, 0, 0).unwrap()
    }

    #[test]
    fn hero_key_keeps_catalog_names_and_folds_other_spellings() {
        assert_eq!(hero_key("Ana"), "Ana");
        assert_eq!(hero_key(" Soldier: 76 "), "Soldier: 76");
        assert_eq!(hero_key("soldier_76"), "Soldier: 76");
        assert_eq!(hero_key("wrecking-ball"), "Wrecking Ball");
        assert_eq!(
            hero_key("Unknown"),
            "Unknown",
            "an unknown hero stays as stored"
        );
        assert_eq!(hero_key(""), "");
    }

    #[test]
    fn hero_key_matches_canonical_hero_for_every_catalog_name() {
        use crate::parse::canonical_hero;
        let names = scuffed_types::HEROES
            .iter()
            .copied()
            .chain(scuffed_types::HERO_NAMES.iter().map(|h| h.display));
        for n in names {
            assert_eq!(hero_key(n), canonical_hero(n), "{n}");
        }
        assert_eq!(hero_key("Lucio"), hero_key("Lúcio"));
        assert_eq!(hero_key("Torbjorn"), hero_key("Torbjörn"));
    }

    #[test]
    fn half_open_window_excludes_end_and_keeps_start() {
        let window = SeasonWindow {
            starts_at: ts(2026, 9, 1),
            ends_at: ts(2026, 12, 1),
        };
        let rows = vec![
            game("Ana", Role::Support, Outcome::Win, ts(2026, 8, 31)),
            game("Ana", Role::Support, Outcome::Win, ts(2026, 9, 1)),
            game("Ashe", Role::Damage, Outcome::Loss, ts(2026, 11, 30)),
            game("Reinhardt", Role::Tank, Outcome::Loss, ts(2026, 12, 1)),
        ];
        let agg = aggregate(&rows, Some(window), None);
        assert_eq!(agg.record.games, 2);
        assert_eq!(agg.record.wins, 1);
        assert_eq!(agg.record.losses, 1);
        assert!(
            (agg.record.win_rate() - 0.5).abs() < f32::EPSILON,
            "win rate {}",
            agg.record.win_rate()
        );
    }

    /// The fast path and the cache give what the reader's lookup gives.
    #[test]
    fn hero_name_matches_the_reader_lookup() {
        let names = scuffed_types::HERO_NAMES.iter().map(|h| h.display);
        let extra = [
            "Lucio",
            "lucio",
            "Torbjorn",
            "soldier 76",
            "",
            "Unknown",
            "Nobody",
        ];
        for name in names.chain(extra) {
            let want = scuffed_types::hero_key_to_name(name).unwrap_or(name);
            assert_eq!(hero_name(name), want, "{name}");
            assert_eq!(hero_name(name), want, "{name} cached");
        }
    }

    /// A row an older picker saved as "Lucio" or "Torbjorn" counts with
    /// the reader's Lúcio or Torbjörn: one row, one filter option.
    #[test]
    fn older_hero_spellings_share_one_row() {
        let rows = vec![
            game("Lucio", Role::Support, Outcome::Win, ts(2026, 9, 2)),
            game("Lúcio", Role::Support, Outcome::Loss, ts(2026, 9, 3)),
            game("Torbjorn", Role::Damage, Outcome::Win, ts(2026, 9, 4)),
            game("Torbjörn", Role::Damage, Outcome::Win, ts(2026, 9, 5)),
        ];
        let agg = aggregate(&rows, None, None);
        let heroes: Vec<(&str, usize)> = agg
            .heroes
            .iter()
            .map(|h| (h.hero.as_str(), h.record.games))
            .collect();
        assert_eq!(heroes.len(), 2, "{heroes:?}");
        assert!(heroes.contains(&("Lúcio", 2)), "{heroes:?}");
        assert!(heroes.contains(&("Torbjörn", 2)), "{heroes:?}");
        let all: Vec<&Game> = rows.iter().collect();
        assert_eq!(distinct_heroes(&all), vec!["Lúcio", "Torbjörn"]);
        let filter = GameFilter {
            hero: Some("Lúcio".into()),
            ..GameFilter::default()
        };
        assert_eq!(filter_games(&rows, &filter).len(), 2);
    }

    #[test]
    fn all_time_and_role_filter() {
        let rows = vec![
            game("Ana", Role::Support, Outcome::Win, ts(2026, 8, 1)),
            game("Ashe", Role::Damage, Outcome::Loss, ts(2026, 9, 2)),
            game("Junker Queen", Role::Tank, Outcome::Win, ts(2026, 9, 2)),
        ];
        let all = aggregate(&rows, None, None);
        assert_eq!(all.record.games, 3);
        assert_eq!(all.heroes.len(), 3);
        let tank = aggregate(&rows, None, Some(Role::Tank));
        assert_eq!(tank.record.games, 1);
        assert_eq!(tank.heroes[0].hero, "Junker Queen");
    }

    #[test]
    fn unknown_outcomes_do_not_count_as_decided() {
        let rows = vec![
            game("Ana", Role::Support, Outcome::Win, ts(2026, 9, 2)),
            game("Ana", Role::Support, Outcome::Unknown, ts(2026, 9, 2)),
        ];
        let agg = aggregate(&rows, None, None);
        assert_eq!(agg.record.games, 2);
        assert_eq!(agg.record.wins, 1);
        assert!((agg.record.win_rate() - 1.0).abs() < f32::EPSILON);
    }

    /// Games immediately before and on a season start stay on the correct
    /// side of the half-open window; hero/map lists and day groups follow.
    #[test]
    fn season_boundary_filters_hero_map_and_day_groups() {
        let window = SeasonWindow {
            starts_at: ts_h(2026, 9, 1, 0),
            ends_at: ts_h(2026, 12, 10, 0),
        };
        let before = game_on(
            "Numbani",
            "Reinhardt",
            Role::Tank,
            Outcome::Win,
            ts_h(2026, 8, 31, 23),
        );
        let on_start = game_on(
            "King's Row",
            "Junker Queen",
            Role::Tank,
            Outcome::Loss,
            ts_h(2026, 9, 1, 0),
        );
        let later = game_on(
            "Ilios",
            "Ana",
            Role::Support,
            Outcome::Win,
            ts_h(2026, 9, 1, 21),
        );
        let rows = vec![later.clone(), on_start.clone(), before.clone()];

        let in_s17 = filter_games(
            &rows,
            &GameFilter {
                window: Some(window),
                ..GameFilter::default()
            },
        );
        assert_eq!(in_s17.len(), 2);
        assert_eq!(in_s17[0].hero, "Ana");
        assert_eq!(in_s17[1].hero, "Junker Queen");
        assert!(in_s17.iter().all(|g| g.hero != "Reinhardt"));

        let tank_s17 = aggregate_filtered(
            &rows,
            &GameFilter {
                window: Some(window),
                roles: vec![Role::Tank],
                ..GameFilter::default()
            },
        );
        assert_eq!(tank_s17.record.games, 1);
        assert_eq!(tank_s17.heroes.len(), 1);
        assert_eq!(tank_s17.heroes[0].hero, "Junker Queen");
        assert_eq!(tank_s17.maps.len(), 1);
        assert_eq!(tank_s17.maps[0].map_name, "King's Row");

        let hero_only = filter_games(
            &rows,
            &GameFilter {
                window: Some(window),
                hero: Some("Ana".into()),
                ..GameFilter::default()
            },
        );
        assert_eq!(hero_only.len(), 1);
        assert_eq!(hero_only[0].map_name, "Ilios");

        let map_only = filter_games(
            &rows,
            &GameFilter {
                window: Some(window),
                map: Some("King's Row".into()),
                ..GameFilter::default()
            },
        );
        assert_eq!(map_only.len(), 1);
        assert_eq!(map_only[0].outcome, Outcome::Loss);

        // Far-apart sittings stay split even if we used calendar days before.
        let day_a = game("Ashe", Role::Damage, Outcome::Win, ts_h(2026, 8, 18, 12));
        let day_b1 = game("Ana", Role::Support, Outcome::Win, ts_h(2026, 9, 2, 12));
        let day_b2 = game(
            "Ashe",
            Role::Damage,
            Outcome::Loss,
            ts_h(2026, 9, 2, 12) + chrono::TimeDelta::minutes(12),
        );
        let day_b3 = game(
            "Junker Queen",
            Role::Tank,
            Outcome::Win,
            ts_h(2026, 9, 2, 12) + chrono::TimeDelta::minutes(24),
        );
        let sitting = vec![&day_b3, &day_b2, &day_b1, &day_a];
        let groups = group_by_session_gap(&sitting, DEFAULT_SESSION_GAP);
        assert_eq!(
            groups.len(),
            2,
            "Aug 18 and Sep 2 must stay split: {groups:?}"
        );
        assert!(
            groups[0].heading.contains("· 2-1"),
            "newest sitting heading {}",
            groups[0].heading
        );
        assert!(
            groups[1].heading.contains("· 1-0"),
            "older sitting heading {}",
            groups[1].heading
        );
        assert!(groups[0].heading.contains("Sep"), "{}", groups[0].heading);
        assert!(groups[1].heading.contains("Aug"), "{}", groups[1].heading);
    }

    #[test]
    fn sitting_gap_keeps_midnight_together_and_splits_idle() {
        let after = game(
            "Ana",
            Role::Support,
            Outcome::Win,
            Utc.with_ymd_and_hms(2026, 8, 29, 0, 10, 0).unwrap(),
        );
        let before = game(
            "Ashe",
            Role::Damage,
            Outcome::Loss,
            Utc.with_ymd_and_hms(2026, 8, 28, 23, 50, 0).unwrap(),
        );
        let earlier = game(
            "Reinhardt",
            Role::Tank,
            Outcome::Win,
            Utc.with_ymd_and_hms(2026, 8, 28, 22, 00, 0).unwrap(),
        );
        let night = group_by_session_gap(&[&after, &before], DEFAULT_SESSION_GAP);
        assert_eq!(
            night.len(),
            1,
            "20 min across midnight is one sitting: {night:?}"
        );
        assert!(
            night[0].heading.contains("· 1-1"),
            "heading {}",
            night[0].heading
        );

        let split = group_by_session_gap(&[&before, &earlier], DEFAULT_SESSION_GAP);
        assert_eq!(
            split.len(),
            2,
            "110 min idle must split sittings: {split:?}"
        );
    }

    #[test]
    fn games_label_singular() {
        let one = Record {
            games: 1,
            wins: 1,
            losses: 0,
            draws: 0,
        };
        assert_eq!(one.games_label(), "1 game");
        let many = Record {
            games: 7,
            wins: 3,
            losses: 4,
            draws: 0,
        };
        assert_eq!(many.games_label(), "7 games");
    }

    #[test]
    fn season_line_appends_undecided() {
        let decided = Record {
            games: 25,
            wins: 10,
            losses: 15,
            draws: 0,
        };
        assert_eq!(decided.undecided(), 0);
        assert_eq!(decided.season_line(), "25 games · 10-15-0");

        let mixed = Record {
            games: 62,
            wins: 10,
            losses: 15,
            draws: 0,
        };
        assert_eq!(mixed.undecided(), 37);
        assert_eq!(mixed.season_line(), "62 games · 10-15-0 · 37 undecided");
        assert!((mixed.win_rate() - 0.4).abs() < f32::EPSILON);

        let one_unknown = Record {
            games: 2,
            wins: 1,
            losses: 0,
            draws: 0,
        };
        assert_eq!(one_unknown.season_line(), "2 games · 1-0-0 · 1 undecided");
    }

    #[test]
    fn role_filter_union_and_distinct_lists() {
        let rows = vec![
            game("Ana", Role::Support, Outcome::Win, ts(2026, 9, 2)),
            game("Ashe", Role::Damage, Outcome::Loss, ts(2026, 9, 2)),
            game("Junker Queen", Role::Tank, Outcome::Win, ts(2026, 9, 2)),
        ];
        let mut roles = RoleFilter::default();
        roles = roles.toggle(Role::Tank);
        roles = roles.toggle(Role::Support);
        let filtered = filter_games(&rows, &GameFilter::from_header(None, roles));
        assert_eq!(filtered.len(), 2);
        let heroes = distinct_heroes(&filtered);
        assert_eq!(heroes, vec!["Ana".to_string(), "Junker Queen".to_string()]);
        let maps = distinct_maps(&filtered);
        assert_eq!(maps, vec!["King's Row".to_string()]);
    }

    /// The Map dropdown offers known maps by name and one "No map" for
    /// every game the rows show as "No map" (empty, or a raw read like
    /// "Ilio5"), and that choice finds those games.
    #[test]
    fn unknown_map_reads_are_one_no_map_choice() {
        let at = ts(2026, 9, 2);
        let rows = vec![
            game_on("Ilio5", "Ana", Role::Support, Outcome::Win, at),
            game_on("", "Ana", Role::Support, Outcome::Loss, at),
            game_on("Ilios", "Ana", Role::Support, Outcome::Win, at),
            game_on("Busan", "Ana", Role::Support, Outcome::Win, at),
        ];
        let all: Vec<&Game> = rows.iter().collect();
        assert_eq!(distinct_maps(&all), ["Busan", "Ilios", NO_MAP]);
        let pick = |map: &str| {
            filter_games(
                &rows,
                &GameFilter {
                    map: Some(map.into()),
                    ..GameFilter::default()
                },
            )
            .iter()
            .map(|g| g.map_name.clone())
            .collect::<Vec<_>>()
        };
        assert_eq!(pick(NO_MAP), ["Ilio5", ""]);
        assert_eq!(pick("Ilios"), ["Ilios"]);
        let known: Vec<&Game> = rows[2..].iter().collect();
        assert_eq!(distinct_maps(&known), ["Busan", "Ilios"]);
    }

    /// A game with one hero run from `start` to `end` (its Tab reads).
    fn timed(hero: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> Game {
        let mut g = game(hero, Role::Tank, Outcome::Win, end);
        g.segments = vec![crate::games::model::SegmentView {
            index: 0,
            hero: hero.into(),
            role: Role::Tank,
            snapshots: 4,
            confirmed: true,
            dismissed: false,
            resolution: None,
            first_seen: start,
            last_seen: end,
        }];
        g
    }

    /// The Overview's sittings use the Games tab's rule, end of one game
    /// to start of the next: at 30 minutes, a 27 minute game that starts
    /// 5 minutes after the one before ends is the same sitting, though its
    /// last Tab read is 32 minutes after the earlier game's.
    #[test]
    fn sittings_split_on_the_gap_from_end_to_start() {
        let t = |h: u32, m: u32| Utc.with_ymd_and_hms(2026, 9, 2, h, m, 0).unwrap();
        let older = timed("Ana", t(20, 0), t(20, 20));
        let newer = timed("Mei", t(20, 25), t(20, 52));
        let gap = chrono::TimeDelta::minutes(30);
        assert!(same_sitting(&newer, &older, gap));
        assert_eq!(group_by_session_gap(&[&newer, &older], gap).len(), 1);
        let later = timed("Mei", t(20, 51), t(21, 0));
        assert!(
            !same_sitting(&later, &older, gap),
            "31 min from end to start"
        );
        assert_eq!(group_by_session_gap(&[&later, &older], gap).len(), 2);
        // A window of 0 is off: each game is its own sitting.
        assert_eq!(session_window_gap(0), chrono::TimeDelta::MIN);
        assert_eq!(
            group_by_session_gap(&[&newer, &older], session_window_gap(0)).len(),
            2
        );
        assert_eq!(session_window_gap(1800), gap);
    }
}
