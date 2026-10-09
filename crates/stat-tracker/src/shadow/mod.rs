//! Shadow recognizers that run next to ocr-v1 for comparison only. The digit
//! matcher reads each accepted scoreboard and the worker logs where the two
//! disagree. The hero matcher (when templates are installed under
//! `<data_dir>/templates/heroes/`) logs each Tab row's hero. Off by default (`shadow_recognizer` in config.toml, or
//! `SCUFFED_SHADOW_RECOGNIZER=1`). Nothing in here changes stored stats,
//! uploads or capture decisions.

pub mod digits;
pub mod heroes;
pub mod log;
pub mod worker;
