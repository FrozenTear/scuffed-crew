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
//! Every field is always present, in [`field_names`] order. A field the
//! reader could not read has no value, confidence 0 and `suspect: true`:
//!
//! * no hero icon pack (or no map / result pack): those fields are suspect,
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

use super::banner::MapTemplates;
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

/// Every field of one frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoardRead {
    pub team_size: usize,
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

/// All field names for a board of `team_size` (5 or 6), in output order.
pub fn field_names(team_size: usize) -> Vec<String> {
    let mut out = vec!["map".to_string(), "mode".into(), "result".into()];
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
    pub fn read_board(&self, frame: &DynamicImage) -> BoardRead {
        let t0 = Instant::now();
        let scoreboard = crate::ocr::preprocess::crop_scoreboard(frame);
        let team_size = crate::detect::hero_portrait::detect_team_size(&scoreboard);
        let team_size = if team_size == 6 { 6 } else { 5 };
        let digit_read = digits::read_board(&scoreboard, team_size, READ_BUDGET).ok();
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
                set(
                    &mut board,
                    "map",
                    Value::Text(info.name.into()),
                    m.score,
                    m.suspect,
                );
                if let Some(mode) = info.mode {
                    set(
                        &mut board,
                        "mode",
                        Value::Text(mode.into()),
                        m.score,
                        m.suspect,
                    );
                }
            }
        }
        board.elapsed_ms = t0.elapsed().as_millis().min(u32::MAX as u128) as u32;
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
        team_size,
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
