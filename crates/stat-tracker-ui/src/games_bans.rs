//! The open Games row's "Bans and picks" block, as a view model: which rows
//! the block has, which chips are in them and what each says. The drawing is
//! in `games_bans_view`. The spec is the UX Spec bans and picks a3b, "chips
//! stay default" (`redesign/ux-spec/bans-picks-a3.md`), on top of v2 and its
//! alpha.2.1 addendum (`redesign/ux-spec/bans-picks.md`).
//!
//! The block is a header (label, a one-line legend) and four rows with their
//! label on the left at every width: Bans, Your ban votes, Hero select and
//! Played. A row that was not read says so in one quiet line, so the layout
//! never jumps between games.
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
//! cv-v1 has no unsure / middle state yet: below the accept bar a slot is
//! stored with no value, and every such slot the row draws is a Not read
//! chip: a ban, a ban vote, a hero select slot, and the lobby ban in the
//! lobby layout (the plain layout has no lobby slot, so no Lobby group). A
//! row with no read at all says so in one quiet line instead. So nothing
//! amber is drawn: no warning sign on chips, no "N unsure" pill, no "Reads
//! may be off" pill, and the legend leaves out "Might be misread". When
//! Vision adds a middle state between accept and reject, amber marks and
//! pills can return. Only results tagged cv-v1 (or with no tag, as sync
//! reads them) are drawn as chips: a result from any other reader is one
//! quiet line, so a new reader cannot come out as plain sure chips here
//! until this file learns its states. Also not drawn: the Their team Played
//! row (the tracker stores only our own hero).

use scuffed_types::{BANS_PICKS_MAX_BANS, BANS_PICKS_MAX_PICKS, BANS_PICKS_NO_PREFERENCE};
use serde_json::Value;
use stat_tracker::model::{BansPicksMissing, BansPicksStatus};

use crate::model::Game;
use crate::settings_model::AdvancedSection;

/// The reader's template spelling of the "no preferred hero" icon
/// ([`BANS_PICKS_NO_PREFERENCE`] is the stored spelling).
const NO_PREFERENCE_TEMPLATE: &str = "no-preference";

/// Our own ban vote slots on the vote screen, numbered 1 to this.
const VOTE_SLOTS: u8 = 3;

/// The only reader whose stored values this block knows how to draw. Kept
/// here, not taken from the reader crate, so a new reader id has to come
/// through this file (and its states with it).
const KNOWN_RECOGNIZER: &str = "cv-v1";
/// The quiet line for a result stored by any other reader.
pub const UNKNOWN_READER: &str = "Not shown: this game was read by a reader this version doesn't know. Update the tracker to see it.";

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
const TEAM_NOT_READ_TIP: &str = "The tracker couldn't tell which team made these bans.";

/// A screen the status says the tracker never saw (Hero select, Your ban
/// votes).
pub const SCREEN_NOT_SEEN: &str = "Screen not seen";
/// Softer than "didn't see": the status also counts a hero select whose
/// first frame was seen, with no picks, as missing.
const HERO_SELECT_NOT_SEEN_TIP: &str = "The tracker didn't read hero select for this game.";
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

/// The legend words, one per glyph the card uses.
pub const LEGEND_BANNED: &str = "Banned";
pub const LEGEND_SWAP: &str = "Swapped hero";
pub const LEGEND_OUTLINE: &str = "Not read or not seen";

/// The words Played chips add for a run the hero bar marks unconfirmed.
const UNCONFIRMED: &str = "unconfirmed";

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
    /// Draw the "You" tag. Only the first chip of a row of our own chips
    /// carries it (hero select slot 1, the first Played chip), so later
    /// Played chips, ours too, have it off.
    pub you_tag: bool,
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
            Slot::Skipped => "Skipped",
            Slot::Empty => "Not seen",
        }
    }

    /// The outline chip shared by Not read and Not seen.
    pub fn outline(&self) -> bool {
        matches!(self.slot, Slot::NotRead | Slot::Empty)
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

/// Which legend items the header shows: only glyphs this card uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Legend {
    pub banned: bool,
    pub swap: bool,
    pub outline: bool,
}

