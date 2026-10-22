//! The OCR on/off switch: what `ocr = "off"` needs, and what the daemon
//! decides at startup.
//!
//! Off is granted only when nothing the tracker needs would go unread:
//!
//! * [`Read`] is every read a tracked game depends on. Each says whether
//!   this build reads it without Tesseract ([`Read::without_ocr`]). While
//!   one does not, off is refused, whatever packs are installed, so the
//!   switch can never leave a member with a tracker that stores nothing.
//! * [`PackReadiness`] is what the readers that have no OCR fallback need
//!   loaded: a main hero pack, the map pack, and from the word pack the
//!   result words, the match-start titles and the end-of-match titles.
//! * [`decide`] turns the requested setting into the one the daemon runs
//!   with. A refusal carries its reasons ([`Refusal`]) and the daemon keeps
//!   OCR on.
//! * [`Switch::start`] is the daemon's startup step. It sets the gate
//!   ([`crate::ocr::gate`]) and writes [`Status`] to `ocr_switch.json` in the
//!   data dir, which is how the app shows the reason, the pack state and the
//!   blocked-call counts.
//! * [`Switch::check`] re-checks the packs while the daemon runs. A pack that
//!   disappears or goes out of date does not turn OCR back on: the reads it
//!   feeds are held, and the status file says which pack it is.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{Config, OcrSetting, ReaderSetting};
use crate::ocr::gate::{self, OcrMode};
use crate::shadow::{Reader, ReaderConfig};

/// File name of [`Status`] in the data dir.
pub const STATUS_FILE: &str = "ocr_switch.json";
/// The temp file [`write_status`] renames into place.
pub const STATUS_TMP_FILE: &str = "ocr_switch.json.tmp";
/// How often [`watch`] looks at the packs and the blocked-call count.
pub const WATCH_INTERVAL: Duration = Duration::from_secs(30);
/// How long daemon startup waits for the reader packs still downloading
/// when off is asked for and a pack is missing ([`needs_pack_wait`]).
pub const PACK_WAIT: Duration = Duration::from_secs(45);

// ------------------------------------------------------------------ reads

/// A read a tracked game depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Read {
    /// The six stat numbers of every row, and the "is this a scoreboard"
    /// check that rides on them (OCR sites T2, T3, T7).
    StatCells,
    /// Which row is the member's own (T4, the name cells).
    OwnRow,
    /// The member's hero (T5).
    Hero,
    /// The map on the Tab top bar (T6).
    TopBarMap,
    /// VICTORY / DEFEAT on the accolade screen and the rank screen, as the
    /// end-of-game signal (P1, P2).
    ResultWord,
    /// The centred end title, and the end-of-match wake (P3, P5).
    EndTitle,
    /// The FINAL SCORE line under a faint end title (P4). It opens the gate
    /// of the end title's OCR read. With OCR off the end title is the
    /// words-v1 read, which has no gate, so the line is not read at all.
    FinalScore,
    /// Map vote, hero ban and hero select (P7).
    MatchStart,
    /// The map beside the accolade screen's result word (P6).
    AccoladeMap,
    /// The check that drops a co-op event board (Junkenstein's Revenge on
    /// Adlersbrunn, stats on one team only) instead of storing it as an
    /// Eichenwalde game. It has no OCR call of its own. It needs two inputs:
    /// the word "Adlersbrunn", which a sure banner read supplies without
    /// OCR (#271), and "both teams have stats", which the new reader counts
    /// on its own board (#266).
    PveBoard,
}

impl Read {
    pub const ALL: [Read; 10] = [
        Read::StatCells,
        Read::OwnRow,
        Read::Hero,
        Read::TopBarMap,
        Read::ResultWord,
        Read::EndTitle,
        Read::FinalScore,
        Read::MatchStart,
        Read::AccoladeMap,
        Read::PveBoard,
    ];

    /// True when the code in this build reads it without Tesseract.
    ///
    /// The lane that replaces an OCR site turns its line to `true` when it
    /// merges, and the OCR-off bench has to show 0 blocked calls for it.
    /// `false` here refuses `ocr = "off"` at start ([`decide`]).
    pub const fn without_ocr(self) -> bool {
        match self {
            // #256: the heroes-v2 icon reads.
            Read::Hero => true,
            // #251: the map pack's banner read. The OCR label was its
            // fallback and is not asked for with OCR off.
            Read::TopBarMap => true,
            // #257 reads it from a words-v1 template, and since #263 it is
            // read only on the way to the end title's OCR. With OCR off
            // that way is shut and nothing asks for the line.
            Read::FinalScore => true,
            // #260: the words-v1 phrase masks.
            Read::MatchStart => true,
            // #266 (O10): under the new reader the digit reader's board is
            // the trust gate and the six stats. The per-cell OCR and the
            // full-board text are never run, and a board the reader refuses
            // is not handed to OCR.
            Read::StatCells => true,
            // #262 (O9) and #273 (O9b): the row comes from the highlight,
            // the own-name seeds and, where both are silent, the row the
            // career panel's hero is read on. The name read was the last
            // fallback and is not asked for with OCR off.
            //
            // The condition: the career hero's row exists only with a hero
            // pack that reads the board. Off is refused without one
            // ([`PackGroup::HeroIcons`], a set for boards of every size), so
            // OCR and its name read stay in that case.
            //
            // The limit, written where the member decides: a Tab is held
            // when the member has never had the highlight, the own hero
            // icon is dead or covered, and no seed is trusted yet. With OCR
            // on, the name read finds that row.
            //
            // To take this back: this line, and the list in the test
            // `this_build_reads_everything_without_ocr`.
            Read::OwnRow => true,
            // #270 (O11): the words-v1 result templates name the word on
            // the accolade screen and the rank screen, for the poll tick's
            // signal and for the Tab frame's outcome. Without them and with
            // OCR off the word is held (`read_outcome_word`), and the pack
            // check asks for both screens ([`PackGroup::ResultWords`]).
            Read::ResultWord => true,
            // #263 (O2): the words-v1 end-title templates read the end
            // title and stand in for the end-reel title card. Without them
            // loaded both are held with OCR off (`Titles::Held`), and the
            // pack check asks for them ([`PackGroup::EndTitles`]).
            Read::EndTitle => true,
            // #265 (O5): the map pack reads the accolade map line. Without
            // a map pack and with OCR off there is no map and no OCR call
            // (`choose_accolade_map` in the daemon), and the pack check
            // asks for the map pack ([`PackGroup::MapNames`]).
            Read::AccoladeMap => true,
            // Both halves are in: ADLERSBRUNN from a sure banner read
            // (#271), and both teams counted on the new reader's board
            // (#266). One limit stays, and is written where the member
            // decides: the word exists only on a sure banner read. A co-op
            // board whose banner is not read sure is not dropped (with OCR
            // on, the label OCR is a second chance to read the word).
            Read::PveBoard => true,
        }
    }

