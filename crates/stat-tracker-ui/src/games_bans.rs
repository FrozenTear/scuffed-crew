//! The open Games row's "Bans and picks" block, as a view model: which rows
//! the block has, which chips are in them and what each says. The drawing is
//! in `games_bans_view`. The spec is the UX Spec bans and picks v2 with the
//! alpha.2.1 addendum (`redesign/ux-spec/bans-picks.md`).
//!
//! The data is the game's stored cv-v1 JSON (`Game::bans_picks`) and its
//! status (`Game::bans_picks_status`). The JSON is read leniently: a value
//! that is not a string, an empty one and a missing list are all "not read",
//! and a stored result that does not parse is no result. Nothing here ever
//! hides the game, only the block.
//!
//! Not drawn yet, because the data does not exist yet: the "might be
//! misread" state (the tracker stores no suspect paths for bans or picks),
//! and the Their team Played row (the tracker stores only our own hero).

use scuffed_types::{
    BANS_PICKS_MAX_BAN_VOTE, BANS_PICKS_MAX_BANS, BANS_PICKS_MAX_PICKS, BANS_PICKS_NO_PREFERENCE,
};
use serde_json::Value;
use stat_tracker::model::BansPicksStatus;

use crate::model::Game;
use crate::settings_model::AdvancedSection;

/// The reader's template spelling of the "no preferred hero" icon
/// ([`BANS_PICKS_NO_PREFERENCE`] is the stored spelling).
const NO_PREFERENCE_TEMPLATE: &str = "no-preference";

pub const HEADER: &str = "Bans and picks";
pub const TEAM_NOT_READ: &str = "Team not read";

/// What one slot of the screen read as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    /// A hero, by display name.
    Hero(String),
    /// The "no preferred hero" icon.
    NoPreference,
    /// The slot could not be read.
    NotRead,
}

/// One chip: a slot, with the words every chip carries (visible name,
/// tooltip) and the flags that pick its look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub slot: Slot,
    /// A ban or the lobby ban: ban glyph, no fill.
    pub ban: bool,
    /// Our own chip: the "You" tag.
    pub you: bool,
    /// Our own Played chip when it differs from our hero select hero.
    pub swap: bool,
    /// The tooltip, which is also the spoken label of the chip.
    pub tip: String,
}

impl Chip {
    /// The visible words: the hero name, "No preference" or "Not read".
    pub fn name(&self) -> &str {
        match &self.slot {
            Slot::Hero(name) => name,
            Slot::NoPreference => "No preference",
            Slot::NotRead => "Not read",
        }
    }
}

/// What a row holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Chips(Vec<Chip>),
    /// One quiet line, in place of chips that were not read.
    Quiet(String),
}

/// One row of the block: a label (team, and a line under it) and its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// "Your team", "Their bans", "Bans" ...
    pub team: &'static str,
    /// "Ban votes", "Hero select", "Played".
    pub line: Option<&'static str>,
    /// A muted note next to the label ("Team not read").
    pub note: Option<&'static str>,
    pub body: Body,
}

/// What the open row shows for bans and picks.
/// Where "Open Settings" goes and what its tooltip says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsLink {
    pub tip: &'static str,
    /// The Advanced row the button opens.
    pub open: AdvancedSection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BansPicks {
    /// Nothing was read and no reason is stored: no block.
    Hidden,
    /// Nothing was read and the game says why: one quiet line.
    Reason {
        line: String,
        /// The line's tooltip, when the spec gives one.
        tip: Option<&'static str>,
        /// "Open Settings" goes with the line.
        settings: Option<SettingsLink>,
    },
    Block(Vec<Row>),
}

impl BansPicks {
    /// Whether any chip in the block is a ban.
    pub fn has_ban(&self) -> bool {
        self.chips().any(|c| c.ban)
    }

    /// Whether any chip in the block is a swap chip.
    pub fn has_swap(&self) -> bool {
        self.chips().any(|c| c.swap)
    }

