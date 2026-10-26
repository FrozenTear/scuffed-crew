use stat_tracker::boundary::{self, ResultMark};
use stat_tracker::capture_gate::{self, Counters, GateState};
use stat_tracker::hero_auth::{self, HeroAuthState, HeroSource};
use stat_tracker::{
    capture, config, detect, mapcrops, names, ocr, parse, reader_apply, setup, shadow, storage,
    sync,
};

use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use chrono::Utc;
use surrealdb_types::Datetime as SurrealDatetime;
use tracing_subscriber::EnvFilter;

mod daemon;
use daemon::*;

/// Window for two word-OCR outcome reads to confirm each other. Sized to span
/// the accolade → rank-screen transition under a starved poller (measured 45s
/// between the last accolade tick and the first rank-screen tick) while still
/// bounding how long a single stray read stays actionable.
const OUTCOME_CONFIRM_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// Long-lived daemon dependencies and configuration, built once in `main` and
/// threaded as one value instead of 11 positional parameters whose adjacent
/// same-typed members were swap-prone (A1).
struct DaemonCtx {
    backend: capture::CaptureBackend,
    store: storage::LocalStore,
    sync_client: Option<sync::SyncClient>,
    player_name: Option<String>,
    capture_output: Option<String>,
    auto_detect: config::AutoDetectConfig,
    /// Quiet period before a finished game is closed. Already clamped so it
    /// is never shorter than post-match grace.
    finished_game_close: std::time::Duration,
    game_process_names: Vec<String>,
    portrait_matcher: Arc<detect::hero_portrait::PortraitMatcher>,
    collect_portraits: bool,
    dump_poll_frames: bool,
    /// Same gate as Tab OCR dumps (`debug_ocr` / `STAT_TRACKER_DEBUG_OCR`).
    /// When set, the poller writes Victory/Defeat evidence frames on confirm
    /// and first word-OCR streak, not every mid-match tick. Also gates the
    /// `debug/accepted` and `debug/rejected` rings (unblanked board pixels).
    debug_ocr: bool,
    data_dir: std::path::PathBuf,
    /// Consecutive scoreboard captures that parsed but resolved no map. Drives
    /// the `debug/mapmiss/` region dump (see `EMPTY_MAP_DUMP_THRESHOLD`).
    empty_map_reads: std::sync::atomic::AtomicUsize,
    /// Shadow digit recognizer (log only), `Some` when `shadow_recognizer`
    /// is on. Never joined: dropping it on exit drops the sender, so a slow
    /// shadow frame can not delay the final session save.
    shadow: Option<shadow::worker::ShadowWorker>,
    /// `reader = "new"`, or `ocr = "off"`, which implies it
    /// ([`stat_tracker::ocr_switch`]). Saved rows use the new reader: the
    /// six stats come from the digit reader only, and the hero, the top-bar
    /// map, the result words and the game-start screens from the reader
    /// packs where they are loaded (see `config::Config::reader`). The
    /// accolade map the game boundaries are given is the map pack's too
    /// ([`choose_accolade_map`]).
    use_new_reader: bool,
    /// `reader = "new"`: the member's own-name seeds, which find the own row
    /// without a name read (`<data_dir>/private/own-name`, alpha.7 O9).
    /// `None` under ocr-v1, which never opens or creates that folder.
    own_name: Option<Arc<std::sync::Mutex<detect::own_row::SeedStore>>>,
    /// Letter-template names. On unless config says `name_reader = false`.
    /// Independent of [`Self::use_new_reader`].
    names: names::NameSwitch,
}

/// Per-capture parameters decided by the session state machine at Tab time.
struct CaptureRequest {
    /// The Tab press that asked for this capture, on the wall and the
    /// monotonic clock: the report kit ages its portrait references from it.
    tab_at: stat_tracker::report::blank::CaptureTime,
    session_id: String,
    create_session: bool,
    game_outcome: detect::MatchOutcome,
    session_map: Option<String>,
    /// Where [`Self::session_map`] was read. The board case of a different-map
    /// Tab trusts the top bar and the accolade only.
    session_map_source: Option<boundary::MapSource>,
    map_candidates: Vec<String>,
    allow_banner_recovery: bool,
    /// The session's per-cell capture-gate state (last accepted + last raw
    /// counters) and how long ago that capture was accepted. Feeds both the
    /// whole-row game-split signal ([`stats_regressed`]) and the per-cell
    /// monotonic-hold + rate-cap gate ([`capture_gate::apply_gate`]).
    prev_gate: Option<(GateState, std::time::Duration)>,
    /// CG-4 C: career/portrait authority carried across captures of this game.
    hero_auth: HeroAuthState,
    /// An end screen (confirmed outcome or result streak) was already seen
    /// for this session. Logged when a stat split opens the next game.
    after_end_screen: bool,
    /// Time since the reset baseline. Rate checks use this, not the age of
    /// the last stored row.
    baseline_age: Option<std::time::Duration>,
    /// Progressed boards already accepted since the current hint.
    progressed_boards: u8,
    /// The deferred board was carried onto this session from the one that
    /// closed. A board deferred here is dropped when the next capture stays.
    deferred_imported: bool,
    /// See [`ActiveGame::reset_streak`].
    reset_streak: u32,
    /// See [`ActiveGame::reset_baseline`].
    reset_baseline: Option<GateState>,
    /// Player row that owns the reset baseline.
    baseline_row: Option<u32>,
    /// Unconfirmed decided word already on the session.
    hint: Option<detect::MatchOutcome>,
    pending_boundary: bool,
    awaiting_first_board: bool,
    /// See [`ActiveGame::text_fallback_locked`]. A locked text-fallback map
    /// is not replaced by a banner read either.
    text_fallback_locked: bool,
    /// A row of this session carries a member's own map choice
    /// ([`session_has_member_map_edit`]). Filled by the capture path, false
    /// from [`build_capture_request`].
    member_map_edit: bool,
    /// Board held off the previous session. Written onto the session this
    /// capture actually stores, at `carried_deferred_at`.
    carried_deferred: Option<Counters>,
    carried_deferred_hero: Option<String>,
    carried_deferred_at: Option<chrono::DateTime<Utc>>,
    /// Result-word reads collected for this game while `reader = "new"`.
    /// Not persisted. A restart drops them, and the next capture keeps the
    /// ocr-v1 result for that field.
    result_evidence: shadow::result::ResultEvidence,
    /// [`REJOIN_CONTINUE_WINDOW`] is open on this game and no rejoin has
    /// continued it yet ([`rejoin_board`]).
    rejoin_window_open: bool,
    /// See [`ActiveGame::rejoin_carry`].
    rejoin_carry: Option<Counters>,
    /// R9-2: this request is for the fresh session a Tab past the grace
    /// window would open, and this is the finished game's own request. The
    /// board decides which one the capture uses ([`finished_game_tab_probe`]).
    /// `None` from [`build_capture_request`].
    finished_game_probe: Option<Box<CaptureRequest>>,
    /// Tab was pressed for the game's last accepted board after its result
    /// was recorded (not just reported after it: a result can be confirmed
    /// while a capture runs). Only such a board can show the result was
    /// false ([`boundary::finished_game_tab`]).
    last_board_after_result: bool,
}

fn main() -> anyhow::Result<()> {
    // First, before any thread, log line or Tesseract use: libgomp reads
    // OMP_THREAD_LIMIT when it loads, so a start without it (a shell, the
    // app with no unit installed) is replaced here by the same binary with
    // it set.
    // Under the systemd unit the variable is already there and this returns.
    let omp = stat_tracker::omp::ensure_thread_limit();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;
    let result = runtime.block_on(run(omp));
    // Whatever is left of the one deadline. No signal yet means the full
    // deadline, including a self-restart after an update. Tasks that have
    // already finished do not wait it out.
    runtime.shutdown_timeout(stat_tracker::replace::shutdown_time_left());
    result
}

/// The game currently in progress. Opened when the poller sees a game-start
/// screen (map vote / hero select / ban) and reused for every Tab capture until
/// the next game starts, so captures taken across hero swaps all land in one
/// session. `outcome` is filled in when the poller reads the post-match screens
/// (or recovered from a captured frame), then back-filled onto the snapshots.
struct ActiveGame {
    /// Restored after a restart with a result screen already seen for it.
    /// Memory only. The first Tab after [`POST_MATCH_GRACE`] from that
    /// result starts a new game ([`should_start_fresh_session`]).
    restored_with_result: bool,
    /// When this daemon run restored the game from a previous run that did
    /// not finish it (a crash or restart mid-game). Memory only. Opens the
    /// [`REJOIN_CONTINUE_WINDOW`]: a same-map board whose stats dropped
    /// (the player rejoined and the game reset their scoreboard) continues
    /// this game instead of splitting it ([`rejoin_board`]).
    restart_recovered_at: Option<Instant>,
    /// Counters this game had before a rejoin reset its scoreboard. Added to
    /// every later board of this game, so stored rows stay cumulative for
    /// the whole match. Persisted in `active_game.json`: a second restart in
    /// the same game keeps adding it, and no new window opens on a game that
    /// already has one, so post-rejoin stats are never counted twice.
    rejoin_carry: Option<Counters>,
    session_id: String,
    /// alpha.8: the newest pending board's row with the reader's best guess
    /// in each unread stat column, those columns in `suspect_fields`. Stored
    /// when the game closes with no row ([`retire_active_game`]); a board
    /// that stores a row makes it moot.
    best_guess: Option<storage::PersonalMatch>,
    outcome: detect::MatchOutcome,
    /// The map actually being played, once a read names it. Never set from
    /// the map vote. [`Self::map_source`] says which read stored it. A
    /// full-board text fallback is recorded and is not a map for a poll
    /// split or a later different-map Tab until a top bar or an accolade
    /// agrees with it.
    map: Option<String>,
    /// Where [`Self::map`] was read. Absent until a read stores the map.
    /// A skeleton from before 0.4.19 has no source; recovery treats a map
    /// on that file as untrusted text.
    map_source: Option<boundary::MapSource>,
    /// Canonicalized names seen on the map-vote screen. The winner is
    /// unknowable at vote time, so these are CANDIDATES only, they constrain
    /// later OCR reads (a read that isn't one of them is a misread) but are
    /// never stored as the played map themselves.
    map_candidates: Vec<String>,
    /// Whether the `match_session` row has been created (on the first capture).
    session_created: bool,
    /// When `outcome` was recorded. Drives the post-match grace window: Tab
    /// presses shortly after the outcome (the post-match scoreboard) still
    /// belong to this game; later ones belong to the next.
    outcome_recorded_at: Option<Instant>,
    /// When the game was opened (start screen seen or first Tab).
    opened_at: Instant,
    /// Last recorded evidence the game is still this game (capture stored,
    /// outcome/map recorded). Bounds how long an unfinished session can
    /// absorb captures, see [`UNFINISHED_SESSION_IDLE`].
    last_activity: Instant,
    /// Per-cell capture-gate state (last accepted + last raw counters) from the
    /// most recent accepted capture. Scoreboard stats are cumulative within a
    /// match, so this drives both the detector-independent game-split signal
    /// (see [`stats_regressed`]) and the per-cell hold gate
    /// ([`capture_gate::apply_gate`]).
    gate: Option<GateState>,
    /// When `gate` was last updated (an accepted capture).
    last_stats_at: Option<Instant>,
    /// CG-4 C: portrait confirm-not-switch + career-ever-ok for this game.
    hero_auth: HeroAuthState,
    /// Result word seen for this session. An unconfirmed hint stays sealable
    /// until a second progressed board, or one progressed board after an arm.
    /// A confirmed result's grace starts when it is recorded, not when the
    /// word was first sighted.
    result_mark: Option<ResultMark>,
    /// Hero select after a board-followed hint. The first reset signal.
    /// The next fresh board splits and seals. A second progressed board, or
    /// one after this arm, drops the hint.
    pending_boundary: bool,
    /// Opened by a start screen; the first board has not been accepted.
    awaiting_first_board: bool,
    /// Consecutive fresh-match boards already seen. One is not a new game.
    /// A refresh, or counted progress, clears this. An unidentified or
    /// implausible row leaves it.
    reset_streak: u32,
    /// Counters the reset is measured from. Set when the first fresh-match
    /// board is held. A refresh replaces them with the accepted gate. An
    /// unidentified or implausible row leaves them.
    reset_baseline: Option<GateState>,
    /// Player row that owns [`Self::reset_baseline`]. A later board counts
    /// as a fresh reset only on this same row.
    baseline_row: Option<u32>,
    /// When [`Self::reset_baseline`] was taken. Rate checks measure from here.
    baseline_at: Option<Instant>,
    /// Progressed boards accepted since the current hint. The second drops it.
    progressed_boards: u8,
    /// First fresh-match board, held off the current session until a split
    /// stores it on the new one. `deferred_at` is that capture's own time.
    deferred: Option<Counters>,
    deferred_hero: Option<String>,
    deferred_at: Option<chrono::DateTime<Utc>>,
    /// Set when `deferred` was carried from the session that just closed.
    /// The first stored capture writes that board and then clears this,
    /// along with `deferred`. A deferral taken on this session stays false
    /// and is dropped if the next stored board does not split.
    deferred_imported: bool,
    /// A different result replaced the hint. The confirming accolade must
    /// not rewrite a text-fallback map after that. Cleared with the hint,
    /// except when recovery drops the hint because its timestamp cannot be
    /// mapped onto this boot: the lock stays.
    text_fallback_locked: bool,
    /// The text-fallback name a sure banner read replaced on this session
    /// ([`banner_replaces_text_fallback`]). The game's own accolade can still
    /// name it (the full-board text and the accolade agreed on it), so the
    /// poll path counts that accolade as this session's map, not as a
    /// different-map end screen that would split the game (F252-1).
    replaced_text_fallback_map: Option<String>,
    /// Result-word reads for `reader = "new"`. Memory only: a restart loses
    /// them and the next capture falls back the result field to ocr-v1.
    result_evidence: shadow::result::ResultEvidence,
    /// The recorded result may not be cleared by a Tab past the grace window
    /// (R9-2, [`finished_game_tab_probe`]): a member set it in the app, or it
    /// came back from a previous run whose source is not known. Memory only.
    outcome_locked: bool,
    /// When Tab was pressed for the board in [`Self::gate`]. A result
    /// confirmed while that capture ran lands before its report, so
    /// [`Self::last_stats_at`] (report time) can be after a result whose
    /// board is from before it. Memory only: a restored game has none, and
    /// none is never after a result ([`CaptureRequest::last_board_after_result`]).
    last_board_pressed_at: Option<Instant>,
}

/// On-disk mirror of [`ActiveGame`] (wall-clock timestamps instead of
/// `Instant`s). The session state machine is otherwise memory-only, and daemon
/// restarts are routine, without this, a restart mid-game either split the
/// game (new session per Tab) or merged it into whatever came next.
#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedGame {
    session_id: String,
    outcome: detect::MatchOutcome,
    #[serde(default)]
    map: Option<String>,
    /// Missing on skeletons written before 0.4.19. Recovery treats a map
    /// with no source as untrusted text ([`boundary::MapSource::TextFallback`]).
    /// A file with no map keeps `None`.
    #[serde(default)]
    map_source: Option<boundary::MapSource>,
    #[serde(default)]
    map_candidates: Vec<String>,
    session_created: bool,
    opened_at: chrono::DateTime<Utc>,
    last_activity: chrono::DateTime<Utc>,
    outcome_recorded_at: Option<chrono::DateTime<Utc>>,
    // Renamed from `last_stats: (u32,u32,u32)`; an old on-disk skeleton simply
    // defaults this to None (one game's cross-restart gate memory lost, the
    // file is best-effort recovery, never the capture itself).
    #[serde(default)]
    gate: Option<GateState>,
    #[serde(default)]
    last_stats_at: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    hero_auth: HeroAuthState,
    #[serde(default)]
    result_outcome: Option<detect::MatchOutcome>,
    #[serde(default)]
    result_confirmed: bool,
    #[serde(default)]
    result_seen_at: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    pending_boundary: bool,
    #[serde(default)]
    awaiting_first_board: bool,
    #[serde(default)]
    reset_streak: u32,
    #[serde(default)]
    reset_baseline: Option<GateState>,
    #[serde(default)]
    baseline_row: Option<u32>,
    #[serde(default)]
    deferred: Option<Counters>,
    #[serde(default)]
    deferred_hero: Option<String>,
    #[serde(default)]
    deferred_at: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    deferred_imported: bool,
    #[serde(default)]
    progressed_boards: u8,
    #[serde(default)]
    baseline_at: Option<chrono::DateTime<Utc>>,
    /// Missing on older skeletons. A file from before this field was not
    /// mid-replacement, so recovery leaves the window open.
    #[serde(default)]
    text_fallback_locked: bool,
    /// See [`ActiveGame::replaced_text_fallback_map`]. Missing on older
    /// skeletons, which had no banner rename.
    #[serde(default)]
    replaced_text_fallback_map: Option<String>,
    /// See [`ActiveGame::best_guess`]. Kept so a restart mid-game still
    /// stores the guess when the game closes. Missing on older skeletons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    best_guess: Option<storage::PersonalMatch>,
    /// See [`ActiveGame::rejoin_carry`]. Missing on older skeletons, which
    /// had no rejoin carry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rejoin_carry: Option<Counters>,
}

/// Apply one [`boundary::decide_poll`] result: record on the open session, or
/// close it and start the next one.
async fn apply_poll_decision(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    decision: boundary::PollDecision,
    now: Instant,
    use_new_reader: bool,
) {
    if let boundary::PollDecision::IgnoreContradictory { kept, ignored } = &decision {
        tracing::info!(
            kept = %kept,
            ignored = %ignored,
            session_id = st.active_game.as_ref().map(|g| g.session_id.as_str()),
            "result word ignored: it does not override the open session"
        );
    }
    let Some(current) = st.active_game.as_ref() else {
        return;
    };
    let mut state = current.boundary_state();
    let before = state.clone();
    let carried_deferred = current
        .deferred
        .map(|counters| (counters, current.deferred_hero.clone(), current.deferred_at));
    let pending_boundary = current.pending_boundary;
    let (candidates, end_screen) = match &decision {
        boundary::PollDecision::Open(open)
            if open.reason == boundary::CloseReason::EndScreenMap =>
        {
            (open.candidates.clone(), Some(open.clone()))
        }
        boundary::PollDecision::Open(open) => (open.candidates.clone(), None),
        _ => (Vec::new(), None),
    };
    let commit = boundary::commit_poll(&mut state, decision, now);
    if let Some(closed) = commit.closed {
        let new_id = format!("{:016x}", rand_id());
        match closed.reason {
            boundary::CloseReason::MapVote => {
                tracing::info!(
                    ?candidates,
                    session_id = %new_id,
                    "auto-detect: map vote: new game"
                );
            }
            boundary::CloseReason::HeroSelectOrBan => {
                tracing::info!(
                    session_id = %new_id,
                    "auto-detect: hero select/ban: new game (map vote missed)"
                );
            }
            boundary::CloseReason::EndScreenMap => {
                let split =
                    end_screen_split_log(pending_boundary, end_screen.as_ref(), closed.seal);
                remember_end_screen_split(&split);
                tracing::info!(
                    reason = closed.reason.log(),
                    session_id = %new_id,
                    split = %split,
                    "auto-detect: new game boundary"
                );
            }
            other => {
                tracing::info!(reason = other.log(), session_id = %new_id, "auto-detect: new game boundary");
            }
        }
        retire_active_game(st, store, data_dir, closed.seal, closed.reason.log()).await;
        let mut g = ActiveGame::open_at(new_id, state.outcome, candidates, now);
        g.apply_boundary_state(&state);
        // A different-map end screen is an accolade read. That is the new
        // session's map, and it is trusted for a later board-case split.
        if g.map.is_some() {
            g.map_source = Some(boundary::MapSource::Accolade);
        }
        // The deferred board belongs to the session being opened. The fresh
        // state has none; put it back so the next stored capture writes it
        // once. That capture clears the hold.
        if let Some((counters, hero, at)) = carried_deferred {
            g.deferred = Some(counters);
            g.deferred_hero = hero;
            g.deferred_at = at;
            g.deferred_imported = true;
        }
        st.active_game = Some(g);
        st.last_game_open = Some(now);
        clear_cadence_wakes(st);
        st.pending_outcome = None;
        persist_active_game(data_dir, st.active_game.as_ref());
        return;
    }
    let Some(g) = st.active_game.as_mut() else {
        return;
    };
    g.apply_boundary_state(&state);
    if let Some(outcome) = commit.recorded_outcome {
        g.touch_at(now);
        tracing::info!(
            ?outcome,
            session_id = %g.session_id,
            "auto-detect: outcome confirmed from post-match screens"
        );
        if g.session_created {
            if let Err(e) = store
                .set_session_outcome(&g.session_id, &g.outcome.to_string())
                .await
            {
                tracing::warn!(error = %e, "failed to back-fill session outcome");
            }
            refresh_snapshot(store, data_dir).await;
        }
    }
    if let Some(map) = commit.adopted_map {
        if !g.map_candidates.is_empty() && !g.map_candidates.contains(&map) {
            tracing::warn!(
                map = %map,
                candidates = ?g.map_candidates,
                "accolade map is not a vote candidate"
            );
        }
        g.map_source = Some(boundary::MapSource::Accolade);
        tracing::info!(
            map = %map,
            session_id = %g.session_id,
            "map recovered from accolade screen"
        );
        g.touch_at(now);
        if g.session_created {
            // `reader = "new"`: a member's own map choice is never renamed,
            // and the mode stored with it is never rewritten (the top-bar
            // rule of F252-3, for the accolade). That is a rule for the
            // rows that carry the choice, not for the session (F265-1): a
            // choice is stamped on the rows that exist when it is made, and
            // the rows captured after it, the game's newest row among them,
            // take the accolade map like any other row. With ocr-v1 every
            // row is written as before.
            let written = if use_new_reader {
                store
                    .set_session_map_keeping_member_choices(&g.session_id, &map)
                    .await
            } else {
                store.set_session_map(&g.session_id, &map).await.map(|()| 0)
            };
            match written {
                Ok(0) => {}
                Ok(kept) => tracing::info!(
                    session_id = %g.session_id,
                    kept,
                    "accolade map not written to the rows whose map a member chose"
                ),
                Err(e) => tracing::warn!(error = %e, "failed to set session map"),
            }
            refresh_snapshot(store, data_dir).await;
        }
    }
    if state != before {
        persist_active_game(data_dir, Some(g));
    }
}

