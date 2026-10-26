//! Debug frames: the poll / rejected / accepted / own-row / map-label rings
//! and archives, and the report kit's board files.

use crate::*;

/// How many poll-tick frames `--dump-poll-frames` keeps (ring buffer on disk).
/// At a 4s poll interval this is ~10 minutes, enough that a defeat's
/// post-match sequence survives even if the next game is already underway
/// before the frames are copied out. Frames are a few MB each.
pub(crate) const POLL_DUMP_KEEP: usize = 150;

/// How many rejected-capture frames are kept in `<data_dir>/debug/rejected/`.
/// A rejected capture records nothing, so the frame is the only evidence for
/// diagnosing why ("it didn't record my game" is undebuggable otherwise).
/// Written only with `debug_ocr` on: the frame is unblanked and shows other
/// players' names.
pub(crate) const REJECTED_KEEP: usize = 30;

/// How many ACCEPTED scoreboard crops are kept in `<data_dir>/debug/accepted/`.
/// A silently-corrupt accepted board (OCR drift that still passed every trust
/// gate) is otherwise unrecoverable for calibration retuning. Written only
/// with `debug_ocr` on: the crop is unblanked and shows other players' names.
/// 60 covers about three evenings of Tab presses (20 held about one game's
/// worth on a busy night); one crop is 1 to 2 MB, so the ring stays under
/// about 120 MB.
pub(crate) const ACCEPTED_KEEP: usize = 60;

/// After this many consecutive scoreboard captures that parsed but resolved no
/// map, dump the map-label region to `debug/mapmiss/` so the miss is
/// diagnosable from raw pixels. The 07-17 Ilios game read no map on any frame
/// and left no pixel evidence of why (mangled OCR text is not enough).
pub(crate) const EMPTY_MAP_DUMP_THRESHOLD: usize = 5;

/// Ring size for the `debug/mapmiss/` map-region dumps.
pub(crate) const MAPMISS_KEEP: usize = 10;

/// Why a poll tick earned a `debug_ocr` evidence PNG. Mid-match ticks with
/// no outcome signal are not saved; `--dump-poll-frames` is the every-tick
/// hammer and stays independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PollDebugHit {
    /// The second agreeing word-OCR tick.
    Confirm,
    /// First word-OCR streak sighting (pre-confirm). A lone VICTORY/SVACTORY
    /// that never gets a second tick still leaves a frame.
    Streak,
}

/// Decide whether this poll tick should write a `debug/poll/` evidence PNG
/// when `debug_ocr` is on. Mirrors the poller's confirm/streak match so a
/// test can pin the triggers without driving the `select!` loop.
pub(crate) fn poll_debug_hit(
    signal: Option<(detect::MatchOutcome, detect::match_end::OutcomeSource)>,
    prior_streak: Option<(detect::MatchOutcome, std::time::Duration)>,
    confirm_window: std::time::Duration,
) -> Option<(PollDebugHit, detect::MatchOutcome)> {
    match signal {
        Some((outcome, _)) => {
            let agreed =
                prior_streak.is_some_and(|(prev, age)| prev == outcome && age <= confirm_window);
            if agreed {
                Some((PollDebugHit::Confirm, outcome))
            } else {
                Some((PollDebugHit::Streak, outcome))
            }
        }
        None => None,
    }
}

pub(crate) fn poll_debug_prefix(hit: PollDebugHit, outcome: detect::MatchOutcome) -> String {
    let kind = match hit {
        PollDebugHit::Confirm => "confirm",
        PollDebugHit::Streak => "streak",
    };
    format!("poll_{kind}_{outcome}")
}

/// Save a debug frame into `dir` as `<prefix>_<timestamp>.png`, keeping at
/// most `keep` PNGs in the directory (oldest by mtime evicted). Each ring gets
/// a dedicated directory (`debug/poll`, `debug/rejected`), so every PNG there
/// participates in the same ring regardless of prefix. On-hit names are
/// `poll_confirm_{outcome}_…` / `poll_streak_{outcome}_…`.
pub(crate) fn save_frame_ring(
    dir: &std::path::Path,
    prefix: &str,
    img: &image::DynamicImage,
    keep: usize,
) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let name = format!(
        "{prefix}_{}.png",
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );
    if let Err(e) = img.save(dir.join(&name)) {
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "failed to save debug frame");
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut frames: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("png"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        if frames.len() > keep {
            frames.sort_by_key(|(t, _)| *t);
            for (_, old) in &frames[..frames.len() - keep] {
                let _ = std::fs::remove_file(old);
            }
        }
    }
}

