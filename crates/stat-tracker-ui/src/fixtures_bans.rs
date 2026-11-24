//! Bans and picks for the `--fixture games` snapshot: example data for the
//! open row's block (UX Spec bans and picks a3b, on v2 and alpha.2.1). Hero
//! choices only, no member data.
//!
//! The JSON is built the way the cv-v1 daemon stores it
//! (`crates/stat-tracker/src/daemon/bans_picks.rs`, `Stored`): a value is
//! only stored when its read clears the accept bar (score 0.45, margin
//! 0.12), so a null value has confidence 0 and a score or margin below the
//! bar; `layout` is `plain` or `lobby` (null when the ban screen was not
//! seen), `lobby_ban` is null outside the lobby layout; and the status is
//! `Partial` only with a screen in `missing`.
//!
//! In these fixtures a vote slot with no hero state (skipped, empty or
//! unclear) has score and margin 0, and `ban_votes` and `ban_score` are
//! both null when the vote screen was not seen. The daemon is looser: a
//! slot showing the no-preference icon has state null but keeps its match
//! score and margin, and it reads the slots and the strip apart, so either
//! list can be null alone.

use serde_json::{Value, json};
use stat_tracker::model::{BansPicksMissing, BansPicksStatus};
use stat_tracker::storage::PersonalMatch;

/// A position read: a value above the bar, or a null one below it.
fn read(value: Option<&str>) -> Value {
    match value {
        Some(v) => json!({ "value": v, "confidence": 0.93, "score": 0.81, "margin": 0.31 }),
        None => json!({ "value": null, "confidence": 0.0, "score": 0.41, "margin": 0.05 }),
    }
}

fn ban(value: Option<&str>, team: Option<&str>) -> Value {
    let mut v = read(value);
    v["team"] = json!(team);
    v
}

fn reads(values: &[Option<&str>]) -> Vec<Value> {
    values.iter().map(|v| read(*v)).collect()
}

/// A vote slot value that stands for the skip icon, not a hero.
const SKIPPED: &str = "<skipped>";
/// A vote slot value that stands for an empty slot (Not seen), not a hero.
const EMPTY: &str = "<empty>";

/// Our own vote slots as the tracker stores them in `ban_votes`: a hero
/// when one is given, [`SKIPPED`] or [`EMPTY`] for those states, otherwise
/// an unclear slot (state null). Only a hero slot ran a match here, so
/// only it has a score (no slot here shows the no-preference icon).
fn vote_slots(values: &[Option<&str>]) -> Vec<Value> {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let (state, hero) = match *v {
                Some(SKIPPED) => (Some("skipped"), None),
                Some(EMPTY) => (Some("empty"), None),
                Some(hero) => (Some("hero"), Some(hero)),
                None => (None, None),
            };
            let (confidence, score, margin) = if hero.is_some() {
                (0.9, 0.78, 0.3)
            } else {
                (0.0, 0.0, 0.0)
            };
            json!({
                "voter": 0,
                "slot": i + 1,
                "state": state,
                "hero": hero,
                "confidence": confidence,
                "score": score,
                "margin": margin,
            })
        })
        .collect()
}

/// The flat `ban_vote` list the daemon also writes: one read per own slot,
/// the hero or an unread value (skipped, empty or unclear).
fn flat_votes(values: &[Option<&str>]) -> Vec<Value> {
    values
        .iter()
        .map(|v| match v.filter(|h| !h.starts_with('<')) {
            Some(hero) => read(Some(hero)),
            None => json!({ "value": null, "confidence": 0.0, "score": 0.0, "margin": 0.0 }),
        })
        .collect()
}

/// The TEAM BAN SCORE strip: the team's three most voted heroes and their
/// counts. Never our own votes, and never drawn by the block.
fn ban_score(strip: &[(&str, u32)]) -> Value {
    strip
        .iter()
        .enumerate()
        .map(|(i, (hero, votes))| {
            json!({
                "rank": i + 1,
                "hero": hero,
                "votes": votes,
                "empty": false,
                "confidence": 0.88,
                "votes_confidence": 0.95,
                "score": 0.74,
                "margin": 0.33,
            })
        })
        .collect()
}

