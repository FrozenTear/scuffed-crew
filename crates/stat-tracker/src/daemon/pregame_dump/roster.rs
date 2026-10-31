//! The roster screen for the pre-game saves: `roster.png` when it shows,
//! `roster_last.png` (staged while it stays up) when it ends.

use super::*;

/// The roster screen on screen: its first frame is saved, each later one
/// staged as [`ROSTER_LAST_NAME`] (nothing held).
pub(super) struct RosterOnScreen {
    folder: String,
    /// When the last roster frame was seen.
    last_at: Instant,
    /// A later frame is staged.
    staged: bool,
}

impl<F: Clone> PregameTracker<F> {
    /// The roster screen: its first frame in a game is saved as
    /// [`ROSTER_NAME`], each later one staged as [`ROSTER_LAST_NAME`]
    /// (replacing the one before), and the staged one saved when the screen
    /// ends: at a phase read, or a frame that is no roster
    /// [`ROSTER_GONE`] after the last roster frame. Once per game.
    pub(super) fn observe_roster(
        &mut self,
        now: Instant,
        seen: Seen,
        frame: Option<F>,
        shots: &mut Vec<PregameOp<F>>,
    ) {
        if let Some(r) = &self.roster
            && seen != Seen::Roster
            && (matches!(seen, Seen::Phase(_))
                || now.saturating_duration_since(r.last_at) > ROSTER_GONE)
        {
            self.end_roster(shots);
        }
        if seen != Seen::Roster || self.game_open(now).is_none() {
            return;
        }
        let Some(frame) = frame else {
            return;
        };
        match (self.roster.as_mut(), self.game.as_ref()) {
            (Some(r), _) => {
                r.last_at = now;
                r.staged = true;
                shots.push(PregameOp::Stage(PregameShot {
                    folder: r.folder.clone(),
                    name: ROSTER_LAST_NAME,
                    frame,
                }));
            }
            (None, Some(game)) if !game.roster_done => {
                shots.push(PregameOp::Save(PregameShot {
                    folder: game.folder.clone(),
                    name: ROSTER_NAME,
                    frame,
                }));
                self.roster = Some(RosterOnScreen {
                    folder: game.folder.clone(),
                    last_at: now,
                    staged: false,
                });
            }
            _ => {}
        }
    }

    /// The roster screen ended: save its staged last frame, and look for
    /// it no more in that game.
    pub(super) fn end_roster(&mut self, shots: &mut Vec<PregameOp<F>>) {
        let Some(r) = self.roster.take() else {
            return;
        };
        if let Some(game) = self.game.as_mut().filter(|g| g.folder == r.folder) {
            game.roster_done = true;
        }
        if r.staged {
            shots.push(PregameOp::Promote {
                folder: r.folder,
                name: ROSTER_LAST_NAME,
            });
        }
    }
}
