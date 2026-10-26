//! The vision pass over a Tab frame: `analyze_frame`, the Tab rows and
//! board text, the own row and its hero, the top-bar map, and the own-name
//! seeds.

use crate::*;

/// The own-name seed store, opened only when the new reader is in effect
/// (`new_reader`, from [`new_reader_in_effect`]). A store kept for another
/// configured in-game name is cleared on open; a name that is not known in
/// this run (not set, or the fetch failed) keeps it
/// ([`detect::own_row::SeedStore::open`]).
pub(crate) fn open_own_name_seeds(
    new_reader: bool,
    config: &config::Config,
) -> Option<Arc<std::sync::Mutex<detect::own_row::SeedStore>>> {
    new_reader.then(|| {
        Arc::new(std::sync::Mutex::new(detect::own_row::SeedStore::open(
            &config.data_dir,
            config.player_name.as_deref(),
        )))
    })
}

/// The trusted own-name seeds for one vision pass. Empty under ocr-v1.
pub(crate) fn own_name_seeds(
    store: Option<&Arc<std::sync::Mutex<detect::own_row::SeedStore>>>,
) -> Vec<detect::own_row::Seed> {
    store
        .map(|store| {
            store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .trusted()
                .to_vec()
        })
        .unwrap_or_default()
}

/// Tell the own-name seed store what one Tab's board says about the trusted
/// seeds, held Tab or stored (session `session_id`). Seeds that keep naming
/// another row than the one the career hero is read on are dropped, so a
/// wrong seed cannot hide the member's real row for good; and the log says
/// once when games go by without any trusted seed.
pub(crate) async fn note_own_seed_verdict(
    store: Option<&Arc<std::sync::Mutex<detect::own_row::SeedStore>>>,
    verdict: detect::own_row::Verdict,
    session_id: &str,
) {
    let Some(store) = store else {
        return;
    };
    let session = detect::own_row::game_key(session_id);
    // No disk write unless seeds are dropped, and that is rare: no
    // blocking task for the common case.
    let noted = store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .note(verdict, &session);
    if noted.dropped > 0 {
        tracing::warn!(
            target: "scuffed_stat_tracker",
            dropped = noted.dropped,
            boards = detect::own_row::DISAGREE_DROPS,
            "own-row seeds named another row than the career hero's on this many boards in a row: dropped, the name line is learned again"
        );
    }
    if noted.unseeded_notice {
        tracing::info!(
            target: "scuffed_stat_tracker",
            games = detect::own_row::UNSEEDED_NOTICE_GAMES,
            "no own-name seed is trusted yet after this many games: the own row comes from the highlight or the career hero alone"
        );
    }
}

/// Hand a gated crop of the own row's name line to the seed store, for the
/// game `session_id` on `map`. The store trusts it only once enough crops
/// match each other: three from highlighted rows over two games, or four
/// over three games on two career heroes
/// ([`detect::own_row::SeedStore::offer`]). Runs off the async threads
/// because trusted seeds are written to disk.
pub(crate) async fn offer_own_name_seed(
    store: Option<&Arc<std::sync::Mutex<detect::own_row::SeedStore>>>,
    candidate: Option<detect::own_row::Candidate>,
    session_id: &str,
    map: &str,
) {
    let (Some(store), Some(candidate)) = (store, candidate) else {
        return;
    };
    let store = Arc::clone(store);
    let game = detect::own_row::GameTag {
        session: detect::own_row::game_key(session_id),
        map: map.to_string(),
        at: Utc::now().timestamp().max(0) as u64,
    };
    let proof = candidate.proof().label();
    let offered = tokio::task::spawn_blocking(move || {
        let mut store = store.lock().unwrap_or_else(|e| e.into_inner());
        let offer = store.offer(candidate, &game);
        (offer, store.trusted().len(), store.pending_len())
    })
    .await;
    if let Ok((offer, trusted, pending)) = offered {
        tracing::info!(target: "scuffed_stat_tracker", ?offer, proof, trusted, pending, "own-row seed crop offered");
    }
}

