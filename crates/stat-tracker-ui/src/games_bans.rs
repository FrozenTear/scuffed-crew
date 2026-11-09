//! The open Games row's "Bans and picks" block, as a view model: which rows
//! the block has, which chips are in them and what each says. The drawing is
//! in `games_bans_view`. The spec is the UX Spec bans and picks a3b, "chips
//! stay default" (`redesign/ux-spec/bans-picks-a3.md`), on top of v2 and its
//! alpha.2.1 addendum (`redesign/ux-spec/bans-picks.md`).
//!
//! The block is a header (label, the unsure pill, a one-line legend) and four
//! rows with their label on the left at every width: Bans, Your ban votes,
//! Hero select and Played. A row that was not read says so in one quiet line,
//! so the layout never jumps between games.
//!
//! The data is the game's stored cv-v1 JSON (`Game::bans_picks`) and its
//! status (`Game::bans_picks_status`). The JSON is read leniently: a value
//! that is not a string, an empty one and a missing list are all "not read",
//! and a stored result that does not parse is no result. Nothing here ever
//! hides the game, only the block. `BansPicksStatus::Partial { missing }`
//! tells a screen the tracker never saw apart from one it saw and could not
//! read; a game stored before the status existed keeps the older "weren't
//! read" wording.
//!
//! Hooks with no data behind them yet: [`Chip::suspect`] (the amber "might be
//! misread" mark and the "N unsure" pill) and [`Block::weak`] (the "Reads may
//! be off" pill). The tracker stores no per-read suspect flag for bans, the
//! lobby ban, ban votes or hero select, and no game-level weak flag: the band
//! that makes a read "suspect" is still an open question for Vision. Both are
//! always false today. Also not drawn, as the data does not exist: the Their
//! team Played row (the tracker stores only our own hero).

use scuffed_types::{
    BANS_PICKS_MAX_BAN_VOTE, BANS_PICKS_MAX_BANS, BANS_PICKS_MAX_PICKS, BANS_PICKS_NO_PREFERENCE,
};
use serde_json::Value;
use stat_tracker::model::{BansPicksMissing, BansPicksStatus};

use crate::model::Game;
use crate::settings_model::AdvancedSection;

/// The reader's template spelling of the "no preferred hero" icon
/// ([`BANS_PICKS_NO_PREFERENCE`] is the stored spelling).
const NO_PREFERENCE_TEMPLATE: &str = "no-preference";

pub const HEADER: &str = "Bans and picks";

/// The row labels, in the order the rows are drawn.
pub const LABEL_BANS: &str = "Bans";
pub const LABEL_VOTES: &str = "Your ban votes";
pub const LABEL_HERO_SELECT: &str = "Hero select";
pub const LABEL_PLAYED: &str = "Played";

/// The group labels inside the Bans row.
pub const GROUP_OWN: &str = "Your team";
pub const GROUP_ENEMY: &str = "Their team";
pub const GROUP_LOBBY: &str = "Lobby";

pub const TEAM_NOT_READ: &str = "Team not read";
const TEAM_NOT_READ_TIP: &str = "Which team made each ban wasn't read for this game.";

/// A screen the status says the tracker never saw (Hero select, Your ban
/// votes).
pub const SCREEN_NOT_SEEN: &str = "Screen not seen";
const HERO_SELECT_NOT_SEEN_TIP: &str = "The tracker didn't see hero select for this game.";
const BAN_VOTE_NOT_SEEN_TIP: &str = "The tracker didn't see the ban vote for this game.";
pub const NO_BAN_SCREEN: &str = "No ban screen seen";
const NO_BAN_SCREEN_TIP: &str = "The tracker didn't see a ban screen for this game. Either the game had none, or the tracker missed it.";
pub const BANS_NOT_READ: &str = "Ban screen seen, heroes not read";
const BANS_NOT_READ_TIP: &str = "The ban screen was seen, but its heroes couldn't be read.";

/// The shipped wording, kept for games stored with no screen status.
pub const LEGACY_BANS: &str = "Bans weren't read for this game.";
pub const LEGACY_VOTES: &str = "Your ban votes weren't read for this game.";
pub const LEGACY_HERO_SELECT: &str = "Hero select wasn't read for this game.";
/// The Played row of a game with no hero (the a3 wording).
pub const PLAYED_NOT_READ: &str = "Not read";

