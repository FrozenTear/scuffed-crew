//! Tests for the bans and picks view model (`games_bans`).

use super::*;
use crate::fixtures_games::{
    HELD, LOBBY_ONLY, NO_BAN, NORMAL, REJECTED_2, SIDE, SKIP, UNDECIDED, UNSURE,
};
use crate::snapshot::games_from_snapshot;

fn game(sid: &str) -> Game {
    games_from_snapshot(&crate::fixtures_games::snapshot())
        .into_iter()
        .find(|g| g.session_id == sid)
        .expect(sid)
}

fn with(json: &str, status: Option<BansPicksStatus>) -> Game {
    let mut g = game(NORMAL);
    g.bans_picks = Some(json.to_string());
    g.bans_picks_status = status;
    g
}

fn block(b: BansPicks) -> Block {
    match b {
        BansPicks::Block(block) => block,
        other => panic!("not a block: {other:?}"),
    }
}

fn block_of(g: &Game) -> Block {
    block(bans_picks(g))
}

fn rows_of(g: &Game) -> Vec<Row> {
    block_of(g).rows
}

fn row<'a>(rows: &'a [Row], label: &str) -> &'a Row {
    rows.iter()
        .find(|r| r.label == label)
        .unwrap_or_else(|| panic!("no {label} row"))
}

fn names(row: &Row) -> Vec<&str> {
    row.chips().map(Chip::name).collect()
}

fn chips(rows: &[Row], label: &str) -> Vec<Chip> {
    row(rows, label).chips().cloned().collect()
}

/// The row's parts as words: group keys and notes as they read, chips
/// in brackets, dividers as a bar.
fn shape(row: &Row) -> Vec<String> {
    row.parts
        .iter()
        .map(|p| match p {
            Part::Key(k) => (*k).to_string(),
            Part::Chips(c) => format!(
                "[{}]",
                c.iter().map(Chip::name).collect::<Vec<_>>().join(", ")
            ),
            Part::Note { text, .. } | Part::Quiet { text, .. } => (*text).to_string(),
            Part::Divider => "|".to_string(),
        })
        .collect()
}

/// The row's quiet line and its tooltip, when it has one.
fn quiet(row: &Row) -> Option<(&'static str, Option<&'static str>)> {
    row.parts.iter().find_map(|p| match p {
        Part::Quiet { text, tip } => Some((*text, *tip)),
        _ => None,
    })
}

fn labels(rows: &[Row]) -> Vec<&'static str> {
    rows.iter().map(|r| r.label).collect()
}

const FOUR: [&str; 4] = [LABEL_BANS, LABEL_VOTES, LABEL_HERO_SELECT, LABEL_PLAYED];

#[test]
fn a_full_read_has_four_rows_and_the_bans_on_one() {
    let b = block_of(&game(NORMAL));
    assert_eq!(labels(&b.rows), FOUR);
    assert_eq!(
        shape(row(&b.rows, LABEL_BANS)),
        [
            "Your team",
            "[Sombra, Mauga]",
            "|",
            "Their team",
            "[Ana, Winston]",
            "|",
            "Lobby",
            "[Sigma]"
        ]
    );
    assert_eq!(
        names(row(&b.rows, LABEL_VOTES)),
        ["Reinhardt", "Tracer", "Genji"]
    );
    // Hero select is always all six, ours first with the You tag.
    let picks = chips(&b.rows, LABEL_HERO_SELECT);
    assert_eq!(
        picks.iter().map(Chip::name).collect::<Vec<_>>(),
        [
            "Wrecking Ball",
            "Tracer",
            "Cassidy",
            "Kiriko",
            "Lúcio",
            "Mercy"
        ]
    );
    assert!(picks[0].you_tag && picks[1..].iter().all(|c| !c.you_tag));
    assert_eq!(names(row(&b.rows, LABEL_PLAYED)), ["Wrecking Ball"]);
    assert_eq!(
        b.legend(),
        Legend {
            banned: true,
            ..Legend::default()
        }
    );
}

#[test]
fn tooltips_say_what_the_chip_is() {
    let rows = rows_of(&game(NORMAL));
    let tip = |label: &str, c: usize| chips(&rows, label)[c].tip.clone();
    assert_eq!(tip(LABEL_BANS, 0), "Banned by your team: Sombra");
    assert_eq!(tip(LABEL_BANS, 1), "Banned by your team: Mauga");
    assert_eq!(tip(LABEL_BANS, 2), "Banned by their team: Ana");
    // The group says Lobby, the chip still says Lobby ban.
    assert_eq!(tip(LABEL_BANS, 4), "Lobby ban: Sigma");
    assert_eq!(tip(LABEL_VOTES, 0), "You voted to ban Reinhardt");
    assert_eq!(tip(LABEL_VOTES, 1), "You voted to ban Tracer");
    assert_eq!(
        tip(LABEL_HERO_SELECT, 0),
        "Shown at hero select: Wrecking Ball, you"
    );
    assert_eq!(tip(LABEL_HERO_SELECT, 1), "Shown at hero select: Tracer");
    assert_eq!(tip(LABEL_PLAYED, 0), "Played Wrecking Ball, you");
}

#[test]
fn a_partial_read_keeps_its_gaps_visible() {
    let b = block_of(&game(UNSURE));
    assert_eq!(labels(&b.rows), FOUR);
    assert_eq!(
        b.legend(),
        Legend {
            banned: true,
            swap: true,
            outline: true,
        }
    );
    let rows = b.rows;
    // Bans with no side: every chip, then Team not read, and no lobby.
    let bans = row(&rows, LABEL_BANS);
    assert_eq!(
        shape(bans),
        ["[Sombra, Mauga, Not read, Winston]", TEAM_NOT_READ]
    );
    assert!(bans.parts.contains(&Part::Note {
        text: TEAM_NOT_READ,
        tip: "The tracker couldn't tell which team made these bans.",
    }));
    let bans = chips(&rows, LABEL_BANS);
    assert_eq!(bans[0].tip, "Banned: Sombra, team not read");
    assert_eq!(bans[2].tip, "Ban 3: not read");
    assert!(bans[2].outline());
    // No preference shows as such, and a null slot as Not read.
    let picks = chips(&rows, LABEL_HERO_SELECT);
    assert_eq!(picks.len(), 6);
    assert_eq!(picks[2].slot, Slot::NoPreference);
    assert_eq!(picks[2].name(), "No preference");
    assert!(!picks[2].outline());
    assert_eq!(picks[3].slot, Slot::NotRead);
    assert_eq!(picks[3].tip, "Hero select slot 4: not read");
    let votes = chips(&rows, LABEL_VOTES);
    assert_eq!(votes[1].tip, "Your ban vote 2: not read");
    assert!(votes.iter().all(|c| !c.you_tag));
    // We picked Reinhardt and played Wrecking Ball.
    let played = chips(&rows, LABEL_PLAYED);
    assert!(played[0].swap && played[0].you_tag);
    assert_eq!(
        played[0].tip,
        "Played Wrecking Ball, Reinhardt at hero select, you"
    );
}

