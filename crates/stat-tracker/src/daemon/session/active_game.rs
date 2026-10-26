//! The open game across restarts: `impl ActiveGame`, active_game.json,
//! restart, rejoin and suspend recovery, adopting the startup session, and
//! held boards imported from a stale skeleton.

use crate::*;

impl ActiveGame {
    #[cfg(test)]
    pub(crate) fn open_now(
        session_id: String,
        outcome: detect::MatchOutcome,
        map_candidates: Vec<String>,
    ) -> Self {
        Self::open_at(session_id, outcome, map_candidates, Instant::now())
    }

    pub(crate) fn open_at(
        session_id: String,
        outcome: detect::MatchOutcome,
        map_candidates: Vec<String>,
        now: Instant,
    ) -> Self {
        ActiveGame {
            best_guess: None,
            session_id,
            outcome_recorded_at: (outcome != detect::MatchOutcome::Unknown).then_some(now),
            outcome,
            map: None,
            map_source: None,
            map_candidates,
            session_created: false,
            opened_at: now,
            last_activity: now,
            gate: None,
            last_stats_at: None,
            hero_auth: HeroAuthState::default(),
            result_mark: None,
            pending_boundary: false,
            awaiting_first_board: false,
            reset_streak: 0,
            reset_baseline: None,
            baseline_row: None,
            baseline_at: None,
            progressed_boards: 0,
            deferred: None,
            deferred_hero: None,
            deferred_at: None,
            deferred_imported: false,
            text_fallback_locked: false,
            replaced_text_fallback_map: None,
            result_evidence: shadow::result::ResultEvidence::default(),
            restored_with_result: false,
            restart_recovered_at: None,
            rejoin_carry: None,
            outcome_locked: false,
            last_board_pressed_at: None,
        }
    }

    pub(crate) fn boundary_state(&self) -> boundary::BoundaryState {
        boundary::BoundaryState {
            map: self.map.clone(),
            outcome: self.outcome,
            outcome_at: self.outcome_recorded_at,
            result: self.result_mark,
            reset_streak: self.reset_streak,
            reset_baseline: self.reset_baseline,
            baseline_row: self.baseline_row,
            last_board_at: self.last_stats_at,
            pending_boundary: self.pending_boundary,
            awaiting_first_board: self.awaiting_first_board,
            progressed_boards: self.progressed_boards,
            baseline_at: self.baseline_at,
            hero: self.hero_auth.accepted_hero.clone(),
            deferred: self.deferred,
            gate: self.gate,
            text_fallback_locked: self.text_fallback_locked,
        }
    }

    pub(crate) fn apply_boundary_state(&mut self, state: &boundary::BoundaryState) {
        self.map = state.map.clone();
        self.outcome = state.outcome;
        self.outcome_recorded_at = state.outcome_at;
        self.result_mark = state.result;
        self.reset_streak = state.reset_streak;
        self.reset_baseline = state.reset_baseline;
        self.baseline_row = state.baseline_row;
        self.pending_boundary = state.pending_boundary;
        self.awaiting_first_board = state.awaiting_first_board;
        self.progressed_boards = state.progressed_boards;
        self.baseline_at = state.baseline_at;
        self.deferred = state.deferred;
        if state.deferred.is_none() {
            self.deferred_hero = None;
            self.deferred_at = None;
            self.deferred_imported = false;
        }
        if state.last_board_at.is_some() {
            self.last_stats_at = state.last_board_at;
        }
        self.gate = state.gate;
        self.text_fallback_locked = state.text_fallback_locked;
    }

    /// Top bar and accolade. A text fallback, and a missing source, are not.
    pub(crate) fn map_is_trusted(&self) -> bool {
        self.map_source
            .is_some_and(boundary::MapSource::trusted_for_board_split)
    }

    pub(crate) fn has_post_result(&self) -> bool {
        boundary::has_post_result(self.outcome, self.result_mark)
    }

    pub(crate) fn finished(&self) -> bool {
        !matches!(self.outcome, detect::MatchOutcome::Unknown)
    }

