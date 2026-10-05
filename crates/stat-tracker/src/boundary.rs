//! New-game boundaries for the stat-tracker session machine.
//!
//! One session stays open across hero swaps. It closes once that match is
//! over and a signal that belongs to the next queue shows up.
//!
//! On 2026-10-05 the user requeued from a Busan defeat straight into a
//! Junkertown victory. The poller saw the defeat accolade (`poll_streak_defeat`,
//! leaving the game in one second) but the map vote never landed on a poll
//! tick. The Junkertown scoreboard stayed on the Busan session: stat drops
//! were held as per-cell OCR noise, and the later Victory word was the first
//! *confirmed* outcome, so it replaced the defeat. These decisions are the
//! gate the poller and the Tab path both call.

use std::time::{Duration, Instant};

use crate::capture_gate::{self, Counters, GATE_COLS, GateState};
use crate::detect::MatchOutcome;

/// How long after an end screen a later result is a new match even when
/// nothing else was seen in between. Accolade and rank screens stay up well
/// under a minute; a couple of minutes is past that.
pub const RESULT_GAP: Duration = Duration::from_secs(120);

/// End-screen evidence kept on the session until it closes.
///
/// A streak (`confirmed == false`) arms boundaries. It is written as the
/// stored outcome only when a boundary actually closes the session, so one
/// hallucinated word does not finish a match by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultMark {
    pub outcome: MatchOutcome,
    pub confirmed: bool,
    pub seen_at: Instant,
}

/// A screen that starts the next match. Hero ban is first-class: recording
/// which heroes were banned stays on
/// [`crate::detect::match_start::detect_ban_screen`] (future work). This
/// variant only answers "the ban UI is up, so the previous match is over."
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartScreen {
    MapVote { candidates: Vec<String> },
    HeroSelect,
    HeroBan,
}

