//! Per-cell capture gate for the scoreboard OCR pipeline.
//!
//! Overwatch scoreboard counters are cumulative within a single match, so a
//! later capture that reads a counter *below* the last accepted value is a
//! misread, not real play. Two field failure modes (2026-07-18 night shift,
//! see `docs/notes/night-shift-backlog.md` item 8 and the step-0 drift
//! analysis) corrupt real games:
//!
//! * **collapse** — a two-digit kill column clips to one digit ("13" → "1"),
//!   or a four-digit accumulator tail-clips ("2341" → "234"). Both are
//!   *decreases* versus the last accepted value.
//! * **inflation** — a ghost leading "9" walks in from an ability icon left of
//!   the elims column ("13" reads "91"/"93"/"99"). These *increase* past any
//!   plausible per-second rate, so a monotonic check alone cannot catch them —
//!   the rate cap here is load-bearing, not optional.
//!
//! This gate is per-cell and one-sided by design: it holds the previous
//! accepted value for a *single* cell that regresses (B) or that jumps beyond a
//! plausible rate without corroboration (C), while every genuinely-advancing
//! cell in the same capture passes through untouched. It is deliberately
//! DISTINCT from the whole-row game-split signal (`stats_regressed`, which
//! requires 2 of 3 of E/D/DMG to drop): a real new game resets every counter
//! and must still split, so the caller passes `split = true` and the gate then
//! accepts the raw read verbatim as the first capture of the new game.
//!
//! ## No downward revision
//!
//! Elims, assists, deaths, damage, healing, and mitigation do not decrease
//! inside one game once the value has been confirmed. A lower read keeps
//! the accepted value and names that field unsure (`e`, `a`, `d`, `dmg`,
//! `h`, `mit`). Three OCR reads of 1 must not replace a held 11.
//!
//! Two cases may still store a lower number. An unconfirmed latch (a
//! fallback, or a value stored while the capture was low-trust) yields to
//! one later clean read, even when that read is lower. A high stored while
//! another column was held down is a row shift, and a later read of the
//! real row can replace it when that read repeats. A value confirmed by a
//! clean read, and not a row shift, cannot go down. A read whose every cell
//! is suspect, including a suspect own row, does not seed or raise the
//! baseline and stores none of its numbers. A real new game is the
//! whole-board reset in `boundary`, and the caller passes `split = true` so
//! this gate stores the new counters instead of holding them.
//!
//! ## Single-field jump
//!
//! A rise in one column past [`JUMP_MAX`], while every other column stays
//! within [`QUIET_RISE`] of its accepted value, is unsure until the next
//! capture agrees, within the corroboration band. 13 read as 18 (+5 elims,
//! damage +100) stays 13. A second read of 18 stores 18. A correcting read
//! does not. Elims 4 to 9 with assists and deaths also up is ordinary play
//! and is stored. A drop in another column is the board moving, not a quiet
//! cell, so that rise is stored too. Two or more columns past their own
//! limits in the same capture are a real stomp and are not held by this
//! rule. The rate cap still holds a kill-column spike such as 9 to 91.
//!
//! ## Wide-column inflation (CG-4 B1 / B2)
//!
//! Pre-CG-4, wide accumulator columns (DMG/HLG/MIT) had **no** rate cap, so a
//! drifted inject (DMG 35031, HLG 22994) was accepted and latched. CG-4 closes
//! that gap without re-capping clean wide advances (real multi-k damage bursts
//! must never hold):
//!
//! * **B1** — when the *current* read is edge-ink suspect, wide cols get a
//!   calibrated rate ceiling (same HoldKind::RateCap path as kill cols).
//! * **B2** — trailing-digit inject heuristic (`accepted_digits+1` and prefix
//!   ≥ accepted) holds as `HoldKind::DigitInject`. Belt for `1681→22994`-style
//!   cases; does **not** cover `2782→22994` alone (prefix < accepted) — that
//!   needs B1 + Lane A geometry.
//!
//! ## Low-trust latch
//!
//! A raw-text fallback, or a first capture whose kill columns are past the
//! parser ceilings, is stored but marked `low_trust`. Those columns are
//! `unconfirmed`. One later clean read, with no suspect cell, replaces an
//! unconfirmed column even when the read is lower. A confirmed column never
//! decreases. A read with every cell suspect does not seed that latch and
//! does not raise it.
//!
//! That replacement still goes through the same holds as any other capture:
//! a decrease of a *confirmed* column is the monotonic hold (B), a kill-column
//! jump past [`max_delta`] without corroboration is the rate cap (C), and a
//! trailing-digit inject is [`HoldKind::DigitInject`] (B2). A replacement
//! below the confirmed value a fallback moved off, or a wide column that is
//! the latched number with its last digit cut off, also falls through to B.
//! While any of those holds is in force, or any column is still unconfirmed,
//! `low_trust` stays set. A second fallback, or a read with edge-ink, does
//! not clear an unconfirmed column.
//!
//! ## Known residuals (documented, out of scope)
//!
//! * **F-CG2-1a** — the raw-continuity split vote anchors on `last_raw`, which
//!   is only trustworthy once at least one capture followed the injection. If a
//!   ≥120s capture gap lands on the very first capture AFTER a clean-slipping
//!   inflated read (`last_raw` still = the inject), a later clean read drops
//!   versus both anchors and can still split mid-game. Requires an idle Tab
//!   immediately after an inject that also evaded the edge-ink flag — compound
//!   odds are low. Revisit only if observed in the field.
//! * **CG-1a (mode c, row-shift).** A capture that reads a different
//!   player's row produces clean cells (edge-ink cannot flag them: the glyphs
//!   are well-centered, just the wrong player's). A higher wrong-row read can
//!   still latch. It no longer walks the accepted value down. The fix for the
//!   high latch is row identity (name-anchored row selection), tracked as the
//!   CG-1a follow-up.
//!
//! ## Fixtures
//!
//! Numeric replay tests below encode real `matches.jsonl` series. The pixel-level
//! edge-ink threshold that feeds the `suspect` mask was calibrated against 20
//! real drift frames at `crates/stat-tracker/test-data/drift-20260720/`
//! (gitignored, local-only — not in CI).
//!
//! ## Layout
//!
//! - `state`: [`Counters`], [`GateState`] and the per-capture result types.
//! - `limits`: the rate model and its limits.
//! - `apply`: [`apply_gate`] and its other entry points.
//! - `fields`: `suspect_fields` names for a capture.
//!
//! Every public item is re-exported here, so `capture_gate::X` paths are
//! unchanged.
//!
//! [`JUMP_MAX`]: limits::JUMP_MAX
//! [`QUIET_RISE`]: limits::QUIET_RISE
//! [`max_delta`]: limits::max_delta

mod apply;
mod fields;
mod limits;
mod state;

pub use apply::{apply_gate, apply_gate_held, apply_gate_seeded, apply_gate_with_trust};
pub use fields::{held_fields, lowered_fields, revised_fields, unsure_fields};
pub use limits::raw_dropped;
pub(crate) use limits::{
    KILL_RATE_DIVISOR_SECS, KILL_RATE_SLACK, WIDE_RATE_PER_SEC, WIDE_RATE_SLACK,
};
pub use state::{COL_FIELD, Counters, GATE_COLS, GateOutcome, GateState, Hold, HoldKind, Unlatch};

// Names the test files saw through `use super::*` before the split: the old
// file's `Duration` import and one private limit.
#[cfg(test)]
use limits::JUMP_MAX;
#[cfg(test)]
use std::time::Duration;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_holds;

#[cfg(test)]
mod tests_replay;

#[cfg(test)]
mod tests_unread;
