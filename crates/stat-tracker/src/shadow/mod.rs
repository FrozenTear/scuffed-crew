//! Shadow recognizers that run next to ocr-v1 for comparison only. The digit
//! matcher reads each accepted scoreboard and the worker logs where the two
//! disagree. Off by default (`shadow_recognizer` in config.toml, or
//! `SCUFFED_SHADOW_RECOGNIZER=1`). Nothing in here changes stored stats,
//! uploads or capture decisions.

pub mod confirm;
pub mod digits;
pub mod log;
pub mod worker;