/// One poll tick, as the session machine sees it.
#[derive(Clone, Debug)]
pub struct PollInput<'a> {
    pub outcome: MatchOutcome,
    pub outcome_at: Option<Instant>,
    pub result: Option<ResultMark>,
    /// A scoreboard capture landed after the end screen. Hero select / ban
    /// is not recorded here — that screen is itself a boundary.
    pub intervening_scoreboard: bool,
    pub map: Option<&'a str>,
    pub signal: Option<MatchOutcome>,
    /// Banner, or the second agreeing word inside the confirm window.
    pub signal_confirmed: bool,
    pub accolade_map: Option<&'a str>,
    pub start_screen: Option<StartScreen>,
    /// POTG / end-reel cadence wake. Never a boundary on its own, and not
    /// an intervening gap.
    pub end_reel: bool,
    pub now: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenNew {
    pub reason: &'static str,
    /// Outcome to store on the session being closed when it does not already
    /// have one (a defeat streak the second poll never confirmed).
    pub seal_outcome: Option<MatchOutcome>,
    pub new_outcome: MatchOutcome,
    pub new_map: Option<String>,
    pub candidates: Vec<String>,
    pub new_result: Option<ResultMark>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateCurrent {
    pub record_outcome: Option<MatchOutcome>,
    pub adopt_map: Option<String>,
    pub result: Option<ResultMark>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PollDecision {
    Keep,
    /// A contradictory word arrived with no new-game gap. The stored
    /// outcome (or the earlier streak) stays.
    IgnoreContradictory {
        kept: MatchOutcome,
        ignored: MatchOutcome,
    },
    Update(UpdateCurrent),
    Open(OpenNew),
}

/// Whether `cur` is a real, sharp fall from the accepted counter: under half,
/// past the OCR jitter band, and not an edge-ink suspect read.
fn sharp_vote(acc: u32, raw: u32, cur: u32, cur_suspect: bool, raw_suspect: bool) -> bool {
    if cur_suspect || acc < 4 || cur.saturating_mul(2) >= acc {
        return false;
    }
    if raw_suspect {
        true
    } else {
        capture_gate::raw_dropped(raw, cur)
    }
}

/// After an end screen, a sharp drop across several cumulative columns is a
/// new game. Two of elims/deaths/damage, or any three of the six counters.
/// One column is still an OCR miss — the capture gate holds that cell.
pub fn post_result_stat_reset(
    prev: &GateState,
    cur: Counters,
    cur_suspect: [bool; GATE_COLS],
) -> bool {
    let acc = prev.accepted.to_array();
    let raw = prev.last_raw.to_array();
    let now = cur.to_array();
    let mut sharp = [false; GATE_COLS];
    for col in 0..GATE_COLS {
        sharp[col] = sharp_vote(
            acc[col],
            raw[col],
            now[col],
            cur_suspect[col],
            prev.last_raw_suspect[col],
        );
    }
    // edd order matches `Counters::edd`: elims, deaths, damage = cols 0, 2, 3.
    let edd = u8::from(sharp[0]) + u8::from(sharp[2]) + u8::from(sharp[3]);
    let n = sharp.iter().filter(|s| **s).count();
    edd >= 2 || n >= 3
}

/// Combine the post-result reset with the mid-match regression split.
///
/// `sharp_reset` is only consulted when an end screen was already seen, so a
/// multi-column drop mid-match still needs the time gap and the
/// same-match suppress. `classic_regressed` is [`crate`] `stats_regressed`
/// (2 of 3 elims/deaths/damage, any size past the jitter band).
pub fn capture_splits(
    after_end_screen: bool,
    create_session: bool,
    suppress_same_unfinished: bool,
    age: Option<Duration>,
    min_gap: Duration,
    classic_regressed: bool,
    sharp_reset: bool,
) -> bool {
    if create_session {
        return false;
    }
    if after_end_screen && sharp_reset {
        return true;
    }
    !suppress_same_unfinished && age.is_some_and(|age| age >= min_gap) && classic_regressed
}

pub fn has_post_result(outcome: MatchOutcome, result: Option<ResultMark>) -> bool {
    remembered_outcome(outcome, result).is_some()
}

fn remembered_outcome(outcome: MatchOutcome, result: Option<ResultMark>) -> Option<MatchOutcome> {
    if outcome.is_decided() {
        Some(outcome)
    } else {
        result.map(|m| m.outcome).filter(|o| o.is_decided())
    }
}

fn evidence_at(
    outcome: MatchOutcome,
    outcome_at: Option<Instant>,
    result: Option<ResultMark>,
) -> Option<Instant> {
    result
        .map(|m| m.seen_at)
        .or(outcome_at.filter(|_| outcome.is_decided()))
}

fn time_gap(at: Option<Instant>, now: Instant) -> bool {
    at.is_some_and(|t| now.saturating_duration_since(t) > RESULT_GAP)
}

fn confident_map(map: Option<&str>) -> Option<&str> {
    map.map(str::trim).filter(|s| !s.is_empty())
}

/// Accolade map when it is a different confident name from the session map.
fn maps_differ<'a>(session_map: Option<&str>, accolade: Option<&'a str>) -> Option<&'a str> {
    let session = confident_map(session_map)?;
    let accolade = confident_map(accolade)?;
    if session.eq_ignore_ascii_case(accolade) {
        None
    } else {
        Some(accolade)
    }
}

fn adopt_map(session_map: Option<&str>, accolade: Option<&str>) -> Option<String> {
    if confident_map(session_map).is_some() {
        return None;
    }
    confident_map(accolade).map(str::to_string)
}

fn open_from_screen(screen: &StartScreen, seal: Option<MatchOutcome>) -> OpenNew {
    let (reason, candidates) = match screen {
        StartScreen::MapVote { candidates } => ("superseded by map vote", candidates.clone()),
        StartScreen::HeroSelect | StartScreen::HeroBan => {
            ("superseded by hero select/ban", Vec::new())
        }
    };
    OpenNew {
        reason,
        seal_outcome: seal,
        new_outcome: MatchOutcome::Unknown,
        new_map: None,
        candidates,
        new_result: None,
    }
}

