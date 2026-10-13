//! Bench: ocr-v1 (Tesseract) next to the new reader (cv-v5 digitiser), per sample.
//!
//! Env:
//! * `SCUFFED_READER_BENCH_SAMPLES`: TSV with a header line and the columns
//!   `id  frame  aspect  region  resolution  palette  game`. `id` is a number,
//!   `aspect` is one of `cells own_row own_hero map outcome team_size`,
//!   `region` is `r{N}.{e,a,d,dmg,h,mit}` for cells and `-` otherwise.
//! * `SCUFFED_READER_BENCH_OUT`: JSONL output, one line per sample.
//! * `SCUFFED_READER_PACK` (optional): a maps pack (a dir holding `atlas.json`,
//!   or a data dir holding `templates/maps`). Without it the new map read is
//!   `not runnable: pack absent`.
//! * `SCUFFED_READER_BENCH_DATA` (optional): data dir for portraits and hero
//!   templates. Default: the config's `data_dir`. The portrait loader may move
//!   stale seeds aside, so point this at a copy.
//! * `SCUFFED_READER_BENCH_PLAN=1`: plan mode. The samples file is a plain list
//!   of frame paths; prints `index  w  h  input  preflight  team_size` with no
//!   OCR (used to build the sample list).
//!
//! Each frame runs once per reader, the way the daemon does it:
//! * ocr-v1 on a Tab frame is `analyze_frame` (preflight, portrait, highlight
//!   row, cell OCR, name match, career hero, map label, lazy full OCR), then
//!   the scoreboard trust gate, `parse::read_scoreboard` and the hero
//!   authority for a fresh game. On a result frame it is the poller's
//!   `detect_outcome_signal`.
//! * the new reader is `Reader::read_board` (digits, hero icons, map banner
//!   when a pack exists) and `Reader::read_result`. Its own row is the
//!   highlight (`resolve_player_row`), no name OCR.
//!
//! A frame narrower than 1.72:1 is a saved board crop (`debug/accepted`):
//! both readers run their board part on it as is (no map, no career panel).
//!
//! The output record holds numbers, enums and `&'static str` keys from fixed
//! tables only. No OCR text, no player name, no path reaches the file.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use image::DynamicImage;
use serde::Serialize;
use stat_tracker::detect::MatchOutcome;
use stat_tracker::detect::hero_portrait::{self, ScoreboardPreflight};
use stat_tracker::detect::match_end::{self, OutcomeSource};
use stat_tracker::hero_auth::{self, HeroAuthState, HeroSource};
use stat_tracker::shadow::banner::{MAPS, MapTemplates};
use stat_tracker::shadow::heroes::HeroTemplates;
use stat_tracker::shadow::reader::{READ_BUDGET, STAT_FIELDS, assemble, board_layout};
use stat_tracker::shadow::result::{Outcome, ResultTemplates};
use stat_tracker::shadow::{BoardRead, BoardStatus, Reader, ReaderConfig, Value, digits};
use stat_tracker::{ocr, parse};

// ------------------------------------------------------------------ record

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Aspect {
    Cells,
    OwnRow,
    OwnHero,
    Map,
    Outcome,
    TeamSize,
}

