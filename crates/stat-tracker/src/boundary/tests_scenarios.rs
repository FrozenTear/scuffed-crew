//! Poll and Tab sequences replayed through the `Machine` simulator
//! (Busan → Junkertown, resets, hints, gaps).

use super::tests::{CLEAN, counters, gate, t0};
use super::*;
use crate::capture_gate::apply_gate;

/// Poll + Tab stand-in. State changes go through [`commit_poll`],
/// [`plan_capture`], and [`note_accepted_capture`] — the functions `main` calls.
struct Machine {
    active: Option<Sess>,
    closed: Vec<Closed>,
    n: u32,
}

struct Closed {
    sess: Sess,
    reason: CloseReason,
}

#[derive(Clone)]
struct Sess {
    id: String,
    state: BoundaryState,
    /// First board of this session after a split. Later tabs replace the gate.
    opened_with: Option<Counters>,
}

impl Sess {
    fn outcome(&self) -> MatchOutcome {
        self.state.outcome
    }
    fn map(&self) -> Option<&str> {
        self.state.map.as_deref()
    }
}
impl Machine {
    fn new(map: &str) -> Self {
        Self {
            active: Some(Sess {
                id: "busan".into(),
                state: BoundaryState::new(Some(map.into())),
                opened_with: None,
            }),
            closed: Vec::new(),
            n: 0,
        }
    }

    fn active(&self) -> &Sess {
        self.active.as_ref().expect("active session")
    }

    fn poll(&mut self, mut build: impl for<'a> FnMut(&'a BoundaryState) -> PollInput<'a>) {
        let Some(sess) = self.active.as_mut() else {
            return;
        };
        let (decision, now) = {
            let input = build(&sess.state);
            (decide_poll(&input), input.now)
        };
        let mut previous = sess.state.clone();
        let commit = commit_poll(&mut sess.state, decision, now);
        if let Some(closed) = commit.closed {
            if let Some(outcome) = closed.seal {
                previous.outcome = outcome;
                previous.outcome_at = Some(now);
            }
            self.n += 1;
            let id = format!("s{}", self.n);
            let prev = Sess {
                id: std::mem::replace(&mut sess.id, id),
                state: previous,
                opened_with: std::mem::take(&mut sess.opened_with),
            };
            self.closed.push(Closed {
                sess: prev,
                reason: closed.reason,
            });
        }
    }

    fn capture(&mut self, cur: Counters, now: Instant) {
        self.capture_board(cur, None, Some(0), now);
    }

    fn capture_board(
        &mut self,
        cur: Counters,
        hero: Option<&str>,
        row_id: Option<u32>,
        now: Instant,
    ) {
        let sess = self.active.as_mut().expect("active");
        let age = sess
            .state
            .last_board_at
            .map(|at| now.saturating_duration_since(at))
            .unwrap_or(Duration::from_secs(30));
        let baseline_age = sess
            .state
            .baseline_at
            .map(|at| now.saturating_duration_since(at));
        let classic = sess.state.gate.as_ref().is_some_and(|gate| {
            age >= Duration::from_secs(120) && post_result_stat_reset(gate, cur, CLEAN)
        });
        let plan = plan_capture(&CapturePlanInput {
            prev_gate: sess.state.gate.as_ref(),
            baseline: sess.state.reset_baseline.as_ref(),
            streak: sess.state.reset_streak,
            cur,
            suspect: CLEAN,
            create_session: false,
            // Same map and hero stay suppressed. A capture that names no
            // other map is this match; the night harness is the path
            // where a different map leaves the gap on.
            suppress_same_unfinished: sess.state.map.is_some(),
            age: sess.state.gate.map(|_| age),
            min_gap: Duration::from_secs(120),
            classic_regressed: classic,
            row_counts: row_id.is_some(),
            row_id,
            baseline_row: sess.state.baseline_row,
            confirmed_end: sess.state.outcome.is_decided(),
            inherited_outcome: sess.state.outcome,
            frame_outcome: MatchOutcome::Unknown,
            session_hero: sess.state.hero.as_deref(),
            hint: sess
                .state
                .result
                .filter(|mark| !mark.confirmed && mark.outcome.is_decided())
                .map(|mark| mark.outcome),
            pending_boundary: sess.state.pending_boundary,
            awaiting_first_board: sess.state.awaiting_first_board,
            baseline_age,
            progressed_boards: sess.state.progressed_boards,
            session_map: sess.state.map.as_deref(),
            incoming_map: sess.state.map.as_deref(),
            session_map_source: None,
        });
        if plan.split {
            let held = sess.state.deferred;
            let new_outcome = plan.stored_outcome;
            let mut previous = sess.state.clone();
            let commit = commit_poll(
                &mut sess.state,
                PollDecision::Open(OpenNew {
                    reason: plan.close_reason.unwrap_or(CloseReason::StatReset),
                    seal_outcome: plan.seal,
                    new_outcome,
                    new_map: None,
                    candidates: Vec::new(),
                    new_result: None,
                }),
                now,
            );
            let closed = commit.closed.expect("split closes the session");
            if let Some(outcome) = closed.seal {
                previous.outcome = outcome;
                previous.outcome_at = Some(now);
            }
            self.n += 1;
            let id = format!("s{}", self.n);
            let prev = Sess {
                id: std::mem::replace(&mut sess.id, id),
                state: previous,
                opened_with: std::mem::take(&mut sess.opened_with),
            };
            self.closed.push(Closed {
                sess: prev,
                reason: closed.reason,
            });
            sess.state.outcome = new_outcome;
            sess.state.outcome_at = new_outcome.is_decided().then_some(now);
            sess.state.gate = Some(gate(cur));
            sess.state.last_board_at = Some(now);
            sess.state.hero = hero.map(str::to_string);
            sess.state.awaiting_first_board = false;
            sess.opened_with = held.or(Some(cur));
            return;
        }
        let accepted = match sess.state.gate {
            Some(prev) => apply_gate(Some((prev, age)), cur, CLEAN, false).state,
            None => gate(cur),
        };
        note_accepted_capture(&mut sess.state, &plan, accepted, now);
        if let Some(hero) = hero
            && !plan.ignore_row
            && !plan.defer
        {
            sess.state.hero = Some(hero.to_string());
        }
    }
}
fn confirm_defeat(m: &mut Machine, now: Instant) {
    // First word is a streak; the agreeing read confirms.
    m.poll(|s| {
        let mut i = poll_of(s, now);
        i.signal = Some(MatchOutcome::Defeat);
        i.signal_confirmed = false;
        i
    });
    assert!(
        !m.active().outcome().is_decided(),
        "a single defeat word is a streak, not a stored outcome"
    );
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(4));
        i.signal = Some(MatchOutcome::Defeat);
        i.signal_confirmed = true;
        i
    });
    assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
    assert_eq!(m.active().map(), Some("Busan"));
}

