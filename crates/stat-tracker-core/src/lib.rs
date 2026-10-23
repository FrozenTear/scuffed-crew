//! The bottom layer of the stat tracker (0.6 N4): config, text parsing,
//! the model types the layers above share, perf and host probes, and the
//! OCR switch's status file.
//!
//! No module here uses another tracker crate. `scuffed-stat-tracker`
//! re-exports every module at the same path, so `stat_tracker::config` and
//! `crate::config` in that crate name these modules. New code may import
//! from `stat_tracker_core` directly; the re-exports are for existing paths.

pub mod config;
pub mod fs_mode;
pub mod hero_auth;
pub mod host_probe;
pub mod model;
pub mod ocr_status;
pub mod omp;
pub mod parse;
pub mod perf;
pub mod perf_log;
pub mod proc_id;
pub mod replace;
pub mod sandbox;
pub mod setup;
pub mod suspect;
pub mod version;
