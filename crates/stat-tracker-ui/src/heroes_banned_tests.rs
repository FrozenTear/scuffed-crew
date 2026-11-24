//! Tests for the When banned model (`heroes_banned`).

use super::*;
use chrono::TimeZone;

fn base() -> Game {
    crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
        crate::cli::FixtureKind::Sample,
    ))
    .remove(0)
}

/// A game with `json` as its stored result, `outcome`, played on Oct `day`.
fn game(json: &str, outcome: Outcome, day: u32) -> Game {
    let mut g = base();
    g.session_id = format!("g-{day}-{}", json.len());
    g.bans_picks = Some(json.to_string());
    g.outcome = outcome;
    g.played_at = Utc.with_ymd_and_hms(2026, 10, day, 20, 0, 0).unwrap();
    g
}

/// A cv-v1 result with these ban slots: (hero key or null, team or null).
fn bans(slots: &[(Option<&str>, Option<&str>)]) -> String {
    bans_lobby(slots, None)
}

fn bans_lobby(slots: &[(Option<&str>, Option<&str>)], lobby: Option<&str>) -> String {
    let q = |v: Option<&str>| v.map_or("null".to_string(), |s| format!("\"{s}\""));
    let list: Vec<String> = slots
        .iter()
        .map(|(h, t)| format!(r#"{{"value":{},"confidence":0.9,"team":{}}}"#, q(*h), q(*t)))
        .collect();
    let lobby = lobby.map_or("null".to_string(), |h| format!(r#"{{"value":"{h}"}}"#));
    format!(
        r#"{{"layout":"lobby","bans":[{}],"lobby_ban":{lobby},"picks":[],"ban_vote":[],"ban_votes":null,"bans_picks_recognizer":"cv-v1"}}"#,
        list.join(",")
    )
}

fn table(games: &[Game]) -> BanTable {
    let refs: Vec<&Game> = games.iter().collect();
    aggregate(&refs)
}

fn row<'a>(t: &'a BanTable, hero: &str) -> &'a BanRow {
    t.rows
        .iter()
        .find(|r| r.hero == hero)
        .unwrap_or_else(|| panic!("no {hero} row"))
}

fn names(rows: &[BanRow]) -> Vec<&'static str> {
    rows.iter().map(|r| r.hero).collect()
}

/// `n` games where `hero` was banned by the enemy, `wins` of them won and
/// the rest lost, played from Oct `day`.
fn games_of(hero: &str, n: usize, wins: usize, day: u32) -> Vec<Game> {
    (0..n)
        .map(|i| {
            let outcome = if i < wins {
                Outcome::Win
            } else {
                Outcome::Loss
            };
            let mut g = game(&bans(&[(Some(hero), Some("enemy"))]), outcome, day);
            g.session_id = format!("{hero}-{i}");
            g
        })
        .collect()
}

#[test]
fn own_and_enemy_bans_split_and_both_feed_the_record() {
    let games = [
        game(
            &bans(&[(Some("kiriko"), Some("own")), (Some("ana"), Some("enemy"))]),
            Outcome::Win,
            1,
        ),
        game(&bans(&[(Some("kiriko"), Some("enemy"))]), Outcome::Loss, 2),
    ];
    let t = table(&games);
    let k = row(&t, "Kiriko");
    assert_eq!((k.own, k.enemy, k.side_not_read), (1, 1, 0));
    assert_eq!((k.record.games, k.record.wins, k.record.losses), (2, 1, 1));
    let a = row(&t, "Ana");
    assert_eq!((a.own, a.enemy), (0, 1));
    assert_eq!(a.record.wins, 1, "an enemy ban still counts");
    assert_eq!(a.role, Role::Support, "the banned hero's role");
    assert_eq!((t.with_ban, t.no_ban), (2, 0));
}

#[test]
fn a_hero_banned_by_both_sides_counts_once_in_games_and_once_per_side() {
    let g = game(
        &bans(&[
            (Some("orisa"), Some("own")),
            (Some("orisa"), Some("enemy")),
            (Some("orisa"), Some("own")),
        ]),
        Outcome::Win,
        1,
    );
    let t = table(&[g]);
    let o = row(&t, "Orisa");
    assert_eq!(o.record.games, 1);
    assert_eq!((o.own, o.enemy, o.side_not_read), (1, 1, 0));
    assert_eq!(t.rows.len(), 1);
}

#[test]
fn a_ban_with_no_team_counts_in_games_but_not_in_the_split() {
    let games = [
        game(&bans(&[(Some("zarya"), None)]), Outcome::Win, 1),
        game(
            &bans(&[(Some("zarya"), None), (Some("zarya"), Some("own"))]),
            Outcome::Loss,
            2,
        ),
    ];
    let t = table(&games);
    let z = row(&t, "Zarya");
    assert_eq!(z.record.games, 2);
    assert_eq!((z.record.wins, z.record.losses), (1, 1));
    assert_eq!((z.own, z.enemy), (1, 0));
    assert_eq!(z.side_not_read, 1, "a read side already places the second");
}

#[test]
fn unread_and_unknown_slots_never_get_a_row() {
    let g = game(
        &bans(&[
            (None, Some("own")),
            (Some("not-a-hero"), Some("enemy")),
            (Some("placeholder"), Some("enemy")),
            (Some("no_preference"), Some("own")),
        ]),
        Outcome::Win,
        1,
    );
    let t = table(&[g]);
    assert!(t.rows.is_empty(), "{:?}", names(&t.rows));
    assert_eq!((t.with_ban, t.no_ban), (0, 1), "left out as no ban read");
    let mixed = game(
        &bans(&[(None, Some("own")), (Some("mercy"), Some("own"))]),
        Outcome::Win,
        2,
    );
    let t = table(&[mixed]);
    assert_eq!(names(&t.rows), ["Mercy"]);
    assert_eq!(t.with_ban, 1);
}

#[test]
fn games_with_no_ban_read_are_counted_as_left_out() {
    let mut none = base();
    none.bans_picks = None;
    let games = [
        none,
        game(&bans(&[]), Outcome::Win, 1),
        game(&bans(&[(None, None), (None, None)]), Outcome::Win, 2),
        game(&bans(&[(Some("sojourn"), Some("enemy"))]), Outcome::Loss, 3),
    ];
    let t = table(&games);
    assert_eq!((t.with_ban, t.no_ban), (1, 3));
    assert_eq!(names(&t.rows), ["Sojourn"]);
}

#[test]
fn a_distinct_lobby_ban_counts_with_no_side() {
    let g = game(
        &bans_lobby(&[(Some("ana"), Some("own"))], Some("sigma")),
        Outcome::Win,
        1,
    );
    let t = table(&[g]);
    let s = row(&t, "Sigma");
    assert_eq!(s.record.games, 1);
    assert_eq!((s.own, s.enemy, s.side_not_read), (0, 0, 0), "no tag");
    assert_eq!(s.lobby, 1, "counted through the lobby ban only");
    assert_eq!((row(&t, "Ana").own, row(&t, "Ana").lobby), (1, 0));

    let alone = game(
        &bans_lobby(&[(None, None)], Some("sigma")),
        Outcome::Loss,
        2,
    );
    let t = table(&[alone]);
    assert_eq!(names(&t.rows), ["Sigma"], "a lobby ban alone is a ban read");
    assert_eq!(t.with_ban, 1);
}

#[test]
fn a_lobby_ban_already_in_a_slot_is_not_counted_twice() {
    let g = game(
        &bans_lobby(&[(Some("sigma"), Some("enemy"))], Some("sigma")),
        Outcome::Win,
        1,
    );
    let t = table(&[g]);
    let s = row(&t, "Sigma");
    assert_eq!(s.record.games, 1);
    assert_eq!((s.own, s.enemy, s.lobby), (0, 1, 0), "the slot places it");
    let unknown = game(&bans_lobby(&[], Some("mystery-hero")), Outcome::Win, 2);
    assert!(table(&[unknown]).rows.is_empty(), "unknown lobby hero");
}

#[test]
fn ban_votes_never_count() {
    let json = r#"{"layout":"plain","bans":[],"lobby_ban":null,"picks":["ana"],
        "ban_vote":["kiriko","mercy","lucio"],
        "ban_votes":[{"voter":0,"slot":1,"state":"hero","hero":"kiriko"},
                     {"voter":1,"slot":1,"state":"hero","hero":"mercy"}],
        "bans_picks_recognizer":"cv-v1"}"#;
    let t = table(&[game(json, Outcome::Win, 1)]);
    assert!(t.rows.is_empty(), "{:?}", names(&t.rows));
    assert_eq!(t.no_ban, 1);
}

