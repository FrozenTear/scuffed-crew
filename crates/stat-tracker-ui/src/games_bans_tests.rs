//! Tests for the bans and picks view model (`games_bans`).

use super::*;
use crate::fixtures_games::{HELD, NORMAL, REJECTED_2, UNDECIDED, UNSURE};
use crate::snapshot::games_from_snapshot;

/// The history games that carry the extra fixture states.
const SIDE: &str = "sess-g-h01";
const NO_BAN: &str = "sess-g-h02";
const SKIP: &str = "sess-g-h03";

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
    assert!(picks[0].you && picks[1..].iter().all(|c| !c.you));
    assert_eq!(names(row(&b.rows, LABEL_PLAYED)), ["Wrecking Ball"]);
    assert_eq!(
        b.legend(),
        Legend {
            banned: true,
            ..Legend::default()
        }
    );
    assert_eq!(b.pill(), None);
}

#[test]
fn tooltips_say_what_the_chip_is() {
    let rows = rows_of(&game(NORMAL));
    let tip = |label: &str, c: usize| chips(&rows, label)[c].tooltip();
    assert_eq!(tip(LABEL_BANS, 0), "Banned: Sombra");
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
            unsure: false,
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
        tip: "Which team made each ban wasn't read for this game.",
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
    assert!(votes.iter().all(|c| !c.you));
    // We picked Reinhardt and played Wrecking Ball.
    let played = chips(&rows, LABEL_PLAYED);
    assert!(played[0].swap && played[0].you);
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
    // Only the lobby ban was read: it stands alone, with no line saying no
    // hero was read.
    assert_eq!(shape(row(&rows, LABEL_BANS)), ["Lobby", "[Sigma]"]);
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

/// Gibraltar: the status says the ban screen and the ban vote were never
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
    assert_eq!(b.pill(), None);
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

/// Samoa: the bans were read but not which team made each, and hero
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
            Some("The tracker didn't see hero select for this game.")
        ))
    );
    assert_eq!(
        names(row(&b.rows, LABEL_VOTES)),
        ["Reinhardt", "Tracer", "Skipped"]
    );
    // No hero select pick, so no swap; the Played chip is plain.
    let played = chips(&b.rows, LABEL_PLAYED);
    assert_eq!(played.len(), 1);
    assert!(played[0].you && !played[0].swap);
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
    assert!(votes.iter().all(|c| !c.you));
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
    let played = played_chip("Wrecking Ball", Some(&own)).unwrap();
    assert!(!played.swap);
    let other = Slot::Hero("Mei".into());
    assert!(played_chip("Wrecking Ball", Some(&other)).unwrap().swap);
    assert!(
        !played_chip("Wrecking Ball", Some(&Slot::NotRead))
            .unwrap()
            .swap
    );
    assert!(
        !played_chip("Wrecking Ball", Some(&Slot::NoPreference))
            .unwrap()
            .swap
    );
    assert!(!played_chip("Wrecking Ball", None).unwrap().swap);
    assert!(played_chip("Unknown", None).is_none());
    assert!(played_chip("  ", None).is_none());
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
    assert!(chips[0].you && chips[1..].iter().all(|c| !c.you));
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
    assert!(chips[0].you && chips[0].swap);
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
    assert!(chips[0].swap && chips[0].you);
    assert_eq!(
        chips[0].tip,
        "Played Wrecking Ball, Reinhardt at hero select, you"
    );
    assert!(chips[1..].iter().all(|c| !c.swap && !c.you));
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
    assert!(chips[0].you);
    assert_eq!(chips[0].tip, "Played Wrecking Ball, you");
    assert!(!block_of(&g).legend().swap);
    // The same holds when the case of the name differs.
    g.segments[2].hero = "REINHARDT".into();
    assert!(played(&g).iter().all(|c| !c.swap));
}

/// No game stores a suspect flag or a weak flag for bans and picks yet,
/// so no fixture state draws amber.
#[test]
fn no_stored_game_is_unsure_or_weak_yet() {
    for sid in [NORMAL, UNSURE, SIDE, NO_BAN, SKIP] {
        let b = block_of(&game(sid));
        assert!(!b.weak, "{sid}");
        assert!(b.chips().all(|c| !c.suspect), "{sid}");
        assert_eq!(b.unsure(), 0, "{sid}");
        assert_eq!(b.pill(), None, "{sid}");
        assert!(!b.legend().unsure, "{sid}");
    }
}

fn mark(b: &mut Block, label: &str, nth: usize) {
    let r = b.rows.iter_mut().find(|r| r.label == label).unwrap();
    let chip = r
        .parts
        .iter_mut()
        .filter_map(|p| match p {
            Part::Chips(chips) => Some(chips),
            _ => None,
        })
        .flat_map(|chips| chips.iter_mut())
        .nth(nth)
        .unwrap();
    chip.suspect = true;
}

/// The hook: once a read carries the flag, its chip turns unsure and the
/// header counts it. Played and slots that are not a hero never count.
#[test]
fn the_suspect_hook_counts_bans_votes_and_hero_select() {
    let mut b = block_of(&game(NORMAL));
    mark(&mut b, LABEL_BANS, 1); // Mauga
    mark(&mut b, LABEL_VOTES, 2); // Genji
    mark(&mut b, LABEL_PLAYED, 0);
    assert_eq!(b.unsure(), 2);
    assert_eq!(b.pill(), Some(Pill::Unsure(2)));
    assert_eq!(Pill::Unsure(2).words(), "2 unsure");
    assert_eq!(
        Pill::Unsure(2).tip(),
        "Reads the tracker is not sure about. They count only bans, ban votes and hero select."
    );
    assert!(b.legend().unsure);
    let mauga = chips(&b.rows, LABEL_BANS)[1].clone();
    assert!(mauga.unsure());
    assert_eq!(
        mauga.tooltip(),
        "Banned: Mauga. Mauga might be misread. Use Report misread if it is wrong."
    );
    // A Not read slot with the flag is still only Not read.
    let mut b = block_of(&game(UNSURE));
    mark(&mut b, LABEL_BANS, 2);
    assert_eq!(b.unsure(), 0);
    assert_eq!(chips(&b.rows, LABEL_BANS)[2].tooltip(), "Ban 3: not read");
}

/// The fallback hook: the game-level flag draws "Reads may be off" only
/// when no chip is marked; per-read flags win.
#[test]
fn the_weak_hook_is_a_fallback_pill() {
    let mut b = block_of(&game(NORMAL));
    b.weak = true;
    assert_eq!(b.pill(), Some(Pill::Weak));
    assert_eq!(Pill::Weak.words(), "Reads may be off");
    assert_eq!(
        Pill::Weak.tip(),
        "The reader wasn't sure about this game's screens. Check the names. Use Report misread if one is wrong."
    );
    assert!(!b.legend().unsure);
    mark(&mut b, LABEL_HERO_SELECT, 0);
    assert_eq!(b.pill(), Some(Pill::Unsure(1)));
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