/// The mark drawn in front of a legend item's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegendMark {
    Ban,
    Swap,
    /// The outline swatch of Not read and Not seen.
    Outline,
}

impl Legend {
    pub fn is_empty(self) -> bool {
        !(self.banned || self.swap || self.outline)
    }

    /// The items to draw, left to right: each mark and its words.
    pub fn items(self) -> Vec<(LegendMark, &'static str)> {
        [
            (self.banned, LegendMark::Ban, LEGEND_BANNED),
            (self.swap, LegendMark::Swap, LEGEND_SWAP),
            (self.outline, LegendMark::Outline, LEGEND_OUTLINE),
        ]
        .into_iter()
        .filter(|(on, ..)| *on)
        .map(|(_, mark, words)| (mark, words))
        .collect()
    }
}

/// The open block and its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub rows: Vec<Row>,
}

impl Block {
    fn chips(&self) -> impl Iterator<Item = &Chip> {
        self.rows.iter().flat_map(Row::chips)
    }

    pub fn legend(&self) -> Legend {
        Legend {
            banned: self
                .chips()
                .any(|c| c.ban && matches!(c.slot, Slot::Hero(_))),
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
    /// Only a reader-off game jumps to the Read bans and picks row.
    pub reveal_row: bool,
}

/// What the open row shows for bans and picks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BansPicks {
    /// No block: nothing was read and no reason is stored (a game from
    /// before the reader, or a good status with no stored result).
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

/// The block for one game. A stored result from an unknown reader is one
/// quiet line. A result that read something is the block. So is one that
/// read nothing when the status says the reader ran on this game (`Ok` or
/// `Partial`): its rows then say, per screen, whether it was seen.
pub fn bans_picks(g: &Game) -> BansPicks {
    let status = g.bans_picks_status.as_ref();
    if let Some(stored) = g.bans_picks.as_deref().and_then(parse) {
        if !stored.known_reader {
            return BansPicks::Reason {
                line: UNKNOWN_READER.to_string(),
                tip: None,
                settings: None,
            };
        }
        if stored.read_something() || screens_known(status) {
            return BansPicks::Block(Block {
                rows: stored.rows(g),
            });
        }
    }
    match status.and_then(reason) {
        Some((line, settings)) => BansPicks::Reason {
            line,
            tip: reason_tip(g.bans_picks_status.as_ref()),
            settings: settings.then(|| settings_link(g.bans_picks_status.as_ref())),
        },
        None => BansPicks::Hidden,
    }
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
/// goes with it. `None` for a status that says the reader ran (`Ok`,
/// `Partial`): a result it stored is drawn as the block, and with no stored
/// result there is nothing to say (the daemon never stores that pair).
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
/// by the Read bans and picks switch (Capture and reading).
fn settings_link(status: Option<&BansPicksStatus>) -> SettingsLink {
    let reveal_row = crate::bans_picks_setting::scrolls_to_row(status);
    match status {
        Some(BansPicksStatus::PackMissing { .. }) => SettingsLink {
            tip: "Settings, website connection. Linking an account lets the tracker download the pack.",
            open: AdvancedSection::WebsiteConnection,
            reveal_row,
        },
        Some(BansPicksStatus::ReaderOff) => SettingsLink {
            tip: "Settings, Capture and reading, Read bans and picks. It takes effect after a restart. Games already played stay unread.",
            open: AdvancedSection::CaptureReading,
            reveal_row,
        },
        _ => SettingsLink {
            tip: "Settings, Capture and reading, Read bans and picks. It takes effect after a restart. Games already played stay unread.",
            open: AdvancedSection::CaptureReading,
            reveal_row,
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

/// The lobby ban slot, as stored with the layout.
#[derive(Clone, PartialEq, Eq)]
enum Lobby {
    /// The plain layout (or no ban screen): no lobby slot.
    None,
    Hero(String),
    /// The lobby layout, its slot below the accept bar.
    NotRead,
}

/// The stored JSON, as plain slots.
struct Stored {
    /// The result was stored by cv-v1 (or carries no tag, which sync also
    /// takes as cv-v1).
    known_reader: bool,
    bans: Vec<Ban>,
    lobby: Lobby,
    picks: Vec<Slot>,
    /// Our own ban votes as (stored slot number, slot), in slot order.
    ban_vote: Vec<(u8, Slot)>,
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
    let known_reader = match v.get("bans_picks_recognizer") {
        None | Some(Value::Null) => true,
        Some(Value::String(id)) => id.trim() == KNOWN_RECOGNIZER,
        Some(_) => false,
    };
    let lobby_layout = v.get("layout").and_then(Value::as_str) == Some("lobby");
    Some(Stored {
        known_reader,
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
        // The lobby layout always has a lobby slot, read or not; the plain
        // layout stores `lobby_ban: null`.
        lobby: match v.get("lobby_ban").map(slot) {
            Some(Slot::Hero(name)) => Lobby::Hero(name),
            _ if lobby_layout => Lobby::NotRead,
            _ => Lobby::None,
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

/// Our own three ban votes from `ban_votes`, in slot order, with their
/// stored slot numbers, which tells a skipped or empty slot apart from one
/// that was not read. An entry whose slot is not 1 to 3, or repeats one
/// already taken, is left out. `None` when the key is missing (games read
/// before it existed), null (a vote screen that did not look like one) or
/// has no usable entry for us: the votes are then not read. The flat
/// `ban_vote` list is not a fallback.
fn own_votes(v: &Value) -> Option<Vec<(u8, Slot)>> {
    let all = v.get("ban_votes")?.as_array()?;
    let mut own: Vec<(u8, Slot)> = Vec::new();
    for e in all
        .iter()
        .filter(|e| e.get("voter").and_then(Value::as_u64) == Some(0))
    {
        let Some(n) = e
            .get("slot")
            .and_then(Value::as_u64)
            .and_then(|n| u8::try_from(n).ok())
            .filter(|n| (1..=VOTE_SLOTS).contains(n))
        else {
            continue;
        };
        if own.iter().any(|(taken, _)| *taken == n) {
            continue;
        }
        own.push((n, {
            let hero = e
                .get("hero")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|h| !h.is_empty());
            match (e.get("state").and_then(Value::as_str), hero) {
                (Some("hero"), Some(key)) => Slot::Hero(hero_name(key)),
                (Some("skipped"), _) => Slot::Skipped,
                (Some("empty"), _) => Slot::Empty,
                _ => Slot::NotRead,
            }
        }));
    }
    if own.is_empty() {
        return None;
    }
    own.sort_by_key(|(n, _)| *n);
    Some(own)
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

    fn votes_read(&self) -> bool {
        self.ban_vote.iter().any(|(_, s)| *s != Slot::NotRead)
    }

    /// A ban, the lobby ban, a hero select slot or a ban vote slot was read
    /// (a hero, No preference, or a skipped or empty vote).
    fn read_something(&self) -> bool {
        self.bans_read()
            || matches!(self.lobby, Lobby::Hero(_))
            || read(&self.picks)
            || self.votes_read()
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
    /// team` chips, then the bans whose team was not read with a `Team not
    /// read` note, then a divider, `Lobby` and its chip (Not read in the
    /// lobby layout when its slot was not read). When no ban hero was read
    /// but the lobby ban was, the stored ban slots come first as Not read
    /// chips (or the quiet line when none is stored), so the row never
    /// reads as "no team bans". When nothing on the ban screen was read, one
    /// quiet line that says whether it was seen. A Not read tip numbers its
    /// slot inside its own group and names the group ("Their ban 2", "Team
    /// not read, ban 1"), so two slots in one row never share a name.
    fn bans_row(&self, status: Option<&BansPicksStatus>) -> Row {
        let mut parts = Vec::new();
        let lobby_read = matches!(self.lobby, Lobby::Hero(_));
        if self.bans_read() {
            for (team, key, nth, by) in [
                (Team::Own, GROUP_OWN, "Your team's ban", "your team"),
                (Team::Enemy, GROUP_ENEMY, "Their ban", "their team"),
            ] {
                let chips: Vec<Chip> = self
                    .bans
                    .iter()
                    .filter(|b| b.team == Some(team))
                    .enumerate()
                    .map(|(i, b)| match &b.slot {
                        Slot::Hero(name) => ban_chip(name, format!("Banned by {by}: {name}")),
                        _ => not_read_chip(format!("{nth} {}: not read", i + 1)),
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
            // Numbered inside their own group, as the team groups are, so
            // no two ban slots share a name.
            let team_less: Vec<Chip> = self
                .bans
                .iter()
                .filter(|b| b.team.is_none())
                .enumerate()
                .map(|(i, b)| match &b.slot {
                    Slot::Hero(name) => ban_chip(name, format!("Banned: {name}, team not read")),
                    _ => not_read_chip(format!("Team not read, ban {}: not read", i + 1)),
                })
                .collect();
            if !team_less.is_empty() {
                if !parts.is_empty() {
                    parts.push(Part::Divider);
                }
                parts.push(Part::Chips(team_less));
                parts.push(Part::Note {
                    text: TEAM_NOT_READ,
                    tip: TEAM_NOT_READ_TIP,
                });
            }
        } else if lobby_read && !self.bans.is_empty() {
            parts.push(Part::Chips(
                (1..=self.bans.len())
                    .map(|n| not_read_chip(format!("Ban {n}: not read")))
                    .collect(),
            ));
        } else {
            parts.push(bans_quiet(status));
        }
        // With no ban hero and no lobby hero read, the quiet line already
        // says the ban screen's heroes were not read, lobby slot included.
        let lobby = match &self.lobby {
            Lobby::Hero(name) => Some(ban_chip(name, format!("Lobby ban: {name}"))),
            Lobby::NotRead if self.bans_read() => {
                Some(not_read_chip("Lobby ban: not read".to_string()))
            }
            Lobby::NotRead | Lobby::None => None,
        };
        if let Some(chip) = lobby {
            if !parts.is_empty() {
                parts.push(Part::Divider);
            }
            parts.push(Part::Key(GROUP_LOBBY));
            parts.push(Part::Chips(vec![chip]));
        }
        Row {
            label: LABEL_BANS,
            parts,
        }
    }

    /// Our own three ban votes, or a quiet line: `Screen not seen` when the
    /// status says the ban vote was never seen, the older wording otherwise.
    fn votes_row(&self, status: Option<&BansPicksStatus>) -> Row {
        let part = if self.votes_read() {
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

/// The quiet line of a Bans row with no hero read: whether the ban screen
/// was seen, or the older wording for a game with no screen status.
fn bans_quiet(status: Option<&BansPicksStatus>) -> Part {
    let (text, tip) = if not_seen(status, BansPicksMissing::BanResults) {
        (NO_BAN_SCREEN, Some(NO_BAN_SCREEN_TIP))
    } else if screens_known(status) {
        (BANS_NOT_READ, Some(BANS_NOT_READ_TIP))
    } else {
        (LEGACY_BANS, None)
    };
    Part::Quiet { text, tip }
}

fn ban_chip(name: &str, tip: String) -> Chip {
    Chip {
        slot: Slot::Hero(name.to_string()),
        ban: true,
        you_tag: false,
        swap: false,
        tip,
    }
}

/// A ban slot (or the lobby slot) below the accept bar.
fn not_read_chip(tip: String) -> Chip {
    Chip {
        slot: Slot::NotRead,
        ban: true,
        you_tag: false,
        swap: false,
        tip,
    }
}

/// Our own three ban votes, in slot order, from `ban_votes`, each tooltip
/// numbered by its stored slot. A skipped slot is a plain neutral chip
/// "Skipped" with no glyph, an empty one an outline chip "Not seen" (the
/// vote may have come after the last frame). The row says "Your ban votes",
/// so the chips carry no "You" tag.
fn vote_chips(slots: &[(u8, Slot)]) -> Vec<Chip> {
    slots
        .iter()
        .map(|(n, slot)| {
            let tip = match slot {
                Slot::Hero(name) => format!("You voted to ban {name}"),
                // Legacy only: `ban_votes` has no no-preference state, and
                // this row no longer reads the flat `ban_vote` list where
                // alpha.2.1 stored it. Kept so the match stays exhaustive.
                Slot::NoPreference => "You voted no preference".to_string(),
                Slot::Skipped => format!("You skipped ban vote {n}"),
                Slot::Empty => {
                    format!("Your ban vote {n}: not seen, it may have come after the last frame")
                }
                Slot::NotRead => format!("Your ban vote {n}: not read"),
            };
            Chip {
                slot: slot.clone(),
                ban: false,
                you_tag: false,
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
                Slot::NotRead | Slot::Skipped | Slot::Empty => {
                    format!("Hero select slot {}: not read", i + 1)
                }
            };
            Chip {
                slot: slot.clone(),
                ban: false,
                you_tag: you,
                swap: false,
                tip,
            }
        })
        .collect()
}

/// Every hero we played, in the order played, from the same runs the hero
/// bar draws (`games_detail::hero_split`), so the two never disagree: a
/// dismissed run folds into the main hero, consecutive runs of one hero are
/// one chip, and a hero played again later keeps its first place. A hero
/// whose every run the bar marks unconfirmed says "unconfirmed" in its
/// tooltip (no amber: Played is not a bans and picks read). A game whose
/// hero was set by hand, or whose runs name no hero, keeps the single chip
/// from the row's hero. Only the first chip can carry the swap glyph, and
/// only when the hero select pick is not one of the Played chips: a pick that
/// was played at some point was not swapped away from.
fn played_chips(g: &Game, own_pick: Option<&Slot>) -> Vec<Chip> {
    let corrected = g.edited_fields.iter().any(|f| f == "hero");
    // (display name, every run of it unconfirmed)
    let mut runs: Vec<(String, bool)> = Vec::new();
    if !corrected && !g.segments.is_empty() {
        for part in crate::games_detail::hero_split(g).bar {
            let hero = part.hero.trim();
            if hero.is_empty() || hero.eq_ignore_ascii_case("unknown") {
                continue;
            }
            let name = hero_name(hero);
            match runs.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                Some((_, unconfirmed)) => *unconfirmed &= part.unconfirmed,
                None => runs.push((name, part.unconfirmed)),
            }
        }
    }
    let Some(((first, first_unconfirmed), rest)) = runs.split_first() else {
        return played_chip(&g.hero, own_pick, false).into_iter().collect();
    };
    let pick_was_played = matches!(own_pick,
        Some(Slot::Hero(picked)) if runs.iter().any(|(n, _)| n.eq_ignore_ascii_case(picked)));
    let own_pick = if pick_was_played { None } else { own_pick };
    played_chip(first, own_pick, *first_unconfirmed)
        .into_iter()
        .chain(rest.iter().map(|(name, unconfirmed)| Chip {
            slot: Slot::Hero(name.clone()),
            ban: false,
            // The "You" tag is drawn once, on the first chip of the row.
            you_tag: false,
            swap: false,
            tip: format!("Then played {name}{}, you", unconfirmed_words(*unconfirmed)),
        }))
        .collect()
}

fn unconfirmed_words(unconfirmed: bool) -> String {
    if unconfirmed {
        format!(", {UNCONFIRMED}")
    } else {
        String::new()
    }
}

/// Our own first Played chip, from the game data. `None` when the game has no
/// hero. It carries the swap glyph when it is a real hero that differs from
/// our own hero select hero, which must be a real hero too.
fn played_chip(own_hero: &str, own_pick: Option<&Slot>, unconfirmed: bool) -> Option<Chip> {
    let hero = own_hero.trim();
    if hero.is_empty() || hero.eq_ignore_ascii_case("unknown") {
        return None;
    }
    let name = hero_name(hero);
    let swapped_from = match own_pick {
        Some(Slot::Hero(picked)) if !picked.eq_ignore_ascii_case(&name) => Some(picked),
        _ => None,
    };
    let unconfirmed = unconfirmed_words(unconfirmed);
    let tip = match swapped_from {
        Some(picked) => format!("Played {name}, {picked} at hero select{unconfirmed}, you"),
        None => format!("Played {name}{unconfirmed}, you"),
    };
    Some(Chip {
        slot: Slot::Hero(name),
        ban: false,
        you_tag: true,
        swap: swapped_from.is_some(),
        tip,
    })
}

#[cfg(test)]
#[path = "games_bans_tests.rs"]
mod tests;
