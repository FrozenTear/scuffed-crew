//! The gate policy: [`apply_gate`] and its trust, held and seeded entry
//! points.

use std::time::Duration;

use super::limits::{
    JUMP_MAX, corroborates, is_kill_col, is_trailing_digit_inject, max_delta, secs_u32,
    sudden_single_field_jump,
};
use super::state::{Counters, GATE_COLS, GateOutcome, GateState, Hold, HoldKind, Unlatch};

/// Apply the per-cell capture gate.
///
/// `prev` — the gate state and elapsed-since carried from the last accepted
/// capture of this game, or `None` for the first capture. `raw` — this
/// capture's parsed counters. `suspect` — per-column edge-ink mask (CG-3): a
/// `true` column had glyph ink touching a crop edge, so its read is stripped of
/// all gate influence (never corroborates a jump, never drives an un-latch),
/// though a *suspect* wide-column advance is now also rate-capped (CG-4 B1)
/// and trailing-digit injects are held (CG-4 B2). `split` — whether the
/// whole-row game-split signal already fired (a real new game).
///
/// On a split or a first capture the raw read is accepted verbatim (a new game
/// legitimately resets every counter) and marked low-trust when the read is
/// untrusted or a kill column is past the ceilings. A read whose every cell
/// is suspect is the exception: none of its numbers are accepted, on a first
/// capture or later. Otherwise each cell is
/// checked independently: a decrease of a confirmed column keeps the previous
/// accepted value and is unsure; an unconfirmed column yields to one clean
/// read even when that read is lower; a trailing-digit inject is held (CG-4 B2);
/// a single-field rise past
/// [`JUMP_MAX`] waits for a matching second read; an increase beyond the
/// plausible rate is held unless a clean previous raw read corroborates it
/// (C / CG-4 B1 for suspect wide cols); an advance within those limits passes
/// through unchanged.
///
/// `apply_gate` is [`apply_gate_with_trust`] with the read marked trusted.
pub fn apply_gate(
    prev: Option<(GateState, Duration)>,
    raw: Counters,
    suspect: [bool; GATE_COLS],
    split: bool,
) -> GateOutcome {
    apply_gate_with_trust(prev, raw, suspect, split, true)
}

fn kill_col_over_ceiling(col: usize, value: u32) -> bool {
    match col {
        0 => value > crate::parse::MAX_ELIMS,
        1 => value > crate::parse::MAX_ASSISTS,
        2 => value > crate::parse::MAX_DEATHS,
        _ => false,
    }
}

/// Kill columns past the shared parse ceilings.
///
/// A 0.4.20 save has no per-column trust bit, so only these columns are
/// marked. A shifted wide column that sits above its real value stays there:
/// a confirmed column does not decrease. Wide columns have no ceiling here.
fn mark_implausible(unconfirmed: &mut [bool; GATE_COLS], c: Counters) {
    let cols = c.to_array();
    for col in 0..GATE_COLS {
        if kill_col_over_ceiling(col, cols[col]) {
            unconfirmed[col] = true;
        }
    }
}

/// An untrusted read must not corroborate the next jump or build a down
/// streak. Recording every column suspect is the same rule the edge-ink
/// mask already uses.
fn raw_suspect_mask(suspect: [bool; GATE_COLS], trusted: bool) -> [bool; GATE_COLS] {
    if trusted { suspect } else { [true; GATE_COLS] }
}

fn fresh_state(raw: Counters, suspect: [bool; GATE_COLS], trusted: bool) -> GateState {
    // An untrusted first capture has not confirmed any column. A trusted
    // one only leaves the kill columns that are past the ceilings
    // unconfirmed. The other columns of that read were fine.
    let mut unconfirmed = [!trusted; GATE_COLS];
    mark_implausible(&mut unconfirmed, raw);
    GateState {
        accepted: raw,
        last_raw: raw,
        last_raw_suspect: raw_suspect_mask(suspect, trusted),
        low_trust: unconfirmed.iter().any(|&c| c),
        unconfirmed,
        ..Default::default()
    }
}