/// The header pills (both amber, with the warning sign).
pub const UNSURE_PILL_TIP: &str =
    "Reads the tracker is not sure about. They count only bans, ban votes and hero select.";
pub const WEAK_PILL: &str = "Reads may be off";
pub const WEAK_PILL_TIP: &str = "The reader wasn't sure about this game's screens. Check the names. Use Report misread if one is wrong.";

/// The legend words, one per glyph the card uses.
pub const LEGEND_BANNED: &str = "Banned";
pub const LEGEND_UNSURE: &str = "Might be misread";
pub const LEGEND_SWAP: &str = "Swapped hero";
pub const LEGEND_OUTLINE: &str = "Not read or not seen";

/// What one slot of the screen read as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    /// A hero, by display name.
    Hero(String),
    /// The "no preferred hero" icon.
    NoPreference,
    /// The slot could not be read.
    NotRead,
    /// A ban vote that was skipped (the circle with a slash).
    Skipped,
    /// A ban vote slot with nothing in it when the screen was captured.
    /// That can be a vote cast after the last frame, so it reads "Not seen",
    /// never "did not vote".
    Empty,
}

/// One chip: a slot, with the words every chip carries (visible name,
/// tooltip) and the flags that pick its look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub slot: Slot,
    /// A ban or the lobby ban: ban glyph (on a read hero), no fill.
    pub ban: bool,
    /// Our own chip: the "You" tag.
    pub you: bool,
    /// Our own Played chip when it differs from our hero select hero.
    pub swap: bool,
    /// The reader was not sure of this hero: amber outline and warning sign,
    /// and it counts toward "N unsure". Hook only: always false today, as
    /// the tracker stores no per-read suspect flag for bans and picks yet
    /// (Vision's confidence band is still open). Never set on Played.
    pub suspect: bool,
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
            Slot::Skipped => "Skipped",
            Slot::Empty => "Not seen",
        }
    }

    /// Whether the chip is drawn unsure: a read hero the reader flagged.
    pub fn unsure(&self) -> bool {
        self.suspect && matches!(self.slot, Slot::Hero(_))
    }

    /// The outline chip shared by Not read and Not seen.
    pub fn outline(&self) -> bool {
        matches!(self.slot, Slot::NotRead | Slot::Empty)
    }

    /// The tooltip as shown and spoken: an unsure chip says so.
    pub fn tooltip(&self) -> String {
        if self.unsure() {
            format!(
                "{}. {} might be misread. Use Report misread if it is wrong.",
                self.tip,
                self.name()
            )
        } else {
            self.tip.clone()
        }
    }
}

/// One piece of a row's body, drawn left to right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// A group label in front of its chips ("Your team", "Their team",
    /// "Lobby").
    Key(&'static str),
    Chips(Vec<Chip>),
    /// A muted note after the chips ("Team not read"), with its tooltip.
    Note {
        text: &'static str,
        tip: &'static str,
    },
    /// One quiet line, in place of chips that were not read or not seen.
    Quiet {
        text: &'static str,
        tip: Option<&'static str>,
    },
    /// A thin drawn line between groups (a 1px container, not a character).
    Divider,
}

/// One row of the block: its label on the left and its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// "Bans", "Your ban votes", "Hero select" or "Played".
    pub label: &'static str,
    pub parts: Vec<Part>,
}

impl Row {
    fn chips(&self) -> impl Iterator<Item = &Chip> {
        self.parts.iter().flat_map(|p| match p {
            Part::Chips(chips) => chips.as_slice(),
            _ => &[],
        })
    }
}

/// The amber pill in the header, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pill {
    /// "N unsure": per-read flags, counting bans, the lobby ban, ban votes
    /// and hero select (never Played).
    Unsure(usize),
    /// "Reads may be off": only a game-level flag exists.
    Weak,
}

impl Pill {
    pub fn words(self) -> String {
        match self {
            Pill::Unsure(n) => format!("{n} unsure"),
            Pill::Weak => WEAK_PILL.to_string(),
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Pill::Unsure(_) => UNSURE_PILL_TIP,
            Pill::Weak => WEAK_PILL_TIP,
        }
    }
}

/// Which legend items the header shows: only glyphs this card uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Legend {
    pub banned: bool,
    pub unsure: bool,
    pub swap: bool,
    pub outline: bool,
}

impl Legend {
    pub fn is_empty(self) -> bool {
        !(self.banned || self.unsure || self.swap || self.outline)
    }
}