fn poll_of(s: &BoundaryState, now: Instant) -> PollInput<'_> {
    PollInput {
        outcome: s.outcome,
        result: s.result,
        pending_boundary: s.pending_boundary,
        awaiting_first_board: s.awaiting_first_board,
        has_board: s.gate.is_some(),
        reset_streak: s.reset_streak,
        map: s.map.as_deref(),
        map_trusted: true,
        hero: s.hero.as_deref(),
        signal: None,
        signal_confirmed: false,
        accolade_map: None,
        start_screen: None,
        block_map_vote: false,
        deferred: s.deferred.is_some(),
        text_fallback_locked: s.text_fallback_locked,
        now,
    }
}

#[test]
fn busan_defeat_then_hero_select_then_junkertown_victory() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.capture(
        counters(14, 22, 6, 2400, 9800, 400),
        now + Duration::from_secs(60),
    );
    confirm_defeat(&mut m, now + Duration::from_secs(8 * 60));

    // Fast requeue: no map-vote frame. Hero select closes Busan.
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(8 * 60 + 20));
        i.start_screen = Some(StartScreen::HeroSelect);
        i
    });
    assert_eq!(m.closed.len(), 1);
    assert_eq!(m.closed[0].reason, CloseReason::HeroSelectOrBan);
    assert_eq!(m.closed[0].sess.map(), Some("Busan"));
    assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Defeat);
    assert_eq!(m.active().id, "s1");
    assert!(m.active().map().is_none());

    m.capture(
        counters(2, 1, 0, 800, 200, 1500),
        now + Duration::from_secs(9 * 60),
    );
    assert_eq!(
        m.active().id,
        "s1",
        "junkertown tabs stay on the new session"
    );

    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(19 * 60));
        i.signal = Some(MatchOutcome::Victory);
        i.signal_confirmed = true;
        i.accolade_map = Some("Junkertown");
        i
    });
    // Second agreeing tick is what the poller calls confirmed; the first
    // word only marks the streak. Drive the confirm the way the daemon
    // does: one confirmed signal on a session that has no prior result.
    assert_eq!(m.active().outcome(), MatchOutcome::Victory);
    assert_eq!(m.active().map(), Some("Junkertown"));
    assert_eq!(m.closed.len(), 1);
    assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Defeat);
}