impl Aspect {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "cells" => Self::Cells,
            "own_row" => Self::OwnRow,
            "own_hero" => Self::OwnHero,
            "map" => Self::Map,
            "outcome" => Self::Outcome,
            "team_size" => Self::TeamSize,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum OutcomeV {
    Victory,
    Defeat,
    Draw,
}

/// A read value. Strings are `&'static str` keys from fixed tables (hero pack
/// stems, banner map keys) or literals in this file, never OCR text.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Val {
    /// Nothing read.
    None,
    Num(u32),
    Row(u8),
    Hero(&'static str),
    Map(&'static str),
    Outcome(OutcomeV),
    TeamSize(u8),
    /// The reader cannot run on this sample (reason is a literal).
    NotRunnable(&'static str),
}

/// Which stat cell, for `cells` samples.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
struct CellRef {
    row: u8,
    field: &'static str,
}

/// One JSONL line. Only numbers, bools, enums and `&'static str`.
#[derive(Debug, Clone, Serialize)]
struct Record {
    id: u64,
    aspect: Aspect,
    cell: Option<CellRef>,
    input: &'static str,
    ocr_v1: Val,
    /// ocr-v1 pipeline state for this frame (literal).
    ocr_v1_status: &'static str,
    /// Where ocr-v1's value came from (literal), when it says.
    ocr_v1_source: Option<&'static str>,
    /// Tesseract mean confidence (cells only).
    ocr_v1_conf: Option<i32>,
    /// Edge-ink flag (cells only).
    ocr_v1_suspect: Option<bool>,
    /// The cell text passes ocr-v1's clean-stat rule (cells only).
    ocr_v1_clean: Option<bool>,
    new: Val,
    new_status: &'static str,
    new_suspect: Option<bool>,
    new_conf: Option<f32>,
    /// Banner map margin, result margin (map / outcome only).
    new_margin: Option<f32>,
    /// Source layout of a result read (literal).
    new_source: Option<&'static str>,
    ms_ocr_v1: f64,
    ms_new: f64,
    /// Peak RSS while that reader ran on this frame (VmHWM reset per reader).
    rss_peak_kb_ocr_v1: u64,
    rss_peak_kb_new: u64,
    /// Highest peak RSS seen so far in this run.
    rss_peak_kb_run: u64,
    /// Threads that can hold a loaded Tesseract model: OCR pool threads
    /// alive plus the calling thread once it ran a region read.
    tess_threads: u32,
    /// All OS threads of the process after the frame (OpenMP included).
    os_threads: u32,
    /// OpenMP worker threads (Tesseract) alive after the frame.
    omp_threads: u32,
    error: Option<&'static str>,
}

impl Record {
    fn blank(id: u64, aspect: Aspect, input: &'static str) -> Self {
        Self {
            id,
            aspect,
            cell: None,
            input,
            ocr_v1: Val::None,
            ocr_v1_status: "-",
            ocr_v1_source: None,
            ocr_v1_conf: None,
            ocr_v1_suspect: None,
            ocr_v1_clean: None,
            new: Val::None,
            new_status: "-",
            new_suspect: None,
            new_conf: None,
            new_margin: None,
            new_source: None,
            ms_ocr_v1: 0.0,
            ms_new: 0.0,
            rss_peak_kb_ocr_v1: 0,
            rss_peak_kb_new: 0,
            rss_peak_kb_run: 0,
            tess_threads: 0,
            os_threads: 0,
            omp_threads: 0,
            error: None,
        }
    }
}

// ------------------------------------------------------------- key tables

/// Hero pack stem (`wrecking-ball`) for any hero identifier, else `None`.
fn hero_key(raw: &str) -> Option<&'static str> {
    let display = scuffed_types::hero_key_to_name(raw)
        .or_else(|| scuffed_types::hero_key_to_name(&parse::canonical_hero(raw)))?;
    scuffed_types::HERO_NAMES
        .iter()
        .find(|h| h.display == display)
        .map(|h| scuffed_types::pack_file_stem(h.file))
}

/// A hero outside the table (ocr-v1's "Unknown", empty) is no read.
fn hero_val(raw: &str) -> Val {
    hero_key(raw).map_or(Val::None, Val::Hero)
}

/// Banner map key for a stored map name, else `unlisted`.
fn map_val(name: &str) -> Val {
    MAPS.iter()
        .find(|m| m.name == name)
        .or_else(|| {
            let c = parse::canonical_map(name)?;
            MAPS.iter().find(|m| m.name == c)
        })
        .map_or(Val::Map("unlisted"), |m| Val::Map(m.key))
}

fn outcome_v(o: MatchOutcome) -> Val {
    match o {
        MatchOutcome::Victory => Val::Outcome(OutcomeV::Victory),
        MatchOutcome::Defeat => Val::Outcome(OutcomeV::Defeat),
        MatchOutcome::Draw => Val::Outcome(OutcomeV::Draw),
        MatchOutcome::Unknown => Val::None,
    }
}

fn new_outcome_v(o: Option<Outcome>) -> Val {
    match o {
        Some(Outcome::Victory) => Val::Outcome(OutcomeV::Victory),
        Some(Outcome::Defeat) => Val::Outcome(OutcomeV::Defeat),
        Some(Outcome::Draw) => Val::Outcome(OutcomeV::Draw),
        None => Val::None,
    }
}

fn result_source(s: Option<&str>) -> Option<&'static str> {
    use stat_tracker::shadow::result::{ACCOLADE, END_TITLE, RANK_SCREEN, TAB_HEADER};
    s.map(|s| {
        [ACCOLADE, RANK_SCREEN, END_TITLE, TAB_HEADER]
            .into_iter()
            .find(|k| *k == s)
            .unwrap_or("plugged_in")
    })
}

fn board_status(s: BoardStatus) -> &'static str {
    match s {
        BoardStatus::Read => "read",
        BoardStatus::NotFound => "not_found",
        BoardStatus::TeamSizeUnknown => "team_size_unknown",
    }
}

