//! Read-only Iced Overview spike for the Scuffed Crew stat tracker.
//!
//! P0 scope: Overview screen, season switch, snapshot read, local aggregation.
//! Does not talk to the daemon write path (`StoreCommand`) and does not change
//! OCR / capture / store schema.

pub mod aggregate;
pub mod app;
pub mod cli;
pub mod fixtures;
pub mod model;
pub mod overview;
pub mod preview;
pub mod seasons;
pub mod snapshot;
pub mod theme;
pub mod widgets;

pub use aggregate::{Aggregates, SeasonWindow, aggregate};
pub use cli::{Cli, FixtureKind};
pub use model::{Game, Role, SeasonSel};
pub use snapshot::load_snapshot;
