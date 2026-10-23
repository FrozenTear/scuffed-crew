//! The end-screen split log line.

use super::types::{CloseReason, OpenNew};
use crate::model::MatchOutcome;

/// The end-screen split, as the daemon log prints it.
///
/// `armed` is whether a boundary was already pending. `sealed_hint` is the
/// hint stored on the session being closed. `accolade_map` is the name the
/// new session takes; this close always has one. A missing hint prints
/// `none`, the same spelling as a present hint's name, not `Some` or `None`.
pub fn format_end_screen_split(
    armed: bool,
    sealed_hint: Option<MatchOutcome>,
    accolade_map: &str,
) -> String {
    let hint = match sealed_hint {
        Some(outcome) => outcome.to_string(),
        None => "none".to_string(),
    };
    format!("armed={armed} sealed_hint={hint} accolade_map={accolade_map}")
}

/// Arm flag and accolade map for an end-screen split log.
///
/// `None` unless this open is an end-screen close that carries a map. The
/// map is required: this close always has one, so there is no `none` stand-in.
pub fn end_screen_log_fields(armed: bool, open: &OpenNew) -> Option<(bool, String)> {
    if open.reason != CloseReason::EndScreenMap {
        return None;
    }
    Some((armed, open.new_map.clone()?))
}
