//! Pure logic for the expanded Games row (UX Spec games-expanded-v2): the
//! per-stat "vs avg" deltas, the tracked span, the hero split, read vs
//! corrected values and the upload and reader status. No iced here; the
//! panel module draws it.

use crate::model::{Game, Outcome, SegmentView};

/// Per-tile deltas start at this many games on the hero (the spec's one
/// 10-game threshold). Fewer gets one "Too few games" caption.
pub const AVG_MIN_GAMES: usize = 10;
/// The average is over at most this many of the newest other games.
pub const AVG_WINDOW: usize = 20;

/// The six stat tiles, in tile order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stat {
    E,
    D,
    A,
    Dmg,
    Heal,
    Mit,
}

impl Stat {
    pub const ALL: [Stat; 6] = [Stat::E, Stat::D, Stat::A, Stat::Dmg, Stat::Heal, Stat::Mit];

    pub fn label(self) -> &'static str {
        match self {
            Stat::E => "E",
            Stat::D => "D",
            Stat::A => "A",
            Stat::Dmg => "DMG",
            Stat::Heal => "HEAL",
            Stat::Mit => "MIT",
        }
    }

    /// The reader's flat name (suspect_fields).
    pub fn flat(self) -> &'static str {
        match self {
            Stat::E => "e",
            Stat::D => "d",
            Stat::A => "a",
            Stat::Dmg => "dmg",
            Stat::Heal => "h",
            Stat::Mit => "mit",
        }
    }

    /// The edit name (edited_fields).
    pub fn edit_name(self) -> &'static str {
        match self {
            Stat::E => "elims",
            Stat::D => "deaths",
            Stat::A => "assists",
            Stat::Dmg => "damage",
            Stat::Heal => "healing",
            Stat::Mit => "mitigation",
        }
    }

    /// The largest value the averages trust, from the site's limits
    /// (`scuffed_types`): a bigger count is a misread digit, so it stays
    /// out of the mean as it does on the website.
    pub fn plausible_max(self) -> u32 {
        match self {
            Stat::E => scuffed_types::PLAUSIBLE_ELIMS_MAX,
            Stat::D => scuffed_types::PLAUSIBLE_DEATHS_MAX,
            Stat::A => scuffed_types::PLAUSIBLE_ASSISTS_MAX,
            Stat::Dmg => scuffed_types::STATS_UPLOAD_MAX_DAMAGE,
            Stat::Heal => scuffed_types::STATS_UPLOAD_MAX_HEALING,
            Stat::Mit => scuffed_types::STATS_UPLOAD_MAX_MITIGATION,
        }
    }

    /// Damage, healing and mitigation read in thousands.
    pub fn big(self) -> bool {
        matches!(self, Stat::Dmg | Stat::Heal | Stat::Mit)
    }

    /// The effective (corrected when edited) value.
    pub fn value(self, g: &Game) -> u32 {
        match self {
            Stat::E => g.elims,
            Stat::D => g.deaths,
            Stat::A => g.assists,
            Stat::Dmg => g.damage,
            Stat::Heal => g.healing,
            Stat::Mit => g.mitigation,
        }
    }

    /// What the reader read.
    pub fn read(self, g: &Game) -> u32 {
        match self {
            Stat::E => g.ocr.elims,
            Stat::D => g.ocr.deaths,
            Stat::A => g.ocr.assists,
            Stat::Dmg => g.ocr.damage,
            Stat::Heal => g.ocr.healing,
            Stat::Mit => g.ocr.mitigation,
        }
    }
}

/// A tile value as the tiles print it: 24, 10.4k.
pub fn tile_value(stat: Stat, n: u32) -> String {
    if stat.big() && n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// `Some((read, set))` when the player corrected this stat to a new value.
pub fn read_vs_set(g: &Game, stat: Stat) -> Option<(u32, u32)> {
    let (read, set) = (stat.read(g), stat.value(g));
    (g.edited_fields.iter().any(|f| f == stat.edit_name()) && read != set).then_some((read, set))
}

/// "Read 22, you set 24".
pub fn read_vs_set_line(stat: Stat, read: u32, set: u32) -> String {
    format!(
        "Read {}, you set {}",
        tile_value(stat, read),
        tile_value(stat, set)
    )
}

/// The one caption while the hero is unknown: no deltas (UX Spec).
pub const AVG_HERO_UNSET: &str = "vs avg: shown once the hero is set.";

/// The player's averages on one hero.
#[derive(Debug, Clone, PartialEq)]
pub struct Averages {
    pub hero: String,
    /// Games on the hero up to this one, this one included.
    pub games: usize,
    /// Earlier games the mean is taken over (at most [`AVG_WINDOW`]).
    pub window: usize,
    /// Mean per stat in [`Stat::ALL`] order. `None` under [`AVG_MIN_GAMES`];
    /// a stat with no trusted value in the window has no mean.
    pub means: Option<[Option<f64>; 6]>,
}

/// Averages on `game`'s main hero over the newest earlier games on that
/// hero, so an old game is not compared with games played after it. Per
/// stat, a value still unsure or past [`Stat::plausible_max`] is left out.
pub fn averages(game: &Game, all: &[Game]) -> Averages {
    // Grouped as the hero rows are, so older "Lucio" games count for Lúcio.
    let hero = crate::aggregate::hero_name(&main_hero(game)).to_string();
    let start = game.started_at();
    let mut others: Vec<&Game> = all
        .iter()
        .filter(|g| g.session_id != game.session_id && g.started_at() < start)
        .filter(|g| crate::aggregate::hero_name(&g.display_hero()).eq_ignore_ascii_case(&hero))
        .collect();
    others.sort_by_key(|g| std::cmp::Reverse(g.started_at()));
    let games = others.len() + 1;
    others.truncate(AVG_WINDOW);
    let window = others.len();
    let means = (games >= AVG_MIN_GAMES && window > 0).then(|| {
        Stat::ALL.map(|stat| {
            let vals: Vec<f64> = others
                .iter()
                .filter(|g| !g.unsure_now(stat.flat()) && stat.value(g) <= stat.plausible_max())
                .map(|g| f64::from(stat.value(g)))
                .collect();
            (!vals.is_empty()).then(|| vals.iter().sum::<f64>() / vals.len() as f64)
        })
    });
    Averages {
        hero,
        games,
        window,
        means,
    }
}

impl Averages {
    /// The tooltip and caption under the tiles.
    pub fn caption(&self) -> String {
        if self.means.is_some() {
            format!(
                "vs avg: your average on {}, last {} games",
                self.hero, self.window
            )
        } else {
            let n = self.games;
            let games = if n == 1 { "game" } else { "games" };
            format!(
                "vs avg: Too few games. {n} {games} on {}, averages start at {AVG_MIN_GAMES}.",
                self.hero
            )
        }
    }
}

/// The line under a tile value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaLine {
    /// "+6 vs avg", "−0.2k vs avg", "±0 vs avg".
    Delta(String),
    /// The read is unsure, so a delta would mislead.
    CheckFirst,
    /// Too few games: the one caption says so, the tile shows nothing.
    None,
}

