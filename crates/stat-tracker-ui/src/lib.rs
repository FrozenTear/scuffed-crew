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
//! `--preview` path.

pub mod aggregate;
pub mod app;
pub mod capture;
pub mod cli;
pub mod commands;
pub mod daemon;
pub mod fixtures;
pub mod games;
pub mod heroes;
pub mod hotkey;
pub mod layout;
pub mod maps;
pub mod model;
pub mod overlay;
#[cfg(feature = "companion")]
pub mod overlay_shell;
pub mod overview;
pub mod seasons;
pub mod settings;
pub mod snapshot;
pub mod theme;
pub mod tray;
pub mod update;
pub mod widgets;

pub use aggregate::{Aggregates, SeasonWindow, aggregate};
pub use cli::{Cli, FixtureKind};
pub use model::{Game, Role, SeasonSel};
pub use snapshot::load_snapshot;
