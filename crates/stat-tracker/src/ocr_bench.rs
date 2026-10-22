//! The OCR on/off bench: the daemon's two read paths over a list of frames.
//!
//! It is an ignored test of this binary, so it sees the private read path
//! (`analyze_frame`) and is never run by a plain `cargo test`. The frames
//! are captures of the game and are not in the repository: the lists come
//! from the environment, and the output holds values and hashes only (file
//! mode 0600), never a frame path or a name.
//!
//! For each frame it runs:
//! * the Tab path: `analyze_frame`, then the parse and the new-reader merge
//!   `handle_capture` does on its result;
//! * the poll path: the calls of the poll tick's blocking closure in
//!   `run_loop`, in the same order, with one `FrameStability`. A frame from
//!   a list gets two ticks (a screen that holds still); a sequence folder is
//!   one run of ticks.
//!
//! The last line of the output is a summary: the gate's mode and counters
//! (`LepTess` instances, Tesseract reads, blocked calls per site), whether
//! the OCR pool and the tessdata lookup were started, peak RSS and threads.
//!
//! ```text
//! OCR_BENCH_OUT=out.jsonl        where the JSON lines go
//! OCR_BENCH_DATA=<data dir>      reader packs (templates/…)
//! OCR_BENCH_OCR=on|off           off sets the gate, as the switch does
//! OCR_BENCH_READER=ocr-v1|new    off implies new
//! OCR_BENCH_TSVS=a.tsv:b.tsv     frame lists: a `frame` column of paths
//! OCR_BENCH_SEQS=dir:dir         folders of sequence folders of frames
//! OCR_BENCH_LIMIT=n              optional: at most n frames per list
//!
//! cargo test -p scuffed-stat-tracker --bin scuffed-stat-tracker -- \
//!     --ignored --exact ocr_bench::run --test-threads 1 --nocapture
//! ```
//!
//! Run it with private XDG dirs, so the tracker's own config and data are
//! not read. Two runs are compared record by record for the OCR-on identity
//! check; one run with `OCR_BENCH_OCR=off` gives the per-site table.

use super::*;
use sha2::{Digest, Sha256};
use std::io::Write as _;

/// A short hash in place of a path or a text.
fn hashed(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} (see the module docs)"))
}

/// A number from `/proc/self/status`, in the file's unit (kB, or a count).
fn proc_status(key: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with(key))
                .and_then(|line| line.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

/// utime + stime of this process, in clock ticks (100 per second).
fn cpu_ticks() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let after = stat.rsplit(')').next().unwrap_or("");
    let fields: Vec<&str> = after.split_whitespace().collect();
    let field = |i: usize| {
        fields
            .get(i)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };
    field(11) + field(12)
}