#[test]
fn a_rate_needs_ten_decided_games_for_that_hero() {
    let mut games = games_of("ana", 9, 5, 1);
    games.extend(games_of("mercy", 10, 6, 2));
    let t = table(&games);
    let ana = row(&t, "Ana");
    assert_eq!(ana.rate(), None);
    assert_eq!(ana.too_few().as_deref(), Some("Too few games (9 of 10)"));
    let mercy = row(&t, "Mercy");
    assert_eq!(mercy.rate(), Some(60));
    assert_eq!(mercy.too_few(), None);
}

#[test]
fn draws_are_in_the_record_and_the_denominator() {
    let mut games = games_of("ana", 9, 5, 1);
    games.push(game(&bans(&[(Some("ana"), Some("own"))]), Outcome::Draw, 2));
    let t = table(&games);
    let ana = row(&t, "Ana");
    assert_eq!(ana.record.wl_label(), "5-4-1");
    assert_eq!(ana.decided(), 10);
    assert_eq!(ana.rate(), Some(50), "5 of 10, the draw is not a win");
}

#[test]
fn undecided_games_count_in_games_only() {
    let mut games = games_of("ana", 9, 5, 1);
    games.push(game(
        &bans(&[(Some("ana"), Some("own"))]),
        Outcome::Unknown,
        2,
    ));
    let t = table(&games);
    let ana = row(&t, "Ana");
    assert_eq!(ana.record.games, 10);
    assert_eq!(ana.decided(), 9);
    assert_eq!(ana.rate(), None, "an undecided game is not the tenth");
    assert_eq!(
        ana.last,
        Utc.with_ymd_and_hms(2026, 10, 2, 20, 0, 0).unwrap()
    );
}