    pub(crate) fn record_outcome(&mut self, outcome: detect::MatchOutcome) {
        self.record_outcome_at(outcome, Instant::now());
    }

    pub(crate) fn record_outcome_at(&mut self, outcome: detect::MatchOutcome, now: Instant) {
        self.outcome = outcome;
        self.outcome_recorded_at = Some(now);
        self.last_activity = now;
        self.outcome_locked = false;
    }

    /// A result the member set in the app (SetOutcome, EditMatch). A Tab
    /// never clears it (R9-2).
    pub(crate) fn record_member_outcome(&mut self, outcome: detect::MatchOutcome) {
        self.record_outcome(outcome);
        self.outcome_locked = outcome.is_decided();
    }

    pub(crate) fn touch_at(&mut self, now: Instant) {
        self.last_activity = now;
    }

    /// Remember a result word. A repeat of the same outcome keeps the first
    /// sighting. The post-match grace is [`Self::outcome_recorded_at`],
    /// stamped when the result is recorded. The hint stays sealable until a
    /// second progressed board, or one progressed board after an arm.
    pub(crate) fn note_result(&mut self, outcome: detect::MatchOutcome, confirmed: bool) {
        if !outcome.is_decided() {
            return;
        }
        let seen_at = self
            .result_mark
            .filter(|m| m.outcome == outcome)
            .map(|m| m.seen_at)
            .unwrap_or_else(Instant::now);
        let confirmed = confirmed
            || self
                .result_mark
                .is_some_and(|m| m.outcome == outcome && m.confirmed);
        self.result_mark = Some(ResultMark {
            outcome,
            confirmed,
            seen_at,
        });
    }
}

pub(crate) fn active_game_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("active_game.json")
}

pub(crate) fn persist_active_game(data_dir: &std::path::Path, game: Option<&ActiveGame>) {
    let path = active_game_path(data_dir);
    let Some(g) = game else {
        let _ = std::fs::remove_file(&path);
        return;
    };
    let now_i = Instant::now();
    let now_w = Utc::now();
    let to_wall = |i: Instant| {
        now_w - chrono::Duration::from_std(now_i.duration_since(i)).unwrap_or_default()
    };
    let persisted = PersistedGame {
        session_id: g.session_id.clone(),
        outcome: g.outcome,
        map: g.map.clone(),
        map_source: g.map_source,
        map_candidates: g.map_candidates.clone(),
        session_created: g.session_created,
        opened_at: to_wall(g.opened_at),
        last_activity: to_wall(g.last_activity),
        outcome_recorded_at: g.outcome_recorded_at.map(to_wall),
        gate: g.gate,
        last_stats_at: g.last_stats_at.map(to_wall),
        hero_auth: g.hero_auth.clone(),
        result_outcome: g.result_mark.map(|m| m.outcome),
        result_confirmed: g.result_mark.is_some_and(|m| m.confirmed),
        result_seen_at: g.result_mark.map(|m| to_wall(m.seen_at)),
        pending_boundary: g.pending_boundary,
        awaiting_first_board: g.awaiting_first_board,
        reset_streak: g.reset_streak,
        reset_baseline: g.reset_baseline,
        baseline_row: g.baseline_row,
        deferred: g.deferred,
        deferred_hero: g.deferred_hero.clone(),
        deferred_at: g.deferred_at,
        deferred_imported: g.deferred_imported,
        progressed_boards: g.progressed_boards,
        baseline_at: g.baseline_at.map(to_wall),
        text_fallback_locked: g.text_fallback_locked,
        replaced_text_fallback_map: g.replaced_text_fallback_map.clone(),
        best_guess: g.best_guess.clone(),
        rejoin_carry: g.rejoin_carry,
    };
    let write = || -> std::io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&persisted)?)?;
        std::fs::rename(&tmp, &path)
    };
    if let Err(e) = write() {
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "failed to persist active game");
    }
}

/// A held board found on a skeleton that is too old to resume.
pub(crate) struct StaleHeldBoard {
    pub(crate) counters: Counters,
    pub(crate) hero: Option<String>,
    pub(crate) map: Option<String>,
    pub(crate) at: Option<chrono::DateTime<Utc>>,
    pub(crate) outcome: detect::MatchOutcome,
    pub(crate) imported: bool,
}