/// Switches for one vision pass, by name, so two of them cannot be swapped at
/// the call site (F251-4).
#[derive(Debug, Clone, Copy)]
pub(crate) struct FrameFlags {
    /// The Tab frame's accolade and rank-screen word reads run: the session
    /// is old enough for an end ([`MIN_BANNER_SESSION_AGE`]).
    pub(crate) allow_banner_recovery: bool,
    /// The open session already has a map (the lazy full-board OCR may skip).
    pub(crate) session_map_known: bool,
    /// The shadow log worker is on.
    pub(crate) shadow_on: bool,
    /// `reader = "new"`: the banner read may supply the top-bar map.
    pub(crate) use_new_reader: bool,
}

/// The blocking vision/OCR pass over one Tab frame, extracted from
/// `handle_capture` so the async orchestration reads as a straight line
/// (QUAL-001). Pure function of its inputs: decides the outcome, runs the
/// pre-OCR preflight, and (when the frame is a scoreboard) the portrait match,
/// per-cell OCR, career/map reads, and lazy full-board OCR. Returns everything
/// the caller needs, or a cheap rejection when the frame has no scoreboard rows.
pub(crate) fn analyze_frame(
    img: image::DynamicImage,
    matcher: Arc<detect::hero_portrait::PortraitMatcher>,
    player_name_owned: Option<String>,
    game_outcome: detect::MatchOutcome,
    flags: FrameFlags,
    own_seeds: Vec<detect::own_row::Seed>,
    names: &names::NameSwitch,
) -> FrameAnalysisOutcome {
    let FrameFlags {
        allow_banner_recovery,
        session_map_known,
        shadow_on,
        use_new_reader,
    } = flags;
    // Outcome: prefer the open game's result (read off the accolade
    // screen by the poller); else the accolade and rank-screen word reads
    // (only when the session is old enough for an end, see
    // MIN_BANNER_SESSION_AGE), which end in the end-title read; else the
    // end-title read alone. T1, an OCR of the top-centre band for a result
    // header, is gone (alpha.7 O4; what that was measured on is in
    // `detect_outcome_text`). The full-board OCR's header line (ET-2, in
    // `handle_capture`) still runs under ocr-v1; the new reader never runs
    // the full-board OCR (O10). The end title is read once (F257-3).
    // `reader = "new"` takes the end title from the words-v1 pack, not OCR
    // (O2). With no end-title templates loaded it falls back to the OCR
    // read, and only that fallback still reads the FINAL SCORE line (its
    // gate), with the words-v1 template (O3).
    let p4 = detect::match_end::FinalScoreSource::for_reader(use_new_reader);
    let titles = detect::match_end::Titles::for_frame(use_new_reader, &img);
    // And the accolade and rank-screen words, when this frame is one of
    // those screens (O11). A Tab frame's read is not result evidence.
    let words = detect::match_end::ResultWords::for_reader(use_new_reader);
    let frame_outcome = if allow_banner_recovery {
        detect::match_end::detect_outcome_with(&img, p4, &titles, &words)
    } else {
        detect::match_end::detect_outcome_text_with(&img, p4, &titles)
    };
    // The open session's outcome wins on a capture that stays in that session.
    // A gap split keeps this frame's own outcome. A reset split stores Unknown.
    let outcome = if game_outcome.is_decided() {
        game_outcome
    } else {
        frame_outcome
    };

    let scoreboard = Arc::new(ocr::preprocess::crop_scoreboard(&img));
    // Pre-OCR preflight. Row dips or header stat labels accept a board, then
    // the pitch has to settle 5 or 6. The table check (layout and brightness,
    // no team colour) does not block that. It runs only when the pitch fails
    // or the two pitches disagree: no table is "not a scoreboard", a table is
    // "team size unsure". The OCR gate downstream stays the final check for
    // frames that pass.
    let (preflight, row_scan) = detect::hero_portrait::preflight_scoreboard(&scoreboard);
    let team_size = match preflight {
        detect::hero_portrait::ScoreboardPreflight::NotAScoreboard => {
            return FrameAnalysisOutcome::NotAScoreboard {
                outcome,
                frame: img,
                scan: row_scan,
            };
        }
        detect::hero_portrait::ScoreboardPreflight::TeamSizeUncertain => {
            return FrameAnalysisOutcome::TeamSizeUncertain {
                outcome,
                frame: img,
                scan: row_scan,
            };
        }
        detect::hero_portrait::ScoreboardPreflight::Ready(team_size) => team_size,
    };
    // 5v5's bottom portrait ends 7 px past the 1440p crop (y 1014 on a
    // 1007 px image), so the slot code skips that row. The portrait match
    // and the hero icon read use a crop extended downward, scaled from
    // 1440p and clamped to the frame. Stat rows, the row scan and the
    // digit reader keep `scoreboard`: they place rows from its height.
    // 6v6 uses the same unpadded crop for both.
    let portrait_board = if team_size == 5 {
        Arc::new(ocr::preprocess::crop_scoreboard_for(&img, team_size))
    } else {
        Arc::clone(&scoreboard)
    };
    // Pass team_size into portrait match + cell OCR so neither re-detects
    // size or re-crops the full scoreboard (P7).
    let player_match = matcher.match_player_hero_with_team_size(&portrait_board, team_size);
    let portrait_match = player_match
        .as_ref()
        .map(|(name, conf, _)| (name.clone(), *conf));
    // The highlighted row stands on its own. A portrait below the match
    // threshold (2026-10-11: the ult-charge ring over Wrecking Ball's portrait
    // dropped it to 0.77) used to return no row at all, so every later Tab of
    // that game was rejected as "player row not found" and the slow full-board
    // OCR ran each time. The hero still comes from the career panel.
    let brightness_row_idx = detect::hero_portrait::resolve_player_row(
        player_match.map(|(_, _, idx)| idx),
        &scoreboard,
        team_size,
    );

    // ocr-v1 calibrates the stat columns and reads every name and stat cell
    // (OCR sites T2, T3, T4). The new reader reads none of them here: its
    // stats are the digit reader's (alpha.7 O10) and its own row comes from
    // the member's own-name seeds, the highlight and the career hero's row
    // (alpha.7 O9, O9b), so the rows are the board's rows with nothing read
    // on them.
    let read_cells = || ocr::recognize_scoreboard_cells_pre_cropped(&scoreboard, team_size);
    let mut rows = read_tab_rows(use_new_reader, read_cells, || ocr::unread_rows(team_size));

    // Player row. ocr-v1: if a player name is configured, scan ALL rows (both
    // teams) for a name match, this handles replays and post-match screens
    // where the player may be on team 2. Fall back to brightness-detected row
    // otherwise. `reader = "new"`: the seed match combined with the
    // highlight, and where both are silent the row the career panel's hero
    // is read on (alpha.7 O9b). Only when none of them names a row are this
    // board's name windows read, to match the configured name as main does.
    let highlight = brightness_row_idx.map(|row| detect::own_row::Highlight {
        row,
        lead: detect::hero_portrait::player_row_highlight(&scoreboard, team_size)
            .filter(|(lit, _)| *lit == row)
            .map_or(0.0, |(_, lead)| lead),
    });
    // The Tab's hero icons (heroes-v2), read once, before the own row: the
    // career hero's row is one of the own row's voices, and the same read
    // is the hero authority below. Never read under ocr-v1.
    let tab_heroes = read_tab_heroes(use_new_reader, || {
        shadow::read_heroes(&img, &portrait_board, team_size)
    });
    let name_recognizer_slot = std::cell::Cell::new(None);
    let OwnRowChoice {
        row_idx,
        name_match,
        own_row,
        candidate,
        verdict: own_seed_verdict,
    } = find_own_row(
        use_new_reader,
        || {
            player_name_owned.as_deref().and_then(|name| {
                let hit = names::match_player_row(
                    &scoreboard,
                    team_size,
                    name,
                    names,
                    names::BoardMatch::Cells,
                    || rows.iter().map(names::name_only).collect(),
                )?;
                name_recognizer_slot.set(Some(hit.recognizer));
                Some(hit.name)
            })
        },
        || {
            detect::own_row::read_own_row(
                Some(&img),
                &scoreboard,
                team_size,
                highlight,
                tab_heroes.as_ref(),
                &own_seeds,
            )
        },
        || {
            own_row_name_fallback(player_name_owned.as_deref(), &scoreboard, team_size, names).map(
                |hit| {
                    name_recognizer_slot.set(Some(hit.recognizer));
                    hit.name
                },
            )
        },
        highlight,
        tab_heroes
            .as_ref()
            .and_then(detect::own_row::career_hero_row),
        !own_seeds.is_empty(),
    );

    // Hero authority (OCR site T5). ocr-v1 reads the career-panel hero
    // title. Guard against garbage OCR (happens when there is no career
    // panel, replay, post-match, by requiring the result to actually
    // match a known hero name, which match_hero_in_text already does). The
    // new reader never reads it: the heroes-v2 reads of the career panel's
    // portrait and of the own row take its place (alpha.7 O8).
    let OwnHeroRead {
        career_hero,
        pack_hero,
        tab_heroes,
    } = read_own_hero(
        use_new_reader,
        || {
            ocr::recognize_region(&ocr::preprocess::crop_career_hero(&img))
                .ok()
                .and_then(|t| parse::match_hero_in_text(&t))
        },
        tab_heroes,
        row_idx,
    );
    // A crop of the own row's name line (the highlighted row, or the career
    // hero's) may seed the own-name set only from a live board of the
    // member's own: no chat over the row, and the career panel showing the
    // hero on that row.
    let own_name_candidate = candidate.filter(|c| {
        let chat = match img.as_rgb8() {
            Some(rgb) => stat_tracker::report::blank::chat_over_board(rgb),
            None => stat_tracker::report::blank::chat_over_board(&img.to_rgb8()),
        };
        detect::own_row::seed_gate(
            c.row,
            c.row_bottom,
            scoreboard.height(),
            chat,
            tab_heroes.as_ref(),
        )
    });
    // The top-bar OCR label is the banner read's fallback (O6). With OCR
    // off there is no fallback: the banner read is the top-bar map or
    // nothing is, and the label is not asked for.
    let map_text = if ocr::gate::is_off() {
        String::new()
    } else {
        ocr::recognize_map_label(&img, preflight)
    };
    let map_panel_raw = if map_text.trim().is_empty() {
        None
    } else {
        Some(map_text)
    };
    let top_bar = choose_top_bar_map(
        use_new_reader,
        || shadow::read_banner(&img),
        map_panel_raw.as_deref().and_then(parse::match_map_in_text),
    );
    let map_from_panel = top_bar.map;
    let map_panel_raw = top_bar_raw_label(top_bar.banner.as_ref(), map_panel_raw);

    // Full-board OCR exists only to supply raw text for hero/map name
    // lookup and the name-in-raw-text stats fallback. On the happy path,
    // player row found and parseable, hero identified (career panel,
    // heroes-v2 icons or portrait match), map already known, it is pure
    // redundancy (adaptive preprocessing plus up to three threshold sweeps), so run
    // it lazily. A portrait match alone satisfies the hero requirement:
    // replay/post-match layouts have no career panel, and the raw-text
    // hero guess loses to the portrait in the priority order anyway (H6).
    //
    // `reader = "new"` never runs it (OCR site T7): its stats, hero and map
    // come from the digit, icon and banner readers, and a Tab those cannot
    // read is refused or held, not rebuilt from text.
    let hero_identified = career_hero.is_some() || pack_hero.is_some() || portrait_match.is_some();
    let ocr = full_board_text(
        use_new_reader,
        || {
            let cells_parse =
                parse::parse_scoreboard_cells(&rows, row_idx, "", "unknown", None).is_some();
            !cells_parse || !hero_identified || (!session_map_known && map_from_panel.is_none())
        },
        || ocr::recognize(&img),
    );
    // The board the trust gate and the stats come from under the new reader,
    // with the banner and the hero icons this pass already read, so the
    // frame is read once per Tab (F251-3, O8).
    let board = read_tab_board(use_new_reader, || {
        shadow::read_board_with(&img, top_bar.banner, tab_heroes)
    });
    // A board the new reader found nothing on (heavy dimming, an unusual UI
    // scale, headers not located) is read with ocr-v1 for this Tab instead
    // of being dropped, when classic OCR is on. The Tab is then stored like
    // an ocr-v1 Tab, with recognizer `ocr-v1`.
    let ocr_fallback = board
        .as_ref()
        .and_then(|board| reader_apply::ocr_fallback_reason(board, ocr::gate::is_off()));
    if let Some(reason) = ocr_fallback {
        tracing::info!(
            target: "scuffed_stat_tracker",
            reason,
            "new reader found nothing on this board: reading it with ocr-v1"
        );
        rows = read_cells();
    }

    FrameAnalysisOutcome::Analyzed(Box::new(FrameAnalysis {
        frame_outcome,
        ocr,
        rows,
        board,
        ocr_fallback: ocr_fallback.is_some(),
        portrait_hero: portrait_match,
        career_hero,
        pack_hero,
        map_from_panel,
        map_panel_raw,
        map_from_banner: top_bar.from_banner,
        shadow_input: shadow::worker::ShadowInput::when(
            shadow_on,
            &scoreboard,
            team_size,
            img.height(),
        ),
        scoreboard: portrait_board,
        player_row_idx: row_idx,
        name_match,
        own_row,
        name_recognizer: name_recognizer_slot.get(),
        own_name_candidate: own_name_candidate.map(|c| c.candidate),
        own_seed_verdict,
        team_size,
        frame: img,
    }))
}