/// Debug rings that hold unblanked board pixels (other players' names).
/// Written only while `debug_ocr` is on.
pub(crate) const BOARD_PIXEL_DEBUG_DIRS: [&str; 2] = ["accepted", "rejected"];

/// Archive a frame whose capture was rejected by a trust gate, for diagnosis.
/// Only with `debug_ocr` on (`debug`): the frame is unblanked. Runs the PNG
/// encode off the async runtime; fire-and-forget. The handle is for tests.
pub(crate) fn save_rejected_frame(
    debug: bool,
    data_dir: &std::path::Path,
    img: image::DynamicImage,
    reason: &'static str,
) -> Option<tokio::task::JoinHandle<()>> {
    if !debug || img.width() < 2 || img.height() < 2 {
        return None;
    }
    let dir = data_dir.join("debug").join("rejected");
    Some(tokio::task::spawn_blocking(move || {
        save_frame_ring(&dir, &format!("rejected_{reason}"), &img, REJECTED_KEEP);
    }))
}

/// A rank screen's `outcome.png` goes to the active game only when that game
/// recorded evidence (a capture, an outcome) this recently.
pub(crate) const RANK_OUTCOME_MAX_IDLE: std::time::Duration =
    std::time::Duration::from_secs(10 * 60);

/// Every name read on a board's rows (each name window), for the report
/// scrub list. Empty under the new reader, which reads no names.
pub(crate) fn board_names(rows: &[ocr::RowOcrResult]) -> Vec<String> {
    rows.iter()
        .flat_map(|row| row.name.iter().chain(row.name_alternates.iter().flatten()))
        .map(|cell| cell.value.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Report kit (`report::kit`): the blanked scoreboard and the reads of this
/// accepted capture. Written off the capture path; a failure is logged and
/// never touches the stored match.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_report_kit(
    data_dir: &std::path::Path,
    session_id: &str,
    frame: &image::DynamicImage,
    team_size: usize,
    ocr_parsed: &storage::PersonalMatch,
    stored: &storage::PersonalMatch,
    captured_at: chrono::DateTime<Utc>,
    tab_at: stat_tracker::report::blank::CaptureTime,
    board_names: Vec<String>,
    confidences: &std::collections::BTreeMap<String, stat_tracker::report::kit::ReaderRead>,
) {
    let Some(layout) = stat_tracker::report::blank::TeamLayout::from_team_size(Some(team_size))
    else {
        return;
    };
    let reads = stat_tracker::report::kit::KitReads::from_capture(
        ocr_parsed,
        stored,
        frame.width(),
        frame.height(),
        layout,
        captured_at,
    )
    .with_confidences(confidences);
    let (data_dir, session_id, frame) = (
        data_dir.to_path_buf(),
        session_id.to_string(),
        frame.clone(),
    );
    tokio::task::spawn_blocking(move || {
        if let Err(e) = stat_tracker::report::kit::record_scoreboard_at(
            &data_dir,
            &session_id,
            &frame,
            Some(team_size),
            &reads,
            tab_at,
        ) {
            tracing::debug!(target: "scuffed_stat_tracker", error = %e, "report kit: scoreboard not written");
        }
        // The board's names stay on this PC; a report hands them to the
        // log scrubber.
        if !board_names.is_empty()
            && let Err(e) =
                stat_tracker::report::kit::record_names(&data_dir, &session_id, &board_names)
        {
            tracing::debug!(target: "scuffed_stat_tracker", error = %e, "report kit: names not written");
        }
    });
}

/// The board written to `debug/accepted`: the unpadded scoreboard crop
/// (1007 px tall at 1440p, 756 px at 1080p), taken from the same origin as
/// [`ocr::preprocess::crop_scoreboard`]. Offline digit and ocr-v1 replay
/// place rows from this height, and the saved boards are the benchmark
/// source. The 5v5 bottom pad stays on the recognition image only.
pub(crate) fn debug_accepted_board(frame: &image::DynamicImage) -> Arc<image::DynamicImage> {
    Arc::new(ocr::preprocess::crop_scoreboard(frame))
}

/// Archive the scoreboard crop of an ACCEPTED capture into a bounded ring, so a
/// board that OCR'd wrong but still passed every trust gate can be inspected
/// after the fact (the capture gate holds bad cells, but the underlying crop is
/// the only way to retune calibration). Only with `debug_ocr` on (`debug`):
/// the crop is unblanked and shows other players' names. Callers pass
/// [`debug_accepted_board`], the unpadded crop. Fire-and-forget;
/// encode off-runtime. The handle is for tests.
pub(crate) fn save_accepted_frame(
    debug: bool,
    data_dir: &std::path::Path,
    board: Arc<image::DynamicImage>,
) -> Option<tokio::task::JoinHandle<()>> {
    if !debug {
        return None;
    }
    let dir = data_dir.join("debug").join("accepted");
    Some(tokio::task::spawn_blocking(move || {
        save_frame_ring(&dir, "accepted", &board, ACCEPTED_KEEP);
    }))
}

/// Local review PNG of the member's own scoreboard row, under
/// `debug/accepted/own-row/`. Only with `debug_ocr` on, and only when the
/// own row was found: the file is that one row, widened left of the name
/// column so the hero portrait is in it. When the header is clipped, the
/// cut is [`detect::own_row::own_row_review_image_from`] of `frame` (the
/// hero reader's widened crop) with the name column shifted by that pad.
/// Other rows are not written. The name-line seed and the report kit are
/// untouched. The handle is for tests.
pub(crate) fn save_own_row_review(
    debug: bool,
    data_dir: &std::path::Path,
    frame: &image::DynamicImage,
    board: Arc<image::DynamicImage>,
    row: Option<usize>,
    team_size: usize,
) -> Option<tokio::task::JoinHandle<()>> {
    if !debug {
        return None;
    }
    let row = row?;
    let frame = frame.clone();
    let dir = data_dir.join("debug").join("accepted").join("own-row");
    Some(tokio::task::spawn_blocking(move || {
        let Some(crop) = detect::own_row::own_row_review_image_from(
            Some(&frame),
            board.as_ref(),
            team_size,
            row,
        ) else {
            return;
        };
        save_frame_ring(&dir, "ownrow", &crop, ACCEPTED_KEEP);
    }))
}

/// Board-pixel debug folders left over on disk while `debug_ocr` is off.
/// They are never deleted (they are the user's files); startup only says
/// they are no longer written.
pub(crate) fn stale_board_debug_dirs(
    debug: bool,
    data_dir: &std::path::Path,
) -> Vec<std::path::PathBuf> {
    if debug {
        return Vec::new();
    }
    BOARD_PIXEL_DEBUG_DIRS
        .iter()
        .map(|name| data_dir.join("debug").join(name))
        .filter(|dir| dir.is_dir())
        .collect()
}

/// Keep the full top-bar map-label box from a Tab whose label is not a flat bar.
/// The first such Tab writes `<session_id>-<width>x<height>-1.png`. A later
/// Tab at least 60 seconds after that writes the matching `-2.png`. Width and
/// height are the capture size. The PNG is the raw box from this frame, with
/// no timer trim and no brightness stretch. The map-label OCR retry does not
/// supply these pixels. A flat bar does not take a slot. Local files under
/// `debug/mapcrops` only.
/// Not part of the rolling debug rings and not included in report bundles.
/// From an accepted capture it is not gated on `debug_ocr`; a rejected
/// capture goes through [`archive_rejected_map_label_crop`]. The handle is
/// for tests.
pub(crate) fn archive_map_label_crop(
    data_dir: &std::path::Path,
    session_id: &str,
    frame: &image::DynamicImage,
) -> Option<tokio::task::JoinHandle<()>> {
    let job = mapcrops::prepare_mapcrop(data_dir, session_id, frame)?;
    Some(tokio::task::spawn_blocking(move || {
        mapcrops::commit_mapcrop(job)
    }))
}

/// [`archive_map_label_crop`] for a capture a trust gate rejected: only with
/// `debug_ocr` on, and never for `preflight` (a menu has no top bar). A
/// rejected frame may be gameplay rather than a Tab board, and there the
/// map-label rectangle covers the kill feed, which shows player names.
pub(crate) fn archive_rejected_map_label_crop(
    debug: bool,
    data_dir: &std::path::Path,
    session_id: &str,
    frame: &image::DynamicImage,
    reason: &'static str,
) -> Option<tokio::task::JoinHandle<()>> {
    if !debug || reason == "preflight" {
        return None;
    }
    archive_map_label_crop(data_dir, session_id, frame)
}