/// Identity of a would-be new session (map-vote, Tab, or stat-regression split).
/// Missing map/hero are wildcards so a vote screen (no hero) can still match
/// an unfinished game that already learned its map from a Tab.
struct IncomingIdentity<'a> {
    map: Option<&'a str>,
    hero: Option<&'a str>,
    vote_candidates: &'a [String],
}

/// What [`stage_capture`] decided to write. Production and the night harness
/// both store from this, so the gate, the map source, and a carried board
/// cannot drift between them.
struct StagedCapture {
    target_session: String,
    create_session: bool,
    map_name: String,
    map_source: Option<boundary::MapSource>,
    outcome: detect::MatchOutcome,
    outcome_label: String,
    gate: capture_gate::GateOutcome,
    carried: Option<Counters>,
    carried_hero: Option<String>,
    carried_at: chrono::DateTime<Utc>,
    split: bool,
    /// Empty when no map was read. Callers store `None`, not `"Unknown"`.
    recorded_map: Option<String>,
    /// `map_name` is a sure banner read that replaces the session's
    /// untrusted text-fallback map ([`banner_replaces_text_fallback`]).
    replaced_text_fallback: bool,
}

/// Everything the blocking vision/OCR pass extracts from one Tab frame.
/// Replaces a 9-tuple whose adjacent same-typed fields were swap-prone.
struct FrameAnalysis {
    /// Result read off this frame only. A split stores this and never the
    /// inherited session outcome.
    frame_outcome: detect::MatchOutcome,
    /// Full-image OCR (hero/map name lookup); the per-cell rows carry stats.
    /// OCR site T7, run under `reader = "ocr-v1"` only: always the empty
    /// text under the new reader ([`full_board_text`]).
    ocr: Result<ocr::OcrResult, Box<dyn std::error::Error + Send + Sync>>,
    /// Column-calibrated per-cell OCR rows, one per scoreboard row. Under
    /// `reader = "new"` nothing is read into them: no name and no stat cell
    /// on any row (`ocr::unread_rows`, [`read_tab_rows`]).
    rows: Vec<ocr::RowOcrResult>,
    /// `reader = "new"`: the board was found empty and this Tab's rows
    /// were read with ocr-v1 instead ([`reader_apply::ocr_fallback_reason`]).
    /// The Tab is then read and stored as an ocr-v1 Tab.
    ocr_fallback: bool,
    /// `reader = "new"`: the digit, icon and banner reads of this Tab, which
    /// carry the trust gate and the stats. `None` under ocr-v1.
    board: Option<shadow::BoardRead>,
    /// Portrait template match: (hero file stem, confidence).
    portrait_hero: Option<(String, f64)>,
    /// Career-panel hero title (most reliable source when present). OCR site
    /// T5, read under `reader = "ocr-v1"` only: always `None` under the new
    /// reader, which uses `pack_hero` instead.
    career_hero: Option<String>,
    /// `reader = "new"`: the hero the heroes-v2 icon reads authenticate: the
    /// own row's and the career panel's portrait, each only when accepted,
    /// and never two different heroes ([`shadow::TabHeroes::authority`]). It
    /// has the career-panel title's authority
    /// ([`hero_auth::resolve_hero_new_reader`]). Always `None` under ocr-v1.
    pack_hero: Option<String>,
    /// Top-bar map label OCR, already canonicalized.
    map_from_panel: Option<String>,
    /// The top bar's raw label ([`top_bar_raw_label`]): the OCR text before
    /// canonicalization, or `ADLERSBRUNN` after a sure banner read of it.
    /// The Adlersbrunn alias check needs the raw label; a literal
    /// Eichenwalde read is not an alias.
    map_panel_raw: Option<String>,
    /// `map_from_panel` is a sure banner read (`reader = "new"`).
    map_from_banner: bool,
    /// Cropped scoreboard the portrait auto-collection reads from. On 5v5
    /// this includes the downward pad so the bottom portrait fits. The
    /// shadow worker gets the unpadded crop, and `debug/accepted` writes
    /// that same unpadded crop ([`debug_accepted_board`]).
    scoreboard: Arc<image::DynamicImage>,
    /// The player's row index. ocr-v1: by name match or brightness
    /// highlight. `reader = "new"`: by the own-name seeds, the highlight
    /// and the career hero's row ([`find_own_row`]); by a name read only
    /// when none of them names a row.
    player_row_idx: Option<usize>,
    /// The name match behind `player_row_idx`, with the name window that
    /// matched. `None` when the row came from the highlight, a seed, or
    /// the career hero, and no name was read.
    name_match: Option<parse::NameMatch>,
    /// Which reader produced `name_match`. `None` when this Tab did not
    /// read a name. `names-v1` is the pack. `ocr-v1` is Tesseract.
    name_recognizer: Option<names::NameRecognizer>,
    /// `reader = "new"`: how `player_row_idx` was found (seed, highlight,
    /// career hero, name fallback). `None` under ocr-v1.
    own_row: Option<detect::own_row::OwnRow>,
    /// `reader = "new"`: the own row's name line (the highlighted row's,
    /// or the career hero's), already through the seeding gate, for the
    /// own-name seed store. Pixels only; never logged, reported or uploaded.
    own_name_candidate: Option<detect::own_row::Candidate>,
    /// `reader = "new"`: what this board says about the trusted own-name
    /// seeds (confirmed, or naming another row than the career hero's).
    own_seed_verdict: detect::own_row::Verdict,
    /// Detected team size (5 or 6), portrait geometry depends on it.
    team_size: usize,
    /// The full frame, kept for the rejected-capture archive.
    frame: image::DynamicImage,
    /// Shadow recognizer input; `None` unless shadow mode is on.
    shadow_input: Option<shadow::worker::ShadowInput>,
}

/// Result of the blocking vision pass: a full analysis, or a cheap rejection
/// by the pre-OCR preflight before any Tesseract/portrait work ran.
enum FrameAnalysisOutcome {
    Analyzed(Box<FrameAnalysis>),
    /// The frame is not a scoreboard: neither row dips nor header labels, or
    /// the pitch failed and the table detector found no table. Carried back
    /// with the frame so it lands in debug/rejected. `scan` is logged with
    /// the rejection. A gameplay frame whose pitch does not settle stays on
    /// this path when it has no table.
    NotAScoreboard {
        outcome: detect::MatchOutcome,
        frame: image::DynamicImage,
        scan: detect::hero_portrait::RowScan,
    },
    /// Row structure is there, but the row pitch does not say 5v5 or 6v6
    /// without contradiction (`RowScan::checked_team_size`). Reading it with
    /// a guessed size cuts every row one slot off and stores neighbours'
    /// stats at high confidence, so the frame is rejected instead.
    TeamSizeUncertain {
        outcome: detect::MatchOutcome,
        frame: image::DynamicImage,
        scan: detect::hero_portrait::RowScan,
    },
    /// The stream had no new frame. Retried like a drawing board, and never
    /// written into debug/rejected.
    StreamMiss,
    /// The share dialog is still up. Retried, and not logged as a missed frame.
    ShareOpening,
    /// The screenshot before this one (a poll's) was still running after
    /// the Tab's lock wait. Retried while Tab is held, and ranked lowest so
    /// it never replaces a frame already grabbed.
    CaptureBusy,
    /// The share was declined or the permission dialog timed out. Not an
    /// error: the one warn already happened, and a later Tab stays quiet.
    Declined,
    /// The screenshot portal is not answering: its request was given up,
    /// or new requests are paused after one was. Not retried (a retry
    /// inside the pause gets the same answer), and not an error: the portal
    /// code logged its own WARN.
    PortalPaused,
}

/// What a Tab capture actually did, reported back to the session state machine.
struct CaptureReport {
    /// A pending board's row with the best guess in its unread columns
    /// ([`apply_best_guess`]). `None` for every other report.
    best_guess: Option<storage::PersonalMatch>,
    /// A snapshot row was written (and the session row created if this was the
    /// session's first capture). False = rejected by a trust gate.
    recorded: bool,
    /// Outcome stored on the snapshot, may have been recovered from the frame
    /// itself (a result word or the end title) when the game's outcome was still
    /// Unknown, in which case the caller back-fills it onto the session.
    outcome: detect::MatchOutcome,
    /// Map stored on the snapshot, if one was read, the caller adopts the
    /// first discovery onto the active game so the whole session shares it.
    map: Option<String>,
    /// Where `map` was read. [`resolved_map`] returns
    /// [`boundary::MapSource::TopBar`] whenever the top bar agrees with the
    /// stored name, trusted or not.
    /// `None` while `map` still holds the kept session map: the top bar was
    /// missing or disagreed, so this capture did not adopt a new source.
    map_source: Option<boundary::MapSource>,
    /// The session the snapshot was actually written to. Differs from the
    /// requested session when a stat regression split off a new game.
    session_id: String,
    /// The capture's stats regressed versus the session's previous accepted
    /// capture, a new game was detected and written to a fresh session; the
    /// caller must replace its active game to match.
    split: bool,
    /// First fresh-match board. Not written to the current session. The
    /// streak and baseline are what the next Tab uses. Also set for a live
    /// match, not only after a result.
    armed_reset: bool,
    /// The hold path. `plan_capture` does not return this for a stored row.
    ignore_row: bool,
    /// Fresh-match streak to keep when `split` is false. A refresh or counted
    /// progress sets this to zero. An unidentified or implausible row leaves it.
    reset_streak: u32,
    /// Baseline to keep when `split` is false and `refresh_baseline` is false.
    reset_baseline: Option<GateState>,
    /// Player row that owns `reset_baseline`.
    baseline_row: Option<u32>,
    /// This capture's accepted gate becomes the reset baseline.
    refresh_baseline: bool,
    /// A second progressed board, or one after an arm, arrived. The caller
    /// clears an unconfirmed hint. A confirmed mark is left alone.
    clear_hint: bool,
    /// First progressed board after a hint. The hint stays.
    count_progress: bool,
    /// This capture's hero came from the career panel.
    career_panel: bool,
    /// Counters of a deferred fresh-match board, stored on the new session
    /// when the reset commits.
    held_counters: Option<Counters>,
    held_hero: Option<String>,
    /// The per-cell capture-gate state after this capture (accepted + raw
    /// counters), carried into the next capture's monotonic-hold + rate-cap
    /// checks and the whole-row regression check. `None` when nothing was
    /// recorded.
    gate_state: Option<GateState>,
    /// Updated hero authority after this capture (carry into the next Tab).
    hero_auth: HeroAuthState,
    /// Hint sealed onto the session this capture closed, when `split` is set.
    seal: Option<detect::MatchOutcome>,
    close_reason: Option<boundary::CloseReason>,
    /// Wall time of a deferred board, taken when that capture was parsed.
    held_at: Option<chrono::DateTime<Utc>>,
    /// `map` is a sure banner read that replaces the session's text-fallback
    /// map ([`banner_replaces_text_fallback`]).
    map_replaces_text_fallback: bool,
    /// See [`ActiveGame::rejoin_carry`]. Set on a stored board of a game a
    /// rejoin continued ([`rejoin_board`]).
    rejoin_carry: Option<Counters>,
    /// R9-2: what the board said when this Tab was past the grace window of
    /// a finished game ([`TabProbe`]); `None` for every other capture, and
    /// for one whose frame was missed or rejected before its board was read.
    finished_game_tab: Option<boundary::FinishedGameTab>,
}

impl CaptureReport {
    /// A board the game has no complete row for yet (`reader = "new"`): it
    /// was read and went through the capture gate, so `gate_state` is set,
    /// but a stat column of this game has still never been read, and no
    /// row was written. The session keeps the gate, the boundary plan and
    /// the hero authority of the capture; the row appears with the first
    /// board that leaves no column unread. Every other report that is not
    /// recorded has no gate state.
    fn pending(&self) -> bool {
        !self.recorded && self.gate_state.is_some()
    }

    /// The capture took its board: a row was recorded, the board was held
    /// as a fresh-match board, or it is kept pending. This, not the retry
    /// loop's verdict on the frame, starts the Tab debounce and drops a
    /// press queued behind the capture (S5-CAP-2). A board a later gate
    /// refused (unreadable own row, the reader's trust gate) is not taken.
    fn board_taken(&self) -> bool {
        self.recorded || self.armed_reset || self.pending()
    }
}

/// Volatile session-tracking state owned by [`run_loop`]. Bundles the mutable
/// locals that the `select!` arms read and write so they travel as one value
/// instead of a fistful of parallel `let mut`s (QUAL-004). Purely
/// organizational, every field keeps the exact meaning it had as a standalone
/// local; no behavior depends on the grouping.
struct SessionState {
    /// Accepted-capture counter driving the periodic sync cadence
    /// (`SYNC_EVERY_N_CAPTURES`).
    capture_count: u32,
    /// When the current game was last (re)opened, debounces poller-driven
    /// new-game detection against `new_game_debounce`.
    last_game_open: Option<Instant>,
    /// Last accepted Tab capture, powers the Tab debounce.
    last_tab_capture: Option<Instant>,
    /// The game currently in progress, opened at the map-vote / hero-select
    /// screen, reused for every capture until the next game starts. Recovered
    /// from the previous run when the daemon restarted mid-game (crash, upgrade,
    /// systemd restart), so the restart neither splits the game into two
    /// sessions nor loses its outcome/map context.
    active_game: Option<ActiveGame>,
    /// Outcome detected by the poller while no game was open, applied to the
    /// next session that opens, if still fresh (`PENDING_OUTCOME_TTL`).
    pending_outcome: Option<(detect::MatchOutcome, Instant)>,
    /// Last result-word OCR read, for confirmation: a word outcome is only
    /// trusted once two reads agree within `OUTCOME_CONFIRM_WINDOW`, so a single
    /// hallucinated OCR read can't finish the open game with a wrong outcome.
    /// The reads may come from different screens (accolade → rank screen) and
    /// need not be consecutive ticks; garbage/transition frames don't reset it.
    /// `map` is this tick's accolade, or the map carried from the previous
    /// agreeing read when this tick has none.
    word_outcome_streak: Option<WordStreak>,
    /// Reference (monotonic, wall) pair for suspend detection
    /// (`SUSPEND_RESET_GAP`): refreshed each cmd tick; wall time advancing much
    /// further than the monotonic clock between ticks means the machine slept.
    suspend_probe: (Instant, chrono::DateTime<Utc>),
    /// Poll-tick OCR stability gate (PR-A): word/phase OCR only runs on crops
    /// that held still since the previous tick, so combat frames stop paying a
    /// Tesseract call every tick. Taken (`mem::take`) into the poll tick's
    /// `spawn_blocking` closure and moved back out through its return value.
    ocr_stability: detect::stability::FrameStability,
    /// Interval ticks skipped since the last performed poll capture (PR-B
    /// adaptive cadence): while [`poll_slow_mode`] holds, only every
    /// [`SLOW_POLL_DIVISOR`]th tick pays the screencopy.
    poll_ticks_skipped: u32,
    /// Deadline through which a POTG / end-reel sighting holds full poll
    /// cadence. None = no active wake. See [`END_REEL_WAKE`].
    end_reel_wake_until: Option<Instant>,
    /// Session id closed while its outcome was still unknown. The next
    /// command tick marks that session's rows synced locally and does not
    /// send them. Other unknown rows stay unsynced. Cleared when the
    /// settle is scheduled.
    settle_closed_unknown: Option<String>,
    /// The running Tab capture's board decides between two sessions
    /// ([`TabProbe`]).
    tab_probe: Option<TabProbe>,
    /// The press of the running Tab capture, taken by the report that lands
    /// for it ([`ActiveGame::last_board_pressed_at`]).
    tab_press: Option<Instant>,
}

/// The Tab request the capture task runs. Ages are measured from `now`,
/// which is [`Instant::now`] in the daemon and the injected clock in tests.
fn build_capture_request(g: &ActiveGame, opened_by_this_tab: bool, now: Instant) -> CaptureRequest {
    let banner_ok =
        opened_by_this_tab || now.saturating_duration_since(g.opened_at) >= MIN_BANNER_SESSION_AGE;
    // This session's own gate. A start screen's first board has none, and
    // the previous game is not an anchor for it.
    let prev_gate = match (g.gate, g.last_stats_at) {
        (Some(state), Some(at)) => Some((state, now.saturating_duration_since(at))),
        _ => None,
    };
    CaptureRequest {
        tab_at: stat_tracker::report::blank::CaptureTime {
            wall: Utc::now(),
            mono: now,
        },
        session_id: g.session_id.clone(),
        create_session: !g.session_created,
        game_outcome: g.outcome,
        session_map: g.map.clone(),
        session_map_source: g.map_source,
        map_candidates: g.map_candidates.clone(),
        allow_banner_recovery: banner_ok,
        prev_gate,
        hero_auth: g.hero_auth.clone(),
        after_end_screen: g.has_post_result(),
        baseline_age: g.baseline_at.map(|at| now.saturating_duration_since(at)),
        progressed_boards: g.progressed_boards,
        deferred_imported: g.deferred_imported,
        reset_streak: g.reset_streak,
        reset_baseline: g.reset_baseline,
        baseline_row: g.baseline_row,
        hint: g
            .result_mark
            .filter(|mark| !mark.confirmed && mark.outcome.is_decided())
            .map(|mark| mark.outcome),
        pending_boundary: g.pending_boundary,
        awaiting_first_board: g.awaiting_first_board,
        text_fallback_locked: g.text_fallback_locked,
        member_map_edit: false,
        carried_deferred: g.deferred,
        carried_deferred_hero: g.deferred_hero.clone(),
        carried_deferred_at: g.deferred_at,
        result_evidence: g.result_evidence.clone(),
        rejoin_window_open: g.rejoin_carry.is_none()
            && !g.finished()
            && g.result_mark.is_none()
            && g.restart_recovered_at
                .is_some_and(|at| now.saturating_duration_since(at) <= REJOIN_CONTINUE_WINDOW),
        rejoin_carry: g.rejoin_carry,
        finished_game_probe: None,
        last_board_after_result: g
            .last_board_pressed_at
            .zip(g.outcome_recorded_at)
            .is_some_and(|(pressed, result)| pressed > result),
    }
}