/// The row reads of one Tab with the daemon's reader switch (OCR sites T2,
/// T3 and T4).
///
/// * `reader = "ocr-v1"`: `cells` runs, exactly as before: the stat columns
///   are calibrated (T2) and every name and stat cell is read (T4, T3).
/// * `reader = "new"`: `cells` never runs, so there is no calibration, no
///   stat cell OCR and no name OCR. `unread` supplies the board's rows with
///   nothing read on them (`ocr::unread_rows`). The one name read left
///   under this reader is [`own_row_name_fallback`], for a board on which
///   neither a seed nor the highlight names a row.
pub(crate) fn read_tab_rows(
    use_new_reader: bool,
    cells: impl FnOnce() -> Vec<ocr::RowOcrResult>,
    unread: impl FnOnce() -> Vec<ocr::RowOcrResult>,
) -> Vec<ocr::RowOcrResult> {
    if use_new_reader { unread() } else { cells() }
}

/// The full-board OCR text of one Tab with the daemon's reader switch (OCR
/// site T7).
///
/// * `reader = "ocr-v1"`: `ocr` runs when `needed` says the cell path did
///   not supply everything, exactly as before.
/// * `reader = "new"`: neither closure runs. The text is empty, which every
///   reader of it already takes as "no text": no stats, no hero, no map, no
///   result header. An OCR failure can no longer fail the Tab.
pub(crate) fn full_board_text(
    use_new_reader: bool,
    needed: impl FnOnce() -> bool,
    ocr: impl FnOnce() -> Result<ocr::OcrResult, Box<dyn std::error::Error + Send + Sync>>,
) -> Result<ocr::OcrResult, Box<dyn std::error::Error + Send + Sync>> {
    if !use_new_reader && needed() {
        return ocr();
    }
    if !use_new_reader {
        tracing::debug!(target: "scuffed_stat_tracker", "skipping full-board OCR: cell path supplied everything");
    }
    Ok(ocr::OcrResult {
        raw_text: String::new(),
        confidence: 0,
    })
}