impl DeltaLine {
    pub fn text(&self) -> Option<String> {
        match self {
            DeltaLine::Delta(d) => Some(format!("{d} vs avg")),
            DeltaLine::CheckFirst => Some("Check first".into()),
            DeltaLine::None => None,
        }
    }
}

/// The delta line for one tile. It uses the corrected value.
pub fn delta_line(g: &Game, stat: Stat, avg: &Averages) -> DeltaLine {
    if g.unsure_now(stat.flat()) {
        return DeltaLine::CheckFirst;
    }
    let i = Stat::ALL.iter().position(|s| *s == stat).unwrap_or(0);
    match avg.means.and_then(|m| m[i]) {
        Some(mean) => DeltaLine::Delta(format_delta(stat, f64::from(stat.value(g)) - mean)),
        None => DeltaLine::None,
    }
}

/// Signed delta with the sign always shown. U+2212 for minus.
pub fn format_delta(stat: Stat, d: f64) -> String {
    let (n, unit) = if stat.big() {
        ((d / 100.0).round() / 10.0, "k")
    } else {
        (d.round(), "")
    };
    if n == 0.0 {
        return "±0".into();
    }
    let sign = if n > 0.0 { "+" } else { "\u{2212}" };
    let mag = n.abs();
    if stat.big() {
        format!("{sign}{mag:.1}{unit}")
    } else {
        format!("{sign}{mag:.0}")
    }
}

/// The Time block: Tracked, and the Length line from the same span. The
/// tracker records no game start or end, so the span (first to last Tab
/// press) is the only length it has, and the game ran at least that long.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tracked {
    /// First to last Tab read, in seconds.
    Span(i64),
    /// One scoreboard read: no span to time.
    OneRead,
    /// No Tab reads kept for this game (an older row).
    NotRecorded,
}

pub fn tracked(g: &Game) -> Tracked {
    let snaps: u32 = g.segments.iter().map(|s| s.snapshots).sum();
    let first = g.segments.iter().map(|s| s.first_seen).min();
    let last = g.segments.iter().map(|s| s.last_seen).max();
    match (first, last) {
        _ if snaps == 1 => Tracked::OneRead,
        (Some(a), Some(b)) if b > a => Tracked::Span((b - a).num_seconds()),
        (Some(_), Some(_)) => Tracked::OneRead,
        _ => Tracked::NotRecorded,
    }
}

/// "9:14", "72:05".
pub fn mm_ss(secs: i64) -> String {
    let s = secs.max(0);
    format!("{}:{:02}", s / 60, s % 60)
}

impl Tracked {
    pub fn line(&self) -> String {
        match self {
            Tracked::Span(s) => format!("Tracked {}", mm_ss(*s)),
            Tracked::OneRead => "Tracked: one scoreboard read".into(),
            Tracked::NotRecorded => "Tracked: not recorded".into(),
        }
    }

    /// The span in seconds, when there is one.
    pub fn span_secs(&self) -> Option<i64> {
        match self {
            Tracked::Span(s) => Some(*s),
            Tracked::OneRead | Tracked::NotRecorded => None,
        }
    }

    /// "Length: at least 9:14", or "Length: not recorded" with no span.
    pub fn length_line(&self) -> String {
        match self.span_secs() {
            Some(s) => format!("Length: at least {}", mm_ss(s)),
            None => "Length: not recorded".into(),
        }
    }

    /// The LENGTH cell: "9:14+" (at least), or `None` (drawn "-") with no span.
    pub fn length_cell(&self) -> Option<String> {
        self.span_secs().map(|s| format!("{}+", mm_ss(s)))
    }

    pub fn note(&self) -> Option<&'static str> {
        match self {
            Tracked::Span(_) => Some("From your first to last Tab press, shorter than the game"),
            Tracked::OneRead => Some("Only one Tab press, so there is no span to time"),
            Tracked::NotRecorded => None,
        }
    }
}

/// The queue title warning for the map line: the sentence when the game has
/// a queue title with no confident value (`QueueTitle::confident_value`,
/// the same gate as the upload), `None` when it has one or was never read
/// (reader off). A chip and the warning never show together.
pub fn queue_warning(g: &Game) -> Option<&'static str> {
    g.queue_title
        .as_ref()
        .filter(|t| t.confident_value().is_none())
        .map(|_| stat_tracker::model::QUEUE_TITLE_WARNING)
}

/// The confident queue ("OPEN QUEUE") for the map line's chip, if any:
/// `QueueTitle::confident_value`, so the chip shows exactly what the upload
/// sends.
pub fn queue_value(g: &Game) -> Option<String> {
    g.queue_title
        .as_ref()?
        .confident_value()
        .map(str::to_string)
}