/// Whether this Tab closes a game that has a result and stored rows. That
/// game is then uploaded right away ([`open_fresh_for_tab_and_sync`]).
fn tab_closes_a_recorded_game(game: Option<&ActiveGame>, now: Instant) -> bool {
    game.is_some_and(|g| g.finished() && g.session_created) && should_start_fresh_session(game, now)
}

/// [`open_fresh_if_tab_starts_one`], then `sync_now` when the game this Tab
/// closed had a result and stored rows. Without this the closed game waited
/// for the next game's periodic sync, and the quiet-close timer only looks
/// at the open game, so a late Tab off the scoreboard left the last game of
/// the night unsent for hours (2026-10-17, Oasis). `sync_now` schedules
/// [`schedule_store_sync`]; it does not block the loop.
async fn open_fresh_for_tab_and_sync<F, Fut>(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    now: Instant,
    sync_now: F,
) -> bool
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let upload_closed = tab_closes_a_recorded_game(st.active_game.as_ref(), now);
    let opened = open_fresh_if_tab_starts_one(st, store, data_dir, now).await;
    if opened && upload_closed {
        sync_now().await;
    }
    opened
}

/// A Tab past the grace window of a finished game whose board decides
/// whether it is that game ([`finished_game_tab_probe`]). Memory only, and
/// only while that Tab's capture runs: [`settle_tab_probe`] takes it when
/// the report lands.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TabProbe {
    /// The finished game the board may continue.
    old_session: String,
    /// The session the Tab opens when it does not (the request's own).
    fresh_session: String,
    /// The Tab press: that session opens as of then, as it did before rc.9.
    press: Instant,
}

/// The capture request for the Tab pressed at `press`: the session choice of
/// [`open_fresh_for_tab_and_sync`], or past the grace window of a finished
/// game that its board may continue ([`finished_game_tab_probe`]), the fresh
/// session's request with the finished game's request inside. Nothing is
/// closed or opened then; [`settle_tab_probe`] does that when the report
/// lands.
async fn tab_capture_request<F, Fut>(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    press: Instant,
    sync_now: F,
) -> CaptureRequest
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    st.tab_press = Some(press);
    if finished_game_tab_probe(st.active_game.as_ref(), press) {
        let old = st.active_game.as_ref().expect("checked by the probe");
        let fresh_session = format!("{:016x}", rand_id());
        // Peeked, not taken: the pending outcome is taken when the fresh
        // session opens.
        let mut pending = st.pending_outcome;
        let fresh = tab_opened_game(
            fresh_session.clone(),
            take_fresh_pending(&mut pending, press),
            carried_deferred(Some(old)),
            press,
        );
        let mut req = build_capture_request(&fresh, true, press);
        req.finished_game_probe = Some(Box::new(build_capture_request(old, false, press)));
        tracing::info!(
            session_id = %old.session_id,
            outcome = %old.outcome,
            "Tab past the post-match grace window: the board decides whether this is the same game"
        );
        st.tab_probe = Some(TabProbe {
            old_session: old.session_id.clone(),
            fresh_session,
            press,
        });
        return req;
    }
    let opened_by_this_tab =
        open_fresh_for_tab_and_sync(st, store, data_dir, press, sync_now).await;
    build_capture_request(
        st.active_game.as_ref().expect("active_game set above"),
        opened_by_this_tab,
        press,
    )
}

/// The capture of a [`TabProbe`] Tab reported (`sid` is the request's
/// session, the fresh one). Returns the session the report is for.
///
/// * The board was the finished game's: that game stays open. The same
///   board ([`boundary::FinishedGameTab::SameBoard`]) is not stored and the
///   result stays. Still being played
///   ([`boundary::FinishedGameTab::StillPlaying`]), the capture stored the
///   board there, and the recorded result was false and is cleared
///   ([`unlatch_false_outcome`]).
/// * Anything else (another game's board, a board that cannot be compared,
///   a rejected or missed frame, a failed capture): the fresh session opens
///   as of the press and the finished game closes, as a Tab past the grace
///   window did before rc.9, with the upload of a closed game that had rows
///   ([`open_fresh_for_tab_and_sync`]). Not when another game took over
///   while the capture ran.
async fn settle_tab_probe<F, Fut>(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    sid: String,
    result: &Result<CaptureReport, String>,
    now: Instant,
    sync_now: F,
) -> String
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let Some(probe) = st.tab_probe.take_if(|probe| probe.fresh_session == sid) else {
        return sid;
    };
    let still_open = st
        .active_game
        .as_ref()
        .is_some_and(|g| g.session_id == probe.old_session);
    match result
        .as_ref()
        .ok()
        .and_then(|report| report.finished_game_tab)
    {
        Some(boundary::FinishedGameTab::SameBoard) => {
            tracing::info!(
                session_id = %probe.old_session,
                "Tab past the grace window shows the same board: same game, result kept"
            );
            probe.old_session
        }
        Some(boundary::FinishedGameTab::StillPlaying) => {
            // A result the member set while this capture ran stays (the
            // report re-seals it on the row).
            let unlocked = st.active_game.as_ref().is_some_and(|g| !g.outcome_locked);
            if still_open && unlocked {
                unlatch_false_outcome(st, store, data_dir, now).await;
            }
            probe.old_session
        }
        Some(boundary::FinishedGameTab::NewGame) | None => {
            if still_open || st.active_game.is_none() {
                let upload_closed = st
                    .active_game
                    .as_ref()
                    .is_some_and(|g| g.finished() && g.session_created);
                open_tab_game(st, store, data_dir, probe.fresh_session, probe.press).await;
                if upload_closed {
                    sync_now().await;
                }
            } else {
                tracing::warn!(
                    session_id = %sid,
                    "another game opened while this Tab was read: its report is not applied"
                );
            }
            sid
        }
    }
}

/// R9-2: the open game's board moved on past the grace window
/// ([`boundary::FinishedGameTab::StillPlaying`]), so its recorded result was
/// false (0.5.0-rc.8 report of 2026-10-22: DEFEAT taken off a red mid-match
/// frame). The result is cleared on the game and in the store, with the
/// result words read for this game (`reader = "new"` stamps them on every
/// later row otherwise), and the next confirmed result word sets it.
///
/// Result words read while the false result was held are not replayed:
/// those ticks were ignored (the game was past its end), and their reads
/// go with the cleared evidence. A real end screen already past is then not
/// read again, and the game ends without a result unless the member sets
/// one. A word streak from at or before the cleared result is dropped, so
/// a repeating misread needs two new reads again; a newer read (an end
/// screen seen while the false result was held) still pairs with the next
/// one inside the 60 s window.
///
/// Rows already uploaded with the false result keep it on the site:
/// `unknown` rows are not uploaded (`storage::LocalStore::get_unsynced`
/// selects `outcome != 'unknown'`, and the site skips them), so the site
/// is corrected only when a result is set on this game again.
async fn unlatch_false_outcome(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    now: Instant,
) {
    let Some(g) = st.active_game.as_mut() else {
        return;
    };
    if let Some(recorded) = g.outcome_recorded_at
        && st
            .word_outcome_streak
            .as_ref()
            .is_some_and(|streak| streak.seen_at <= recorded)
    {
        st.word_outcome_streak = None;
    }
    let cleared = g.outcome;
    g.outcome = detect::MatchOutcome::Unknown;
    g.outcome_recorded_at = None;
    g.result_mark = None;
    // Cleared with the hint ([`ActiveGame::text_fallback_locked`]).
    g.text_fallback_locked = false;
    g.result_evidence = Default::default();
    g.touch_at(now);
    tracing::warn!(
        session_id = %g.session_id,
        cleared = %cleared,
        "the board moved on after the result was recorded: the game is still being played, result cleared \
         (if its rows were already uploaded, the site keeps the old result until one is set again)"
    );
    if g.session_created
        && let Err(e) = store
            .set_session_outcome(&g.session_id, &detect::MatchOutcome::Unknown.to_string())
            .await
    {
        tracing::warn!(error = %e, "failed to clear the session outcome");
    }
    persist_active_game(data_dir, Some(g));
    refresh_snapshot(store, data_dir).await;
}

/// Open a session when this Tab is past grace or the unfinished idle bound.
/// Returns whether this Tab opened it. The deferred board of the session
/// being closed is carried onto the new one.
async fn open_fresh_if_tab_starts_one(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    now: Instant,
) -> bool {
    if !should_start_fresh_session(st.active_game.as_ref(), now) {
        return false;
    }
    open_tab_game(st, store, data_dir, format!("{:016x}", rand_id()), now).await;
    true
}

/// The deferred board a Tab-opened game carries from `previous`.
type CarriedDeferred = (Counters, Option<String>, Option<chrono::DateTime<Utc>>);

fn carried_deferred(previous: Option<&ActiveGame>) -> Option<CarriedDeferred> {
    previous.and_then(|g| {
        g.deferred
            .map(|counters| (counters, g.deferred_hero.clone(), g.deferred_at))
    })
}

/// The game a Tab opens as `session_id` at `now`: the pending outcome
/// `inherited`, and the deferred board of the game it replaces.
fn tab_opened_game(
    session_id: String,
    inherited: Option<detect::MatchOutcome>,
    carried: Option<CarriedDeferred>,
    now: Instant,
) -> ActiveGame {
    let mut opened = ActiveGame::open_at(
        session_id,
        inherited.unwrap_or(detect::MatchOutcome::Unknown),
        Vec::new(),
        now,
    );
    if let Some((counters, hero, at)) = carried {
        opened.deferred = Some(counters);
        opened.deferred_hero = hero;
        opened.deferred_at = at;
        opened.deferred_imported = true;
    }
    opened
}

/// Close the open game and open the Tab's game `session_id` at `now`.
async fn open_tab_game(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    session_id: String,
    now: Instant,
) {
    let carried = carried_deferred(st.active_game.as_ref());
    retire_active_game(st, store, data_dir, None, "superseded by Tab-opened game").await;
    let inherited = take_fresh_pending(&mut st.pending_outcome, now);
    st.active_game = Some(tab_opened_game(session_id, inherited, carried, now));
    st.last_game_open = Some(now);
    clear_cadence_wakes(st);
    persist_active_game(data_dir, st.active_game.as_ref());
}

/// What the vision pass (or a test stub) read off one Tab.
struct BoardFacts<'a> {
    counters: Counters,
    suspect: [bool; capture_gate::GATE_COLS],
    /// Per-cell parse succeeded. The capture gate treats false as low-trust.
    /// The boundary plan's `row_counts` is this bit: a raw-text fallback
    /// does not count as a stat-reset row. A row with held cells does
    /// (`boundary::plan_capture_held` leaves those columns out).
    trusted_cells: bool,
    /// `reader = "new"`: columns the digit reader did not vouch for. The
    /// capture has no number for them (`capture_gate::apply_gate_held`).
    /// All false under ocr-v1.
    held: [bool; capture_gate::GATE_COLS],
    /// `reader = "new"`: the own row is suspect (alpha.7 O9), so it may be
    /// another player's. The capture gate takes the read as untrusted, and
    /// the boundary plan lets it anchor the baseline but not split alone
    /// (`boundary::plan_capture_held`). False under ocr-v1.
    suspect_row: bool,
    row_id: Option<u32>,
    hero: &'a str,
    map_from_panel: Option<&'a str>,
    /// `map_from_panel` is a sure banner read, not the top-bar OCR.
    panel_from_banner: bool,
    parsed_map: &'a str,
    frame_outcome: detect::MatchOutcome,
}

/// Upper bounds for [`tests::capture_futures_stay_small`] (debug build,
/// rc.7: 15,168 and 1,640 bytes). Unboxing the KDE direct or portal grab
/// puts `capture_screen_output_for` at 4.6 KB or 28 KB.
#[cfg(all(test, debug_assertions))]
const HANDLE_CAPTURE_FUTURE_MAX: usize = 20 * 1024;
#[cfg(all(test, debug_assertions))]
const CAPTURE_FUTURE_MAX: usize = 3 * 1024;

/// One result-word sighting. `map` survives onto the confirming tick when
/// that tick's screen has no map of its own.
struct WordStreak {
    outcome: detect::MatchOutcome,
    seen_at: Instant,
    map: Option<String>,
}

/// Record this result-word tick and return the map the decision should see.
///
/// A tick with no map keeps the map from the previous read of the same
/// outcome, and only while that read is still inside
/// [`OUTCOME_CONFIRM_WINDOW`]. A different outcome, or an older read, drops
/// the carried map so it cannot relabel the session.
fn note_word_streak(
    streak: &mut Option<WordStreak>,
    outcome: detect::MatchOutcome,
    seen_at: Instant,
    this_map: Option<String>,
) -> Option<String> {
    let carried = streak.as_ref().and_then(|prior| {
        let age = seen_at.saturating_duration_since(prior.seen_at);
        (prior.outcome == outcome && age <= OUTCOME_CONFIRM_WINDOW)
            .then(|| prior.map.clone())
            .flatten()
    });
    let map = this_map.or(carried);
    *streak = Some(WordStreak {
        outcome,
        seen_at,
        map: map.clone(),
    });
    map
}

/// What one poll tick decided about the result word, before the session
/// machine runs. Production and the night harness both call this, so the
/// carried map and the confirm rule cannot drift.
struct ResolvedWordTick {
    signal: Option<detect::MatchOutcome>,
    signal_confirmed: bool,
    accolade_map: Option<String>,
}

fn resolve_word_tick(
    streak: &mut Option<WordStreak>,
    signal: Option<(detect::MatchOutcome, detect::match_end::OutcomeSource)>,
    seen_at: Instant,
    this_map: Option<String>,
) -> ResolvedWordTick {
    let prior = streak.as_ref().map(|prior| {
        (
            prior.outcome,
            seen_at.saturating_duration_since(prior.seen_at),
        )
    });
    let hit = poll_debug_hit(signal, prior, OUTCOME_CONFIRM_WINDOW);
    match signal {
        Some((outcome, source)) => {
            let agreed = matches!(hit, Some((PollDebugHit::Confirm, _)));
            let accolade_map = note_word_streak(streak, outcome, seen_at, this_map);
            if !agreed {
                tracing::debug!(
                    ?outcome,
                    ?source,
                    "result word read: awaiting agreeing read"
                );
            }
            ResolvedWordTick {
                signal: Some(outcome),
                signal_confirmed: agreed,
                accolade_map,
            }
        }
        None => ResolvedWordTick {
            signal: None,
            signal_confirmed: false,
            accolade_map: this_map,
        },
    }
}

#[cfg(test)]
#[test]
fn reused_frame_is_not_a_confirm_and_a_new_pair_is() {
    let victory = Some((
        detect::MatchOutcome::Victory,
        detect::match_end::OutcomeSource::ResultWord,
    ));
    let seen = Some(capture::SeenStream { id: 1, seq: 4 });
    let held_at = std::time::Instant::now();
    let mut streak = Some(WordStreak {
        outcome: detect::MatchOutcome::Victory,
        seen_at: held_at,
        map: None,
    });
    // The poll loop continues before `resolve_word_tick` when this is
    // true. Applying the word moves `seen_at`, so an inverted predicate
    // fails here.
    let skips = capture::reused_frame_skips_the_read(false, Some(4), Some(1), seen);
    assert!(skips);
    if !skips {
        let _ = resolve_word_tick(&mut streak, victory, std::time::Instant::now(), None);
    }
    assert_eq!(
        streak.as_ref().map(|prior| prior.seen_at),
        Some(held_at),
        "a reused frame must not move the streak"
    );
    let mut if_applied = Some(WordStreak {
        outcome: detect::MatchOutcome::Victory,
        seen_at: held_at,
        map: None,
    });
    let skips_fresh = capture::reused_frame_skips_the_read(
        true,
        Some(5),
        Some(1),
        Some(capture::SeenStream { id: 1, seq: 4 }),
    );
    assert!(!skips_fresh);
    if !skips_fresh {
        let applied = resolve_word_tick(&mut if_applied, victory, std::time::Instant::now(), None);
        assert!(applied.signal.is_some());
    }
    assert_ne!(
        if_applied.as_ref().map(|prior| prior.seen_at),
        Some(held_at),
        "a new frame must move the streak"
    );
    // Bounded by name: `analyze_frame`, which used to end the slice, is in
    // src/daemon/tab/vision.rs since 0.6 S3.
    let run_loop = crate::main_tests::fn_source("run_loop");
    let call = run_loop
        .find("reused_frame_skips_the_read(")
        .expect("the poll must call the skip helper");
    let after_call = &run_loop[call..];
    let kept = after_call
        .find("seen_stream = Some")
        .expect("a kept frame is recorded after the skip");
    let window = &after_call[..kept];
    assert!(
        window.contains("continue"),
        "deleting the continue after a reused frame must fail: {window}"
    );
    assert!(
        run_loop.contains("_ = stat_tracker::replace::shutdown_notified() =>"),
        "the daemon select must wake on shutdown_notified"
    );

    assert!(capture::poll_uses_frame(
        true,
        Some(5),
        Some(1),
        Some(capture::SeenStream { id: 1, seq: 4 })
    ));
    assert!(!capture::poll_uses_frame(
        true,
        Some(5),
        Some(1),
        Some(capture::SeenStream { id: 1, seq: 5 })
    ));
    assert!(capture::poll_uses_frame(
        true,
        Some(1),
        Some(2),
        Some(capture::SeenStream { id: 1, seq: 40 })
    ));
    let mut streak = None;
    let now = std::time::Instant::now();
    let first = resolve_word_tick(
        &mut streak,
        Some((
            detect::MatchOutcome::Victory,
            detect::match_end::OutcomeSource::ResultWord,
        )),
        now,
        None,
    );
    assert!(!first.signal_confirmed);
    let second = resolve_word_tick(
        &mut streak,
        Some((
            detect::MatchOutcome::Victory,
            detect::match_end::OutcomeSource::ResultWord,
        )),
        now,
        None,
    );
    assert!(second.signal_confirmed);
    assert_eq!(second.signal, Some(detect::MatchOutcome::Victory));
}

/// The words-pack result read one poll tick adds to the open game's result
/// evidence (`reader = "new"`, #253). Only a tick with an outcome signal
/// adds one, as before.
///
/// `taken` is the read the tick's signal detection already made of this
/// frame's result words (the accolade or rank screen, alpha.7 O11). It is
/// `shadow::read_result`'s read of the same frame, so it is the evidence
/// and the frame is not read again. Without one (an end title, or a word
/// OCR read because the pack lacks that screen) `read`
/// reads the frame here.
fn tick_result_read(
    use_new_reader: bool,
    has_signal: bool,
    taken: Option<shadow::result::ResultRead>,
    read: impl FnOnce() -> shadow::result::ResultRead,
) -> Option<shadow::result::ResultRead> {
    if !(use_new_reader && has_signal) {
        return None;
    }
    Some(taken.unwrap_or_else(read))
}

/// The [`boundary::PollInput`] for one open session. `map_trusted` is the
/// stored source, so a text fallback stays absent on the poll path.
fn poll_input_from_game<'a>(
    game: &'a ActiveGame,
    signal: Option<detect::MatchOutcome>,
    signal_confirmed: bool,
    accolade_map: Option<&'a str>,
    start_screen: Option<boundary::StartScreen>,
    block_map_vote: bool,
    now: Instant,
) -> boundary::PollInput<'a> {
    boundary::PollInput {
        outcome: game.outcome,
        result: game.result_mark,
        pending_boundary: game.pending_boundary,
        awaiting_first_board: game.awaiting_first_board,
        has_board: game.gate.is_some(),
        reset_streak: game.reset_streak,
        map: game.map.as_deref(),
        map_trusted: game.map_is_trusted(),
        hero: game.hero_auth.accepted_hero.as_deref(),
        signal,
        signal_confirmed,
        accolade_map: accolade_map.map(|read| {
            // The replaced text-fallback name is this session's own map
            // (F252-1).
            match (
                game.replaced_text_fallback_map.as_deref(),
                game.map.as_deref(),
            ) {
                (Some(old), Some(map)) if read.trim().eq_ignore_ascii_case(old.trim()) => map,
                _ => read,
            }
        }),
        start_screen,
        block_map_vote,
        deferred: game.deferred.is_some(),
        text_fallback_locked: game.text_fallback_locked,
        now,
    }
}