    /// The read in a member's words, as it goes into a sentence.
    pub const fn plain(self) -> &'static str {
        match self {
            Read::StatCells => "the scoreboard numbers",
            Read::OwnRow => "which row is yours",
            Read::Hero => "your hero",
            Read::TopBarMap => "the map on the scoreboard",
            Read::ResultWord => "the result on the end screens",
            Read::EndTitle => "the end-of-match title",
            Read::FinalScore => "the final score line",
            Read::MatchStart => "the match-start screens",
            Read::AccoladeMap => "the map on the end screen",
            Read::PveBoard => "whether a scoreboard is a co-op event board",
        }
    }

    /// The reads this build still needs Tesseract for.
    pub fn needing_ocr() -> Vec<Read> {
        Read::ALL
            .into_iter()
            .filter(|read| !read.without_ocr())
            .collect()
    }
}

// ------------------------------------------------------------------ packs

/// One thing a reader without an OCR fallback needs loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackGroup {
    /// A hero pack for boards of every size: heroes-v2 or heroes-v1.
    /// heroes-v2-1080 alone reads no 1440p board. Two reads stand on it:
    /// the hero ([`Read::Hero`]) and, on a board with no highlight and no
    /// seed match, the own row ([`Read::OwnRow`]: the row the career
    /// panel's hero is read on).
    HeroIcons,
    /// maps-v1 (the hand-installed `templates/maps` folder does not count).
    MapNames,
    /// words-v1: VICTORY and DEFEAT for the accolade screen and for the
    /// rank screen.
    ResultWords,
    /// words-v1: every match-start title the phase detector reads.
    PhaseTitles,
    /// words-v1: the centred end title, a VICTORY and a DEFEAT template
    /// (`end_title_*`). The end-of-match wake reads the same templates.
    EndTitles,
}

/// Every process-wide read of the new reader (`shadow::read_*`) and the
/// groups it cannot work without. A read that is missing here has no place
/// in [`PackReadiness`], so a test compares this list with the functions in
/// `shadow/reader.rs`.
pub const READER_ENTRIES: &[(&str, &[PackGroup])] = &[
    ("read_board", &[PackGroup::HeroIcons, PackGroup::MapNames]),
    (
        "read_board_with",
        &[PackGroup::HeroIcons, PackGroup::MapNames],
    ),
    (
        "read_board_with_banner",
        &[PackGroup::HeroIcons, PackGroup::MapNames],
    ),
    ("read_top_bar", &[PackGroup::MapNames]),
    ("read_banner", &[PackGroup::MapNames]),
    ("read_accolade_map", &[PackGroup::MapNames]),
    ("read_heroes", &[PackGroup::HeroIcons]),
    ("read_result", &[PackGroup::ResultWords]),
    ("read_result_screens", &[PackGroup::ResultWords]),
    // The FINAL SCORE line only opens the gate of the end title's OCR
    // read (#263). With OCR off nothing asks for it, so it needs no group.
    ("read_final_score", &[]),
    ("read_end_title", &[PackGroup::EndTitles]),
    ("read_phrase", &[PackGroup::PhaseTitles]),
];

/// What the readers without an OCR fallback have loaded, as one reader sees
/// it ([`Reader::pack_readiness`]).
///
/// It is part of the status file, which the app and the tracker may read and
/// write at different versions (an update without a restart): a field that
/// is missing reads as `false`, and an unknown field is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PackReadiness {
    /// [`PackGroup::HeroIcons`].
    pub heroes: bool,
    /// [`PackGroup::MapNames`].
    pub maps: bool,
    /// words-v1 loads at all.
    pub words: bool,
    /// [`PackGroup::ResultWords`].
    pub result_words: bool,
    /// [`PackGroup::PhaseTitles`].
    pub phase_titles: bool,
    /// [`PackGroup::EndTitles`].
    pub end_titles: bool,
}

/// One pack OCR-off cannot run without, and what is wrong with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackProblem {
    HeroesMissing,
    MapsMissing,
    WordsMissing,
    /// words-v1 is there, without the accolade or rank screen result words.
    WordsLackResultWords,
    /// words-v1 is there, without a match-start title.
    WordsLackPhaseTitles,
    /// words-v1 is there, without the end-title templates.
    WordsLackEndTitles,
}

impl PackProblem {
    /// The pack file on the pack list (for logs, not for members).
    pub const fn pack(self) -> &'static str {
        match self {
            Self::HeroesMissing => "heroes-v2 or heroes-v1",
            Self::MapsMissing => "maps-v1",
            _ => "words-v1",
        }
    }

    /// "the map pack is not installed", in a member's words.
    pub const fn sentence(self) -> &'static str {
        match self {
            Self::HeroesMissing => "the hero pack is not installed",
            Self::MapsMissing => "the map pack is not installed",
            Self::WordsMissing => "the word pack is not installed",
            Self::WordsLackResultWords => {
                "the word pack is out of date (it has no result words for the end screens)"
            }
            Self::WordsLackPhaseTitles => {
                "the word pack is out of date (it has no match-start titles)"
            }
            Self::WordsLackEndTitles => {
                "the word pack is out of date (it has no end-of-match titles)"
            }
        }
    }

    /// What stays unread while this pack is the problem.
    pub const fn held(self) -> &'static str {
        match self {
            Self::HeroesMissing => "heroes",
            Self::MapsMissing => "the map",
            Self::WordsMissing => "results and match starts",
            Self::WordsLackResultWords => "results",
            Self::WordsLackPhaseTitles => "match starts",
            Self::WordsLackEndTitles => "the end-of-match title",
        }
    }
}

impl PackReadiness {
    /// The packs under `data_dir`, loaded the way the daemon's reader loads
    /// them. Decodes the pack folders, so the app runs it off its UI thread.
    pub fn on_disk(data_dir: &Path) -> Self {
        Reader::load(&ReaderConfig::from_data_dir(data_dir)).pack_readiness()
    }

    /// Whether `group` is loaded.
    pub const fn has(&self, group: PackGroup) -> bool {
        match group {
            PackGroup::HeroIcons => self.heroes,
            PackGroup::MapNames => self.maps,
            PackGroup::ResultWords => self.result_words,
            PackGroup::PhaseTitles => self.phase_titles,
            PackGroup::EndTitles => self.end_titles,
        }
    }

    /// Every pack problem, in pack-list order. Empty when the packs are
    /// what OCR-off needs.
    pub fn problems(&self) -> Vec<PackProblem> {
        let mut out = Vec::new();
        if !self.heroes {
            out.push(PackProblem::HeroesMissing);
        }
        if !self.maps {
            out.push(PackProblem::MapsMissing);
        }
        if !self.words {
            out.push(PackProblem::WordsMissing);
        } else {
            if !self.result_words {
                out.push(PackProblem::WordsLackResultWords);
            }
            if !self.phase_titles {
                out.push(PackProblem::WordsLackPhaseTitles);
            }
            if !self.end_titles {
                out.push(PackProblem::WordsLackEndTitles);
            }
        }
        out
    }

    pub fn ready(&self) -> bool {
        self.problems().is_empty()
    }

    /// `(ready, needed)` over the three packs. An out-of-date word pack is
    /// not ready.
    pub fn counts(&self) -> (usize, usize) {
        let words = self.words && self.result_words && self.phase_titles && self.end_titles;
        let ready = [self.heroes, self.maps, words]
            .into_iter()
            .filter(|ok| *ok)
            .count();
        (ready, 3)
    }