/// The new reader's board of one Tab. `None` under `reader = "ocr-v1"`,
/// where `read` never runs. A panic in the reader is "no board": the Tab is
/// refused, it does not fail and nothing else reads it.
pub(crate) fn read_tab_board(
    use_new_reader: bool,
    read: impl FnOnce() -> shadow::BoardRead,
) -> Option<shadow::BoardRead> {
    if !use_new_reader {
        return None;
    }
    Some(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|_| {
            tracing::warn!(target: "scuffed_stat_tracker", "board reader panicked; no board for this Tab");
            shadow::reader::not_found(shadow::BoardStatus::NotFound)
        }),
    )
}

/// The own row of one Tab (OCR site T4).
pub(crate) struct OwnRowChoice {
    pub(crate) row_idx: Option<usize>,
    /// The name match behind `row_idx`: ocr-v1's, or the new reader's name
    /// fallback. `None` when the row came from seeds or the highlight.
    pub(crate) name_match: Option<parse::NameMatch>,
    /// New reader: how the row was found. `None` under ocr-v1.
    pub(crate) own_row: Option<detect::own_row::OwnRow>,
    /// New reader: the highlighted row's name line, before the seeding gate.
    pub(crate) candidate: Option<detect::own_row::RowCandidate>,
    /// New reader: what the board says about the trusted seeds.
    pub(crate) verdict: detect::own_row::Verdict,
}