fn static_field(k: usize) -> &'static str {
    STAT_FIELDS[k]
}

/// ocr-v1's cell number (`parse::parse_cell_number`: digits only).
fn cell_number(s: &str) -> Option<u32> {
    let d: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if d.is_empty() { None } else { d.parse().ok() }
}

/// ocr-v1's clean-stat rule (`ocr::is_clean_stat`).
fn clean_stat(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit()) && s.chars().all(|c| c.is_ascii_digit() || c == ',')
}

// --------------------------------------------------------------- process

fn status_kb(key: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(key))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

/// Reset VmHWM to the current RSS (Linux `clear_refs` 5).
fn reset_peak() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

fn peak_kb() -> u64 {
    status_kb("VmHWM:")
}

/// (tess, all, omp). Pool threads are named `ocr-{i}`. Threads a thread
/// spawns inherit its name, so OpenMP workers show up as repeats of the name
/// of the thread that started them: those repeats are the `omp` count.
fn thread_counts(caller_ran_tess: bool) -> (u32, u32, u32) {
    let mut names: BTreeMap<String, u32> = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir("/proc/self/task") {
        for e in rd.flatten() {
            let n = std::fs::read_to_string(e.path().join("comm")).unwrap_or_default();
            *names.entry(n.trim().to_string()).or_default() += 1;
        }
    }
    let pool = names.keys().filter(|n| n.starts_with("ocr-")).count() as u32;
    let all = names.values().sum();
    let omp = names.values().map(|c| c - 1).sum();
    (pool + caller_ran_tess as u32, all, omp)
}

// ---------------------------------------------------------- ocr-v1 path

/// What ocr-v1 produced on one frame. No text leaves this struct except
/// through the key tables above.
/// One ocr-v1 cell: number, Tesseract confidence, edge-ink flag, clean-stat.
type CellV1 = (Option<u32>, i32, bool, bool);