/// The Tab path for one frame.
fn tab(
    img: image::DynamicImage,
    matcher: &Arc<detect::hero_portrait::PortraitMatcher>,
    new: bool,
) -> serde_json::Value {
    use serde_json::json;
    let flags = FrameFlags {
        allow_banner_recovery: true,
        session_map_known: false,
        shadow_on: false,
        use_new_reader: new,
    };
    let a = match analyze_frame(
        img,
        Arc::clone(matcher),
        None,
        detect::MatchOutcome::Unknown,
        flags,
        // No own-name seeds: the bench has no data dir of a member.
        Vec::new(),
        &names::NameSwitch::off(),
    ) {
        FrameAnalysisOutcome::Analyzed(a) => *a,
        FrameAnalysisOutcome::NotAScoreboard { outcome, .. } => {
            return json!({"status": "not_a_scoreboard", "outcome": outcome.to_string()});
        }
        FrameAnalysisOutcome::TeamSizeUncertain { outcome, .. } => {
            return json!({"status": "team_size_uncertain", "outcome": outcome.to_string()});
        }
        FrameAnalysisOutcome::StreamMiss
        | FrameAnalysisOutcome::ShareOpening
        | FrameAnalysisOutcome::Declined
        | FrameAnalysisOutcome::CaptureBusy
        | FrameAnalysisOutcome::PortalPaused => {
            return json!({"status": "stream_miss"});
        }
    };
    let (raw, ocr_conf, ocr_err) = match &a.ocr {
        Ok(text) => (text.raw_text.clone(), text.confidence, None),
        Err(e) => (String::new(), -1, Some(e.to_string())),
    };
    let mut rec = json!({
        "status": "analyzed",
        "team_size": a.team_size,
        "own_row": a.player_row_idx,
        "rows_n": a.rows.len(),
        "rows_sha": hashed(&format!("{:?}", a.rows)),
        "frame_outcome": a.frame_outcome.to_string(),
        "career_hero": a.career_hero,
        "pack_hero": a.pack_hero,
        "portrait": a.portrait_hero.as_ref().map(|(name, score)| json!([name, score])),
        "map_from_panel": a.map_from_panel,
        "map_panel_raw_sha": a.map_panel_raw.as_deref().map(hashed),
        "map_from_banner": a.map_from_banner,
        "raw_sha": hashed(&raw),
        "raw_len": raw.len(),
        "ocr_conf": ocr_conf,
        "ocr_err": ocr_err,
        "looks_like_scoreboard": parse::looks_like_scoreboard(&a.rows),
    });
    let portrait = a
        .portrait_hero
        .as_ref()
        .map(|(name, score)| (name.as_str(), *score));
    let state = HeroAuthState::default();
    if new {
        // The new reader: the board `analyze_frame` read decides, as in
        // `handle_capture`. No OCR text is parsed.
        let Some(board) = a.board.as_ref() else {
            rec["read"] = json!("held:noboard");
            return rec;
        };
        rec["board_status"] = json!(format!("{:?}", board.status));
        if let Some(why) = reader_apply::board_refusal(board, a.team_size) {
            rec["read"] = json!(format!("held:refused:{why}"));
            return rec;
        }
        rec["both_teams"] = json!(reader_apply::both_teams_have_stats(board, a.team_size));
        let own = reader_apply::own_row(None, None, a.player_row_idx, a.team_size);
        let read = match reader_apply::read_own_row_stats(board, own, "unknown") {
            Ok(read) => read,
            Err(miss) => {
                rec["read"] = json!(format!("held:{miss:?}"));
                return rec;
            }
        };
        let m = &read.matched;
        let (hero, source, _) = hero_auth::resolve_hero_new_reader(
            a.pack_hero.as_deref(),
            portrait,
            &state,
            parse::canonical_hero,
        );
        rec["read"] = json!("read");
        rec["row"] = json!([
            m.elims,
            m.assists,
            m.deaths,
            m.damage,
            m.healing,
            m.mitigation
        ]);
        rec["held_cells"] = json!(read.held);
        rec["hero"] = json!(hero);
        rec["hero_source"] = json!(source.label());
        return rec;
    }
    if !parse::looks_like_scoreboard(&a.rows) {
        rec["read"] = json!("held:noscoreboard");
        return rec;
    }
    let read = match parse::read_scoreboard(&a.rows, a.player_row_idx, &raw, "unknown", None) {
        Ok(read) => read,
        Err(_) => {
            rec["read"] = json!("held:parse_miss");
            return rec;
        }
    };
    let m = &read.matched;
    let (hero, source, _) = hero_auth::resolve_hero(
        a.career_hero.as_deref(),
        portrait,
        &m.hero,
        &state,
        parse::canonical_hero,
    );
    rec["read"] = json!("parsed");
    rec["trusted_cells"] = json!(read.trusted_cells);
    rec["ocr_row"] = json!([
        m.elims,
        m.assists,
        m.deaths,
        m.damage,
        m.healing,
        m.mitigation
    ]);
    rec["ocr_map"] = json!(m.map_name);
    rec["hero"] = json!(hero);
    rec["hero_source"] = json!(source.label());
    rec
}