/// Where a stale hold is written. An imported hold stays on its session.
/// A fresh hold is the next game, so it gets its own unknown session.
pub(crate) struct ArchivedHold {
    pub(crate) session_id: String,
    pub(crate) counters: Counters,
    pub(crate) hero: Option<String>,
    pub(crate) map: String,
    pub(crate) outcome: String,
    pub(crate) at: chrono::DateTime<Utc>,
}

/// A skeleton [`retire_stale_skeleton`] removed, plus a hold to keep.
pub(crate) struct StaleDrop {
    pub(crate) session_id: String,
    pub(crate) archive: Option<ArchivedHold>,
}

/// What [`admit_persisted_game`] decided about `active_game.json`.
pub(crate) enum ActiveAdmission {
    Open(Box<ActiveGame>),
    /// The file parsed, and the game is past the idle bound (or its clock
    /// cannot be mapped onto this boot). The rows are still in the store.
    /// `hold` is a board that was never written.
    Stale {
        session_id: String,
        hold: Option<StaleHeldBoard>,
    },
    Absent,
}

/// Recover the open game from a previous daemon run, if it is still plausibly
/// the current game (last activity within [`UNFINISHED_SESSION_IDLE`]).
/// Timestamps that predate the current boot (recovery across a reboot) are
/// treated as stale rather than clamped. [`startup_session`] is the session
/// restore. [`recover_or_sync_active_game`] deletes a stale skeleton and
/// uploads before that restore.
pub(crate) fn recover_active_game(data_dir: &std::path::Path) -> Option<ActiveGame> {
    match admit_persisted_game(data_dir) {
        ActiveAdmission::Open(game) => Some(*game),
        ActiveAdmission::Stale { .. } | ActiveAdmission::Absent => None,
    }
}

pub(crate) fn admit_persisted_game(data_dir: &std::path::Path) -> ActiveAdmission {
    let bytes = match std::fs::read(active_game_path(data_dir)) {
        Ok(bytes) => bytes,
        Err(_) => return ActiveAdmission::Absent,
    };
    let Ok(persisted) = serde_json::from_slice::<PersistedGame>(&bytes) else {
        return ActiveAdmission::Absent;
    };
    let session_id = persisted.session_id.clone();
    let hold = persisted.deferred.map(|counters| StaleHeldBoard {
        counters,
        hero: persisted.deferred_hero.clone(),
        map: persisted.map.clone(),
        at: persisted.deferred_at,
        outcome: persisted.outcome,
        imported: persisted.deferred_imported,
    });
    let crash_seen = stat_tracker::report::marker::last_crash_seen(data_dir);
    match active_game_from_persisted(persisted, crash_seen) {
        Some(game) => ActiveAdmission::Open(Box::new(game)),
        None => ActiveAdmission::Stale { session_id, hold },
    }
}