#[test]
fn busan_defeat_then_stat_reset_without_map_vote() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.capture(
        counters(18, 7, 9, 6400, 11000, 800),
        now + Duration::from_secs(60),
    );
    confirm_defeat(&mut m, now + Duration::from_secs(8 * 60));

    // First Junkertown board. Same-match suppress is true and the age is
    // 10s. One drop only arms the streak — it must not inherit nothing yet,
    // and it must not open a session.
    let first = counters(1, 3, 0, 220, 80, 400);
    m.capture(first, now + Duration::from_secs(8 * 60 + 25));
    assert!(
        m.closed.is_empty(),
        "one post-result drop does not open a session"
    );
    assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
    assert_eq!(m.active().state.reset_streak, 1);
    assert_eq!(
        m.active().state.gate.map(|g| g.accepted.elims),
        Some(18),
        "the first drop is not written onto the finished game"
    );

    // Second consecutive drop. The new session keeps the raw counters and
    // none of Busan's outcome, hint, or grace.
    m.capture(
        counters(2, 4, 1, 400, 120, 700),
        now + Duration::from_secs(8 * 60 + 40),
    );
    assert_eq!(m.closed.len(), 1, "the second drop opens the session");
    assert_eq!(m.closed[0].reason, CloseReason::StatReset);
    assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Defeat);
    assert_eq!(m.closed[0].sess.map(), Some("Busan"));
    assert!(
        !m.active().outcome().is_decided(),
        "the junkertown session must not be stored as Busan's defeat"
    );
    assert!(m.active().state.outcome_at.is_none());
    assert!(m.active().state.result.is_none());
    assert_eq!(
        m.active().state.gate.map(|g| g.accepted.elims),
        Some(2),
        "the new session keeps the raw reset, it does not hold Busan's elims"
    );

    m.capture(
        counters(4, 5, 1, 900, 300, 2200),
        now + Duration::from_secs(12 * 60),
    );
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(19 * 60));
        i.signal = Some(MatchOutcome::Victory);
        i.signal_confirmed = true;
        i.accolade_map = Some("Junkertown");
        i
    });
    assert_eq!(m.active().outcome(), MatchOutcome::Victory);
    assert_eq!(m.active().map(), Some("Junkertown"));
    assert_eq!(m.closed.len(), 1);
}

#[test]
fn hero_select_after_a_board_arms_and_the_next_board_drops_the_hint() {
    // Live shape: one defeat word (`poll_streak_defeat`), queue pops
    // before the agreeing read, hero select with no map vote.
    let now = t0();
    let mut m = Machine::new("Busan");
    m.capture(counters(11, 20, 5, 2100, 8000, 100), now);
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(30));
        i.signal = Some(MatchOutcome::Defeat);
        i.signal_confirmed = false;
        i
    });
    assert!(!m.active().outcome().is_decided());
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(45));
        i.start_screen = Some(StartScreen::HeroSelect);
        i
    });
    assert!(
        m.closed.is_empty(),
        "a hero select after a board-followed hint arms, it does not seal"
    );
    assert!(m.active().state.pending_boundary);
    assert_eq!(
        m.active().state.result.map(|mark| mark.outcome),
        Some(MatchOutcome::Defeat)
    );
    assert!(!m.active().outcome().is_decided());
    m.capture(
        counters(14, 22, 6, 2600, 9000, 200),
        now + Duration::from_secs(70),
    );
    assert!(
        m.active().state.result.is_none(),
        "a progressed board after the armed start screen drops the hint"
    );
    assert!(!m.active().state.pending_boundary);
    assert!(m.closed.is_empty());
}

