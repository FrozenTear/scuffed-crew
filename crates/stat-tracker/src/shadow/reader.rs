//! Single entry point for the main-reader switch: one Tab frame in, every
//! field out with value, confidence and suspect flag.
//!
//! Field names are what the API validates:
//!
//! * `map`, `mode`, `result`
//! * `r{N}.hero` and `r{N}.{e,a,d,dmg,h,mit}`, rows numbered top to bottom
//!   over the whole board: `0..=9` for 5v5, `0..=11` for 6v6 (team 1 first,
//!   then team 2, the order the digit and hero matchers already use).
//!
//! When no Tab board is on the frame, the read says so ([`BoardStatus`]):
//! `team_size` is `None` and only the match fields (`map`, `mode`, `result`)
//! are present, all unread. Nothing falls back to 5v5.
//!
//! Otherwise every field is always present, in [`field_names`] order. A field
//! the reader could not read has no value, confidence 0 and `suspect: true`:
//!
//! * no hero icon pack (or no map / result pack): those fields are suspect,
//! * `map` goes through ocr-v1's own name function
//!   ([`crate::parse::canonical_map`], see [`map_name`]) so both readers store
//!   the same string; a map ocr-v1 cannot name is kept but suspect,
//! * `mode` comes from the map (single-mode maps only) and is suspect
//!   whenever the map is,
//! * `result` comes from result frames (accolade screen, rank screen, see
//!   [`super::result`]), never from the Tab frame alone: each result frame
//!   gives a source-tagged [`ResultRead`], all reads of one match go into one
//!   [`ResultEvidence`], and the field is sure only with
//!   [`super::result::MIN_AGREE`] agreeing sure reads and no conflict. Without
//!   evidence it stays suspect.
//!
//! Packs load from a configurable directory ([`ReaderConfig`]). Nothing here
//! writes stats; the caller decides what to store.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use image::DynamicImage;
use serde::Serialize;

use super::banner::{MapInfo, MapTemplates};
use super::digits;
use super::heroes::{HeroTemplates, RowClass};
use super::result::{ResultEvidence, ResultRead, ResultTemplates};

/// Stat field suffixes, in board column order (`digits::FIELDS`).
pub const STAT_FIELDS: [&str; 6] = ["e", "a", "d", "dmg", "h", "mit"];
/// Time allowed for the digit and hero matchers per frame.
pub const READ_BUDGET: Duration = Duration::from_secs(2);
/// Env var naming the pack root used when [`init`] was never called.
pub const TEMPLATE_DIR_ENV: &str = "SCUFFED_TEMPLATE_DIR";

/// A field value: stats are integers, names are text.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Value {
    Int(u32),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FieldRead {
    pub name: String,
    pub value: Option<Value>,
    pub confidence: f32,
    pub suspect: bool,
}

impl FieldRead {
    fn unread(name: String) -> Self {
        Self {
            name,
            value: None,
            confidence: 0.0,
            suspect: true,
        }
    }
}

/// Whether a Tab board was found on the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardStatus {
    /// A board with a measured 5v5 or 6v6 layout: every field is present.
    Read,
    /// No board: the capture path's preflight (row dips or header stat
    /// labels) fails, the six stat labels are not in the Tab header strip,
    /// or fewer than 3 rows have 4 read stat cells (ocr-v1's final check).
    /// `team_size` is `None`, no row fields.
    NotFound,
    /// Passed the preflight, but the row pitch gives no 5v5 or 6v6 under
    /// Tracker's rule (no pitch, implausible pitches, or dip and spectral
    /// pitches that disagree). `team_size` is `None`, no row fields.
    TeamSizeUnknown,
}

/// Every field of one frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoardRead {
    pub status: BoardStatus,
    /// 5 or 6; `None` unless `status` is [`BoardStatus::Read`].
    pub team_size: Option<usize>,
    pub fields: Vec<FieldRead>,
    pub elapsed_ms: u32,
}

impl BoardRead {
    pub fn get(&self, name: &str) -> Option<&FieldRead> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Names of the suspect fields, in field order.
    pub fn suspect_fields(&self) -> Vec<&str> {
        self.fields
            .iter()
            .filter(|f| f.suspect)
            .map(|f| f.name.as_str())
            .collect()
    }

    /// Replace the `result` field (from [`Reader::read_result`]).
    pub fn set_result(&mut self, result: FieldRead) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.name == "result") {
            *f = FieldRead {
                name: "result".into(),
                ..result
            };
        }
    }
}

