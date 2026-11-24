//! When the daemon writes `live_snapshot.json`.
//!
//! A mutation only marks the snapshot dirty. A dirty snapshot is written
//! when a GUI is running (see `gui_presence`) and [`DEBOUNCE`] has passed
//! since the last write, or when a write is owed whatever the GUI: a game
//! just finished ([`MUST_SETTLE`] after the mark, so the result and the last
//! Tab of the game land in one write), or the daemon is stopping. Startup
//! writes once directly. With no GUI and no owed write nothing runs, so a
//! closed GUI (including one quit from the tray) costs no snapshot work.

use std::time::{Duration, Instant};

/// Minimum gap between two writes for a running GUI.
pub(crate) const DEBOUNCE: Duration = Duration::from_secs(2);
/// An owed write waits this long after its mark for the rest of the burst.
pub(crate) const MUST_SETTLE: Duration = Duration::from_secs(3);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SnapshotPolicy {
    dirty: bool,
    /// Set when a finished game owes a write; the first mark's time.
    must_since: Option<Instant>,
    last_write: Option<Instant>,
    /// Bumped by every mark, so a write clears only what it saw.
    generation: u64,
}

impl SnapshotPolicy {
    pub(crate) const fn new() -> Self {
        Self {
            dirty: false,
            must_since: None,
            last_write: None,
            generation: 0,
        }
    }

    pub(crate) fn mark(&mut self, now: Instant, owed: bool) {
        self.dirty = true;
        self.generation = self.generation.wrapping_add(1);
        if owed && self.must_since.is_none() {
            self.must_since = Some(now);
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn dirty(&self) -> bool {
        self.dirty
    }

    /// Whether to write now.
    pub(crate) fn due(&self, now: Instant, gui_present: bool) -> bool {
        if !self.dirty {
            return false;
        }
        let gap_ok = self
            .last_write
            .is_none_or(|last| now.duration_since(last) >= DEBOUNCE);
        let owed = self
            .must_since
            .is_some_and(|since| now.duration_since(since) >= MUST_SETTLE);
        gap_ok && (gui_present || owed)
    }

    /// A write that started at `now` after mark `generation` succeeded. A
    /// mark that came in during the write keeps the snapshot dirty.
    pub(crate) fn wrote(&mut self, now: Instant, generation: u64) {
        if self.generation == generation {
            self.dirty = false;
            self.must_since = None;
        }
        self.last_write = Some(now);
    }
}