/// One poll tick after the word is resolved. [`run_loop`] and the night
/// harness both call this, so the post-tick glue cannot drift: the
/// start-screen block, [`poll_input_from_game`] (including the carried
/// accolade map), [`apply_poll_decision`], a confirmed word with no game
/// open, and [`open_detected_game`].
#[allow(clippy::too_many_arguments)]
async fn apply_poll_tick(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    tick: &ResolvedWordTick,
    start_screen: Option<boundary::StartScreen>,
    now: Instant,
    debounce: std::time::Duration,
    use_new_reader: bool,
) {
    let block_map_vote = start_screen.as_ref().is_some_and(|screen| {
        start_screen_blocked(
            st.active_game.as_ref(),
            st.last_game_open,
            now,
            debounce,
            screen,
        )
    });
    let decision = st.active_game.as_ref().map(|g| {
        boundary::decide_poll(&poll_input_from_game(
            g,
            tick.signal,
            tick.signal_confirmed,
            tick.accolade_map.as_deref(),
            start_screen.clone(),
            block_map_vote,
            now,
        ))
    });
    let opened = matches!(decision, Some(boundary::PollDecision::Open(_)));
    if let Some(decision) = decision {
        apply_poll_decision(st, store, data_dir, decision, now, use_new_reader).await;
    } else if tick.signal_confirmed
        && let Some(outcome) = tick.signal
    {
        // No game open yet, applies to the next session if one opens
        // within the TTL.
        st.pending_outcome = Some((outcome, now));
    }
    // An open session is only closed by `decide_poll`. This arm starts a
    // session when nothing is active yet.
    if !opened
        && st.active_game.is_none()
        && let Some(screen) = &start_screen
    {
        open_detected_game(st, data_dir, screen, now, debounce);
    }
}

/// How long direct capture and the screenshot portal wait after the Tab
/// press for the game to draw the scoreboard. The screen stream waits for a
/// buffer instead.
const DIRECT_TAB_DRAW_WAIT: std::time::Duration = std::time::Duration::from_millis(400);

/// Start the Tab capture for `press` (the key-event instant): pick or open
/// the session, then spawn the single-flight capture task, which reports on
/// `capture_tx`. Returns the task and its shared flight state.
async fn start_tab_capture(
    st: &mut SessionState,
    ctx: &Arc<DaemonCtx>,
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    sync_backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    capture_tx: &tokio::sync::mpsc::UnboundedSender<(String, Result<CaptureReport, String>)>,
    press: Instant,
    stream_hold: capture::KeyHold,
) -> (
    tokio::task::JoinHandle<()>,
    Arc<stat_tracker::tab_retry::TabFlight>,
) {
    let store = &ctx.store;
    let data_dir: &std::path::Path = &ctx.data_dir;
    let sync_client = ctx.sync_client.as_ref();
    // Session choice: reuse the active game (mid-game, or post-match
    // scoreboard within the grace window), or open a fresh one, inheriting
    // a still-fresh outcome the poller saw before any game was open. Past
    // the grace window of a finished game, the board may decide (R9-2).
    let req = tab_capture_request(st, store, data_dir, press, || {
        schedule_store_sync(sync_task, sync_backoff, store, sync_client, data_dir, None);
        std::future::ready(())
    })
    .await;
    let on_stream = capture::stream_route_active();
    let sid = req.session_id.clone();
    let tx = capture_tx.clone();
    let ctx_task = Arc::clone(ctx);
    let flight = stat_tracker::tab_retry::TabFlight::new(press);
    let task = tokio::spawn(stat_tracker::tab_retry::with_flight(
        Arc::clone(&flight),
        capture::with_tab_press(press, async move {
            // Direct capture and the screenshot portal wait for the overlay
            // to finish drawing, counted from the key event. The screen
            // stream waits for a buffer instead.
            if !on_stream {
                tokio::time::sleep_until(tokio::time::Instant::from_std(
                    press + DIRECT_TAB_DRAW_WAIT,
                ))
                .await;
            }
            // Keeps a paused screen stream running from the key event until
            // the Tab retry sequence ends (`capture::end_tab_stream_hold`).
            let result = capture::with_tab_stream_hold(stream_hold, handle_capture(&ctx_task, req))
                .await
                // `{:#}` keeps anyhow's context chain in the string.
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send((sid, result));
        }),
    ));
    (task, flight)
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    ctx: Arc<DaemonCtx>,
    early_sigterm: Option<tokio::signal::unix::Signal>,
) -> anyhow::Result<()> {
    // Ergonomic locals over the shared context; spawned tasks clone the Arc.
    let backend = &ctx.backend;
    let store = &ctx.store;
    let sync_client = ctx.sync_client.as_ref();
    let capture_output = ctx.capture_output.as_deref();
    let auto_detect = &ctx.auto_detect;
    let dump_poll_frames = ctx.dump_poll_frames;
    let debug_ocr = ctx.debug_ocr;
    let data_dir: &std::path::Path = &ctx.data_dir;

    let mut game_gate = detect::game_running::GameProcessGate::new(&ctx.game_process_names);
    // Starts false so a boot with the game already closed does not close a
    // share that was never opened. The close runs on the running edge only.
    let mut game_was_running = false;
    // The GUI's Stop button (and systemd) send SIGTERM, shut down as
    // gracefully as Ctrl+C, with a final sync.
    // `early_sigterm` is the same handler, registered before startup's wait
    // for the reader packs (only when that wait ran): a SIGTERM that came
    // since is waiting in it.
    let mut sigterm = match early_sigterm {
        Some(sigterm) => sigterm,
        None => tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
    };
    // From here a SIGTERM is a clean shutdown (run.marker removed). Before
    // this line the default action still applies. Tests wait for it.
    tracing::info!("signal handlers ready");
    tokio::spawn(async {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(term) => term,
                Err(_) => return,
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        stat_tracker::replace::note_shutdown_signal();
    });
    // Failed open used to wait only on Ctrl+C. SIGTERM was registered above
    // and never polled, so `systemctl stop` hit TimeoutStopSec then SIGKILL,
    // and exit 0 kept Restart=on-failure from trying again. The pid file
    // (acquired in main) made the GUI show "running" the whole time.
    let mut kbd = match open_keyboard_or_shutdown(&mut sigterm).await? {
        Some(stream) => stream,
        None => return Ok(()),
    };

    let poll_interval = auto_detect.poll_interval(ctx.use_new_reader);
    tracing::info!(
        poll_ms = poll_interval.as_millis() as u64,
        new_reader = ctx.use_new_reader,
        source = auto_detect.poll_interval_source(),
        "end-screen poll interval"
    );
    let new_game_debounce = std::time::Duration::from_secs(auto_detect.cooldown_secs);
    let finished_close = ctx.finished_game_close;
    let use_new_reader = ctx.use_new_reader;

    // Periodic sync runs as a spawned task so a slow or hung server can't
    // stall Tab capture, polling, or shutdown. Single-flight: while one sync
    // is in the air, the next trigger is skipped (the following one picks up
    // whatever it missed). Shutdown joins that task before the final upload
    // so the two never `mark_synced` the same ids. The client's HTTP timeout
    // bounds both the in-flight wait and the final upload.
    //
    // After a server/network failure the next *periodic* trigger waits on
    // `sync_backoff` (exponential, capped, reset on success). Shutdown does
    // not consult that clock, it still joins the in-flight task, then uploads
    // once. `sync_rev` compare-and-swap stays inside `try_sync_with`.
    let mut sync_task: Option<tokio::task::JoinHandle<()>> = None;
    let sync_backoff = Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    if let Some(client) = sync_client {
        let creds = client.credentials();
        match sync::auth_pause_reason(data_dir, &creds.server_url, &creds.token) {
            Some(sync::AuthPauseReason::DeletePending) => {
                sync_backoff
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record_delete_pending();
                tracing::error!(
                    "{}. Sync resumes when a new token is saved.",
                    sync::SYNC_STATS_DELETE_PENDING_STATUS
                );
            }
            Some(sync::AuthPauseReason::Rejected | sync::AuthPauseReason::Expired) => {
                sync_backoff
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record_auth_rejected();
                if let Some(status) =
                    sync::auth_pause_status(data_dir, &creds.server_url, &creds.token)
                {
                    tracing::error!("{status}. Sync resumes when the URL or token changes.");
                }
            }
            None => {}
        }
    }

    // Startup drops a stale skeleton and schedules the upload on `sync_task`.
    // It does not await HTTP. `last_game_open` still comes from
    // `startup_session`, so the debounce survives the restart. SIGTERM waits
    // on `finish_sync_on_shutdown`.
    let mut st =
        begin_daemon_session(data_dir, &mut sync_task, &sync_backoff, store, sync_client).await;
    if let Some(g) = &st.active_game {
        tracing::info!(
            session_id = %g.session_id,
            outcome = %g.outcome,
            "recovered open game from previous run"
        );
    }

    // Tab OCR also runs as a spawned task (single-flight), reporting back on
    // this channel. Awaited inline, one capture starved the poller for a
    // measured 45-70s, long enough to miss the ~3s VICTORY/DEFEAT banner and
    // the whole accolade screen, i.e. the outcome. The 400ms "let the game
    // render the scoreboard" wait sleeps inside the task too.
    let (capture_tx, mut capture_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, Result<CaptureReport, String>)>();
    let mut capture_task: Option<tokio::task::JoinHandle<()>> = None;
    // The running Tab capture until its report is handled, a newer press
    // waiting for that report, and the last release.
    let mut tabs = stat_tracker::tab_retry::TabKeys::default();
    // A Tab press that preempted a poll, handled by the keyboard arm on the
    // next turn of the loop.
    let mut pending_key: Option<Result<detect::TabKey, Box<dyn std::error::Error>>> = None;
    // The stream hold of a press queued behind a running capture. Dropped
    // when that capture reports and the press does not run.
    let mut queued_stream_hold: Option<capture::KeyHold> = None;
    // Taken as a Tab press preempts a poll, before the poll's grab is
    // dropped, so the stream does not pause and resume in between.
    let mut preempt_stream_hold: Option<capture::KeyHold> = None;

    let mut poll_timer = tokio::time::interval(poll_interval);
    poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // GUI command queue (manual outcome edits, session deletion), checked on
    // its own timer so edits apply even while no game is running.
    let mut cmd_timer = tokio::time::interval(tokio::time::Duration::from_secs(3));
    cmd_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut idle_sync_checked = Instant::now();
    let mut binary_checked = tokio::time::Instant::now();
    let mut logged_binary_wait = false;
    let mut exec_failed_at: Option<std::time::Instant> = None;
    let mut seen_stream: Option<capture::SeenStream> = None;
    let mut poll_failures = PollFailureStreak::default();

    loop {
        // `biased` polls in source order. Shutdown is first, so a signal
        // that is already waiting beats a poll tick that is also ready.
        // A capture or OCR already running in this arm is raced with
        // `stop_for_shutdown`. A poll write that has started is not: it
        // finishes inside the shared deadline (`finish_within_shutdown`).
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => {
                stat_tracker::replace::note_shutdown_signal();
                tracing::info!("shutting down");
                shutdown_from_signal(
                    &mut st,
                    &mut capture_task,
                    &mut capture_rx,
                    &mut sync_task,
                    sync_client,
                    &sync_backoff,
                    store,
                    data_dir,
                )
                .await;
                return Ok(());
            }
            _ = sigterm.recv() => {
                stat_tracker::replace::note_shutdown_signal();
                tracing::info!("SIGTERM received, shutting down");
                shutdown_from_signal(
                    &mut st,
                    &mut capture_task,
                    &mut capture_rx,
                    &mut sync_task,
                    sync_client,
                    &sync_backoff,
                    store,
                    data_dir,
                )
                .await;
                return Ok(());
            }
            // The background listener can see Ctrl+C before this loop's
            // own handler. `note_shutdown_signal` wakes this arm.
            _ = stat_tracker::replace::shutdown_notified() => {
                tracing::info!("shutting down");
                shutdown_from_signal(
                    &mut st,
                    &mut capture_task,
                    &mut capture_rx,
                    &mut sync_task,
                    sync_client,
                    &sync_backoff,
                    store,
                    data_dir,
                )
                .await;
                return Ok(());
            }
            result = pending_or_next(&mut pending_key, kbd.wait_tab()) => {
                // Held from a poll this event preempted. Dropped at the end
                // of this arm, once a press has its own hold, and on a
                // keyboard error too.
                let bridge = preempt_stream_hold.take();
                match result {
                    // A release stops the running capture's retries: a
                    // frame after it shows the game, not the board. It is
                    // kept for a press queued behind that capture.
                    Ok(key) if !key.down => tabs.on_release(key.at),
                    Ok(key) => {
                        // The kernel's key-event time, not when this loop got
                        // round to it (a poll can hold the loop for most of a
                        // second). The stream deadline is measured from it.
                        let press = key.at;
                        let running = game_gate.is_running();
                        capture::note_game_seen(game_was_running, running, data_dir);
                        if capture::share_close_due(
                            capture::stream_route_active(),
                            game_was_running,
                            running,
                            || capture::share_restorable(data_dir),
                        ) {
                            capture::close_if_open().await;
                        }
                        game_was_running = running;
                        if !running {
                            tracing::debug!("Tab ignored: game process not running");
                            continue;
                        }

                        // A capture whose task is gone without a report (a
                        // panic) must not hold later presses forever. A task
                        // that finished with its report still queued is still
                        // running: that report lands next.
                        let capture_gone = capture_task.as_ref().is_none_or(|t| t.is_finished())
                            && capture_rx.is_empty();
                        if let Some(press) = tabs.on_press(press, st.last_tab_capture, capture_gone) {
                            // Resume a paused screen stream from the key
                            // event, before the session choice below.
                            let hold = capture::stream_key_hold(press);
                            queued_stream_hold = None;
                            let (task, flight) = start_tab_capture(
                                &mut st,
                                &ctx,
                                &mut sync_task,
                                &sync_backoff,
                                &capture_tx,
                                press,
                                hold,
                            )
                            .await;
                            tabs.started(flight);
                            capture_task = Some(task);
                        } else if tabs.queued_press() == Some(press) {
                            // Queued by this press. An ignored or debounced
                            // press leaves an earlier queued hold alone.
                            queued_stream_hold = Some(capture::stream_key_hold(press));
                        }
                        drop(bridge);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "keyboard devices lost: attempting to reopen");
                        match detect::MultiKeyboardStream::open() {
                            Ok(new_kbd) => {
                                kbd = new_kbd;
                                tracing::info!("keyboard monitoring reopened");
                            }
                            Err(e2) => {
                                tracing::error!(error = %e2, "failed to reopen keyboard: exiting");
                                drain_capture(capture_task.take()).await;
                                let task = sync_task.take();
                                if let Some(client) = sync_client {
                                    drain_sync_then(task, || try_sync(store, client, data_dir, None)).await;
                                } else {
                                    drain_sync_then(task, || std::future::ready(())).await;
                                }
                                // Exit non-zero so Restart=on-failure starts a
                                // new process once a device is readable again.
                                // Exit 0 here looked like a clean stop.
                                anyhow::bail!(
                                    "keyboard devices lost and could not be reopened: {e2}"
                                );
                            }
                        }
                    }
                }
            }
            Some((sid, result)) = capture_rx.recv() => {
                let board_taken = result.as_ref().is_ok_and(CaptureReport::board_taken);
                // A Tab past the grace window of a finished game: its board
                // picked the session (R9-2).
                let sid = settle_tab_probe(&mut st, store, data_dir, sid, &result, Instant::now(), || {
                    schedule_store_sync(&mut sync_task, &sync_backoff, store, sync_client, data_dir, None);
                    std::future::ready(())
                })
                .await;
                // The report handler is the same function the night harness
                // calls. Sync stays here so a slow upload cannot hide inside it.
                if apply_capture_report(&mut st, store, data_dir, &sid, result, Instant::now()).await {
                    st.capture_count += 1;
                    maybe_spawn_periodic_sync(
                        &mut sync_task,
                        &sync_backoff,
                        store,
                        sync_client,
                        data_dir,
                        st.capture_count,
                    );
                    refresh_snapshot(store, data_dir).await;
                }
                // A Tab pressed while that capture ran runs now, unless the
                // capture took its board (it would read the same board). A
                // taken board starts the debounce; a refused one does not.
                let queued_hold = queued_stream_hold.take();
                if let Some(press) = tabs.on_report(&mut st.last_tab_capture, board_taken) {
                    let hold = queued_hold.unwrap_or_else(|| capture::stream_key_hold(press));
                    let (task, flight) = start_tab_capture(
                        &mut st,
                        &ctx,
                        &mut sync_task,
                        &sync_backoff,
                        &capture_tx,
                        press,
                        hold,
                    )
                    .await;
                    tabs.started(flight);
                    capture_task = Some(task);
                }
            }
            _ = poll_timer.tick(), if auto_detect.enabled => {
                // Mid-match: a Tab capture saturates the OCR pool, skip the
                // full poll so we do not contend (H3). During end-reel wake
                // that skip starves the ~3s Victory/Defeat window (2026-09-06:
                // Tab held, wake set, no poll result word). Run a cheap
                // outcome-only path instead; do not adopt Tab noscoreboard
                // Victory here (explicit follow-up).
                let tab_in_flight = capture_task.as_ref().is_some_and(|t| !t.is_finished());
                let now = Instant::now();
                if skip_poll_for_tab_in_flight(tab_in_flight, &st, now) {
                    tracing::debug!("poll tick skipped: Tab capture in flight");
                    continue;
                }
                let outcome_only = poll_outcome_only_while_tab_busy(tab_in_flight, &st, now);
                if outcome_only {
                    tracing::debug!(
                        "cheap outcome poll: Tab capture in flight during end-reel wake"
                    );
                }
                let running = game_gate.is_running();
                capture::note_game_seen(game_was_running, running, data_dir);
                if capture::share_close_due(
                    capture::stream_route_active(),
                    game_was_running,
                    running,
                    || capture::share_restorable(data_dir),
                ) {
                    capture::close_if_open().await;
                }
                game_was_running = running;
                if !running {
                    clear_cadence_wakes(&mut st);
                    st.ocr_stability.reset();
                    poll_failures.game_stopped();
                    continue;
                }

                // PR-B: adaptive cadence, mid-match, pay the screencopy only
                // on every SLOW_POLL_DIVISORth tick. The skipped ticks cost
                // nothing (no capture, no scans); full cadence resumes the
                // moment any end/start evidence lands in session state.
                // On Older screenshot and KDE direct the same skip runs out
                // of a match too, down to IDLE_SHOT_POLL_FLOOR.
                let divisor = poll_divisor(&st, Instant::now(), capture::shot_route(), poll_interval);
                if divisor > 1 {
                    st.poll_ticks_skipped += 1;
                    if st.poll_ticks_skipped < divisor {
                        tracing::trace!(divisor, "poll tick skipped: adaptive slow cadence");
                        continue;
                    }
                }

                // A Tab press preempts the poll's grab and analysis: the poll
                // is dropped (the next tick polls again) and the press is
                // kept for the keyboard arm, never lost. A release does not
                // preempt (`next_press` records it), so the outcome-only
                // poll in the ~3 s end-reel window survives letting go of Tab.
                // On a screenshot route the end-reel outcome-only poll waits
                // up to OUTCOME_POLL_LOCK_WAIT for a Tab grab's shot lock;
                // an ordinary poll that finds it held is skipped at once.
                let mut bridge = None;
                let grabbed = match stop_for_shutdown_or_key(
                    capture::poll_grab(backend, capture_output, outcome_only),
                    stat_tracker::replace::shutdown_notified(),
                    with_bridge_hold(
                        stat_tracker::tab_retry::next_press(&mut kbd, &mut tabs),
                        &mut bridge,
                        capture::stream_bridge_hold,
                    ),
                    &mut pending_key,
                )
                .await
                {
                    PollRace::Done(grabbed) => Ok(grabbed),
                    PollRace::Shutdown => Err(()),
                    PollRace::Preempted => {
                        preempt_stream_hold = bridge.take();
                        tracing::debug!("poll dropped during its grab: a Tab press comes first");
                        continue;
                    }
                };
                // Reset only for a poll that ran its grab: a dropped one, or
                // one turned away by a held screenshot lock, leaves the slow
                // cadence due on the next tick.
                if poll_resets_cadence(&grabbed) {
                    st.poll_ticks_skipped = 0;
                }
                match grabbed {
                    Err(()) => {
                        shutdown_from_signal(
                            &mut st,
                            &mut capture_task,
                            &mut capture_rx,
                            &mut sync_task,
                            sync_client,
                            &sync_backoff,
                            store,
                            data_dir,
                        )
                        .await;
                        return Ok(());
                    }
                    Ok(Ok(captured)) => {
                        if let Some((failed, over)) = poll_failures.recovered(Instant::now()) {
                            tracing::info!(
                                failed,
                                secs = over.as_secs(),
                                "poll capture works again"
                            );
                        }
                        if capture::reused_frame_skips_the_read(
                            captured.is_new,
                            captured.seq,
                            captured.stream,
                            seen_stream,
                        ) {
                            log_reused_frame_skip();
                            continue;
                        }
                        // Recorded only once this frame is read: a poll that
                        // a Tab drops leaves the frame unread.
                        let kept_frame = match (captured.stream, captured.seq) {
                            (Some(id), Some(seq)) => Some(capture::SeenStream { id, seq }),
                            _ => None,
                        };
                        let img = captured.image;
                        let dump_dir = dump_poll_frames.then(|| data_dir.join("debug").join("poll"));
                        // On-hit evidence (debug_ocr): confirm + first streak only.
                        // `--dump-poll-frames` still writes every tick as `poll_*`.
                        let on_hit_dir = debug_ocr.then(|| data_dir.join("debug").join("poll"));
                        let prior_streak = st
                            .word_outcome_streak
                            .as_ref()
                            .map(|streak| (streak.outcome, streak.seen_at.elapsed()));
                        // A copy: a poll that a Tab drops leaves the
                        // session's stability history as it was.
                        let mut stability = st.ocr_stability.clone();
                        // Dropping this join detaches the blocking OCR. That
                        // task does not write a match row. The row is written
                        // below, and only when shutdown has not begun and no
                        // Tab key event preempted it.
                        let analyzed = stop_for_shutdown_or_key(
                            tokio::task::spawn_blocking(move || {
                            let analyze_started = Instant::now();
                            if let Some(dir) = &dump_dir {
                                save_frame_ring(dir, "poll", &img, POLL_DUMP_KEEP);
                            }
                            // One RGBA→RGB conversion shared by the outcome + phase
                            // detectors (P6); title OCR still uses the original frame.
                            let rgb = img.to_rgb8();
                            // PR-A: the polled variants gate their OCR on the
                            // crop holding still across consecutive ticks,
                            // combat frames stop paying a Tesseract call per
                            // tick; static post-match/phase screens still read
                            // (one tick later at most).
                            // `reader = "new"` reads the end title from the
                            // words-v1 pack, once per tick, for the outcome
                            // signal and for the wake, and no OCR reads it
                            // (alpha.7 O2). A first sure read only wakes;
                            // the same word on the next tick is the signal.
                            // With no end-title templates loaded the titles
                            // fall back to OCR, as before O2. Only that
                            // fallback still reads the FINAL SCORE line (its
                            // gate), with the words-v1 template (alpha.7 O3).
                            let titles = detect::match_end::Titles::for_rgb(use_new_reader, &rgb);
                            // And it reads the accolade and rank-screen
                            // result word from the words-v1 pack, behind the
                            // same lit-crop gate and stability wait as the
                            // OCR it replaces (alpha.7 O11).
                            let words = detect::match_end::ResultWords::for_reader(use_new_reader);
                            let signal = detect::match_end::detect_outcome_signal_polled(
                                &img,
                                &rgb,
                                &mut stability,
                                detect::match_end::FinalScoreSource::for_reader(use_new_reader),
                                &titles,
                                &words,
                            );
                            // Cheap wake path: while Tab cell OCR is in
                            // flight, skip the phase and accolade-map reads
                            // so it keeps the pool. Outcome signal and
                            // end-reel refresh still run (same confirm /
                            // on-hit dump machinery below). Such a tick
                            // reads the outcome word but not the map, so two
                            // of them confirm with no map and seal onto the
                            // open session. Otherwise only a result-word
                            // tick reads the map ([`choose_accolade_map`]).
                            let accolade_map = choose_accolade_map(
                                use_new_reader,
                                outcome_only,
                                signal.map(|(_, source)| source),
                                || shadow::read_accolade_map(&rgb),
                                || detect::match_end::read_accolade_map(&img),
                            );
                            // `reader = "new"` confirms the vote, ban and
                            // select screens with the words-v1 phrase masks,
                            // not OCR (alpha.7 O7).
                            let phase = if outcome_only {
                                detect::GamePhase::Unknown
                            } else {
                                detect::match_start::detect_phase_polled(
                                    &img,
                                    &rgb,
                                    &mut stability,
                                    detect::match_start::PhaseSource::for_reader(use_new_reader),
                                )
                            };
                            // Wake hint only, does not confirm an outcome.
                            // Ban Heroes is a distinct hook (`detect_ban_screen`)
                            // and is excluded inside detect_end_reel; do not
                            // treat GamePhase::HeroBan as end_reel_wake_until.
                            // `reader = "new"`: a sure end title wakes, and
                            // the title card's phrase OCR does not run.
                            let end_reel =
                                detect::match_end::detect_end_reel_with(&img, &rgb, &titles);
                            // Result templates are not on the Tab frame. A
                            // polled outcome frame feeds the new reader's
                            // result. The read the signal came from is that
                            // evidence: the frame is not read twice (O11).
                            let result_read = tick_result_read(
                                use_new_reader,
                                signal.is_some(),
                                words.into_taken(),
                                || shadow::read_result(&img),
                            );
                            if let Some(dir) = &on_hit_dir
                                && let Some((kind, outcome)) =
                                    poll_debug_hit(signal, prior_streak, OUTCOME_CONFIRM_WINDOW)
                            {
                                save_frame_ring(
                                    dir,
                                    &poll_debug_prefix(kind, outcome),
                                    &img,
                                    POLL_DUMP_KEEP,
                                );
                            }
                            // Report kit (spec section 3): the rank screen's
                            // word box, cut from this same frame, only when
                            // the rank screen detector fired on it.
                            let rank_outcome_png = stat_tracker::report::kit::rank_outcome_png(
                                signal.map(|(_, source)| source),
                                &img,
                            );
                            stat_tracker::perf::record(
                                stat_tracker::perf::Stage::PollAnalyze,
                                analyze_started.elapsed(),
                            );
                            (signal, phase, accolade_map, end_reel, stability, result_read, rank_outcome_png)
                        }),
                            stat_tracker::replace::shutdown_notified(),
                            stat_tracker::tab_retry::next_press(&mut kbd, &mut tabs),
                            &mut pending_key,
                        )
                        .await;
                        let analyzed = match analyzed {
                            PollRace::Done(joined) => Ok(joined),
                            PollRace::Shutdown => Err(()),
                            PollRace::Preempted => {
                                tracing::debug!("poll dropped during its read: a Tab press comes first");
                                continue;
                            }
                        };
                        if let Some(seen) = kept_frame {
                            seen_stream = Some(seen);
                        }
                        let (signal, phase, accolade_map, end_reel, stability, result_read, rank_outcome_png) = match analyzed {
                            Err(()) => {
                                shutdown_from_signal(
                                    &mut st,
                                    &mut capture_task,
                                    &mut capture_rx,
                                    &mut sync_task,
                                    sync_client,
                                    &sync_backoff,
                                    store,
                                    data_dir,
                                )
                                .await;
                                return Ok(());
                            }
                            Ok(joined) => joined.unwrap_or_else(|_| {
                                (None, detect::GamePhase::Unknown, None, false, detect::stability::FrameStability::default(), None, None)
                            }),
                        };
                        if stat_tracker::replace::shutdown_requested() {
                            shutdown_from_signal(
                                &mut st,
                                &mut capture_task,
                                &mut capture_rx,
                                &mut sync_task,
                                sync_client,
                                &sync_backoff,
                                store,
                                data_dir,
                            )
                            .await;
                            return Ok(());
                        }
                        if let Some(png) = rank_outcome_png
                            && let Some(game) = st.active_game.as_ref()
                            // the rank screen follows the game it belongs
                            // to: a game quiet for longer gets no outcome crop
                            && game.last_activity.elapsed() <= RANK_OUTCOME_MAX_IDLE
                            && let Err(e) = stat_tracker::report::kit::record_outcome_png(
                                data_dir,
                                &game.session_id,
                                &png,
                            )
                        {
                            tracing::debug!(error = %e, "report kit: outcome crop not written");
                        }
                        st.ocr_stability = stability;
                        if let Some(read) = result_read
                            && let Some(game) = st.active_game.as_mut()
                        {
                            game.result_evidence.add(read);
                        }

                        if end_reel {
                            let already_awake = st
                                .end_reel_wake_until
                                .is_some_and(|t| Instant::now() < t);
                            st.end_reel_wake_until = Some(Instant::now() + END_REEL_WAKE);
                            if !already_awake {
                                tracing::info!(
                                    hold_secs = END_REEL_WAKE.as_secs(),
                                    "end-reel / POTG: holding full poll cadence"
                                );
                            }
                        }

                        // Every outcome (end title, accolade or rank screen)
                        // needs a second agreeing read within the
                        // confirmation window; no source acts on one tick
                        // (0.5.0-rc.9). The streak is recorded before
                        // the decision and carries a map only inside that
                        // window. [`resolve_word_tick`] is the same rule the
                        // night harness runs.
                        let now = Instant::now();
                        let tick = resolve_word_tick(
                            &mut st.word_outcome_streak,
                            signal,
                            now,
                            accolade_map,
                        );

                        // Boundaries live in `boundary::decide_poll`. A map vote
                        // or a hero ban after a board-followed hint splits and
                        // seals it. A hero select only arms. A confirmed word
                        // on a trusted different map opens the next session.
                        // A start screen still inside the debounce does not
                        // open another session.
                        let start_screen = match &phase {
                            detect::GamePhase::MapVote { maps } => {
                                let candidates: Vec<String> = maps
                                    .iter()
                                    .filter_map(|m| parse::canonical_map(m))
                                    .collect();
                                Some(boundary::StartScreen::MapVote { candidates })
                            }
                            detect::GamePhase::HeroSelect => Some(boundary::StartScreen::HeroSelect),
                            detect::GamePhase::HeroBan => Some(boundary::StartScreen::HeroBan),
                            _ => None,
                        };
                        // `retire_active_game` takes the game out of memory
                        // before the store calls return. Dropping this future
                        // on the signal would cancel the best-guess or the
                        // sealed outcome, and shutdown would have nothing
                        // left to save. The write runs to completion inside
                        // whatever of the shared deadline is left, then the
                        // loop shuts down.
                        let wrote = finish_within_shutdown(
                            async {
                                apply_poll_tick(
                                    &mut st,
                                    store,
                                    data_dir,
                                    &tick,
                                    start_screen,
                                    now,
                                    new_game_debounce,
                                    use_new_reader,
                                )
                                .await;
                            },
                            stat_tracker::replace::shutdown_notified(),
                            stat_tracker::replace::shutdown_time_left,
                        )
                        .await;
                        if wrote.is_err() || stat_tracker::replace::shutdown_requested() {
                            shutdown_from_signal(
                                &mut st,
                                &mut capture_task,
                                &mut capture_rx,
                                &mut sync_task,
                                sync_client,
                                &sync_backoff,
                                store,
                                data_dir,
                            )
                            .await;
                            return Ok(());
                        }
                    }
                    Ok(Err(e)) => {
                        // The game gate ran first: the game is running. A
                        // known state (a held lock, a share opening, declined
                        // or closed) is no failure and has its own log line.
                        match poll_failures.on_error(&e.to_string(), Instant::now()) {
                            PollErrorLog::Expected => {
                                tracing::trace!(error = %e, "poll capture skipped");
                            }
                            PollErrorLog::Warn => {
                                tracing::warn!(
                                    error = %e,
                                    failures = poll_failures.count,
                                    "poll capture failed while the game runs; repeats are logged at most every 5 minutes"
                                );
                            }
                            PollErrorLog::Trace => {
                                tracing::trace!(error = %e, failures = poll_failures.count, "poll capture failed");
                            }
                        }
                    }
                }
            }
            _ = cmd_timer.tick() => {
                // Auto-detect off never ticks the poll arm, so the share
                // still closes here when the game stops. The same edge, and
                // only when a restore token can reopen it.
                let running = game_gate.is_running();
                capture::note_game_seen(game_was_running, running, data_dir);
                if capture::share_close_due(
                    capture::stream_route_active(),
                    game_was_running,
                    running,
                    || capture::share_restorable(data_dir),
                ) {
                    capture::close_if_open().await;
                }
                game_was_running = running;
                if binary_checked.elapsed() >= std::time::Duration::from_secs(60) {
                    binary_checked = tokio::time::Instant::now();
                    if let Some(path) = stat_tracker::replace::replaced_binary_path() {
                        if st.active_game.is_some() {
                            if !logged_binary_wait {
                                tracing::info!(
                                    "tracker binary on disk was replaced; waiting until the current game is saved"
                                );
                                logged_binary_wait = true;
                            }
                        } else if !stat_tracker::replace::exec_retry_due(
                            exec_failed_at,
                            std::time::Instant::now(),
                            stat_tracker::replace::EXEC_RETRY_BACKOFF,
                        ) {
                            tracing::debug!(
                                "tracker binary on disk was replaced; waiting after a failed re-exec"
                            );
                        } else {
                            shutdown_from_signal(
                                &mut st,
                                &mut capture_task,
                                &mut capture_rx,
                                &mut sync_task,
                                sync_client,
                                &sync_backoff,
                                store,
                                data_dir,
                            )
                            .await;
                            let marker =
                                stat_tracker::report::marker::release_run_marker(data_dir);
                            let released_pid =
                                stat_tracker::replace::release_own_pid_file(data_dir);
                            if let Err(err) =
                                stat_tracker::replace::restart_for_replaced_binary(&path)
                            {
                                // A failed re-exec keeps this process. The
                                // systemd path exits and does not return.
                                tracing::error!(
                                    error = %err,
                                    "could not re-exec the new tracker; keeping this process"
                                );
                                exec_failed_at = Some(std::time::Instant::now());
                                if released_pid {
                                    stat_tracker::replace::restore_own_pid_file(data_dir);
                                }
                                let _ = stat_tracker::report::marker::restore_run_marker(
                                    data_dir,
                                    marker.as_ref(),
                                );
                            } else {
                                return Ok(());
                            }
                        }
                    } else {
                        logged_binary_wait = false;
                        exec_failed_at = None;
                    }
                }
                // Quiet-session timer. Runs on this existing tick so it does
                // not take a screenshot or run OCR, and it does not wait for
                // Tab. The upload is scheduled; it does not block the loop.
                close_quiet_session_and_sync(
                    &mut st,
                    store,
                    data_dir,
                    Instant::now(),
                    finished_close,
                    || {
                        schedule_store_sync(
                            &mut sync_task,
                            &sync_backoff,
                            store,
                            sync_client,
                            data_dir,
                            None,
                        );
                        std::future::ready(())
                    },
                )
                .await;
                schedule_settled_unknown(
                    &mut st,
                    &mut sync_task,
                    &sync_backoff,
                    store,
                    sync_client,
                    data_dir,
                );
                maybe_resume_sync_after_settings_change(sync_client, &sync_backoff, data_dir);
                // Suspend detection (m4): after a sleep, every Instant-based
                // window believes no time passed. Treat resume like a daemon
                // restart, drop the volatile windows and re-admit the active
                // game only through the wall-clock recovery bound (the on-disk
                // skeleton was persisted with correct wall times pre-suspend).
                // A skeleton past that bound is dropped. Decided rows upload.
                // Unknown rows for that session are held locally.
                let mono = st.suspend_probe.0.elapsed();
                let wall = (Utc::now() - st.suspend_probe.1).to_std().unwrap_or(mono);
                if wall > mono + SUSPEND_RESET_GAP {
                    tracing::info!(
                        gap_secs = (wall - mono).as_secs(),
                        "suspend/clock-jump detected: resetting session windows"
                    );
                    // Schedule the upload, including a stale skeleton. Then
                    // `resume_after_suspend` re-admits through `startup_session`,
                    // so the debounce comes back from `opened_at`. HTTP does
                    // not run on this tick.
                    let restored = begin_daemon_session(
                        data_dir,
                        &mut sync_task,
                        &sync_backoff,
                        store,
                        sync_client,
                    )
                    .await;
                    adopt_startup_session(&mut st, restored);
                }
                st.suspend_probe = (Instant::now(), Utc::now());

                apply_queued_commands(
                    &mut st,
                    store,
                    data_dir,
                    &mut sync_task,
                    &sync_backoff,
                    sync_client,
                )
                .await;
                maybe_spawn_idle_sync(
                    &mut sync_task,
                    &sync_backoff,
                    store,
                    sync_client,
                    data_dir,
                    &mut idle_sync_checked,
                    Instant::now(),
                )
                .await;
            }
        }
    }
}