/// The new reader's name fallback (OCR site T4, kept for one case): when
/// neither an own-name seed nor the highlight names a row, match the
/// configured name on this board's name windows. This is the only name
/// read under `reader = "new"`.
///
/// `name_reader` on asks [`names::match_player_row`]. A loaded names-v1
/// pack supplies a row when that row's confidence meets the pack's own
/// threshold. A blank row, or one under the threshold, is filled from
/// Tesseract for that row. With the switch on and no pack, the match uses
/// this function's one name-window read and does not call
/// [`names::read_names`], so the board is not read a second time. With the
/// switch off, the Tesseract read is the one this fallback used before the
/// name reader.
///
/// With classic OCR off there is no such read: it returns `None` before
/// anything of OCR is touched (no crop, no OCR pool, no call at the gate).
/// A board with no seed match, no highlight and no row for the career
/// panel's hero is then held, with a WARN. That is the limit of
/// [`stat_tracker::ocr_switch::Read::OwnRow`].
pub(crate) fn own_row_name_fallback(
    player_name: Option<&str>,
    scoreboard: &image::DynamicImage,
    team_size: usize,
    names: &names::NameSwitch,
) -> Option<names::PlayerRowMatch> {
    if ocr::gate::is_off() {
        return None;
    }
    let name = player_name.filter(|name| !name.trim().is_empty())?;
    names::match_player_row(
        scoreboard,
        team_size,
        name,
        names,
        names::BoardMatch::FullBoard,
        || ocr::recognize_scoreboard_names_pre_cropped(scoreboard, team_size),
    )
}