pub(crate) fn active_game_from_persisted(
    p: PersistedGame,
    crash_seen: Option<chrono::DateTime<Utc>>,
) -> Option<ActiveGame> {
    let to_instant =
        |w: chrono::DateTime<Utc>| Instant::now().checked_sub((Utc::now() - w).to_std().ok()?);
    if let Some(reason) = restart_resume_refusal(&p, crash_seen, Utc::now()) {
        tracing::info!(
            target: "scuffed_stat_tracker",
            session_id = %p.session_id,
            map = p.map.as_deref().unwrap_or(""),
            reason,
            "previous game not resumed after restart: the next Tab starts a new game"
        );
        return None;
    }
    let last_activity = to_instant(p.last_activity)?;
    if last_activity.elapsed() > UNFINISHED_SESSION_IDLE {
        return None;
    }
    let result_mark = recover_result_mark(
        p.result_outcome,
        p.result_confirmed,
        p.result_seen_at,
        &to_instant,
    );
    let hint_recovered = result_mark.is_some();
    Some(ActiveGame {
        best_guess: p.best_guess,
        session_id: p.session_id,
        outcome: p.outcome,
        map: p.map.clone(),
        map_source: p
            .map_source
            .or_else(|| p.map.is_some().then_some(boundary::MapSource::TextFallback)),
        map_candidates: p.map_candidates,
        session_created: p.session_created,
        opened_at: to_instant(p.opened_at)?,
        last_activity,
        // An unrecoverable timestamp behaves as "unstamped", which the grace
        // logic already treats as stale, the outcome can't leak forward.
        outcome_recorded_at: p.outcome_recorded_at.and_then(to_instant),
        last_stats_at: p.last_stats_at.and_then(to_instant),
        gate: p.gate,
        hero_auth: p.hero_auth,
        result_mark,
        // An arm with no hint would hold full poll cadence forever.
        pending_boundary: p.pending_boundary && hint_recovered,
        awaiting_first_board: p.awaiting_first_board,
        reset_streak: p.reset_streak,
        reset_baseline: p.reset_baseline,
        baseline_row: p.baseline_row,
        baseline_at: p.baseline_at.and_then(to_instant),
        progressed_boards: p.progressed_boards,
        deferred: p.deferred,
        deferred_hero: p.deferred_hero,
        deferred_at: p.deferred_at,
        deferred_imported: p.deferred_imported,
        // The hint is dropped when its timestamp cannot be mapped onto
        // this boot. The lock stays, so a text-fallback name does not
        // become open for an accolade just because the hint did not
        // survive the restart. The arm is the other way around: it is
        // dropped in that same case, above.
        text_fallback_locked: p.text_fallback_locked,
        replaced_text_fallback_map: p.replaced_text_fallback_map,
        result_evidence: shadow::result::ResultEvidence::default(),
        restored_with_result: hint_recovered,
        restart_recovered_at: None,
        rejoin_carry: p.rejoin_carry,
        // The source of a restored result is not saved: never un-latched.
        outcome_locked: p.outcome.is_decided(),
        last_board_pressed_at: None,
    })
}

/// Longest gap between the previous run stopping and this restart for
/// which the restart resumes that game. A crash and restart mid-game takes
/// well under a minute; the next game's first Tab usually comes after a
/// result screen, a queue and hero select. Past this, the next Tab starts a
/// new game. The gap is measured from when the previous run was last seen
/// alive ([`restart_stop_time`]), not from its last Tab: a player who did
/// not press Tab for a few minutes before a crash is still in that game.
pub(crate) const RESUME_AFTER_RESTART_MAX_GAP: std::time::Duration =
    std::time::Duration::from_secs(5 * 60);

/// Why the game in `active_game.json` is not resumed after a restart, or
/// `None` when it is clearly the same game. An unfinished game with a
/// stored board resumes only when it has a known map and the previous run
/// stopped at most [`RESUME_AFTER_RESTART_MAX_GAP`] ago
/// ([`restart_stop_time`]). The 20 minute [`UNFINISHED_SESSION_IDLE`]
/// bound on its last capture still applies. A Tab after the restart
/// that names another map still splits on the boundary's different-map
/// rule.
///
/// A game with a result already seen is restored so its result and its
/// post-match scoreboard stay on it. It is marked
/// [`ActiveGame::restored_with_result`], so the first Tab after
/// [`POST_MATCH_GRACE`] from that result starts a new game. A game whose
/// outcome is recorded starts a new game on that Tab anyway.
pub(crate) fn restart_resume_refusal(
    p: &PersistedGame,
    crash_seen: Option<chrono::DateTime<Utc>>,
    now: chrono::DateTime<Utc>,
) -> Option<&'static str> {
    if p.outcome.is_decided() {
        return None;
    }
    // A session with no stored board (opened by a start screen, or holding
    // the next game's board after a shutdown) cannot become a duplicate
    // game, so it keeps the [`UNFINISHED_SESSION_IDLE`] bound alone.
    if !p.session_created {
        return None;
    }
    if known_label(p.map.as_deref()).is_none() {
        return Some("no map recorded, so it cannot be matched");
    }
    if p.last_activity > now {
        return Some("last capture is later than now (clock moved back)");
    }
    match (now - restart_stop_time(p, crash_seen, now)).to_std() {
        Ok(gap) if gap <= RESUME_AFTER_RESTART_MAX_GAP => None,
        Ok(_) => Some("previous run stopped too long ago"),
        Err(_) => Some("last capture is later than now (clock moved back)"),
    }
}