#[derive(Default)]
struct OcrV1 {
    status: &'static str,
    team_size: Option<usize>,
    cells: Vec<Vec<CellV1>>,
    own_row: Option<usize>,
    own_row_source: Option<&'static str>,
    hero: Option<Val>,
    hero_source: Option<&'static str>,
    map: Option<Val>,
    outcome: Option<(Val, &'static str)>,
    ms: f64,
    caller_ran_tess: bool,
}

/// `analyze_frame` + trust gate + `read_scoreboard` + fresh-game hero
/// authority. `crop` skips the full-frame parts (career, map, full OCR).
fn run_ocr_v1(
    img: &DynamicImage,
    crop: bool,
    matcher: &hero_portrait::PortraitMatcher,
    player_name: Option<&str>,
) -> OcrV1 {
    let t0 = Instant::now();
    let mut out = OcrV1 {
        status: "analyzed",
        ..Default::default()
    };
    let scoreboard = if crop {
        img.clone()
    } else {
        ocr::preprocess::crop_scoreboard(img)
    };
    let (preflight, _) = hero_portrait::preflight_scoreboard(&scoreboard);
    let team_size = match preflight {
        ScoreboardPreflight::NotAScoreboard => {
            out.status = "not_a_scoreboard";
            out.ms = ms(t0);
            return out;
        }
        ScoreboardPreflight::TeamSizeUncertain => {
            out.status = "team_size_uncertain";
            out.ms = ms(t0);
            return out;
        }
        ScoreboardPreflight::Ready(n) => n,
    };
    out.team_size = Some(team_size);
    let player_match = matcher.match_player_hero_with_team_size(&scoreboard, team_size);
    let portrait = player_match.as_ref().map(|(n, c, _)| (n.clone(), *c));
    let brightness_row =
        hero_portrait::resolve_player_row(player_match.map(|(_, _, i)| i), &scoreboard, team_size);
    let rows = ocr::recognize_scoreboard_cells_pre_cropped(&scoreboard, team_size);
    let name_match = player_name.and_then(|n| parse::match_player_row_by_name(&rows, n));
    let row_idx = name_match.as_ref().map(|m| m.row).or(brightness_row);
    out.own_row = row_idx;
    out.own_row_source = match (&name_match, brightness_row) {
        (Some(_), _) => Some("name"),
        (None, Some(_)) => Some("highlight"),
        _ => None,
    };
    out.cells = rows
        .iter()
        .map(|r| {
            r.stats
                .iter()
                .map(|c| {
                    let v = c.value.trim();
                    (cell_number(v), c.confidence, c.suspect, clean_stat(v))
                })
                .collect()
        })
        .collect();

    let (career_hero, map_from_panel) = if crop {
        (None, None)
    } else {
        out.caller_ran_tess = true;
        let career = ocr::recognize_region(&ocr::preprocess::crop_career_hero(img))
            .ok()
            .and_then(|t| parse::match_hero_in_text(&t));
        let map_text = ocr::recognize_map_label(img, preflight);
        let map = if map_text.trim().is_empty() {
            None
        } else {
            parse::match_map_in_text(&map_text)
        };
        (career, map)
    };
    out.map = Some(map_from_panel.as_deref().map_or(Val::None, map_val));

    let cells_parse = parse::parse_scoreboard_cells(&rows, row_idx, "", "unknown", None).is_some();
    let hero_identified = career_hero.is_some() || portrait.is_some();
    // First capture of a game: the session map is not known yet.
    let need_full = !crop && (!cells_parse || !hero_identified || map_from_panel.is_none());
    let raw_text = if need_full {
        ocr::recognize(img).map(|r| r.raw_text).unwrap_or_default()
    } else {
        String::new()
    };

    if !parse::looks_like_scoreboard(&rows) {
        out.status = "rejected_noscoreboard";
        out.ms = ms(t0);
        return out;
    }
    match parse::read_scoreboard(&rows, row_idx, &raw_text, "unknown", player_name) {
        Ok(read) => {
            let p = read.matched;
            let (hero, source, _) = hero_auth::resolve_hero(
                career_hero.as_deref(),
                portrait.as_ref().map(|(n, c)| (n.as_str(), *c)),
                &p.hero,
                &HeroAuthState::default(),
                parse::canonical_hero,
            );
            out.hero = Some(hero_val(&hero));
            out.hero_source = Some(match source {
                HeroSource::CareerPanel => "career_panel",
                HeroSource::Portrait => "portrait",
                HeroSource::Held => "held",
                HeroSource::OcrText => "ocr_text",
            });
            if !read.trusted_cells {
                out.status = "raw_text_fallback";
            }
        }
        Err(_) => out.status = "parse_miss",
    }
    out.ms = ms(t0);
    out
}

fn run_ocr_v1_outcome(img: &DynamicImage) -> OcrV1 {
    let t0 = Instant::now();
    let sig = match_end::detect_outcome_signal(img);
    OcrV1 {
        status: "analyzed",
        outcome: Some(match sig {
            Some((o, src)) => (
                outcome_v(o),
                match src {
                    OutcomeSource::Banner => "banner",
                    OutcomeSource::ResultWord => "result_word",
                    OutcomeSource::RankScreen => "rank_screen",
                    OutcomeSource::EndTitle => "end_title",
                },
            ),
            None => (Val::None, "none"),
        }),
        ms: ms(t0),
        caller_ran_tess: true,
        ..Default::default()
    }
}

fn ms(t0: Instant) -> f64 {
    (t0.elapsed().as_secs_f64() * 1e4).round() / 10.0
}

// ------------------------------------------------------------- new path

struct NewRead {
    board: BoardRead,
    own_row: Option<usize>,
    ms: f64,
}

/// `Reader::read_board` on a full frame. On a board crop, its board part
/// (layout, digits, hero icons, assemble) without the frame crop and map.
fn run_new(
    img: &DynamicImage,
    crop: bool,
    reader: &Reader,
    heroes: Option<&HeroTemplates>,
    matcher: &hero_portrait::PortraitMatcher,
) -> NewRead {
    let t0 = Instant::now();
    let board = if crop {
        match board_layout(img) {
            Err(s) => stat_tracker::shadow::reader::not_found(s),
            Ok(n) => {
                let d = digits::read_board(img, n, READ_BUDGET).ok();
                let rows_ok = d.as_ref().is_some_and(|d| {
                    d.rows
                        .iter()
                        .filter(|r| r.cells.iter().filter(|c| c.value.is_some()).count() >= 4)
                        .count()
                        >= 3
                });
                if rows_ok {
                    let h = heroes.and_then(|h| h.read_board(img, n, READ_BUDGET).ok());
                    assemble(n, d.as_ref(), h.as_ref().map(|h| &h.rows[..]))
                } else {
                    stat_tracker::shadow::reader::not_found(BoardStatus::NotFound)
                }
            }
        }
    } else {
        reader.read_board(img)
    };
    // Own row: the highlight, with the reader's own team size. No name OCR.
    let own_row = board.team_size.and_then(|n| {
        let sb = if crop {
            img.clone()
        } else {
            ocr::preprocess::crop_scoreboard(img)
        };
        let pm = matcher.match_player_hero_with_team_size(&sb, n);
        hero_portrait::resolve_player_row(pm.map(|(_, _, i)| i), &sb, n)
    });
    NewRead {
        board,
        own_row,
        ms: ms(t0),
    }
}

fn field_val(board: &BoardRead, name: &str) -> (Val, Option<bool>, Option<f32>) {
    match board.get(name) {
        None => (Val::None, None, None),
        Some(f) => {
            let v = match &f.value {
                None => Val::None,
                Some(Value::Int(n)) => Val::Num(*n),
                Some(Value::Text(t)) if name.ends_with(".hero") => hero_val(t),
                Some(Value::Text(t)) if name == "map" => map_val(t),
                Some(Value::Text(_)) => Val::NotRunnable("unexpected text field"),
            };
            (v, Some(f.suspect), Some(f.confidence))
        }
    }
}

// ----------------------------------------------------------------- input

struct Sample {
    id: u64,
    frame: PathBuf,
    aspect: Aspect,
    cell: Option<(usize, usize)>,
}

fn parse_region(s: &str) -> Option<(usize, usize)> {
    let rest = s.strip_prefix('r')?;
    let (r, f) = rest.split_once('.')?;
    Some((r.parse().ok()?, STAT_FIELDS.iter().position(|x| *x == f)?))
}

fn read_samples(path: &Path) -> Vec<Sample> {
    let text = std::fs::read_to_string(path).expect("read samples");
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 4 {
            continue;
        }
        let (Ok(id), Some(aspect)) = (cols[0].parse(), Aspect::parse(cols[2])) else {
            eprintln!("skip malformed sample line");
            continue;
        };
        out.push(Sample {
            id,
            frame: PathBuf::from(cols[1]),
            aspect,
            cell: parse_region(cols[3]),
        });
    }
    out
}