/// Names of the suspect fields of `board` (owned, for logs and the API).
pub fn suspect_field_names(board: &BoardRead) -> Vec<String> {
    board
        .suspect_fields()
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Match-level fields, present on every read (also without a board).
pub const MATCH_FIELDS: [&str; 3] = ["map", "mode", "result"];

/// All field names for a board of `team_size` (5 or 6), in output order.
pub fn field_names(team_size: usize) -> Vec<String> {
    let mut out: Vec<String> = MATCH_FIELDS.iter().map(|s| s.to_string()).collect();
    for r in 0..2 * team_size {
        out.push(format!("r{r}.hero"));
        for s in STAT_FIELDS {
            out.push(format!("r{r}.{s}"));
        }
    }
    out
}

/// Where the optional packs live. `None` skips that reader.
#[derive(Debug, Clone, Default)]
pub struct ReaderConfig {
    pub hero_dir: Option<PathBuf>,
    pub map_dir: Option<PathBuf>,
    pub result_dir: Option<PathBuf>,
}

impl ReaderConfig {
    /// `<root>/templates/{heroes,maps,result}`; `root` is the data dir.
    pub fn from_data_dir(root: &Path) -> Self {
        Self {
            hero_dir: Some(HeroTemplates::dir_in(root)),
            map_dir: Some(MapTemplates::dir_in(root)),
            result_dir: Some(ResultTemplates::dir_in(root)),
        }
    }

    /// [`TEMPLATE_DIR_ENV`] as the data dir, else no packs.
    pub fn from_env() -> Self {
        std::env::var_os(TEMPLATE_DIR_ENV)
            .map(|d| Self::from_data_dir(Path::new(&d)))
            .unwrap_or_default()
    }
}

/// The loaded packs. Digits need none (their templates are embedded).
#[derive(Default)]
pub struct Reader {
    heroes: Option<HeroTemplates>,
    maps: Option<MapTemplates>,
    results: Option<ResultTemplates>,
}

impl Reader {
    pub fn load(config: &ReaderConfig) -> Self {
        let heroes = config.hero_dir.as_deref().and_then(HeroTemplates::load_dir);
        let maps = config.map_dir.as_deref().and_then(MapTemplates::load_dir);
        let results = config
            .result_dir
            .as_deref()
            .and_then(ResultTemplates::load_dir);
        tracing::info!(
            heroes = heroes.is_some(),
            maps = maps.is_some(),
            results = results.is_some(),
            "shadow reader packs"
        );
        Self {
            heroes,
            maps,
            results,
        }
    }

    pub fn has_heroes(&self) -> bool {
        self.heroes.is_some()
    }

    /// Read every field of one full Tab frame. `result` stays suspect.
    /// Without a board (or without a clear 5v5 / 6v6 layout) the read is
    /// [`not_found`] with that status: no team size, no row fields, no map.
    pub fn read_board(&self, frame: &DynamicImage) -> BoardRead {
        let t0 = Instant::now();
        let scoreboard = crate::ocr::preprocess::crop_scoreboard(frame);
        let team_size = match board_layout(&scoreboard) {
            Ok(n) => n,
            Err(status) => {
                let mut b = not_found(status);
                b.elapsed_ms = elapsed_ms(t0);
                return b;
            }
        };
        let digit_read = digits::read_board(&scoreboard, team_size, READ_BUDGET).ok();
        if digit_read.as_ref().is_some_and(|d| !has_stat_rows(d)) {
            let mut b = not_found(BoardStatus::NotFound);
            b.elapsed_ms = elapsed_ms(t0);
            return b;
        }
        let hero_read = self
            .heroes
            .as_ref()
            .and_then(|h| h.read_board(&scoreboard, team_size, READ_BUDGET).ok());
        let mut board = assemble(
            team_size,
            digit_read.as_ref(),
            hero_read.as_ref().map(|h| &h.rows[..]),
        );
        if let Some(maps) = &self.maps {
            let m = maps.read(&frame.to_rgb8());
            if let Some(info) = m.map {
                let n = map_name(info);
                let suspect = m.suspect || !n.known_to_ocr_v1;
                set(&mut board, "map", Value::Text(n.name), m.score, suspect);
                if let Some(mode) = n.mode {
                    set(
                        &mut board,
                        "mode",
                        Value::Text(mode.into()),
                        m.score,
                        suspect,
                    );
                }
            }
        }
        board.elapsed_ms = elapsed_ms(t0);
        board
    }

    /// Read one result frame (accolade screen, rank screen, or a plugged-in
    /// layout). Add the read to the match's [`ResultEvidence`].
    pub fn read_result(&self, result_frame: &DynamicImage) -> ResultRead {
        match &self.results {
            Some(t) => t.read(&result_frame.to_rgb8()),
            None => ResultRead {
                outcome: None,
                source: None,
                score: 0.0,
                margin: 0.0,
                suspect: true,
            },
        }
    }

    /// [`Self::read_board`] with the `result` field from a match's evidence.
    pub fn read_board_with_evidence(
        &self,
        frame: &DynamicImage,
        evidence: &ResultEvidence,
    ) -> BoardRead {
        let mut b = self.read_board(frame);
        b.set_result(result_field(evidence));
        b
    }

    /// [`Self::read_board`] plus the result from this match's result frames.
    pub fn read_board_with_results(
        &self,
        frame: &DynamicImage,
        result_frames: &[&DynamicImage],
    ) -> BoardRead {
        let mut e = ResultEvidence::default();
        for f in result_frames {
            e.add(self.read_result(f));
        }
        self.read_board_with_evidence(frame, &e)
    }
}

fn elapsed_ms(t0: Instant) -> u32 {
    t0.elapsed().as_millis().min(u32::MAX as u128) as u32
}

/// The layout of a cropped scoreboard. A board is found with the capture
/// path's preflight (main.rs): row dips at a plausible pitch, or 3 to 10
/// header stat labels. See [`layout_from_scan`] for the size.
///
/// The preflight alone also passes gameplay frames, Practice Range and
/// history Teams screens: their rows give dips at a 5v5-like pitch. None of
/// them has the six stat labels in the Tab header strip, so a board also
/// needs [`digits::stat_columns_found`]; otherwise it is
/// [`BoardStatus::NotFound`].
pub fn board_layout(scoreboard: &DynamicImage) -> Result<usize, BoardStatus> {
    let scan = crate::detect::hero_portrait::scan_rows(scoreboard);
    let labels = crate::ocr::preprocess::header_label_groups(scoreboard).len();
    let n = layout_from_scan(&scan, labels);
    if n != Err(BoardStatus::NotFound) && !digits::stat_columns_found(scoreboard) {
        return Err(BoardStatus::NotFound);
    }
    n
}

/// Rows with at least this many read stat cells count toward a board, and a
/// board needs [`MIN_STAT_ROWS`] of them: ocr-v1's final check
/// (`parse::looks_like_scoreboard`, 3 rows with 4 clean cells) on the digit
/// reads instead of OCR text.
const MIN_STAT_CELLS: usize = 4;
const MIN_STAT_ROWS: usize = 3;

fn has_stat_rows(d: &digits::BoardRead) -> bool {
    d.rows
        .iter()
        .filter(|r| r.cells.iter().filter(|c| c.value.is_some()).count() >= MIN_STAT_CELLS)
        .count()
        >= MIN_STAT_ROWS
}

/// The size comes from Tracker's own rule, `RowScan::checked_team_size`
/// (#158): split at a row pitch of 0.079 (6v6 below, 5v5 at or above), pitches
/// outside 0.062 to 0.095 ignored, and dip and spectral pitches that both
/// count but disagree give no size. Here that is
/// [`BoardStatus::TeamSizeUnknown`], as the capture path rejects such a frame
/// as team size uncertain. With no pitch at all there is no layout either,
/// where `checked_team_size` would default to 5. Nothing guesses a size.
pub fn layout_from_scan(
    scan: &crate::detect::hero_portrait::RowScan,
    header_labels: usize,
) -> Result<usize, BoardStatus> {
    if !scan.looks_like_scoreboard() && !(3..=10).contains(&header_labels) {
        return Err(BoardStatus::NotFound);
    }
    if scan.spectral_pitch.is_none() && scan.median_pitch.is_none() {
        return Err(BoardStatus::TeamSizeUnknown);
    }
    match scan.checked_team_size() {
        Some(n @ (5 | 6)) => Ok(n),
        _ => Err(BoardStatus::TeamSizeUnknown),
    }
}

/// A read for a frame without a usable board: `team_size` is `None` and only
/// the match fields are present, all unread.
pub fn not_found(status: BoardStatus) -> BoardRead {
    BoardRead {
        status,
        team_size: None,
        fields: MATCH_FIELDS
            .iter()
            .map(|s| FieldRead::unread(s.to_string()))
            .collect(),
        elapsed_ms: 0,
    }
}

/// The map name and mode the reader emits for a banner match.
#[derive(Debug, Clone, PartialEq)]
pub struct MapName {
    pub name: String,
    pub mode: Option<&'static str>,
    /// ocr-v1's table names this map (as this map, not another one). When
    /// false, the field stays suspect: ocr-v1 would not store the name.
    pub known_to_ocr_v1: bool,
}

/// Route a banner match through ocr-v1's map-name function
/// ([`crate::parse::canonical_map`]) so both readers emit the same string and
/// mode ([`crate::parse::map_mode`]).
///
/// Two banner maps fold onto a different map in ocr-v1's table: "Temple of
/// Anubis" (pattern `anubis` gives Throne of Anubis) and "Ecopoint:
/// Antarctica" (pattern `antarctic` gives Antarctic Peninsula). The banner
/// match knows which map it saw, so those keep their own name and are
/// suspect rather than stored as the wrong map. Maps ocr-v1 has no entry for
/// (Practice Range, Hanamura, ...) also keep their banner name, suspect.
pub fn map_name(info: &MapInfo) -> MapName {
    let other_banner_map = |n: &str| super::banner::MAPS.iter().any(|m| m.name == n);
    match crate::parse::canonical_map(info.name) {
        Some(n) if n == info.name || !other_banner_map(&n) => MapName {
            mode: crate::parse::map_mode(&n).or(info.mode),
            name: n,
            known_to_ocr_v1: true,
        },
        _ => MapName {
            name: info.name.to_string(),
            mode: info.mode,
            known_to_ocr_v1: false,
        },
    }
}

/// The `result` field a match's evidence supports.
pub fn result_field(evidence: &ResultEvidence) -> FieldRead {
    let d = evidence.decide();
    FieldRead {
        name: "result".into(),
        value: d.outcome.map(|o| Value::Text(o.as_str().into())),
        confidence: d.confidence,
        suspect: d.suspect,
    }
}

fn set(board: &mut BoardRead, name: &str, value: Value, confidence: f32, suspect: bool) {
    if let Some(f) = board.fields.iter_mut().find(|f| f.name == name) {
        f.value = Some(value);
        f.confidence = confidence;
        f.suspect = suspect;
    }
}

/// Build the field list from the digit and hero matcher outputs. Rows in
/// both are already numbered top to bottom over the full board.
pub fn assemble(
    team_size: usize,
    digit_read: Option<&digits::BoardRead>,
    hero_rows: Option<&[super::heroes::HeroRead]>,
) -> BoardRead {
    let mut board = BoardRead {
        status: BoardStatus::Read,
        team_size: Some(team_size),
        fields: field_names(team_size)
            .into_iter()
            .map(FieldRead::unread)
            .collect(),
        elapsed_ms: 0,
    };
    if let Some(d) = digit_read {
        for (r, row) in d.rows.iter().enumerate().take(2 * team_size) {
            for (k, c) in row.cells.iter().enumerate() {
                let name = format!("r{r}.{}", STAT_FIELDS[k]);
                if let Some(f) = board.fields.iter_mut().find(|f| f.name == name) {
                    f.value = c.value.map(Value::Int);
                    f.confidence = c.confidence;
                    f.suspect = c.suspect || c.value.is_none();
                }
            }
        }
    }
    for h in hero_rows.unwrap_or(&[]) {
        if h.row >= 2 * team_size {
            continue;
        }
        let name = format!("r{}.hero", h.row);
        if let Some(f) = board.fields.iter_mut().find(|f| f.name == name) {
            match (h.class, &h.hero) {
                (RowClass::Hero, Some(hero)) if !h.suspect => {
                    f.value = Some(Value::Text(hero.clone()));
                    f.confidence = h.score;
                    f.suspect = false;
                }
                // an empty slot (6v6 waiting for a player) has no hero to read
                (RowClass::Empty, _) if !h.suspect => {
                    f.confidence = h.score;
                    f.suspect = false;
                }
                _ => f.confidence = h.score,
            }
        }
    }
    board
}

static READER: OnceLock<Reader> = OnceLock::new();

/// Load the packs for [`read_board`]. Only the first call has an effect;
/// returns false when the reader was already set up.
pub fn init(config: &ReaderConfig) -> bool {
    READER.set(Reader::load(config)).is_ok()
}

/// Read every field of one Tab frame with the process-wide reader (packs
/// from [`init`], else from [`TEMPLATE_DIR_ENV`], else none).
pub fn read_board(frame: &DynamicImage) -> BoardRead {
    READER
        .get_or_init(|| Reader::load(&ReaderConfig::from_env()))
        .read_board(frame)
}

/// Read one result frame with the process-wide reader.
pub fn read_result(result_frame: &DynamicImage) -> ResultRead {
    READER
        .get_or_init(|| Reader::load(&ReaderConfig::from_env()))
        .read_result(result_frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::hero_portrait::RowScan;
    use crate::shadow::digits::{CellRead, RowRead};
    use crate::shadow::heroes::HeroRead;

    fn digits_board(rows: usize) -> digits::BoardRead {
        digits::BoardRead {
            rows: (0..rows)
                .map(|r| RowRead {
                    cells: std::array::from_fn(|k| CellRead {
                        value: Some((r * 10 + k) as u32),
                        confidence: 0.9,
                        suspect: r == 3 && k == 2,
                    }),
                })
                .collect(),
            elapsed_ms: 1,
        }
    }

    fn hero(row: usize, class: RowClass, hero: Option<&str>, suspect: bool) -> HeroRead {
        HeroRead {
            row,
            class,
            hero: hero.map(str::to_string),
            best: hero.unwrap_or("?").into(),
            score: 0.8,
            lead: 0.2,
            second: "x".into(),
            suspect,
        }
    }

    #[test]
    fn five_v_five_rows_are_0_to_9() {
        let n = field_names(5);
        assert_eq!(&n[..3], ["map", "mode", "result"]);
        assert_eq!(n.len(), 3 + 10 * 7);
        assert_eq!(
            &n[3..10],
            [
                "r0.hero", "r0.e", "r0.a", "r0.d", "r0.dmg", "r0.h", "r0.mit"
            ]
        );
        assert_eq!(n.last().unwrap(), "r9.mit");
        assert!(!n.iter().any(|f| f.starts_with("r10.")));
    }

    #[test]
    fn six_v_six_rows_are_0_to_11() {
        let n = field_names(6);
        assert_eq!(n.len(), 3 + 12 * 7);
        assert_eq!(n.last().unwrap(), "r11.mit");
        assert!(n.contains(&"r6.hero".to_string()));
        assert!(!n.iter().any(|f| f.starts_with("r12.")));
    }

    #[test]
    fn rows_map_top_to_bottom_over_both_teams() {
        // 6v6: digit row 7 is team 2's second row and must land on r7
        let b = assemble(6, Some(&digits_board(12)), None);
        assert_eq!(b.get("r0.e").unwrap().value, Some(Value::Int(0)));
        assert_eq!(b.get("r7.d").unwrap().value, Some(Value::Int(72)));
        assert_eq!(b.get("r11.mit").unwrap().value, Some(Value::Int(115)));
        let b5 = assemble(5, Some(&digits_board(10)), None);
        assert_eq!(b5.get("r5.e").unwrap().value, Some(Value::Int(50)));
        assert_eq!(b5.get("r9.mit").unwrap().value, Some(Value::Int(95)));
    }

    #[test]
    fn suspect_list_names_unread_and_unsure_fields() {
        let heroes = [
            hero(0, RowClass::Hero, Some("ana"), false),
            hero(1, RowClass::Unknown, None, true),
            hero(2, RowClass::Empty, None, false),
        ];
        let b = assemble(5, Some(&digits_board(10)), Some(&heroes));
        assert_eq!(
            b.get("r0.hero").unwrap().value,
            Some(Value::Text("ana".into()))
        );
        let s = suspect_field_names(&b);
        assert!(s.contains(&"r3.d".to_string()), "unsure digit");
        assert!(s.contains(&"r1.hero".to_string()), "flagged hero");
        assert!(!s.contains(&"r0.hero".to_string()));
        assert!(
            !s.contains(&"r2.hero".to_string()),
            "empty slot is not suspect"
        );
        assert!(s.contains(&"r3.hero".to_string()), "no read at all");
        for f in ["map", "mode", "result"] {
            assert!(s.contains(&f.to_string()));
        }
        assert!(!s.contains(&"r0.e".to_string()));
    }

    #[test]
    fn no_hero_pack_leaves_hero_fields_suspect() {
        let b = assemble(5, Some(&digits_board(10)), None);
        for r in 0..10 {
            let f = b.get(&format!("r{r}.hero")).unwrap();
            assert!(f.suspect && f.value.is_none());
        }
        let reader = Reader::load(&ReaderConfig::default());
        assert!(!reader.has_heroes());
    }

    #[test]
    fn set_result_fills_the_result_field() {
        let mut b = assemble(5, None, None);
        b.set_result(FieldRead {
            name: "anything".into(),
            value: Some(Value::Text("defeat".into())),
            confidence: 0.9,
            suspect: false,
        });
        let f = b.get("result").unwrap();
        assert_eq!(f.value, Some(Value::Text("defeat".into())));
        assert!(!f.suspect);
        // without a result pack the reader reads nothing, and no evidence
        // leaves the field suspect
        let reader = Reader::load(&ReaderConfig::default());
        let img = DynamicImage::new_rgb8(64, 36);
        assert_eq!(reader.read_result(&img).outcome, None);
        let b = reader.read_board_with_results(&img, &[&img]);
        assert!(b.get("result").unwrap().suspect);
    }

    #[test]
    fn result_field_needs_two_agreeing_sure_reads() {
        use crate::shadow::result::{ACCOLADE, Outcome, RANK_SCREEN};
        let read = |src: &str| ResultRead {
            outcome: Some(Outcome::Defeat),
            source: Some(src.into()),
            score: 0.95,
            margin: 0.5,
            suspect: false,
        };
        let mut e = ResultEvidence::default();
        e.add(read(RANK_SCREEN));
        assert!(result_field(&e).suspect);
        e.add(read(ACCOLADE));
        let f = result_field(&e);
        assert_eq!(f.value, Some(Value::Text("defeat".into())));
        assert!(!f.suspect);
    }

    #[test]
    fn no_board_is_not_found_without_team_size() {
        let reader = Reader::load(&ReaderConfig::default());
        for (w, h) in [(2560, 1440), (1920, 1080)] {
            for img in [
                DynamicImage::new_rgb8(w, h),
                DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                    w,
                    h,
                    image::Rgb([200, 200, 200]),
                )),
            ] {
                let b = reader.read_board(&img);
                assert_eq!(b.status, BoardStatus::NotFound, "{w}x{h}");
                assert_eq!(b.team_size, None, "no 5v5 fallback");
                let names: Vec<&str> = b.fields.iter().map(|f| f.name.as_str()).collect();
                assert_eq!(names, MATCH_FIELDS);
                assert!(b.fields.iter().all(|f| f.suspect && f.value.is_none()));
            }
        }
        let json = serde_json::to_string(&not_found(BoardStatus::NotFound)).unwrap();
        assert!(json.contains(r#""status":"not_found""#), "{json}");
        assert!(json.contains(r#""team_size":null"#), "{json}");
        let b = assemble(6, None, None);
        assert_eq!((b.status, b.team_size), (BoardStatus::Read, Some(6)));
    }

    /// Top-left of the 1440p scoreboard crop inside a 2560x1440 frame, and
    /// its size (`crop_scoreboard`).
    const CROP_X: u32 = 448;
    const CROP_Y: u32 = 216;
    const CROP_W: u32 = 1664;
    const CROP_H: u32 = 1007;

    /// Where `crop_scoreboard` cuts the board out of a `w`x`h` frame, as
    /// (x, y, width, height). Checked against the real crop in
    /// [`crop_rect_matches_crop_scoreboard`].
    fn crop_rect(w: u32, h: u32) -> (u32, u32, u32, u32) {
        let (gx, gy, gw, gh) = crate::ocr::preprocess::game_rect_16_9(w, h);
        let x = gx + (gw as f64 * 0.175) as u32;
        let y = gy + (gh as f64 * 0.15) as u32;
        let cw = ((gw as f64 * 0.65) as u32).min(w - x);
        let ch = ((gh as f64 * 0.70) as u32).min(h - y);
        (x, y, cw, ch)
    }

    /// Crop px at the 1440p reference board (1007 px high) scaled to `ch`.
    fn at_scale(px: f64, ch: u32) -> f64 {
        px * ch as f64 / CROP_H as f64
    }

    /// Six dark stat labels on a bright strip, 26 reference px high, at
    /// crop row `top`: where a Tab board has E A D DMG H MIT.
    fn paint_header(img: &mut image::RgbImage, frame_w: u32, frame_h: u32, top: u32) {
        let (cx, cy, cw, ch) = crop_rect(frame_w, frame_h);
        let strip = at_scale(26.0, ch).round() as u32;
        let half = at_scale(6.0, ch);
        for dy in 0..strip {
            for x in 0..cw {
                let label = (0..6)
                    .any(|i| (x as f64 - at_scale(960.0 + i as f64 * 100.0, ch)).abs() < half);
                let v = if label { 30 } else { 225 };
                img.put_pixel(cx + x, cy + top + dy, image::Rgb([v, v, v]));
            }
        }
    }

    /// Synthetic stand-in for a frame that is not a Tab board but passes the
    /// row-dip preflight: saturated background with grey bands across the
    /// crop's name strip every `pitch` of crop height, from the top down to
    /// `rows_to` of the crop height. `header_at` paints six dark stat labels
    /// on a bright strip at that crop row (0 = the Tab header strip, in
    /// 1440p reference px). Built from the measured signals only (frame
    /// size, dip count, pitch, header labels), no captured pixels.
    fn stand_in_at(
        frame_w: u32,
        frame_h: u32,
        pitch: f64,
        rows_to: f64,
        header_at: Option<u32>,
    ) -> DynamicImage {
        let (cx, cy, cw, ch) = crop_rect(frame_w, frame_h);
        let mut img = image::RgbImage::from_pixel(frame_w, frame_h, image::Rgb([40, 90, 160]));
        let band = at_scale(12.0, ch).round().max(2.0) as u32;
        let p = pitch * ch as f64;
        let mut y = p / 2.0;
        while y < rows_to * ch as f64 {
            for dy in 0..band {
                for x in 0..cw {
                    img.put_pixel(cx + x, cy + y as u32 + dy, image::Rgb([150, 150, 150]));
                }
            }
            y += p;
        }
        if let Some(top) = header_at {
            paint_header(&mut img, frame_w, frame_h, at_scale(top as f64, ch) as u32);
        }
        DynamicImage::ImageRgb8(img)
    }

    fn stand_in(pitch: f64, rows_to: f64, header_at: Option<u32>) -> DynamicImage {
        stand_in_at(2560, 1440, pitch, rows_to, header_at)
    }

    #[test]
    fn crop_rect_matches_crop_scoreboard() {
        assert_eq!(crop_rect(2560, 1440), (CROP_X, CROP_Y, CROP_W, CROP_H));
        for (w, h) in [
            (2560, 1440),
            (1920, 1080),
            (2048, 1152),
            (3840, 2160),
            (2560, 1437),
        ] {
            let (x, y, cw, ch) = crop_rect(w, h);
            let mut m = image::RgbImage::new(w, h);
            m.put_pixel(x, y, image::Rgb([255, 0, 0]));
            m.put_pixel(x + cw - 1, y + ch - 1, image::Rgb([0, 255, 0]));
            let c = crate::ocr::preprocess::crop_scoreboard(&DynamicImage::ImageRgb8(m)).to_rgb8();
            assert_eq!(c.dimensions(), (cw, ch), "{w}x{h}");
            assert_eq!(c.get_pixel(0, 0).0, [255, 0, 0], "{w}x{h}");
            assert_eq!(c.get_pixel(cw - 1, ch - 1).0, [0, 255, 0], "{w}x{h}");
        }
    }

    #[test]
    fn gameplay_practice_and_history_stand_ins_are_not_found() {
        let marker = {
            let mut m = image::RgbImage::new(2560, 1440);
            m.put_pixel(CROP_X, CROP_Y, image::Rgb([255, 0, 0]));
            crate::ocr::preprocess::crop_scoreboard(&DynamicImage::ImageRgb8(m)).to_rgb8()
        };
        assert_eq!(marker.dimensions(), (CROP_W, CROP_H));
        assert_eq!(marker.get_pixel(0, 0).0, [255, 0, 0], "crop origin moved");

        let reader = Reader::load(&ReaderConfig::default());
        let cases = [
            // gameplay frames (dip and spectral pitch as measured: 0.090,
            // 0.094, 0.086), no header labels
            ("gameplay a", stand_in(0.0904, 0.45, None)),
            ("gameplay b", stand_in(0.0943, 0.45, None)),
            ("gameplay c", stand_in(0.0864, 0.45, None)),
            // Practice Range: one team block of rows at about 0.084, board
            // header not in the Tab header strip
            ("practice range", stand_in(0.0844, 0.5, None)),
            // history Teams screen: rows at about 0.084, its header sits well
            // below the Tab header strip
            ("history teams", stand_in(0.0844, 0.45, Some(150))),
            // six stat labels where a Tab board has them, rows, but no stat
            // digits at all: ocr-v1's 3-rows-of-4-cells check fails
            ("labels, no digits", stand_in(0.0904, 0.45, Some(0))),
        ];
        for (what, frame) in cases {
            let crop = crate::ocr::preprocess::crop_scoreboard(&frame);
            let scan = crate::detect::hero_portrait::scan_rows(&crop);
            let labels = crate::ocr::preprocess::header_label_groups(&crop).len();
            // the stand-in fools the preflight and the pitch rule alone
            assert!(scan.looks_like_scoreboard(), "{what}: {scan:?}");
            assert_eq!(layout_from_scan(&scan, labels), Ok(5), "{what}: {scan:?}");
            let b = reader.read_board(&frame);
            assert_eq!(b.status, BoardStatus::NotFound, "{what}");
            assert_eq!(b.team_size, None, "{what}");
            let names: Vec<&str> = b.fields.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(names, MATCH_FIELDS, "{what}");
        }
        // the labels stand-in has its columns; the others do not
        let cols = |f: &DynamicImage| {
            digits::stat_columns_found(&crate::ocr::preprocess::crop_scoreboard(f))
        };
        assert!(cols(&stand_in(0.0904, 0.45, Some(0))));
        assert!(!cols(&stand_in(0.0844, 0.45, Some(150))));
        assert!(!cols(&stand_in(0.0904, 0.45, None)));
    }

    /// Name, frame width and height, dip pitch, spectral pitch, and the
    /// layout the pitch rule gives on those two pitches.
    type FalseBoard = (
        &'static str,
        u32,
        u32,
        f64,
        Option<f64>,
        Result<usize, BoardStatus>,
    );

    /// Tracker's 10 false boards from public X posts (#206): frame size,
    /// measured dip and spectral pitch, and what the pitch rule alone says.
    /// None of them is a Tab board, and none has the six stat labels in the
    /// Tab header strip (0 to 2 label groups each).
    const X_FALSE_BOARDS: [FalseBoard; 10] = [
        ("PotG a", 1920, 1080, 0.0833, Some(0.1005), Ok(5)),
        ("PotG b", 1920, 1080, 0.0833, Some(0.1005), Ok(5)),
        (
            "PotG 2048",
            2048,
            1152,
            0.1079,
            None,
            Err(BoardStatus::TeamSizeUnknown),
        ),
        ("video thumbnail", 1920, 1080, 0.0926, None, Ok(5)),
        ("Top 500", 1920, 1080, 0.0926, Some(0.0992), Ok(5)),
        ("match history", 1920, 1080, 0.0913, Some(0.0860), Ok(5)),
        ("graphic 1080", 1920, 1080, 0.0952, Some(0.0847), Ok(5)),
        (
            "graphic 4K",
            3840,
            2160,
            0.1085,
            None,
            Err(BoardStatus::TeamSizeUnknown),
        ),
        // the old rule (spectral first) gave 6
        (
            "hero menu",
            1920,
            1080,
            0.0886,
            Some(0.0714),
            Err(BoardStatus::TeamSizeUnknown),
        ),
        ("graphic 2048", 2048, 1152, 0.1104, Some(0.0782), Ok(6)),
    ];

    /// The pitch that decides the size under the pitch rule (the plausible
    /// one), else the dip pitch.
    fn deciding_pitch(dip: f64, spectral: Option<f64>) -> f64 {
        let ok = |p: f64| (0.062..=0.095).contains(&p);
        if ok(dip) {
            dip
        } else {
            spectral.filter(|&p| ok(p)).unwrap_or(dip)
        }
    }

    #[test]
    fn x_false_board_stand_ins_are_not_found() {
        let reader = Reader::load(&ReaderConfig::default());
        for (what, w, h, dip, spectral, rule) in X_FALSE_BOARDS {
            // the pitch rule alone, on the measured values
            let measured = scan(4, Some(dip), spectral);
            assert!(measured.looks_like_scoreboard(), "{what}");
            assert_eq!(layout_from_scan(&measured, 0), rule, "{what}");

            // a stand-in at the same frame size with rows at the pitch that
            // decided the size, no stat labels in the header strip
            let pitch = deciding_pitch(dip, spectral);
            let frame = stand_in_at(w, h, pitch, 0.45, None);
            let crop = crate::ocr::preprocess::crop_scoreboard(&frame);
            let scan = crate::detect::hero_portrait::scan_rows(&crop);
            let labels = crate::ocr::preprocess::header_label_groups(&crop).len();
            assert!(scan.looks_like_scoreboard(), "{what}: {scan:?}");
            let got = scan.median_pitch.unwrap();
            assert!(
                (got - pitch).abs() < 0.004,
                "{what}: pitch {got} vs {pitch}"
            );
            if let Ok(n) = rule {
                assert_eq!(layout_from_scan(&scan, labels), Ok(n), "{what}: {scan:?}");
            }
            assert!(!digits::stat_columns_found(&crop), "{what}");

            let b = reader.read_board(&frame);
            assert_eq!(b.status, BoardStatus::NotFound, "{what}");
            assert_eq!(b.team_size, None, "{what}");
            let names: Vec<&str> = b.fields.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(names, MATCH_FIELDS, "{what}");
        }
    }

    /// Stat values for a synthetic board, two teams of up to 6 rows. Row 1
    /// is a fresh 0 row, the others carry 1 to 6 digits and commas.
    const STAND_IN_STATS: [[u32; 6]; 12] = [
        [12, 3, 3, 5480, 950, 2347],
        [0, 0, 0, 0, 0, 0],
        [27, 14, 9, 18744, 1203, 87],
        [5, 10, 1, 101, 23456, 0],
        [8, 2, 4, 9876, 33, 104512],
        [41, 6, 7, 765, 4321, 11],
        [1, 11, 2, 3300, 7, 1000],
        [16, 5, 0, 22058, 619, 4096],
        [3, 19, 8, 47, 15998, 2],
        [9, 4, 6, 6140, 280, 39017],
        [22, 7, 5, 11111, 0, 765],
        [6, 1, 3, 8023, 1450, 333],
    ];

    /// Blue and red, the default team colours.
    const BLUE_RED: [[u8; 3]; 2] = [[40, 110, 220], [200, 50, 50]];

    /// Synthetic stand-in shaped like a Tab board: per team, `team` rows at
    /// `pitch` of crop height, each a team-coloured bar across the name
    /// strip with a white name block, and the stat values in neutral grey
    /// under the six header labels (team 2 starts at 0.565 of the crop, as
    /// on the real board). `header` false leaves the Tab header strip out,
    /// as on the old Overwatch 1 scoreboard. No names, portraits or
    /// captured pixels; the digits are the embedded 1440p templates.
    fn tab_stand_in(
        frame_w: u32,
        frame_h: u32,
        team: usize,
        pitch: f64,
        colours: [[u8; 3]; 2],
        header: bool,
    ) -> DynamicImage {
        let (cx, cy, cw, ch) = crop_rect(frame_w, frame_h);
        let mut img = image::RgbImage::from_pixel(frame_w, frame_h, image::Rgb([20, 22, 30]));
        for y in 0..ch {
            for x in 0..cw {
                img.put_pixel(cx + x, cy + y, image::Rgb([18, 20, 32]));
            }
        }
        if header {
            paint_header(&mut img, frame_w, frame_h, 0);
        }
        let (chf, cwf) = (ch as f64, cw as f64);
        let p = pitch * chf;
        let gap = at_scale(3.0, ch).round().max(1.0) as u32;
        let mut board = image::RgbImage::new(cw, ch);
        for r in 0..2 * team {
            let base = if r < team { 0.025 } else { 0.565 } * chf;
            let top = base + (r % team) as f64 * p;
            let ctr = top + p / 2.0;
            let colour = image::Rgb(colours[r / team]);
            for y in top as u32..(top + p) as u32 - gap {
                for x in (0.04 * cwf) as u32..(0.56 * cwf) as u32 {
                    img.put_pixel(cx + x, cy + y, colour);
                }
            }
            for y in (ctr - 0.15 * p) as u32..(ctr + 0.15 * p) as u32 {
                for x in (0.10 * cwf) as u32..(0.30 * cwf) as u32 {
                    img.put_pixel(cx + x, cy + y, image::Rgb([235, 235, 235]));
                }
            }
            for (k, &v) in STAND_IN_STATS[r].iter().enumerate() {
                let col = at_scale(960.0 + k as f64 * 100.0, ch) as usize;
                digits::test_glyphs::draw_stat(&mut board, v, k, col, ctr as usize - 10);
            }
        }
        for (x, y, px) in board.enumerate_pixels() {
            if px.0[0] > 0 {
                img.put_pixel(cx + x, cy + y, *px);
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    /// Real row pitches (fraction of crop height) of 5v5 and 6v6 Tab boards.
    const PITCH_5V5: f64 = 0.0873;
    const PITCH_6V6: f64 = 0.0754;

    fn assert_stand_in_read(b: &BoardRead, team: usize, what: &str) {
        assert_eq!(b.status, BoardStatus::Read, "{what}");
        assert_eq!(b.team_size, Some(team), "{what}");
        for (r, row) in STAND_IN_STATS.iter().take(2 * team).enumerate() {
            for (k, &v) in row.iter().enumerate() {
                let name = format!("r{r}.{}", STAT_FIELDS[k]);
                assert_eq!(
                    b.get(&name).and_then(|f| f.value.clone()),
                    Some(Value::Int(v)),
                    "{what} {name}"
                );
            }
        }
    }

    #[test]
    fn tab_stand_ins_read_with_their_team_size() {
        let reader = Reader::load(&ReaderConfig::default());
        for (team, pitch) in [(5, PITCH_5V5), (6, PITCH_6V6)] {
            let frame = tab_stand_in(2560, 1440, team, pitch, BLUE_RED, true);
            let crop = crate::ocr::preprocess::crop_scoreboard(&frame);
            let scan = crate::detect::hero_portrait::scan_rows(&crop);
            let got = scan.median_pitch.unwrap();
            assert!((got - pitch).abs() < 0.004, "{team}v{team}: {scan:?}");
            let b = reader.read_board(&frame);
            assert_stand_in_read(&b, team, &format!("{team}v{team}"));
        }
    }

    /// Team colours never decide anything: rows, teams and the size come
    /// from the layout (row pitch, team 2 at 0.565 of the crop) and from
    /// brightness. Swapped, purple/yellow and yellow/purple teams read the
    /// same as blue/red.
    #[test]
    fn recoloured_tab_stand_ins_read_the_same() {
        let reader = Reader::load(&ReaderConfig::default());
        let purple = [140, 60, 200];
        let yellow = [230, 200, 40];
        for (team, pitch) in [(5, PITCH_5V5), (6, PITCH_6V6)] {
            let mut base =
                reader.read_board(&tab_stand_in(2560, 1440, team, pitch, BLUE_RED, true));
            base.elapsed_ms = 0;
            for (what, colours) in [
                ("red/blue", [BLUE_RED[1], BLUE_RED[0]]),
                ("purple/yellow", [purple, yellow]),
                ("yellow/purple", [yellow, purple]),
            ] {
                let what = format!("{team}v{team} {what}");
                let mut b =
                    reader.read_board(&tab_stand_in(2560, 1440, team, pitch, colours, true));
                assert_stand_in_read(&b, team, &what);
                b.elapsed_ms = 0;
                assert_eq!(b, base, "{what}");
            }
        }
    }

    /// The two old Overwatch 1 scoreboards from X (#206): rows and stat
    /// digits, but no stat labels in the Tab header strip. The pitch rule
    /// alone gives 6 (1920x1080) and 5 (2560x1437).
    #[test]
    fn ow1_scoreboard_stand_ins_are_not_found() {
        let reader = Reader::load(&ReaderConfig::default());
        for (what, w, h, team, dip, spectral) in [
            ("OW1 1080", 1920, 1080, 6, 0.0754, 0.0767),
            ("OW1 2560x1437", 2560, 1437, 5, 0.0826, 0.0806),
        ] {
            assert_eq!(
                layout_from_scan(&scan(4, Some(dip), Some(spectral)), 0),
                Ok(team),
                "{what}"
            );
            let frame = tab_stand_in(w, h, team, dip, BLUE_RED, false);
            let crop = crate::ocr::preprocess::crop_scoreboard(&frame);
            let scan = crate::detect::hero_portrait::scan_rows(&crop);
            let labels = crate::ocr::preprocess::header_label_groups(&crop).len();
            assert_eq!(
                layout_from_scan(&scan, labels),
                Ok(team),
                "{what}: {scan:?}"
            );
            assert!(!digits::stat_columns_found(&crop), "{what}");
            let b = reader.read_board(&frame);
            assert_eq!(b.status, BoardStatus::NotFound, "{what}");
            assert_eq!(b.team_size, None, "{what}");
        }
    }

    fn scan(dips: usize, dip: Option<f64>, spectral: Option<f64>) -> RowScan {
        RowScan {
            dip_count: dips,
            median_pitch: dip,
            spectral_pitch: spectral,
        }
    }

    #[test]
    fn disagreeing_or_implausible_pitches_are_team_size_unknown() {
        let unknown = Err(BoardStatus::TeamSizeUnknown);
        // the real 1080p 6v6 case: dip 0.0794 says 5, spectral 0.0754 says 6
        assert_eq!(
            layout_from_scan(&scan(5, Some(0.0794), Some(0.0754)), 6),
            unknown
        );
        // both plausible, opposite sides of the 0.079 split
        assert_eq!(
            layout_from_scan(&scan(5, Some(0.083), Some(0.074)), 6),
            unknown
        );
        // measured, but neither plausible (0.101 / 0.102 traps)
        assert_eq!(
            layout_from_scan(&scan(4, Some(0.101), Some(0.102)), 6),
            unknown
        );
        // header labels only, no pitch at all: no 5v5 default
        assert_eq!(layout_from_scan(&scan(0, None, None), 6), unknown);
        // no preflight at all
        assert_eq!(
            layout_from_scan(&scan(1, None, None), 0),
            Err(BoardStatus::NotFound)
        );
        // agreeing or single plausible pitches still read
        assert_eq!(
            layout_from_scan(&scan(5, Some(0.083), Some(0.084)), 6),
            Ok(5)
        );
        assert_eq!(
            layout_from_scan(&scan(5, Some(0.0745), Some(0.102)), 6),
            Ok(6)
        );
        assert_eq!(layout_from_scan(&scan(5, Some(0.074), None), 0), Ok(6));
    }

    #[test]
    fn not_found_keeps_a_result_slot() {
        let mut b = not_found(BoardStatus::TeamSizeUnknown);
        b.set_result(FieldRead {
            name: "result".into(),
            value: Some(Value::Text("victory".into())),
            confidence: 0.9,
            suspect: false,
        });
        assert_eq!(
            b.get("result").unwrap().value,
            Some(Value::Text("victory".into()))
        );
        assert_eq!(b.team_size, None);
    }

    /// The two banner maps ocr-v1's table folds onto another map.
    const OCR_V1_COLLISIONS: [&str; 2] = ["Temple of Anubis", "Ecopoint: Antarctica"];

    #[test]
    fn map_names_match_ocr_v1_for_every_template() {
        use crate::parse::{canonical_map, map_mode};
        use crate::shadow::banner::MAPS;
        let mut known = 0;
        for info in MAPS {
            let n = map_name(info);
            match canonical_map(info.name) {
                Some(v1) if !OCR_V1_COLLISIONS.contains(&info.name) => {
                    assert_eq!(n.name, v1, "{} differs from ocr-v1", info.key);
                    assert!(n.known_to_ocr_v1, "{}", info.key);
                    assert_eq!(n.mode, map_mode(&v1).or(info.mode), "{}", info.key);
                    // ocr-v1 is idempotent on what we emit: storing it and
                    // canonicalising again gives the same string
                    assert_eq!(canonical_map(&n.name).as_deref(), Some(v1.as_str()));
                    known += 1;
                }
                Some(v1) => {
                    assert_ne!(v1, info.name);
                    assert_eq!(n.name, info.name, "keeps its own map");
                    assert!(!n.known_to_ocr_v1, "{} must stay suspect", info.key);
                }
                None => {
                    assert_eq!(n.name, info.name, "{}", info.key);
                    assert!(!n.known_to_ocr_v1, "{} unknown to ocr-v1", info.key);
                }
            }
        }
        // every map in ocr-v1's table is in the banner pack
        assert_eq!(known, 34);
        for c in OCR_V1_COLLISIONS {
            assert!(MAPS.iter().any(|m| m.name == c), "{c}");
        }
    }

    #[test]
    fn accented_map_names_match_ocr_v1_with_and_without_accents() {
        use crate::parse::canonical_map;
        use crate::shadow::banner::MAPS;
        let by_key = |k: &str| MAPS.iter().find(|m| m.key == k).unwrap();
        let cases = [
            (
                "Watchpoint: Grímsvötn",
                &[
                    "Watchpoint: Grímsvötn",
                    "Watchpoint: Grimsvotn",
                    "WATCHPOINT: GRÍMSVÖTN",
                    "GRIMSVOTN",
                    "Grímsvötn",
                    "grimsvötn",
                ][..],
            ),
            (
                "Esperanca",
                &[
                    "Esperança",
                    "Esperanca",
                    "ESPERANÇA",
                    "ESPERANCA",
                    "esperança",
                ][..],
            ),
        ];
        for (want, spellings) in cases {
            let info = MAPS.iter().find(|m| m.name == want).unwrap();
            assert_eq!(map_name(info).name, want);
            assert!(map_name(info).known_to_ocr_v1);
            for s in spellings {
                assert_eq!(canonical_map(s).as_deref(), Some(want), "{s}");
            }
        }
        // by banner key too, so a renamed template entry cannot slip through
        assert_eq!(
            map_name(by_key("watchpoint-grimsvotn")).name,
            "Watchpoint: Grímsvötn"
        );
        assert_eq!(map_name(by_key("esperanca")).name, "Esperanca");
    }

    #[test]
    fn values_serialize_as_plain_json() {
        let f = FieldRead {
            name: "r0.dmg".into(),
            value: Some(Value::Int(2674)),
            confidence: 0.5,
            suspect: false,
        };
        assert_eq!(
            serde_json::to_string(&f).unwrap(),
            r#"{"name":"r0.dmg","value":2674,"confidence":0.5,"suspect":false}"#
        );
    }
}
