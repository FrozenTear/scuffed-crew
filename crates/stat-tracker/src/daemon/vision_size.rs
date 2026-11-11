//! The one-time ask for a game capture size the hero icon reader has no
//! icon spots for (ultrawide, 16:10, an odd UI scale).
//!
//! A Tab grab notes its frame size ([`note_frame`]). Once the capture has
//! finished, recorded or not, [`after_capture`] decides what to show for
//! that size ([`stat_tracker::packs::layout::vision_sizes::Asker`]) and
//! writes it into the status file for the app. Nothing here touches a
//! frame, the game, or the network: a size that is unsupported or that
//! cannot be checked never stops the tracker or drops a game.
//!
//! Dev only: `SCUFFED_DEV_FAKE_RESOLUTION=3440x1440` makes the check use
//! that size instead of the real frame's, to test the notice. Capture, the
//! frames and the readers are not changed by it, and nothing is written to
//! the config.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use stat_tracker::ocr_status::{VisionSizeStatus, publish_vision_size};
use stat_tracker::packs::layout::vision_sizes::{self, Asker, Decision, ScreenSize};

/// Dev-only environment variable: the capture size the notice check uses
/// in place of the real one. Not a setting, never stored.
pub(crate) const FAKE_SIZE_ENV: &str = "SCUFFED_DEV_FAKE_RESOLUTION";

/// The size of the last Tab frame grabbed, taken by [`after_capture`].
static LAST_FRAME: Mutex<Option<ScreenSize>> = Mutex::new(None);
/// This run's decisions.
static ASKER: Mutex<Option<Asker>> = Mutex::new(None);
static FAKE_SIZE: OnceLock<Option<ScreenSize>> = OnceLock::new();

/// Remember a grabbed Tab frame's size. Only the two numbers are kept.
pub(crate) fn note_frame(width: u32, height: u32) {
    *LAST_FRAME.lock().unwrap_or_else(|e| e.into_inner()) = Some(ScreenSize::new(width, height));
}

/// The dev-only fake size from the variable's value, if it is a size.
pub(crate) fn fake_size_from(raw: Option<&str>) -> Option<ScreenSize> {
    raw.and_then(ScreenSize::parse)
}

fn fake_size() -> Option<ScreenSize> {
    *FAKE_SIZE.get_or_init(|| {
        let fake = fake_size_from(std::env::var(FAKE_SIZE_ENV).ok().as_deref());
        if let Some(fake) = fake {
            tracing::warn!(target: "scuffed_stat_tracker", size = %fake, "dev only: {FAKE_SIZE_ENV} is set, the capture size notice uses this size; capture is not changed");
        }
        fake
    })
}

/// The size the notice check uses: the fake one when set, else the frame's.
pub(crate) fn checked_size(frame: ScreenSize, fake: Option<ScreenSize>) -> ScreenSize {
    fake.unwrap_or(frame)
}

/// Run after a Tab capture finished. Nothing when no frame was grabbed.
pub(crate) fn after_capture(data_dir: &Path) {
    let Some(frame) = LAST_FRAME.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    let size = checked_size(frame, fake_size());
    let mut asker = ASKER.lock().unwrap_or_else(|e| e.into_inner());
    check_size(asker.get_or_insert_with(Asker::default), data_dir, size);
}

/// Decide for `size` and publish the ask or the warning. A size decided
/// before in this run keeps what was published for it.
pub(crate) fn check_size(asker: &mut Asker, data_dir: &Path, size: ScreenSize) {
    let decision = asker.decide(
        size,
        || vision_sizes::support_in(data_dir, size),
        || vision_sizes::Asks::load(data_dir),
    );
    match decision {
        None | Some(Decision::Quiet) => {}
        Some(Decision::Ask(size)) => {
            tracing::info!(target: "scuffed_stat_tracker", size = %size, "the hero icon reader has no icon spots for this capture size: asking once; the game is recorded with OCR");
            publish(data_dir, size, false);
        }
        Some(Decision::Warn(size, reason)) => {
            tracing::warn!(target: "scuffed_stat_tracker", size = %size, reason = %reason, "could not tell if the hero icon reader supports this capture size; recording goes on and nothing is collected");
            publish(data_dir, size, true);
        }
    }
}

fn publish(data_dir: &Path, size: ScreenSize, unreadable: bool) {
    publish_vision_size(
        data_dir,
        Some(VisionSizeStatus {
            width: size.width,
            height: size.height,
            unreadable,
        }),
    );
}