/// The configured name's row among the name fallback's rows, when those are
/// the whole board.
///
/// A defensive check, not a known hazard. The index found here picks the
/// row on the digit reader's board (alpha.7 O10), so the names read has to
/// cover the same rows, in the same order. It does today:
/// `ocr::recognize_scoreboard_names_pre_cropped` leaves out a row only when
/// its crop falls outside the board, the preflight only settles on 5 or 6
/// rows a team, and for both the last row still fits (R2-6). If that ever
/// changes, a short set names no row and the capture is held, instead of an
/// index that may no longer mean the same row (R262-E).
#[cfg(test)]
pub(crate) fn name_on_a_full_board(
    rows: &[ocr::RowOcrResult],
    name: &str,
    team_size: usize,
) -> Option<parse::NameMatch> {
    names::match_named_rows(rows, name, team_size, names::BoardMatch::FullBoard)
}

/// Find the member's own row with the daemon's reader switch.
///
/// * `reader = "ocr-v1"`: `name_match` (the name OCR over every row) runs,
///   and the row is that match, else the highlighted row, exactly as before.
///   The seeds are not read.
/// * `reader = "new"`: `seed_read` (the own-name seeds matched on every row,
///   combined with the highlight in [`detect::own_row::combine`]) gives the
///   row. A seed against a strong highlight is no row: the capture is held.
///   Where seed and highlight are both silent, the row the career panel's
///   hero is read on is the row, marked suspect (O9b; no name is read for
///   it, so it works with Tesseract off).
///   When none of them names a row, `name_fallback` is asked: the one name
///   read left under the new reader, main's own (the name windows OCRed and
///   matched against the configured name), for this board only. Its row is
///   stored as main stores a name-matched row, and marked suspect when a
///   seed for this board exists and matched no row (F262-18). A lane that
///   turns Tesseract off passes a fallback that returns `None`; such a
///   board is then held.
///   A panic in the seed read does not fail the Tab: the highlight is used,
///   marked suspect when seeds exist (`seeded`), else the career hero's row
///   (`hero_row`).
pub(crate) fn find_own_row(
    use_new_reader: bool,
    name_match: impl FnOnce() -> Option<parse::NameMatch>,
    seed_read: impl FnOnce() -> detect::own_row::OwnRowRead,
    name_fallback: impl FnOnce() -> Option<parse::NameMatch>,
    highlight: Option<detect::own_row::Highlight>,
    hero_row: Option<usize>,
    seeded: bool,
) -> OwnRowChoice {
    if !use_new_reader {
        let name_match = name_match();
        return OwnRowChoice {
            row_idx: name_match.map(|m| m.row).or(highlight.map(|h| h.row)),
            name_match,
            own_row: None,
            candidate: None,
            verdict: detect::own_row::Verdict::Silent,
        };
    }
    let read = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(seed_read)) {
        Ok(read) => read,
        Err(_) => {
            tracing::warn!(target: "scuffed_stat_tracker", "own-row seed read panicked; the highlight row is used");
            detect::own_row::OwnRowRead::unread(highlight, hero_row, seeded)
        }
    };
    let mut own = read.own;
    let mut name_match = None;
    if own.source == detect::own_row::Source::Neither
        && let Some(found) = name_fallback()
    {
        own = detect::own_row::OwnRow::from_name(found.row, read.seeded);
        name_match = Some(found);
    }
    tracing::info!(
        target: "scuffed_stat_tracker",
        row = ?own.row,
        source = own.source.label(),
        trust = ?own.trust,
        best = read.best,
        next = read.next,
        hero_row = ?read.hero_row,
        "own row from seeds, highlight and career hero"
    );
    OwnRowChoice {
        row_idx: own.row,
        name_match,
        own_row: Some(own),
        candidate: read.candidate,
        verdict: read.verdict,
    }
}

