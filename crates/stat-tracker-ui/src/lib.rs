//! Iced 0.14 tracker GUI for the Scuffed Crew stat tracker (P4 + P3 overlay).
//!
//! Reads `live_snapshot.json` / `active_game` via the daemon lib API and writes
//! `StoreCommand` files under `<data_dir>/commands/`. Seasons come from
//! `GET /api/public/seasons`, cached to `<data_dir>/seasons.json`. Settings
//! write `config.toml` through `Config::save`. Capture preview uses the
//! existing capture backends. The companion overlay is a `--companion`
//! iced_layershell process the daemon window starts and stops. Companion
//! show/hide shortcut is evdev in this process (Settings → Companion). Does
//! not change OCR, capture, sync, or the store schema. There is no software
//! `--preview` path. Release notes from the tracker changelog are bundled
//! into this binary and shown from the update prompt, once after the
//! installed version changes, and from About.

mod account_card;
pub mod aggregate;
pub mod app;
/// Shared with build.rs; the lib only compiles it for its tests.
#[cfg(test)]
mod build_id_hash;
pub mod capture;
mod capture_reading;
mod capture_shared;
mod chrome;
pub mod cli;
pub mod clipboard;
pub mod commands;
pub mod companion_gate;
#[cfg(test)]
mod copy_rules;
pub mod daemon;
mod filter_chips;
pub mod fixtures;
mod form_widgets;
pub mod games;
pub mod heroes;
pub mod hotkey;
pub mod icons;
#[cfg(test)]
mod include_tracked;
pub mod instance;
pub mod layout;
pub mod maps;
pub mod model;
pub mod nav;
pub mod nav_fit;
pub mod notes;
pub mod overlay;
#[cfg(feature = "companion")]
pub mod overlay_shell;
pub mod overview;
mod overview_heroes;
mod overview_main;
mod overview_page;
mod overview_side;
pub mod report;
pub mod restart_prompt;
pub mod seasons;
mod session_bits;
pub mod settings;
pub mod settings_cards;
mod settings_env;
mod settings_fixed;
pub mod settings_model;
mod settings_open;
pub mod settings_page;
mod settings_rows;
pub mod settings_save;
pub mod settings_update;
pub mod setup_guide;
pub mod snapshot;
mod stat_tiles;
pub mod theme;
pub mod tracker_status;
pub mod tray;
pub mod uninstall;
pub mod update;
mod update_check;
mod what_leaves;
pub mod widgets;
mod winrate_bar;

pub use aggregate::{Aggregates, SeasonWindow, aggregate};
pub use cli::{Cli, FixtureKind};
pub use model::{Game, Role, SeasonSel};
pub use snapshot::load_snapshot;