#[test]
fn win_rate_sort_puts_heroes_under_ten_last_in_both_directions() {
    let mut games = games_of("cassidy", 12, 7, 1);
    games.extend(games_of("kiriko", 20, 8, 2));
    games.extend(games_of("lucio", 7, 3, 3));
    games.extend(games_of("venture", 4, 2, 4));
    let mut rows = table(&games).rows;
    sort_rows(&mut rows, SortKey::WinRate, false);
    assert_eq!(names(&rows), ["Cassidy", "Kiriko", "Lúcio", "Venture"]);
    sort_rows(&mut rows, SortKey::WinRate, true);
    assert_eq!(
        names(&rows),
        ["Kiriko", "Cassidy", "Lúcio", "Venture"],
        "ascending does not pull the unrated rows up"
    );
}

#[test]
fn ties_go_to_more_decided_then_the_name() {
    let mut games = games_of("zarya", 5, 2, 1);
    games.extend(games_of("ana", 5, 2, 2));
    let mut undecided = games_of("mei", 5, 2, 3);
    undecided[0].outcome = Outcome::Unknown;
    games.extend(undecided);
    let mut rows = table(&games).rows;
    for ascending in [false, true] {
        sort_rows(&mut rows, SortKey::Games, ascending);
        assert_eq!(names(&rows), ["Ana", "Zarya", "Mei"], "{ascending}");
    }
    sort_rows(&mut rows, SortKey::Recent, false);
    assert_eq!(names(&rows), ["Mei", "Ana", "Zarya"]);
    sort_rows(&mut rows, SortKey::Hero, true);
    assert_eq!(names(&rows), ["Ana", "Mei", "Zarya"]);
}

#[test]
fn the_default_sort_is_games_high_to_low_and_headers_flip_it() {
    let mut ui = BanUi::default();
    assert_eq!((ui.sort, ui.ascending), (SortKey::Games, false));
    update(&mut ui, Msg::Header(SortKey::Games));
    assert!(ui.ascending);
    update(&mut ui, Msg::Header(SortKey::Hero));
    assert_eq!((ui.sort, ui.ascending), (SortKey::Hero, true));
    update(&mut ui, Msg::Sort(SortKey::WinRate));
    assert_eq!((ui.sort, ui.ascending), (SortKey::WinRate, false));
    let mut games = games_of("ana", 3, 1, 1);
    games.extend(games_of("orisa", 5, 1, 2));
    let mut rows = table(&games).rows;
    sort_rows(&mut rows, SortKey::default(), false);
    assert_eq!(names(&rows), ["Orisa", "Ana"]);
}