/// The ban results screen as stored.
enum Results {
    /// Not seen: `layout` null, no bans, `lobby_ban` null.
    NotSeen,
    /// Four bans, no lobby slot.
    Plain(Vec<Value>),
    /// Four bans and the lobby slot (`None`: below the bar).
    Lobby(Vec<Value>, Option<&'static str>),
}

/// The vote screen as stored: our three slots and the strip, or not seen.
type Votes<'a> = Option<(&'a [Option<&'a str>], &'a [(&'a str, u32)])>;

fn stored(results: Results, picks: &[Option<&str>], votes: Votes<'_>) -> String {
    let (layout, bans, lobby_ban) = match results {
        Results::NotSeen => (Value::Null, Vec::new(), Value::Null),
        Results::Plain(bans) => (json!("plain"), bans, Value::Null),
        Results::Lobby(bans, lobby) => (json!("lobby"), bans, read(lobby)),
    };
    let (ban_vote, ban_votes, strip) = match votes {
        Some((own, strip)) => (flat_votes(own), json!(vote_slots(own)), ban_score(strip)),
        None => (Vec::new(), Value::Null, Value::Null),
    };
    json!({
        "layout": layout,
        "bans": bans,
        "lobby_ban": lobby_ban,
        "picks": reads(picks),
        // The flat list is legacy and never shown.
        "ban_vote": ban_vote,
        "ban_votes": ban_votes,
        "ban_score": strip,
        "bans_picks_recognizer": "cv-v1",
    })
    .to_string()
}

const SIX: [Option<&str>; 6] = [
    Some("wrecking-ball"),
    Some("tracer"),
    Some("cassidy"),
    Some("kiriko"),
    Some("lucio"),
    Some("mercy"),
];

/// The team's strip in the full game. Widowmaker, Pharah and Zarya are in
/// no other part of that game, so a test can see the strip is never drawn.
pub const FULL_STRIP: [(&str, u32); 3] = [("widowmaker", 3), ("pharah", 2), ("zarya", 1)];

/// Everything read: bans on both sides, the lobby ban, our three ban
/// votes and six hero select slots. Our hero select pick is our hero, so no swap.
pub fn full(m: &mut PersonalMatch) {
    m.bans_picks = Some(stored(
        Results::Lobby(
            vec![
                ban(Some("sombra"), Some("own")),
                ban(Some("mauga"), Some("own")),
                ban(Some("ana"), Some("enemy")),
                ban(Some("winston"), Some("enemy")),
            ],
            Some("sigma"),
        ),
        &SIX,
        Some((
            &[Some("reinhardt"), Some("tracer"), Some("genji")],
            &FULL_STRIP,
        )),
    ));
    m.bans_picks_status = Some(BansPicksStatus::Ok);
}

/// Read in part, every screen seen: the side of the bans is unknown, one
/// ban is below the bar, one hero select slot is No preference and one is
/// below the bar, one ban vote is unclear, and we picked Reinhardt but
/// played Wrecking Ball (the swap).
pub fn partial(m: &mut PersonalMatch) {
    m.bans_picks = Some(stored(
        Results::Plain(vec![
            ban(Some("sombra"), None),
            ban(Some("mauga"), None),
            ban(None, None),
            ban(Some("winston"), None),
        ]),
        &[
            Some("reinhardt"),
            Some("tracer"),
            Some("no_preference"),
            None,
            Some("lucio"),
            Some("mercy"),
        ],
        Some((
            &[Some("reinhardt"), None, Some("genji")],
            &[("reinhardt", 2), ("genji", 1), ("mei", 1)],
        )),
    ));
    m.bans_picks_status = Some(BansPicksStatus::Ok);
}

/// Nepal: every ban read, but not which team made it, and hero select was
/// never seen (`Partial` with `missing: [hero_select]`).
pub fn side(m: &mut PersonalMatch) {
    m.bans_picks = Some(stored(
        Results::Plain(vec![
            ban(Some("sombra"), None),
            ban(Some("mauga"), None),
            ban(Some("ana"), None),
            ban(Some("winston"), None),
        ]),
        &[],
        Some((
            &[Some("reinhardt"), Some("tracer"), Some(SKIPPED)],
            &[("reinhardt", 2), ("tracer", 1), ("mei", 1)],
        )),
    ));
    m.bans_picks_status = Some(BansPicksStatus::Partial {
        missing: vec![BansPicksMissing::HeroSelect],
    });
}

/// Lijiang Tower: no ban screen and no ban vote seen, hero select read.
pub fn no_ban_screen(m: &mut PersonalMatch) {
    m.bans_picks = Some(stored(Results::NotSeen, &SIX, None));
    m.bans_picks_status = Some(BansPicksStatus::Partial {
        missing: vec![BansPicksMissing::BanResults, BansPicksMissing::BanVote],
    });
}

/// Bans of both teams, no lobby ban, and our votes: one hero, one skipped,
/// one empty (Not seen).
pub fn skip_votes(m: &mut PersonalMatch) {
    m.bans_picks = Some(stored(
        Results::Plain(vec![
            ban(Some("sombra"), Some("own")),
            ban(Some("mauga"), Some("own")),
            ban(Some("ana"), Some("enemy")),
            ban(Some("winston"), Some("enemy")),
        ]),
        &SIX,
        Some((
            &[Some("reinhardt"), Some(SKIPPED), Some(EMPTY)],
            &[("reinhardt", 3), ("ana", 2), ("mei", 1)],
        )),
    ));
    m.bans_picks_status = Some(BansPicksStatus::Ok);
}

/// Junkertown: the lobby layout with its lobby ban read and none of the
/// four ban slots read (all below the bar). The block draws four Not read
/// chips, then the Lobby group.
pub fn lobby_only(m: &mut PersonalMatch) {
    m.bans_picks = Some(stored(
        Results::Lobby(
            vec![
                ban(None, Some("own")),
                ban(None, Some("own")),
                ban(None, Some("enemy")),
                ban(None, Some("enemy")),
            ],
            Some("sigma"),
        ),
        &SIX,
        Some((
            &[Some("reinhardt"), Some("tracer"), Some("genji")],
            &[("reinhardt", 2), ("tracer", 2), ("genji", 1)],
        )),
    ));
    m.bans_picks_status = Some(BansPicksStatus::Ok);
}

/// King's Row: the widest Bans row. Long names in both team groups, one
/// ban read with no team (so it goes under Team not read), and the lobby
/// ban: `Your team | Their team | Team not read | Lobby`, which wraps at
/// 1024.
pub fn mixed(m: &mut PersonalMatch) {
    m.bans_picks = Some(stored(
        Results::Lobby(
            vec![
                ban(Some("lifeweaver"), Some("own")),
                ban(Some("junker-queen"), Some("own")),
                ban(Some("soldier-76"), Some("enemy")),
                ban(Some("widowmaker"), None),
            ],
            Some("ramattra"),
        ),
        &SIX,
        Some((
            &[Some("reinhardt"), Some("tracer"), Some("genji")],
            &[("reinhardt", 2), ("tracer", 2), ("genji", 1)],
        )),
    ));
    m.bans_picks_status = Some(BansPicksStatus::Ok);
}

/// Nothing read, the reader was off: one quiet line and "Open Settings".
pub fn reader_off(m: &mut PersonalMatch) {
    m.bans_picks = None;
    m.bans_picks_status = Some(BansPicksStatus::ReaderOff);
}

/// Nothing read, the game ran at 1080p.
pub fn frame_size(m: &mut PersonalMatch) {
    m.bans_picks = None;
    m.bans_picks_status = Some(BansPicksStatus::FrameSize {
        width: 1920,
        height: 1080,
    });
}
