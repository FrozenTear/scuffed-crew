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

/// The deferred board a Tab-opened game carries from `previous`.
type CarriedDeferred = (Counters, Option<String>, Option<chrono::DateTime<Utc>>);

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

/// One result-word sighting. `map` survives onto the confirming tick when
/// that tick's screen has no map of its own.
struct WordStreak {
    outcome: detect::MatchOutcome,
    seen_at: Instant,
    map: Option<String>,
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