fn is_crop(img: &DynamicImage) -> bool {
    (img.width() as f64 / img.height().max(1) as f64) < 1.72
}

fn maps_pack() -> Option<(MapTemplates, PathBuf)> {
    let dir = PathBuf::from(std::env::var_os("SCUFFED_READER_PACK")?);
    let direct = MapTemplates::load_dir(&dir).map(|m| (m, dir.clone()));
    direct.or_else(|| {
        let d = MapTemplates::dir_in(&dir);
        MapTemplates::load_dir(&d).map(|m| (m, d))
    })
}

// ------------------------------------------------------------------ main

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("error")),
        )
        .with_writer(std::io::stderr)
        .init();

    let config = stat_tracker::config::Config::read_stored().expect("config");
    let data_dir = std::env::var_os("SCUFFED_READER_BENCH_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| config.data_dir.clone());
    ocr::set_ocr_threads(config.ocr_threads_resolved());
    let player_name = config.player_name.clone().filter(|n| !n.trim().is_empty());
    let samples_path = PathBuf::from(
        std::env::var_os("SCUFFED_READER_BENCH_SAMPLES").expect("SCUFFED_READER_BENCH_SAMPLES"),
    );

    if std::env::var_os("SCUFFED_READER_BENCH_PLAN").is_some() {
        plan(&samples_path);
        return;
    }

    let out_path = PathBuf::from(
        std::env::var_os("SCUFFED_READER_BENCH_OUT").expect("SCUFFED_READER_BENCH_OUT"),
    );
    let matcher = Arc::new(hero_portrait::PortraitMatcher::load(
        &hero_portrait::portraits_dir(&data_dir),
    ));
    let pack = maps_pack();
    let mut rc = ReaderConfig::from_data_dir(&data_dir);
    rc.map_dir = pack.as_ref().map(|(_, d)| d.clone());
    let reader = Reader::load(&rc);
    let heroes = HeroTemplates::load_dir(&HeroTemplates::dir_in(&data_dir));
    let results_present = ResultTemplates::load_dir(&ResultTemplates::dir_in(&data_dir)).is_some();
    eprintln!(
        "ocr_threads={} heroes_pack={} maps_pack={} result_pack={} player_name_set={}",
        config.ocr_threads_resolved(),
        heroes.is_some(),
        pack.is_some(),
        results_present,
        player_name.is_some()
    );

    let samples = read_samples(&samples_path);
    let mut by_frame: BTreeMap<PathBuf, Vec<&Sample>> = BTreeMap::new();
    for s in &samples {
        by_frame.entry(s.frame.clone()).or_default().push(s);
    }
    let mut out = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&out_path)
            .expect("open out"),
    );
    let mut run_peak = 0u64;
    let total = by_frame.len();
    let t_run = Instant::now();
    for (i, (frame, group)) in by_frame.iter().enumerate() {
        let img = match image::open(frame) {
            Ok(i) => DynamicImage::ImageRgb8(i.to_rgb8()),
            Err(_) => {
                for s in group {
                    let mut r = Record::blank(s.id, s.aspect, "-");
                    r.error = Some("open_failed");
                    write_rec(&mut out, &r);
                }
                continue;
            }
        };
        let crop = is_crop(&img);
        let input = if crop { "board_crop" } else { "full_frame" };
        let only_outcome = group.iter().all(|s| s.aspect == Aspect::Outcome);
        let has_outcome = group.iter().any(|s| s.aspect == Aspect::Outcome);
        let has_board = group.iter().any(|s| s.aspect != Aspect::Outcome);

        // ---- ocr-v1
        reset_peak();
        let v1 = if only_outcome {
            run_ocr_v1_outcome(&img)
        } else {
            let mut b = run_ocr_v1(&img, crop, &matcher, player_name.as_deref());
            if has_outcome {
                let o = run_ocr_v1_outcome(&img);
                b.outcome = o.outcome;
                b.ms += o.ms;
            }
            b
        };
        let rss_v1 = peak_kb();
        // ---- new
        reset_peak();
        let t_new = Instant::now();
        let nb = if has_board {
            Some(run_new(&img, crop, &reader, heroes.as_ref(), &matcher))
        } else {
            None
        };
        let nres = has_outcome.then(|| reader.read_result(&img));
        let ms_new = ms(t_new);
        let rss_new = peak_kb();
        run_peak = run_peak.max(rss_v1).max(rss_new);
        let (tess, os, omp) = thread_counts(v1.caller_ran_tess);

        for s in group {
            let mut r = Record::blank(s.id, s.aspect, input);
            r.ms_ocr_v1 = v1.ms;
            r.ms_new = nb.as_ref().map_or(ms_new, |n| {
                if s.aspect == Aspect::Outcome {
                    ms_new
                } else {
                    n.ms
                }
            });
            r.rss_peak_kb_ocr_v1 = rss_v1;
            r.rss_peak_kb_new = rss_new;
            r.rss_peak_kb_run = run_peak;
            r.tess_threads = tess;
            r.os_threads = os;
            r.omp_threads = omp;
            r.ocr_v1_status = v1.status;
            if let Some(n) = &nb {
                r.new_status = board_status(n.board.status);
            }
            match s.aspect {
                Aspect::Cells => {
                    let Some((row, k)) = s.cell else {
                        r.error = Some("bad_region");
                        write_rec(&mut out, &r);
                        continue;
                    };
                    r.cell = Some(CellRef {
                        row: row as u8,
                        field: static_field(k),
                    });
                    if let Some(c) = v1.cells.get(row).and_then(|cs| cs.get(k)) {
                        r.ocr_v1 = c.0.map_or(Val::None, Val::Num);
                        r.ocr_v1_conf = Some(c.1);
                        r.ocr_v1_suspect = Some(c.2);
                        r.ocr_v1_clean = Some(c.3);
                    }
                    if let Some(n) = &nb {
                        let (v, sus, conf) =
                            field_val(&n.board, &format!("r{row}.{}", STAT_FIELDS[k]));
                        r.new = v;
                        r.new_suspect = sus;
                        r.new_conf = conf;
                    }
                }
                Aspect::OwnRow => {
                    r.ocr_v1 = v1.own_row.map_or(Val::None, |i| Val::Row(i as u8));
                    r.ocr_v1_source = v1.own_row_source;
                    if let Some(n) = &nb {
                        r.new = n.own_row.map_or(Val::None, |i| Val::Row(i as u8));
                        r.new_suspect = Some(n.own_row.is_none());
                    }
                }
                Aspect::OwnHero => {
                    r.ocr_v1 = v1.hero.unwrap_or(Val::None);
                    r.ocr_v1_source = v1.hero_source;
                    if let Some(n) = &nb
                        && let Some(row) = n.own_row
                    {
                        let (v, sus, conf) = field_val(&n.board, &format!("r{row}.hero"));
                        r.new = v;
                        r.new_suspect = sus;
                        r.new_conf = conf;
                    } else if nb.is_some() {
                        r.new_suspect = Some(true);
                    }
                }
                Aspect::Map => {
                    r.ocr_v1 = v1.map.unwrap_or(Val::None);
                    if crop {
                        r.new = Val::NotRunnable("not runnable: board crop has no banner");
                    } else if pack.is_none() {
                        r.new = Val::NotRunnable("not runnable: pack absent");
                        r.new_status = "not_runnable";
                    } else if let Some(n) = &nb {
                        let (v, sus, conf) = field_val(&n.board, "map");
                        r.new = v;
                        r.new_suspect = sus;
                        r.new_conf = conf;
                        // margin from a direct banner read (same pack)
                        if let Some((m, _)) = &pack {
                            r.new_margin = Some(m.read(&img.to_rgb8()).margin);
                        }
                    }
                }
                Aspect::Outcome => {
                    if let Some((v, src)) = v1.outcome {
                        r.ocr_v1 = v;
                        r.ocr_v1_source = Some(src);
                    }
                    r.ocr_v1_status = "analyzed";
                    if let Some(res) = &nres {
                        r.new = if results_present {
                            new_outcome_v(res.outcome)
                        } else {
                            Val::NotRunnable("not runnable: result pack absent")
                        };
                        r.new_status = if results_present {
                            "read"
                        } else {
                            "not_runnable"
                        };
                        r.new_suspect = Some(res.suspect);
                        r.new_conf = Some(res.score);
                        r.new_margin = Some(res.margin);
                        r.new_source = result_source(res.source.as_deref());
                    }
                }
                Aspect::TeamSize => {
                    r.ocr_v1 = v1.team_size.map_or(Val::None, |n| Val::TeamSize(n as u8));
                    if let Some(n) = &nb {
                        r.new = n
                            .board
                            .team_size
                            .map_or(Val::None, |n| Val::TeamSize(n as u8));
                        r.new_suspect = Some(n.board.team_size.is_none());
                    }
                }
            }
            write_rec(&mut out, &r);
        }
        let _ = out.flush();
        eprintln!(
            "[{}/{}] {} samples, v1 {:.0} ms, new {:.0} ms, {:.0}s elapsed",
            i + 1,
            total,
            group.len(),
            v1.ms,
            ms_new,
            t_run.elapsed().as_secs_f64()
        );
    }
    eprintln!(
        "done: {} samples, {} frames, {:.1}s, run peak {} kB",
        samples.len(),
        total,
        t_run.elapsed().as_secs_f64(),
        run_peak
    );
}