    /// Whether any chip in the block was not read.
    pub fn has_not_read(&self) -> bool {
        self.chips().any(|c| c.slot == Slot::NotRead)
    }

    fn chips(&self) -> impl Iterator<Item = &Chip> {
        let rows: &[Row] = match self {
            BansPicks::Block(rows) => rows,
            _ => &[],
        };
        rows.iter().flat_map(|r| match &r.body {
            Body::Chips(chips) => chips.as_slice(),
            Body::Quiet(_) => &[],
        })
    }
}

/// The block for one game.
pub fn bans_picks(g: &Game) -> BansPicks {
    if let Some(stored) = g.bans_picks.as_deref().and_then(parse)
        && stored.read_something()
    {
        return BansPicks::Block(stored.rows(&g.hero));
    }
    match g.bans_picks_status.as_ref().and_then(reason) {
        Some((line, settings)) => BansPicks::Reason {
            line,
            tip: reason_tip(g.bans_picks_status.as_ref()),
            settings: settings.then(|| settings_link(g.bans_picks_status.as_ref())),
        },
        None => BansPicks::Hidden,
    }
}

/// The quiet line for a game that read nothing, and whether "Open Settings"
/// goes with it. `None` for a status that read something (a game that read
/// whole or in part, but whose JSON holds nothing: no line, as no spec text
/// covers it).
fn reason(status: &BansPicksStatus) -> Option<(String, bool)> {
    let (line, settings) = match status {
        BansPicksStatus::Ok | BansPicksStatus::Partial { .. } => return None,
        BansPicksStatus::NoFrames => (
            "Not read: the tracker didn't see the pre-game screens for this game.".to_string(),
            false,
        ),
        BansPicksStatus::FrameSize { width, height } if *width > 0 && *height > 0 => (
            format!("Not read: this game ran at {width}x{height}. Bans and picks need 2560x1440."),
            false,
        ),
        BansPicksStatus::FrameSize { .. } => (
            "Not read: bans and picks need a 2560x1440 game.".to_string(),
            false,
        ),
        BansPicksStatus::PackMissing { linked: false } => (
            "Not read: the hero icon pack isn't installed. Link a site account in Settings and the tracker downloads it."
                .to_string(),
            true,
        ),
        BansPicksStatus::PackMissing { linked: true } => (
            "Not read: the hero icon pack isn't installed yet. The tracker fetches it in the background."
                .to_string(),
            false,
        ),
        BansPicksStatus::PackFailed => (
            "Not read: the hero icon pack didn't load. Reinstall it, or turn off Read bans and picks in Settings."
                .to_string(),
            true,
        ),
        BansPicksStatus::ReaderFailed => (
            "Not read: the reader failed on this game's screens.".to_string(),
            false,
        ),
        BansPicksStatus::ReaderOff => (
            "Not read: the Read bans and picks setting is off.".to_string(),
            true,
        ),
    };
    Some((line, settings))
}

/// The "Open Settings" button: its tooltip and the row it opens. A missing
/// pack is fixed by linking an account (Website connection), everything else
/// by the Read bans and picks switch (Troubleshooting).
fn settings_link(status: Option<&BansPicksStatus>) -> SettingsLink {
    match status {
        Some(BansPicksStatus::PackMissing { .. }) => SettingsLink {
            tip: "Settings, website connection. Linking an account lets the tracker download the pack.",
            open: AdvancedSection::WebsiteConnection,
        },
        _ => SettingsLink {
            tip: "Settings, Troubleshooting, Read bans and picks. It takes effect after a restart. Games already played stay unread.",
            open: AdvancedSection::Troubleshooting,
        },
    }
}

/// The tooltip of a reason line, when the spec gives one.
fn reason_tip(status: Option<&BansPicksStatus>) -> Option<&'static str> {
    matches!(status, Some(BansPicksStatus::FrameSize { .. }))
        .then_some("Only 2560x1440 is read. 1080p and 4K are not.")
}