/// The hero authority inputs of one Tab (OCR site T5).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OwnHeroRead {
    /// ocr-v1: the career-panel title. `None` under the new reader.
    pub(crate) career_hero: Option<String>,
    /// New reader: the hero the heroes-v2 icon reads authenticate
    /// ([`shadow::TabHeroes::authority`]). `None` under ocr-v1, and on any
    /// abstain.
    pub(crate) pack_hero: Option<String>,
    /// New reader: the whole icon read, for the board read. `None` under
    /// ocr-v1, and after a caught panic.
    pub(crate) tab_heroes: Option<shadow::TabHeroes>,
}

/// The hero icons of one Tab (the heroes-v2 read of the whole board and of
/// the career panel's portrait) with the daemon's reader switch.
///
/// * `reader = "ocr-v1"`: `heroes` is never called.
/// * `reader = "new"`: `heroes` runs once per Tab. Its read names the own
///   row where seed and highlight are silent
///   ([`detect::own_row::career_hero_row`]) and is the hero authority
///   ([`read_own_hero`]). A panic in the icon reader is `None`: no hero
///   authority and no career-hero row, and never a failed Tab, the same
///   isolation [`choose_top_bar_map`] gives the banner.
pub(crate) fn read_tab_heroes(
    use_new_reader: bool,
    heroes: impl FnOnce() -> shadow::TabHeroes,
) -> Option<shadow::TabHeroes> {
    if !use_new_reader {
        return None;
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(heroes)) {
        Ok(read) => Some(read),
        Err(_) => {
            tracing::warn!(target: "scuffed_stat_tracker", "hero icon reader panicked; no hero authority for this Tab");
            None
        }
    }
}

/// Read the hero authority of one Tab with the daemon's reader switch.
///
/// * `reader = "ocr-v1"`: `career` (the career-panel OCR) runs, exactly as
///   before. The icons are not read.
/// * `reader = "new"`: the career-panel title is never read. `tab_heroes`
///   is this Tab's icon read ([`read_tab_heroes`]), and `pack_hero` is the
///   hero those reads authenticate ([`shadow::TabHeroes::authority`]).
///   Suspect or special reads, two reads naming different heroes, or no icon
///   read (a panic in the icon reader) are an abstain: no hero, and never an
///   OCR read in its place.
pub(crate) fn read_own_hero(
    use_new_reader: bool,
    career: impl FnOnce() -> Option<String>,
    tab_heroes: Option<shadow::TabHeroes>,
    own_row: Option<usize>,
) -> OwnHeroRead {
    if !use_new_reader {
        return OwnHeroRead {
            career_hero: career(),
            pack_hero: None,
            tab_heroes: None,
        };
    }
    let pack_hero = tab_heroes
        .as_ref()
        .and_then(|read| read.authority(own_row))
        .map(str::to_string);
    OwnHeroRead {
        career_hero: None,
        pack_hero,
        tab_heroes,
    }
}

