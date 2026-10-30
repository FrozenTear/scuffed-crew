pub mod hero_portrait;
pub mod loading_screen;
pub mod match_end;
pub mod match_start;
pub mod names;
pub mod own_row;
#[cfg(test)]
pub(crate) mod tests_loading_stand_ins;
// Moved to `frame/stability.rs` (0.6 lane D V2); old path kept.
pub use crate::frame::stability;
// Tab key input and the game-process check read no screen: they live in
// `input.rs` and `game_running.rs`. Re-exported at their old paths.
pub use crate::game_running;
pub use crate::input::{MultiKeyboardStream, TabKey};

pub use crate::model::MatchOutcome;

#[derive(Debug, Clone, PartialEq)]
pub enum GamePhase {
    MapVote {
        maps: Vec<String>,
    },
    /// Hero ban vote (`BAN HEROES` / `VOTE TO BAN`).
    ///
    /// Future product: OCR should register the voted bans (out of scope
    /// here). [`match_start::detect_ban_screen`] is the cheap layout hook.
    /// Must not set `end_reel_wake_until` — ban UI is not an outcome-cadence
    /// wake. The poller already treats this as a match-start phase only.
    HeroBan,
    HeroSelect,
    InGame,
    Unknown,
}