#[test]
fn a_ban_no_preference_is_not_read() {
    let g = with(
        r#"{"bans":[{"value":"no_preference","team":"own"},{"value":"ana","team":"enemy"}]}"#,
        Some(BansPicksStatus::Ok),
    );
    let rows = rows_of(&g);
    assert_eq!(
        shape(row(&rows, LABEL_BANS)),
        ["Your team", "[Not read]", "|", "Their team", "[Ana]"]
    );
}

#[test]
fn lists_are_capped_and_plain_strings_read() {
    let g = with(
        r#"{"bans":["ana","reaper","mauga","sigma","zarya"],
            "picks":["ana","reaper","mauga","sigma","zarya","mercy","lucio"],
            "ban_vote":["no-preference"]}"#,
        Some(BansPicksStatus::Ok),
    );
    let rows = rows_of(&g);
    // No side on any ban: one group, four chips.
    assert_eq!(names(row(&rows, LABEL_BANS)).len(), 4);
    assert_eq!(names(row(&rows, LABEL_HERO_SELECT)).len(), 6);
    // The flat `ban_vote` list is legacy and is not shown as our votes.
    assert_eq!(quiet(row(&rows, LABEL_VOTES)), Some((LEGACY_VOTES, None)));
}

/// Games stored before the screen status existed keep the shipped
/// "weren't read" wording, one quiet line per row.
#[test]
fn an_older_game_keeps_the_weren_t_read_wording() {
    let g = with(r#"{"bans":[],"lobby_ban":{"value":"sigma"}}"#, None);
    let rows = rows_of(&g);
    assert_eq!(labels(&rows), FOUR);
    // Only the lobby ban was read and no ban slot is stored: the line says
    // the bans were not read, then the Lobby group.
    assert_eq!(
        shape(row(&rows, LABEL_BANS)),
        ["Bans weren't read for this game.", "|", "Lobby", "[Sigma]"]
    );
    assert_eq!(
        quiet(row(&rows, LABEL_VOTES)),
        Some(("Your ban votes weren't read for this game.", None))
    );
    assert_eq!(
        quiet(row(&rows, LABEL_HERO_SELECT)),
        Some(("Hero select wasn't read for this game.", None))
    );
    let g = with(
        r#"{"bans":[{"value":null,"team":"own"}],"picks":[{"value":"ana"}]}"#,
        None,
    );
    let rows = rows_of(&g);
    assert_eq!(
        quiet(row(&rows, LABEL_BANS)),
        Some(("Bans weren't read for this game.", None))
    );
    assert_eq!(quiet(row(&rows, LABEL_VOTES)), Some((LEGACY_VOTES, None)));
    let g = with(
        r#"{"bans":[{"value":"ana","team":"own"}],
            "ban_votes":[{"voter":0,"slot":1,"state":"hero","hero":"ana"}]}"#,
        None,
    );
    let rows = rows_of(&g);
    assert_eq!(
        quiet(row(&rows, LABEL_HERO_SELECT)),
        Some((LEGACY_HERO_SELECT, None))
    );
}

/// Lijiang Tower: the status says the ban screen and the ban vote were never
/// seen. Not "weren't read", and no amber.
#[test]
fn no_ban_screen_seen_comes_from_the_status() {
    let g = with(
        r#"{"picks":[{"value":"reinhardt"},{"value":"tracer"}]}"#,
        Some(BansPicksStatus::Partial {
            missing: vec![BansPicksMissing::BanResults, BansPicksMissing::BanVote],
        }),
    );
    let b = block_of(&g);
    assert_eq!(labels(&b.rows), FOUR);
    assert_eq!(
        quiet(row(&b.rows, LABEL_BANS)),
        Some((
            "No ban screen seen",
            Some(
                "The tracker didn't see a ban screen for this game. Either the game had none, or the tracker missed it."
            )
        ))
    );
    assert_eq!(
        quiet(row(&b.rows, LABEL_VOTES)),
        Some((
            "Screen not seen",
            Some("The tracker didn't see the ban vote for this game.")
        ))
    );
    assert_eq!(
        names(row(&b.rows, LABEL_HERO_SELECT)),
        ["Reinhardt", "Tracer"]
    );
    // The fixture game: same words, and a card with no glyphs has no
    // legend.
    let b = block_of(&game(NO_BAN));
    assert_eq!(quiet(row(&b.rows, LABEL_BANS)).unwrap().0, NO_BAN_SCREEN);
    assert_eq!(quiet(row(&b.rows, LABEL_VOTES)).unwrap().0, SCREEN_NOT_SEEN);
    assert_eq!(names(row(&b.rows, LABEL_HERO_SELECT)).len(), 6);
    assert!(b.legend().is_empty(), "{:?}", b.legend());
}

#[test]
fn a_seen_ban_screen_with_no_hero_read_says_so() {
    let g = with(
        r#"{"bans":[{"value":null,"team":"own"},{"value":null,"team":"enemy"}],
            "picks":[{"value":"reinhardt"}]}"#,
        Some(BansPicksStatus::Ok),
    );
    let rows = rows_of(&g);
    assert_eq!(
        quiet(row(&rows, LABEL_BANS)),
        Some((
            "Ban screen seen, heroes not read",
            Some("The ban screen was seen, but its heroes couldn't be read.")
        ))
    );
    // A screen that was seen and not read keeps the "weren't read" line.
    assert_eq!(quiet(row(&rows, LABEL_VOTES)), Some((LEGACY_VOTES, None)));
    // The same under a Partial status that lists other screens.
    let g = with(
        r#"{"bans":[{"value":null,"team":"own"}],
            "ban_votes":[{"voter":0,"slot":1,"state":"hero","hero":"ana"}]}"#,
        Some(BansPicksStatus::Partial {
            missing: vec![BansPicksMissing::HeroSelect],
        }),
    );
    let rows = rows_of(&g);
    assert_eq!(quiet(row(&rows, LABEL_BANS)).unwrap().0, BANS_NOT_READ);
    assert_eq!(
        quiet(row(&rows, LABEL_HERO_SELECT)).unwrap().0,
        SCREEN_NOT_SEEN
    );
    // Hero select seen (Ok) but not read keeps the older line, not "Screen
    // not seen".
    let g = with(
        r#"{"bans":[{"value":"ana","team":"own"}]}"#,
        Some(BansPicksStatus::Ok),
    );
    assert_eq!(
        quiet(row(&rows_of(&g), LABEL_HERO_SELECT)),
        Some((LEGACY_HERO_SELECT, None))
    );
}

