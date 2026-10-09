//! Shadow recognizers that run next to ocr-v1 for comparison only. The digit
//! matcher reads each accepted scoreboard and the worker logs where the two
//! disagree. The hero matcher (when templates are installed under
//! `<data_dir>/templates/heroes/`) logs each Tab row's hero. Off by default
//! (`shadow_recognizer` in config.toml, or `SCUFFED_SHADOW_RECOGNIZER=1`). Nothing in here changes stored stats,
//! uploads or capture decisions.
//!
//! [`reader`] is the single entry point for the main-reader switch
//! ([`read_board`]: every field with value, confidence and suspect flag). It
//! adds the map banner ([`banner`]) and result word ([`result`]) readers, whose
//! templates, like the hero icons, come from an optional local pack.

pub mod banner;
pub mod confirm;
pub mod digits;
pub mod heroes;
pub mod log;
pub mod reader;
pub mod result;
pub mod worker;

pub use reader::{
    BoardRead, BoardStatus, FieldRead, MapName, Reader, ReaderConfig, Value, field_names, init,
    map_name, read_board, read_result, result_field, suspect_field_names,
};