/// Under `reader = "new"`, the accolade map was read with OCR because no
/// map pack is loaded (none synced yet, or none that loads). Logged once
/// per process (F265-3), like the FINAL SCORE line's fallback. With OCR off
/// there is no such fallback and no such line ([`choose_accolade_map`]).
/// Returns whether this call logged (tests).
fn warn_accolade_map_ocr_fallback() -> bool {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let first = !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed);
    if first {
        tracing::warn!(
            "reader = \"new\" but no map pack is loaded; \
             the accolade map is read with OCR"
        );
    }
    first
}

/// The map on this poll tick's accolade screen, for the session machine.
///
/// Only a result-word tick reads the map. The rank screen does not, and a
/// banner tick is the gameplay HUD under a result flash, so that crop is not
/// a map. A tick taken while Tab cell OCR is in flight (`outcome_only`) reads
/// no map either.
///
/// With `reader = "ocr-v1"` the map is the title OCR of the strip, as
/// before. With `reader = "new"` it is the map pack's read of the accolade
/// map line ([`shadow::read_accolade_map`]): sure and storable, or nothing.
/// When the pack read the frame and abstains there is no map; the OCR is
/// not run in its place, and a panic in the pack reader is an abstain too,
/// so it never costs the tick its result word. When no map pack is loaded
/// nothing read the frame, and the OCR reads it as under ocr-v1, with one
/// warning ([`warn_accolade_map_ocr_fallback`]): the rule of the top bar,
/// the FINAL SCORE line and the phase titles without their packs. With
/// classic OCR off there is no OCR to read it: no map pack means no map,
/// and the OCR read is not asked for (the switch does not grant off
/// without the map pack, so this is a pack that went away). What the
/// session does with the map is the same for both readers
/// (`boundary::decide_poll`).
fn choose_accolade_map(
    use_new_reader: bool,
    outcome_only: bool,
    signal: Option<detect::match_end::OutcomeSource>,
    pack: impl FnOnce() -> shadow::AccoladeRead,
    ocr: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if outcome_only || signal != Some(detect::match_end::OutcomeSource::ResultWord) {
        return None;
    }
    if !use_new_reader {
        return ocr();
    }
    let read = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(pack)) {
        Ok(shadow::AccoladeRead::Map(read)) => read,
        Ok(shadow::AccoladeRead::Abstain) => return None,
        // No map pack, and classic OCR is off: held. `ocr` is not called.
        Ok(shadow::AccoladeRead::NoPack) if ocr::gate::is_off() => return None,
        Ok(shadow::AccoladeRead::NoPack) => {
            warn_accolade_map_ocr_fallback();
            return ocr();
        }
        Err(_) => {
            tracing::warn!("accolade map reader panicked; no accolade map for this tick");
            return None;
        }
    };
    tracing::info!(
        map = %read.name,
        score = read.score,
        margin = read.margin,
        recognizer = shadow::banner::ACCOLADE_MAP_RECOGNIZER_ID,
        "map read from accolade screen (map pack)"
    );
    Some(read.name)
}

/// A prepared analysis for tests of [`handle_capture`]. Production never sets it.
#[cfg(test)]
struct InjectedCapture {
    analysis: FrameAnalysis,
    board: Option<shadow::BoardRead>,
}

