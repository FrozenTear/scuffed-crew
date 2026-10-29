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
    // Allocator limits, also before any thread (daemon/memtune.rs).
    daemon::memtune::apply();
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

/// What the Tab capture task sends back to the run loop.
type CaptureReports = tokio::sync::mpsc::UnboundedReceiver<(String, Result<CaptureReport, String>)>;

#[cfg(test)]
mod ocr_bench;

#[cfg(test)]
mod main_tests;