#[test]
fn bad_stored_results_never_panic_and_never_hide_the_game_elsewhere() {
    let bad = [
        "",
        "not json",
        "[]",
        "42",
        "null",
        r#""a string""#,
        r#"{"bans":"kiriko"}"#,
        r#"{"bans":[42,true,null,{"value":7,"team":3}]}"#,
        r#"{"bans":[{"value":["ana"],"team":{"x":1}}],"lobby_ban":12}"#,
        r#"{"bans":[{"value":"ana","team":"own"}],"bans_picks_recognizer":"cv-v9"}"#,
        r#"{"bans":[{"value":"ana","team":"own"}],"bans_picks_recognizer":5}"#,
        r#"{"bans":[{"value":"ana","team":"own"}"#,
    ];
    let (_dir, mut app) = crate::fixtures::test_app();
    let mut games: Vec<Game> = bad
        .iter()
        .enumerate()
        .map(|(i, json)| {
            let mut g = game(json, Outcome::Win, 1);
            g.session_id = format!("bad-{i}");
            g
        })
        .collect();
    games.push(game(&bans(&[(Some("ana"), Some("own"))]), Outcome::Win, 2));
    app.games = games;
    let shown = crate::heroes_page::shown_games(&app);
    assert_eq!(shown.len(), bad.len() + 1, "Played keeps every game");
    let t = aggregate(&shown);
    assert_eq!(names(&t.rows), ["Ana"]);
    assert_eq!((t.with_ban, t.no_ban), (1, bad.len()));
    assert_eq!(
        crate::maps_tab::maps_games(&app).len(),
        bad.len() + 1,
        "Maps keeps every game"
    );
    let played = crate::aggregate::aggregate_games(&shown);
    assert_eq!(played.record.games, bad.len() + 1, "Overview totals too");
    let tagged = bans(&[(Some("ana"), Some("own"))]).replace("cv-v1", "cv-v2");
    assert!(game_bans(&game(&tagged, Outcome::Win, 1)).is_empty());
    let untagged = r#"{"bans":[{"value":"ana","team":"own"}]}"#;
    assert_eq!(game_bans(&game(untagged, Outcome::Win, 1)).len(), 1);
}

#[test]
fn the_heroes_filters_apply_and_the_open_game_is_left_out() {
    let (_dir, mut app) = crate::fixtures::test_app();
    app.seasons.seasons =
        serde_json::from_str(crate::fixtures_full::seasons_json()).expect("seasons");
    let mut a = game(&bans(&[(Some("ana"), Some("own"))]), Outcome::Win, 20);
    a.role = Role::Tank;
    a.map_name = "Ilios".into();
    let mut b = game(&bans(&[(Some("mercy"), Some("own"))]), Outcome::Win, 1);
    b.role = Role::Support;
    b.map_name = "Busan".into();
    b.played_at = Utc.with_ymd_and_hms(2026, 9, 1, 20, 0, 0).unwrap();
    app.games = vec![a, b];
    let heroes = |app: &crate::app::TrackerApp| {
        let shown = crate::heroes_page::shown_games(app);
        names(&aggregate(&shown).rows)
    };
    assert_eq!(heroes(&app).len(), 2);
    app.season = crate::model::SeasonSel::Season("season-5".into());
    assert_eq!(heroes(&app), ["Ana"], "season");
    app.season = crate::model::SeasonSel::AllTime;
    app.roles = crate::model::RoleFilter::default().toggle(Role::Support);
    assert_eq!(
        heroes(&app),
        ["Mercy"],
        "the role played, not the banned one"
    );
    app.roles = crate::model::RoleFilter::default();
    app.filter_map = Some("Ilios".into());
    assert_eq!(heroes(&app), ["Ana"], "map");
    app.filter_map = None;
    app.games[0].outcome = Outcome::Unknown;
    app.live_game_session = Some(app.games[0].session_id.clone());
    assert_eq!(heroes(&app), ["Mercy"], "the open game waits");
}

#[test]
fn the_stats_line_names_the_left_out_games() {
    let mut t = BanTable {
        rows: table(&games_of("ana", 1, 1, 1)).rows,
        with_ban: 212,
        no_ban: 48,
    };
    assert_eq!(
        stats_line("All time", &t),
        "All time · 1 hero · 212 games with a ban read · 48 games with no ban read left out"
    );
    t.with_ban = 1;
    t.no_ban = 0;
    assert_eq!(
        stats_line("Season 5", &t),
        "Season 5 · 1 hero · 1 game with a ban read"
    );
}