/// A side with no ban has no group: no empty "Their team" key.
#[test]
fn a_team_with_no_ban_has_no_group() {
    let g = with(
        r#"{"bans":[{"value":"ana","team":"own"},{"value":"mauga","team":"own"}],
            "lobby_ban":{"value":"sigma"}}"#,
        Some(BansPicksStatus::Ok),
    );
    assert_eq!(
        shape(row(&rows_of(&g), LABEL_BANS)),
        ["Your team", "[Ana, Mauga]", "|", "Lobby", "[Sigma]"]
    );
}

/// The Played row is never hidden: a game with no hero says Not read.
#[test]
fn played_with_no_hero_is_a_quiet_line() {
    let mut g = game(NORMAL);
    g.segments.clear();
    g.hero = "Unknown".into();
    let rows = rows_of(&g);
    assert_eq!(labels(&rows), FOUR);
    assert_eq!(quiet(row(&rows, LABEL_PLAYED)), Some(("Not read", None)));
}

/// Nepal: the bans were read but not which team made each, and hero
/// select was never seen.
#[test]
fn team_not_read_and_hero_select_not_seen() {
    let b = block_of(&game(SIDE));
    assert_eq!(labels(&b.rows), FOUR);
    assert_eq!(
        shape(row(&b.rows, LABEL_BANS)),
        ["[Sombra, Mauga, Ana, Winston]", TEAM_NOT_READ]
    );
    assert_eq!(
        quiet(row(&b.rows, LABEL_HERO_SELECT)),
        Some((
            "Screen not seen",
            Some("The tracker didn't read hero select for this game.")
        ))
    );
    assert_eq!(
        names(row(&b.rows, LABEL_VOTES)),
        ["Reinhardt", "Tracer", "Skipped"]
    );
    // No hero select pick, so no swap; the Played chip is plain.
    let played = chips(&b.rows, LABEL_PLAYED);
    assert_eq!(played.len(), 1);
    assert!(played[0].you_tag && !played[0].swap);
    assert_eq!(
        b.legend(),
        Legend {
            banned: true,
            ..Legend::default()
        }
    );
}

#[test]
fn skipped_is_a_plain_chip_and_not_seen_an_outline() {
    let b = block_of(&game(SKIP));
    let votes = chips(&b.rows, LABEL_VOTES);
    assert_eq!(
        votes.iter().map(Chip::name).collect::<Vec<_>>(),
        ["Reinhardt", "Skipped", "Not seen"]
    );
    assert!(!votes[1].outline() && !votes[1].ban && !votes[1].swap);
    assert!(votes[2].outline());
    assert_eq!(votes[1].tip, "You skipped ban vote 2");
    assert_eq!(
        votes[2].tip,
        "Your ban vote 3: not seen, it may have come after the last frame"
    );
    assert_eq!(
        b.legend(),
        Legend {
            banned: true,
            outline: true,
            ..Legend::default()
        }
    );
    // Skips alone draw no outline swatch in the legend.
    let g = with(
        r#"{"bans":[{"value":"ana","team":"own"}],
            "ban_votes":[{"voter":0,"slot":1,"state":"skipped","hero":null},
                         {"voter":0,"slot":2,"state":"skipped","hero":null}]}"#,
        Some(BansPicksStatus::Ok),
    );
    assert!(!block_of(&g).legend().outline);
}

fn vote_row(g: &Game) -> Vec<Chip> {
    let rows = rows_of(g);
    assert!(
        matches!(row(&rows, LABEL_VOTES).parts.as_slice(), [Part::Chips(_)]),
        "vote chips"
    );
    chips(&rows, LABEL_VOTES)
}

#[test]
fn ban_votes_show_skipped_and_empty_slots() {
    // Three skips, like the Eichenwalde game: not three Not read chips.
    let g = with(
        r#"{"bans":[{"value":"ana","team":"own"}],
            "ban_vote":[{"value":null},{"value":null},{"value":null}],
            "ban_votes":[
              {"voter":0,"slot":3,"state":"empty","hero":null},
              {"voter":0,"slot":1,"state":"skipped","hero":null},
              {"voter":0,"slot":2,"state":"hero","hero":"ana"},
              {"voter":1,"slot":1,"state":"hero","hero":"mei"}]}"#,
        Some(BansPicksStatus::Ok),
    );
    let votes = vote_row(&g);
    let slots: Vec<_> = votes.iter().map(|c| c.slot.clone()).collect();
    assert_eq!(
        slots,
        [Slot::Skipped, Slot::Hero("Ana".into()), Slot::Empty]
    );
    assert_eq!(votes[0].name(), "Skipped");
    assert_eq!(votes[0].tip, "You skipped ban vote 1");
    assert!(
        votes[2].tip.starts_with("Your ban vote 3: not seen"),
        "{}",
        votes[2].tip
    );
    assert!(votes.iter().all(|c| !c.you_tag));
}

#[test]
fn an_unclear_slot_is_not_read() {
    // A hero state with no confident hero, or no state, is Not read.
    let g = with(
        r#"{"bans":[{"value":"ana","team":"own"}],
            "ban_votes":[{"voter":0,"slot":1,"state":"hero","hero":null},
                         {"voter":0,"slot":2,"state":null,"hero":null},
                         {"voter":0,"slot":3,"state":"skipped","hero":null}]}"#,
        Some(BansPicksStatus::Ok),
    );
    let got: Vec<_> = vote_row(&g).iter().map(|c| c.slot.clone()).collect();
    assert_eq!(got, [Slot::NotRead, Slot::NotRead, Slot::Skipped]);
}

/// Votes are never taken from the flat `ban_vote` list, which holds the
/// six roster preferred heroes in games stored by alpha.2.1 or older.
fn no_vote_row(g: &Game) {
    let rows = rows_of(g);
    assert_eq!(
        quiet(row(&rows, LABEL_VOTES)),
        Some(("Your ban votes weren't read for this game.", None)),
        "the votes show as not read, with no chips"
    );
}

#[test]
fn legacy_flat_ban_vote_without_ban_votes_shows_no_votes() {
    // What alpha.2.1 stored: the six roster rows, no `ban_votes` key.
    let g = with(
        r#"{"bans":[{"value":"ana","team":"own"}],
            "picks":[{"value":"ana"}],
            "ban_vote":[{"value":"mei"},{"value":"tracer"},{"value":"genji"},
                        {"value":"sigma"},{"value":"zarya"},{"value":"mercy"}]}"#,
        Some(BansPicksStatus::Ok),
    );
    no_vote_row(&g);
    let rows = rows_of(&g);
    let names: Vec<_> = rows.iter().flat_map(|r| names(r)).collect();
    assert!(
        !names.contains(&"Mei") && !names.contains(&"Tracer") && !names.contains(&"Genji"),
        "{names:?}"
    );
}

