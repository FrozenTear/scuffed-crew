//! Shared frame pixels that both readers (the cv reader and Tesseract), the
//! detectors and the privacy blanker use: board geometry (`geometry`), row
//! and team-size layout (`layout`) and the poll-tick stability gate
//! (`stability`). Nothing here reads text, calls Tesseract or touches packs:
//! production code here names only `frame` itself, the core modules
//! (`stat-tracker-core`) and outside crates (`tests_layering` checks this).
//!
//! By convention, declare any `#[cfg(test)] mod …;` at the bottom of this
//! file, after the production modules.

pub mod geometry;
pub mod layout;
pub mod stability;
