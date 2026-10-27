//! The daemon's code, moved out of `main.rs` by job (0.6 MAIN-15).
//!
//! Every move is pure: `scripts/check-daemon-split.py` checks each item
//! against the base `main.rs`. Each file takes the crate root's names with
//! `use crate::*;`, and this module re-exports each file's items, so
//! `main.rs` and `main_tests` name them as before.

mod debug_dumps;
mod event_loop;
mod lifecycle;
mod live_upload;
mod poll;
mod session;
mod shutdown;
mod sync_sched;
mod tab;
#[cfg(test)]
mod tests_debug_dumps;
#[cfg(test)]
mod tests_live_upload;

pub(crate) use debug_dumps::*;
pub(crate) use event_loop::*;
pub(crate) use lifecycle::*;
pub(crate) use live_upload::*;
pub(crate) use poll::*;
pub(crate) use session::*;
pub(crate) use shutdown::*;
pub(crate) use sync_sched::*;
pub(crate) use tab::*;