#[test]
fn a_missing_or_null_ban_votes_is_not_read() {
    // Null (vote screen not recognised), with a flat list beside it.
    let g = with(
        r#"{"bans":[{"value":"ana","team":"own"}],"picks":[{"value":"ana"}],
            "ban_vote":[{"value":"mei"}],"ban_votes":null}"#,
        Some(BansPicksStatus::Ok),
    );
    no_vote_row(&g);
    // Only teammates' slots: none of ours, so not read either.
    let g = with(
        r#"{"bans":[{"value":"ana","team":"own"}],"picks":[{"value":"ana"}],
            "ban_votes":[{"voter":1,"slot":1,"state":"hero","hero":"mei"}]}"#,
        Some(BansPicksStatus::Ok),
    );
    no_vote_row(&g);
    // A game that only has a flat list reads as nothing at all there.
    let g = with(
        r#"{"ban_vote":[{"value":"mei"}]}"#,
        Some(BansPicksStatus::Ok),
    );
    assert!(
        !parse(g.bans_picks.as_deref().unwrap())
            .unwrap()
            .read_something()
    );
}

#[test]
fn an_empty_slot_is_not_seen_not_a_missing_vote() {
    let g = with(
        r#"{"bans":[{"value":"ana","team":"own"}],
            "ban_votes":[{"voter":0,"slot":1,"state":"empty","hero":null},
                         {"voter":0,"slot":2,"state":"skipped","hero":null},
                         {"voter":0,"slot":3,"state":"empty","hero":null}]}"#,
        Some(BansPicksStatus::Ok),
    );
    let votes = vote_row(&g);
    assert_eq!(
        votes.iter().map(Chip::name).collect::<Vec<_>>(),
        ["Not seen", "Skipped", "Not seen"]
    );
    for c in &votes {
        let tip = c.tip.to_lowercase();
        assert!(!tip.contains("didn't") && !tip.contains("did not"), "{tip}");
    }
    assert_eq!(votes[1].tip, "You skipped ban vote 2");
    assert!(votes[0].tip.contains("not seen"), "{}", votes[0].tip);
}

#[test]
fn a_swap_needs_two_real_heroes_that_differ() {
    let own = Slot::Hero("Wrecking Ball".into());
    let played = played_chip("Wrecking Ball", Some(&own), false).unwrap();
    assert!(!played.swap);
    let other = Slot::Hero("Mei".into());
    assert!(
        played_chip("Wrecking Ball", Some(&other), false)
            .unwrap()
            .swap
    );
    assert!(
        !played_chip("Wrecking Ball", Some(&Slot::NotRead), false)
            .unwrap()
            .swap
    );
    assert!(
        !played_chip("Wrecking Ball", Some(&Slot::NoPreference), false)
            .unwrap()
            .swap
    );
    assert!(!played_chip("Wrecking Ball", None, false).unwrap().swap);
    assert!(played_chip("Unknown", None, false).is_none());
    assert!(played_chip("  ", None, false).is_none());
}

/// The normal game, played as the given runs (hero, snapshots), all
/// confirmed, like the Eichenwalde game that showed only its first hero.
fn played_as(runs: &[(&str, u32)]) -> Game {
    let mut g = game(NORMAL);
    let base = g.segments[0].clone();
    g.segments = runs
        .iter()
        .enumerate()
        .map(|(i, (hero, snapshots))| crate::model::SegmentView {
            index: i as u32,
            hero: (*hero).into(),
            snapshots: *snapshots,
            confirmed: true,
            ..base.clone()
        })
        .collect();
    g
}

fn played(g: &Game) -> Vec<Chip> {
    chips(&rows_of(g), LABEL_PLAYED)
}

#[test]
fn played_lists_every_hero_in_the_order_played() {
    let g = played_as(&[("Wrecking Ball", 12), ("Sigma", 6), ("Sierra", 6)]);
    let chips = played(&g);
    let names: Vec<_> = chips.iter().map(Chip::name).collect();
    assert_eq!(names, ["Wrecking Ball", "Sigma", "Sierra"]);
    // The You tag is drawn once, on the first chip.
    assert!(chips[0].you_tag && chips[1..].iter().all(|c| !c.you_tag));
    assert!(chips.iter().all(|c| !c.ban));
    assert!(chips.iter().all(|c| !c.swap), "we picked Wrecking Ball");
    assert_eq!(chips[0].tip, "Played Wrecking Ball, you");
    assert_eq!(chips[1].tip, "Then played Sigma, you");
    assert_eq!(chips[2].tip, "Then played Sierra, you");
    assert!(!block_of(&g).legend().swap);
    // The hero bar shows the same heroes in the same order.
    let bar: Vec<_> = crate::games_detail::hero_split(&g)
        .bar
        .into_iter()
        .map(|b| b.hero)
        .collect();
    assert_eq!(bar, ["Wrecking Ball", "Sigma", "Sierra"]);
}

#[test]
fn played_merges_repeats_and_keeps_a_return_in_first_place() {
    let g = played_as(&[
        ("Wrecking Ball", 4),
        ("Wrecking Ball", 3),
        ("Sigma", 6),
        ("Unknown", 2),
        ("Wrecking Ball", 5),
        ("Sierra", 2),
    ]);
    let names: Vec<_> = played(&g).iter().map(|c| c.name().to_string()).collect();
    assert_eq!(names, ["Wrecking Ball", "Sigma", "Sierra"]);
}

#[test]
fn played_follows_the_hero_bar_for_dismissed_and_unconfirmed_runs() {
    // UNSURE: Wrecking Ball, a one-snapshot unconfirmed Sigma, Wrecking
    // Ball again. The bar shows the Sigma run, so the card does too.
    let g = game(UNSURE);
    let names: Vec<_> = played(&g).iter().map(|c| c.name().to_string()).collect();
    assert_eq!(names, ["Wrecking Ball", "Sigma"]);
    // Dismissed, the run folds into Wrecking Ball on the bar and the card.
    let mut g = g;
    g.segments[1].dismissed = true;
    let names: Vec<_> = played(&g).iter().map(|c| c.name().to_string()).collect();
    assert_eq!(names, ["Wrecking Ball"]);
}

#[test]
fn a_corrected_hero_keeps_the_single_played_chip() {
    let mut g = played_as(&[("Wrecking Ball", 12), ("Sigma", 6), ("Sierra", 6)]);
    g.edited_fields.push("hero".into());
    g.hero = "Sigma".into();
    let chips = played(&g);
    assert_eq!(chips.len(), 1);
    assert_eq!(chips[0].name(), "Sigma");
    assert!(chips[0].you_tag && chips[0].swap);
    assert_eq!(
        chips[0].tip,
        "Played Sigma, Wrecking Ball at hero select, you"
    );
    // No runs at all: the row's hero, as before.
    let mut g = game(NORMAL);
    g.segments.clear();
    let chips = played(&g);
    assert_eq!(chips.len(), 1);
    assert_eq!(chips[0].tip, "Played Wrecking Ball, you");
}