#[cfg(test)]
static INJECTED_CAPTURE: std::sync::Mutex<Option<InjectedCapture>> = std::sync::Mutex::new(None);

/// Held by each test that injects a capture, so two such tests never take
/// each other's injection.
#[cfg(test)]
static INJECTED_CAPTURE_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn take_injected_capture() -> Option<(FrameAnalysis, Option<shadow::BoardRead>)> {
    #[cfg(test)]
    {
        return INJECTED_CAPTURE
            .lock()
            .unwrap()
            .take()
            .map(|item| (item.analysis, item.board));
    }
    #[cfg(not(test))]
    None
}

/// A screen grab error for tests of [`handle_capture`]'s missed-capture
/// arms, returned by the next [`grab_tab_attempt`]. Production never sets it.
#[cfg(test)]
static INJECTED_GRAB_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn take_injected_grab_error() -> Option<String> {
    #[cfg(test)]
    {
        return INJECTED_GRAB_ERROR.lock().unwrap().take();
    }
    #[cfg(not(test))]
    None
}

/// Why the new reader refuses an analysed Tab frame's board: the trust
/// gate `handle_capture` runs on it ([`reader_apply::board_refusal`]), here
/// inside the retried attempt so a refused board is grabbed again while
/// Tab is held (S5-CAP-2). `None` under ocr-v1, for a board read with
/// ocr-v1 instead, and for a board the reader vouches for.
fn reader_refusal(analysis: &FrameAnalysis, use_new_reader: bool) -> Option<&'static str> {
    if !use_new_reader || analysis.ocr_fallback {
        return None;
    }
    match &analysis.board {
        Some(board) => reader_apply::board_refusal(board, analysis.team_size),
        None => reader_apply::board_refusal(
            &shadow::reader::not_found(shadow::BoardStatus::NotFound),
            analysis.team_size,
        ),
    }
}

/// How a Tab grab ended, for the retry loop. A frame with no full
/// scoreboard (the preflight's, or a board the new reader refuses), a Tab
/// that got no fresh stream frame, and a share that is still opening are
/// tried again ([`stat_tracker::tab_retry::TabResult::retries`]).
/// A declined share, a team size the pitch does not settle, and an analysed
/// frame the reader takes are not.
fn tab_result(
    outcome: &FrameAnalysisOutcome,
    use_new_reader: bool,
) -> stat_tracker::tab_retry::TabResult {
    use stat_tracker::tab_retry::TabResult;
    match outcome {
        FrameAnalysisOutcome::Analyzed(a) if reader_refusal(a, use_new_reader).is_some() => {
            TabResult::BoardRefused
        }
        FrameAnalysisOutcome::Analyzed(_) => TabResult::Accepted,
        FrameAnalysisOutcome::NotAScoreboard { .. } => TabResult::NotAScoreboard,
        FrameAnalysisOutcome::StreamMiss
        | FrameAnalysisOutcome::ShareOpening
        | FrameAnalysisOutcome::CaptureBusy => TabResult::NoFrameYet,
        FrameAnalysisOutcome::Declined
        | FrameAnalysisOutcome::PortalPaused
        | FrameAnalysisOutcome::TeamSizeUncertain { .. } => TabResult::Final,
    }
}

/// Why a Tab grab is tried again, for the log.
fn tab_retry_reason(outcome: &FrameAnalysisOutcome, use_new_reader: bool) -> &'static str {
    match outcome {
        FrameAnalysisOutcome::Analyzed(a) if reader_refusal(a, use_new_reader).is_some() => {
            "the scoreboard reader refused the board (see board_refusal above)"
        }
        FrameAnalysisOutcome::NotAScoreboard { .. } => {
            "rejected by the scoreboard check (the frame shows no full scoreboard)"
        }
        FrameAnalysisOutcome::StreamMiss => "no fresh stream frame arrived",
        FrameAnalysisOutcome::ShareOpening => "the share is still opening",
        FrameAnalysisOutcome::CaptureBusy => "the previous screenshot was still running",
        FrameAnalysisOutcome::Declined
        | FrameAnalysisOutcome::PortalPaused
        | FrameAnalysisOutcome::TeamSizeUncertain { .. }
        | FrameAnalysisOutcome::Analyzed(_) => "not retried",
    }
}

/// How useful a Tab outcome is, so a later grab never replaces a better
/// one. Higher is better: analysed and taken by the reader (5), analysed
/// but refused by the reader (4), team size unsure (3), not a scoreboard
/// (2), stream miss (1), share still opening or declined, a busy lock or a
/// paused portal (0). Any analysed board outranks the rejects, as in rc.7:
/// it carries the header outcome fallback and the own-seed verdict, which
/// a reject does not. All of them carry the frame's outcome (the report
/// back-fills an end-of-match VICTORY or DEFEAT from a reject too), and the
/// rank does not look at it: a refused board with no outcome outranks an
/// unsure team size that read one, and that read is lost. An outcome-aware
/// tie-break is left for 0.6.
fn tab_outcome_rank(outcome: &FrameAnalysisOutcome, use_new_reader: bool) -> u8 {
    match outcome {
        FrameAnalysisOutcome::Analyzed(a) if reader_refusal(a, use_new_reader).is_some() => 4,
        FrameAnalysisOutcome::ShareOpening
        | FrameAnalysisOutcome::Declined
        | FrameAnalysisOutcome::CaptureBusy
        | FrameAnalysisOutcome::PortalPaused => 0,
        FrameAnalysisOutcome::StreamMiss => 1,
        FrameAnalysisOutcome::NotAScoreboard { .. } => 2,
        FrameAnalysisOutcome::TeamSizeUncertain { .. } => 3,
        FrameAnalysisOutcome::Analyzed(_) => 5,
    }
}

fn log_reused_frame_skip() {
    static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let debug = {
        let mut last = LAST.lock().unwrap_or_else(|err| err.into_inner());
        let now = Instant::now();
        let debug =
            capture::reused_frame_skip_is_debug(*last, now, capture::REUSED_FRAME_LOG_WINDOW);
        if debug {
            *last = Some(now);
        }
        debug
    };
    if debug {
        tracing::debug!("poll skipped: stream frame already seen");
    } else {
        tracing::trace!("poll skipped: stream frame already seen");
    }
}

async fn grab_tab_attempt(
    backend: &capture::CaptureBackend,
    output: Option<&str>,
    attempt: usize,
) -> Result<capture::CapturedFrame, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(err) = take_injected_grab_error() {
        return Err(err.into());
    }
    if attempt != 0 {
        // A retry is not a new key press: on the stream it takes the next
        // buffer that arrives, with no second overlay delay.
        let press = capture::tab_retry_press(std::time::Instant::now());
        let grab = capture::capture_screen_output_for(backend, output, capture::CaptureKind::Tab);
        return capture::with_tab_press(press, grab).await;
    }
    // The first attempt keeps the original key, unless it starts so late
    // (a press queued behind another capture) that the key's frame budget
    // is gone: then it takes the next buffer, as a retry does. While the
    // share is still opening it waits up to 5 seconds for a fresh frame, not
    // the 1200 ms frame budget. If that open uses up the frame budget, the
    // grab runs again from now.
    let key = capture::tab_press().unwrap_or_else(std::time::Instant::now);
    let press = capture::first_grab_press(key, std::time::Instant::now());
    capture::with_tab_press(
        press,
        capture::first_tab_grab(
            press,
            || press.elapsed(),
            std::time::Instant::now,
            || capture::capture_screen_output_for(backend, output, capture::CaptureKind::Tab),
            || tokio::time::sleep(std::time::Duration::from_millis(50)),
        ),
    )
    .await
}