    /// "Reader packs ready: 2 of 3".
    pub fn status_line(&self) -> String {
        let (ready, needed) = self.counts();
        format!("Reader packs ready: {ready} of {needed}")
    }

    /// The line the app shows while the daemon runs with OCR off and a pack
    /// is gone or out of date, or `None` when the packs are fine.
    pub fn hold_banner(&self) -> Option<String> {
        let problems = self.problems();
        if problems.is_empty() {
            return None;
        }
        let held = dedup(problems.iter().map(|p| p.held()));
        let verb = if held == ["the map"] { "is" } else { "are" };
        Some(format!(
            "Reading on hold: {}, so {} {verb} not read. Classic OCR stays off. {HOLD_FIX}",
            join_list(problems.iter().map(|p| p.sentence())),
            join_list(held.into_iter()),
        ))
    }
}

/// How a member gets the packs after a refused click.
pub const CLICK_FIX: &str = "Sign in and download the reader packs (Settings, Open setup guide), then turn classic OCR off again.";
/// How a member gets the packs after the tracker refused at start.
pub const DAEMON_FIX: &str =
    "Sign in and download the reader packs (Settings, Open setup guide), then restart the tracker.";
/// Said when this build is what stands in the way.
pub const LATER_VERSION: &str = "A later version can turn it off.";
/// After [`LATER_VERSION`] when the tracker refused at start: the switch is
/// off in the file, and this is how the member clears the message.
pub const TURN_BACK_ON: &str =
    "To clear this message, turn Use classic OCR (Tesseract) on in Settings and save.";
/// What a member can do while a pack is gone with OCR off.
pub const HOLD_FIX: &str =
    "Download the reader packs again (Settings, Open setup guide), or turn classic OCR on.";
/// Shown when the packs came back after the daemon refused for them.
pub const RESTART_TO_APPLY: &str =
    "The reader packs are ready now. Restart the tracker to turn classic OCR off.";

fn dedup<'a>(parts: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut seen: Vec<&str> = Vec::new();
    for part in parts {
        if !seen.contains(&part) {
            seen.push(part);
        }
    }
    seen
}

fn join_list<'a>(parts: impl Iterator<Item = &'a str>) -> String {
    let parts: Vec<&str> = parts.collect();
    match parts.as_slice() {
        [] => String::new(),
        [one] => (*one).to_string(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

fn capitalised(sentence: &str) -> String {
    let mut chars = sentence.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

// --------------------------------------------------------------- decision

/// Why `ocr = "off"` is not granted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Refusal {
    /// Reads this build still needs Tesseract for. No pack fixes these.
    #[serde(deserialize_with = "known_only")]
    pub reads: Vec<Read>,
    /// Packs that are missing or out of date.
    #[serde(deserialize_with = "known_only")]
    pub packs: Vec<PackProblem>,
}

/// A list from the status file, without the entries this version does not
/// know (a read or a pack problem a later tracker added). One unknown word
/// must not make the whole file unreadable.
fn known_only<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let raw = Vec::<serde_json::Value>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect())
}

/// Who a refusal is worded for: the next step differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// The member just clicked the switch in Settings.
    Click,
    /// The tracker refused at start: the switch is already off in the file.
    Daemon,
}

impl Refusal {
    pub fn is_empty(&self) -> bool {
        self.reads.is_empty() && self.packs.is_empty()
    }

    /// The reason, with what to do next.
    pub fn text(&self, audience: Audience) -> String {
        let mut out = String::from("Classic OCR stays on:");
        if !self.reads.is_empty() {
            out.push_str(&format!(
                " this tracker version cannot read {} without it yet.",
                join_list(self.reads.iter().map(|r| r.plain()))
            ));
        }
        if !self.packs.is_empty() {
            let sentence = join_list(self.packs.iter().map(|p| p.sentence()));
            if self.reads.is_empty() {
                out.push_str(&format!(" {sentence}."));
            } else {
                out.push_str(&format!(" {}.", capitalised(&sentence)));
            }
        }
        // While this build stands in the way no pack turns it off, so no
        // pack is asked for.
        let next = match (self.reads.is_empty(), audience) {
            (false, Audience::Click) => LATER_VERSION.to_string(),
            (false, Audience::Daemon) => format!("{LATER_VERSION} {TURN_BACK_ON}"),
            (true, Audience::Click) => CLICK_FIX.to_string(),
            (true, Audience::Daemon) => DAEMON_FIX.to_string(),
        };
        out.push(' ');
        out.push_str(&next);
        out
    }
}

/// What the daemon runs with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// What config (or the env override) asked for.
    pub requested: OcrSetting,
    /// What runs. On when off was refused.
    pub effective: OcrSetting,
    /// Why `effective` is on although `requested` is off.
    pub refusal: Option<Refusal>,
}

impl Decision {
    /// The reader that supplies saved stats: `ocr = "off"` implies the new
    /// reader, otherwise `reader` decides. The daemon, the shadow log and
    /// the problem report all take it from [`effective_reader`].
    pub fn reader(&self, config: &Config) -> ReaderSetting {
        effective_reader(self.effective, config.reader)
    }
}

/// The reader in use for an effective OCR setting and the `reader` key.
pub const fn effective_reader(effective: OcrSetting, reader: ReaderSetting) -> ReaderSetting {
    match effective {
        OcrSetting::Off => ReaderSetting::New,
        OcrSetting::On => reader,
    }
}

/// `requested` checked against this build and the packs. On is always
/// granted. Off is granted only when `needing_ocr` is empty (this build
/// reads everything without Tesseract) and every pack is ready; otherwise
/// OCR stays on and the reasons come back.
pub fn decide(requested: OcrSetting, packs: &PackReadiness, needing_ocr: &[Read]) -> Decision {
    decide_with(requested, packs.problems(), needing_ocr)
}

/// [`decide`] when the packs were not looked at because this build refuses
/// anyway: the refusal names the reads and no pack.
fn decide_for_build(requested: OcrSetting, needing_ocr: &[Read]) -> Decision {
    decide_with(requested, Vec::new(), needing_ocr)
}

fn decide_with(requested: OcrSetting, packs: Vec<PackProblem>, needing_ocr: &[Read]) -> Decision {
    let refusal = match requested {
        OcrSetting::On => None,
        OcrSetting::Off => {
            let refusal = Refusal {
                reads: needing_ocr.to_vec(),
                packs,
            };
            (!refusal.is_empty()).then_some(refusal)
        }
    };
    Decision {
        requested,
        effective: if refusal.is_some() {
            OcrSetting::On
        } else {
            requested
        },
        refusal,
    }
}

// ----------------------------------------------------------------- status