/// The open block: its rows, and the game-level weak flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub rows: Vec<Row>,
    /// The game-level "reader weak" flag, the fallback for the per-read
    /// flags. Hook only: no such field is stored on the game yet, so it is
    /// always false today (see [`reader_weak`]).
    pub weak: bool,
}

impl Block {
    fn chips(&self) -> impl Iterator<Item = &Chip> {
        self.rows.iter().flat_map(Row::chips)
    }

    /// How many reads are unsure: bans, the lobby ban, ban votes and hero
    /// select. Played never counts, nor do Not read, Not seen, Skipped and
    /// No preference.
    pub fn unsure(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.label != LABEL_PLAYED)
            .flat_map(Row::chips)
            .filter(|c| c.unsure())
            .count()
    }

    /// The header pill. Per-read flags win over the game-level one, which is
    /// drawn only when no chip is marked.
    pub fn pill(&self) -> Option<Pill> {
        match self.unsure() {
            0 if self.weak => Some(Pill::Weak),
            0 => None,
            n => Some(Pill::Unsure(n)),
        }
    }

    pub fn legend(&self) -> Legend {
        Legend {
            banned: self
                .chips()
                .any(|c| c.ban && matches!(c.slot, Slot::Hero(_))),
            unsure: self.unsure() > 0,
            swap: self.chips().any(|c| c.swap),
            outline: self.chips().any(Chip::outline),
        }
    }
}

/// Where "Open Settings" goes and what its tooltip says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsLink {
    pub tip: &'static str,
    /// The Advanced row the button opens.
    pub open: AdvancedSection,
}

/// What the open row shows for bans and picks.
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
    Block(Block),
}

/// The block for one game.
pub fn bans_picks(g: &Game) -> BansPicks {
    if let Some(stored) = g.bans_picks.as_deref().and_then(parse)
        && stored.read_something()
    {
        return BansPicks::Block(Block {
            rows: stored.rows(g),
            weak: reader_weak(g),
        });
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

/// The game-level "reads may be off" flag. Hook only: neither the game nor
/// its status stores one yet (it would be a `bans_picks_weak` flag, the
/// fallback the spec allows if per-read flags are not done), so this is
/// always false. Wire it here when the tracker stores it.
fn reader_weak(_g: &Game) -> bool {
    false
}

/// Whether the status says the tracker never saw this screen.
fn not_seen(status: Option<&BansPicksStatus>, screen: BansPicksMissing) -> bool {
    matches!(status, Some(BansPicksStatus::Partial { missing }) if missing.contains(&screen))
}

/// Whether the status says which screens were seen (games stored before the
/// status existed have none, and keep the older wording).
fn screens_known(status: Option<&BansPicksStatus>) -> bool {
    matches!(
        status,
        Some(BansPicksStatus::Ok | BansPicksStatus::Partial { .. })
    )
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
            "Not read: Read bans and picks was off for this game.".to_string(),
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
        // Only `ban_votes` is read for our votes. The flat `ban_vote` list is
        // never a source: games stored by 0.6.0-alpha.2.1 or older hold the
        // six roster preferred heroes there, which are not votes. A game
        // with no `ban_votes` key (or a null one) has its votes not read.
        ban_vote: own_votes(&v).unwrap_or_default(),
    })
}

