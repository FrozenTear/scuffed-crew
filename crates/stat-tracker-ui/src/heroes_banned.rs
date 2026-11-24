//! The Heroes page's When banned mode (issue 779, UX Spec
//! `ban-winrate.md`), as a model: one row per hero that was banned in a
//! game, with FrozenTear's record in those games. No drawing here; that is
//! `heroes_banned_view`.
//!
//! What counts (spec section 2):
//!
//! - The four ban slots of the stored result, read leniently by
//!   [`crate::games_bans::parse`], from a known reader only (cv-v1 or no
//!   tag). A slot counts when its hero is a known pack hero
//!   ([`scuffed_types::hero_key_to_name`]); an unread or unknown one never
//!   gets a row.
//! - The lobby ban, when it is a known hero and not one of the counted
//!   slots. It has no team, so it adds to Games and the record only.
//! - Never the ban votes (`ban_vote`, `ban_votes`): votes, not results.
//!
//! A game counts once per hero, whatever the number of slots, and once on
//! each side that banned it (Your ban, Enemy). A ban whose team was not
//! read adds to "Side not read" unless a read side already has that hero.
//! A game with no counted ban is left out, and counted as "no ban read".
//! Nothing here can drop a game from any other view: it only reads.
//!
//! The rate follows [`crate::small_sample`]: decided is wins, losses and
//! draws, the rate needs [`RATE_MIN`] decided games, undecided games count
//! in Games only. No "Other" fold: a rare ban still gets its row.

use chrono::{DateTime, Utc};
use scuffed_types::HeroRole;

use crate::aggregate::Record;
use crate::games_bans::{Lobby, Slot, Team};
use crate::model::{Game, Outcome, Role};
use crate::small_sample::{RATE_MIN, TOO_FEW, decided, rate};

/// Played (today's cards or list) or When banned (this table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Played,
    WhenBanned,
}

/// What the table sorts by. The segment offers the first three, the
/// column headers the rest too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    Games,
    WinRate,
    Recent,
    Hero,
    Decided,
}

impl SortKey {
    /// Names A to Z first; every other key starts high to low.
    fn starts_ascending(self) -> bool {
        self == SortKey::Hero
    }
}

/// The table's sort, kept for the session like Cards | List.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BanUi {
    pub sort: SortKey,
    pub ascending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    /// The Games, Win rate, Recent segment: that key, its usual direction.
    Sort(SortKey),
    /// A column header: that key, or the other direction when it is the
    /// key already.
    Header(SortKey),
}

pub fn update(ui: &mut BanUi, msg: Msg) {
    match msg {
        Msg::Header(key) if key == ui.sort => ui.ascending = !ui.ascending,
        Msg::Sort(key) | Msg::Header(key) => {
            ui.sort = key;
            ui.ascending = key.starts_ascending();
        }
    }
}

/// Who banned a hero in one game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sides {
    pub own: bool,
    pub enemy: bool,
    /// A slot whose team was not read, with no read side for this hero.
    pub unread: bool,
}

/// The heroes counted as banned in one game, each once, by display name,
/// in slot order with a distinct lobby ban last. Empty for a game with no
/// stored result, one that does not parse, one from another reader, or
/// one with no known hero in a ban slot.
pub fn game_bans(g: &Game) -> Vec<(&'static str, Sides)> {
    let Some(stored) = g.bans_picks.as_deref().and_then(crate::games_bans::parse) else {
        return Vec::new();
    };
    if !stored.known_reader {
        return Vec::new();
    }
    let mut out: Vec<(&'static str, Sides)> = Vec::new();
    for ban in &stored.bans {
        let Some(hero) = known_hero(&ban.slot) else {
            continue;
        };
        let at = match out.iter().position(|(h, _)| *h == hero) {
            Some(i) => i,
            None => {
                out.push((hero, Sides::default()));
                out.len() - 1
            }
        };
        let sides = &mut out[at].1;
        match ban.team {
            Some(Team::Own) => sides.own = true,
            Some(Team::Enemy) => sides.enemy = true,
            None => sides.unread = true,
        }
    }
    for (_, sides) in &mut out {
        // A read side already places the hero; the unread slot adds nothing.
        sides.unread &= !(sides.own || sides.enemy);
    }
    if let Lobby::Hero(name) = &stored.lobby
        && let Some(hero) = scuffed_types::hero_key_to_name(name)
        && !out.iter().any(|(h, _)| *h == hero)
    {
        out.push((hero, Sides::default()));
    }
    out
}

/// The display name of a slot read as a known hero.
fn known_hero(slot: &Slot) -> Option<&'static str> {
    match slot {
        Slot::Hero(name) => scuffed_types::hero_key_to_name(name),
        _ => None,
    }
}

/// The banned hero's own role, from the hero list.
pub fn hero_role(hero: &str) -> Role {
    match scuffed_types::role_for_hero_name(hero) {
        Some(HeroRole::Tank) => Role::Tank,
        Some(HeroRole::Damage) => Role::Damage,
        Some(HeroRole::Support) => Role::Support,
        None => Role::Unknown,
    }
}

/// One banned hero in the table.
#[derive(Debug, Clone, PartialEq)]
pub struct BanRow {
    /// The display name.
    pub hero: &'static str,
    /// The banned hero's role, not the one FrozenTear played.
    pub role: Role,
    /// Games where it was banned, and FrozenTear's result in them.
    pub record: Record,
    /// Games where our own team banned it.
    pub own: usize,
    /// Games where the enemy team banned it.
    pub enemy: usize,
    /// Games where only a ban with no team read named it.
    pub side_not_read: usize,
    /// The newest game where it was banned.
    pub last: DateTime<Utc>,
}