/// One poll tick: the calls of the blocking closure in `run_loop`
/// (`outcome_only` false), in its order.
fn poll(
    img: &image::DynamicImage,
    stability: &mut detect::stability::FrameStability,
    new: bool,
) -> serde_json::Value {
    use serde_json::json;
    let rgb = img.to_rgb8();
    let titles = detect::match_end::Titles::for_rgb(new, &rgb);
    let signal = detect::match_end::detect_outcome_signal_polled(
        img,
        &rgb,
        stability,
        detect::match_end::FinalScoreSource::for_reader(new),
        &titles,
        &detect::match_end::ResultWords::for_reader(new),
    );
    let accolade_map = choose_accolade_map(
        new,
        false,
        signal.as_ref().map(|(_, source)| *source),
        || shadow::read_accolade_map(&rgb),
        || detect::match_end::read_accolade_map(img),
    );
    let phase = detect::match_start::detect_phase_polled(
        img,
        &rgb,
        stability,
        detect::match_start::PhaseSource::for_reader(new),
    );
    let end_reel = detect::match_end::detect_end_reel_with(img, &rgb, &titles);
    let result_read = (new && signal.is_some()).then(|| shadow::read_result(img));
    json!({
        "signal": signal.map(|(outcome, source)| json!([outcome.to_string(), format!("{source:?}")])),
        "accolade_map": accolade_map,
        "phase": format!("{phase:?}"),
        "end_reel": end_reel,
        "result_read": result_read.map(|read| format!("{read:?}")),
    })
}

fn open(path: &str) -> Option<image::DynamicImage> {
    image::open(path)
        .ok()
        .map(|img| image::DynamicImage::ImageRgb8(img.to_rgb8()))
}

/// The `frame` column of each list, each list sorted and without repeats.
fn listed_frames(lists: &str, limit: usize) -> Vec<String> {
    let mut frames = Vec::new();
    for list in lists.split(':').filter(|list| !list.is_empty()) {
        let text = std::fs::read_to_string(list).expect("read a frame list");
        let mut lines = text.lines();
        let header: Vec<&str> = lines.next().expect("a header line").split('\t').collect();
        let column = header
            .iter()
            .position(|name| *name == "frame")
            .expect("a `frame` column");
        let set: std::collections::BTreeSet<String> = lines
            .filter_map(|line| line.split('\t').nth(column))
            .map(str::to_string)
            .collect();
        frames.extend(set.into_iter().take(limit));
    }
    frames
}

/// Every folder below each of `dirs`, as its frames in name order.
fn sequences(dirs: &str, limit: usize) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for dir in dirs.split(':').filter(|dir| !dir.is_empty()) {
        let mut folders: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .expect("read a sequence dir")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_dir())
            .collect();
        folders.sort();
        for folder in folders {
            let mut files: Vec<String> = std::fs::read_dir(&folder)
                .expect("read a sequence folder")
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| {
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| matches!(ext, "png" | "jpg" | "jpeg"))
                })
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            files.sort();
            files.truncate(limit);
            out.push(files);
        }
    }
    out
}

