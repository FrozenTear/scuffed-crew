//! Shadow digit recognizer: a template matcher that reads each accepted
//! scoreboard next to ocr-v1 and logs where the two disagree. Off by default
//! (`shadow_recognizer` in config.toml, or `SCUFFED_SHADOW_RECOGNIZER=1`).
//! Nothing here feeds stored stats, the capture gate, or uploads.

pub mod digits;
pub mod log;
pub mod worker;
