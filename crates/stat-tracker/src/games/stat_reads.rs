//! The reads of each own-row stat over the current game, for the Unsure
//! confirm rule ([`crate::unsure_rule`], alpha.3 item 5).
//!
//! The tracker pushes one read per own-row stat per taken board into
//! [`StatReadLog`] and keeps it in `stat_reads.json` next to the snapshot.
//! The desktop app reads it back and flags a stat of that game only when
//! [`confirm_stat`] says [`Verdict::Unsure`], live and at game end. Only the
//! latest game has a log: an older game keeps the flags stored on its row.
//!
//! Fail safe: a missing, unreadable or foreign log only means no verdict.
//! It never drops, blocks or changes a stored game.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::storage::PersonalMatch;
use crate::suspect::flat_suspect_name;
use crate::unsure_rule::{StatConfirm, StatRead, Verdict, confirm_stat};

/// The log's file in the data directory.
pub const FILE: &str = "stat_reads.json";

/// The six own-row stats as flat suspect names, in gate order (E A D DMG
/// H MIT), the order of `reader_apply::own_row_guesses`.
pub const STATS: [&str; 6] = ["e", "a", "d", "dmg", "h", "mit"];

/// One Tab's read of each own-row stat. `None`: this Tab pushes nothing for
/// the stat (no identified own row, or the cell was never attempted).
pub type TabReads = [Option<StatRead>; 6];

/// One read as saved. A reset never rides on a saved read: a new game
/// starts a new log.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SavedRead {
    pub value: Option<u32>,
    pub confidence: f32,
    pub unsure: bool,
}

impl From<StatRead> for SavedRead {
    fn from(r: StatRead) -> Self {
        Self {
            value: r.value,
            confidence: r.confidence,
            unsure: r.unsure,
        }
    }
}

impl SavedRead {
    /// The read for the rule.
    pub fn to_read(self) -> StatRead {
        StatRead {
            value: self.value,
            confidence: self.confidence,
            unsure: self.unsure,
            reset: false,
        }
    }
}

/// The reads of the current game, one list per stat, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatReadLog {
    /// The game these reads belong to.
    pub session_id: String,
    pub reads: [Vec<SavedRead>; 6],
    /// The reads of a held fresh-match board. They are not on the current
    /// game: they join the next game when the board is carried into it, and
    /// are dropped when the hold is.
    #[serde(default)]
    pub held: [Vec<SavedRead>; 6],
}

impl StatReadLog {
    /// A fresh-match board was held (boundary `defer`). `carried`: the hold
    /// keeps the board for the next game; its reads replace any older hold.
    /// A hold that keeps no board drops the reads.
    pub fn hold(&mut self, tab: &TabReads, carried: bool) {
        self.held = Default::default();
        if carried {
            push_tab(&mut self.held, tab);
        }
    }

    /// A taken board of `session_id`. A different session is a new game:
    /// the old reads are cleared, and the held board's reads start the new
    /// game when `carried_board` (the board was carried into it) or are
    /// dropped otherwise.
    pub fn push(&mut self, session_id: &str, carried_board: bool, tab: &TabReads) {
        if self.session_id != session_id {
            let held = std::mem::take(&mut self.held);
            self.reads = if carried_board {
                held
            } else {
                Default::default()
            };
            self.session_id = session_id.to_string();
        }
        push_tab(&mut self.reads, tab);
    }

    /// The rule's verdict for each stat of `session_id`, `None` for a stat
    /// with no reads (the stored flag stands then). All `None` when the log
    /// is for another game.
    pub fn confirm(&self, session_id: &str, game_over: bool) -> [Option<StatConfirm>; 6] {
        if self.session_id != session_id || session_id.is_empty() {
            return [None; 6];
        }
        std::array::from_fn(|i| {
            let reads: Vec<StatRead> = self.reads[i].iter().map(|r| r.to_read()).collect();
            (!reads.is_empty()).then(|| confirm_stat(&reads, game_over))
        })
    }
}

fn push_tab(lists: &mut [Vec<SavedRead>; 6], tab: &TabReads) {
    for (list, read) in lists.iter_mut().zip(tab) {
        if let Some(read) = read {
            list.push((*read).into());
        }
    }
}

/// The log in `data_dir`, or an empty one when it is missing or unreadable.
pub fn load(data_dir: &Path) -> StatReadLog {
    std::fs::read(data_dir.join(FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Load the log, apply `f` and write it back (temp file, then rename). An
/// error is logged and dropped: the log never blocks a capture.
pub fn update(data_dir: &Path, f: impl FnOnce(&mut StatReadLog)) {
    let mut log = load(data_dir);
    f(&mut log);
    let path = data_dir.join(FILE);
    let tmp = data_dir.join(format!("{FILE}.tmp"));
    let res = serde_json::to_vec(&log)
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(&tmp, bytes))
        .and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = res {
        tracing::warn!(target: "scuffed_stat_tracker", error = %e, "stat read log not written");
    }
}

/// Put the verdicts on the rows of their game: a stat with a verdict is
/// flagged when it is [`Verdict::Unsure`] and not flagged otherwise. A stat
/// with no verdict keeps its stored flag, and so does every other field.
pub fn apply_verdicts(rows: &mut [PersonalMatch], session_id: &str, v: &[Option<StatConfirm>; 6]) {
    for row in rows.iter_mut().filter(|r| r.session_id == session_id) {
        for (flat, c) in STATS.iter().zip(v) {
            let Some(c) = c else { continue };
            row.suspect_fields
                .retain(|name| flat_suspect_name(name) != Some(flat));
            if c.verdict == Verdict::Unsure {
                row.suspect_fields.push((*flat).to_string());
            }
        }
    }
}

/// The confidence line under an Unsure stat: the reader's confidence in the
/// value the rule judged, "read N at X%" when that is not the stored number,
/// and "No read" when no read read that value (never 0%).
pub fn confidence_line(c: &StatConfirm, stored: u32) -> String {
    let Some(conf) = c.confidence else {
        return "No read".to_string();
    };
    let pct = (conf * 100.0).round() as u32;
    match c.value {
        Some(v) if v != stored => format!("Read {v} at {pct}%"),
        _ => format!("Read at {pct}%"),
    }
}

#[cfg(test)]
mod tests;