async fn handle_capture(ctx: &DaemonCtx, mut req: CaptureRequest) -> anyhow::Result<CaptureReport> {
    if ctx.use_new_reader {
        req.member_map_edit = session_has_member_map_edit(&ctx.store, &req).await;
    }
    // QUAL-002 follow-up (multi-file seam split, parked for USER per DR-1 A4)
    // Ergonomic locals over the context/request (the body predates A1).
    let backend = &ctx.backend;
    let store = &ctx.store;
    let player_name = ctx.player_name.as_deref();
    let capture_output = ctx.capture_output.as_deref();
    let collect_portraits = ctx.collect_portraits;
    let data_dir: &std::path::Path = &ctx.data_dir;
    let session_id: &str = &req.session_id;
    let game_outcome = req.game_outcome;
    let session_map = req.session_map.as_deref();
    let allow_banner_recovery = req.allow_banner_recovery;

    let injected = take_injected_capture();
    let (analysis, board_override) = if let Some(injected) = injected {
        injected
    } else {
        tracing::info!("Tab detected: capturing screen (hold Tab to keep scoreboard visible)");
        // A rejected Tab frame is captured again after a short wait, unless
        // a newer Tab press takes over (`stat_tracker::tab_retry`).
        let own_seeds = own_name_seeds(ctx.own_name.as_ref());
        let names = ctx.names.clone();
        let flight = stat_tracker::tab_retry::current_flight();
        // No more grabs for this press once the retries end: the screen
        // stream may pause while the result is analysed.
        let use_new_reader = ctx.use_new_reader;
        let (analysis, _retries) = then_end_tab_hold(stat_tracker::tab_retry::with_retries(
            &flight,
            stat_tracker::tab_retry::tab_retry_waits(capture::shot_route()),
            |outcome| tab_result(outcome, use_new_reader),
            |outcome| tab_outcome_rank(outcome, use_new_reader),
            |outcome| tab_retry_reason(outcome, use_new_reader),
            |attempt| {
                let matcher = Arc::clone(&ctx.portrait_matcher);
                // Clone player_name so the blocking closure can own it.
                let player_name_owned = player_name.map(|s| s.to_string());
                let flags = FrameFlags {
                    allow_banner_recovery,
                    session_map_known: session_map.is_some(),
                    shadow_on: ctx.shadow.is_some(),
                    use_new_reader: ctx.use_new_reader,
                };
                let own_seeds = own_seeds.clone();
                let names = names.clone();
                async move {
                    let grabbed = grab_tab_attempt(backend, capture_output, attempt).await;
                    let img = match grabbed {
                        Ok(captured) => captured.image,
                        Err(err) if capture::is_stream_miss(&err.to_string()) => {
                            return Ok(FrameAnalysisOutcome::StreamMiss);
                        }
                        Err(err) if capture::is_share_opening(&err.to_string()) => {
                            return Ok(FrameAnalysisOutcome::ShareOpening);
                        }
                        Err(err) if capture::is_declined_capture(&err.to_string()) => {
                            return Ok(FrameAnalysisOutcome::Declined);
                        }
                        Err(err) if capture::is_capture_busy(&err.to_string()) => {
                            return Ok(FrameAnalysisOutcome::CaptureBusy);
                        }
                        Err(err) if capture::is_portal_unanswered(&err.to_string()) => {
                            return Ok(FrameAnalysisOutcome::PortalPaused);
                        }
                        Err(err) => {
                            return Err(
                                anyhow::Error::from_boxed(err).context("screen capture failed")
                            );
                        }
                    };
                    let analysis = tokio::task::spawn_blocking(move || {
                        let started = Instant::now();
                        let cpu_started = stat_tracker::perf::thread_cpu();
                        let analysis = analyze_frame(
                            img,
                            matcher,
                            player_name_owned,
                            game_outcome,
                            flags,
                            own_seeds,
                            &names,
                        );
                        stat_tracker::perf::record_cpu(
                            stat_tracker::perf::Stage::TabAnalyze,
                            stat_tracker::perf::thread_cpu().saturating_sub(cpu_started),
                        );
                        stat_tracker::perf::record(
                            stat_tracker::perf::Stage::TabAnalyze,
                            started.elapsed(),
                        );
                        // With classic OCR off there was no OCR, and no OCR workers.
                        if !ocr::gate::is_off() {
                            stat_tracker::omp::log_threads_after_first_ocr();
                        }
                        analysis
                    })
                    .await?;
                    if let FrameAnalysisOutcome::Analyzed(a) = &analysis
                        && let Some(board_refusal) = reader_refusal(a, use_new_reader)
                    {
                        tracing::info!(board_refusal, "Tab frame refused by the scoreboard reader");
                    }
                    anyhow::Ok(analysis)
                }
            },
            tokio::time::sleep,
        ))
        .await?;
        let analysis = match analysis {
            FrameAnalysisOutcome::Analyzed(a) => Ok(*a),
            FrameAnalysisOutcome::StreamMiss
            | FrameAnalysisOutcome::ShareOpening
            | FrameAnalysisOutcome::Declined
            | FrameAnalysisOutcome::CaptureBusy
            | FrameAnalysisOutcome::PortalPaused => {
                if matches!(analysis, FrameAnalysisOutcome::StreamMiss) {
                    tracing::info!("Tab capture missed: no fresh stream frame");
                }
                if matches!(analysis, FrameAnalysisOutcome::CaptureBusy) {
                    tracing::info!("Tab capture missed: the previous screenshot was still running");
                }
                if matches!(analysis, FrameAnalysisOutcome::PortalPaused) {
                    tracing::info!("Tab capture missed: the screenshot portal is not answering");
                }
                return Ok(CaptureReport {
                    best_guess: None,
                    recorded: false,
                    outcome: game_outcome,
                    map: None,
                    map_source: None,
                    session_id: session_id.to_string(),
                    split: false,
                    armed_reset: false,
                    ignore_row: false,
                    reset_streak: req.reset_streak,
                    reset_baseline: req.reset_baseline,
                    baseline_row: req.baseline_row,
                    refresh_baseline: false,
                    clear_hint: false,
                    count_progress: false,
                    career_panel: false,
                    held_counters: None,
                    held_hero: None,
                    gate_state: None,
                    hero_auth: req.hero_auth.clone(),
                    seal: None,
                    close_reason: None,
                    held_at: None,
                    map_replaces_text_fallback: false,
                    rejoin_carry: None,
                    finished_game_tab: None,
                });
            }
            FrameAnalysisOutcome::NotAScoreboard {
                outcome,
                frame,
                scan,
            } => {
                tracing::warn!(
                    dip_count = scan.dip_count,
                    dip_pitch = ?scan.median_pitch,
                    spectral_pitch = ?scan.spectral_pitch,
                    "capture rejected: not a scoreboard (saved to debug/rejected when debug_ocr is on)"
                );
                Err((outcome, frame, "preflight"))
            }
            FrameAnalysisOutcome::TeamSizeUncertain {
                outcome,
                frame,
                scan,
            } => {
                tracing::warn!(
                    dip_count = scan.dip_count,
                    dip_pitch = ?scan.median_pitch,
                    spectral_pitch = ?scan.spectral_pitch,
                    "capture rejected: row pitch does not settle 5v5 vs 6v6 (saved to debug/rejected when debug_ocr is on)"
                );
                Err((outcome, frame, "teamsize"))
            }
        };
        let analysis = match analysis {
            Ok(a) => a,
            Err((outcome, frame, reason)) => {
                archive_rejected_map_label_crop(
                    ctx.debug_ocr,
                    data_dir,
                    session_id,
                    &frame,
                    reason,
                );
                save_rejected_frame(ctx.debug_ocr, data_dir, frame, reason);
                return Ok(CaptureReport {
                    best_guess: None,
                    recorded: false,
                    outcome,
                    map: None,
                    map_source: None,
                    session_id: session_id.to_string(),
                    split: false,
                    armed_reset: false,
                    ignore_row: false,
                    reset_streak: req.reset_streak,
                    reset_baseline: req.reset_baseline,
                    baseline_row: req.baseline_row,
                    refresh_baseline: false,
                    clear_hint: false,
                    count_progress: false,
                    career_panel: false,
                    held_counters: None,
                    held_hero: None,
                    gate_state: None,
                    hero_auth: req.hero_auth.clone(),
                    seal: None,
                    close_reason: None,
                    held_at: None,
                    map_replaces_text_fallback: false,
                    rejoin_carry: None,
                    finished_game_tab: None,
                });
            }
        };
        (analysis, None)
    };
    let FrameAnalysis {
        mut frame_outcome,
        ocr,
        rows,
        board: analysis_board,
        ocr_fallback,
        portrait_hero,
        career_hero,
        pack_hero,
        map_from_panel,
        map_panel_raw,
        map_from_banner,
        scoreboard: scoreboard_img,
        player_row_idx,
        name_match,
        own_row,
        name_recognizer,
        own_name_candidate,
        own_seed_verdict,
        team_size,
        frame: frame_img,
        shadow_input,
    } = analysis;
    if ctx.use_new_reader {
        // R9-2: a probe Tab's verdict is the finished game's, whose
        // session exists; the fresh id may never be used.
        let seed_session = req
            .finished_game_probe
            .as_ref()
            .map_or(session_id, |old| old.session_id.as_str());
        note_own_seed_verdict(ctx.own_name.as_ref(), own_seed_verdict, seed_session).await;
    }
    let ocr_result = ocr
        .map_err(anyhow::Error::from_boxed)
        .context("full-board OCR failed")?;
    // `reader = "new"`: the board the trust gate and the stats come from.
    // No board (a test injection without one) is a board that was not found.
    // A board found empty with OCR on was read with ocr-v1 instead
    // (`ocr_fallback`): this Tab then takes the ocr-v1 path for its stats
    // and is stored with recognizer `ocr-v1`.
    let board_reader = ctx.use_new_reader && !ocr_fallback;
    let new_board = board_reader.then(|| {
        board_override
            .or(analysis_board)
            .unwrap_or_else(|| shadow::reader::not_found(shadow::BoardStatus::NotFound))
    });
    // The member's row on that board: the row `find_own_row` chose from the
    // own-name seeds and the highlight (alpha.7 O9), or from its one name
    // fallback. No name is matched here.
    let board_row = reader_apply::own_row(None, None, player_row_idx, team_size);

    // Last-resort outcome source: the result header printed inside the
    // scoreboard region itself. Observed 2026-08-16 22:19:24Z (session
    // 018cea57): the full-board OCR of a Tab frame began "~ Defeat" while
    // every dedicated detector returned Unknown and the game stayed unknown.
    // The full-board text is already paid for, so this costs nothing; it is
    // limited to the first lines (header position) so chat or player names
    // deeper in the board can never supply a result (fleet::tracker-wl ET-2).
    if matches!(frame_outcome, detect::MatchOutcome::Unknown) {
        match parse::outcome_from_board_header(&ocr_result.raw_text) {
            detect::MatchOutcome::Unknown => {}
            o => {
                tracing::info!(outcome = ?o, "outcome read from scoreboard OCR header line");
                frame_outcome = o;
            }
        }
    }
    let mut outcome = if game_outcome.is_decided() {
        game_outcome
    } else {
        frame_outcome
    };

    tracing::info!(?outcome, "frame analysis");
    let preview_end = ocr_result
        .raw_text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= 120)
        .last()
        .unwrap_or(0);
    // With classic OCR off the board text is not asked for: nothing to log.
    if !ocr::gate::is_off() {
        tracing::info!(
            confidence = ocr_result.confidence,
            text_preview = &ocr_result.raw_text[..preview_end],
            "OCR result"
        );
    }

    let player_row_conf = player_row_idx
        .and_then(|i| rows.get(i))
        .map(|r| r.mean_confidence);
    tracing::info!(
        ?player_row_idx,
        player_row_conf,
        rows = rows.len(),
        text_confidence = ocr_result.confidence,
        "scoreboard cell OCR complete"
    );

    // Trust gate (OCR site T3): don't parse frames that don't look like a
    // scoreboard (menus, replay browser, desktop). Better to record nothing
    // than to scrape stats out of a random screen. ocr-v1 decides on its
    // cell text. The new reader decides on its own board: found, the same
    // team size, digit values on it. A board it refuses is not handed to OCR.
    let board_refusal = new_board
        .as_ref()
        .and_then(|board| reader_apply::board_refusal(board, team_size));
    let looks_like_scoreboard = match &new_board {
        Some(_) => board_refusal.is_none(),
        None => parse::looks_like_scoreboard(&rows),
    };
    if !looks_like_scoreboard {
        tracing::warn!(
            rows = rows.len(),
            board_refusal,
            "capture rejected: frame does not look like a scoreboard (saved to debug/rejected when debug_ocr is on)"
        );
        archive_rejected_map_label_crop(
            ctx.debug_ocr,
            data_dir,
            session_id,
            &frame_img,
            "noscoreboard",
        );
        save_rejected_frame(ctx.debug_ocr, data_dir, frame_img, "noscoreboard");
        return Ok(CaptureReport {
            best_guess: None,
            recorded: false,
            outcome,
            map: None,
            map_source: None,
            session_id: session_id.to_string(),
            split: false,
            armed_reset: false,
            ignore_row: false,
            reset_streak: req.reset_streak,
            reset_baseline: req.reset_baseline,
            baseline_row: req.baseline_row,
            refresh_baseline: false,
            clear_hint: false,
            count_progress: false,
            career_panel: false,
            held_counters: None,
            held_hero: None,
            gate_state: None,
            hero_auth: req.hero_auth.clone(),
            seal: None,
            close_reason: None,
            held_at: None,
            map_replaces_text_fallback: false,
            rejoin_carry: None,
            finished_game_tab: None,
        });
    }

    let mut outcome_label = outcome.to_string();
    // CG-4 C: mutates across this capture; carried back on CaptureReport.
    let mut hero_auth = req.hero_auth.clone();

    // The member's stats. ocr-v1: the cell text, else the name-anchored
    // raw-text fallback. New reader: the digit reads of the own row and
    // nothing else; a cell it does not vouch for is held, and a row with
    // under four sure cells is refused (never a text fallback, never an OCR
    // number). A suspect own row (alpha.7 O9: it may be another player's)
    // stores none of its numbers and does not seed or raise the running
    // totals. The boundary plan still lets it anchor the reset baseline,
    // and it does not split a game by itself (`BoardFacts::suspect_row`).
    let own_row_suspect = own_row.is_some_and(|own| own.suspect());
    let scoreboard_read = match &new_board {
        Some(board) => reader_apply::read_own_row_stats(board, board_row, &outcome_label)
            .map(|read| (read.matched, true, read.held)),
        None => parse::read_scoreboard(
            &rows,
            player_row_idx,
            &ocr_result.raw_text,
            &outcome_label,
            text_fallback_name(board_reader, player_row_idx, player_name),
        )
        .map(|read| {
            (
                read.matched,
                read.trusted_cells,
                [false; capture_gate::GATE_COLS],
            )
        }),
    };
    match scoreboard_read {
        Ok((mut parsed, trusted_cells, held_cols)) => {
            // The own-row name reader for this Tab. Absent when the row
            // came from a highlight, a seed, or the career hero.
            parsed.name_recognizer = name_recognizer.map(|id| id.as_str().to_string());
            // R9-2: a Tab past the grace window of a finished game. This
            // board decides whether it continues that game (then the capture
            // is that game's, with its false result cleared when the board
            // moved on) or is the fresh session's, as before rc.9.
            let mut finished_game_tab = None;
            if let Some(old) = req.finished_game_probe.take() {
                let probe_suspect = if board_reader {
                    held_cols
                } else {
                    parse::player_row_suspect_mask(&rows, player_row_idx)
                };
                let verdict = match old.prev_gate {
                    Some((prev, age)) => boundary::finished_game_tab(
                        &prev,
                        old.session_map.as_deref(),
                        map_from_panel.as_deref(),
                        &Counters {
                            elims: parsed.elims,
                            assists: parsed.assists,
                            deaths: parsed.deaths,
                            damage: parsed.damage,
                            healing: parsed.healing,
                            mitigation: parsed.mitigation,
                        },
                        &probe_suspect,
                        &held_cols,
                        trusted_cells && player_row_idx.is_some() && !own_row_suspect,
                        age,
                        old.last_board_after_result,
                    ),
                    None => boundary::FinishedGameTab::NewGame,
                };
                tracing::info!(
                    ?verdict,
                    finished_session = %old.session_id,
                    map = ?map_from_panel,
                    elims = parsed.elims,
                    deaths = parsed.deaths,
                    "Tab past the grace window: board compared with the finished game"
                );
                if verdict == boundary::FinishedGameTab::SameBoard {
                    // A late look at the finished game's board: the game
                    // and its result stay, and the board is not stored, so
                    // it neither replaces the game's last row nor restarts
                    // the window ([`FINISHED_GAME_TAB_MAX_GAP`]).
                    return Ok(CaptureReport {
                        finished_game_tab: Some(verdict),
                        ..blank_rejected_report(
                            old.game_outcome,
                            &old.session_id,
                            old.hero_auth.clone(),
                            &old,
                        )
                    });
                }
                if verdict == boundary::FinishedGameTab::StillPlaying {
                    req = *old;
                    // The result is cleared when this report lands
                    // (`settle_tab_probe`); this row does not carry it, nor
                    // the result words read for it (`unlatch_false_outcome`).
                    req.game_outcome = detect::MatchOutcome::Unknown;
                    req.hint = None;
                    req.after_end_screen = false;
                    req.result_evidence = Default::default();
                    // A live board and a result word on the same frame
                    // disagree, and this frame was read with the fresh
                    // session's age gate: its outcome is not taken.
                    frame_outcome = detect::MatchOutcome::Unknown;
                    if ctx.use_new_reader {
                        req.member_map_edit = session_has_member_map_edit(&ctx.store, &req).await;
                    }
                    hero_auth = req.hero_auth.clone();
                    outcome = if req.game_outcome.is_decided() {
                        req.game_outcome
                    } else {
                        frame_outcome
                    };
                    outcome_label = outcome.to_string();
                    parsed.outcome = outcome_label.clone();
                }
                finished_game_tab = Some(verdict);
            }
            let session_id: &str = &req.session_id;
            if held_cols.iter().any(|&held| held) {
                // Until the gate runs, a held column reads as the game's
                // last accepted value, so the split and reset checks see no
                // move in it.
                let prior = gate_prev_for_store(req.prev_gate, req.awaiting_first_board)
                    .map(|(state, _)| state.accepted);
                fill_held_cells(&mut parsed, held_cols, prior);
                tracing::info!(
                    held = ?held_cols,
                    "new reader held stat cells it does not vouch for"
                );
            }
            if own_row_suspect {
                tracing::info!(
                    ?player_row_idx,
                    "own row is suspect: none of its stats are stored or used to raise the totals"
                );
            }
            if !trusted_cells {
                tracing::warn!(
                    ?player_row_idx,
                    elims = parsed.elims,
                    assists = parsed.assists,
                    deaths = parsed.deaths,
                    damage = parsed.damage,
                    healing = parsed.healing,
                    mitigation = parsed.mitigation,
                    "stat cells unreadable: using raw-text fallback (low trust)"
                );
            }
            // Hero authority (CG-4 C): career-panel always wins; portrait may
            // confirm current hero but may only switch if career never succeeded
            // this game and ≥2 consecutive matches ≥0.85. See hero_auth::resolve_hero.
            // reader = "new" (O8): the heroes-v2 icon reads (career panel
            // portrait, own row) take the career-panel title's place, and no
            // OCR text names the hero.
            let portrait = portrait_hero
                .as_ref()
                .map(|(name, conf)| (name.as_str(), *conf));
            let (hero, source, next_auth) = if ctx.use_new_reader {
                hero_auth::resolve_hero_new_reader(
                    pack_hero.as_deref(),
                    portrait,
                    &hero_auth,
                    parse::canonical_hero,
                )
            } else {
                hero_auth::resolve_hero(
                    career_hero.as_deref(),
                    portrait,
                    &parsed.hero,
                    &hero_auth,
                    parse::canonical_hero,
                )
            };
            hero_auth = next_auth;
            parsed.hero = hero;
            parsed.role = parse::guess_role_public(&parsed.hero);
            let source_label = source.label();
            tracing::info!(
                hero = %parsed.hero,
                source = source_label,
                career_ever_ok = hero_auth.career_ever_ok,
                portrait_conf = portrait_hero.as_ref().map(|(_, c)| *c),
                "hero resolved (CG-4 C authority)"
            );

            // Auto-collect a portrait reference when the hero is identified and
            // collection is enabled, or, always, when the career panel (the
            // authoritative OCR read) names a hero whose reference is missing.
            // The historical Doctrine stand-in digest (PROVISIONAL_PORTRAITS)
            // still counts as missing, so a capture replaces that old file.
            // A file with any other bytes is a real crop or a user file and
            // is never overwritten. The player's row must be known: cropping
            // row 0 when the row was not identified would store someone
            // else's hero and then stick. The portrait matcher reads the
            // installed hero pack. A local crop is used only for a stem that
            // pack does not have, and only when a pack is installed. With no
            // pack the matcher names no hero. Only the career-panel source
            // may seed (a portrait/held/text guess must not template itself).
            // The heroes-v2 icon reads (`reader = "new"`) do not force a
            // seed either: a wrong seed sticks. Only the
            // `collect_portraits` opt-in seeds under the new reader.
            let portraits_path = detect::hero_portrait::portraits_dir(data_dir);
            let reference_path =
                detect::hero_portrait::portrait_reference_path(&portraits_path, &parsed.hero);
            let slot_replaceable = parsed.hero != "Unknown"
                && detect::hero_portrait::portrait_slot_is_replaceable(&reference_path);
            let career_panel = matches!(source, HeroSource::CareerPanel);
            let save_portrait = detect::hero_portrait::should_save_portrait_crop(
                career_panel,
                collect_portraits,
                slot_replaceable,
                player_row_idx.is_some(),
                detect::hero_portrait::portrait_seed_layout_known(team_size, name_match.as_ref()),
            );
            if save_portrait && career_panel {
                tracing::info!(hero = %parsed.hero, "no real portrait reference for career-panel hero: seeding one from this capture");
            }
            if save_portrait && let Some(row) = player_row_idx {
                // Shared geometry (5v5/6v6 + team gap): an inlined 5v5-only copy
                // here used to mis-crop 6v6/team-2 references into the template
                // library.
                let dims = (scoreboard_img.width(), scoreboard_img.height());
                if let Some(r) = detect::hero_portrait::portrait_rect(dims, row, team_size) {
                    let crop = scoreboard_img.crop_imm(r.x, r.y, r.w, r.h);
                    if let Err(e) = detect::hero_portrait::save_captured_portrait(
                        &portraits_path,
                        &parsed.hero,
                        &crop,
                    ) {
                        tracing::warn!(error = %e, hero = %parsed.hero, "portrait save failed");
                    }
                }
            }

            let captured_at = Utc::now();
            let now = SurrealDatetime::from(captured_at);

            // Edge-ink suspect mask (CG-3) for the player's row, read from the same
            // per-cell OCR the stats came from. Threaded into BOTH the split decision
            // (DUP-1: a suspect column must not vote as a regression) and the capture
            // gate (a suspect read never corroborates a jump or drives an un-latch).
            // Under the new reader the mask is its held cells: no OCR cell
            // was read, so there is no edge ink to look at.
            let suspect = if board_reader {
                held_cols
            } else {
                parse::player_row_suspect_mask(&rows, player_row_idx)
            };

            // Stat-regression boundary (detector-independent): scoreboard stats
            // are cumulative within a match, so if this capture's counters sit
            // below the session's previous accepted capture, the poller missed
            // the game boundary and this board belongs to a new game.
            // Same-map/hero unfinished session: a stat-looking regression after
            // the 120s gap is almost always OCR noise or a late scoreboard of
            // this match. Do not split it off. A fresh-match drop is a candidate
            // new game even inside that gap. The first is held.
            let raw_counters = Counters {
                elims: parsed.elims,
                assists: parsed.assists,
                deaths: parsed.deaths,
                damage: parsed.damage,
                healing: parsed.healing,
                mitigation: parsed.mitigation,
            };
            let mut facts = BoardFacts {
                counters: raw_counters,
                suspect,
                // `trusted_cells` is `stats_from_row` on the identified row,
                // which is the same predicate as `parse::row_counts`.
                trusted_cells,
                held: held_cols,
                suspect_row: own_row_suspect,
                row_id: identified_row_id(player_row_idx),
                hero: &parsed.hero,
                map_from_panel: map_from_panel.as_deref(),
                panel_from_banner: map_from_banner,
                parsed_map: &parsed.map_name,
                frame_outcome,
            };
            let rejoin = rejoin_board(&req, &facts);
            note_rejoin(&req, &rejoin, raw_counters);
            facts.counters = rejoin.counters;
            let plan = plan_from_board(&req, &facts);
            if plan.skip_store {
                if plan.defer {
                    tracing::info!(
                        session_id = %session_id,
                        elims = parsed.elims,
                        deaths = parsed.deaths,
                        damage = parsed.damage,
                        "fresh-match board held: not written onto the current game"
                    );
                }
                // A deferred board with a held cell is not carried
                // (`plan.deferred_counters` is none): no hero or time either.
                let carried = plan.defer && plan.deferred_counters.is_some();
                return Ok(CaptureReport {
                    finished_game_tab,
                    ..skipped_capture_report(
                        &plan,
                        session_id,
                        source.is_authority(),
                        hero_auth,
                        carried.then(|| parsed.hero.clone()),
                        carried.then_some(captured_at),
                    )
                });
            }
            let mut staged = stage_capture(&req, &plan, &facts, captured_at);
            // Shadow recognizer: hand the accepted board to the log-only
            // worker. try_send, never blocks; a busy worker drops it.
            if let (Some(worker), Some(input)) = (&ctx.shadow, &shadow_input) {
                worker.try_submit(shadow::worker::ShadowJob::new(
                    input,
                    &rows,
                    player_row_idx,
                    &staged.target_session,
                    captured_at,
                ));
            }
            let split = staged.split;
            outcome = staged.outcome;
            outcome_label = staged.outcome_label.clone();
            parsed.outcome = outcome_label.clone();
            let target_session = staged.target_session.clone();
            // This board's session, including a split's new game. The first
            // Tab that still has the map label wins; later Tabs no-op.
            archive_map_label_crop(data_dir, &target_session, &frame_img);
            if split {
                tracing::info!(
                    old_session = %session_id,
                    new_session = %target_session,
                    elims = parsed.elims,
                    deaths = parsed.deaths,
                    damage = parsed.damage,
                    after_end_screen = req.after_end_screen,
                    reset_streak = plan.reset_streak,
                    "player stats regressed: previous game never closed; splitting into a new session"
                );
            }
            let gate = &staged.gate;
            // CG-4 B3: always surface the per-column suspect mask on the accept
            // path so a latched inflation can be diagnosed as flagged vs clean
            // (the 07-22 HLG 22994 case was undiagnosable without this).
            if suspect.iter().any(|&s| s) {
                tracing::info!(
                    session_id = %target_session,
                    suspect = ?suspect,
                    raw_elims = raw_counters.elims,
                    raw_assists = raw_counters.assists,
                    raw_deaths = raw_counters.deaths,
                    raw_damage = raw_counters.damage,
                    raw_healing = raw_counters.healing,
                    raw_mitigation = raw_counters.mitigation,
                    accepted_healing = gate.accepted.healing,
                    accepted_damage = gate.accepted.damage,
                    accepted_mitigation = gate.accepted.mitigation,
                    "capture accepted with edge-ink suspect mask"
                );
            }
            for h in &gate.holds {
                if h.kind == capture_gate::HoldKind::Unread {
                    // Not a misread caught by the gate: the reader gave no
                    // number for the cell (or a first read was not taken).
                    tracing::info!(
                        session_id = %target_session,
                        col = h.col,
                        raw = h.raw,
                        held = h.held,
                        "no sure read of a stat cell: the last accepted value stays"
                    );
                    continue;
                }
                if board_reader {
                    // No OCR read the cell: the digit read does not fit
                    // the game so far.
                    tracing::warn!(
                        session_id = %target_session,
                        col = h.col,
                        kind = ?h.kind,
                        raw = h.raw,
                        held = h.held,
                        "capture gate held a cell (the read does not fit the game so far)"
                    );
                    continue;
                }
                tracing::warn!(
                    session_id = %target_session,
                    col = h.col,
                    kind = ?h.kind,
                    raw = h.raw,
                    held = h.held,
                    "capture gate held a cell (suspected OCR misread)"
                );
            }
            for u in &gate.unlatches {
                if u.replaced_unconfirmed {
                    tracing::warn!(
                        session_id = %target_session,
                        col = u.col,
                        raw = u.raw,
                        revised_from = u.revised_from,
                        "capture gate replaced an unconfirmed cell"
                    );
                } else {
                    tracing::warn!(
                        session_id = %target_session,
                        col = u.col,
                        raw = u.raw,
                        revised_from = u.revised_from,
                        "capture gate un-latched a cell (clean reads revised a latched value down)"
                    );
                }
            }
            if gate.state.low_trust {
                tracing::warn!(
                    session_id = %target_session,
                    elims = gate.accepted.elims,
                    assists = gate.accepted.assists,
                    deaths = gate.accepted.deaths,
                    damage = gate.accepted.damage,
                    healing = gate.accepted.healing,
                    mitigation = gate.accepted.mitigation,
                    "stored capture is low-trust"
                );
            }
            let held_stats = gate.accepted;
            let held_cells = gate.holds.clone();
            // The stats as read, before the gate's held values replace them:
            // the report kit's `ocr_v1` (spec section 2). ocr-v1: the
            // Tesseract read. `reader = "new"`: there is no Tesseract read,
            // so the field carries the digit read before the gate, and for a
            // cell with no sure read the stored value (set below).
            let tesseract_stats = (
                parsed.elims,
                parsed.assists,
                parsed.deaths,
                parsed.damage,
                parsed.healing,
                parsed.mitigation,
            );
            parsed.elims = held_stats.elims;
            parsed.assists = held_stats.assists;
            parsed.deaths = held_stats.deaths;
            parsed.damage = held_stats.damage;
            parsed.healing = held_stats.healing;
            parsed.mitigation = held_stats.mitigation;
            parsed.map_name = staged.map_name.clone();

            // Untrusted Deathmatch falls back to the open map. A split stays
            // Unknown. Adlersbrunn with stats on only one team is not stored
            // as Eichenwalde. The current row is inserted before the carried
            // board, so a failed insert leaves the hold for the next Tab.
            if prepare_capture_row(
                &mut parsed,
                &mut staged,
                req.session_map.as_deref(),
                map_panel_raw.as_deref(),
                &ocr_result.raw_text,
                match &new_board {
                    Some(board) => reader_apply::both_teams_have_stats(board, team_size),
                    None => parse::both_teams_have_stats(&rows, team_size),
                },
            ) == PreparedRow::Drop
            {
                return Ok(CaptureReport {
                    finished_game_tab,
                    ..blank_rejected_report(outcome, session_id, hero_auth, &req)
                });
            }

            // The session is owned by the active game (map-vote to accolade). The
            // first capture creates the session row; later captures (including hero
            // swaps and the post-match scoreboard) append to the same session.
            parsed.session_id = target_session.clone();
            // The read before the gate, kept for the report kit's `ocr_v1`
            // field: this capture's fields with the stats as they were read
            // (Tesseract under ocr-v1, the digit reader under the new one).
            let mut ocr_parsed = parsed.clone();
            (
                ocr_parsed.elims,
                ocr_parsed.assists,
                ocr_parsed.deaths,
                ocr_parsed.damage,
                ocr_parsed.healing,
                ocr_parsed.mitigation,
            ) = tesseract_stats;
            // The new reader's own read of each stat before the gate held
            // any: the best guess for a column that never gets a sure read.
            let mut raw_guess: Option<[u32; capture_gate::GATE_COLS]> = None;
            // The reader's confidences, for the report kit (empty under ocr-v1).
            let mut kit_confidences = std::collections::BTreeMap::new();
            if let Some(board) = new_board {
                let guesses = reader_apply::own_row_guesses(&board, board_row);
                offer_own_name_seed(
                    ctx.own_name.as_ref(),
                    own_name_candidate,
                    &target_session,
                    &parsed.map_name,
                )
                .await;
                kit_confidences = reader_apply::kit_confidences(&board, board_row);
                kit_confidences.insert(
                    "result".to_string(),
                    stat_tracker::report::kit::ReaderRead::from_field(&shadow::result_field(
                        &req.result_evidence,
                    )),
                );
                apply_new_reader(
                    &mut parsed,
                    &req.result_evidence,
                    board_row,
                    own_row_suspect,
                    board,
                    pack_hero.as_deref(),
                );
                // The reader's number for each cell, sure or not; a cell with
                // none keeps what the row holds.
                let held = [
                    parsed.elims,
                    parsed.assists,
                    parsed.deaths,
                    parsed.damage,
                    parsed.healing,
                    parsed.mitigation,
                ];
                raw_guess = Some(std::array::from_fn(|col| guesses[col].unwrap_or(held[col])));
                let prior = gate_prev_for_store(req.prev_gate, req.awaiting_first_board);
                let gate_suspect = if own_row_suspect {
                    [true; capture_gate::GATE_COLS]
                } else {
                    suspect
                };
                staged.gate = hold_new_reader_stats_with_guesses(
                    &mut parsed,
                    prior,
                    gate_suspect,
                    held_cols,
                    split,
                    trusted_cells && !own_row_suspect,
                    carried_board(staged.carried, staged.carried_at, captured_at),
                    guesses,
                );
                // A held cell has no read of its own: the kit's `ocr_v1`
                // repeats the stored value there, never a number carried in
                // from before a split.
                let stored = [
                    parsed.elims,
                    parsed.assists,
                    parsed.deaths,
                    parsed.damage,
                    parsed.healing,
                    parsed.mitigation,
                ];
                let read = [
                    &mut ocr_parsed.elims,
                    &mut ocr_parsed.assists,
                    &mut ocr_parsed.deaths,
                    &mut ocr_parsed.damage,
                    &mut ocr_parsed.healing,
                    &mut ocr_parsed.mitigation,
                ];
                for (col, cell) in read.into_iter().enumerate() {
                    if held_cols[col] {
                        *cell = stored[col];
                    }
                }
                // A member's own hero pick on this game is never overridden
                // by a later Tab's read (O8): this row carries the pick too.
                if !split && target_session == session_id {
                    carry_session_hero_edit(store, session_id, &mut parsed).await;
                }
            } else {
                note_gate_suspects(
                    &mut parsed,
                    held_stats,
                    &capture_gate::unsure_fields(&held_cells),
                );
            }
            let report = CaptureReport {
                best_guess: None,
                recorded: true,
                outcome,
                map: staged.recorded_map.clone(),
                map_source: staged.map_source,
                session_id: target_session.clone(),
                split,
                armed_reset: false,
                ignore_row: plan.ignore_row,
                reset_streak: plan.reset_streak,
                reset_baseline: plan.reset_baseline,
                baseline_row: plan.baseline_row,
                refresh_baseline: plan.refresh_baseline,
                clear_hint: plan.clear_hint,
                count_progress: plan.count_progress,
                career_panel: source.is_authority(),
                held_counters: None,
                held_hero: None,
                gate_state: Some(staged.gate.state),
                hero_auth,
                seal: plan.seal,
                close_reason: plan.close_reason,
                held_at: None,
                map_replaces_text_fallback: staged.replaced_text_fallback,
                rejoin_carry: rejoin.carry,
                finished_game_tab,
            };
            // A stat column this game has never read is a placeholder, not
            // a number: no row is written while one is left (F266-1). The
            // board still went through the gate and the boundary plan, and
            // the report carries both, so the game goes on from here and
            // its row appears with the first board that reads the column.
            let unread = staged.gate.state.unread_cols();
            if !unread.is_empty() {
                tracing::info!(
                    session_id = %target_session,
                    ?unread,
                    split,
                    "board read but not stored: a stat column of this game has no sure read yet"
                );
                // The best guess waits on the active game. A later Tab with
                // a sure read stores the real row; if none comes, the game
                // stores this guess when it closes (alpha.8).
                // An all-suspect read, including a suspect own row, must not
                // be saved later as the game's best guess either.
                let best_guess = if own_row_suspect || suspect.iter().all(|&cell| cell) {
                    None
                } else {
                    raw_guess.map(|guess| {
                        let mut row = parsed.clone();
                        apply_best_guess(&mut row, &unread, guess);
                        row
                    })
                };
                return Ok(CaptureReport {
                    recorded: false,
                    best_guess,
                    ..report
                });
            }
            let created_this_capture =
                commit_capture_rows(store, data_dir, &staged, &parsed, now).await?;

            tracing::info!(
                hero = %parsed.hero,
                map = %parsed.map_name,
                elims = parsed.elims,
                deaths = parsed.deaths,
                "parsed scoreboard"
            );
            let recorded_map = staged.recorded_map.clone();

            // Dump the accepted scoreboard crop to a bounded ring so a corrupt
            // ACCEPTED board is diagnosable after the fact: tonight's corruption
            // was undiagnosable because only rejected frames were ever saved.
            // Only with debug_ocr on: the crop shows other players' names.
            // The own-row file is a separate crop of this player's row, widened
            // left of the name column so the hero portrait is included. A
            // header clipped at the scoreboard's left edge is cut from the
            // widened frame crop the hero reader uses.
            save_own_row_review(
                ctx.debug_ocr,
                data_dir,
                &frame_img,
                scoreboard_img,
                player_row_idx,
                team_size,
            );
            // The unpadded crop, not the 5v5 recognition image: offline
            // replay places rows from crop height, and these files are the
            // benchmark boards.
            save_accepted_frame(ctx.debug_ocr, data_dir, debug_accepted_board(&frame_img));
            record_report_kit(
                data_dir,
                &target_session,
                &frame_img,
                team_size,
                &ocr_parsed,
                &parsed,
                captured_at,
                req.tab_at,
                board_names(&rows),
                &kit_confidences,
            );

            // Re-derive the session's hero timeline from its RAW snapshots after
            // every capture (HS-1). A single capture can mislabel (career panel
            // shows the spectated hero while dead; portrait matching can misfire),
            // and a late hero swap must be recorded as its own segment rather than
            // stamped across the whole game (the old set_session_hero last-write
            // recorded a 97%-Ana game as Rein). set_session_hero is now a MANUAL
            // repair helper only. Per-snapshot reads stay raw; the derived primary
            // drives the displayed/uploaded hero.
            if !created_this_capture
                && let Err(e) = store.refresh_session_hero_timeline(&target_session).await
            {
                tracing::debug!(error = %e, "failed to refresh session hero timeline");
            }
            // Diagnostics: after N consecutive scoreboard captures that parsed but
            // resolved no map, dump the map-label region so the next failure is
            // debuggable from raw pixels (the 07-17 Ilios game left none).
            use std::sync::atomic::Ordering;
            if recorded_map.is_none() {
                let n = ctx.empty_map_reads.fetch_add(1, Ordering::Relaxed) + 1;
                if n >= EMPTY_MAP_DUMP_THRESHOLD {
                    ctx.empty_map_reads.store(0, Ordering::Relaxed);
                    let region = ocr::preprocess::crop_map_name(&frame_img);
                    let dir = data_dir.join("debug").join("mapmiss");
                    tracing::warn!(
                        consecutive = n,
                        "N consecutive scoreboard captures resolved no map: dumping map region to debug/mapmiss"
                    );
                    tokio::task::spawn_blocking(move || {
                        save_frame_ring(&dir, "mapmiss", &region, MAPMISS_KEEP);
                    });
                }
            } else {
                ctx.empty_map_reads.store(0, Ordering::Relaxed);
            }

            Ok(report)
        }
        Err(miss) => {
            // Recording another row's stats would be worse than recording nothing.
            // A found row whose cells did not parse is not the same failure as a
            // frame where the row itself is missing. CellsUnreadable also fires
            // when the name shows up only in chat or the kill feed.
            let reason = match miss {
                parse::ScoreboardMiss::PlayerRowNotFound if ctx.use_new_reader => {
                    // The new reader finds the row from the own-name seeds,
                    // the highlight and the career hero; `player_name` only
                    // feeds the name fallback, so "set player_name" is not
                    // the fix here. With classic OCR off there was no name
                    // read to fall back on, so the line does not claim one.
                    let name_read = if ocr::gate::is_off() {
                        "classic OCR is off, so the name was not read"
                    } else {
                        "and the name read found none"
                    };
                    tracing::warn!(
                        source = ?own_row.map(|own| own.source.label()),
                        "capture held: own row not found. No own-name seed matched a row, no row is \
                         highlighted and the career panel's hero is on no single row of your team \
                         ({name_read}), or a seed and a clear highlight name \
                         different rows. Nothing is stored for this Tab (saved to debug/rejected \
                         when debug_ocr is on)"
                    );
                    "noplayerrow"
                }
                parse::ScoreboardMiss::PlayerRowNotFound => {
                    tracing::warn!(
                        "capture rejected: player row not found (saved to debug/rejected when debug_ocr is on; \
                     set player_name in config.toml if it is missing)"
                    );
                    "noplayerrow"
                }
                parse::ScoreboardMiss::CellsUnreadable => {
                    // Empty under the new reader: it reads no OCR cell.
                    let cells = player_row_idx.and_then(|i| rows.get(i)).map(|row| {
                        row.stats
                            .iter()
                            .take(6)
                            .map(|cell| cell.value.clone())
                            .collect::<Vec<_>>()
                    });
                    tracing::warn!(
                        ?player_row_idx,
                        ?cells,
                        "capture rejected: stat cells unreadable (saved to debug/rejected when debug_ocr is on)"
                    );
                    "unreadable"
                }
            };
            archive_rejected_map_label_crop(
                ctx.debug_ocr,
                data_dir,
                session_id,
                &frame_img,
                reason,
            );
            save_rejected_frame(ctx.debug_ocr, data_dir, frame_img, reason);
            Ok(CaptureReport {
                best_guess: None,
                recorded: false,
                outcome,
                map: None,
                map_source: None,
                session_id: session_id.to_string(),
                split: false,
                armed_reset: false,
                ignore_row: true,
                reset_streak: req.reset_streak,
                reset_baseline: req.reset_baseline,
                baseline_row: req.baseline_row,
                refresh_baseline: false,
                clear_hint: false,
                count_progress: false,
                career_panel: false,
                held_counters: None,
                held_hero: None,
                gate_state: None,
                hero_auth,
                seal: None,
                close_reason: None,
                held_at: None,
                map_replaces_text_fallback: false,
                rejoin_carry: None,
                finished_game_tab: None,
            })
        }
    }
}