/// One ban as stored.
struct Ban {
    slot: Slot,
    team: Option<Team>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Team {
    Own,
    Enemy,
}

/// The stored JSON, as plain slots.
struct Stored {
    bans: Vec<Ban>,
    lobby_ban: Option<String>,
    picks: Vec<Slot>,
    ban_vote: Vec<Slot>,
}

fn parse(json: &str) -> Option<Stored> {
    let v: Value = serde_json::from_str(json).ok()?;
    if !v.is_object() {
        return None;
    }
    let list = |key: &str, max: usize| -> Vec<Value> {
        v.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().take(max).cloned().collect())
            .unwrap_or_default()
    };
    Some(Stored {
        bans: list("bans", BANS_PICKS_MAX_BANS)
            .iter()
            .map(|e| Ban {
                // A ban is a hero or nothing; no preference there is not read.
                slot: match slot(e) {
                    Slot::NoPreference => Slot::NotRead,
                    other => other,
                },
                team: match e.get("team").and_then(Value::as_str) {
                    Some("own") => Some(Team::Own),
                    Some("enemy") => Some(Team::Enemy),
                    _ => None,
                },
            })
            .collect(),
        lobby_ban: match v.get("lobby_ban").map(slot) {
            Some(Slot::Hero(name)) => Some(name),
            _ => None,
        },
        picks: list("picks", BANS_PICKS_MAX_PICKS)
            .iter()
            .map(slot)
            .collect(),
        ban_vote: list("ban_vote", BANS_PICKS_MAX_BAN_VOTE)
            .iter()
            .map(slot)
            .collect(),
    })
}

/// A stored entry: `{value, ..}` or a plain string. A value that is a hero
/// key becomes the hero's display name.
fn slot(entry: &Value) -> Slot {
    let raw = match entry {
        Value::Object(read) => read.get("value").and_then(Value::as_str),
        Value::String(plain) => Some(plain.as_str()),
        _ => None,
    };
    match raw.map(str::trim).filter(|r| !r.is_empty()) {
        None => Slot::NotRead,
        Some(BANS_PICKS_NO_PREFERENCE | NO_PREFERENCE_TEMPLATE) => Slot::NoPreference,
        Some(key) => Slot::Hero(hero_name(key)),
    }
}

fn hero_name(key: &str) -> String {
    scuffed_types::hero_key_to_name(key).map_or_else(
        || key.replace(['_', '-'], " "),
        std::string::ToString::to_string,
    )
}

fn read(slots: &[Slot]) -> bool {
    slots.iter().any(|s| *s != Slot::NotRead)
}

impl Stored {
    fn bans_read(&self) -> bool {
        self.bans.iter().any(|b| matches!(b.slot, Slot::Hero(_)))
    }

    /// The block is drawn when a ban, the lobby ban, a hero select slot or
    /// a ban vote slot was read (a hero, or No preference).
    fn read_something(&self) -> bool {
        self.bans_read() || self.lobby_ban.is_some() || read(&self.picks) || read(&self.ban_vote)
    }

    fn rows(&self, own_hero: &str) -> Vec<Row> {
        let mut rows = self.ban_rows();
        if let Some(name) = &self.lobby_ban {
            rows.push(Row {
                team: "Lobby ban",
                line: None,
                note: None,
                body: Body::Chips(vec![ban_chip(name, format!("Lobby ban: {name}"))]),
            });
        }
        let votes = read(&self.ban_vote);
        let picks = read(&self.picks);
        match (votes, picks) {
            (false, false) => rows.push(quiet(
                "Your team",
                None,
                "Ban votes and hero select weren't read for this game.",
            )),
            (votes, picks) => {
                rows.push(if votes {
                    chips("Your team", "Ban votes", vote_chips(&self.ban_vote))
                } else {
                    quiet(
                        "Your team",
                        Some("Ban votes"),
                        "Ban votes weren't read for this game.",
                    )
                });
                rows.push(if picks {
                    chips("Your team", "Hero select", pick_chips(&self.picks))
                } else {
                    quiet(
                        "Your team",
                        Some("Hero select"),
                        "Hero select wasn't read for this game.",
                    )
                });
            }
        }
        if let Some(played) = played_chip(own_hero, self.picks.first()) {
            rows.push(chips("Your team", "Played", vec![played]));
        }
        rows
    }

