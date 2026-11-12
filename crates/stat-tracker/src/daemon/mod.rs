//! The daemon's code, moved out of `main.rs` by job (0.6 MAIN-15).
//!
//! Every move is pure: `scripts/check-daemon-split.py` checks each item
//! against the base `main.rs`. Each file takes the crate root's names with
//! `use crate::*;`, and this module re-exports each file's items, so
//! `main.rs` and `main_tests` name them as before.

pub(crate) mod bans_picks;
mod debug_dumps;
mod event_loop;
pub(crate) mod focus;
pub(crate) mod gui_presence;
mod lifecycle;
mod live_upload;
pub(crate) mod memtune;
mod poll;
pub(crate) mod postgame_dump;
pub(crate) mod pregame_ban_title;
pub(crate) mod pregame_dump;
pub(crate) mod query_server;
pub(crate) mod queue_title;
mod session;
mod shutdown;
pub(crate) mod snapshot_policy;
mod sync_sched;
mod tab;
#[cfg(test)]
mod tests_debug_dumps;
#[cfg(test)]
mod tests_focus_gate;
#[cfg(test)]
mod tests_focus_pause;
#[cfg(test)]
mod tests_focus_sources;
#[cfg(test)]
mod tests_frame_copies;
#[cfg(test)]
mod tests_gui_presence;
#[cfg(test)]
mod tests_live_upload;
mod training_sched;
pub(crate) mod vision_size;
// The loading stand-ins shared with `detect/loading_screen.rs`'s tests;
// the lib's test-only module is not built for the bin's tests.
#[cfg(test)]
#[path = "../detect/tests_loading_stand_ins.rs"]
mod tests_loading_stand_ins;
#[cfg(test)]
mod tests_memtune;
#[cfg(test)]
mod tests_postgame_dump;
#[cfg(test)]
mod tests_postgame_round;
#[cfg(test)]
mod tests_pregame_ban_title;
#[cfg(test)]
mod tests_pregame_dump;
#[cfg(test)]
mod tests_pregame_one_tick;
#[cfg(test)]
mod tests_pregame_pending;
#[cfg(test)]
mod tests_pregame_reveal;
#[cfg(test)]
mod tests_pregame_roster;
#[cfg(test)]
mod tests_pregame_staging;
#[cfg(test)]
mod tests_pregame_tick;
#[cfg(test)]
mod tests_query_server;
#[cfg(test)]
mod tests_roster_cadence;
#[cfg(test)]
mod tests_snapshot_policy;
#[cfg(test)]
mod tests_training_sched;
#[cfg(test)]
mod tests_vision_size;

pub(crate) use debug_dumps::*;
pub(crate) use event_loop::*;
pub(crate) use lifecycle::*;
pub(crate) use live_upload::*;
pub(crate) use poll::*;
pub(crate) use session::*;
pub(crate) use shutdown::*;
pub(crate) use sync_sched::*;
pub(crate) use tab::*;
pub(crate) use training_sched::*;