#[test]
fn only_the_first_played_chip_marks_a_swap() {
    // UNSURE picked Reinhardt at hero select, and we never played him.
    let mut g = game(UNSURE);
    g.segments[1].confirmed = true;
    g.segments[1].hero = "Sigma".into();
    g.segments[2].hero = "Mei".into();
    let chips = played(&g);
    let names: Vec<_> = chips.iter().map(Chip::name).collect();
    assert_eq!(names, ["Wrecking Ball", "Sigma", "Mei"]);
    assert!(chips[0].swap && chips[0].you_tag);
    assert_eq!(
        chips[0].tip,
        "Played Wrecking Ball, Reinhardt at hero select, you"
    );
    assert!(chips[1..].iter().all(|c| !c.swap && !c.you_tag));
    assert_eq!(chips[2].tip, "Then played Mei, you");
    assert!(block_of(&g).legend().swap);
}

#[test]
fn no_swap_mark_when_the_pick_is_one_of_the_played_chips() {
    // UNSURE picked Reinhardt at hero select and went back to him later.
    let mut g = game(UNSURE);
    g.segments[1].confirmed = true;
    g.segments[1].hero = "Sigma".into();
    g.segments[2].hero = "Reinhardt".into();
    let chips = played(&g);
    let names: Vec<_> = chips.iter().map(Chip::name).collect();
    assert_eq!(names, ["Wrecking Ball", "Sigma", "Reinhardt"]);
    assert!(chips.iter().all(|c| !c.swap));
    assert!(chips[0].you_tag);
    assert_eq!(chips[0].tip, "Played Wrecking Ball, you");
    assert!(!block_of(&g).legend().swap);
    // The same holds when the case of the name differs.
    g.segments[2].hero = "REINHARDT".into();
    assert!(played(&g).iter().all(|c| !c.swap));
}

/// A real cv-v1 record below the accept bar (value null, confidence 0,
/// score 0.41, margin 0.05, as `Reader::read_of` writes it) is a Not read
/// outline chip, with no warning words: cv-v1 has no unsure state (a3b
/// decisions).
#[test]
fn a_below_bar_read_is_a_not_read_chip() {
    let below = r#""value":null,"confidence":0.0,"score":0.41,"margin":0.05"#;
    let g = with(
        &format!(
            r#"{{"layout":"plain",
                "bans":[{{"value":"ana","team":"own","confidence":0.93,"score":0.81,"margin":0.31}},
                        {{{below},"team":"enemy"}}],
                "lobby_ban":null,
                "picks":[{{"value":"reinhardt","confidence":0.9,"score":0.8,"margin":0.3}},{{{below}}}],
                "ban_votes":[{{"voter":0,"slot":1,"state":"hero","hero":null,
                              "confidence":0.0,"score":0.41,"margin":0.05}},
                             {{"voter":0,"slot":2,"state":"hero","hero":"mei",
                              "confidence":0.9,"score":0.78,"margin":0.3}}],
                "bans_picks_recognizer":"cv-v1"}}"#
        ),
        Some(BansPicksStatus::Ok),
    );
    let b = block_of(&g);
    assert_eq!(
        shape(row(&b.rows, LABEL_BANS)),
        ["Your team", "[Ana]", "|", "Their team", "[Not read]"]
    );
    let ban = &chips(&b.rows, LABEL_BANS)[1];
    assert_eq!(ban.slot, Slot::NotRead);
    assert!(ban.outline());
    assert_eq!(ban.tip, "Their ban 1: not read");
    let pick = &chips(&b.rows, LABEL_HERO_SELECT)[1];
    assert_eq!(pick.slot, Slot::NotRead);
    assert_eq!(pick.tip, "Hero select slot 2: not read");
    let votes = chips(&b.rows, LABEL_VOTES);
    assert_eq!(votes.len(), 2);
    assert_eq!(votes[0].slot, Slot::NotRead);
    assert!(votes[0].outline());
    assert_eq!(votes[0].tip, "Your ban vote 1: not read");
    for c in b.chips() {
        let tip = c.tip.to_lowercase();
        assert!(!tip.contains("misread") && !tip.contains("unsure"), "{tip}");
    }
    assert!(b.legend().outline);
    // The partial fixture's ban below the bar too.
    assert!(
        chips(&block_of(&game(UNSURE)).rows, LABEL_BANS)
            .iter()
            .any(|c| c.name() == "Not read")
    );
}

#[test]
fn nothing_read_shows_the_reason_and_never_hides_the_game() {
    let BansPicks::Reason {
        line,
        tip,
        settings,
    } = bans_picks(&game(UNDECIDED))
    else {
        panic!("reason")
    };
    assert_eq!(line, "Not read: Read bans and picks was off for this game.");
    assert!(tip.is_none());
    let link = settings.unwrap();
    assert!(link.tip.contains("Read bans and picks"));
    assert_eq!(link.open, AdvancedSection::Troubleshooting);

    let BansPicks::Reason {
        line,
        tip,
        settings,
    } = bans_picks(&game(REJECTED_2))
    else {
        panic!("reason")
    };
    assert_eq!(
        line,
        "Not read: this game ran at 1920x1080. Bans and picks need 2560x1440."
    );
    assert_eq!(tip, Some("Only 2560x1440 is read. 1080p and 4K are not."));
    assert!(settings.is_none());
}

