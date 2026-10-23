//! Text the tracker reads off a capture, in four layers: the map
//! catalog and its upload policy (`catalog`), map names from OCR text
//! (`map_text`, uses the catalog), the player's row by name (`names`), and
//! the scoreboard reader (`board`, uses all three). Every public item is
//! re-exported here, so `parse::X` paths are unchanged.

// Hero list + matching live in scuffed-types (hero-stats W1). Re-export for
// existing parse:: call sites and keep scoreboard find_hero available here.
pub use scuffed_types::{HEROES, canonical_hero, find_hero, match_hero_in_text};

mod board;
mod catalog;
mod map_text;
mod names;

pub use board::{
    MAX_ASSISTS, MAX_DEATHS, MAX_ELIMS, ScoreboardMiss, ScoreboardRead, both_teams_have_stats,
    looks_like_scoreboard, match_from_stats, outcome_from_board_header, parse_scoreboard_cells,
    player_row_suspect_mask, read_scoreboard, row_counts,
};
pub use catalog::{
    guess_role_public, hero_is_unknown_label, known_map_names, map_is_known, map_is_untracked,
    map_mode, mode_is_untracked, review_suspect_fields, stats_row_is_tracked, stored_game_mode,
    untracked_close_reason, upload_identity_blank, uploaded_game_mode,
};
pub use map_text::{
    canonical_map, exact_map_in_text, is_adlersbrunn_alias, map_fuzzy_threshold, match_map_in_text,
    normalize_ocr_glyphs, reject_pve_adlersbrunn,
};
pub use names::{NameMatch, find_player_row_by_name, match_player_row_by_name};

#[cfg(test)]
mod tests_board;

#[cfg(test)]
mod tests_catalog;

#[cfg(test)]
mod tests_heroes;

#[cfg(test)]
mod tests_map_text;