/// Our own three ban votes from `ban_votes`, in slot order, which tells a
/// skipped or empty slot apart from one that was not read. `None` when the
/// key is missing (games read before it existed), null (a vote screen that
/// did not look like one) or has no entry for us: the votes are then not
/// read. The flat `ban_vote` list is not a fallback.
fn own_votes(v: &Value) -> Option<Vec<Slot>> {
    let all = v.get("ban_votes")?.as_array()?;
    let mut own: Vec<(u64, Slot)> = all
        .iter()
        .filter(|e| e.get("voter").and_then(Value::as_u64) == Some(0))
        .map(|e| {
            let n = e.get("slot").and_then(Value::as_u64).unwrap_or(u64::MAX);
            let hero = e
                .get("hero")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|h| !h.is_empty());
            let s = match (e.get("state").and_then(Value::as_str), hero) {
                (Some("hero"), Some(key)) => Slot::Hero(hero_name(key)),
                (Some("skipped"), _) => Slot::Skipped,
                (Some("empty"), _) => Slot::Empty,
                _ => Slot::NotRead,
            };
            (n, s)
        })
        .collect();
    if own.is_empty() {
        return None;
    }
    own.sort_by_key(|(n, _)| *n);
    Some(
        own.into_iter()
            .take(BANS_PICKS_MAX_BAN_VOTE)
            .map(|(_, s)| s)
            .collect(),
    )
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

    /// The four rows, always in this order and never hidden, so the block
    /// keeps its height from game to game.
    fn rows(&self, g: &Game) -> Vec<Row> {
        let status = g.bans_picks_status.as_ref();
        let played = played_chips(g, self.picks.first());
        let played = if played.is_empty() {
            Part::Quiet {
                text: PLAYED_NOT_READ,
                tip: None,
            }
        } else {
            Part::Chips(played)
        };
        vec![
            self.bans_row(status),
            self.votes_row(status),
            self.hero_select_row(status),
            Row {
                label: LABEL_PLAYED,
                parts: vec![played],
            },
        ]
    }

    /// Bans of both teams on one row: `Your team` chips, a divider, `Their
    /// team` chips, a divider, `Lobby` and its chip. When any ban has no
    /// side, every ban chip then `Team not read`. When nothing on the ban
    /// screen was read, one quiet line that says whether it was seen. A
    /// lobby ban read with no team ban is drawn alone, with no quiet line
    /// that would say no hero was read.
    fn bans_row(&self, status: Option<&BansPicksStatus>) -> Row {
        let mut parts = Vec::new();
        if !self.bans_read() {
            if self.lobby_ban.is_none() {
                let (text, tip) = if not_seen(status, BansPicksMissing::BanResults) {
                    (NO_BAN_SCREEN, Some(NO_BAN_SCREEN_TIP))
                } else if screens_known(status) {
                    (BANS_NOT_READ, Some(BANS_NOT_READ_TIP))
                } else {
                    (LEGACY_BANS, None)
                };
                parts.push(Part::Quiet { text, tip });
            }
        } else if self.bans.iter().any(|b| b.team.is_none()) {
            let chips = self
                .bans
                .iter()
                .enumerate()
                .map(|(i, b)| match &b.slot {
                    Slot::Hero(name) => ban_chip(name, format!("Banned: {name}, team not read")),
                    _ => not_read_chip(true, format!("Ban {}: not read", i + 1)),
                })
                .collect();
            parts.push(Part::Chips(chips));
            parts.push(Part::Note {
                text: TEAM_NOT_READ,
                tip: TEAM_NOT_READ_TIP,
            });
        } else {
            for (team, key, nth) in [
                (Team::Own, GROUP_OWN, "Your team's ban"),
                (Team::Enemy, GROUP_ENEMY, "Their ban"),
            ] {
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
                if chips.is_empty() {
                    continue;
                }
                if !parts.is_empty() {
                    parts.push(Part::Divider);
                }
                parts.push(Part::Key(key));
                parts.push(Part::Chips(chips));
            }
        }
        if let Some(name) = &self.lobby_ban {
            if !parts.is_empty() {
                parts.push(Part::Divider);
            }
            parts.push(Part::Key(GROUP_LOBBY));
            parts.push(Part::Chips(vec![ban_chip(
                name,
                format!("Lobby ban: {name}"),
            )]));
        }
        Row {
            label: LABEL_BANS,
            parts,
        }
    }

    /// Our own three ban votes, or a quiet line: `Screen not seen` when the
    /// status says the ban vote was never seen, the older wording otherwise.
    fn votes_row(&self, status: Option<&BansPicksStatus>) -> Row {
        let part = if read(&self.ban_vote) {
            Part::Chips(vote_chips(&self.ban_vote))
        } else if not_seen(status, BansPicksMissing::BanVote) {
            Part::Quiet {
                text: SCREEN_NOT_SEEN,
                tip: Some(BAN_VOTE_NOT_SEEN_TIP),
            }
        } else {
            Part::Quiet {
                text: LEGACY_VOTES,
                tip: None,
            }
        };
        Row {
            label: LABEL_VOTES,
            parts: vec![part],
        }
    }

    /// All six hero select slots, ours first with the `You` tag, or a quiet
    /// line as for the votes.
    fn hero_select_row(&self, status: Option<&BansPicksStatus>) -> Row {
        let part = if read(&self.picks) {
            Part::Chips(pick_chips(&self.picks))
        } else if not_seen(status, BansPicksMissing::HeroSelect) {
            Part::Quiet {
                text: SCREEN_NOT_SEEN,
                tip: Some(HERO_SELECT_NOT_SEEN_TIP),
            }
        } else {
            Part::Quiet {
                text: LEGACY_HERO_SELECT,
                tip: None,
            }
        };
        Row {
            label: LABEL_HERO_SELECT,
            parts: vec![part],
        }
    }
}