#[test]
fn every_status_has_its_spec_line() {
    let line = |s: BansPicksStatus| {
        let mut g = game(HELD);
        g.bans_picks = None;
        g.bans_picks_status = Some(s);
        match bans_picks(&g) {
            BansPicks::Reason { line, settings, .. } => (line, settings.is_some()),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(
        line(BansPicksStatus::NoFrames),
        (
            "Not read: the tracker didn't see the pre-game screens for this game.".into(),
            false
        )
    );
    assert_eq!(
        line(BansPicksStatus::FrameSize {
            width: 0,
            height: 0
        }),
        (
            "Not read: bans and picks need a 2560x1440 game.".into(),
            false
        )
    );
    let (text, button) = line(BansPicksStatus::PackMissing { linked: false });
    assert!(text.starts_with("Not read: the hero icon pack isn't installed."));
    assert!(text.contains("Link a site account in Settings"));
    assert!(button);
    let (text, button) = line(BansPicksStatus::PackMissing { linked: true });
    assert!(text.contains("fetches it in the background"));
    assert!(!button);
    let (text, button) = line(BansPicksStatus::PackFailed);
    assert!(text.contains("Reinstall it"));
    assert!(button);
    let (text, button) = line(BansPicksStatus::ReaderFailed);
    assert_eq!(text, "Not read: the reader failed on this game's screens.");
    assert!(!button);
}

#[test]
fn open_settings_opens_the_row_that_holds_the_fix() {
    let open = |s: BansPicksStatus| {
        let mut g = game(HELD);
        g.bans_picks = None;
        g.bans_picks_status = Some(s);
        match bans_picks(&g) {
            BansPicks::Reason { settings, .. } => settings.map(|l| l.open),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(
        open(BansPicksStatus::ReaderOff),
        Some(AdvancedSection::Troubleshooting)
    );
    assert_eq!(
        open(BansPicksStatus::PackFailed),
        Some(AdvancedSection::Troubleshooting)
    );
    assert_eq!(
        open(BansPicksStatus::PackMissing { linked: false }),
        Some(AdvancedSection::WebsiteConnection)
    );
    assert_eq!(open(BansPicksStatus::NoFrames), None);
}

#[test]
fn an_old_game_with_no_status_has_no_block() {
    let mut g = game(HELD);
    g.bans_picks = None;
    g.bans_picks_status = None;
    assert_eq!(bans_picks(&g), BansPicks::Hidden);
    // A good status with nothing in the JSON has no text to show either.
    g.bans_picks_status = Some(BansPicksStatus::Ok);
    assert_eq!(bans_picks(&g), BansPicks::Hidden);
}

#[test]
fn a_stored_result_that_does_not_parse_falls_back_to_the_status() {
    for bad in ["", "not json", "[]", "null", r#"{"bans":"x"}"#] {
        let g = with(bad, Some(BansPicksStatus::ReaderFailed));
        assert!(matches!(bans_picks(&g), BansPicks::Reason { .. }), "{bad}");
    }
    // Nothing read in a result that parses is the same as no result.
    let g = with(
        r#"{"bans":[{"value":null}],"picks":[{"value":null}]}"#,
        Some(BansPicksStatus::ReaderOff),
    );
    assert!(matches!(bans_picks(&g), BansPicks::Reason { .. }));
}

#[test]
fn unknown_hero_keys_still_read_as_words() {
    assert_eq!(hero_name("some_new-hero"), "some new hero");
    assert_eq!(hero_name("wrecking-ball"), "Wrecking Ball");
}

/// Four unread ban slots as cv-v1 stores them (value null, confidence 0,
/// a score below the bar), with a team from the bars.
const FOUR_NULL_BANS: &str = r#"[
    {"value":null,"confidence":0.0,"score":0.41,"margin":0.05,"team":"own"},
    {"value":null,"confidence":0.0,"score":0.38,"margin":0.02,"team":"own"},
    {"value":null,"confidence":0.0,"score":0.43,"margin":0.09,"team":"enemy"},
    {"value":null,"confidence":0.0,"score":0.40,"margin":0.04,"team":"enemy"}]"#;

/// No team ban read, the lobby ban read: the four stored slots are Not read
/// chips, then the Lobby group. Never `Lobby [Sigma]` alone, which would
/// read as "no team bans".
#[test]
fn a_lobby_ban_alone_keeps_the_unread_ban_slots() {
    let g = with(
        &format!(
            r#"{{"layout":"lobby","bans":{FOUR_NULL_BANS},
                "lobby_ban":{{"value":"sigma","confidence":0.9,"score":0.8,"margin":0.3}},
                "bans_picks_recognizer":"cv-v1"}}"#
        ),
        Some(BansPicksStatus::Ok),
    );
    let b = block_of(&g);
    let bans = row(&b.rows, LABEL_BANS);
    assert_eq!(
        shape(bans),
        [
            "[Not read, Not read, Not read, Not read]",
            "|",
            "Lobby",
            "[Sigma]"
        ]
    );
    let tips: Vec<_> = bans.chips().map(|c| c.tip.as_str()).collect();
    assert_eq!(
        tips,
        [
            "Ban 1: not read",
            "Ban 2: not read",
            "Ban 3: not read",
            "Ban 4: not read",
            "Lobby ban: Sigma"
        ]
    );
    assert!(bans.chips().take(4).all(Chip::outline));
    // The legend gains the outline entry by itself.
    assert_eq!(
        b.legend(),
        Legend {
            banned: true,
            outline: true,
            ..Legend::default()
        }
    );
    // The fixture game for this case draws the same.
    let b = block_of(&game(LOBBY_ONLY));
    assert_eq!(
        shape(row(&b.rows, LABEL_BANS)),
        [
            "[Not read, Not read, Not read, Not read]",
            "|",
            "Lobby",
            "[Sigma]"
        ]
    );
    assert!(b.legend().outline);
}

/// No ban slot stored at all and the lobby ban read: the status line, then
/// the Lobby group.
#[test]
fn a_lobby_ban_with_no_stored_ban_slot_keeps_the_status_line() {
    let g = with(
        r#"{"layout":"lobby","bans":[],"lobby_ban":{"value":"sigma"}}"#,
        Some(BansPicksStatus::Ok),
    );
    assert_eq!(
        shape(row(&rows_of(&g), LABEL_BANS)),
        ["Ban screen seen, heroes not read", "|", "Lobby", "[Sigma]"]
    );
}

/// The lobby layout with its slot below the bar is `Lobby [Not read]`; the
/// plain layout has no lobby slot and no Lobby group.
#[test]
fn an_unread_lobby_slot_is_not_read_only_in_the_lobby_layout() {
    let bans = r#"[{"value":"ana","team":"own"},{"value":"mei","team":"enemy"}]"#;
    let g = with(
        &format!(
            r#"{{"layout":"lobby","bans":{bans},
                "lobby_ban":{{"value":null,"confidence":0.0,"score":0.3,"margin":0.02}}}}"#
        ),
        Some(BansPicksStatus::Ok),
    );
    let rows = rows_of(&g);
    let r = row(&rows, LABEL_BANS);
    assert_eq!(
        shape(r),
        [
            "Your team",
            "[Ana]",
            "|",
            "Their team",
            "[Mei]",
            "|",
            "Lobby",
            "[Not read]"
        ]
    );
    let lobby = r.chips().last().unwrap();
    assert!(lobby.outline());
    assert_eq!(lobby.tip, "Lobby ban: not read");
    // The plain layout stores `lobby_ban: null`: no Lobby group.
    let g = with(
        &format!(r#"{{"layout":"plain","bans":{bans},"lobby_ban":null}}"#),
        Some(BansPicksStatus::Ok),
    );
    assert_eq!(
        shape(row(&rows_of(&g), LABEL_BANS)),
        ["Your team", "[Ana]", "|", "Their team", "[Mei]"]
    );
    // A null value outside the lobby layout is no lobby slot either.
    let g = with(
        &format!(r#"{{"layout":"plain","bans":{bans},"lobby_ban":{{"value":null}}}}"#),
        Some(BansPicksStatus::Ok),
    );
    assert!(!shape(row(&rows_of(&g), LABEL_BANS)).contains(&"Lobby".to_string()));
    // Nothing on the ban screen read: the quiet line covers the lobby slot.
    let g = with(
        &format!(
            r#"{{"layout":"lobby","bans":{FOUR_NULL_BANS},"lobby_ban":{{"value":null}},
                "picks":[{{"value":"ana"}}]}}"#
        ),
        Some(BansPicksStatus::Ok),
    );
    assert_eq!(
        shape(row(&rows_of(&g), LABEL_BANS)),
        ["Ban screen seen, heroes not read"]
    );
}