    /// The bans: split by team when every ban has a side, one row when any
    /// has none, one quiet line when no ban was read.
    fn ban_rows(&self) -> Vec<Row> {
        if !self.bans_read() {
            return vec![quiet("Bans", None, "Bans weren't read for this game.")];
        }
        if self.bans.iter().any(|b| b.team.is_none()) {
            let chips = self
                .bans
                .iter()
                .enumerate()
                .map(|(i, b)| match &b.slot {
                    Slot::Hero(name) => ban_chip(name, format!("Banned: {name}, team not read")),
                    _ => not_read_chip(true, format!("Ban {}: not read", i + 1)),
                })
                .collect();
            return vec![Row {
                team: "Bans",
                line: None,
                note: Some(TEAM_NOT_READ),
                body: Body::Chips(chips),
            }];
        }
        [
            (Team::Own, "Your team's bans", "Your team's ban"),
            (Team::Enemy, "Their bans", "Their ban"),
        ]
        .into_iter()
        .filter_map(|(team, label, nth)| {
            let chips: Vec<Chip> = self
                .bans
                .iter()
                .filter(|b| b.team == Some(team))
                .enumerate()
                .map(|(i, b)| match &b.slot {
                    Slot::Hero(name) => ban_chip(name, format!("Banned: {name}")),
                    _ => not_read_chip(true, format!("{nth} {}: not read", i + 1)),
                })
                .collect();
            (!chips.is_empty()).then_some(Row {
                team: label,
                line: None,
                note: None,
                body: Body::Chips(chips),
            })
        })
        .collect()
    }
}

fn chips(team: &'static str, line: &'static str, chips: Vec<Chip>) -> Row {
    Row {
        team,
        line: Some(line),
        note: None,
        body: Body::Chips(chips),
    }
}

fn quiet(team: &'static str, line: Option<&'static str>, text: &str) -> Row {
    Row {
        team,
        line,
        note: None,
        body: Body::Quiet(text.to_string()),
    }
}

fn ban_chip(name: &str, tip: String) -> Chip {
    Chip {
        slot: Slot::Hero(name.to_string()),
        ban: true,
        you: false,
        swap: false,
        tip,
    }
}

fn not_read_chip(ban: bool, tip: String) -> Chip {
    Chip {
        slot: Slot::NotRead,
        ban,
        you: false,
        swap: false,
        tip,
    }
}

/// Our own ban vote first (the local player is row 0), then teammates.
fn vote_chips(slots: &[Slot]) -> Vec<Chip> {
    slots
        .iter()
        .enumerate()
        .map(|(i, slot)| {
            let you = i == 0;
            let who = if you { "You" } else { "Teammate" };
            let tip = match slot {
                Slot::Hero(name) => format!("{who} voted to ban {name}"),
                Slot::NoPreference => format!("{who} voted no preference"),
                Slot::NotRead => format!("Ban vote slot {}: not read", i + 1),
            };
            Chip {
                slot: slot.clone(),
                ban: false,
                you,
                swap: false,
                tip,
            }
        })
        .collect()
}

/// Our own pick first (we are always row 0 at hero select).
fn pick_chips(slots: &[Slot]) -> Vec<Chip> {
    slots
        .iter()
        .enumerate()
        .map(|(i, slot)| {
            let you = i == 0;
            let tip = match slot {
                Slot::Hero(name) if you => format!("Shown at hero select: {name}, you"),
                Slot::Hero(name) => format!("Shown at hero select: {name}"),
                Slot::NoPreference => "Shown at hero select: no preference".to_string(),
                Slot::NotRead => format!("Hero select slot {}: not read", i + 1),
            };
            Chip {
                slot: slot.clone(),
                ban: false,
                you,
                swap: false,
                tip,
            }
        })
        .collect()
}