#[test]
#[ignore = "a bench over private frames: see the module docs for how to run it"]
fn run() {
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt as _;
    let out_path = env("OCR_BENCH_OUT");
    let data = std::path::PathBuf::from(env("OCR_BENCH_DATA"));
    let ocr_off = env("OCR_BENCH_OCR") == "off";
    // `ocr = "off"` implies the new reader.
    let new = ocr_off || env("OCR_BENCH_READER") == "new";
    let limit: usize = std::env::var("OCR_BENCH_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);

    ocr::set_ocr_threads(config::Config::default().ocr_threads_resolved());
    let wanted = if ocr_off {
        ocr::gate::OcrMode::Off
    } else {
        ocr::gate::OcrMode::On
    };
    // The gate is set directly: the switch refuses off while a read still
    // needs OCR, and this bench is how those reads are found.
    ocr::gate::set_mode(wanted);
    assert_eq!(ocr::gate::mode(), wanted, "the gate took the mode");
    let packs = new.then(|| {
        shadow::init(&shadow::ReaderConfig::from_data_dir(&data));
        format!("{:?}", shadow::pack_readiness())
    });
    let matcher = Arc::new(detect::hero_portrait::PortraitMatcher::load_installed(
        &data,
    ));

    let singles = listed_frames(&std::env::var("OCR_BENCH_TSVS").unwrap_or_default(), limit);
    let sequences = sequences(&std::env::var("OCR_BENCH_SEQS").unwrap_or_default(), limit);

    let mut out = std::fs::File::create(&out_path).expect("create the output file");
    std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(0o600))
        .expect("make the output private");
    let mut max_threads = proc_status("Threads:");
    let mut frames = 0usize;
    let (mut tab_ms, mut poll_ms) = (0f64, 0f64);
    let started = Instant::now();
    let cpu_start = cpu_ticks();

    for path in &singles {
        let Some(img) = open(path) else {
            writeln!(
                out,
                "{}",
                json!({"frame": hashed(path), "status": "open_failed"})
            )
            .unwrap();
            continue;
        };
        // A screen that holds still: two ticks with one stability state.
        let mut stability = detect::stability::FrameStability::default();
        let timer = Instant::now();
        let first = poll(&img, &mut stability, new);
        let second = poll(&img, &mut stability, new);
        poll_ms += timer.elapsed().as_secs_f64() * 1000.0;
        let timer = Instant::now();
        let board = tab(img, &matcher, new);
        tab_ms += timer.elapsed().as_secs_f64() * 1000.0;
        writeln!(
            out,
            "{}",
            json!({
                "frame": hashed(path),
                "kind": "single",
                "poll1": first,
                "poll2": second,
                "tab": board
            })
        )
        .unwrap();
        frames += 1;
        max_threads = max_threads.max(proc_status("Threads:"));
    }
    for sequence in &sequences {
        let mut stability = detect::stability::FrameStability::default();
        for path in sequence {
            let Some(img) = open(path) else {
                writeln!(
                    out,
                    "{}",
                    json!({"frame": hashed(path), "status": "open_failed"})
                )
                .unwrap();
                continue;
            };
            let timer = Instant::now();
            let tick = poll(&img, &mut stability, new);
            poll_ms += timer.elapsed().as_secs_f64() * 1000.0;
            let timer = Instant::now();
            let board = tab(img, &matcher, new);
            tab_ms += timer.elapsed().as_secs_f64() * 1000.0;
            writeln!(
                out,
                "{}",
                json!({"frame": hashed(path), "kind": "seq", "poll1": tick, "tab": board})
            )
            .unwrap();
            frames += 1;
            max_threads = max_threads.max(proc_status("Threads:"));
        }
    }

    // The counters below are this mode's only when the mode held for the
    // whole run.
    assert_eq!(ocr::gate::mode(), wanted, "the gate kept its mode");
    let blocked: Vec<serde_json::Value> = ocr::gate::blocked_calls()
        .iter()
        .map(|b| json!([b.site.to_string(), b.count]))
        .collect();
    let summary = json!({
        "summary": true,
        "frames": frames,
        "singles": singles.len(),
        "sequences": sequences.len(),
        "reader_new": new,
        "ocr_off": ocr_off,
        "gate_mode": format!("{:?}", ocr::gate::mode()),
        "packs": packs,
        "wall_s": started.elapsed().as_secs_f64(),
        "cpu_s": (cpu_ticks() - cpu_start) as f64 / 100.0,
        "tab_ms_total": tab_ms,
        "poll_ms_total": poll_ms,
        "vm_rss_kb": proc_status("VmRSS:"),
        "vm_hwm_kb": proc_status("VmHWM:"),
        "threads_now": proc_status("Threads:"),
        "threads_max": max_threads,
        "pool_started": ocr::pool_started(),
        "tessdata_lang_probed": ocr::tessdata_lang_probed(),
        "leptess_created": ocr::gate::leptess_created(),
        "tesseract_calls": ocr::gate::tesseract_calls(),
        "blocked_total": ocr::gate::blocked_total(),
        "blocked": blocked,
    });
    writeln!(out, "{summary}").unwrap();
    eprintln!("{summary:#}");
}