/// One grey bar: the bans with a known team keep their groups, and only
/// the team-less ones go under Team not read.
#[test]
fn only_bans_with_no_team_go_under_team_not_read() {
    let g = with(
        r#"{"layout":"lobby","bans":[
              {"value":"sombra","team":"own"},
              {"value":null,"confidence":0.0,"score":0.2,"margin":0.01,"team":null},
              {"value":"ana","team":"enemy"},
              {"value":"winston","team":"enemy"}],
            "lobby_ban":{"value":"sigma"}}"#,
        Some(BansPicksStatus::Ok),
    );
    let rows = rows_of(&g);
    let r = row(&rows, LABEL_BANS);
    assert_eq!(
        shape(r),
        [
            "Your team",
            "[Sombra]",
            "|",
            "Their team",
            "[Ana, Winston]",
            "|",
            "[Not read]",
            "Team not read",
            "|",
            "Lobby",
            "[Sigma]"
        ]
    );
    assert!(r.parts.contains(&Part::Note {
        text: "Team not read",
        tip: "The tracker couldn't tell which team made these bans.",
    }));
    let tips: Vec<_> = r.chips().map(|c| c.tip.as_str()).collect();
    assert_eq!(
        tips,
        [
            "Banned by your team: Sombra",
            "Banned by their team: Ana",
            "Banned by their team: Winston",
            "Ban 2: not read",
            "Lobby ban: Sigma"
        ]
    );
    // A read hero with no team says so on its own chip.
    let g = with(
        r#"{"bans":[{"value":"sombra","team":"own"},{"value":"mei","team":null}]}"#,
        Some(BansPicksStatus::Ok),
    );
    let rows = rows_of(&g);
    let r = row(&rows, LABEL_BANS);
    assert_eq!(
        shape(r),
        ["Your team", "[Sombra]", "|", "[Mei]", "Team not read"]
    );
    assert_eq!(r.chips().last().unwrap().tip, "Banned: Mei, team not read");
    for tip in r.chips().map(|c| c.tip.as_str()) {
        assert!(!tip.contains("for this game"), "{tip}");
    }
}

/// Only cv-v1 (or no tag, as sync reads it) is drawn as chips. Any other
/// reader's result is one quiet line, never plain sure chips.
#[test]
fn a_result_from_another_reader_is_not_drawn_as_chips() {
    let body = r#""bans":[{"value":"ana","team":"own"}],"picks":[{"value":"mei"}]"#;
    for tag in [r#""cv-v1""#, "null"] {
        let g = with(
            &format!(r#"{{{body},"bans_picks_recognizer":{tag}}}"#),
            Some(BansPicksStatus::Ok),
        );
        assert!(matches!(bans_picks(&g), BansPicks::Block(_)), "{tag}");
    }
    let g = with(&format!("{{{body}}}"), Some(BansPicksStatus::Ok));
    assert!(matches!(bans_picks(&g), BansPicks::Block(_)), "no tag");
    for tag in [r#""cv-v2""#, r#""""#, r#""CV-V1""#, "2"] {
        let g = with(
            &format!(r#"{{{body},"bans_picks_recognizer":{tag}}}"#),
            Some(BansPicksStatus::Ok),
        );
        assert_eq!(
            bans_picks(&g),
            BansPicks::Reason {
                line: "Not shown: this game was read by a reader this version doesn't know. Update the tracker to see it."
                    .into(),
                tip: None,
                settings: None,
            },
            "{tag}"
        );
    }
}

/// The TEAM BAN SCORE strip is the team's totals, never our own votes: its
/// heroes show nowhere in the block.
#[test]
fn the_ban_score_strip_never_shows_up_in_your_ban_votes() {
    let g = game(NORMAL);
    let json: Value = serde_json::from_str(g.bans_picks.as_deref().unwrap()).unwrap();
    let strip: Vec<String> = json["ban_score"]
        .as_array()
        .expect("the fixture writes the strip")
        .iter()
        .map(|e| hero_name(e["hero"].as_str().unwrap()))
        .collect();
    assert_eq!(strip, ["Widowmaker", "Pharah", "Zarya"]);
    let rows = rows_of(&g);
    assert_eq!(
        names(row(&rows, LABEL_VOTES)),
        ["Reinhardt", "Tracer", "Genji"]
    );
    let all: Vec<&str> = rows.iter().flat_map(|r| names(r)).collect();
    for hero in &strip {
        assert!(!all.contains(&hero.as_str()), "{hero} in {all:?}");
    }
    // A strip with no `ban_votes` gives no vote chips either.
    let g = with(
        r#"{"bans":[{"value":"ana","team":"own"}],"ban_votes":null,
            "ban_score":[{"rank":1,"hero":"widowmaker","votes":3,"empty":false}]}"#,
        Some(BansPicksStatus::Ok),
    );
    no_vote_row(&g);
}

/// The fixture records look like what the daemon stores.
#[test]
fn fixture_records_have_the_daemon_shape() {
    let games = games_from_snapshot(&crate::fixtures_games::snapshot());
    let mut lobby_layouts = 0;
    for g in games.iter().filter(|g| g.bans_picks.is_some()) {
        let sid = &g.session_id;
        let v: Value = serde_json::from_str(g.bans_picks.as_deref().unwrap()).unwrap();
        assert_eq!(v["bans_picks_recognizer"], "cv-v1", "{sid}");
        let layout = v["layout"].as_str();
        assert!(
            matches!(layout, None | Some("plain" | "lobby")),
            "{sid}: {layout:?}"
        );
        lobby_layouts += usize::from(layout == Some("lobby"));
        // The lobby slot exactly in the lobby layout.
        assert_eq!(layout == Some("lobby"), !v["lobby_ban"].is_null(), "{sid}");
        if layout.is_none() {
            assert_eq!(v["bans"].as_array().unwrap().len(), 0, "{sid}");
        } else {
            assert_eq!(v["bans"].as_array().unwrap().len(), 4, "{sid}");
        }
        // Every null value is below the bar, with confidence 0.
        let mut reads: Vec<&Value> = v["bans"].as_array().unwrap().iter().collect();
        reads.extend(v["picks"].as_array().unwrap());
        if !v["lobby_ban"].is_null() {
            reads.push(&v["lobby_ban"]);
        }
        for r in reads {
            if r["value"].is_null() {
                assert_eq!(r["confidence"], 0.0, "{sid}: {r}");
                let (score, margin) = (r["score"].as_f64().unwrap(), r["margin"].as_f64().unwrap());
                assert!(score < 0.45 || margin < 0.12, "{sid}: {r}");
            } else {
                assert!(r["confidence"].as_f64().unwrap() > 0.0, "{sid}: {r}");
            }
        }
        // `ban_votes` and `ban_score` are null together, when the vote
        // screen was not seen; a slot with no hero ran no match.
        assert_eq!(v["ban_votes"].is_null(), v["ban_score"].is_null(), "{sid}");
        if let Some(slots) = v["ban_votes"].as_array() {
            for s in slots {
                if s["hero"].is_null() {
                    assert_eq!(s["confidence"], 0.0, "{sid}: {s}");
                    if s["state"] != "hero" {
                        assert_eq!(s["score"], 0.0, "{sid}: {s}");
                        assert_eq!(s["margin"], 0.0, "{sid}: {s}");
                    }
                }
            }
        }
        // `Partial` always names a missing screen (else it is `Ok`).
        if let Some(BansPicksStatus::Partial { missing }) = &g.bans_picks_status {
            assert!(!missing.is_empty(), "{sid}");
        }
    }
    assert!(lobby_layouts >= 2, "{lobby_layouts}");
    // The game with no ban screen seen stores neither layout nor lobby, and
    // its vote screen was not seen either.
    let v: Value = serde_json::from_str(game(NO_BAN).bans_picks.as_deref().unwrap()).unwrap();
    assert!(v["layout"].is_null() && v["lobby_ban"].is_null());
    assert!(v["ban_votes"].is_null() && v["ban_score"].is_null());
}

/// The user copy as written, not compared to its own constants.
#[test]
fn the_user_words_are_spelled_out() {
    let b = block_of(&game(UNSURE));
    assert_eq!(
        labels(&b.rows),
        ["Bans", "Your ban votes", "Hero select", "Played"]
    );
    assert_eq!(
        b.legend()
            .items()
            .into_iter()
            .map(|(_, w)| w)
            .collect::<Vec<_>>(),
        ["Banned", "Swapped hero", "Not read or not seen"]
    );
    let side = block_of(&game(SIDE));
    let note = row(&side.rows, "Bans").parts.iter().find_map(|p| match p {
        Part::Note { text, .. } => Some(*text),
        _ => None,
    });
    assert_eq!(note, Some("Team not read"));
    assert_eq!(HEADER, "Bans and picks");
    let groups: Vec<_> = row(&block_of(&game(NORMAL)).rows, "Bans")
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Key(k) => Some(*k),
            _ => None,
        })
        .collect();
    assert_eq!(groups, ["Your team", "Their team", "Lobby"]);
}