/// When the previous run stopped, as far as the tracker can tell. The crash
/// marker's `last_seen` heartbeat (at most a minute before the crash) when
/// it is later than the game's last capture, else that last capture. An
/// old crash marker from an earlier run is older than the game's last
/// capture and changes nothing; a clean shutdown leaves no marker, so it
/// falls back to the last capture as before. A heartbeat in the future
/// (clock moved back) is ignored.
pub(crate) fn restart_stop_time(
    p: &PersistedGame,
    crash_seen: Option<chrono::DateTime<Utc>>,
    now: chrono::DateTime<Utc>,
) -> chrono::DateTime<Utc> {
    match crash_seen {
        Some(seen) if seen > p.last_activity && seen <= now => seen,
        _ => p.last_activity,
    }
}

/// Log the board that continued a game after a rejoin.
pub(crate) fn note_rejoin(req: &CaptureRequest, rejoin: &RejoinRead, raw: Counters) {
    if rejoin.continued {
        tracing::info!(
            target: "scuffed_stat_tracker",
            session_id = %req.session_id,
            raw_elims = raw.elims,
            raw_deaths = raw.deaths,
            raw_damage = raw.damage,
            elims = rejoin.counters.elims,
            deaths = rejoin.counters.deaths,
            damage = rejoin.counters.damage,
            "rejoin after restart: same map with reset stats continues the recovered game"
        );
    }
}

/// How long after a restart restored an unfinished game a rejoin can still
/// continue it ([`rejoin_board`]). Measured from the restart, which comes
/// right after the crash. Overwatch puts the player back in the match a
/// minute or two after the client comes back; past this, a stat drop is
/// handled by the normal stat-reset rule and splits as before.
pub(crate) const REJOIN_CONTINUE_WINDOW: std::time::Duration =
    std::time::Duration::from_secs(5 * 60);

/// Open the rejoin window on a game this run restored unfinished from the
/// previous run: no result seen, a stored board and a known map. A game
/// that a rejoin already continued keeps its carry and gets no new window:
/// its next board counts from the rejoin, below the stored total, and must
/// not be read as a second rejoin.
pub(crate) fn mark_restart_recovered(st: &mut SessionState, now: Instant) {
    if let Some(g) = st.active_game.as_mut()
        && g.rejoin_carry.is_none()
        && !g.finished()
        && g.result_mark.is_none()
        && g.session_created
        && known_label(g.map.as_deref()).is_some()
    {
        g.restart_recovered_at = Some(now);
    }
}

/// What a rejoin does to one board's counters. See [`rejoin_board`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RejoinRead {
    pub(crate) counters: Counters,
    /// The carry to keep on the game after this board.
    pub(crate) carry: Option<Counters>,
    /// This board is the one that continued the game after a rejoin.
    pub(crate) continued: bool,
}

pub(crate) fn add_carry(
    carry: Counters,
    cur: Counters,
    held: &[bool; capture_gate::GATE_COLS],
) -> Counters {
    // A held column already carries the game's last accepted value, which
    // includes the carry. Adding it again would double it.
    let add = |col: usize, c: u32, v: u32| if held[col] { v } else { c.saturating_add(v) };
    Counters {
        elims: add(0, carry.elims, cur.elims),
        assists: add(1, carry.assists, cur.assists),
        deaths: add(2, carry.deaths, cur.deaths),
        damage: add(3, carry.damage, cur.damage),
        healing: add(4, carry.healing, cur.healing),
        mitigation: add(5, carry.mitigation, cur.mitigation),
    }
}

