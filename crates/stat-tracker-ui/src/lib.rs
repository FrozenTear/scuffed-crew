//! Iced 0.14 tracker GUI for the Scuffed Crew stat tracker (P2).
//!
//! Reads `live_snapshot.json` / `active_game` via the daemon lib API and writes
//! `StoreCommand` files under `<data_dir>/commands/`. Seasons come from
//! `GET /api/public/seasons`, cached to `<data_dir>/seasons.json`. Does not
//! change OCR, capture, sync, or the store schema. There is no software
//! `--preview` path.

pub mod aggregate;
pub mod app;
pub mod cli;
pub mod commands;
pub mod fixtures;
pub mod games;
pub mod heroes;
pub mod layout;
pub mod maps;
pub mod model;
pub mod overview;
pub mod seasons;
pub mod snapshot;
pub mod theme;
pub mod widgets;

pub use aggregate::{Aggregates, SeasonWindow, aggregate};
pub use cli::{Cli, FixtureKind};
pub use model::{Game, Role, SeasonSel};
pub use snapshot::load_snapshot;