#[test]
fn hero_ban_is_a_boundary_after_a_result() {
    let now = t0();
    let mut m = Machine::new("Busan");
    confirm_defeat(&mut m, now);
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(15));
        i.start_screen = Some(StartScreen::HeroBan);
        i
    });
    assert_eq!(m.closed.len(), 1);
    assert_eq!(m.closed[0].reason, CloseReason::HeroSelectOrBan);
    assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Defeat);
}

#[test]
fn long_post_match_screen_does_not_split_on_time_alone() {
    let now = t0();
    let mut m = Machine::new("Busan");
    confirm_defeat(&mut m, now);
    // Accolade / rank still up. The clock passed two minutes and the map
    // OCR disagrees, but nothing has left this post-match flow.
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(120) + Duration::from_secs(30));
        i.signal = Some(MatchOutcome::Defeat);
        i.signal_confirmed = true;
        i.accolade_map = Some("Junkertown");
        i
    });
    assert!(m.closed.is_empty());
    assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
    assert_eq!(m.active().map(), Some("Busan"));
    // A contradictory word with only that clock gap is ignored too.
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(120) + Duration::from_secs(40));
        i.signal = Some(MatchOutcome::Victory);
        i.signal_confirmed = true;
        i
    });
    assert!(m.closed.is_empty());
    assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
    assert_eq!(m.active().map(), Some("Busan"));
}

#[test]
fn contradictory_result_without_a_gap_does_not_overwrite() {
    let now = t0();
    let mut m = Machine::new("Busan");
    confirm_defeat(&mut m, now);
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(20));
        i.signal = Some(MatchOutcome::Victory);
        i.signal_confirmed = true;
        i.accolade_map = Some("Junkertown");
        i
    });
    assert!(m.closed.is_empty());
    assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
    assert_eq!(
        m.active().map(),
        Some("Busan"),
        "a same-screen map read must not replace the session map"
    );
}

#[test]
fn rising_scoreboard_between_results_does_not_split() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.capture(counters(10, 4, 3, 3000, 1000, 0), now);
    confirm_defeat(&mut m, now + Duration::from_secs(30));
    // Same match, stats still climbing. That tab is not a new game, and
    // a later contradictory word must not split off a second finished session.
    m.capture(
        counters(12, 5, 3, 3400, 1400, 0),
        now + Duration::from_secs(40),
    );
    assert!(m.closed.is_empty());
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(70));
        i.signal = Some(MatchOutcome::Victory);
        i.signal_confirmed = true;
        i.accolade_map = Some("Junkertown");
        i
    });
    assert!(
        m.closed.is_empty(),
        "a rising scoreboard is not a gap that opens another finished game"
    );
    assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
    assert_eq!(m.active().map(), Some("Busan"));
}

#[test]
fn accolade_map_mismatch_after_a_scoreboard_does_not_split() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.capture(counters(10, 4, 3, 3000, 1000, 0), now);
    confirm_defeat(&mut m, now + Duration::from_secs(30));
    // Rank-screen tab: stats did not reset. A misread map on the next
    // accolade must not open a second session.
    m.capture(
        counters(11, 4, 3, 3200, 1100, 0),
        now + Duration::from_secs(50),
    );
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(70));
        i.signal = Some(MatchOutcome::Defeat);
        i.signal_confirmed = true;
        i.accolade_map = Some("Junkertown");
        i
    });
    assert!(
        m.closed.is_empty(),
        "a map misread after a same-match scoreboard does not split"
    );
    assert_eq!(m.active().map(), Some("Busan"));
    assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
}

#[test]
fn end_reel_wake_does_not_split_or_open_a_gap() {
    // `decide_poll` has no end-reel input. The wake only changes poll
    // cadence in `main`. A tick with no new-game screen stays on Busan.
    let now = t0();
    let mut m = Machine::new("Busan");
    confirm_defeat(&mut m, now);
    m.poll(|s| poll_of(s, now + Duration::from_secs(10)));
    assert!(m.closed.is_empty());
    assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
    // A contradictory word still inside the couple-of-minutes window,
    // with only the POTG wake in between, stays on Busan.
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(30));
        i.signal = Some(MatchOutcome::Victory);
        i.signal_confirmed = true;
        i
    });
    assert!(m.closed.is_empty());
    assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
    assert_eq!(m.active().map(), Some("Busan"));
}