/// Our own Played chip, from the game data. `None` when the game has no
/// hero. It carries the swap glyph when it is a real hero that differs from
/// our own hero select hero, which must be a real hero too.
fn played_chip(own_hero: &str, own_pick: Option<&Slot>) -> Option<Chip> {
    let hero = own_hero.trim();
    if hero.is_empty() || hero.eq_ignore_ascii_case("unknown") {
        return None;
    }
    let name = hero_name(hero);
    let swapped_from = match own_pick {
        Some(Slot::Hero(picked)) if !picked.eq_ignore_ascii_case(&name) => Some(picked),
        _ => None,
    };
    let tip = match swapped_from {
        Some(picked) => format!("Played {name}, {picked} at hero select, you"),
        None => format!("Played {name}, you"),
    };
    Some(Chip {
        slot: Slot::Hero(name),
        ban: false,
        you: true,
        swap: swapped_from.is_some(),
        tip,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures_games::{HELD, NORMAL, REJECTED_2, UNDECIDED, UNSURE};
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

    fn rows(b: BansPicks) -> Vec<Row> {
        match b {
            BansPicks::Block(rows) => rows,
            other => panic!("not a block: {other:?}"),
        }
    }

    fn names(row: &Row) -> Vec<&str> {
        match &row.body {
            Body::Chips(chips) => chips.iter().map(Chip::name).collect(),
            Body::Quiet(_) => Vec::new(),
        }
    }

    #[test]
    fn a_full_read_has_every_row_in_order() {
        let b = bans_picks(&game(NORMAL));
        assert!(b.has_ban());
        assert!(!b.has_swap());
        assert!(!b.has_not_read());
        let rows = rows(b);
        let labels: Vec<_> = rows.iter().map(|r| (r.team, r.line)).collect();
        assert_eq!(
            labels,
            [
                ("Your team's bans", None),
                ("Their bans", None),
                ("Lobby ban", None),
                ("Your team", Some("Ban votes")),
                ("Your team", Some("Hero select")),
                ("Your team", Some("Played")),
            ]
        );
        assert_eq!(names(&rows[0]), ["Sombra", "Mauga"]);
        assert_eq!(names(&rows[1]), ["Ana", "Winston"]);
        assert_eq!(names(&rows[2]), ["Sigma"]);
        assert_eq!(names(&rows[3]).len(), 6);
        assert_eq!(names(&rows[4])[0], "Wrecking Ball");
        assert_eq!(names(&rows[5]), ["Wrecking Ball"]);
    }

    #[test]
    fn tooltips_say_what_the_chip_is() {
        let rows = rows(bans_picks(&game(NORMAL)));
        let tip = |r: usize, c: usize| match &rows[r].body {
            Body::Chips(chips) => chips[c].tip.clone(),
            Body::Quiet(_) => String::new(),
        };
        assert_eq!(tip(0, 0), "Banned: Sombra");
        assert_eq!(tip(2, 0), "Lobby ban: Sigma");
        assert_eq!(tip(3, 0), "You voted to ban Reinhardt");
        assert_eq!(tip(3, 1), "Teammate voted to ban Tracer");
        assert_eq!(tip(4, 0), "Shown at hero select: Wrecking Ball, you");
        assert_eq!(tip(4, 1), "Shown at hero select: Tracer");
        assert_eq!(tip(5, 0), "Played Wrecking Ball, you");
    }

    #[test]
    fn a_partial_read_keeps_its_gaps_visible() {
        let b = bans_picks(&game(UNSURE));
        assert!(b.has_ban());
        assert!(b.has_swap());
        assert!(b.has_not_read());
        let rows = rows(b);
        // Bans with no side are one row with a note, and no lobby ban row.
        assert_eq!(rows[0].team, "Bans");
        assert_eq!(rows[0].note, Some(TEAM_NOT_READ));
        assert_eq!(names(&rows[0]), ["Sombra", "Mauga", "Not read", "Winston"]);
        assert!(rows.iter().all(|r| r.team != "Lobby ban"));
        let Body::Chips(bans) = &rows[0].body else {
            panic!("chips")
        };
        assert_eq!(bans[0].tip, "Banned: Sombra, team not read");
        assert_eq!(bans[2].tip, "Ban 3: not read");
        // No preference shows as such, and a null slot as Not read.
        let Body::Chips(picks) = &rows[2].body else {
            panic!("chips")
        };
        assert_eq!(picks[2].slot, Slot::NoPreference);
        assert_eq!(picks[2].name(), "No preference");
        assert_eq!(picks[3].slot, Slot::NotRead);
        assert_eq!(picks[3].tip, "Hero select slot 4: not read");
        let Body::Chips(votes) = &rows[1].body else {
            panic!("chips")
        };
        assert_eq!(votes[1].tip, "Teammate voted no preference");
        assert_eq!(votes[3].tip, "Ban vote slot 4: not read");
        // We picked Reinhardt and played Wrecking Ball.
        let Body::Chips(played) = &rows[3].body else {
            panic!("chips")
        };
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
        let rows = rows(bans_picks(&g));
        assert_eq!(names(&rows[0]), ["Not read"]);
        assert_eq!(names(&rows[1]), ["Ana"]);
    }

    #[test]
    fn lists_are_capped_and_plain_strings_read() {
        let g = with(
            r#"{"bans":["ana","reaper","mauga","sigma","zarya"],
                "picks":["ana","reaper","mauga","sigma","zarya","mercy","lucio"],
                "ban_vote":["no-preference"]}"#,
            Some(BansPicksStatus::Ok),
        );
        let rows = rows(bans_picks(&g));
        // No side on any ban: one row, four chips.
        assert_eq!(names(&rows[0]).len(), 4);
        let hero_select = rows.iter().find(|r| r.line == Some("Hero select")).unwrap();
        assert_eq!(names(hero_select).len(), 6);
        let votes = rows.iter().find(|r| r.line == Some("Ban votes")).unwrap();
        assert_eq!(names(votes), ["No preference"]);
    }

    #[test]
    fn unread_parts_get_a_quiet_line() {
        let g = with(r#"{"bans":[],"lobby_ban":{"value":"sigma"}}"#, None);
        let rows = rows(bans_picks(&g));
        let quiet: Vec<_> = rows
            .iter()
            .filter_map(|r| match &r.body {
                Body::Quiet(t) => Some(t.as_str()),
                Body::Chips(_) => None,
            })
            .collect();
        assert_eq!(
            quiet,
            [
                "Bans weren't read for this game.",
                "Ban votes and hero select weren't read for this game."
            ]
        );
        let g = with(
            r#"{"bans":[{"value":"ana","team":"own"}],"picks":[{"value":"ana"}]}"#,
            None,
        );
        let rows = rows_of(&g);
        assert!(rows.iter().any(|r| matches!(&r.body,
            Body::Quiet(t) if t == "Ban votes weren't read for this game.")));
        let g = with(
            r#"{"bans":[{"value":"ana","team":"own"}],"ban_vote":[{"value":"ana"}]}"#,
            None,
        );
        let rows = rows_of(&g);
        assert!(rows.iter().any(|r| matches!(&r.body,
            Body::Quiet(t) if t == "Hero select wasn't read for this game.")));
    }

    fn rows_of(g: &Game) -> Vec<Row> {
        rows(bans_picks(g))
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
        assert_eq!(line, "Not read: the Read bans and picks setting is off.");
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
}