fn rand_id() -> u64 {
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    seed ^ (std::process::id() as u64).wrapping_mul(0x517cc1b727220a95)
}

/// Minimum gap between full snapshot rewrites (P11). Capture/poll can mark the
/// export dirty faster than this; the next due flush (or a forced one after
/// sync / shutdown) actually rewrites the file.
const SNAPSHOT_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(2);

/// (dirty, last successful export). Module-level so capture + sync tasks share it.
static SNAPSHOT_STATE: std::sync::Mutex<(bool, Option<std::time::Instant>)> =
    std::sync::Mutex::new((false, None));

/// Mark the live snapshot dirty and export if the debounce window has elapsed.
async fn refresh_snapshot(store: &storage::LocalStore, data_dir: &std::path::Path) {
    refresh_snapshot_inner(store, data_dir, false).await;
}

/// Export immediately (sync complete, shutdown). Resets dirty.
async fn refresh_snapshot_force(store: &storage::LocalStore, data_dir: &std::path::Path) {
    refresh_snapshot_inner(store, data_dir, true).await;
}

/// Flush a debounce-deferred snapshot once its window has passed. Called from
/// the cmd-timer tick so a dirty snapshot never waits on the next mutation.
async fn flush_snapshot_if_due(store: &storage::LocalStore, data_dir: &std::path::Path) {
    let due = {
        let state = SNAPSHOT_STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.0
            && state
                .1
                .is_none_or(|last| last.elapsed() >= SNAPSHOT_DEBOUNCE)
    };
    if due {
        refresh_snapshot_inner(store, data_dir, true).await;
    }
}

/// Shutdown path: export a trailing dirty snapshot regardless of the window,
/// the GUI must see the final state (try_sync only force-exports on success).
async fn flush_snapshot_if_dirty(store: &storage::LocalStore, data_dir: &std::path::Path) {
    let dirty = SNAPSHOT_STATE.lock().unwrap_or_else(|e| e.into_inner()).0;
    if dirty {
        refresh_snapshot_inner(store, data_dir, true).await;
    }
}

async fn refresh_snapshot_inner(
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    force: bool,
) {
    {
        let mut state = SNAPSHOT_STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.0 = true;
        if !force
            && let Some(last) = state.1
            && last.elapsed() < SNAPSHOT_DEBOUNCE
        {
            return;
        }
    }
    match store.export_snapshot(data_dir).await {
        Ok(()) => {
            if let Ok(mut state) = SNAPSHOT_STATE.lock() {
                state.0 = false;
                state.1 = Some(std::time::Instant::now());
            }
        }
        Err(e) => tracing::debug!(error = %e, "live snapshot refresh failed"),
    }
}

/// One command tick of the GUI queue. A SetOutcome or a delete schedules
/// an upload on this same tick, so a deleted game leaves the site right
/// away instead of at the next Tab sync or daemon restart (O3). An edit
/// does not; the idle sync tick picks it up.
async fn apply_queued_commands(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    sync_task: &mut Option<tokio::task::JoinHandle<()>>,
    sync_backoff: &Arc<std::sync::Mutex<sync::SyncBackoff>>,
    sync_client: Option<&sync::SyncClient>,
) {
    let cmds = storage::read_commands(data_dir);
    if cmds.is_empty() {
        flush_snapshot_if_due(store, data_dir).await;
        return;
    }
    for (cmd_file, cmd) in &cmds {
        tracing::info!(?cmd, "applying GUI command");
        match cmd {
            storage::StoreCommand::SetOutcome {
                session_id,
                outcome,
            } => {
                if let Some(g) = st.active_game.as_mut()
                    && g.session_id == *session_id
                {
                    g.record_member_outcome(
                        outcome.parse().unwrap_or(detect::MatchOutcome::Unknown),
                    );
                    persist_active_game(data_dir, Some(g));
                }
            }
            storage::StoreCommand::DeleteSession { session_id } => {
                // A deleted game's report kit goes too (QC1-6).
                stat_tracker::report::kit::remove(data_dir, session_id);
                if st
                    .active_game
                    .as_ref()
                    .is_some_and(|g| g.session_id == *session_id)
                {
                    st.active_game = None;
                    persist_active_game(data_dir, None);
                }
            }
            storage::StoreCommand::EditMatch { session_id, edit } => {
                if let Some(g) = st.active_game.as_mut()
                    && g.session_id == *session_id
                    && let Some(outcome) = &edit.outcome
                {
                    g.record_member_outcome(
                        outcome.parse().unwrap_or(detect::MatchOutcome::Unknown),
                    );
                    persist_active_game(data_dir, Some(g));
                }
            }
            storage::StoreCommand::ResolveSegment { .. }
            | storage::StoreCommand::RetryUpload { .. }
            | storage::StoreCommand::RetryRejectedUploads => {}
        }
        match store.apply_command(cmd).await {
            Ok(()) => {
                storage::remove_command_file(cmd_file);
                if matches!(
                    cmd,
                    storage::StoreCommand::SetOutcome { .. }
                        | storage::StoreCommand::DeleteSession { .. }
                        | storage::StoreCommand::RetryUpload { .. }
                        | storage::StoreCommand::RetryRejectedUploads
                ) {
                    schedule_store_sync(
                        sync_task,
                        sync_backoff,
                        store,
                        sync_client,
                        data_dir,
                        None,
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "GUI command failed; will retry");
            }
        }
    }
    refresh_snapshot(store, data_dir).await;
}

/// How a poll's grab or read ended when raced against shutdown and the
/// keyboard.
enum PollRace<T> {
    Done(T),
    Shutdown,
    /// A Tab press arrived first. The work was dropped and the press is in
    /// the caller's `pending` slot.
    Preempted,
}

/// Race `work` against shutdown and the next Tab press. Shutdown is polled
/// first, so a signal that is already waiting wins even when `work` is also
/// ready. Finished work beats a ready key (its result is applied and the
/// key waits in the channel). Otherwise the key wins, it is stored in
/// `pending` for the keyboard arm, and `work` is dropped, so a Tab never
/// waits behind a poll and is never lost.
///
/// This is for the poll's capture grab and OCR only. Dropping `work`
/// detaches a `spawn_blocking` task. That OCR keeps running and its result
/// is discarded. It does not write a match row. A future that writes game
/// state goes through [`finish_within_shutdown`] instead: dropping it would
/// cancel a store call that had already taken the game out of memory.
async fn stop_for_shutdown_or_key<T, K, S, KF>(
    work: impl std::future::Future<Output = T>,
    shutdown: S,
    key: KF,
    pending: &mut Option<K>,
) -> PollRace<T>
where
    S: std::future::Future<Output = ()>,
    KF: std::future::Future<Output = K>,
{
    tokio::pin!(work);
    tokio::pin!(shutdown);
    tokio::pin!(key);
    tokio::select! {
        biased;
        _ = &mut shutdown => PollRace::Shutdown,
        value = &mut work => PollRace::Done(value),
        key = &mut key => {
            *pending = Some(key);
            PollRace::Preempted
        }
    }
}

/// `key`, and a stream hold taken the moment a press comes in, before the
/// race that awaits this drops the poll's grab (and the grab's hold). A
/// keyboard error takes none.
async fn with_bridge_hold<K, E, H>(
    key: impl std::future::Future<Output = Result<K, E>>,
    slot: &mut Option<H>,
    hold: impl FnOnce() -> H,
) -> Result<K, E> {
    let key = key.await;
    if key.is_ok() {
        *slot = Some(hold());
    }
    key
}

/// Run a Tab's retry sequence, then end the press's stream hold
/// ([`capture::end_tab_stream_hold`]): no grab follows, so the stream may
/// pause while the result is analysed.
///
/// Not an async fn: an async wrapper keeps the retries future in its own
/// state as well as the awaited copy, which doubled that part of
/// `handle_capture`'s future (23,280 B in a debug build). `inspect` wraps
/// it in place.
fn then_end_tab_hold<F: std::future::Future>(
    retries: F,
) -> impl std::future::Future<Output = F::Output> {
    use futures_util::FutureExt as _;
    retries.inspect(|_| {
        capture::end_tab_stream_hold();
    })
}

/// The key event a dropped poll kept, else the next one from `next`. A kept
/// event returns on the first poll, so the keyboard arm takes it at once.
async fn pending_or_next<K>(
    pending: &mut Option<K>,
    next: impl std::future::Future<Output = K>,
) -> K {
    match pending.take() {
        Some(key) => key,
        None => next.await,
    }
}

/// [`stop_for_shutdown_or_key`] with no key: the shutdown race alone.
#[cfg(test)]
async fn stop_for_shutdown<T, S>(
    work: impl std::future::Future<Output = T>,
    shutdown: S,
) -> Result<T, ()>
where
    S: std::future::Future<Output = ()>,
{
    let mut never: Option<()> = None;
    match stop_for_shutdown_or_key(work, shutdown, std::future::pending::<()>(), &mut never).await {
        PollRace::Done(value) => Ok(value),
        PollRace::Shutdown => Err(()),
        PollRace::Preempted => unreachable!("a pending key never arrives"),
    }
}

/// Let a game-state write finish. If `shutdown` fires first, `work` keeps
/// running until `budget` elapses. The budget is read when the signal
/// fires, so it is the shared deadline's remainder (`shutdown_time_left`),
/// not a zero timeout captured before the signal.
///
/// `Err` means the deadline passed with the write still running. That drop
/// is the same bound as the rest of shutdown. A signal by itself does not
/// drop the write.
async fn finish_within_shutdown<T, S, B>(
    work: impl std::future::Future<Output = T>,
    shutdown: S,
    budget: B,
) -> Result<T, ()>
where
    S: std::future::Future<Output = ()>,
    B: FnOnce() -> std::time::Duration,
{
    tokio::pin!(work);
    tokio::pin!(shutdown);
    tokio::select! {
        biased;
        _ = &mut shutdown => match tokio::time::timeout(budget(), &mut work).await {
            Ok(value) => Ok(value),
            Err(_elapsed) => Err(()),
        },
        value = &mut work => Ok(value),
    }
}

/// What the Tab capture task sends back to the run loop.
type CaptureReports = tokio::sync::mpsc::UnboundedReceiver<(String, Result<CaptureReport, String>)>;

#[cfg(test)]
mod ocr_bench;

#[cfg(test)]
mod main_tests;