#[test]
fn hero_select_mid_match_does_not_open_a_session() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(30));
        i.start_screen = Some(StartScreen::HeroSelect);
        i
    });
    assert!(m.closed.is_empty());
    assert_eq!(m.active().id, "busan");
}

#[test]
fn quarter_drop_on_two_boards_splits_and_does_not_hold_the_old_game() {
    // Deaths and damage at about a quarter, with the rest of the board,
    // is a new game. The first such board is not written onto the old
    // session. The second closes it. The new counters are stored as read.
    // The capture gate must not hold them back onto the old totals.
    let now = t0();
    let mut m = Machine::new("Ilios");
    m.capture(counters(20, 8, 8, 8000, 4000, 2000), now);
    let low = counters(2, 1, 2, 1500, 400, 200);
    let first_at = now + Duration::from_secs(50);
    m.capture(low, first_at);
    assert!(
        m.closed.is_empty(),
        "one reset board does not close the game"
    );
    assert_eq!(m.active().state.gate.unwrap().accepted.deaths, 8);
    assert_eq!(m.active().state.gate.unwrap().accepted.damage, 8000);
    assert_eq!(m.active().state.reset_streak, 1);
    assert!(m.active().state.deferred.is_some());
    let held = apply_gate(
        Some((m.active().state.gate.unwrap(), Duration::from_secs(50))),
        low,
        CLEAN,
        false,
    );
    assert_eq!(held.accepted.deaths, 8, "without a split the drop is held");
    assert_eq!(held.accepted.damage, 8000);
    assert!(!held.holds.is_empty());
    m.capture(
        counters(3, 1, 2, 1600, 500, 250),
        now + Duration::from_secs(110),
    );
    assert_eq!(m.closed.len(), 1);
    assert_eq!(m.closed[0].reason, CloseReason::StatReset);
    assert_eq!(m.closed[0].sess.state.gate.unwrap().accepted.damage, 8000);
    let opened = m
        .active()
        .opened_with
        .expect("new game stores the low board");
    assert_eq!(opened.deaths, 2);
    assert_eq!(opened.damage, 1500);
    assert_eq!(m.active().state.gate.unwrap().accepted.deaths, 2);
    assert_eq!(m.active().state.gate.unwrap().accepted.damage, 1600);
    let fresh = apply_gate(
        Some((
            m.closed[0].sess.state.gate.unwrap(),
            Duration::from_secs(60),
        )),
        counters(3, 1, 2, 1600, 500, 250),
        CLEAN,
        true,
    );
    assert!(fresh.holds.is_empty(), "a split does not hold the new game");
    assert_eq!(fresh.accepted.deaths, 2);
    assert_eq!(fresh.accepted.damage, 1600);
}

/// CAP-8: an unidentified row stored between two own-row fresh boards
/// does not make the second one "too young". The reset age is the
/// baseline's, so the second fresh board commits the split instead of
/// becoming the baseline and merging the two games.
#[test]
fn an_unidentified_row_between_fresh_boards_does_not_merge_the_games() {
    let now = t0();
    let mut m = Machine::new("Ilios");
    m.capture(counters(20, 8, 8, 8000, 4000, 2000), now);
    let first = counters(2, 1, 1, 900, 300, 100);
    m.capture(first, now + Duration::from_secs(50));
    assert_eq!(m.active().state.reset_streak, 1, "the first reset signal");
    assert_eq!(m.active().state.deferred, Some(first));
    // The own row was not found on this Tab. The row is stored.
    m.capture_board(
        counters(2, 1, 1, 950, 320, 110),
        None,
        None,
        now + Duration::from_secs(55),
    );
    assert!(m.closed.is_empty());
    assert_eq!(m.active().state.reset_streak, 1, "the streak stays");
    assert_eq!(m.active().state.deferred, Some(first), "the hold stays");
    // The own row again, 10 s after the unidentified row.
    m.capture(
        counters(3, 1, 1, 1000, 350, 120),
        now + Duration::from_secs(65),
    );
    assert_eq!(m.closed.len(), 1, "the second fresh board splits");
    assert_eq!(m.closed[0].reason, CloseReason::StatReset);
    assert_eq!(
        m.closed[0]
            .sess
            .state
            .reset_baseline
            .map(|g| g.accepted.elims),
        Some(20),
        "the old game kept its mature baseline"
    );
    assert_eq!(
        m.active().opened_with,
        Some(first),
        "the held first board opens the new game"
    );
}

