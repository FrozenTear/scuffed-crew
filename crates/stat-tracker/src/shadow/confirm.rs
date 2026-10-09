//! Frame-to-frame confirmation of unsure cells, for the shadow log only.
//!
//! Scoreboard stats only grow during a match. When the frames just before and
//! just after an unsure cell both read that cell as the same value, with sure
//! (non-suspect) reads, the frame between must hold that value too. A cell is
//! confirmed only when its own reading already is that value: confirmation
//! never changes a value, never touches the matcher's `suspect` flag, and
//! cannot turn a wrong read into a confirmed one unless both neighbours were
//! wrong the same way with sure reads.
//!
//! This cannot be scored on single screenshots. It is unit-tested on
//! synthetic sequences and needs validation on live shadow logs.

use chrono::{DateTime, TimeDelta, Utc};

use super::digits::BoardRead;

/// Neighbouring frames further apart than this confirm nothing.
pub const MAX_SPAN: TimeDelta = TimeDelta::seconds(120);

/// One matcher read with the context needed to line it up with its neighbours.
#[derive(Debug, Clone)]
pub struct Frame {
    pub session: String,
    pub at: DateTime<Utc>,
    pub team_size: usize,
    pub board: BoardRead,
}

impl Frame {
    /// Value of a sure (non-suspect) read at `row`, `field`.
    fn sure(&self, row: usize, field: usize) -> Option<u32> {
        let c = &self.board.rows.get(row)?.cells[field];
        if c.suspect { None } else { c.value }
    }

    fn same_layout(&self, other: &Frame) -> bool {
        self.session == other.session
            && self.team_size == other.team_size
            && self.board.rows.len() == other.board.rows.len()
    }
}