#[test]
fn the_empty_cards_say_why() {
    let empty = BanTable::default();
    assert_eq!(
        empty_copy(false, 0, &empty, true),
        Some((
            "No bans yet",
            "They show here once the tracker saves a game with a ban read.".into()
        ))
    );
    assert_eq!(
        empty_copy(true, 0, &empty, true),
        Some((
            "No games match these filters",
            "Clear a chip above to see when a hero was banned.".into()
        ))
    );
    let unread = BanTable {
        no_ban: 857,
        ..BanTable::default()
    };
    assert_eq!(
        empty_copy(true, 857, &unread, true),
        Some((
            "No bans read yet",
            "857 games in this view have no ban read. They are left out of this table.".into()
        ))
    );
    assert_eq!(
        empty_copy(true, 857, &unread, false).unwrap().1,
        "857 games in this view have no ban read. They are left out of this table. The bans and picks reader is off in Settings, Capture and reading."
    );
    let rows = table(&games_of("ana", 1, 1, 1));
    assert_eq!(empty_copy(true, 1, &rows, false), None);
}

#[test]
fn the_copy_never_reads_as_advice() {
    let source = include_str!("heroes_banned.rs").to_lowercase()
        + &include_str!("heroes_banned_view.rs").to_lowercase();
    for banned in [
        "should ban",
        "best ban",
        "good ban",
        "bad ban",
        "overwatch 2",
        "\u{2014}",
        "\u{2013}",
    ] {
        assert!(!source.contains(banned), "{banned:?}");
    }
    assert!(CAPTION.contains("not a suggestion of who to ban"));
}

#[test]
fn the_lobby_count_is_games_named_only_by_a_lobby_ban() {
    let games = [
        game(&bans_lobby(&[(None, None)], Some("sigma")), Outcome::Win, 1),
        game(&bans_lobby(&[], Some("sigma")), Outcome::Loss, 2),
        game(
            &bans_lobby(&[(Some("sigma"), Some("own"))], Some("sigma")),
            Outcome::Win,
            3,
        ),
        game(
            &bans_lobby(&[(Some("sigma"), None)], Some("sigma")),
            Outcome::Win,
            4,
        ),
        game(&bans(&[(Some("ana"), Some("enemy"))]), Outcome::Win, 5),
    ];
    let t = table(&games);
    let s = row(&t, "Sigma");
    assert_eq!(s.record.games, 4);
    assert_eq!(
        (s.own, s.enemy, s.side_not_read, s.lobby),
        (1, 0, 1, 2),
        "a lobby ban that is also a slot adds no Lobby count"
    );
    assert_eq!(row(&t, "Ana").lobby, 0);
}

/// `hero` banned by the enemy in one lost game on each Oct day, in the
/// order given.
fn on_days(hero: &str, days: &[u32]) -> Vec<Game> {
    days.iter()
        .map(|&day| {
            let mut g = game(&bans(&[(Some(hero), Some("enemy"))]), Outcome::Loss, day);
            g.session_id = format!("{hero}-{day}");
            g
        })
        .collect()
}

fn oct(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, day, 20, 0, 0).unwrap()
}

#[test]
fn recent_is_the_newest_game_when_the_list_is_newest_first() {
    // app.games is newest first, so the last game seen is the oldest.
    let mut games = on_days("ana", &[9, 5, 1]);
    games.extend(on_days("mercy", &[8, 7, 6]));
    let t = table(&games);
    assert_eq!(row(&t, "Ana").last, oct(9));
    assert_eq!(row(&t, "Mercy").last, oct(8));
    let mut rows = t.rows;
    sort_rows(&mut rows, SortKey::Recent, false);
    assert_eq!(names(&rows), ["Ana", "Mercy"]);
    sort_rows(&mut rows, SortKey::Recent, true);
    assert_eq!(names(&rows), ["Mercy", "Ana"]);
}

#[test]
fn recent_is_the_newest_game_in_any_order() {
    // Mixed dates, the two heroes interleaved: ana 3, mercy 6, ana 7,
    // mercy 2, ana 1, mercy 4.
    let ana = on_days("ana", &[3, 7, 1]);
    let mercy = on_days("mercy", &[6, 2, 4]);
    let games: Vec<Game> = ana
        .into_iter()
        .zip(mercy)
        .flat_map(|(a, m)| [a, m])
        .collect();
    let t = table(&games);
    assert_eq!(row(&t, "Ana").last, oct(7));
    assert_eq!(row(&t, "Mercy").last, oct(6));
    let mut rows = t.rows;
    sort_rows(&mut rows, SortKey::Recent, false);
    assert_eq!(names(&rows), ["Ana", "Mercy"]);
}