/// CAP-8: a fresh-shaped board on the baseline's own row that is too
/// young to count as a reset is stored, but it is not the baseline. The
/// baseline, its time and the streak stay, and the reset still fires
/// once the boards are old enough.
#[test]
fn a_young_fresh_board_does_not_re_anchor_the_baseline() {
    let now = t0();
    let mut m = Machine::new("Ilios");
    m.capture(counters(20, 8, 8, 8000, 4000, 2000), now);
    assert_eq!(m.active().state.baseline_at, Some(now));
    m.capture(counters(1, 0, 0, 200, 50, 0), now + Duration::from_secs(10));
    assert!(m.closed.is_empty());
    assert_eq!(
        m.active().state.baseline_at,
        Some(now),
        "a fresh-shaped own-row board is not the baseline"
    );
    assert_eq!(
        m.active().state.reset_baseline.map(|g| g.accepted.elims),
        Some(20)
    );
    // 50 s after the baseline, 40 s after the last stored row.
    let first = counters(2, 1, 1, 900, 300, 100);
    m.capture(first, now + Duration::from_secs(50));
    assert!(m.closed.is_empty());
    assert_eq!(m.active().state.reset_streak, 1, "the first reset signal");
    assert_eq!(m.active().state.deferred, Some(first));
    m.capture(
        counters(3, 1, 1, 1000, 350, 120),
        now + Duration::from_secs(60),
    );
    assert_eq!(m.closed.len(), 1, "the second fresh board splits");
    assert_eq!(m.closed[0].reason, CloseReason::StatReset);
}

#[test]
fn one_frame_near_zero_on_three_stats_does_not_split() {
    // Two or three stats near zero is a bad read, not a new game. The
    // old values stay, and those fields are flagged. A split needs the
    // whole board, on two scoreboards.
    let now = t0();
    let mut m = Machine::new("Ilios");
    let high = counters(20, 8, 8, 8000, 4000, 2000);
    m.capture(high, now);
    let dipped = counters(0, 1, 8, 8000, 0, 2000);
    m.capture(dipped, now + Duration::from_secs(50));
    assert!(
        m.closed.is_empty(),
        "one partial drop does not close the game"
    );
    let saved = m.active().state.gate.unwrap();
    assert_eq!(saved.accepted.elims, 20);
    assert_eq!(saved.accepted.assists, 8);
    assert_eq!(saved.accepted.healing, 4000);
    assert_eq!(saved.accepted.deaths, 8);
    assert_eq!(saved.accepted.damage, 8000);
    let held = apply_gate(
        Some((gate(high), Duration::from_secs(50))),
        dipped,
        CLEAN,
        false,
    );
    let flags = crate::capture_gate::unsure_fields(&held.holds);
    assert!(flags.contains(&"e"), "{flags:?}");
    assert!(flags.contains(&"a"), "{flags:?}");
    assert!(flags.contains(&"h"), "{flags:?}");
}

#[test]
fn confirmed_victory_replaces_an_unconfirmed_defeat_hint() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.poll(|s| {
        let mut i = poll_of(s, now);
        i.signal = Some(MatchOutcome::Defeat);
        i.signal_confirmed = false;
        i
    });
    assert!(!m.active().outcome().is_decided());
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(8));
        i.signal = Some(MatchOutcome::Victory);
        i.signal_confirmed = true;
        i
    });
    assert!(m.closed.is_empty(), "the confirm stays on this session");
    assert_eq!(m.active().outcome(), MatchOutcome::Victory);
    assert_eq!(m.active().map(), Some("Busan"));
}

#[test]
fn an_unarmed_trusted_different_map_confirmation_splits() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.capture(counters(10, 4, 3, 3000, 1000, 0), now);
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(20));
        i.signal = Some(MatchOutcome::Defeat);
        i.signal_confirmed = false;
        i
    });
    m.capture(
        counters(12, 5, 3, 3400, 1200, 0),
        now + Duration::from_secs(30),
    );
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(40));
        i.signal = Some(MatchOutcome::Victory);
        i.signal_confirmed = true;
        i.accolade_map = Some("Junkertown");
        i
    });
    assert_eq!(
        m.closed.len(),
        1,
        "both names are trusted and they differ, so this is a boundary even with no arm"
    );
    assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Defeat);
    assert_eq!(m.closed[0].sess.map(), Some("Busan"));
    assert_eq!(m.closed[0].reason, CloseReason::EndScreenMap);
    assert_eq!(m.active().outcome(), MatchOutcome::Victory);
    assert_eq!(m.active().map(), Some("Junkertown"));
}