/// Rejoin after a crash (2026-10-15 Rialto). Overwatch starts a rejoining
/// player's scoreboard again from zero, so the first boards after the
/// restart look like a fresh match and the stat-reset rule split the game.
///
/// Inside [`REJOIN_CONTINUE_WINDOW`] after [`mark_restart_recovered`], a
/// board continues the restored game when all of these hold:
/// * the board names the session's map (an actual read on this Tab, not
///   the session map filled in), so a different map still starts a new game;
/// * it is the identified own row with trusted cells and not a suspect row;
/// * at least two of elims, deaths and damage fell below the last accepted
///   board.
///
/// The pre-rejoin counters become the game's carry and are added to this
/// and every later board of the game, so the stored rows stay cumulative
/// (pre-crash plus post-rejoin) and never go down. Lower values never
/// overwrite the higher pre-crash ones, and the capture gate sees normal
/// growth. Only one continuation per window: once the carry is set, a later
/// drop goes through the normal reset rule.
pub(crate) fn rejoin_board(req: &CaptureRequest, facts: &BoardFacts<'_>) -> RejoinRead {
    let raw = facts.counters;
    if let Some(carry) = req.rejoin_carry {
        return RejoinRead {
            counters: add_carry(carry, raw, &facts.held),
            carry: Some(carry),
            continued: false,
        };
    }
    let unchanged = RejoinRead {
        counters: raw,
        carry: None,
        continued: false,
    };
    if !req.rejoin_window_open
        || !facts.trusted_cells
        || facts.suspect_row
        || facts.row_id.is_none()
    {
        return unchanged;
    }
    let read_map = facts
        .map_from_panel
        .filter(|m| !m.trim().is_empty())
        .or(Some(facts.parsed_map).filter(|m| !m.trim().is_empty()));
    let same_map = matches!(
        (known_label(req.session_map.as_deref()), known_label(read_map)),
        (Some(session), Some(read)) if session.eq_ignore_ascii_case(read)
    );
    let Some((prev, _)) = req.prev_gate else {
        return unchanged;
    };
    let before = prev.accepted;
    let fell = [
        (0, before.elims, raw.elims),
        (2, before.deaths, raw.deaths),
        (3, before.damage, raw.damage),
    ]
    .iter()
    .filter(|(col, b, c)| !facts.held[*col] && c < b)
    .count();
    if !same_map || fell < 2 {
        return unchanged;
    }
    RejoinRead {
        counters: add_carry(before, raw, &facts.held),
        carry: Some(before),
        continued: true,
    }
}

/// Test double for a stale start. Production uses [`begin_daemon_session`]
/// and does not await this.
///
/// The callback receives the dropped session id. `Some` settles that
/// session's unknown rows and still uploads decided rows. `None` is a
/// fresh skeleton or an empty start, and nothing is settled. A fresh
/// skeleton stays on disk. [`startup_session`] restores it afterwards,
/// including `last_game_open`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn recover_or_sync_active_game<F, Fut>(
    data_dir: &std::path::Path,
    store: &storage::LocalStore,
    sync_now: F,
) -> SessionState
where
    F: FnOnce(Option<String>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let dropped = retire_stale_skeleton(data_dir);
    if let Some(drop) = dropped.as_ref()
        && let Some(archive) = &drop.archive
    {
        archive_stale_hold(store, data_dir, archive).await;
    }
    let dropped = dropped.map(|drop| drop.session_id);
    sync_now(dropped.clone()).await;
    if let Some(session_id) = &dropped {
        tracing::info!(
            target: "scuffed_stat_tracker",
            session_id = %session_id,
            "synced unsynced rows after dropping a stale active game"
        );
    }
    startup_session(data_dir)
}

/// A result timestamp that cannot be mapped back onto this boot drops the
/// hint only. The rest of the recovered session still loads, including the
/// text-fallback lock: losing the hint must not reopen the accolade window.
pub(crate) fn recover_result_mark(
    outcome: Option<detect::MatchOutcome>,
    confirmed: bool,
    seen: Option<chrono::DateTime<Utc>>,
    to_instant: &impl Fn(chrono::DateTime<Utc>) -> Option<Instant>,
) -> Option<ResultMark> {
    let outcome = outcome.filter(|outcome| outcome.is_decided())?;
    let seen_at = to_instant(seen?)?;
    Some(ResultMark {
        outcome,
        confirmed,
        seen_at,
    })
}