/// What the daemon last wrote about the switch. Values only: settings, pack
/// flags, counts and source locations.
///
/// Read across versions: a missing field takes its default and an unknown
/// one is ignored, so a field added or dropped later does not make the file
/// unreadable to the other side.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    /// The daemon that wrote this.
    pub pid: u32,
    pub requested: OcrSetting,
    pub effective: OcrSetting,
    /// Why the daemon kept OCR on at startup.
    pub refusal: Option<Refusal>,
    /// The packs as the daemon's reader last saw them. Not checked while
    /// `requested` is on.
    pub packs: Option<PackReadiness>,
    /// OCR calls the gate blocked in this daemon run.
    pub blocked_total: u64,
    /// The same, per call site ([`gate::Site::key`]).
    pub blocked: Vec<StatusSite>,
    /// Tesseract instances this daemon run created.
    pub leptess_created: u64,
    /// The daemon's screen share is open. Capture now reads this so it does
    /// not take a portal screenshot beside that share on KDE.
    pub share_open: bool,
    /// Why captures are stopped after a declined screen share. Empty when
    /// the share was not declined. The app shows this in Tracker health.
    pub share_notice: String,
    /// Why the capture route asked for is not the one in use (KDE direct
    /// asked for, KWin did not answer). Empty otherwise. The app shows it
    /// under the capture method in Settings.
    pub capture_notice: String,
}

/// One blocked call site in [`Status`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusSite {
    /// `file:line function`, a place in the tracker's source.
    pub site: String,
    pub count: u64,
}

impl Status {
    /// The line for the app's health panel and Settings, or `None` when
    /// there is nothing to say (OCR on as asked, or off with every pack and
    /// no read stopped at the gate).
    pub fn notice(&self) -> Option<String> {
        if self.requested.is_on() {
            return None;
        }
        match (&self.refusal, &self.packs) {
            (Some(refusal), Some(packs)) if refusal.reads.is_empty() && packs.ready() => {
                Some(RESTART_TO_APPLY.to_string())
            }
            (Some(refusal), _) => Some(refusal.text(Audience::Daemon)),
            (None, Some(packs)) if !packs.ready() => packs.hold_banner(),
            (None, _) if self.blocked_total > 0 => Some(format!(
                "Classic OCR is off, and {} read{} still asked for it and got nothing. \
                 What they feed is not saved. Please send a problem report.",
                self.blocked_total,
                if self.blocked_total == 1 { "" } else { "s" }
            )),
            (None, _) => None,
        }
    }

    /// The part of [`Self::notice`] that belongs in Tracker health: only
    /// what is wrong while the tracker runs with OCR off (a pack on hold, a
    /// read that was stopped). A refused `off` is not a health problem: the
    /// tracker runs classic OCR and reads everything, so health stays as it
    /// is and the reason is shown in Settings.
    pub fn health_notice(&self) -> Option<String> {
        self.effective.is_off().then(|| self.notice()).flatten()
    }
}

pub fn status_path(data_dir: &Path) -> PathBuf {
    data_dir.join(STATUS_FILE)
}

