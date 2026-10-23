// The bottom layer, in its own crate (0.6 N4). Each module is re-exported
// here at the path it had in this crate, so `stat_tracker::config::…` and
// `crate::config::…` keep resolving.
pub use stat_tracker_core::{
    config, fs_mode, hero_auth, host_probe, model, ocr_status, omp, parse, perf, perf_log, proc_id,
    replace, sandbox, setup, suspect, version,
};

pub mod boundary;
pub mod capture;
pub mod capture_gate;
pub mod detect;
pub mod frame;
pub mod mapcrops;
pub mod names;
pub mod ocr;
pub mod ocr_switch;
pub mod packs;
pub mod reader_apply;
pub mod report;
pub mod shadow;
pub mod stats;
pub mod storage;
pub mod sync;
pub mod tab_retry;

/// Pins the layering of the coming lib split (0.6 N4).
#[cfg(test)]
mod tests_layering;

/// README, bootstrap and changelog pins on this package's version.
#[cfg(test)]
mod tests_release_pins;