#[test]
fn time_does_not_drop_a_sealable_hint() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.poll(|s| {
        let mut i = poll_of(s, now);
        i.signal = Some(MatchOutcome::Defeat);
        i.signal_confirmed = false;
        i
    });
    assert!(m.active().state.result.is_some());
    m.poll(|s| poll_of(s, now + Duration::from_secs(180)));
    assert!(
        m.active().state.result.is_some(),
        "a hint stays sealable until a clean board arrives after it"
    );
    assert_eq!(phase_of(&m.active().state), Phase::PostResultStreak);
}

#[test]
fn newest_unconfirmed_word_replaces_an_older_hint() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.poll(|s| {
        let mut i = poll_of(s, now);
        i.signal = Some(MatchOutcome::Defeat);
        i.signal_confirmed = false;
        i
    });
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(5));
        i.signal = Some(MatchOutcome::Victory);
        i.signal_confirmed = false;
        i
    });
    assert!(m.closed.is_empty());
    assert!(!m.active().outcome().is_decided());
    assert_eq!(
        m.active().state.result.map(|mark| mark.outcome),
        Some(MatchOutcome::Victory),
        "the newest unconfirmed word replaces the older hint"
    );
}

#[test]
fn unconfirmed_word_does_not_split_a_confirmed_result() {
    let now = t0();
    let mut m = Machine::new("Busan");
    confirm_defeat(&mut m, now);
    m.capture(
        counters(12, 5, 3, 3400, 1400, 0),
        now + Duration::from_secs(20),
    );
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(30));
        i.signal = Some(MatchOutcome::Victory);
        i.signal_confirmed = false;
        i.accolade_map = Some("Junkertown");
        i
    });
    assert!(m.closed.is_empty());
    assert_eq!(m.active().outcome(), MatchOutcome::Defeat);
    assert_eq!(m.active().map(), Some("Busan"));
}

#[test]
fn word_then_start_screen_seals_without_a_clock_window() {
    // A board plus one word, then a map vote. Ninety seconds later is
    // the same as three: there is no hint timer. The vote seals.
    let now = t0();
    let mut m = Machine::new("Busan");
    m.capture(counters(11, 20, 5, 2100, 8000, 100), now);
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(2));
        i.signal = Some(MatchOutcome::Victory);
        i.signal_confirmed = false;
        i
    });
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(90));
        i.start_screen = Some(StartScreen::MapVote {
            candidates: vec!["Junkertown".into()],
        });
        i
    });
    assert_eq!(m.closed.len(), 1);
    assert_eq!(m.closed[0].sess.outcome(), MatchOutcome::Victory);
    assert!(!m.active().outcome().is_decided());
}

#[test]
fn live_board_continuation_clears_a_hint() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.capture(counters(8, 3, 2, 1800, 4000, 100), now);
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(2));
        i.signal = Some(MatchOutcome::Victory);
        i.signal_confirmed = false;
        i
    });
    assert!(m.active().state.result.is_some());
    m.capture(
        counters(10, 4, 2, 2100, 4500, 120),
        now + Duration::from_secs(10),
    );
    assert_eq!(
        m.active().state.result.map(|mark| mark.outcome),
        Some(MatchOutcome::Victory),
        "the first progressed board keeps the hint"
    );
    m.capture(
        counters(12, 5, 3, 2500, 5000, 160),
        now + Duration::from_secs(20),
    );
    assert!(
        m.active().state.result.is_none(),
        "the second progressed board drops the hint"
    );
    assert!(m.closed.is_empty());
}