fn ban_chip(name: &str, tip: String) -> Chip {
    Chip {
        slot: Slot::Hero(name.to_string()),
        ban: true,
        you: false,
        swap: false,
        suspect: false,
        tip,
    }
}

fn not_read_chip(ban: bool, tip: String) -> Chip {
    Chip {
        slot: Slot::NotRead,
        ban,
        you: false,
        swap: false,
        suspect: false,
        tip,
    }
}

/// Our own three ban votes, in slot order, from `ban_votes`. A skipped slot
/// is a plain neutral chip "Skipped" with no glyph, an empty one an outline
/// chip "Not seen" (the vote may have come after the last frame). The row
/// says "Your ban votes", so the chips carry no "You" tag.
fn vote_chips(slots: &[Slot]) -> Vec<Chip> {
    slots
        .iter()
        .enumerate()
        .map(|(i, slot)| {
            let tip = match slot {
                Slot::Hero(name) => format!("You voted to ban {name}"),
                // Legacy only: `ban_votes` has no no-preference state, and
                // this row no longer reads the flat `ban_vote` list where
                // alpha.2.1 stored it. Kept so the match stays exhaustive.
                Slot::NoPreference => "You voted no preference".to_string(),
                Slot::Skipped => format!("You skipped ban vote {}", i + 1),
                Slot::Empty => format!(
                    "Your ban vote {}: not seen, it may have come after the last frame",
                    i + 1
                ),
                Slot::NotRead => format!("Your ban vote {}: not read", i + 1),
            };
            Chip {
                slot: slot.clone(),
                ban: false,
                you: false,
                swap: false,
                suspect: false,
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
                Slot::NotRead | Slot::Skipped | Slot::Empty => {
                    format!("Hero select slot {}: not read", i + 1)
                }
            };
            Chip {
                slot: slot.clone(),
                ban: false,
                you,
                swap: false,
                suspect: false,
                tip,
            }
        })
        .collect()
}

/// Every hero we played, in the order played, from the same runs the hero
/// bar draws (`games_detail::hero_split`), so the two never disagree: a
/// dismissed run folds into the main hero, consecutive runs of one hero are
/// one chip, and a hero played again later keeps its first place. A game
/// whose hero was set by hand, or whose runs name no hero, keeps the single
/// chip from the row's hero. Only the first chip can carry the swap glyph, and
/// only when the hero select pick is not one of the Played chips: a pick that
/// was played at some point was not swapped away from.
fn played_chips(g: &Game, own_pick: Option<&Slot>) -> Vec<Chip> {
    let corrected = g.edited_fields.iter().any(|f| f == "hero");
    let mut names: Vec<String> = Vec::new();
    if !corrected && !g.segments.is_empty() {
        for part in crate::games_detail::hero_split(g).bar {
            let hero = part.hero.trim();
            if hero.is_empty() || hero.eq_ignore_ascii_case("unknown") {
                continue;
            }
            let name = hero_name(hero);
            if !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                names.push(name);
            }
        }
    }
    let Some((first, rest)) = names.split_first() else {
        return played_chip(&g.hero, own_pick).into_iter().collect();
    };
    let pick_was_played = matches!(own_pick,
        Some(Slot::Hero(picked)) if names.iter().any(|n| n.eq_ignore_ascii_case(picked)));
    let own_pick = if pick_was_played { None } else { own_pick };
    played_chip(first, own_pick)
        .into_iter()
        .chain(rest.iter().map(|name| Chip {
            slot: Slot::Hero(name.clone()),
            ban: false,
            // The "You" tag is drawn once, on the first chip of the row.
            you: false,
            swap: false,
            suspect: false,
            tip: format!("Then played {name}, you"),
        }))
        .collect()
}

/// Our own first Played chip, from the game data. `None` when the game has no
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
        suspect: false,
        tip,
    })
}

#[cfg(test)]
#[path = "games_bans_tests.rs"]
mod tests;