/// The status the daemon last wrote, or `None` when there is none or it
/// does not parse.
pub fn read_status(data_dir: &Path) -> Option<Status> {
    let bytes = std::fs::read(status_path(data_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

static DAEMON_SHARE_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SHARE_NOTICE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
static CAPTURE_NOTICE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
static STATUS_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn daemon_share_open() -> bool {
    DAEMON_SHARE_OPEN.load(std::sync::atomic::Ordering::Acquire)
}

pub fn share_notice() -> String {
    SHARE_NOTICE
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

pub fn capture_notice() -> String {
    CAPTURE_NOTICE
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
}

/// Write the capture-route sentence into the status file (empty clears
/// it), keeping everything else there. Later status writes keep it too.
pub fn publish_capture_notice(data_dir: &Path, notice: &str) {
    let _guard = STATUS_WRITE.lock().unwrap_or_else(|err| err.into_inner());
    *CAPTURE_NOTICE.lock().unwrap_or_else(|err| err.into_inner()) = notice.to_string();
    let mut status = read_status(data_dir).unwrap_or_default();
    if status.capture_notice == notice {
        return;
    }
    status.capture_notice = notice.to_string();
    let _ = write_status(data_dir, &status);
}

/// The in-memory notice. The status file is updated by
/// [`publish_share_notice`].
pub fn set_share_notice(notice: &str) {
    *SHARE_NOTICE.lock().unwrap_or_else(|err| err.into_inner()) = notice.to_string();
}

/// Remember whether the daemon's share is open and write it into the status
/// file now, so the app does not wait for the next OCR watch. The decline
/// sentence is kept.
pub fn publish_share_open(data_dir: &Path, open: bool) {
    DAEMON_SHARE_OPEN.store(open, std::sync::atomic::Ordering::Release);
    let _guard = STATUS_WRITE.lock().unwrap_or_else(|err| err.into_inner());
    let notice = share_notice();
    let mut status = read_status(data_dir).unwrap_or_default();
    status.share_open = daemon_share_open();
    status.share_notice = notice;
    let _ = write_status(data_dir, &status);
}

/// Write the decline sentence into the status file. An empty notice clears
/// it. OCR republish reads the same memory, so it does not wipe the line.
pub fn publish_share_notice(data_dir: &Path, notice: &str) {
    let _guard = STATUS_WRITE.lock().unwrap_or_else(|err| err.into_inner());
    let notice = {
        let mut slot = SHARE_NOTICE.lock().unwrap_or_else(|err| err.into_inner());
        *slot = notice.to_string();
        slot.clone()
    };
    let mut status = read_status(data_dir).unwrap_or_default();
    status.share_open = daemon_share_open();
    status.share_notice = notice;
    let _ = write_status(data_dir, &status);
}

/// Write `status` next to the other daemon files (temp file, then rename).
pub fn write_status(data_dir: &Path, status: &Status) -> std::io::Result<()> {
    let path = status_path(data_dir);
    let tmp = data_dir.join(STATUS_TMP_FILE);
    let bytes = serde_json::to_vec_pretty(status).map_err(std::io::Error::other)?;
    let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, &path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

// ----------------------------------------------------------------- switch

/// True when daemon startup should give the reader packs that are still
/// downloading a moment before it decides: off is asked for, this build
/// could grant it, and a pack is missing. Startup syncs heroes-v1 first and
/// the other packs in the background, so without the wait a first start
/// refuses off for the whole run.
pub fn needs_pack_wait(config: &Config) -> bool {
    pack_wait_applies(
        config.ocr_control().requested(),
        &Read::needing_ocr(),
        || PackReadiness::on_disk(&config.data_dir),
    )
}

/// [`needs_pack_wait`] with its inputs as arguments. `packs` is looked at
/// only when off is asked for and this build could grant it.
pub fn pack_wait_applies(
    requested: OcrSetting,
    needing_ocr: &[Read],
    packs: impl FnOnce() -> PackReadiness,
) -> bool {
    requested.is_off() && needing_ocr.is_empty() && !packs().ready()
}

/// The daemon's switch state for one run.
pub struct Switch {
    pub decision: Decision,
    data_dir: PathBuf,
    /// The reader the packs are asked of when off was refused and the
    /// process-wide reader is not this one. It re-loads a pack whose folder
    /// changed, like the process-wide one.
    own_reader: Option<Reader>,
    /// The packs as last seen. `None` when they were never looked at: OCR
    /// asked on, or a build that refuses off whatever the packs are.
    packs: Option<PackReadiness>,
    written: Option<Status>,
    /// The last write of the status file failed (said once at WARN).
    write_failed: bool,
}

impl std::fmt::Debug for Switch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Switch")
            .field("decision", &self.decision)
            .field("packs", &self.packs)
            .finish_non_exhaustive()
    }
}

impl Switch {
    /// Daemon startup: read the request, check this build and the packs
    /// when it is off, set the gate, log the outcome and write the status
    /// file.
    ///
    /// The packs are loaded at most once, and only when they can change the
    /// answer: off is asked for and this build could grant it. A build that
    /// refuses anyway loads nothing and watches nothing.
    ///
    /// When off is granted, or refused for a pack while `reader = "new"`,
    /// the reader that was loaded becomes the process-wide one
    /// ([`crate::shadow::install`]), so the daemon does not load the packs a
    /// second time. Refused for a pack under `reader = "ocr-v1"`, the reader
    /// stays with the switch, only to see the packs arrive.
    pub fn start(config: &Config) -> Self {
        Self::start_with(config, &Read::needing_ocr(), |data_dir| {
            Reader::load(&ReaderConfig::from_data_dir(data_dir))
        })
    }

    /// [`Self::start`] with the reads this build still needs OCR for and
    /// the reader as arguments (tests). `reader` runs only when off is
    /// requested and `needing_ocr` is empty.
    pub fn start_with(
        config: &Config,
        needing_ocr: &[Read],
        reader: impl FnOnce(&Path) -> Reader,
    ) -> Self {
        let requested = config.ocr_control().requested();
        let reader =
            (requested.is_off() && needing_ocr.is_empty()).then(|| reader(&config.data_dir));
        let packs = reader.as_ref().map(Reader::pack_readiness);
        let decision = match &packs {
            Some(packs) => decide(requested, packs, needing_ocr),
            None => decide_for_build(requested, needing_ocr),
        };
        let own_reader = match reader {
            Some(reader) if decision.reader(config).is_new() => {
                // The new reader reads this run (off was granted, or it was
                // refused for a pack and `reader = "new"`): this is the
                // reader. When one was installed before (a second start in
                // one process), that one stays, and the watch asks it.
                crate::shadow::install(reader);
                None
            }
            other => other,
        };
        let mut switch = Self {
            decision,
            data_dir: config.data_dir.clone(),
            own_reader,
            packs,
            written: None,
            write_failed: false,
        };
        switch.apply();
        switch.publish();
        switch
    }

    /// Set the gate and say what was decided.
    fn apply(&self) {
        match (&self.decision.effective, &self.decision.refusal) {
            (OcrSetting::Off, _) => {
                gate::set_mode(OcrMode::Off);
                tracing::info!(
                    "classic OCR is off (ocr = \"off\"): the new reader reads everything and Tesseract is never started"
                );
            }
            (OcrSetting::On, Some(refusal)) => {
                gate::set_mode(OcrMode::On);
                tracing::warn!(
                    reason = %refusal.text(Audience::Daemon),
                    packs = ?refusal.packs.iter().map(|p| p.pack()).collect::<Vec<_>>(),
                    reads = ?refusal.reads,
                    "ocr = \"off\" was refused: classic OCR stays on for this run"
                );
            }
            (OcrSetting::On, None) => gate::set_mode(OcrMode::On),
        }
    }

    /// True when the daemon runs without Tesseract. It then reads with the
    /// new reader whatever `reader` says ([`Decision::reader`]).
    pub fn is_off(&self) -> bool {
        self.decision.effective.is_off()
    }

    fn status(&self) -> Status {
        // Two places can share a key (calls from outside the source
        // folder are all `other.rs:1`): one line per key, counts added.
        let mut by_key: std::collections::BTreeMap<String, u64> = Default::default();
        for b in gate::blocked_calls() {
            let count = by_key.entry(b.site.key()).or_default();
            *count = count.saturating_add(b.count);
        }
        Status {
            pid: std::process::id(),
            requested: self.decision.requested,
            effective: self.decision.effective,
            refusal: self.decision.refusal.clone(),
            packs: self.packs,
            blocked_total: gate::blocked_total(),
            blocked: by_key
                .into_iter()
                .map(|(site, count)| StatusSite { site, count })
                .collect(),
            leptess_created: gate::leptess_created(),
            share_open: daemon_share_open(),
            share_notice: share_notice(),
            capture_notice: capture_notice(),
        }
    }

    /// Write the status file when it changed since the last write.
    fn publish(&mut self) {
        let _guard = STATUS_WRITE.lock().unwrap_or_else(|err| err.into_inner());
        let status = self.status();
        if self.written.as_ref() == Some(&status) {
            return;
        }
        match write_status(&self.data_dir, &status) {
            Ok(()) => {
                self.written = Some(status);
                self.write_failed = false;
            }
            // Once per failure, not on every tick: without the file the app
            // cannot say what the tracker decided.
            Err(e) if !self.write_failed => {
                self.write_failed = true;
                tracing::warn!(
                    error = %e,
                    file = STATUS_FILE,
                    "could not write the OCR switch status; the app will not show what the tracker decided"
                );
            }
            Err(e) => tracing::debug!(error = %e, "could not write the OCR switch status"),
        }
    }

    /// The packs right now, as the reader in use (or, after a refusal, the
    /// switch's own reader) has them loaded.
    fn packs_now(&self) -> PackReadiness {
        match &self.own_reader {
            Some(reader) => reader.pack_readiness(),
            None => crate::shadow::pack_readiness(),
        }
    }

    /// One look at the packs and the counters. Logs when the pack state
    /// changes and rewrites the status file when anything did. The daemon
    /// also calls it once when it shuts down, so the file holds the run's
    /// last counts, with OCR on as well.
    pub fn check(&mut self) {
        if self.watches() {
            let now = self.packs_now();
            if self.packs != Some(now) {
                self.log_pack_change(&now);
                self.packs = Some(now);
            }
        }
        self.publish();
    }

    fn log_pack_change(&self, now: &PackReadiness) {
        if self.is_off() {
            match now.hold_banner() {
                Some(banner) => tracing::warn!(
                    packs = ?now.problems().iter().map(|p| p.pack()).collect::<Vec<_>>(),
                    "{banner} (the tracker does not fall back to classic OCR while it is off)"
                ),
                None => tracing::info!("the reader packs are back: nothing is on hold"),
            }
        } else if now.ready()
            && self
                .decision
                .refusal
                .as_ref()
                .is_some_and(|r| r.reads.is_empty())
        {
            tracing::info!("{RESTART_TO_APPLY}");
        }
    }

    /// Whether the watch has anything to do: only when the packs were
    /// looked at, which is when off was asked for and this build could
    /// grant it. Then a pack that goes away is held and named, and packs
    /// that arrive after a refusal are announced ("restart the tracker").
    pub fn watches(&self) -> bool {
        self.packs.is_some()
    }
}

/// The switch as the daemon shares it between the watch and shutdown.
pub type SharedSwitch = std::sync::Arc<std::sync::Mutex<Switch>>;

/// Re-check `switch` every [`WATCH_INTERVAL`] for as long as the daemon
/// runs. Meant for `tokio::spawn`. Ends at once when OCR was asked to stay
/// on: there is nothing to watch then.
pub async fn watch(switch: SharedSwitch) {
    if !switch.lock().unwrap_or_else(|e| e.into_inner()).watches() {
        return;
    }
    let mut timer = tokio::time::interval(WATCH_INTERVAL);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        timer.tick().await;
        // The check stats pack folders and may write the status file: off
        // the async workers.
        let switch = std::sync::Arc::clone(&switch);
        let _ = tokio::task::spawn_blocking(move || {
            switch.lock().unwrap_or_else(|e| e.into_inner()).check()
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const READY: PackReadiness = PackReadiness {
        heroes: true,
        maps: true,
        words: true,
        result_words: true,
        phase_titles: true,
        end_titles: true,
    };

    #[test]
    fn off_is_granted_only_with_every_pack_and_every_read() {
        let granted = decide(OcrSetting::Off, &READY, &[]);
        assert_eq!(granted.effective, OcrSetting::Off);
        assert_eq!(granted.refusal, None);
        assert_eq!(READY.counts(), (3, 3));
        assert_eq!(READY.status_line(), "Reader packs ready: 3 of 3");

        let cases = [
            (
                PackReadiness {
                    heroes: false,
                    ..READY
                },
                PackProblem::HeroesMissing,
                PackGroup::HeroIcons,
                "the hero pack is not installed",
            ),
            (
                PackReadiness {
                    maps: false,
                    ..READY
                },
                PackProblem::MapsMissing,
                PackGroup::MapNames,
                "the map pack is not installed",
            ),
            (
                PackReadiness {
                    words: false,
                    result_words: false,
                    phase_titles: false,
                    end_titles: false,
                    ..READY
                },
                PackProblem::WordsMissing,
                PackGroup::ResultWords,
                "the word pack is not installed",
            ),
            (
                PackReadiness {
                    result_words: false,
                    ..READY
                },
                PackProblem::WordsLackResultWords,
                PackGroup::ResultWords,
                "no result words",
            ),
            (
                PackReadiness {
                    phase_titles: false,
                    ..READY
                },
                PackProblem::WordsLackPhaseTitles,
                PackGroup::PhaseTitles,
                "no match-start titles",
            ),
            (
                PackReadiness {
                    end_titles: false,
                    ..READY
                },
                PackProblem::WordsLackEndTitles,
                PackGroup::EndTitles,
                "no end-of-match titles",
            ),
        ];
        for (packs, problem, group, named) in cases {
            let refused = decide(OcrSetting::Off, &packs, &[]);
            assert_eq!(refused.requested, OcrSetting::Off);
            assert_eq!(refused.effective, OcrSetting::On, "{packs:?} keeps OCR on");
            let refusal = refused.refusal.expect("a refusal carries its reasons");
            assert_eq!(refusal.packs, [problem]);
            assert!(refusal.reads.is_empty());
            let click = refusal.text(Audience::Click);
            assert!(click.starts_with("Classic OCR stays on: "), "{click}");
            assert!(click.contains(named), "{click}");
            assert!(click.ends_with(CLICK_FIX), "{click}");
            let daemon = refusal.text(Audience::Daemon);
            assert!(daemon.ends_with(DAEMON_FIX), "{daemon}");
            assert!(!daemon.contains("turn classic OCR off again"), "{daemon}");
            assert_eq!(packs.counts(), (2, 3), "{packs:?}");
            assert!(!packs.has(group), "{packs:?}");
        }
    }

    #[test]
    fn off_is_refused_while_this_build_needs_ocr_for_a_read() {
        // Every pack is there. One read without a replacement is enough.
        for read in Read::ALL {
            let refused = decide(OcrSetting::Off, &READY, &[read]);
            assert_eq!(refused.effective, OcrSetting::On, "{read:?}");
            let refusal = refused.refusal.expect("the reason");
            assert_eq!(refusal.reads, [read]);
            assert!(refusal.packs.is_empty());
            let reason = format!(
                "Classic OCR stays on: this tracker version cannot read {} without it yet. {LATER_VERSION}",
                read.plain()
            );
            assert_eq!(refusal.text(Audience::Click), reason);
            // At start the switch is already off in the file: the member is
            // told how to clear the message.
            assert_eq!(
                refusal.text(Audience::Daemon),
                format!("{reason} {TURN_BACK_ON}")
            );
        }
        // Reads and packs together: both are named, and the member is not
        // sent to download packs that would not turn it off.
        let both = decide(
            OcrSetting::Off,
            &PackReadiness {
                maps: false,
                ..READY
            },
            &[Read::StatCells, Read::ResultWord],
        )
        .refusal
        .unwrap();
        assert_eq!(
            both.text(Audience::Daemon),
            format!(
                "Classic OCR stays on: this tracker version cannot read the scoreboard numbers \
                 and the result on the end screens without it yet. The map pack is not installed. \
                 {LATER_VERSION} {TURN_BACK_ON}"
            )
        );
        // A build that refuses does not look at the packs, and names none.
        let for_build = decide_for_build(OcrSetting::Off, &[Read::PveBoard]);
        assert_eq!(for_build.effective, OcrSetting::On);
        let refusal = for_build.refusal.expect("the reason");
        assert!(refusal.packs.is_empty());
        assert_eq!(
            refusal.text(Audience::Click),
            format!(
                "Classic OCR stays on: this tracker version cannot read whether a scoreboard \
                 is a co-op event board without it yet. {LATER_VERSION}"
            )
        );
        assert_eq!(
            decide_for_build(OcrSetting::On, &[Read::PveBoard]).refusal,
            None
        );
    }

    /// What this build can do, and the one place that pins it. Each line
    /// of [`Read::without_ocr`] moved when its lane merged; the decision
    /// follows from the list. Taking a read back is its line there and its
    /// name in `STILL_NEEDS_OCR` here: every other test follows the flags.
    #[test]
    fn this_build_reads_everything_without_ocr() {
        const STILL_NEEDS_OCR: &[Read] = &[];
        assert_eq!(Read::needing_ocr(), STILL_NEEDS_OCR);
        let decision = decide(OcrSetting::Off, &READY, &Read::needing_ocr());
        if STILL_NEEDS_OCR.is_empty() {
            assert_eq!(
                decision.effective,
                OcrSetting::Off,
                "with every pack installed, this build turns OCR off"
            );
            assert_eq!(decision.refusal, None);
        } else {
            assert_eq!(decision.effective, OcrSetting::On);
            assert_eq!(
                decision.refusal.unwrap().reads,
                Read::needing_ocr(),
                "with every pack installed, off is still refused on this build"
            );
        }
        for read in Read::ALL {
            assert!(!read.plain().is_empty());
        }
    }

    /// The condition of [`Read::OwnRow`]. Without OCR the own row of a
    /// board with no highlight and no seed match is the row the career
    /// panel's hero is read on, and that needs a hero pack that reads the
    /// board. Without one, off is refused whatever the build can do, so OCR
    /// stays on and the name read is still there for that board.
    #[test]
    fn without_a_hero_pack_off_is_refused_so_the_own_row_keeps_its_name_read() {
        let no_heroes = PackReadiness {
            heroes: false,
            ..READY
        };
        // even for a build with no read left on OCR
        for needing in [&[][..], &Read::needing_ocr()[..]] {
            let refused = decide(OcrSetting::Off, &no_heroes, needing);
            assert_eq!(refused.effective, OcrSetting::On);
            let refusal = refused.refusal.expect("the reason");
            assert_eq!(refusal.packs, [PackProblem::HeroesMissing]);
        }
        assert!(!no_heroes.has(PackGroup::HeroIcons));
        // with the pack, the own row is no reason to keep OCR on
        let with_heroes = decide(OcrSetting::Off, &READY, &[]);
        assert_eq!(with_heroes.effective, OcrSetting::Off);
        assert!(READY.has(PackGroup::HeroIcons));
    }

    #[test]
    fn on_is_always_granted_and_never_looks_at_the_packs_or_the_build() {
        let none = PackReadiness::default();
        let on = decide(OcrSetting::On, &none, &Read::ALL);
        assert_eq!(on.effective, OcrSetting::On);
        assert_eq!(on.refusal, None);
        assert_eq!(none.counts(), (0, 3));
    }

    #[test]
    fn off_implies_the_new_reader_and_on_leaves_the_reader_key() {
        use ReaderSetting::{New, OcrV1};
        assert_eq!(effective_reader(OcrSetting::Off, OcrV1), New);
        assert_eq!(effective_reader(OcrSetting::Off, New), New);
        assert_eq!(effective_reader(OcrSetting::On, OcrV1), OcrV1);
        assert_eq!(effective_reader(OcrSetting::On, New), New);
        // a file that chose ocr-v1 (missing `reader` is the new reader)
        let config = Config {
            reader: OcrV1,
            ..Config::default()
        };
        assert_eq!(decide(OcrSetting::Off, &READY, &[]).reader(&config), New);
        // a refused off runs with the file's reader
        assert_eq!(
            decide(OcrSetting::Off, &READY, &[Read::StatCells]).reader(&config),
            OcrV1
        );
    }

    #[test]
    fn startup_waits_for_packs_only_when_that_could_turn_ocr_off() {
        let missing = || PackReadiness::default();
        let never = || -> PackReadiness { panic!("the packs were looked at") };
        assert!(pack_wait_applies(OcrSetting::Off, &[], missing));
        assert!(!pack_wait_applies(OcrSetting::Off, &[], || READY));
        // OCR on, or a build that refuses off anyway: no wait, no pack load
        assert!(!pack_wait_applies(OcrSetting::On, &[], never));
        assert!(!pack_wait_applies(
            OcrSetting::Off,
            &[Read::StatCells],
            never
        ));
    }

    #[test]
    fn the_refusal_names_every_missing_pack_once() {
        let none = PackReadiness::default();
        let refusal = decide(OcrSetting::Off, &none, &[]).refusal.unwrap();
        assert_eq!(
            refusal.text(Audience::Click),
            format!(
                "Classic OCR stays on: the hero pack is not installed, the map pack is not \
                 installed and the word pack is not installed. {CLICK_FIX}"
            )
        );
        // a missing word pack is not also reported as out of date
        assert_eq!(
            none.problems(),
            [
                PackProblem::HeroesMissing,
                PackProblem::MapsMissing,
                PackProblem::WordsMissing
            ]
        );
        // an old word pack lists each thing it lacks
        let old = PackReadiness {
            result_words: false,
            phase_titles: false,
            end_titles: false,
            ..READY
        };
        assert_eq!(
            old.problems(),
            [
                PackProblem::WordsLackResultWords,
                PackProblem::WordsLackPhaseTitles,
                PackProblem::WordsLackEndTitles
            ]
        );
        assert_eq!(old.counts(), (2, 3));
    }

    #[test]
    fn member_text_uses_plain_pack_names() {
        let texts = [
            PackReadiness::default()
                .hold_banner()
                .expect("nothing loaded"),
            decide(OcrSetting::Off, &PackReadiness::default(), &Read::ALL)
                .refusal
                .unwrap()
                .text(Audience::Daemon),
            decide(OcrSetting::Off, &PackReadiness::default(), &[])
                .refusal
                .unwrap()
                .text(Audience::Click),
        ];
        for text in texts {
            for id in ["maps-v1", "words-v1", "heroes-v", "FINAL SCORE", "p4_"] {
                assert!(!text.contains(id), "{id} in member text: {text}");
            }
            assert!(!text.contains('—') && !text.contains('–'), "{text}");
        }
        // the log still gets the pack ids
        assert_eq!(PackProblem::MapsMissing.pack(), "maps-v1");
        assert_eq!(PackProblem::WordsLackPhaseTitles.pack(), "words-v1");
    }

    #[test]
    fn the_hold_banner_says_what_is_not_read_and_never_offers_ocr_by_itself() {
        assert_eq!(READY.hold_banner(), None);
        let no_maps = PackReadiness {
            maps: false,
            ..READY
        };
        assert_eq!(
            no_maps.hold_banner().unwrap(),
            format!(
                "Reading on hold: the map pack is not installed, so the map is not read. \
                 Classic OCR stays off. {HOLD_FIX}"
            )
        );
        let two = PackReadiness {
            heroes: false,
            end_titles: false,
            ..READY
        };
        let banner = two.hold_banner().unwrap();
        assert!(
            banner.contains("so heroes and the end-of-match title are not read"),
            "{banner}"
        );
        // two problems that hold the same thing say it once
        let gone = PackReadiness {
            maps: false,
            words: false,
            result_words: false,
            phase_titles: false,
            end_titles: false,
            ..READY
        };
        assert!(
            gone.hold_banner()
                .unwrap()
                .contains("so the map and results and match starts are not read")
        );
        assert_eq!(dedup(["a", "b", "a"].into_iter()), ["a", "b"]);
    }

    fn status(
        requested: OcrSetting,
        refusal: Option<Refusal>,
        packs: Option<PackReadiness>,
    ) -> Status {
        Status {
            pid: 7,
            requested,
            effective: if refusal.is_some() {
                OcrSetting::On
            } else {
                requested
            },
            refusal,
            packs,
            blocked_total: 0,
            blocked: Vec::new(),
            leptess_created: 0,
            share_open: false,
            share_notice: String::new(),
            capture_notice: String::new(),
        }
    }

    #[test]
    fn the_notice_follows_the_daemon_state() {
        // OCR on as asked: nothing to say, whatever the packs are.
        assert_eq!(status(OcrSetting::On, None, None).notice(), None);
        // Off and every pack loaded: nothing to say.
        assert_eq!(status(OcrSetting::Off, None, Some(READY)).notice(), None);
        // Refused at startup for a pack: the reason, in the daemon's words.
        let missing = PackReadiness {
            maps: false,
            ..READY
        };
        let for_pack = decide(OcrSetting::Off, &missing, &[]).refusal.unwrap();
        assert_eq!(
            status(OcrSetting::Off, Some(for_pack.clone()), Some(missing)).notice(),
            Some(for_pack.text(Audience::Daemon))
        );
        // Refused for a pack, and the packs arrived since: a restart applies it.
        assert_eq!(
            status(OcrSetting::Off, Some(for_pack), Some(READY)).notice(),
            Some(RESTART_TO_APPLY.to_string())
        );
        // Refused because this build needs OCR: a restart changes nothing,
        // so the reason stays even with every pack.
        let for_build = decide(OcrSetting::Off, &READY, &[Read::StatCells])
            .refusal
            .unwrap();
        assert_eq!(
            status(OcrSetting::Off, Some(for_build.clone()), Some(READY)).notice(),
            Some(for_build.text(Audience::Daemon))
        );
        // Running with OCR off and a pack went away: the hold banner.
        assert_eq!(
            status(OcrSetting::Off, None, Some(missing)).notice(),
            missing.hold_banner()
        );
        // Running with OCR off and a read still went to the gate: say so.
        let mut blocked = status(OcrSetting::Off, None, Some(READY));
        blocked.blocked_total = 3;
        let notice = blocked.notice().expect("blocked reads are not silent");
        assert!(notice.contains("3 reads still asked for it"), "{notice}");
        assert!(notice.contains("not saved"), "{notice}");
        blocked.blocked_total = 1;
        assert!(blocked.notice().unwrap().contains("1 read still asked"));
    }

    /// Tracker health takes only what is wrong while OCR is off. A refused
    /// off runs classic OCR and reads everything: health stays "Ready".
    #[test]
    fn a_refused_off_is_not_a_health_problem() {
        let missing = PackReadiness {
            maps: false,
            ..READY
        };
        let for_pack = decide(OcrSetting::Off, &missing, &[]).refusal;
        let for_build = decide_for_build(OcrSetting::Off, &[Read::StatCells]).refusal;
        for refused in [
            status(OcrSetting::Off, for_pack.clone(), Some(missing)),
            status(OcrSetting::Off, for_pack, Some(READY)),
            status(OcrSetting::Off, for_build, None),
        ] {
            assert!(refused.notice().is_some(), "Settings says why");
            assert_eq!(refused.health_notice(), None, "{refused:?}");
        }
        assert_eq!(status(OcrSetting::On, None, None).health_notice(), None);
        assert_eq!(
            status(OcrSetting::Off, None, Some(READY)).health_notice(),
            None
        );
        let held = status(OcrSetting::Off, None, Some(missing));
        assert_eq!(held.health_notice(), missing.hold_banner());
        let mut blocked = status(OcrSetting::Off, None, Some(READY));
        blocked.blocked_total = 2;
        assert_eq!(blocked.health_notice(), blocked.notice());
        assert!(blocked.health_notice().is_some());
    }

    /// The app and the tracker can be of different versions (an update
    /// without a restart). A status file with a field missing, a field this
    /// version does not know, or a read or pack problem it does not know,
    /// still reads.
    #[test]
    fn a_status_file_from_another_version_still_reads() {
        let dir = tempfile::tempdir().unwrap();
        let write = |text: &str| std::fs::write(status_path(dir.path()), text).unwrap();
        // fields missing, one unknown
        write(r#"{"pid": 9, "requested": "off", "effective": "on", "a_later_field": [1, 2]}"#);
        let old = read_status(dir.path()).expect("a file with fields missing");
        assert_eq!(
            (old.pid, old.requested, old.effective),
            (9, OcrSetting::Off, OcrSetting::On)
        );
        assert_eq!(
            (old.refusal.clone(), old.packs, old.blocked_total),
            (None, None, 0)
        );
        assert!(!old.share_open);
        assert!(old.share_notice.is_empty());
        // a pack flag this version does not have, and one it has is missing
        write(
            r#"{"pid": 9, "requested": "off", "effective": "off",
                "packs": {"heroes": true, "maps": true, "end_titles": true}}"#,
        );
        let packs = read_status(dir.path()).unwrap().packs.unwrap();
        assert!(packs.heroes && packs.maps && !packs.words, "{packs:?}");
        // a read and a pack problem a later tracker added
        write(
            r#"{"pid": 9, "requested": "off", "effective": "on",
                "refusal": {"reads": ["stat_cells", "a_later_read"],
                            "packs": ["maps_missing", "a_later_problem"]}}"#,
        );
        let refusal = read_status(dir.path()).unwrap().refusal.unwrap();
        assert_eq!(refusal.reads, [Read::StatCells]);
        assert_eq!(refusal.packs, [PackProblem::MapsMissing]);
        // every read and pack problem of this version survives the trip
        let all = Refusal {
            reads: Read::ALL.to_vec(),
            packs: vec![
                PackProblem::HeroesMissing,
                PackProblem::MapsMissing,
                PackProblem::WordsMissing,
                PackProblem::WordsLackResultWords,
                PackProblem::WordsLackPhaseTitles,
                PackProblem::WordsLackEndTitles,
            ],
        };
        let text = serde_json::to_string(&all).unwrap();
        assert_eq!(serde_json::from_str::<Refusal>(&text).unwrap(), all);
        // not JSON at all: none
        write("not json");
        assert_eq!(read_status(dir.path()), None);
    }

    #[test]
    fn the_status_file_round_trips_and_a_broken_one_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_status(dir.path()), None, "no file yet");
        let refusal = decide(OcrSetting::Off, &READY, &[Read::OwnRow]).refusal;
        let mut written = status(OcrSetting::Off, refusal, Some(READY));
        written.blocked_total = 3;
        written.blocked = vec![StatusSite {
            site: "main.rs:1 recognize".into(),
            count: 3,
        }];
        write_status(dir.path(), &written).unwrap();
        assert_eq!(read_status(dir.path()), Some(written));
        let text = std::fs::read_to_string(status_path(dir.path())).unwrap();
        assert!(text.contains("\"requested\": \"off\""), "{text}");
        assert!(text.contains("\"effective\": \"on\""), "{text}");
        assert!(text.contains("\"own_row\""), "{text}");
        // no temp file is left behind
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, [std::ffi::OsString::from(STATUS_FILE)]);
        std::fs::write(status_path(dir.path()), b"{ not json").unwrap();
        assert_eq!(read_status(dir.path()), None);
    }

    /// Every `shadow::read_*` entry point has a line in [`READER_ENTRIES`],
    /// so what it needs is part of [`PackReadiness`]. A new read without a
    /// line fails here.
    #[test]
    fn every_reader_entry_point_has_its_pack_groups() {
        let source = include_str!("shadow/reader.rs");
        let production = source.split("\n#[cfg(test)]").next().unwrap();
        let mut found: Vec<&str> = production
            .lines()
            .filter_map(|line| line.strip_prefix("pub fn read_"))
            .filter_map(|rest| rest.split(['(', '<']).next())
            .collect();
        found.sort_unstable();
        let mut listed: Vec<&str> = READER_ENTRIES
            .iter()
            .map(|(name, _)| name.trim_start_matches("read_"))
            .collect();
        listed.sort_unstable();
        assert_eq!(
            found, listed,
            "shadow/reader.rs and ocr_switch::READER_ENTRIES name different reads"
        );
        assert!(
            found.len() >= 12,
            "the scan found the entry points: {found:?}"
        );
        // A reader with nothing loaded has none of the groups, and one with
        // everything has all of them: every group is a readiness field.
        for (name, groups) in READER_ENTRIES {
            for group in *groups {
                assert!(READY.has(*group), "{name}");
                assert!(!PackReadiness::default().has(*group), "{name}");
            }
        }
    }
}