impl BanRow {
    /// Wins, losses and draws.
    pub fn decided(&self) -> usize {
        decided(&self.record)
    }

    /// The whole percent, from [`RATE_MIN`] decided games.
    pub fn rate(&self) -> Option<i64> {
        rate(&self.record)
    }

    /// "Too few games (8 of 10)" in place of the rate, or `None` when the
    /// hero has one.
    pub fn too_few(&self) -> Option<String> {
        self.rate()
            .is_none()
            .then(|| format!("{TOO_FEW} ({} of {RATE_MIN})", self.decided()))
    }
}

/// The whole table for a set of games.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BanTable {
    pub rows: Vec<BanRow>,
    /// Games with at least one counted ban.
    pub with_ban: usize,
    /// Games left out: no counted ban.
    pub no_ban: usize,
}

/// Count `games` (already filtered, the open game already left out).
/// Rows come in no set order; see [`sort_rows`].
pub fn aggregate(games: &[&Game]) -> BanTable {
    let mut t = BanTable::default();
    for g in games {
        let bans = game_bans(g);
        if bans.is_empty() {
            t.no_ban += 1;
            continue;
        }
        t.with_ban += 1;
        for (hero, sides) in bans {
            let at = match t.rows.iter().position(|r| r.hero == hero) {
                Some(i) => i,
                None => {
                    t.rows.push(BanRow {
                        hero,
                        role: hero_role(hero),
                        record: Record {
                            games: 0,
                            wins: 0,
                            losses: 0,
                            draws: 0,
                        },
                        own: 0,
                        enemy: 0,
                        side_not_read: 0,
                        last: g.played_at,
                    });
                    t.rows.len() - 1
                }
            };
            let r = &mut t.rows[at];
            r.record.games += 1;
            match g.outcome {
                Outcome::Win => r.record.wins += 1,
                Outcome::Loss => r.record.losses += 1,
                Outcome::Draw => r.record.draws += 1,
                Outcome::Unknown => {}
            }
            r.own += usize::from(sides.own);
            r.enemy += usize::from(sides.enemy);
            r.side_not_read += usize::from(sides.unread);
            r.last = r.last.max(g.played_at);
        }
    }
    t
}

/// Sort `rows` by `key`. Ties go to more decided games, then the name A
/// to Z, in either direction. By win rate the heroes without a rate always
/// come after the rated ones, ascending or descending.
pub fn sort_rows(rows: &mut [BanRow], key: SortKey, ascending: bool) {
    use std::cmp::Ordering;
    let tie = |a: &BanRow, b: &BanRow| {
        b.decided()
            .cmp(&a.decided())
            .then_with(|| a.hero.to_lowercase().cmp(&b.hero.to_lowercase()))
    };
    let dir = |o: Ordering| if ascending { o } else { o.reverse() };
    rows.sort_by(|a, b| match key {
        SortKey::Games => dir(a.record.games.cmp(&b.record.games)).then_with(|| tie(a, b)),
        SortKey::Decided => dir(a.decided().cmp(&b.decided())).then_with(|| tie(a, b)),
        SortKey::Recent => dir(a.last.cmp(&b.last)).then_with(|| tie(a, b)),
        SortKey::Hero => dir(a.hero.to_lowercase().cmp(&b.hero.to_lowercase())),
        SortKey::WinRate => match (a.rate(), b.rate()) {
            (Some(x), Some(y)) => dir(x.cmp(&y)).then_with(|| tie(a, b)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => tie(a, b),
        },
    });
}

/// The past-results caption, always shown in When banned.
pub const CAPTION: &str =
    "Past results when this hero was banned in the game, not a suggestion of who to ban.";
/// The sentence added when the bans and picks reader is off.
pub const READER_OFF: &str = "The bans and picks reader is off in Settings, Capture and reading.";

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// "All time · 8 heroes · 212 games with a ban read · 48 games with no ban
/// read left out", on one line. The left-out part is dropped at 0.
pub fn stats_line(scope: &str, t: &BanTable) -> String {
    let mut line = format!(
        "{scope} · {} · {}",
        count(t.rows.len(), "hero", "heroes"),
        count(t.with_ban, "game with a ban read", "games with a ban read"),
    );
    if t.no_ban > 0 {
        line.push_str(&format!(
            " · {} left out",
            count(t.no_ban, "game with no ban read", "games with no ban read")
        ));
    }
    line
}

/// The card in place of the table, `None` when there are rows: no games
/// at all (`has_games` false), filters that leave none (`shown` 0), or
/// `shown` games with no ban read. `reader_on` is the Read bans and picks
/// switch as the tracker runs it.
pub fn empty_copy(
    has_games: bool,
    shown: usize,
    t: &BanTable,
    reader_on: bool,
) -> Option<(&'static str, String)> {
    if !has_games {
        Some((
            "No bans yet",
            "They show here once the tracker saves a game with a ban read.".into(),
        ))
    } else if shown == 0 {
        Some((
            "No games match these filters",
            "Clear a chip above to see when a hero was banned.".into(),
        ))
    } else if t.rows.is_empty() {
        let mut line = if t.no_ban == 1 {
            "1 game in this view has no ban read. It is left out of this table.".to_string()
        } else {
            format!(
                "{} games in this view have no ban read. They are left out of this table.",
                t.no_ban
            )
        };
        if !reader_on {
            line.push(' ');
            line.push_str(READER_OFF);
        }
        Some(("No bans read yet", line))
    } else {
        None
    }
}

#[cfg(test)]
#[path = "heroes_banned_tests.rs"]
mod tests;
