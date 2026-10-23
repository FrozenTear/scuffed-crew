//! Shared data of the stat tracker: the types and constants that more than
//! one area reads or writes. A match result, an OCR row read, a stored game
//! and its hero timeline, and the digit reader's id.
//!
//! Each item used to live next to its first user (`detect`, `ocr`,
//! `storage`, `shadow::digits`), and the modules that only needed the type
//! were tied into one dependency cycle through it. They live here now, below
//! every module that uses them, and stay reachable at their old paths
//! (`detect::MatchOutcome`, `storage::PersonalMatch`, ...) through `pub use`.
//! Nothing here reads a frame, opens the store or talks to the network.

use serde::{Deserialize, Serialize};
use surrealdb_types::Datetime as SurrealDatetime;
use surrealdb_types::SurrealValue;

// ------------------------------------------------------------- match result

/// A match result. The canonical wire/storage spelling is the lowercase
/// `Display` form ("victory"/"defeat"/"draw"/"unknown") — every layer that
/// needs a string goes through `to_string()`, and parsing goes through
/// `FromStr` (strict: anything else is an error, callers decide whether that
/// means `Unknown`). Do not hand-roll translations of these names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchOutcome {
    Victory,
    Defeat,
    Draw,
    Unknown,
}

impl std::fmt::Display for MatchOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MatchOutcome::Victory => "victory",
            MatchOutcome::Defeat => "defeat",
            MatchOutcome::Draw => "draw",
            MatchOutcome::Unknown => "unknown",
        })
    }
}

impl std::str::FromStr for MatchOutcome {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "victory" => Ok(MatchOutcome::Victory),
            "defeat" => Ok(MatchOutcome::Defeat),
            "draw" => Ok(MatchOutcome::Draw),
            "unknown" => Ok(MatchOutcome::Unknown),
            other => Err(format!("not a match outcome: {other:?}")),
        }
    }
}

impl MatchOutcome {
    /// Parse storage/GUI outcome strings, including legacy `win`/`loss` spellings
    /// from older local data. Unknown / empty / garbage → [`MatchOutcome::Unknown`].
    pub fn parse_lenient(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "victory" | "win" => MatchOutcome::Victory,
            "defeat" | "loss" => MatchOutcome::Defeat,
            "draw" => MatchOutcome::Draw,
            // FromStr's canonical spellings are all matched above, so any
            // other string (incl. "unknown"/"") can only be Unknown.
            _ => MatchOutcome::Unknown,
        }
    }

    pub fn is_win(self) -> bool {
        matches!(self, MatchOutcome::Victory)
    }

    pub fn is_loss(self) -> bool {
        matches!(self, MatchOutcome::Defeat)
    }

    pub fn is_decided(self) -> bool {
        !matches!(self, MatchOutcome::Unknown)
    }

    /// CSS suffix for history/dashboard rows: `win` / `loss` / `draw` / `undecided`.
    pub fn row_class(self) -> &'static str {
        match self {
            MatchOutcome::Victory => "win",
            MatchOutcome::Defeat => "loss",
            MatchOutcome::Draw => "draw",
            MatchOutcome::Unknown => "undecided",
        }
    }

    /// CSS class for outcome text colour.
    pub fn text_class(self) -> &'static str {
        match self {
            MatchOutcome::Victory => "outcome-win",
            MatchOutcome::Defeat => "outcome-loss",
            MatchOutcome::Draw => "outcome-draw",
            MatchOutcome::Unknown => "outcome-unknown",
        }
    }
}

// --------------------------------------------------------------- OCR reads

#[derive(Debug)]
pub struct CellOcrResult {
    pub value: String,
    pub confidence: i32,
    /// Glyph ink touches a vertical crop edge (CG-3 clip/bleed). Suspect values
    /// are still parsed — they may be right — but the capture gate strips their
    /// corroboration/un-latch influence (see `capture_gate::apply_gate`).
    pub suspect: bool,
}

#[derive(Debug)]
pub struct RowOcrResult {
    /// Read of name window 0 (`ocr::preprocess::name_windows`).
    pub name: Option<CellOcrResult>,
    /// Reads of the other name windows: `name_alternates[k]` is window
    /// `k + 1`, `None` where that read failed, so the index always names the
    /// window. Empty for 6v6, which has one window. The row matcher scores
    /// `name` and these together and keeps the best.
    pub name_alternates: Vec<Option<CellOcrResult>>,
    pub stats: Vec<CellOcrResult>,
    pub mean_confidence: i32,
}