/// Unsure cells of `cur`, as `(row, field)`, that `prev` and `next` confirm.
///
/// All three frames must come from one session with the same layout, in time
/// order, with `next` at most [`MAX_SPAN`] after `prev`.
pub fn confirmed(prev: &Frame, cur: &Frame, next: &Frame) -> Vec<(usize, usize)> {
    let ordered = prev.at <= cur.at && cur.at <= next.at;
    if !prev.same_layout(cur) || !cur.same_layout(next) || !ordered || next.at - prev.at > MAX_SPAN
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (r, row) in cur.board.rows.iter().enumerate() {
        for (k, c) in row.cells.iter().enumerate() {
            let Some(v) = c.value else { continue };
            if c.suspect && prev.sure(r, k) == Some(v) && next.sure(r, k) == Some(v) {
                out.push((r, k));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shadow::digits::{CellRead, RowRead};

    /// One cell per entry: (value, suspect); unlisted cells are sure 1s.
    fn frame(session: &str, secs: i64, cells: &[(usize, usize, Option<u32>, bool)]) -> Frame {
        let mut rows: Vec<RowRead> = (0..10)
            .map(|_| RowRead {
                cells: std::array::from_fn(|_| CellRead {
                    value: Some(1),
                    confidence: 0.9,
                    suspect: false,
                }),
            })
            .collect();
        for &(r, k, value, suspect) in cells {
            rows[r].cells[k] = CellRead {
                value,
                confidence: if suspect { 0.3 } else { 0.9 },
                suspect,
            };
        }
        Frame {
            session: session.into(),
            at: DateTime::from_timestamp(1_760_000_000 + secs, 0).unwrap(),
            team_size: 5,
            board: BoardRead {
                rows,
                elapsed_ms: 10,
            },
        }
    }

    #[test]
    fn unsure_zero_between_two_sure_zeros_is_confirmed() {
        let p = frame("s", 0, &[(9, 2, Some(0), false)]);
        let c = frame("s", 5, &[(9, 2, Some(0), true)]);
        let n = frame("s", 10, &[(9, 2, Some(0), false)]);
        assert_eq!(confirmed(&p, &c, &n), vec![(9, 2)]);
    }

    #[test]
    fn needs_both_neighbours_sure_and_equal() {
        let c = frame("s", 5, &[(9, 2, Some(0), true)]);
        // the stat went up after this frame: the 0 may already be a 3
        let p = frame("s", 0, &[(9, 2, Some(0), false)]);
        let n = frame("s", 10, &[(9, 2, Some(3), false)]);
        assert!(confirmed(&p, &c, &n).is_empty());
        // an unsure neighbour confirms nothing
        let n = frame("s", 10, &[(9, 2, Some(0), true)]);
        assert!(confirmed(&p, &c, &n).is_empty());
        let p2 = frame("s", 0, &[(9, 2, Some(0), true)]);
        let n2 = frame("s", 10, &[(9, 2, Some(0), false)]);
        assert!(confirmed(&p2, &c, &n2).is_empty());
        // an empty neighbour cell confirms nothing
        let p3 = frame("s", 0, &[(9, 2, None, false)]);
        assert!(confirmed(&p3, &c, &n2).is_empty());
    }

    #[test]
    fn never_confirms_a_value_the_neighbours_disagree_with() {
        // neighbours pin the cell at 4; the unsure read says 6: left alone, not changed
        let p = frame("s", 0, &[(3, 1, Some(4), false)]);
        let c = frame("s", 5, &[(3, 1, Some(6), true)]);
        let n = frame("s", 10, &[(3, 1, Some(4), false)]);
        assert!(confirmed(&p, &c, &n).is_empty());
        assert_eq!(c.board.rows[3].cells[1].value, Some(6));
    }

    #[test]
    fn only_unsure_cells_are_listed() {
        let p = frame("s", 0, &[]);
        let c = frame("s", 5, &[(0, 0, Some(1), true), (4, 5, Some(1), true)]);
        let n = frame("s", 10, &[]);
        assert_eq!(confirmed(&p, &c, &n), vec![(0, 0), (4, 5)]);
        let quiet = frame("s", 5, &[]);
        assert!(confirmed(&p, &quiet, &n).is_empty());
    }

    #[test]
    fn frames_must_line_up() {
        let p = frame("s", 0, &[(9, 2, Some(0), false)]);
        let c = frame("s", 5, &[(9, 2, Some(0), true)]);
        let n = frame("s", 10, &[(9, 2, Some(0), false)]);
        // another session (a new match)
        let other = frame("t", 10, &[(9, 2, Some(0), false)]);
        assert!(confirmed(&p, &c, &other).is_empty());
        // another team size or row count
        let mut six = n.clone();
        six.team_size = 6;
        assert!(confirmed(&p, &c, &six).is_empty());
        let mut short = n.clone();
        short.board.rows.pop();
        assert!(confirmed(&p, &c, &short).is_empty());
        // out of order
        assert!(confirmed(&n, &c, &p).is_empty());
        // too far apart
        let late = frame("s", 121, &[(9, 2, Some(0), false)]);
        assert!(confirmed(&p, &c, &late).is_empty());
        let edge = frame("s", 120, &[(9, 2, Some(0), false)]);
        assert_eq!(confirmed(&p, &c, &edge), vec![(9, 2)]);
    }

    /// A synthetic match: stats grow every few frames, the matcher is unsure
    /// on some frames, right or wrong. Only right unsure reads may be
    /// confirmed, and a stat that changes next to an unsure frame is never
    /// confirmed even when the unsure read happens to be right.
    #[test]
    fn synthetic_match_confirms_only_right_unsure_reads() {
        // truth per frame for one cell (deaths), 5 s apart
        let truth = [0u32, 0, 0, 0, 1, 1, 1, 3, 3, 3, 3, 4];
        // (frame, read value) where the matcher is unsure
        let unsure: &[(usize, u32)] = &[(1, 0), (3, 3), (5, 1), (7, 3), (9, 0), (11, 4)];
        let frames: Vec<Frame> = truth
            .iter()
            .enumerate()
            .map(|(i, &t)| {
                let cell = match unsure.iter().find(|u| u.0 == i) {
                    Some(&(_, v)) => (9, 2, Some(v), true),
                    None => (9, 2, Some(t), false),
                };
                frame("m", 5 * i as i64, &[cell])
            })
            .collect();
        let mut got = Vec::new();
        for i in 1..frames.len() - 1 {
            if !confirmed(&frames[i - 1], &frames[i], &frames[i + 1]).is_empty() {
                got.push(i);
            }
        }
        // 1 and 5: right, between two sure reads of the same value.
        // 3: wrong 3 between a sure 0 and a sure 1. 7: right, but the stat
        // changed around it (sure 1, then sure 3). 9: wrong 0 between sure 3s.
        // 11: last frame, no next.
        assert_eq!(got, vec![1, 5]);
        for &i in &got {
            assert_eq!(frames[i].board.rows[9].cells[2].value, Some(truth[i]));
        }
    }
}