/// A Played run the hero bar marks unconfirmed says so in its tooltip.
#[test]
fn an_unconfirmed_played_run_says_unconfirmed() {
    // UNSURE: Wrecking Ball, then a one-snapshot unconfirmed Sigma.
    let chips = played(&game(UNSURE));
    assert_eq!(chips[1].name(), "Sigma");
    assert_eq!(chips[1].tip, "Then played Sigma, unconfirmed, you");
    assert!(!chips[0].tip.contains("unconfirmed"), "{}", chips[0].tip);
    // Confirmed, the word goes.
    let mut g = game(UNSURE);
    g.segments[1].confirmed = true;
    g.segments[1].snapshots = 6;
    assert_eq!(played(&g)[1].tip, "Then played Sigma, you");
    // The first chip says so too when its only run is unconfirmed.
    assert_eq!(
        played_chip("Sigma", None, true).unwrap().tip,
        "Played Sigma, unconfirmed, you"
    );
    assert_eq!(
        played_chip("Sigma", Some(&Slot::Hero("Mei".into())), true)
            .unwrap()
            .tip,
        "Played Sigma, Mei at hero select, unconfirmed, you"
    );
}

/// A game the reader ran on (status `Ok` or `Partial`) that read nothing
/// still has its block, with the per-screen lines; a game from before the
/// reader still has none.
#[test]
fn a_game_the_reader_ran_on_but_read_nothing_says_so_per_screen() {
    let nothing = format!(
        r#"{{"layout":"plain","bans":{FOUR_NULL_BANS},"lobby_ban":null,"picks":[],
            "ban_vote":[],"ban_votes":null,"ban_score":null,"bans_picks_recognizer":"cv-v1"}}"#
    );
    let g = with(
        &nothing,
        Some(BansPicksStatus::Partial {
            missing: vec![BansPicksMissing::BanVote, BansPicksMissing::HeroSelect],
        }),
    );
    let rows = rows_of(&g);
    assert_eq!(labels(&rows), FOUR);
    assert_eq!(
        quiet(row(&rows, "Bans")).unwrap().0,
        "Ban screen seen, heroes not read"
    );
    assert_eq!(
        quiet(row(&rows, "Your ban votes")).unwrap().0,
        "Screen not seen"
    );
    assert_eq!(
        quiet(row(&rows, "Hero select")).unwrap().0,
        "Screen not seen"
    );
    // Ok, nothing read, no ban screen stored.
    let g = with(r#"{"bans":[],"picks":[]}"#, Some(BansPicksStatus::Ok));
    assert!(matches!(bans_picks(&g), BansPicks::Block(_)));
    // No status: a game from before the reader keeps no block.
    let g = with(&nothing, None);
    assert_eq!(bans_picks(&g), BansPicks::Hidden);
}

/// Vote tooltips use the stored slot number; slots outside 1 to 3 and
/// repeats are left out.
#[test]
fn vote_tooltips_use_the_stored_slot_number() {
    let g = with(
        r#"{"bans":[{"value":"ana","team":"own"}],
            "ban_votes":[
              {"voter":0,"slot":3,"state":"skipped","hero":null},
              {"voter":0,"slot":1,"state":"hero","hero":"mei"},
              {"voter":0,"slot":1,"state":"skipped","hero":null},
              {"voter":0,"slot":0,"state":"skipped","hero":null},
              {"voter":0,"slot":4,"state":"empty","hero":null},
              {"voter":0,"state":"empty","hero":null}]}"#,
        Some(BansPicksStatus::Ok),
    );
    let votes = vote_row(&g);
    let tips: Vec<_> = votes.iter().map(|c| c.tip.as_str()).collect();
    assert_eq!(tips, ["You voted to ban Mei", "You skipped ban vote 3"]);
    // Only bad slots: the votes are not read.
    let g = with(
        r#"{"bans":[{"value":"ana","team":"own"}],
            "ban_votes":[{"voter":0,"slot":9,"state":"skipped","hero":null}]}"#,
        Some(BansPicksStatus::Ok),
    );
    no_vote_row(&g);
}