/// One bar segment of the hero timeline, in time order.
#[derive(Debug, Clone, PartialEq)]
pub struct BarPart {
    pub hero: String,
    pub snapshots: u32,
    /// Drawn as the main hero (solid accent).
    pub main: bool,
    /// A one-snapshot swap still waiting for Confirm or Dismiss.
    pub unconfirmed: bool,
    /// Segment index for Confirm / Dismiss.
    pub index: u32,
}

/// One hero in the split line.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitPart {
    pub hero: String,
    pub percent: u32,
    pub unconfirmed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeroSplit {
    pub main: String,
    pub bar: Vec<BarPart>,
    pub split: Vec<SplitPart>,
}

fn is_unconfirmed(s: &SegmentView) -> bool {
    !s.confirmed && !s.dismissed
}

/// Whether the player set the hero by hand (Edit or the picker).
fn hero_set_by_hand(g: &Game) -> bool {
    g.edited_fields.iter().any(|f| f == "hero")
}

/// The main hero: the one set by hand, as the row shows it, else
/// [`read_main_hero`]. Edit stores a hero fix as `corrected_hero` only and
/// the runs keep what the reader read, so the hand fix wins here.
pub fn main_hero(g: &Game) -> String {
    if hero_set_by_hand(g) {
        g.display_hero()
    } else {
        read_main_hero(g)
    }
}

/// The hero with the most snapshots (a dismissed run folds into it). Ties
/// and a game with no runs keep the row's hero.
fn read_main_hero(g: &Game) -> String {
    let row = g.display_hero();
    let mut best: Option<(String, u32)> = None;
    for seg in g.segments.iter().filter(|s| !s.dismissed) {
        let n: u32 = g
            .segments
            .iter()
            .filter(|s| !s.dismissed && s.hero == seg.hero)
            .map(|s| s.snapshots)
            .sum();
        let better = match &best {
            None => true,
            Some((h, b)) => n > *b || (n == *b && seg.hero == row && *h != row),
        };
        if better {
            best = Some((seg.hero.clone(), n));
        }
    }
    best.map_or(row, |(h, _)| h)
}

/// The timeline bar and the "Hero NN% · Hero NN%" split. Each run is sized
/// by its snapshots. A dismissed run folds into the main hero, and with the
/// hero set by hand so do the runs of the hero the reader had as main.
pub fn hero_split(g: &Game) -> HeroSplit {
    let main = main_hero(g);
    let read_main = hero_set_by_hand(g).then(|| read_main_hero(g));
    let mut bar: Vec<BarPart> = Vec::new();
    for s in &g.segments {
        let hero = if s.dismissed || read_main.as_ref() == Some(&s.hero) {
            main.clone()
        } else {
            s.hero.clone()
        };
        let unconfirmed = is_unconfirmed(s) && hero != main;
        if let Some(last) = bar.last_mut()
            && last.hero == hero
            && !last.unconfirmed
            && !unconfirmed
        {
            last.snapshots += s.snapshots;
            continue;
        }
        bar.push(BarPart {
            main: hero == main,
            hero,
            snapshots: s.snapshots,
            unconfirmed,
            index: s.index,
        });
    }
    if bar.is_empty() {
        bar.push(BarPart {
            hero: main.clone(),
            snapshots: 1,
            main: true,
            unconfirmed: false,
            index: 0,
        });
    }
    let total: u32 = bar.iter().map(|b| b.snapshots).sum::<u32>().max(1);
    let mut split: Vec<SplitPart> = Vec::new();
    for b in &bar {
        match split.iter_mut().find(|p| p.hero == b.hero) {
            Some(p) => {
                p.percent += b.snapshots;
                p.unconfirmed &= b.unconfirmed;
            }
            None => split.push(SplitPart {
                hero: b.hero.clone(),
                percent: b.snapshots,
                unconfirmed: b.unconfirmed,
            }),
        }
    }
    split.sort_by_key(|p| std::cmp::Reverse(p.percent));
    for p in &mut split {
        p.percent = ((p.percent as f64 * 100.0) / total as f64).round() as u32;
    }
    HeroSplit { main, bar, split }
}