#[test]
fn stray_word_then_clean_continuation_then_start_screen_does_not_split() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.capture_board(
        counters(8, 3, 2, 1800, 4000, 100),
        Some("Zenyatta"),
        Some(2),
        now,
    );
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(30));
        i.signal = Some(MatchOutcome::Victory);
        i.signal_confirmed = false;
        i
    });
    m.capture_board(
        counters(10, 4, 2, 2200, 4600, 140),
        Some("Mercy"),
        Some(2),
        now + Duration::from_secs(40),
    );
    assert_eq!(
        m.active().state.result.map(|mark| mark.outcome),
        Some(MatchOutcome::Victory),
        "the first progressed board keeps the hint"
    );
    m.capture_board(
        counters(12, 5, 3, 2600, 5000, 180),
        Some("Mercy"),
        Some(2),
        now + Duration::from_secs(50),
    );
    assert!(
        m.active().state.result.is_none(),
        "the second progressed board drops the hint"
    );
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(50));
        i.start_screen = Some(StartScreen::HeroSelect);
        i
    });
    assert!(
        m.closed.is_empty(),
        "hero swap on a climbing board is the same match"
    );
    assert!(!m.active().outcome().is_decided());
    assert_eq!(m.active().state.hero.as_deref(), Some("Mercy"));
}

#[test]
fn hero_select_a_minute_later_keeps_the_hint_armed() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.capture(counters(11, 20, 5, 2100, 8000, 100), now);
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(10));
        i.signal = Some(MatchOutcome::Defeat);
        i.signal_confirmed = false;
        i
    });
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(90));
        i.start_screen = Some(StartScreen::HeroSelect);
        i
    });
    assert!(m.closed.is_empty());
    assert!(m.active().state.pending_boundary);
    assert_eq!(
        m.active().state.result.map(|mark| mark.outcome),
        Some(MatchOutcome::Defeat),
        "a minute later the hint is still sealable"
    );
}

#[test]
fn deferred_board_is_held_for_the_new_session() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.capture(
        counters(18, 7, 9, 6400, 11000, 800),
        now + Duration::from_secs(60),
    );
    confirm_defeat(&mut m, now + Duration::from_secs(8 * 60));
    let first = counters(1, 3, 0, 220, 80, 400);
    m.capture(first, now + Duration::from_secs(8 * 60 + 25));
    assert!(m.closed.is_empty());
    assert_eq!(m.active().state.deferred, Some(first));
    assert!(
        m.active()
            .state
            .gate
            .is_some_and(|g| g.accepted.elims == 18)
    );
    let second = counters(2, 4, 1, 400, 120, 700);
    m.capture(second, now + Duration::from_secs(8 * 60 + 40));
    assert_eq!(m.closed.len(), 1);
    assert_eq!(m.active().opened_with, Some(first));
    assert_eq!(m.active().state.gate.map(|g| g.accepted), Some(second));
    assert_eq!(
        m.closed[0].sess.state.gate.map(|g| g.accepted.elims),
        Some(18)
    );
}

#[test]
fn hero_change_alone_does_not_split_a_live_match() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.capture_board(
        counters(8, 3, 2, 1800, 4000, 100),
        Some("Zenyatta"),
        Some(0),
        now,
    );
    m.capture_board(
        counters(10, 4, 2, 2200, 4600, 140),
        Some("Wrecking Ball"),
        Some(0),
        now + Duration::from_secs(60),
    );
    assert!(m.closed.is_empty());
    assert_eq!(m.active().state.hero.as_deref(), Some("Wrecking Ball"));
}

#[test]
fn same_stats_after_a_word_do_not_clear_the_hint() {
    let now = t0();
    let board = counters(8, 3, 2, 1800, 4000, 100);
    let mut m = Machine::new("Busan");
    m.capture(board, now);
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(30));
        i.signal = Some(MatchOutcome::Defeat);
        i.signal_confirmed = false;
        i
    });
    m.capture(board, now + Duration::from_secs(40));
    assert_eq!(
        m.active().state.result.map(|mark| mark.outcome),
        Some(MatchOutcome::Defeat)
    );
    assert!(m.closed.is_empty());
}

#[test]
fn map_vote_during_a_live_match_splits_without_sealing() {
    let now = t0();
    let mut m = Machine::new("Busan");
    m.capture(counters(8, 3, 2, 1800, 4000, 100), now);
    m.poll(|s| {
        let mut i = poll_of(s, now + Duration::from_secs(200));
        i.start_screen = Some(StartScreen::MapVote {
            candidates: vec!["Junkertown".into(), "Ilios".into()],
        });
        i
    });
    assert_eq!(m.closed.len(), 1);
    assert_eq!(m.closed[0].reason, CloseReason::MapVote);
    assert!(!m.closed[0].sess.outcome().is_decided());
    assert!(m.active().state.awaiting_first_board);
}