fn open_for_result(
    input: &PollInput<'_>,
    seal: Option<MatchOutcome>,
    signal: MatchOutcome,
) -> OpenNew {
    let new_map = confident_map(input.accolade_map).map(str::to_string);
    OpenNew {
        reason: if maps_differ(input.map, input.accolade_map).is_some() && input.signal.is_none() {
            "superseded by accolade map mismatch"
        } else if maps_differ(input.map, input.accolade_map).is_some()
            && input
                .signal
                .is_some_and(|s| remembered_outcome(input.outcome, input.result) == Some(s))
        {
            // Same result word, different map, after a gap: the map is what
            // proves this end screen is not the one already stored.
            "superseded by accolade map mismatch"
        } else {
            "superseded by later result screen"
        },
        seal_outcome: seal,
        new_outcome: signal,
        new_map,
        candidates: Vec::new(),
        new_result: Some(ResultMark {
            outcome: signal,
            confirmed: input.signal_confirmed,
            seen_at: input.now,
        }),
    }
}

/// What this poll tick does to the open session.
///
/// `end_reel` is part of the input so a POTG wake cannot be forgotten and
/// wired up as a split later. It does not change the decision.
pub fn decide_poll(input: &PollInput<'_>) -> PollDecision {
    let _ = input.end_reel;

    let prior = remembered_outcome(input.outcome, input.result);
    let anchor = evidence_at(input.outcome, input.outcome_at, input.result);
    let timed = time_gap(anchor, input.now);
    let gap_for_contradiction = input.intervening_scoreboard || timed;
    let seal = if input.outcome.is_decided() {
        None
    } else {
        prior
    };

    if prior.is_some()
        && let Some(screen) = input.start_screen.as_ref()
    {
        return PollDecision::Open(open_from_screen(screen, seal));
    }

    if let Some(signal) = input.signal.filter(|o| o.is_decided()) {
        if let Some(prev) = prior {
            let same = prev == signal;
            let map_changed = maps_differ(input.map, input.accolade_map).is_some();
            // A different map on the same end screen is not a new match: the
            // contradictory word needs a scoreboard or a couple of minutes
            // between the two results, and a map mismatch needs that same gap.
            // Otherwise a second OCR of the screen we already stored would
            // either overwrite the outcome or cut the session in two.
            let split = (!same && gap_for_contradiction)
                || (same && timed)
                || (map_changed && gap_for_contradiction);
            if split {
                return PollDecision::Open(open_for_result(input, seal, signal));
            }
            if !same {
                return PollDecision::IgnoreContradictory {
                    kept: prev,
                    ignored: signal,
                };
            }
            let seen_at = input.result.map(|m| m.seen_at).unwrap_or(input.now);
            let result = ResultMark {
                outcome: signal,
                confirmed: input.result.is_some_and(|m| m.confirmed) || input.signal_confirmed,
                seen_at,
            };
            let record = (input.signal_confirmed && !input.outcome.is_decided()).then_some(signal);
            let adopt = adopt_map(input.map, input.accolade_map);
            if record.is_none() && adopt.is_none() && input.result == Some(result) {
                return PollDecision::Keep;
            }
            return PollDecision::Update(UpdateCurrent {
                record_outcome: record,
                adopt_map: adopt,
                result: Some(result),
            });
        }

        let result = ResultMark {
            outcome: signal,
            confirmed: input.signal_confirmed,
            seen_at: input.now,
        };
        return PollDecision::Update(UpdateCurrent {
            record_outcome: (input.signal_confirmed && !input.outcome.is_decided())
                .then_some(signal),
            adopt_map: adopt_map(input.map, input.accolade_map),
            result: Some(result),
        });
    }

    if let Some(new_map) = maps_differ(input.map, input.accolade_map)
        && prior.is_some()
        && (timed || input.intervening_scoreboard)
    {
        return PollDecision::Open(OpenNew {
            reason: "superseded by accolade map mismatch",
            seal_outcome: seal,
            new_outcome: MatchOutcome::Unknown,
            new_map: Some(new_map.to_string()),
            candidates: Vec::new(),
            new_result: None,
        });
    }

    if let Some(map) = adopt_map(input.map, input.accolade_map) {
        return PollDecision::Update(UpdateCurrent {
            record_outcome: None,
            adopt_map: Some(map),
            result: input.result,
        });
    }

    PollDecision::Keep
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_gate::{HoldKind, apply_gate};

    fn t0() -> Instant {
        Instant::now() + Duration::from_secs(3_600)
    }

    fn input<'a>(outcome: MatchOutcome, result: Option<ResultMark>, now: Instant) -> PollInput<'a> {
        PollInput {
            outcome,
            outcome_at: outcome.is_decided().then_some(now),
            result,
            intervening_scoreboard: false,
            map: None,
            signal: None,
            signal_confirmed: false,
            accolade_map: None,
            start_screen: None,
            end_reel: false,
            now,
        }
    }

    fn counters(e: u32, a: u32, d: u32, dmg: u32, hlg: u32, mit: u32) -> Counters {
        Counters {
            elims: e,
            assists: a,
            deaths: d,
            damage: dmg,
            healing: hlg,
            mitigation: mit,
        }
    }

    fn gate(c: Counters) -> GateState {
        GateState {
            accepted: c,
            last_raw: c,
            ..GateState::default()
        }
    }

    const CLEAN: [bool; GATE_COLS] = [false; GATE_COLS];

    /// In-memory stand-in for the poller + Tab path. It applies
    /// [`decide_poll`] and [`capture_splits`] the same way `main` does.
    struct Machine {
        active: Option<Sess>,
        closed: Vec<Closed>,
        n: u32,
    }

    struct Closed {
        sess: Sess,
        reason: &'static str,
    }

    #[derive(Clone)]
    struct Sess {
        id: String,
        map: Option<String>,
        outcome: MatchOutcome,
        outcome_at: Option<Instant>,
        result: Option<ResultMark>,
        intervening: bool,
        gate: Option<GateState>,
    }

    impl Machine {
        fn new(map: &str, _now: Instant) -> Self {
            Self {
                active: Some(Sess {
                    id: "busan".into(),
                    map: Some(map.into()),
                    outcome: MatchOutcome::Unknown,
                    outcome_at: None,
                    result: None,
                    intervening: false,
                    gate: None,
                }),
                closed: Vec::new(),
                n: 0,
            }
        }

        fn active(&self) -> &Sess {
            self.active.as_ref().expect("active session")
        }

        fn poll(&mut self, mut build: impl for<'a> FnMut(&'a Sess) -> PollInput<'a>) {
            let Some(sess) = self.active.as_ref() else {
                return;
            };
            let decision = {
                let input = build(sess);
                decide_poll(&input)
            };
            self.apply(decision);
        }

        fn apply(&mut self, decision: PollDecision) {
            match decision {
                PollDecision::Keep | PollDecision::IgnoreContradictory { .. } => {}
                PollDecision::Update(u) => {
                    let g = self.active.as_mut().expect("active");
                    if let Some(mark) = u.result {
                        g.result = Some(mark);
                    }
                    if let Some(outcome) = u.record_outcome
                        && !g.outcome.is_decided()
                    {
                        g.outcome = outcome;
                        g.outcome_at = g.result.map(|m| m.seen_at);
                    }
                    if g.map.is_none()
                        && let Some(map) = u.adopt_map
                    {
                        g.map = Some(map);
                    }
                }
                PollDecision::Open(o) => {
                    let seen = o.new_result.map(|m| m.seen_at);
                    self.open_new(o.reason, o.seal_outcome, |g| {
                        g.outcome = o.new_outcome;
                        if o.new_outcome.is_decided() {
                            g.outcome_at = Some(seen.unwrap_or_else(t0));
                        }
                        g.map = o.new_map;
                        g.result = o.new_result;
                    });
                }
            }
        }

        fn open_new(
            &mut self,
            reason: &'static str,
            seal: Option<MatchOutcome>,
            init: impl FnOnce(&mut Sess),
        ) {
            let mut prev = self.active.take().expect("session to close");
            if !prev.outcome.is_decided() {
                let outcome = seal
                    .or(prev.result.map(|m| m.outcome))
                    .filter(|o| o.is_decided());
                if let Some(outcome) = outcome {
                    prev.outcome = outcome;
                    prev.outcome_at = prev.result.map(|m| m.seen_at);
                }
            }
            self.closed.push(Closed { sess: prev, reason });
            self.n += 1;
            let mut g = Sess {
                id: format!("s{}", self.n),
                map: None,
                outcome: MatchOutcome::Unknown,
                outcome_at: None,
                result: None,
                intervening: false,
                gate: None,
            };
            init(&mut g);
            self.active = Some(g);
        }

        fn capture(&mut self, cur: Counters, _now: Instant) {
            let g = self.active.as_mut().expect("active");
            let after_end = has_post_result(g.outcome, g.result);
            let (age, classic, sharp) = match g.gate {
                Some(state) => {
                    let age = Some(Duration::from_secs(10));
                    let classic = false;
                    let sharp = post_result_stat_reset(&state, cur, CLEAN);
                    (age, classic, sharp)
                }
                None => (None, false, false),
            };
            let split = capture_splits(after_end, false, true, age, RESULT_GAP, classic, sharp);
            if split {
                self.open_new("superseded by stat reset", None, |g| {
                    g.gate = Some(gate(cur));
                });
                return;
            }
            let already_post = after_end;
            if let Some(state) = g.gate {
                g.gate = Some(
                    apply_gate(Some((state, Duration::from_secs(10))), cur, CLEAN, false).state,
                );
            } else {
                g.gate = Some(gate(cur));
            }
            if already_post {
                g.intervening = true;
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
            !m.active().outcome.is_decided(),
            "a single defeat word is a streak, not a stored outcome"
        );
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(4));
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = true;
            i
        });
        assert_eq!(m.active().outcome, MatchOutcome::Defeat);
        assert_eq!(m.active().map.as_deref(), Some("Busan"));
    }

    fn poll_of<'a>(s: &'a Sess, now: Instant) -> PollInput<'a> {
        PollInput {
            outcome: s.outcome,
            outcome_at: s.outcome_at,
            result: s.result,
            intervening_scoreboard: s.intervening,
            map: s.map.as_deref(),
            signal: None,
            signal_confirmed: false,
            accolade_map: None,
            start_screen: None,
            end_reel: false,
            now,
        }
    }

    #[test]
    fn busan_defeat_then_hero_select_then_junkertown_victory() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
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
        assert_eq!(m.closed[0].reason, "superseded by hero select/ban");
        assert_eq!(m.closed[0].sess.map.as_deref(), Some("Busan"));
        assert_eq!(m.closed[0].sess.outcome, MatchOutcome::Defeat);
        assert_eq!(m.active().id, "s1");
        assert!(m.active().map.is_none());

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
        assert_eq!(m.active().outcome, MatchOutcome::Victory);
        assert_eq!(m.active().map.as_deref(), Some("Junkertown"));
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].sess.outcome, MatchOutcome::Defeat);
    }

    #[test]
    fn busan_defeat_then_stat_reset_without_map_vote() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
        m.capture(
            counters(18, 7, 9, 6400, 11000, 800),
            now + Duration::from_secs(60),
        );
        confirm_defeat(&mut m, now + Duration::from_secs(8 * 60));

        // First Junkertown board, well inside the 120s mid-match gap, and
        // `capture_splits` is called with same-match suppress = true.
        m.capture(
            counters(1, 3, 0, 220, 80, 400),
            now + Duration::from_secs(8 * 60 + 25),
        );
        assert_eq!(m.closed.len(), 1, "stat reset opens the second session");
        assert_eq!(m.closed[0].reason, "superseded by stat reset");
        assert_eq!(m.closed[0].sess.outcome, MatchOutcome::Defeat);
        assert_eq!(m.closed[0].sess.map.as_deref(), Some("Busan"));
        assert_eq!(
            m.active().gate.map(|g| g.accepted.elims),
            Some(1),
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
        assert_eq!(m.active().outcome, MatchOutcome::Victory);
        assert_eq!(m.active().map.as_deref(), Some("Junkertown"));
        assert_eq!(m.closed.len(), 1);
    }

    #[test]
    fn defeat_streak_then_hero_select_keeps_the_loss() {
        // Live shape: one defeat word (`poll_streak_defeat`), queue pops
        // before the agreeing read, hero select with no map vote.
        let now = t0();
        let mut m = Machine::new("Busan", now);
        m.capture(counters(11, 20, 5, 2100, 8000, 100), now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(30));
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = false;
            i
        });
        assert!(!m.active().outcome.is_decided());
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(45));
            i.start_screen = Some(StartScreen::HeroSelect);
            i
        });
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].sess.outcome, MatchOutcome::Defeat);
        assert_eq!(m.closed[0].sess.map.as_deref(), Some("Busan"));
        assert!(!m.active().outcome.is_decided());
    }

    #[test]
    fn hero_ban_is_a_boundary_after_a_result() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
        confirm_defeat(&mut m, now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(15));
            i.start_screen = Some(StartScreen::HeroBan);
            i
        });
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].reason, "superseded by hero select/ban");
        assert_eq!(m.closed[0].sess.outcome, MatchOutcome::Defeat);
    }

    #[test]
    fn contradictory_result_after_a_couple_of_minutes_splits() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
        confirm_defeat(&mut m, now);
        // No scoreboard, no hero select. Time is the only gap.
        m.poll(|s| {
            let mut i = poll_of(s, now + RESULT_GAP + Duration::from_secs(5));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = false;
            i.accolade_map = Some("Junkertown");
            i
        });
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].reason, "superseded by later result screen");
        assert_eq!(m.closed[0].sess.outcome, MatchOutcome::Defeat);
        assert_eq!(m.closed[0].sess.map.as_deref(), Some("Busan"));
        assert_eq!(m.active().outcome, MatchOutcome::Victory);
        assert_eq!(m.active().map.as_deref(), Some("Junkertown"));
    }

    #[test]
    fn contradictory_result_without_a_gap_does_not_overwrite() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
        confirm_defeat(&mut m, now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(20));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i.accolade_map = Some("Junkertown");
            i
        });
        assert!(m.closed.is_empty());
        assert_eq!(m.active().outcome, MatchOutcome::Defeat);
        assert_eq!(
            m.active().map.as_deref(),
            Some("Busan"),
            "a same-screen map read must not replace the session map"
        );
    }

    #[test]
    fn scoreboard_between_results_is_a_gap_for_a_contradictory_word() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
        m.capture(counters(10, 4, 3, 3000, 1000, 0), now);
        confirm_defeat(&mut m, now + Duration::from_secs(30));
        // Post-match board: stats did not reset, so this stays Busan, but
        // it is intervening evidence for the next result word.
        m.capture(
            counters(12, 5, 3, 3400, 1400, 0),
            now + Duration::from_secs(40),
        );
        assert!(m.closed.is_empty());
        assert!(m.active().intervening);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(70));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i.accolade_map = Some("Junkertown");
            i
        });
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].sess.outcome, MatchOutcome::Defeat);
        assert_eq!(m.active().outcome, MatchOutcome::Victory);
        assert_eq!(m.active().map.as_deref(), Some("Junkertown"));
    }

    #[test]
    fn accolade_map_mismatch_after_a_gap_splits() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
        confirm_defeat(&mut m, now);
        m.poll(|s| {
            let mut i = poll_of(s, now + RESULT_GAP + Duration::from_secs(1));
            // Same word, different map — the map is the mismatch signal.
            i.signal = Some(MatchOutcome::Defeat);
            i.signal_confirmed = true;
            i.accolade_map = Some("Junkertown");
            i
        });
        assert_eq!(m.closed.len(), 1);
        assert_eq!(m.closed[0].reason, "superseded by accolade map mismatch");
        assert_eq!(m.closed[0].sess.map.as_deref(), Some("Busan"));
        assert_eq!(m.closed[0].sess.outcome, MatchOutcome::Defeat);
        assert_eq!(m.active().map.as_deref(), Some("Junkertown"));
        assert_eq!(m.active().outcome, MatchOutcome::Defeat);
    }

    #[test]
    fn end_reel_wake_does_not_split_or_open_a_gap() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
        confirm_defeat(&mut m, now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(10));
            i.end_reel = true;
            i
        });
        assert!(m.closed.is_empty());
        assert!(!m.active().intervening);
        assert_eq!(m.active().outcome, MatchOutcome::Defeat);
        // A contradictory word still inside the couple-of-minutes window,
        // with only the POTG wake in between, stays on Busan.
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(30));
            i.signal = Some(MatchOutcome::Victory);
            i.signal_confirmed = true;
            i.end_reel = true;
            i
        });
        assert!(m.closed.is_empty());
        assert_eq!(m.active().outcome, MatchOutcome::Defeat);
        assert_eq!(m.active().map.as_deref(), Some("Busan"));
    }

    #[test]
    fn hero_select_mid_match_does_not_open_a_session() {
        let now = t0();
        let mut m = Machine::new("Busan", now);
        m.poll(|s| {
            let mut i = poll_of(s, now + Duration::from_secs(30));
            i.start_screen = Some(StartScreen::HeroSelect);
            i
        });
        assert!(m.closed.is_empty());
        assert_eq!(m.active().id, "busan");
    }

    #[test]
    fn single_cell_ocr_noise_is_held_and_does_not_split() {
        let prev_c = counters(22, 7, 5, 6400, 11000, 800);
        let prev = gate(prev_c);
        let mut cur = prev_c;
        cur.elims = 2;
        assert!(
            !post_result_stat_reset(&prev, cur, CLEAN),
            "one sharp column is an OCR miss, not a new game"
        );
        assert!(
            !capture_splits(
                true,
                false,
                true,
                Some(Duration::from_secs(5)),
                RESULT_GAP,
                false,
                false
            ),
            "after an end screen a single-column drop still does not split"
        );
        assert!(
            !capture_splits(
                false,
                false,
                false,
                Some(Duration::from_secs(5)),
                RESULT_GAP,
                true,
                true
            ),
            "mid-match, even a sharp reset waits out the gap"
        );
        let gated = apply_gate(Some((prev, Duration::from_secs(30))), cur, CLEAN, false);
        assert!(
            gated
                .holds
                .iter()
                .any(|h| h.col == 0 && h.kind == HoldKind::Monotonic && h.raw == 2),
            "the gate still holds the noisy cell: {holds:?}",
            holds = gated.holds
        );
        assert_eq!(gated.accepted.elims, 22);
    }

    #[test]
    fn post_result_reset_is_several_columns_and_ignores_the_mid_match_gap() {
        let prev = gate(counters(18, 7, 9, 6400, 11000, 800));
        let cur = counters(1, 3, 0, 220, 80, 400);
        assert!(post_result_stat_reset(&prev, cur, CLEAN));
        assert!(capture_splits(
            true,
            false,
            true,
            Some(Duration::from_secs(10)),
            RESULT_GAP,
            false,
            true,
        ));
        // Healing + damage + elims, deaths not sharp (9 → 6).
        let cur = counters(4, 8, 6, 900, 400, 900);
        assert!(post_result_stat_reset(&prev, cur, CLEAN));
    }

    #[test]
    fn first_accolade_fills_an_empty_map_and_does_not_split() {
        let now = t0();
        let decision = decide_poll(&PollInput {
            outcome: MatchOutcome::Unknown,
            outcome_at: None,
            result: None,
            intervening_scoreboard: false,
            map: None,
            signal: Some(MatchOutcome::Defeat),
            signal_confirmed: true,
            accolade_map: Some("Busan"),
            start_screen: None,
            end_reel: false,
            now,
        });
        match decision {
            PollDecision::Update(u) => {
                assert_eq!(u.record_outcome, Some(MatchOutcome::Defeat));
                assert_eq!(u.adopt_map.as_deref(), Some("Busan"));
            }
            other => panic!("expected update, got {other:?}"),
        }
    }

    #[test]
    fn input_builder_uses_session_fields() {
        // Keeps `input()` alive so a future caller can see the default shape
        // (no signal, no wake) is Keep.
        let now = t0();
        let decision = decide_poll(&input(MatchOutcome::Unknown, None, now));
        assert_eq!(decision, PollDecision::Keep);
    }
}