impl HeroSplit {
    /// "Wrecking Ball 92% · Sigma 8% unconfirmed".
    pub fn line(&self) -> String {
        self.split
            .iter()
            .map(|p| {
                let mut s = format!("{} {}%", p.hero, p.percent);
                if p.unconfirmed {
                    s.push_str(" unconfirmed");
                }
                s
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// The swaps waiting for Confirm or Dismiss, as (segment index, prompt).
    pub fn swaps(&self) -> Vec<(u32, String)> {
        self.bar
            .iter()
            .filter(|b| b.unconfirmed)
            .map(|b| {
                let snaps = if b.snapshots == 1 {
                    "1 snapshot".to_string()
                } else {
                    format!("{} snapshots", b.snapshots)
                };
                (b.index, format!("{} for {snaps} only. Swap?", b.hero))
            })
            .collect()
    }
}

/// "Read Ilios, you set Lijiang Tower" for a map or hero set by hand
/// (`field` is the store's edited name: "map_name" or "hero").
pub fn read_vs_set_name(g: &Game, field: &str) -> Option<String> {
    let (read, set, none) = match field {
        "map_name" => (&g.ocr.map_name, &g.map_name, "no map"),
        "hero" => (&g.ocr.hero, &g.hero, "no hero"),
        _ => return None,
    };
    if !g.edited_fields.iter().any(|f| f == field) || read.eq_ignore_ascii_case(set) {
        return None;
    }
    let read = read.trim();
    let read = if read.is_empty() || read.eq_ignore_ascii_case("unknown") {
        none
    } else {
        read
    };
    Some(format!("Read {read}, you set {set}"))
}

/// Dismissed runs, as (segment index, line), each with an Undo that takes
/// the Dismiss back.
pub fn dismissed_runs(g: &Game) -> Vec<(u32, String)> {
    g.segments
        .iter()
        .filter(|s| s.dismissed)
        .map(|s| (s.index, format!("{} run dismissed", s.hero)))
        .collect()
}

/// Upload and reader status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upload {
    Uploaded,
    Waiting,
    Rejected(String),
    /// A map or hero is missing, so the uploader keeps the game back
    /// (`sync::row_needs_review`). The names are what is missing.
    Held(&'static str),
    /// Still being played: it sends once the game ends.
    InProgress,
    /// No website linked: games stay on this computer.
    SyncOff,
    /// Ended with no result. The uploader skips it (and may mark the row
    /// synced without sending it) until a result is picked.
    Undecided,
}

/// The game's upload state. `in_progress` is the open session, and
/// `sync_on` whether a website is linked ([`crate::app::TrackerApp::sync_on`]).
pub fn upload(g: &Game, in_progress: bool, sync_on: bool) -> Upload {
    if let Some(msg) = g.upload_reject.as_deref() {
        return Upload::Rejected(msg.to_string());
    }
    let missing = g.review_fields();
    let held = match (missing.contains(&"map"), missing.contains(&"hero")) {
        (true, true) => Some("map and hero"),
        (true, false) => Some("map"),
        (false, true) => Some("hero"),
        (false, false) => None,
    };
    let no_result = g.outcome == Outcome::Unknown && !in_progress;
    // With no website linked nothing is sent: a live game does not send
    // once it ends, and a game with no result is not uploaded.
    if !sync_on && (in_progress || no_result) {
        return held.map_or(Upload::SyncOff, Upload::Held);
    }
    // An ended game with no result was never sent, whatever `synced` says:
    // the daemon marks such a row synced without uploading it.
    if no_result {
        return held.map_or(Upload::Undecided, Upload::Held);
    }
    if g.synced {
        return Upload::Uploaded;
    }
    if let Some(what) = held {
        return Upload::Held(what);
    }
    if in_progress {
        Upload::InProgress
    } else if !sync_on {
        Upload::SyncOff
    } else {
        Upload::Waiting
    }
}

/// The notice under an uploaded game a map or hero is missing from (an
/// older build sent it blank), as the classic card says it: "Map is
/// missing. Pick one to fix it on the site." Its Set control is
/// [`crate::games_picker::offers_set`]'s. `None` for any other game.
pub fn site_fix_notice(g: &Game, up: &Upload) -> Option<&'static str> {
    if *up != Upload::Uploaded || g.session_id.is_empty() {
        return None;
    }
    g.review_notice()
}

impl Upload {
    pub fn title(&self) -> &'static str {
        match self {
            Upload::Uploaded => "Uploaded",
            Upload::Waiting => "Waiting to upload",
            Upload::Rejected(_) => "Upload rejected:",
            Upload::Held(_) => "Needs a look",
            Upload::InProgress => "Game in progress",
            Upload::SyncOff => "Not uploading",
            Upload::Undecided => "Not uploaded",
        }
    }

    /// The line beside or under the title.
    pub fn detail(&self, recognizer: &str) -> String {
        match self {
            Upload::Uploaded => format!("Read by {recognizer}"),
            Upload::Waiting => format!("Read by {recognizer} · sends when the server answers"),
            Upload::Rejected(msg) => format!("Server: {msg}"),
            Upload::Held(what) => {
                let (verb, it) = match *what {
                    "map" => ("is", "the map"),
                    "hero" => ("is", "the hero"),
                    _ => ("are", "them"),
                };
                format!(
                    "The {what} {verb} missing. This game stays on this computer until you set {it}."
                )
            }
            Upload::InProgress => format!("Read by {recognizer} · sends after the game ends"),
            Upload::SyncOff => {
                "Only on this computer · sign in to the website in Settings to upload".into()
            }
            Upload::Undecided => "No result yet · it uploads once you pick one".into(),
        }
    }

    pub fn rejected(&self) -> bool {
        matches!(self, Upload::Rejected(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GameOcr, Outcome, Role};
    use chrono::{DateTime, TimeZone, Utc};

    fn at(h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 2, h, m, s).unwrap()
    }

    fn seg(
        i: u32,
        hero: &str,
        snaps: u32,
        confirmed: bool,
        a: DateTime<Utc>,
        b: DateTime<Utc>,
    ) -> SegmentView {
        SegmentView {
            index: i,
            hero: hero.into(),
            role: Role::Tank,
            snapshots: snaps,
            confirmed,
            dismissed: false,
            resolution: None,
            first_seen: a,
            last_seen: b,
        }
    }

    fn game(id: &str, hero: &str, e: u32, dmg: u32, when: DateTime<Utc>) -> Game {
        Game {
            session_id: id.into(),
            hero: hero.into(),
            map_name: "Ilios".into(),
            role: Role::Tank,
            outcome: Outcome::Win,
            elims: e,
            deaths: 6,
            assists: 9,
            damage: dmg,
            healing: 1000,
            mitigation: 5000,
            played_at: when,
            edited: false,
            edited_fields: Vec::new(),
            ocr: GameOcr {
                hero: hero.into(),
                role: "Tank".into(),
                map_name: "Ilios".into(),
                outcome: "victory".into(),
                elims: e,
                deaths: 6,
                assists: 9,
                damage: dmg,
                healing: 1000,
                mitigation: 5000,
            },
            segments: Vec::new(),
            upload_reject: None,
            suspect_fields: Vec::new(),
            synced: false,
            recognizer: "cv-v1".into(),
            bans_picks: None,
            bans_picks_status: None,
            queue_title: None,
        }
    }

    fn history(n: usize) -> Vec<Game> {
        (0..n)
            .map(|i| {
                game(
                    &format!("h{i}"),
                    "Wrecking Ball",
                    18,
                    9200,
                    at(10, i as u32, 0),
                )
            })
            .collect()
    }

    #[test]
    fn deltas_need_ten_games_on_the_hero() {
        let g = game("g", "Wrecking Ball", 24, 10_400, at(20, 0, 0));
        let mut all = history(8);
        all.push(g.clone());
        let few = averages(&g, &all);
        assert_eq!(few.games, 9);
        assert!(few.means.is_none());
        assert_eq!(delta_line(&g, Stat::E, &few), DeltaLine::None);
        assert_eq!(
            few.caption(),
            "vs avg: Too few games. 9 games on Wrecking Ball, averages start at 10."
        );
        all.push(game("h9", "Wrecking Ball", 18, 9200, at(11, 0, 0)));
        let ok = averages(&g, &all);
        assert_eq!((ok.games, ok.window), (10, 9));
        assert!(
            averages(&g, &all)
                .means
                .unwrap()
                .iter()
                .all(Option::is_some)
        );
        assert_eq!(delta_line(&g, Stat::E, &ok).text().unwrap(), "+6 vs avg");
        assert_eq!(
            delta_line(&g, Stat::Dmg, &ok).text().unwrap(),
            "+1.2k vs avg"
        );
        assert_eq!(delta_line(&g, Stat::D, &ok).text().unwrap(), "±0 vs avg");
        assert_eq!(
            ok.caption(),
            "vs avg: your average on Wrecking Ball, last 9 games"
        );
    }

    #[test]
    fn average_window_is_the_newest_twenty_on_the_same_hero() {
        let g = game("g", "Wrecking Ball", 20, 9000, at(23, 0, 0));
        let mut all = history(30);
        all.push(game("x", "Sigma", 99, 99_000, at(22, 0, 0)));
        all[0].elims = 100; // the oldest, outside the window
        let avg = averages(&g, &all);
        assert_eq!((avg.games, avg.window), (31, 20));
        assert_eq!(avg.means.unwrap()[0], Some(18.0));
    }

    /// Older "Lucio" and "Torbjorn" games count toward Lúcio and Torbjörn,
    /// as in the hero rows, and the reverse.
    #[test]
    fn averages_group_older_hero_spellings() {
        for (old, now) in [("Lucio", "Lúcio"), ("Torbjorn", "Torbjörn")] {
            let all: Vec<Game> = (0..12)
                .map(|i| {
                    let hero = if i % 2 == 0 { old } else { now };
                    game(&format!("h{i}"), hero, 18, 9200, at(10, i as u32, 0))
                })
                .collect();
            for hero in [old, now] {
                let g = game("g", hero, 20, 9000, at(23, 0, 0));
                let avg = averages(&g, &all);
                assert_eq!((avg.games, avg.window), (13, 12), "{hero}");
                assert_eq!(avg.hero, now);
                assert_eq!(
                    avg.caption(),
                    format!("vs avg: your average on {now}, last 12 games")
                );
            }
        }
    }

    /// The mean leaves out newer games, other games' unsure values and
    /// values past the site's limits.
    #[test]
    fn averages_skip_newer_unsure_and_implausible_values() {
        let g = game("g", "Wrecking Ball", 20, 9000, at(23, 0, 0));
        let mut all = history(12);
        all[11].elims = 500;
        all[10].suspect_fields = vec!["e".into()];
        all[10].elims = 90;
        all.push(game("new", "Wrecking Ball", 80, 9000, at(23, 30, 0)));
        let avg = averages(&g, &all);
        assert_eq!((avg.games, avg.window), (13, 12));
        assert_eq!(avg.means.unwrap()[0], Some(18.0));
        // Only implausible values: no mean, no delta line.
        let mut all = history(12);
        for h in &mut all {
            h.deaths = 60;
        }
        let avg = averages(&g, &all);
        assert_eq!(avg.means.unwrap()[1], None);
        assert_eq!(delta_line(&g, Stat::D, &avg), DeltaLine::None);
    }

    #[test]
    fn minus_uses_the_minus_sign_and_k_for_big_stats() {
        assert_eq!(format_delta(Stat::D, -1.4), "\u{2212}1");
        assert_eq!(format_delta(Stat::Heal, -240.0), "\u{2212}0.2k");
        assert_eq!(format_delta(Stat::Mit, 30.0), "±0");
    }

    #[test]
    fn an_unsure_tile_says_check_first_and_an_edit_clears_it() {
        let mut g = game("g", "Wrecking Ball", 24, 8900, at(20, 0, 0));
        g.suspect_fields = vec!["dmg".into(), "h".into()];
        let avg = averages(&g, &history(12));
        assert_eq!(delta_line(&g, Stat::Dmg, &avg), DeltaLine::CheckFirst);
        assert_eq!(
            delta_line(&g, Stat::Heal, &avg).text().unwrap(),
            "Check first"
        );
        g.edited_fields = vec!["damage".into()];
        assert!(matches!(
            delta_line(&g, Stat::Dmg, &avg),
            DeltaLine::Delta(_)
        ));
    }

    #[test]
    fn tracked_span_is_first_to_last_seen_in_mm_ss() {
        let mut g = game("g", "Wrecking Ball", 24, 1, at(20, 0, 0));
        assert_eq!(tracked(&g), Tracked::NotRecorded);
        g.segments = vec![
            seg(0, "Wrecking Ball", 6, true, at(20, 0, 10), at(20, 5, 0)),
            seg(1, "Sigma", 1, false, at(20, 5, 30), at(20, 5, 30)),
            seg(2, "Wrecking Ball", 5, true, at(20, 6, 0), at(20, 11, 15)),
        ];
        assert_eq!(tracked(&g), Tracked::Span(665));
        assert_eq!(tracked(&g).line(), "Tracked 11:05");
        g.segments = vec![seg(0, "Mei", 1, true, at(20, 0, 0), at(20, 0, 0))];
        assert_eq!(tracked(&g), Tracked::OneRead);
        assert_eq!(tracked(&g).line(), "Tracked: one scoreboard read");
    }

    /// Item 19: the Length line and the LENGTH cell show the one length the
    /// tracker has, the Tab span, as a floor ("at least"), and say "not
    /// recorded" (the cell "-") when there is no span.
    #[test]
    fn length_is_the_tab_span_as_a_floor_or_not_recorded() {
        let mut g = game("g", "Mei", 24, 1, at(20, 0, 0));
        assert_eq!(tracked(&g).length_line(), "Length: not recorded");
        assert_eq!(tracked(&g).length_cell(), None);
        g.segments = vec![seg(0, "Mei", 1, true, at(20, 0, 0), at(20, 0, 0))];
        assert_eq!(tracked(&g).length_line(), "Length: not recorded");
        assert_eq!(tracked(&g).length_cell(), None);
        g.segments = vec![seg(0, "Mei", 4, true, at(20, 0, 10), at(20, 9, 24))];
        assert_eq!(tracked(&g).length_line(), "Length: at least 9:14");
        assert_eq!(tracked(&g).length_cell().as_deref(), Some("9:14+"));
        assert_eq!(tracked(&g).span_secs(), Some(554));
    }

    #[test]
    fn split_marks_a_one_snapshot_swap_unconfirmed() {
        let mut g = game("g", "Wrecking Ball", 24, 1, at(20, 0, 0));
        g.segments = vec![
            seg(0, "Wrecking Ball", 6, true, at(20, 0, 0), at(20, 5, 0)),
            seg(1, "Sigma", 1, false, at(20, 5, 30), at(20, 5, 30)),
            seg(2, "Wrecking Ball", 5, true, at(20, 6, 0), at(20, 11, 0)),
        ];
        let s = hero_split(&g);
        assert_eq!(s.main, "Wrecking Ball");
        assert_eq!(s.bar.len(), 3);
        assert!(s.bar[1].unconfirmed && !s.bar[1].main);
        assert_eq!(s.line(), "Wrecking Ball 92% · Sigma 8% unconfirmed");
        assert_eq!(
            s.swaps(),
            vec![(1, "Sigma for 1 snapshot only. Swap?".into())]
        );

        g.segments[1].dismissed = true;
        let s = hero_split(&g);
        assert_eq!(s.line(), "Wrecking Ball 100%");
        assert_eq!(s.bar.len(), 1, "a dismissed run folds into the main hero");
        assert!(s.swaps().is_empty());

        g.segments[1].dismissed = false;
        g.segments[1].confirmed = true;
        let s = hero_split(&g);
        assert_eq!(s.line(), "Wrecking Ball 92% · Sigma 8%");
        assert!(s.swaps().is_empty());
    }

    /// A hero set by hand wins over the timeline: a game read as Ana for 8
    /// Tab presses and set to Reinhardt heads, fills the bar and compares
    /// as Reinhardt, like its row.
    #[test]
    fn a_hand_set_hero_wins_over_the_timeline() {
        let mut g = game("g", "Ana", 24, 9000, at(23, 0, 0));
        g.segments = vec![seg(0, "Ana", 8, true, at(22, 50, 0), at(23, 0, 0))];
        g.hero = "Reinhardt".into();
        g.edited_fields = vec!["hero".into()];
        assert_eq!(g.ocr.hero, "Ana");
        assert_eq!(main_hero(&g), "Reinhardt");
        let s = hero_split(&g);
        assert_eq!(s.main, "Reinhardt");
        assert_eq!(s.line(), "Reinhardt 100%");
        assert_eq!(s.bar.len(), 1);
        assert!(s.bar[0].main && s.bar[0].hero == "Reinhardt");
        assert!(s.swaps().is_empty());
        // vs avg: Reinhardt's games, not Ana's.
        let mut all: Vec<Game> = (0..12)
            .map(|i| game(&format!("r{i}"), "Reinhardt", 10, 9000, at(10, i as u32, 0)))
            .collect();
        all.extend((0..12).map(|i| game(&format!("a{i}"), "Ana", 30, 9000, at(11, i as u32, 0))));
        let avg = averages(&g, &all);
        assert_eq!(avg.hero, "Reinhardt");
        assert_eq!((avg.games, avg.window), (13, 12));
        assert_eq!(avg.means.unwrap()[0], Some(10.0));
        // Without the hand edit the timeline still names the main hero.
        g.edited_fields.clear();
        assert_eq!(main_hero(&g), "Ana");
    }

    /// With the hero set by hand, the read main hero's runs fold into it,
    /// and another hero's run stays its own.
    #[test]
    fn a_hand_set_hero_takes_the_read_main_runs_only() {
        let mut g = game("g", "Ana", 24, 9000, at(23, 0, 0));
        g.segments = vec![
            seg(0, "Ana", 6, true, at(22, 40, 0), at(22, 50, 0)),
            seg(1, "Mercy", 2, true, at(22, 51, 0), at(22, 55, 0)),
            seg(2, "Ana", 2, true, at(22, 56, 0), at(23, 0, 0)),
        ];
        g.hero = "Reinhardt".into();
        g.edited_fields = vec!["hero".into()];
        assert_eq!(hero_split(&g).line(), "Reinhardt 80% · Mercy 20%");
    }

    #[test]
    fn no_runs_reads_the_row_hero_at_100() {
        let g = game("g", "Mei", 12, 1, at(20, 0, 0));
        assert_eq!(hero_split(&g).line(), "Mei 100%");
    }

    #[test]
    fn read_vs_set_only_for_an_edited_change() {
        let mut g = game("g", "Wrecking Ball", 24, 1, at(20, 0, 0));
        g.ocr.elims = 22;
        assert_eq!(read_vs_set(&g, Stat::E), None, "not in edited_fields");
        g.edited_fields = vec!["elims".into()];
        assert_eq!(read_vs_set(&g, Stat::E), Some((22, 24)));
        assert_eq!(read_vs_set_line(Stat::E, 22, 24), "Read 22, you set 24");
        assert_eq!(read_vs_set(&g, Stat::D), None);
    }

    /// An ended game with no result reads as not uploaded even when the
    /// daemon marked its row synced without sending it, with sync on or
    /// off; a live one, or a held one, keeps its own state.
    #[test]
    fn an_undecided_game_never_reads_uploaded() {
        let mut g = game("g", "Wrecking Ball", 24, 1, at(20, 0, 0));
        g.outcome = Outcome::Unknown;
        g.synced = true;
        assert_eq!(upload(&g, false, true), Upload::Undecided);
        assert_eq!(upload(&g, false, true).title(), "Not uploaded");
        assert_eq!(
            upload(&g, false, true).detail(""),
            "No result yet · it uploads once you pick one"
        );
        assert_eq!(
            upload(&g, false, false),
            Upload::SyncOff,
            "sync off: not uploaded either"
        );
        g.map_name.clear();
        assert_eq!(upload(&g, false, false), Upload::Held("map"));
        g.map_name = "Ilios".into();
        g.synced = false;
        assert_eq!(upload(&g, true, true), Upload::InProgress);
        g.map_name.clear();
        assert_eq!(upload(&g, false, true), Upload::Held("map"));
        g.map_name = "Ilios".into();
        g.outcome = Outcome::Win;
        g.synced = true;
        assert_eq!(upload(&g, false, true), Upload::Uploaded);
    }

    /// With no website linked a live game never says it sends after the
    /// game ends, and a held live game's picker never says it uploads once
    /// it ends.
    #[test]
    fn a_live_game_with_sync_off_is_not_uploading() {
        let mut g = game("g", "Wrecking Ball", 24, 1, at(20, 0, 0));
        g.outcome = Outcome::Unknown;
        assert_eq!(upload(&g, true, false), Upload::SyncOff);
        assert_eq!(upload(&g, true, false).title(), "Not uploading");
        g.synced = true;
        assert_eq!(upload(&g, true, false), Upload::SyncOff);
        g.synced = false;
        g.outcome = Outcome::Win;
        assert_eq!(upload(&g, true, false), Upload::SyncOff);
        assert_eq!(upload(&g, true, true), Upload::InProgress);
        g.hero = "Unknown".into();
        assert_eq!(upload(&g, true, false), Upload::Held("hero"));
        assert_eq!(
            crate::games_picker::release_line(
                &g,
                crate::games_picker::Field::Hero,
                Some("Mei"),
                true,
                false
            ),
            "Saving releases this game. It stays on this computer: no website is linked."
        );
    }

    #[test]
    fn upload_status_lines() {
        let mut g = game("g", "Wrecking Ball", 24, 1, at(20, 0, 0));
        assert_eq!(upload(&g, false, true), Upload::Waiting);
        assert_eq!(
            upload(&g, false, true).detail(&g.recognizer),
            "Read by cv-v1 · sends when the server answers"
        );
        assert_eq!(upload(&g, true, true), Upload::InProgress);
        assert_eq!(
            upload(&g, true, true).detail(&g.recognizer),
            "Read by cv-v1 · sends after the game ends"
        );
        assert_eq!(upload(&g, false, false), Upload::SyncOff);
        assert!(
            upload(&g, false, false)
                .detail("")
                .contains("sign in to the website in Settings")
        );
        g.synced = true;
        assert_eq!(upload(&g, false, true).title(), "Uploaded");
        assert_eq!(
            upload(&g, false, true).detail(&g.recognizer),
            "Read by cv-v1"
        );
        g.synced = false;
        g.upload_reject = Some("reader version cv-v0 is no longer accepted.".into());
        let u = upload(&g, false, true);
        assert!(u.rejected());
        assert_eq!(u.title(), "Upload rejected:");
        assert_eq!(
            u.detail(&g.recognizer),
            "Server: reader version cv-v0 is no longer accepted."
        );
    }

    /// An uploaded game with a blank map or hero (an older build sent it)
    /// says what is missing and that a pick fixes it on the site; other
    /// games get no such notice.
    #[test]
    fn an_uploaded_game_missing_a_map_says_to_fix_it_on_the_site() {
        let mut g = game("g", "Wrecking Ball", 24, 1, at(20, 0, 0));
        g.synced = true;
        let up = upload(&g, false, true);
        assert_eq!(site_fix_notice(&g, &up), None);
        g.map_name.clear();
        let up = upload(&g, false, true);
        assert_eq!(up, Upload::Uploaded);
        assert_eq!(
            site_fix_notice(&g, &up),
            Some("Map is missing. Pick one to fix it on the site.")
        );
        g.synced = false;
        let up = upload(&g, false, true);
        assert_eq!(up, Upload::Held("map"));
        assert_eq!(site_fix_notice(&g, &up), None, "the Held box says it");
        g.synced = true;
        g.session_id.clear();
        let up = upload(&g, false, true);
        assert_eq!(up, Upload::Uploaded);
        assert_eq!(site_fix_notice(&g, &up), None, "imported: no session id");
    }

    /// A game the uploader keeps back says so instead of "Waiting".
    #[test]
    fn a_held_game_needs_a_look_not_waiting() {
        let mut g = game("g", "Wrecking Ball", 24, 1, at(20, 0, 0));
        g.map_name.clear();
        let u = upload(&g, false, true);
        assert_eq!(u, Upload::Held("map"));
        assert_eq!(u.title(), "Needs a look");
        assert_eq!(
            u.detail(""),
            "The map is missing. This game stays on this computer until you set the map."
        );
        g.hero = "Unknown".into();
        assert_eq!(upload(&g, false, false), Upload::Held("map and hero"));
        assert_eq!(
            upload(&g, false, true).detail(""),
            "The map and hero are missing. This game stays on this computer until you set them."
        );
        g.map_name = "Ilios".into();
        assert_eq!(upload(&g, true, true), Upload::Held("hero"));
        assert_eq!(
            upload(&g, false, true).detail(""),
            "The hero is missing. This game stays on this computer until you set the hero."
        );
        g.hero = "Mei".into();
        assert_eq!(upload(&g, false, true), Upload::Waiting);
    }

    /// A hand-set map or hero shows what was read; an untouched one does not.
    #[test]
    fn hand_set_map_and_hero_say_what_was_read() {
        let mut g = game("g", "Wrecking Ball", 24, 1, at(20, 0, 0));
        g.ocr.map_name = String::new();
        g.ocr.hero = "Unknown".into();
        assert_eq!(read_vs_set_name(&g, "map_name"), None);
        g.edited_fields = vec!["map_name".into(), "hero".into()];
        assert_eq!(
            read_vs_set_name(&g, "map_name").as_deref(),
            Some("Read no map, you set Ilios")
        );
        assert_eq!(
            read_vs_set_name(&g, "hero").as_deref(),
            Some("Read no hero, you set Wrecking Ball")
        );
        g.ocr.map_name = "Lijiang Tower".into();
        assert_eq!(
            read_vs_set_name(&g, "map_name").as_deref(),
            Some("Read Lijiang Tower, you set Ilios")
        );
        g.ocr.map_name = "ilios".into();
        assert_eq!(read_vs_set_name(&g, "map_name"), None);
        assert_eq!(read_vs_set_name(&g, "e"), None);
    }

    #[test]
    fn dismissed_runs_offer_an_undo() {
        let mut g = game("g", "Wrecking Ball", 24, 1, at(20, 0, 0));
        g.segments = vec![
            seg(0, "Wrecking Ball", 20, true, at(20, 0, 0), at(20, 9, 0)),
            seg(1, "Mei", 1, false, at(20, 9, 0), at(20, 9, 30)),
        ];
        assert!(dismissed_runs(&g).is_empty());
        g.segments[1].dismissed = true;
        assert_eq!(
            dismissed_runs(&g),
            vec![(1, "Mei run dismissed".to_string())]
        );
    }

    /// Site sync is on only with both a URL and a token.
    #[test]
    fn sync_counts_as_on_only_with_a_url_and_a_token() {
        use crate::app::sync_on_for;
        use stat_tracker::config::{Config, SyncConfig};
        let with = |url: &str, token: &str| Config {
            sync: Some(SyncConfig {
                server_url: url.into(),
                token: token.into(),
            }),
            ..Config::default()
        };
        assert!(!sync_on_for(&Config::default()));
        assert!(!sync_on_for(&with("", "tok")));
        assert!(!sync_on_for(&with("https://crew.example", "  ")));
        assert!(sync_on_for(&with("https://crew.example", "tok")));
    }

    /// The queue title warning shows only for a game with no confident
    /// value; the chip only for one with it. The reader off (no title)
    /// shows neither.
    #[test]
    fn the_queue_warning_shows_only_without_a_confident_value() {
        use stat_tracker::model::{QUEUE_TITLE_WARNING, QueueTitle};
        let mut g = game("a", "Ana", 1, 1, at(20, 0, 0));
        assert_eq!((queue_warning(&g), queue_value(&g)), (None, None));
        let confident = QueueTitle {
            value: Some("OPEN QUEUE".into()),
            confidence: 0.93,
            ..QueueTitle::default()
        };
        g.queue_title = Some(confident.clone());
        assert_eq!(queue_warning(&g), None);
        assert_eq!(queue_value(&g).as_deref(), Some("OPEN QUEUE"));
        // Weak with no earlier value: the warning, no chip.
        g.queue_title = Some(QueueTitle {
            unsure: true,
            ..QueueTitle::default()
        });
        assert_eq!(queue_warning(&g), Some(QUEUE_TITLE_WARNING));
        assert_eq!(queue_value(&g), None);
        // A value and the flag together (a row stored before the title was
        // write-once): the chip only, never the warning beside it.
        g.queue_title = Some(QueueTitle {
            unsure: true,
            ..confident
        });
        assert_eq!(queue_warning(&g), None);
        assert_eq!(queue_value(&g).as_deref(), Some("OPEN QUEUE"));
    }

    /// The chip uses the upload's gate (`QueueTitle::confident_value`): a
    /// stored value under the bar, at NaN, past 1 or blank shows the
    /// warning and no chip; exactly at the bar shows the chip.
    #[test]
    fn a_stored_value_below_the_bar_shows_the_warning_not_a_chip() {
        use stat_tracker::model::{QUEUE_TITLE_MIN_SCORE, QUEUE_TITLE_WARNING, QueueTitle};
        let mut g = game("a", "Ana", 1, 1, at(20, 0, 0));
        let stored = |value: &str, confidence: f32| QueueTitle {
            value: Some(value.into()),
            confidence,
            ..QueueTitle::default()
        };
        for (value, confidence) in [
            ("OPEN QUEUE", 0.79),
            ("OPEN QUEUE", 0.5),
            ("OPEN QUEUE", 0.0),
            ("OPEN QUEUE", f32::NAN),
            ("OPEN QUEUE", 1.5),
            ("  ", 0.93),
        ] {
            g.queue_title = Some(stored(value, confidence));
            assert_eq!(queue_value(&g), None, "{value:?} at {confidence}");
            assert_eq!(
                queue_warning(&g),
                Some(QUEUE_TITLE_WARNING),
                "{value:?} at {confidence}"
            );
        }
        g.queue_title = Some(stored(" OPEN QUEUE ", QUEUE_TITLE_MIN_SCORE));
        assert_eq!(queue_value(&g).as_deref(), Some("OPEN QUEUE"));
        assert_eq!(queue_warning(&g), None);
    }
}