/// The name the raw-text stats fallback (OCR site T7) may search for.
///
/// ocr-v1: the configured name, as before. `reader = "new"`: the configured
/// name only when the own row was found. With no own row (seed and highlight
/// disagree, or neither fired) there is no name, so the full-board text
/// cannot name a row in its place and the capture is held.
pub(crate) fn text_fallback_name(
    use_new_reader: bool,
    player_row_idx: Option<usize>,
    player_name: Option<&str>,
) -> Option<&str> {
    if use_new_reader && player_row_idx.is_none() {
        return None;
    }
    player_name
}

/// Row id passed into [`boundary::plan_capture`]. `None` when the player
/// row was not identified.
pub(crate) fn identified_row_id(player_row_idx: Option<usize>) -> Option<u32> {
    player_row_idx.map(|idx| idx as u32)
}

/// The top-bar map of one Tab, and the banner read behind it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TopBarChoice {
    pub(crate) map: Option<String>,
    /// `map` is a sure banner read, not the top-bar OCR.
    pub(crate) from_banner: bool,
    /// The banner read, for the board's map field (see
    /// [`FrameAnalysis::banner`]).
    pub(crate) banner: Option<Option<shadow::banner::MapRead>>,
}

/// The top-bar map of one Tab: the read capture policy stores as
/// [`boundary::MapSource::TopBar`].
///
/// With `reader = "new"` a sure banner read (`shadow::top_bar_map` over
/// `banner()`: the map pack's glyph match, no OCR) is the top-bar map. It
/// then goes through the same policy as the top-bar OCR: the session map, the
/// vote-candidate veto, a hinted different-map split and the untracked-map
/// rules. When the banner reader abstains (no pack, no banner, a flagged
/// read, or a name the tracker would not store) the top-bar OCR is used.
/// A panic in the banner reader is an abstain too: it never fails the Tab,
/// the same isolation [`read_tab_board`] gives the board read (F251-2). With
/// `reader = "ocr-v1"` the banner is never read and the OCR label is
/// returned unchanged.
pub(crate) fn choose_top_bar_map(
    use_new_reader: bool,
    banner: impl FnOnce() -> Option<shadow::banner::MapRead>,
    ocr_map: Option<String>,
) -> TopBarChoice {
    let ocr = |banner| TopBarChoice {
        map: ocr_map.clone(),
        from_banner: false,
        banner,
    };
    if !use_new_reader {
        return ocr(None);
    }
    let read = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(banner)) {
        Ok(read) => read,
        Err(_) => {
            tracing::warn!(target: "scuffed_stat_tracker", "banner reader panicked; using the top-bar OCR for this Tab");
            return ocr(None);
        }
    };
    match read.as_ref().and_then(shadow::top_bar_map) {
        Some(top) => {
            tracing::info!(
                target: "scuffed_stat_tracker",
                map = %top.name,
                score = top.score,
                margin = top.margin,
                ocr_agrees = ocr_map.as_deref() == Some(top.name.as_str()),
                "top-bar map from the map pack"
            );
            TopBarChoice {
                map: Some(top.name),
                from_banner: true,
                banner: Some(read),
            }
        }
        None => ocr(Some(read)),
    }
}

/// The top bar's raw label of one Tab, for the Adlersbrunn check in
/// [`prepare_capture_row`] (alpha.7 M1).
///
/// Adlersbrunn is the Halloween label of Eichenwalde, and a Junkenstein
/// board is told from a real game by that label plus stats on one team
/// only. ocr-v1 finds the label in the top-bar OCR text
/// ([`parse::is_adlersbrunn_alias`]). With `reader = "new"` a sure banner
/// read of `ADLERSBRUNN` ([`shadow::banner::MapRead::is_adlersbrunn`]: the
/// map pack's glyph match, no OCR) says the same thing, so the label is that
/// spelling whatever the OCR text was, the way [`choose_top_bar_map`] takes
/// the sure banner read over the OCR label for the map. With OCR off it is
/// the only thing that says it.
///
/// Every other banner read (another map, a flagged read, no banner, no map
/// pack) and `reader = "ocr-v1"` (`banner` is `None`: the banner is never
/// read) leave the OCR label exactly as read.
pub(crate) fn top_bar_raw_label(
    banner: Option<&Option<shadow::banner::MapRead>>,
    ocr_label: Option<String>,
) -> Option<String> {
    match banner {
        Some(Some(read)) if read.is_adlersbrunn() => Some(shadow::banner::ADLERSBRUNN.to_string()),
        _ => ocr_label,
    }
}
