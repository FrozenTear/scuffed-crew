//! Bans and picks for the `--fixture games` snapshot: example data for the
//! open row's block (UX Spec bans and picks a3b, on v2 and alpha.2.1). Hero
//! choices only, no member data.

use serde_json::{Value, json};
use stat_tracker::model::{BansPicksMissing, BansPicksStatus};
use stat_tracker::storage::PersonalMatch;

fn read(value: Option<&str>) -> Value {
    json!({ "value": value, "confidence": 0.93, "score": 0.81, "margin": 0.31 })
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
/// a slot that was not read (state null).
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
            json!({
                "voter": 0,
                "slot": i + 1,
                "state": state,
                "hero": hero,
                "confidence": 0.93,
                "score": 0.81,
                "margin": 0.31,
            })
        })
        .collect()
}

fn stored(
    bans: Vec<Value>,
    lobby: Option<&str>,
    picks: &[Option<&str>],
    votes: &[Option<&str>],
) -> String {
    json!({
        "layout": "open_queue",
        "bans": bans,
        "lobby_ban": read(lobby),
        "picks": reads(picks),
        // The flat list holds heroes only; it is legacy and never shown.
        "ban_vote": reads(&votes.iter().map(|v| v.filter(|h| !h.starts_with('<'))).collect::<Vec<_>>()),
        "ban_votes": vote_slots(votes),
        "bans_picks_recognizer": "cv-v1",
    })
    .to_string()
}

/// Everything read: bans on both sides, the lobby ban, our three ban
/// votes and six hero select slots. Our hero select pick is our hero, so no swap.
pub fn full(m: &mut PersonalMatch) {
    m.bans_picks = Some(stored(
        vec![
            ban(Some("sombra"), Some("own")),
            ban(Some("mauga"), Some("own")),
            ban(Some("ana"), Some("enemy")),
            ban(Some("winston"), Some("enemy")),
        ],
        Some("sigma"),
        &[
            Some("wrecking-ball"),
            Some("tracer"),
            Some("cassidy"),
            Some("kiriko"),
            Some("lucio"),
            Some("mercy"),
        ],
        &[Some("reinhardt"), Some("tracer"), Some("genji")],
    ));
    m.bans_picks_status = Some(BansPicksStatus::Ok);
}

/// Read in part: the side of the bans is unknown (hero select was not
/// read for it), one hero select slot is No preference and one is not
/// read, one ban vote is not read, and we picked Reinhardt but played
/// Wrecking Ball (the swap).
pub fn partial(m: &mut PersonalMatch) {
    m.bans_picks = Some(stored(
        vec![
            ban(Some("sombra"), None),
            ban(Some("mauga"), None),
            ban(None, None),
            ban(Some("winston"), None),
        ],
        None,
        &[
            Some("reinhardt"),
            Some("tracer"),
            Some("no_preference"),
            None,
            Some("lucio"),
            Some("mercy"),
        ],
        &[Some("reinhardt"), None, Some("genji")],
    ));
    m.bans_picks_status = Some(BansPicksStatus::Partial {
        missing: Vec::new(),
    });
}

/// Samoa: every ban read, but not which team made it, and hero select was
/// never seen (`Partial` with `missing: [hero_select]`).
pub fn side(m: &mut PersonalMatch) {
    m.bans_picks = Some(stored(
        vec![
            ban(Some("sombra"), None),
            ban(Some("mauga"), None),
            ban(Some("ana"), None),
            ban(Some("winston"), None),
        ],
        None,
        &[],
        &[Some("reinhardt"), Some("tracer"), Some(SKIPPED)],
    ));
    m.bans_picks_status = Some(BansPicksStatus::Partial {
        missing: vec![BansPicksMissing::HeroSelect],
    });
}

/// Gibraltar: no ban screen and no ban vote seen, hero select read.
pub fn no_ban_screen(m: &mut PersonalMatch) {
    m.bans_picks = Some(stored(
        Vec::new(),
        None,
        &[
            Some("wrecking-ball"),
            Some("tracer"),
            Some("cassidy"),
            Some("kiriko"),
            Some("lucio"),
            Some("mercy"),
        ],
        &[],
    ));
    m.bans_picks_status = Some(BansPicksStatus::Partial {
        missing: vec![BansPicksMissing::BanResults, BansPicksMissing::BanVote],
    });
}

/// Bans of both teams, no lobby ban, and our votes: one hero, one skipped,
/// one empty (Not seen).
pub fn skip_votes(m: &mut PersonalMatch) {
    m.bans_picks = Some(stored(
        vec![
            ban(Some("sombra"), Some("own")),
            ban(Some("mauga"), Some("own")),
            ban(Some("ana"), Some("enemy")),
            ban(Some("winston"), Some("enemy")),
        ],
        None,
        &[
            Some("wrecking-ball"),
            Some("tracer"),
            Some("cassidy"),
            Some("kiriko"),
            Some("lucio"),
            Some("mercy"),
        ],
        &[Some("reinhardt"), Some(SKIPPED), Some(EMPTY)],
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