fn write_rec(out: &mut impl std::io::Write, r: &Record) {
    let line = serde_json::to_string(r).expect("record serializes");
    let _ = writeln!(out, "{line}");
}

/// Plan mode: per frame, size, input kind, preflight and team size. No OCR.
fn plan(list: &Path) {
    let text = std::fs::read_to_string(list).expect("read list");
    for (i, path) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let Ok(img) = image::open(path) else {
            println!("{i}\t0\t0\t-\topen_failed\t-");
            continue;
        };
        let img = DynamicImage::ImageRgb8(img.to_rgb8());
        let crop = is_crop(&img);
        let sb = if crop {
            img.clone()
        } else {
            ocr::preprocess::crop_scoreboard(&img)
        };
        let (pf, _) = hero_portrait::preflight_scoreboard(&sb);
        let (state, n) = match pf {
            ScoreboardPreflight::NotAScoreboard => ("not_a_scoreboard", 0),
            ScoreboardPreflight::TeamSizeUncertain => ("team_size_uncertain", 0),
            ScoreboardPreflight::Ready(n) => ("ready", n),
        };
        let layout = board_layout(&sb).map_or_else(board_status, |_| "read");
        // Palette hint: share of strongly magenta pixels (sampled grid).
        let rgb = img.to_rgb8();
        let (mut mag, mut tot) = (0u32, 0u32);
        for y in (0..rgb.height()).step_by(8) {
            for x in (0..rgb.width()).step_by(8) {
                let p = rgb.get_pixel(x, y).0;
                tot += 1;
                mag += (p[0] > 120
                    && p[2] > 120
                    && (p[1] as u32) * 10 < (p[0] as u32).min(p[2] as u32) * 6)
                    as u32;
            }
        }
        println!(
            "{i}\t{}\t{}\t{}\t{state}\t{n}\t{layout}\t{:.4}",
            img.width(),
            img.height(),
            if crop { "board_crop" } else { "full_frame" },
            mag as f64 / tot.max(1) as f64
        );
    }
}