/// [`apply_gate`] with an explicit trust bit.
///
/// `trusted` is false when the six stats came from the raw-text fallback.
/// That latch is `low_trust` even when every column is in range, and so is
/// a first capture whose kill columns are implausible: the first capture
/// used to be stored as the baseline with no check, which is how a shifted
/// assists/deaths pair locked for the rest of the match.
///
/// A later read that is trusted and has no suspect cell replaces an
/// *unconfirmed* column on that one read, even when the number is lower.
/// A confirmed column still takes the monotonic hold (B). The rate cap (C)
/// and the trailing-digit inject hold (B2) still apply, including to a
/// low-trust column whose clean read jumps past them. A replacement below
/// the confirmed value a fallback moved off, and a wide column whose clean
/// read is the latched number with its last digit cut off, also fall
/// through to B. A row-shift high still waits for the same lower number to
/// repeat. `low_trust` stays set while any column is held or still
/// unconfirmed. A second fallback, or a read with edge-ink, does not clear
/// an unconfirmed column. An untrusted read is recorded as suspect on every
/// column, so it does not corroborate the next jump. A read with every cell
/// suspect does not seed or raise the baseline.
pub fn apply_gate_with_trust(
    prev: Option<(GateState, Duration)>,
    raw: Counters,
    suspect: [bool; GATE_COLS],
    split: bool,
    trusted: bool,
) -> GateOutcome {
    apply_gate_held(prev, raw, suspect, [false; GATE_COLS], split, trusted)
}

/// [`apply_gate_with_trust`] with cells the reader did not vouch for.
///
/// `held[col]` means this capture has no number for the column
/// (`reader = "new"`: the digit read was suspect or empty, and nothing else
/// may supply it). `raw` for that column is ignored. The accepted value
/// stays ([`HoldKind::Unread`]), and the column's previous raw read, its
/// suspect bit and its streaks are carried over untouched, so the next
/// capture is compared with the last real read. A held column never votes:
/// it cannot corroborate a jump, make another column's jump look lone or
/// quiet, confirm an unconfirmed column, or tag a row shift.
///
/// On a first capture or a split there is no accepted value to keep. The
/// column is a placeholder 0 marked [`GateState::unread`], and the caller
/// stores no row for the game while any column is unread
/// ([`GateState::unread_cols`]). The first real read of the column is then
/// taken as a first capture's would be, with no rate cap or jump hold
/// against the placeholder, unless another column takes the monotonic hold
/// on that capture: a board whose read columns drop may be the next game's,
/// and its number must not land in this game's never-read column. That
/// read is an [`HoldKind::Unread`] hold and the column stays unread.
///
/// The rate checks of a column measure from its last accepted read
/// ([`GateState::held_secs`]), so the first sure read after a run of held
/// captures is not judged against one capture interval.
///
/// A game with no row yet whose reads so far were all low-trust
/// ([`GateState::only_low_trust_reads`]: a suspect own row, which may be
/// another player's) is started over by the first trusted board: that board
/// is gated as a first capture. Nothing of the game is stored yet, so there
/// is no row the trusted read has to agree with, and holding it against a
/// suspect row's numbers would keep the game from ever getting a row
/// (R3-2).
///
/// With no held column and no unread column this is exactly
/// [`apply_gate_with_trust`].
pub fn apply_gate_held(
    prev: Option<(GateState, Duration)>,
    raw: Counters,
    suspect: [bool; GATE_COLS],
    held: [bool; GATE_COLS],
    split: bool,
    trusted: bool,
) -> GateOutcome {
    apply_gate_seeded(prev, raw, suspect, held, split, trusted, None)
}