/// Wall-vs-monotonic divergence treated as a suspend (m4). `Instant` is
/// CLOCK_MONOTONIC, which freezes during suspend, after resume every
/// in-memory window (grace, pending TTL, idle bound) silently believes no
/// time passed, so yesterday's post-match state can swallow today's first
/// game. Well above tick jitter and NTP step corrections, far below any
/// meaningful sleep.
pub(crate) const SUSPEND_RESET_GAP: std::time::Duration = std::time::Duration::from_secs(60);

/// Session state at process start, and again after a suspend.
///
/// `opened_at` is the instant the session opened, which is also when
/// `last_game_open` was set. Restoring it keeps the debounce across a
/// restart so the first ban or select does not split a session that is
/// still inside it.
pub(crate) fn startup_session(data_dir: &std::path::Path) -> SessionState {
    let active_game = recover_active_game(data_dir);
    let last_game_open = active_game.as_ref().map(|g| g.opened_at);
    SessionState {
        capture_count: 0,
        last_game_open,
        last_tab_capture: None,
        active_game,
        pending_outcome: None,
        word_outcome_streak: None,
        suspend_probe: (Instant::now(), Utc::now()),
        ocr_stability: detect::stability::FrameStability::default(),
        poll_ticks_skipped: 0,
        end_reel_wake_until: None,
        settle_closed_unknown: None,
        tab_probe: None,
        tab_press: None,
    }
}