// ----------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Every string the printer can emit: enum tags, field names, literals in
    /// this file, and keys from the fixed hero and map tables.
    fn allowed() -> std::collections::HashSet<String> {
        let mut a: std::collections::HashSet<String> = [
            // record fields and enum tags
            "id",
            "aspect",
            "cell",
            "input",
            "ocr_v1",
            "ocr_v1_status",
            "ocr_v1_source",
            "ocr_v1_conf",
            "ocr_v1_suspect",
            "ocr_v1_clean",
            "new",
            "new_status",
            "new_suspect",
            "new_conf",
            "new_margin",
            "new_source",
            "ms_ocr_v1",
            "ms_new",
            "rss_peak_kb_ocr_v1",
            "rss_peak_kb_new",
            "rss_peak_kb_run",
            "tess_threads",
            "os_threads",
            "omp_threads",
            "error",
            "row",
            "field",
            "none",
            "num",
            "hero",
            "map",
            "outcome",
            "team_size",
            "not_runnable",
            "cells",
            "own_row",
            "own_hero",
            "victory",
            "defeat",
            "draw",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        a.extend(STAT_FIELDS.iter().map(|s| s.to_string()));
        a.extend(
            scuffed_types::HERO_NAMES
                .iter()
                .map(|h| scuffed_types::pack_file_stem(h.file).to_string()),
        );
        a.extend(MAPS.iter().map(|m| m.key.to_string()));
        // literals used in this file
        let src = include_str!("reader_bench.rs");
        // Literals of the code above the tests only (the tests hold junk text).
        let mut rest = &src[..src.find("#[cfg(test)]").unwrap()];
        while let Some(i) = rest.find('"') {
            let after = &rest[i + 1..];
            let Some(j) = after.find('"') else { break };
            a.insert(after[..j].to_string());
            rest = &after[j + 1..];
        }
        a
    }

    fn strings(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::String(s) => out.push(s.clone()),
            serde_json::Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
            serde_json::Value::Object(o) => o.iter().for_each(|(k, x)| {
                out.push(k.clone());
                strings(x, out)
            }),
            _ => {}
        }
    }

    fn assert_clean(r: &Record) {
        let v = serde_json::to_value(r).unwrap();
        let mut s = Vec::new();
        strings(&v, &mut s);
        let allow = allowed();
        for x in s {
            assert!(
                allow.contains(&x),
                "printer emitted a string outside the fixed tables: {x:?}"
            );
        }
    }

    #[test]
    fn free_text_cannot_reach_the_printer() {
        // OCR-ish text and a player-like name go through every converter that
        // takes text; none of it may survive into the record.
        let junk = [
            "Zyxwv Player#1234",
            "K1NG'S R0W???",
            "  ",
            "ana but typo’d",
            "\u{0}",
        ];
        for j in junk {
            let mut r = Record::blank(1, Aspect::OwnHero, "full_frame");
            r.ocr_v1 = hero_val(j);
            r.new = map_val(j);
            r.new_source = result_source(Some(j));
            assert_clean(&r);
            assert!(!serde_json::to_string(&r).unwrap().contains(j.trim()) || j.trim().is_empty());
        }
    }

    #[test]
    fn every_value_kind_prints_only_allowed_strings() {
        let vals = [
            Val::None,
            Val::Num(12345),
            Val::Row(7),
            hero_val("Wrecking Ball"),
            hero_val("wrecking_ball"),
            map_val("King's Row"),
            Val::Outcome(OutcomeV::Victory),
            Val::TeamSize(5),
            Val::NotRunnable("not runnable: pack absent"),
        ];
        for (i, v) in vals.into_iter().enumerate() {
            let mut r = Record::blank(i as u64, Aspect::Cells, "board_crop");
            r.cell = Some(CellRef {
                row: 3,
                field: static_field(4),
            });
            r.ocr_v1 = v;
            r.new = v;
            r.new_source = result_source(Some("rank_screen"));
            r.error = Some("open_failed");
            assert_clean(&r);
        }
        assert_eq!(hero_val("Wrecking Ball"), Val::Hero("wrecking-ball"));
    }

    #[test]
    fn record_has_no_owned_string_fields() {
        // Compile-time guard: a String in Record or Val would make this
        // `Copy`-style check fail to build. Val is Copy, so it cannot own text.
        fn is_copy<T: Copy>() {}
        is_copy::<Val>();
        is_copy::<CellRef>();
        let src = include_str!("reader_bench.rs");
        let rec = &src[src.find("struct Record {").unwrap()..];
        let rec = &rec[..rec.find("\n}").unwrap()];
        assert!(!rec.contains("String"), "Record must not hold a String");
    }
}