/// [`apply_gate_held`] for a capture that opens a game together with an
/// earlier board of that same game.
///
/// `seed` is that board (the fresh-match board the boundary plan held off
/// the previous game and now writes onto the new one) and how long before
/// this capture it was read. It is a complete board: every column on it was
/// a sure read. On a first capture or a split, a held column takes the
/// seed's value instead of the placeholder 0, as an [`HoldKind::Unread`]
/// hold, and is not unread: the game has a read of it, on the row stored
/// just before this one. Without the seed that row would show a number and
/// this one a 0 under it. The seed does nothing on any other capture, and
/// nothing for a column that was read.
pub fn apply_gate_seeded(
    prev: Option<(GateState, Duration)>,
    raw: Counters,
    mut suspect: [bool; GATE_COLS],
    mut held: [bool; GATE_COLS],
    split: bool,
    trusted: bool,
    seed: Option<(Counters, Duration)>,
) -> GateOutcome {
    let already_unread = held.iter().all(|&cell| cell);
    for col in 0..GATE_COLS {
        suspect[col] |= held[col];
    }
    // Every cell is suspect: another player's row, or ink on every cell.
    // None of these numbers may seed or raise the monotonic baseline, and
    // none are stored. An unread cell keeps the last accepted value, or
    // stays a placeholder when the game has no row yet. The raw counters
    // stay on `boundary_anchor` so the reset baseline can still move.
    let discard_row = suspect.iter().all(|&cell| cell);
    if discard_row {
        held = [true; GATE_COLS];
    }
    let boundary_anchor = (discard_row && !already_unread).then_some(raw);
    let restart = trusted && prev.is_some_and(|(state, _)| state.only_low_trust_reads());
    let Some((state, elapsed)) = prev.filter(|_| !split && !restart) else {
        let read = raw.to_array();
        let mut first = read;
        let mut unread = held;
        let mut held_secs = [0u32; GATE_COLS];
        let mut holds = Vec::new();
        for col in 0..GATE_COLS {
            if !held[col] {
                continue;
            }
            first[col] = 0;
            if let Some((board, age)) = seed {
                first[col] = board.to_array()[col];
                unread[col] = false;
                held_secs[col] = secs_u32(age);
                holds.push(Hold {
                    col,
                    kind: HoldKind::Unread,
                    raw: read[col],
                    held: first[col],
                });
            }
        }
        let accepted = Counters::from_array(first);
        let mut state = fresh_state(accepted, suspect, trusted);
        state.unread = unread;
        state.held_secs = held_secs;
        if seed.is_some() {
            // A seeded column's previous raw read is the seed's, a sure read.
            for (flag, &cell) in state.last_raw_suspect.iter_mut().zip(&held) {
                if cell {
                    *flag = !trusted;
                }
            }
        }
        state.boundary_anchor = boundary_anchor;
        return GateOutcome {
            accepted,
            state,
            holds,
            unlatches: Vec::new(),
        };
    };

    let prev_acc = state.accepted.to_array();
    let prev_raw = state.last_raw.to_array();
    let prev_raw_suspect = state.last_raw_suspect;
    let read = raw.to_array();
    // A held column reads as "no change", so no other column's check sees a
    // move in it. The first read of an unread column has no previous value:
    // `jump_prev` takes the read itself for that column, for the same reason.
    let mut cur = read;
    let mut jump_prev = prev_acc;
    let mut first_read = [false; GATE_COLS];
    let mut unread = state.unread;
    let mut refused_first_read = state.refused_first_read;
    for col in 0..GATE_COLS {
        if held[col] {
            cur[col] = prev_acc[col];
        } else if state.unread[col] {
            first_read[col] = true;
            jump_prev[col] = cur[col];
            unread[col] = false;
            refused_first_read[col] = false;
        }
    }
    // A column's accepted value is as old as its last accepted read. After a
    // run of held captures that is longer than `elapsed` (F266-3).
    let col_elapsed = |col: usize| elapsed + Duration::from_secs(u64::from(state.held_secs[col]));
    // The single-field jump check of `col`. Another column that is back
    // from a held run moved over that whole run, not on this capture: its
    // rise is neither a second jump nor a busy board for `col`.
    let lone_jump = |col: usize| {
        let mut before = jump_prev;
        for other in 0..GATE_COLS {
            if other != col && state.held_secs[other] > 0 {
                before[other] = cur[other];
            }
        }
        sudden_single_field_jump(col, before, cur)
    };
    let mut out = cur;
    let mut down_len = state.down_streak_len;
    let mut down_last = state.down_streak_last;
    let mut unconfirmed = state.unconfirmed;
    let mut confirmed_floor = state.confirmed_floor;
    let mut has_confirmed_floor = state.has_confirmed_floor;
    let mut row_shift = state.row_shift;
    let mut holds = Vec::new();
    let mut unlatches = Vec::new();
    // A fully clean per-cell read may replace unconfirmed columns. Confirmed
    // columns, and every increase, still go through B, C, and B2 below.
    let replacing = state.low_trust && trusted && !suspect.iter().any(|&s| s);

    for col in 0..GATE_COLS {
        if held[col] {
            // No number for this column on this capture. Streaks stay.
            holds.push(Hold {
                col,
                kind: HoldKind::Unread,
                raw: read[col],
                held: prev_acc[col],
            });
            continue;
        }
        if first_read[col] {
            // Nothing was accepted for this column yet: store the read.
            down_len[col] = 0;
            down_last[col] = 0;
            continue;
        }
        // A confirmed value stays. A low-trust column yields to one clean
        // read even when that read is lower. A row-shift high still waits
        // for the same lower number to repeat.
        let clean_board = trusted && !suspect.iter().any(|&flag| flag);
        let clean_match = clean_board && !prev_raw_suspect[col] && prev_raw[col] == cur[col];
        let replace_low = clean_board && unconfirmed[col] && cur[col] < prev_acc[col];
        let revise_down = replace_low || (row_shift[col] && clean_match);
        if cur[col] < prev_acc[col] && revise_down {
            // Below the value a fallback moved off a confirmed column, or a
            // wide column that is just the latched number with its last
            // digit cut off. Both are clips. Fall through to the monotonic
            // hold instead of storing them as the new trusted value.
            let below_floor = has_confirmed_floor[col] && cur[col] < confirmed_floor[col];
            let wide_clip = !is_kill_col(col)
                && is_trailing_digit_inject(cur[col], prev_acc[col], col_elapsed(col));
            if !below_floor && !wide_clip {
                unlatches.push(Unlatch {
                    col,
                    raw: cur[col],
                    revised_from: prev_acc[col],
                    replaced_unconfirmed: true,
                });
                out[col] = cur[col];
                down_len[col] = 0;
                down_last[col] = 0;
                continue;
            }
        }
        if cur[col] < prev_acc[col] {
            // Cumulative counters do not fall inside one game. Keep the
            // accepted value. A repeated lower read does not revise it down.
            down_len[col] = 0;
            down_last[col] = 0;
            out[col] = prev_acc[col];
            holds.push(Hold {
                col,
                kind: HoldKind::Monotonic,
                raw: cur[col],
                held: prev_acc[col],
            });
        } else {
            // At or above the accepted value → no active decrease run.
            down_len[col] = 0;
            down_last[col] = 0;

            // (CG-4 B2) Trailing-digit inject on wide cols only — kill cols
            // already have a tight rate cap (and 9→91 would otherwise re-label
            // as DigitInject). Hold before rate/corroboration so a repeated
            // inject cannot corroborate itself past the guard.
            if !is_kill_col(col)
                && cur[col] > prev_acc[col]
                && is_trailing_digit_inject(prev_acc[col], cur[col], col_elapsed(col))
            {
                out[col] = prev_acc[col];
                holds.push(Hold {
                    col,
                    kind: HoldKind::DigitInject,
                    raw: cur[col],
                    held: prev_acc[col],
                });
                continue;
            }

            let ceiling =
                prev_acc[col].saturating_add(max_delta(col, col_elapsed(col), suspect[col]));
            // A suspect prior read must never corroborate the current jump.
            let corroborated = !prev_raw_suspect[col] && corroborates(prev_raw[col], cur[col]);
            if cur[col] > ceiling && !corroborated {
                // (C / CG-4 B1) implausible jump, uncorroborated → hold last accepted.
                out[col] = prev_acc[col];
                holds.push(Hold {
                    col,
                    kind: HoldKind::RateCap,
                    raw: cur[col],
                    held: prev_acc[col],
                });
            } else if !corroborated && lone_jump(col) {
                // One column jumped past JUMP_MAX. Keep the old value until
                // the next capture matches.
                out[col] = prev_acc[col];
                holds.push(Hold {
                    col,
                    kind: HoldKind::Jump,
                    raw: cur[col],
                    held: prev_acc[col],
                });
            }
            // else: plausible advance (or corroborated jump) → keep cur[col].
        }
    }

    // The first read of a never-read column is not taken on a capture where
    // another column dropped. Nothing vouches for which game that board is
    // from: with the game end missed, it is the next game's first board, and
    // its number would land unflagged in this game's column (F266-2). The
    // column stays unread, so the game still has no complete row.
    let mut refused = [false; GATE_COLS];
    if holds.iter().any(|hold| hold.kind == HoldKind::Monotonic) {
        for col in 0..GATE_COLS {
            if !first_read[col] {
                continue;
            }
            first_read[col] = false;
            refused[col] = true;
            unread[col] = true;
            refused_first_read[col] = true;
            out[col] = prev_acc[col];
            holds.push(Hold {
                col,
                kind: HoldKind::Unread,
                raw: cur[col],
                held: prev_acc[col],
            });
        }
    }
    // No number for the column on this capture.
    let no_read = |col: usize| held[col] || refused[col];

    // A kill column that rose past JUMP_MAX is a row shift when it sits in
    // the same adjacent run of cells as a column held down this frame: the
    // next cell, or the next risen cell along that run. A drop on the far
    // side of the board does not tag it. Keep the tag while any drop is
    // still held, so two later reads of the real row can replace the high.
    let decreased = holds.iter().any(|h| h.kind == HoldKind::Monotonic);
    let dropped: Vec<usize> = holds
        .iter()
        .filter(|hold| hold.kind == HoldKind::Monotonic)
        .map(|hold| hold.col)
        .collect();
    let rose = |col: usize| {
        is_kill_col(col)
            && !first_read[col]
            && out[col] > prev_acc[col]
            && cur[col].saturating_sub(prev_acc[col]) > JUMP_MAX[col]
    };
    let mut beside_drop = [false; GATE_COLS];
    let mut grew = true;
    while grew {
        grew = false;
        for col in 0..GATE_COLS {
            if beside_drop[col] || !rose(col) {
                continue;
            }
            let touches = (0..GATE_COLS).any(|other| {
                other.abs_diff(col) == 1 && (dropped.contains(&other) || beside_drop[other])
            });
            if touches {
                beside_drop[col] = true;
                grew = true;
            }
        }
    }
    for col in 0..GATE_COLS {
        if no_read(col) {
            // No read: the tag is carried as it was.
            continue;
        }
        if beside_drop[col] {
            row_shift[col] = true;
        } else if out[col] < prev_acc[col] {
            row_shift[col] = false;
        } else if decreased && row_shift[col] && out[col] == prev_acc[col] {
            row_shift[col] = true;
        } else if !decreased {
            row_shift[col] = false;
        }
    }

    let accepted = Counters::from_array(out);
    // Adopting a fallback number marks that column unconfirmed. If the
    // column was confirmed, keep that value as a floor so a later clip
    // cannot replace it.
    if !trusted {
        for col in 0..GATE_COLS {
            if first_read[col] {
                // The placeholder under a first read is not a confirmed
                // value, so it leaves no floor.
                unconfirmed[col] = true;
                continue;
            }
            if out[col] == cur[col] && out[col] != prev_acc[col] {
                if !unconfirmed[col] {
                    confirmed_floor[col] = prev_acc[col];
                    has_confirmed_floor[col] = true;
                }
                unconfirmed[col] = true;
            }
        }
    }
    if replacing {
        for col in 0..GATE_COLS {
            if out[col] != cur[col] {
                continue;
            }
            // A ghost under an implausible kill latch is stored, but the
            // column stays unconfirmed until a non-suspect read already
            // agreed with it, or this clean read replaced the latch.
            // Otherwise B would lock the ghost in. Plausible columns,
            // including a fallback assists of 40, still confirm on this read.
            let over = kill_col_over_ceiling(col, prev_acc[col]);
            let agrees = !prev_raw_suspect[col] && cur[col] == prev_raw[col];
            let replaced = unlatches
                .iter()
                .any(|unlatch| unlatch.col == col && unlatch.replaced_unconfirmed);
            if over && !agrees && !replaced {
                continue;
            }
            unconfirmed[col] = false;
            has_confirmed_floor[col] = false;
            confirmed_floor[col] = 0;
        }
    }
    // A 0.4.20 save has no low_trust bit, so a shifted latch loads as
    // trusted. Keeping the implausible number marks those columns so the
    // next clean read can replace them. Wide columns are not marked; a
    // shifted one stays where it is: a confirmed column does not decrease.
    mark_implausible(&mut unconfirmed, accepted);
    let low_trust = unconfirmed.iter().any(|&c| c) || (state.low_trust && !holds.is_empty());
    // A held column was not read: its previous raw read and that read's
    // suspect bit stand for the next capture.
    let mut last_raw = read;
    let mut last_raw_suspect = raw_suspect_mask(suspect, trusted);
    let mut held_secs = [0u32; GATE_COLS];
    for col in 0..GATE_COLS {
        if no_read(col) {
            last_raw[col] = prev_raw[col];
            last_raw_suspect[col] = prev_raw_suspect[col];
        }
        // The accepted value gets older while the column is held, and while
        // a gate hold keeps a read out after such a run. A column with no
        // held run behind it keeps the capture interval, as under ocr-v1.
        let kept_out = out[col] != cur[col];
        if !unread[col] && (held[col] || (kept_out && state.held_secs[col] > 0)) {
            held_secs[col] = state.held_secs[col].saturating_add(secs_u32(elapsed));
        }
    }
    GateOutcome {
        accepted,
        state: GateState {
            accepted,
            last_raw: Counters::from_array(last_raw),
            down_streak_len: down_len,
            down_streak_last: down_last,
            last_raw_suspect,
            low_trust,
            unconfirmed,
            confirmed_floor,
            has_confirmed_floor,
            row_shift,
            unread,
            held_secs,
            refused_first_read,
            // Set by the capture path after the gate, not here.
            unsure_shown: [0; GATE_COLS],
            has_unsure_shown: [false; GATE_COLS],
            boundary_anchor,
        },
        holds,
        unlatches,
    }
}