/// Whether a cell's OCR text looks like a real stat value: at least one digit,
/// nothing but digits and thousands-separator commas.
pub fn is_clean_stat(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let has_digit = text.chars().any(|c| c.is_ascii_digit());
    let all_valid = text.chars().all(|c| c.is_ascii_digit() || c == ',');
    has_digit && all_valid
}

// ------------------------------------------------------------- stored games

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SurrealValue)]
pub struct PersonalMatch {
    /// Local store record id — set on rows read back from SurrealKV, `None`
    /// on freshly parsed captures. Excluded from the JSON snapshot/log
    /// (readers identify games by `session_id`); used to mark exactly the
    /// fetched rows as synced.
    #[serde(skip)]
    #[surreal(default)]
    pub id: Option<surrealdb_types::RecordId>,
    pub hero: String,
    pub map_name: String,
    pub game_mode: String,
    pub role: String,
    pub outcome: String,
    pub elims: u32,
    pub deaths: u32,
    pub assists: u32,
    pub damage: u32,
    pub healing: u32,
    pub mitigation: u32,
    pub played_at: SurrealDatetime,
    #[serde(default)]
    pub synced: bool,
    /// Payload revision for sync compare-and-swap. Every statement that
    /// changes an uploaded field bumps this in the same write that sets
    /// `synced = false`. `mark_synced` stamps a row only when the revision
    /// still matches the value captured at read time, so a local write during
    /// the HTTP upload is not marked synced. Missing on legacy rows (treated
    /// as 0).
    #[serde(default)]
    #[surreal(default)]
    pub sync_rev: u64,
    /// Server refused this row with HTTP 400 or 422. The row stays
    /// `synced = false` and is left out of automatic uploads until the
    /// member edits the game or retries the upload. The string is the
    /// game-list line (`Upload rejected by server: <message>`), including
    /// the server code when the body sent one. Missing on older rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub upload_reject: Option<String>,
    #[serde(default)]
    pub session_id: String,

    // --- Manual edit overlay (transparency) ---------------------------------
    // The OCR-captured values above are IMMUTABLE. A manual correction records
    // the fixed value in the matching `corrected_*` field and appends the field
    // name to `edited_fields`; the effective (displayed) value is `corrected_*`
    // if present, else the OCR read. This keeps "OCR read X → corrected Y"
    // recoverable for the badge/tooltip and never destroys the original read.
    // All fields are additive Options / empty-by-default, so rows written before
    // this feature deserialize unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_hero: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_map_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_elims: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_deaths: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_assists: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_damage: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_healing: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub corrected_mitigation: Option<u32>,
    /// Names of fields carrying a manual correction (e.g. `["elims", "hero"]`).
    /// Non-empty ⇒ the row is edited (drives the badge).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[surreal(default)]
    pub edited_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub edited_at: Option<SurrealDatetime>,

    // --- Hero timeline (HS-1) -----------------------------------------------
    /// Consecutive-run hero segments derived from the session's RAW snapshots
    /// (see `storage::derive_hero_segments`). The daemon stamps this after each
    /// capture; legacy rows (empty) are derived on the fly in
    /// `storage::latest_per_game`. Per-snapshot `hero`/`role` stay raw — this field
    /// carries the swap history, and the primary hero/role for the game is the
    /// snapshot-count majority over these segments ([`primary_hero_role`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[surreal(default)]
    pub heroes_played: Vec<HeroSegment>,
    /// Manual confirm/dismiss decisions for hero segments, by segment index.
    /// Source of truth for the flags inside `heroes_played`, which are
    /// recomputed from these on every re-derivation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[surreal(default)]
    pub segment_resolutions: Vec<SegmentResolution>,
    /// Recognizer that produced the stored numbers. Empty on rows written
    /// before the column existed; those read as `ocr-v1` at upload time.
    #[serde(default)]
    #[surreal(default)]
    pub recognizer: String,
    /// Flat or leftover row names the new reader was not sure about.
    /// Uploads flatten this to the server's allowlist.
    #[serde(default)]
    #[surreal(default)]
    pub suspect_fields: Vec<String>,
    /// Which reader found this player's own scoreboard row.
    /// `names-v1` or `ocr-v1`. Absent when this match never read a name.
    /// A later snapshot of the same match keeps the value already stored, and
    /// every upload of the match sends it. Omitted from JSON when absent, so
    /// an older row sends nothing and an older server still accepts the row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub name_recognizer: Option<String>,
}

