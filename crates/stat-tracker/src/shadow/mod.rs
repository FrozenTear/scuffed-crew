//! Shadow recognizers that run next to ocr-v1 for comparison only. The digit
//! matcher reads each accepted scoreboard and the worker logs where the two
//! disagree. The hero matcher (when templates are installed under
//! `<data_dir>/templates/heroes/`) logs each Tab row's hero. Off by default
//! (`shadow_recognizer` in config.toml, or `SCUFFED_SHADOW_RECOGNIZER=1`).
//! Nothing in here changes stored stats, uploads or capture decisions.
//!
//! Saved stats use [`reader`] only when config `reader` is `new`. That switch
//! lives in `reader_apply` and, for the top-bar map ([`read_top_bar`]), in the
//! capture path, not in this worker. `reader = "new"` also forces the log on.
//!
//! The bans and picks reader ([`bans_picks`], `cv-v1`) is a separate
//! recognizer for the ban results, hero select and ban vote screens. It is on
//! by default; `STAT_TRACKER_BANS_PICKS_RECOGNIZER=off` turns it off. It needs
//! its own icon pack under `<data_dir>/templates/bans-picks-v1/`.
//!
//! The queue title reader ([`queue_title`], `rank-v1`) reads the queue title
//! (for example OPEN QUEUE or ROLE QUEUE) at the top left of the map vote,
//! hero ban and hero select screens. It is off by default;
//! `STAT_TRACKER_QUEUE_TITLE_RECOGNIZER=on` turns it on. It needs its own
//! template pack under `<data_dir>/templates/queue-title-v1/` and returns
//! `None` for any title the pack does not know.
//!
//! The rank update reader ([`rank`], `rank-v1`) reads the competitive rank
//! screen after a game. It is off by default; `STAT_TRACKER_RANK_RECOGNIZER=on`
//! turns it on, and it needs its own pack under `<data_dir>/templates/rank-v1/`.
//!
//! [`reader`] is the single entry point for the main-reader switch
//! ([`read_board`]: every field with value, confidence and suspect flag). It
//! adds the map banner ([`banner`]) and result word ([`result`]) readers, whose
//! templates, like the hero icons, come from an optional local pack.

pub mod banner;
pub mod bans_picks;
pub mod confirm;
pub mod digits;
pub mod heroes;
pub mod log;
mod ncc;
pub mod queue_title;
pub mod rank;
pub mod reader;
pub mod result;
pub mod worker;

pub use reader::{
    AccoladeRead, BoardRead, BoardStatus, FieldRead, MapName, Reader, ReaderConfig, TabHeroes,
    TopBarMap, Value, field_names, init, install, map_name, pack_readiness, read_accolade_map,
    read_banner, read_board, read_board_with, read_board_with_banner, read_end_title,
    read_final_score, read_heroes, read_phrase, read_result, read_result_screens, read_top_bar,
    result_field, result_screens, suspect_field_names, top_bar_map,
};

#[cfg(test)]
pub(crate) mod tests_fixtures;
