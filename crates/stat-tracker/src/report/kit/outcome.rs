//! `outcome.png`: the result word box of the competitive rank screen.

use std::io;
use std::path::Path;

use image::DynamicImage;

use super::super::blank;
use super::store::{
    OUTCOME_PNG, encode_png, kit_root, private_dir, prune_locked, retention, session_dir,
    was_removed, write_lock, write_private,
};

/// Whether a polled frame may give `outcome.png` (spec section 3).
///
/// Only when the rank screen detector fired on this frame AND the scoreboard
/// detector did not accept it. The rank word match is loose (a title such as
/// "QUICKDRAW" contains DRAW), so a Tab-held scoreboard could fire it, and
/// on a scoreboard the word box holds the member's portrait, badge, and
/// title next to their name.
pub fn rank_outcome_allowed(
    source: Option<crate::detect::match_end::OutcomeSource>,
    preflight: crate::frame::layout::ScoreboardPreflight,
    rows_look_like_scoreboard: bool,
) -> bool {
    source == Some(crate::detect::match_end::OutcomeSource::RankScreen)
        && preflight == crate::frame::layout::ScoreboardPreflight::NotAScoreboard
        && !rows_look_like_scoreboard
}

/// The `outcome.png` bytes for one polled frame, or `None`
/// ([`rank_outcome_allowed`] decides).
pub fn rank_outcome_png(
    source: Option<crate::detect::match_end::OutcomeSource>,
    frame: &DynamicImage,
) -> Option<Vec<u8>> {
    if source != Some(crate::detect::match_end::OutcomeSource::RankScreen) {
        return None;
    }
    let board = crate::ocr::preprocess::crop_scoreboard(frame);
    let (preflight, scan) = crate::frame::layout::preflight_scoreboard(&board);
    if !rank_outcome_allowed(source, preflight, scan.looks_like_scoreboard()) {
        return None;
    }
    encode_png(&blank::outcome_band(frame)).ok()
}

/// Store the bytes from [`rank_outcome_png`] as the session's `outcome.png`.
pub fn record_outcome_png(data_dir: &Path, session_id: &str, png: &[u8]) -> io::Result<()> {
    let Some(dir) = session_dir(data_dir, session_id) else {
        return Ok(());
    };
    let _guard = write_lock();
    if was_removed(&dir) {
        return Ok(());
    }
    private_dir(&kit_root(data_dir))?;
    private_dir(&dir)?;
    write_private(&dir, OUTCOME_PNG, png)?;
    prune_locked(data_dir, retention());
    Ok(())
}
