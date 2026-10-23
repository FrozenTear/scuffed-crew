pub mod boundary;
pub mod capture;
pub mod capture_gate;
pub mod config;
pub mod detect;
pub mod fs_mode;
pub mod hero_auth;
pub mod host_probe;
pub mod mapcrops;
pub mod model;
pub mod names;
pub mod ocr;
pub mod ocr_status;
pub mod ocr_switch;
pub mod omp;
pub mod packs;
pub mod parse;
pub mod perf;
pub mod perf_log;
pub mod proc_id;
pub mod reader_apply;
pub mod replace;
pub mod report;
pub mod sandbox;
pub mod setup;
pub mod shadow;
pub mod stats;
pub mod storage;
pub mod suspect;
pub mod sync;
pub mod tab_retry;
pub mod version;

/// Pins the layering of the coming lib split (0.6 N4).
#[cfg(test)]
mod tests_layering;
