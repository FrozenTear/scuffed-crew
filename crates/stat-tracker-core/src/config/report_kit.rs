//! Report kit retention: `report_kit_max_games` and `report_kit_max_mb`.

use super::Config;

/// Games kept by default. Config: `report_kit_max_games`.
pub const DEFAULT_MAX_GAMES: usize = 50;
/// Total size cap by default, in MB. Config: `report_kit_max_mb`.
pub const DEFAULT_MAX_MB: u64 = 500;

/// How many game kits (`report::kit`) are kept and how large they may be
/// together. The oldest games go first; the newest game is never trimmed,
/// even when it alone is over `max_bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub max_games: usize,
    pub max_bytes: u64,
}

impl Default for Retention {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_GAMES, DEFAULT_MAX_MB)
    }
}

impl Retention {
    /// `max_games` is at least 1, so the newest game always stays.
    pub fn new(max_games: usize, max_mb: u64) -> Self {
        Self {
            max_games: max_games.max(1),
            max_bytes: max_mb.saturating_mul(1024 * 1024),
        }
    }
}

impl Config {
    /// Report kit retention from `report_kit_max_games` and `report_kit_max_mb`.
    pub fn report_kit_retention(&self) -> Retention {
        Retention::new(
            self.report_kit_max_games.unwrap_or(DEFAULT_MAX_GAMES),
            self.report_kit_max_mb.unwrap_or(DEFAULT_MAX_MB),
        )
    }
}
