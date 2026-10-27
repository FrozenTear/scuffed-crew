//! "Show Companion during games" (the 0.6 Settings page, stored in
//! ui_state.json) and what it gates.
//!
//! On, the Companion shows while a game runs, and the bar's Companion
//! switch, the tray item and the shortcut hide or show it for one game.
//! Off, it never starts, and those three are off too: the bar switch is
//! drawn disabled with "Companion off in Settings", the tray item and the
//! shortcut only say so, and the per-game hold is not touched. The classic
//! layout has no control for the setting, so it ignores it and behaves as
//! in 0.5.

use crate::app::TrackerApp;
use crate::nav::NavStyle;

/// The bar switch's label while the setting is off.
pub const OFF_LABEL: &str = "Companion off in Settings";

/// Toast for the tray item or the shortcut while the setting is off.
pub const OFF_TOAST: &str =
    "The Companion is off. Turn on Show Companion during games in Settings to use it.";

/// What the Companion switch in the bar draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompanionChip {
    pub showing: bool,
    pub game_running: bool,
    /// The setting lets the switch work.
    pub enabled: bool,
}

impl CompanionChip {
    pub fn label(self) -> &'static str {
        if self.enabled {
            crate::overlay::companion_copy(self.showing, self.game_running)
        } else {
            OFF_LABEL
        }
    }
}

impl TrackerApp {
    /// The Companion may show, and the bar, the tray and the shortcut may
    /// hide or show it.
    pub fn companion_enabled(&self) -> bool {
        self.nav_style == NavStyle::ClassicSidebar || self.page_state.companion_during_games
    }

    pub fn companion_chip(&self) -> CompanionChip {
        CompanionChip {
            showing: self.overlay_showing(),
            game_running: self.game_running,
            enabled: self.companion_enabled(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_in_settings_says_so_and_the_copy_has_no_dash() {
        let off = CompanionChip {
            showing: false,
            game_running: true,
            enabled: false,
        };
        assert_eq!(off.label(), OFF_LABEL);
        let on = CompanionChip {
            enabled: true,
            ..off
        };
        assert_eq!(on.label(), "Companion hidden");
        for copy in [OFF_LABEL, OFF_TOAST] {
            assert!(!copy.contains('\u{2014}') && !copy.contains('\u{2013}'));
        }
    }

    #[test]
    fn off_keeps_the_hold_and_the_classic_layout_ignores_the_setting() {
        let (dir, mut app) = crate::fixtures::test_app();
        app.game_running = true;
        app.page_state.companion_during_games = false;
        let hold = app.overlay_hold.clone();
        let _ = app.update(crate::app::Message::ToggleOverlay);
        assert_eq!(app.overlay_hold, hold, "the hold is not flipped while off");
        assert_eq!(app.toast.as_deref(), Some(OFF_TOAST));
        assert!(!app.overlay_showing());
        assert!(!app.companion_chip().enabled);
        assert_eq!(crate::seasons::load_overlay_hidden_key(dir.path()), None);

        app.nav_style = NavStyle::ClassicSidebar;
        assert!(app.companion_enabled());
        assert!(app.overlay_showing(), "classic shows it as in 0.5");
    }

    /// With the setting off and a game already running at launch, the
    /// Companion is not started at all (not started, then stopped a tick
    /// later).
    #[test]
    fn off_at_launch_never_starts_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::seasons::save_companion_during_games(dir.path(), false).expect("store");
        let app = crate::app::TrackerApp::new(crate::fixtures::test_cli(dir.path())).0;
        assert!(app.overlay_child.is_none());
        assert!(!app.page_state.companion_during_games);
    }
}