/// Resume after a suspend. Drops the volatile windows and re-admits the
/// active game through the same recovery startup uses, so `last_game_open`
/// is `opened_at` again and a ban inside the debounce does not split.
/// Take the session [`begin_daemon_session`] already restored. Do not read
/// the skeleton a second time.
pub(crate) fn adopt_startup_session(st: &mut SessionState, restored: SessionState) {
    st.pending_outcome = None;
    clear_cadence_wakes(st);
    st.last_tab_capture = None;
    st.last_game_open = restored.last_game_open;
    st.active_game = restored.active_game;
    st.settle_closed_unknown = restored.settle_closed_unknown;
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn resume_after_suspend(st: &mut SessionState, data_dir: &std::path::Path) {
    st.pending_outcome = None;
    clear_cadence_wakes(st);
    st.last_tab_capture = None;
    let recovered = startup_session(data_dir);
    st.last_game_open = recovered.last_game_open;
    st.active_game = recovered.active_game;
    if st.active_game.is_none() {
        persist_active_game(data_dir, None);
    }
}

/// Delete a stale skeleton without uploading. The daemon loop schedules the
/// upload on `sync_task` so startup does not wait on HTTP. A held board is
/// returned so the caller can write it before that upload.
pub(crate) fn retire_stale_skeleton(data_dir: &std::path::Path) -> Option<StaleDrop> {
    match admit_persisted_game(data_dir) {
        ActiveAdmission::Stale { session_id, hold } => {
            let archive = hold.map(|hold| {
                // A fresh hold belongs to the next game. The skeleton map is
                // the previous game, so keeping it would archive that board
                // as Busan and a later GUI result would upload it as Busan.
                let (session_id, map, outcome) = if hold.imported {
                    (
                        session_id.clone(),
                        hold.map.unwrap_or_default(),
                        hold.outcome.to_string(),
                    )
                } else {
                    (
                        format!("{:016x}", rand_id()),
                        String::new(),
                        "unknown".into(),
                    )
                };
                tracing::warn!(
                    target: "scuffed_stat_tracker",
                    session_id = %session_id,
                    outcome = %outcome,
                    "stale restart kept a held board in local history"
                );
                ArchivedHold {
                    session_id,
                    counters: hold.counters,
                    hero: hold.hero,
                    map,
                    outcome,
                    at: hold.at.unwrap_or_else(Utc::now),
                }
            });
            persist_active_game(data_dir, None);
            tracing::info!(
                target: "scuffed_stat_tracker",
                session_id = %session_id,
                "dropped stale active game; holding its unknown rows and uploading decided ones"
            );
            Some(StaleDrop {
                session_id,
                archive,
            })
        }
        ActiveAdmission::Open(_) | ActiveAdmission::Absent => None,
    }
}

/// Write a stale hold into local history. An imported hold keeps the
/// persisted map and outcome. A fresh hold is an unknown session with an
/// empty map. Unknown rows are not uploaded. A Deathmatch map stays local
/// even if a later edit sets a result.
pub(crate) async fn archive_stale_hold(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    archive: &ArchivedHold,
) {
    let hero = archive.hero.clone().unwrap_or_default();
    if let Err(e) = store_held_board(
        store,
        data_dir,
        &archive.session_id,
        &hero,
        &archive.map,
        &archive.outcome,
        archive.counters,
        archive.at,
    )
    .await
    {
        tracing::warn!(
            target: "scuffed_stat_tracker",
            error = %e,
            session_id = %archive.session_id,
            "failed to keep a stale held board in local history"
        );
    }
}

/// Startup and resume. Drop a stale skeleton, schedule its upload, then
/// restore the session. Does not wait for HTTP.
pub(crate) async fn begin_daemon_session(
    data_dir: &std::path::Path,
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    store: &storage::LocalStore,
    client: Option<&sync::SyncClient>,
) -> SessionState {
    let dropped = retire_stale_skeleton(data_dir);
    if let Some(drop) = dropped.as_ref()
        && let Some(archive) = &drop.archive
    {
        archive_stale_hold(store, data_dir, archive).await;
    }
    let dropped = dropped.map(|drop| drop.session_id);
    // A stale skeleton is no longer the open game. Mark that session's
    // `unknown` rows synced locally and do not send them. A fresh skeleton
    // stays, and those rows stay unsynced until a result, a new-game
    // boundary, or the 6-hour bound. A clean shutdown keeps it.
    schedule_store_sync(sync_task, backoff, store, client, data_dir, dropped.clone());
    // A resume within 20 minutes must not write a pending imported hold.
    // The skeleton map is often empty, and writing it here would store the
    // board under that empty map. A stale start already archived the hold
    // above, with the persisted map and outcome.
    let mut st = startup_session(data_dir);
    // A game restored unfinished opens the rejoin window: a crash usually
    // drops the player out of the match, and the rejoin resets their stats.
    mark_restart_recovered(&mut st, Instant::now());
    st
}

/// Hero for a held board. Prefer the hero captured with the hold. Otherwise
/// use the newest stored row, the same fallback a later Tab uses for the
/// current hero.
pub(crate) async fn held_board_hero(
    store: &storage::LocalStore,
    session_id: &str,
    deferred_hero: Option<String>,
) -> String {
    if let Some(hero) = deferred_hero.filter(|hero| !hero.is_empty()) {
        return hero;
    }
    store
        .get_session_snapshots(session_id)
        .await
        .ok()
        .and_then(|rows| rows.last().map(|row| row.hero.clone()))
        .unwrap_or_default()
}

/// Write the imported hold onto its session. The caller clears the hold
/// only after this returns Ok.
pub(crate) async fn write_imported_hold(
    game: &ActiveGame,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let Some(counters) = game.deferred else {
        return Ok(());
    };
    let hero = held_board_hero(store, &game.session_id, game.deferred_hero.clone()).await;
    let map_name = game.map.clone().unwrap_or_default();
    let outcome = game.outcome.to_string();
    let played_at = game.deferred_at.unwrap_or_else(Utc::now);
    store_held_board(
        store,
        data_dir,
        &game.session_id,
        &hero,
        &map_name,
        &outcome,
        counters,
        played_at,
    )
    .await
}

pub(crate) fn clear_imported_hold(game: &mut ActiveGame) {
    game.session_created = true;
    game.deferred = None;
    game.deferred_hero = None;
    game.deferred_at = None;
    game.deferred_imported = false;
}