/// Count an unconfirmed hero segment toward the majority (it was a real swap).
pub const SEGMENT_CONFIRM: &str = "confirm";
/// Mark a hero segment as a misread: excluded from display and from the
/// majority vote. The raw snapshots stay untouched.
pub const SEGMENT_DISMISS: &str = "dismiss";

/// One consecutive run of the same hero across a session's snapshots, in
/// capture-time order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SurrealValue)]
pub struct HeroSegment {
    pub hero: String,
    pub role: String,
    pub first_seen: SurrealDatetime,
    pub last_seen: SurrealDatetime,
    /// Number of captures in the run.
    pub snapshots: u32,
    /// ≥2 snapshots, or manually confirmed. A 1-snapshot segment is recorded
    /// but unconfirmed — a lone minority read is indistinguishable from a
    /// misread (the 07-18 Lijiang "Wuyang" flap), so it needs a human call.
    pub confirmed: bool,
    /// Manual resolution ([`SEGMENT_CONFIRM`] / [`SEGMENT_DISMISS`]), applied
    /// from the match's `segment_resolutions` during derivation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[surreal(default)]
    pub resolution: Option<String>,
}

impl HeroSegment {
    /// Manually dismissed as a misread — excluded from display and majority.
    pub fn is_dismissed(&self) -> bool {
        self.resolution.as_deref() == Some(SEGMENT_DISMISS)
    }
}

/// A manual decision about one derived hero segment, keyed by index into the
/// derived segment list (stable: snapshots only ever append, so existing
/// segments never change position).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SurrealValue)]
pub struct SegmentResolution {
    pub segment: u32,
    /// [`SEGMENT_CONFIRM`] or [`SEGMENT_DISMISS`].
    pub action: String,
}

impl PersonalMatch {
    /// True when at least one field carries a manual correction.
    pub fn is_edited(&self) -> bool {
        !self.edited_fields.is_empty()
    }

    /// Id sent on upload. A missing column is `ocr-v1`.
    pub fn stored_recognizer(&self) -> &str {
        scuffed_types::effective_recognizer(&self.recognizer)
    }

    /// Server message for a quarantined row. Empty and missing both mean
    /// the row is still eligible for upload.
    pub fn upload_rejection(&self) -> Option<&str> {
        self.upload_reject
            .as_deref()
            .map(str::trim)
            .filter(|message| !message.is_empty())
    }

    /// True when `field` (storage name, e.g. `"map_name"`) was manually edited.
    pub fn field_edited(&self, field: &str) -> bool {
        self.edited_fields.iter().any(|f| f == field)
    }

    /// True when this row carries a member's own map choice (a map edit or
    /// pick: `corrected_map_name`, or `map_name` in `edited_fields`).
    pub fn has_member_map_edit(&self) -> bool {
        self.corrected_map_name.is_some() || self.field_edited("map_name")
    }

    /// Effective hero: a manual correction wins, else the majority-derived
    /// primary from the hero timeline, else the raw OCR read.
    pub fn display_hero(&self) -> &str {
        if let Some(h) = self.corrected_hero.as_deref() {
            return h;
        }
        primary_hero_role(&self.heroes_played)
            .map(|(h, _)| h)
            .unwrap_or(&self.hero)
    }
    /// Effective role, resolved like [`Self::display_hero`].
    pub fn display_role(&self) -> &str {
        if let Some(r) = self.corrected_role.as_deref() {
            return r;
        }
        primary_hero_role(&self.heroes_played)
            .map(|(_, r)| r)
            .unwrap_or(&self.role)
    }
    pub fn display_map_name(&self) -> &str {
        self.corrected_map_name.as_deref().unwrap_or(&self.map_name)
    }
    pub fn display_outcome(&self) -> &str {
        self.corrected_outcome.as_deref().unwrap_or(&self.outcome)
    }
    pub fn display_elims(&self) -> u32 {
        self.corrected_elims.unwrap_or(self.elims)
    }
    pub fn display_deaths(&self) -> u32 {
        self.corrected_deaths.unwrap_or(self.deaths)
    }
    pub fn display_assists(&self) -> u32 {
        self.corrected_assists.unwrap_or(self.assists)
    }
    pub fn display_damage(&self) -> u32 {
        self.corrected_damage.unwrap_or(self.damage)
    }
    pub fn display_healing(&self) -> u32 {
        self.corrected_healing.unwrap_or(self.healing)
    }
    pub fn display_mitigation(&self) -> u32 {
        self.corrected_mitigation.unwrap_or(self.mitigation)
    }
}

/// The primary hero/role for a game: snapshot-count majority across its hero
/// segments. Dismissed segments never vote; unconfirmed segments vote only
/// when every non-dismissed segment is unconfirmed (a lone flap must not
/// outvote 25 solid captures, but an all-unconfirmed game still needs a
/// label). Ties break to the hero seen latest, then alphabetically for
/// determinism. `None` when nothing can vote (no segments, or all dismissed)
/// — callers fall back to the raw read.
pub fn primary_hero_role(segments: &[HeroSegment]) -> Option<(&str, &str)> {
    let live: Vec<&HeroSegment> = segments.iter().filter(|s| !s.is_dismissed()).collect();
    let voters: Vec<&HeroSegment> = if live.iter().any(|s| s.confirmed) {
        live.iter().copied().filter(|s| s.confirmed).collect()
    } else {
        live
    };
    struct Vote<'a> {
        count: u32,
        latest: chrono::DateTime<chrono::Utc>,
        role: &'a str,
    }
    let mut votes: std::collections::HashMap<&str, Vote> = std::collections::HashMap::new();
    for s in voters {
        let seen: chrono::DateTime<chrono::Utc> = s.last_seen.into();
        let v = votes.entry(s.hero.as_str()).or_insert(Vote {
            count: 0,
            latest: seen,
            role: &s.role,
        });
        v.count += s.snapshots;
        if seen >= v.latest {
            v.latest = seen;
            v.role = &s.role;
        }
    }
    votes
        .into_iter()
        .max_by(|a, b| {
            a.1.count
                .cmp(&b.1.count)
                .then(a.1.latest.cmp(&b.1.latest))
                .then(b.0.cmp(a.0))
        })
        .map(|(hero, v)| (hero, v.role))
}

// ------------------------------------------------------------ digit reader

/// Identifies this recognizer's output. Bump it whenever any value,
/// confidence or suspect flag can change, so logged reads from different
/// builds are never pooled by mistake. Every `shadow/digits.jsonl` line
/// carries it. The `recognizer_snapshot_is_pinned` test fails until the bump
/// is made when, on its synthetic fixtures, any value or suspect flag
/// changes, any calibration constant changes, a pinned sample confidence
/// moves more than 0.02, or a board's mean confidence moves more than 0.005.
/// Float drift below those tolerances is accepted on purpose.
///
/// * `cv-v1`: template matcher as shipped in 0.4.23 (raw margins, 0.06 cut).
/// * `cv-v2`: per-digit calibrated confidence, suspect below 0.35.
/// * `cv-v3`: 1080p templates and typical margins rebuilt with real
///   1920x1080 captures added to the downscaled 1440p set; cells with too
///   little ink for the digits read are flagged.
/// * `cv-v4`: pair checks for 4 vs 6 and for dim 0 vs 3 can lift a glyph
///   margin that sits below the suspect line; touching glyphs can also be
///   split along slanted cut lines.
/// * `cv-v5`: a thousands-separator gap (width in range, or bottom-band comma
///   ink) joins digit groups instead of splitting them; any ink left of the
///   picked group inside the cell forces suspect.
/// * `cv-v6`: alpha.8 reader. Written as `shadow::digits::RECOGNIZER_ID_NO_OCR` when the
///   tracker runs with OCR off, so those rows and reports can be told apart.
/// * `cv-v7`: header labels are found with
///   `ocr::preprocess::header_label_groups_scaled`: the dark cut follows the
///   strip's background brightness and a 2 px "H" label is kept on boards
///   narrower than 1664 px. Downscaled and dimmed boards now find their
///   columns; boards that already found them read the same.
/// * `cv-v8`: the digit reads are the same as `cv-v7`. A held own-row cell
///   now stores the reader's unsure read, flagged, when the capture gate
///   would take that read, instead of the game's older accepted value
///   (report a6f3). Stored stats can differ from `cv-v7` rows.
pub const RECOGNIZER_ID: &str = "cv-v8";
