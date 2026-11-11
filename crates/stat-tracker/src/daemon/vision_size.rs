//! The ask for a game capture size no installed hero icon pack declares:
//! ultrawide, 16:10, or any other size the installed packs do not list
//! (a desktop scale that changes the capture size included).
//!
//! A Tab grab notes its frame size ([`note_frame`]). After the capture has
//! finished and its result was sent on, [`after_capture`] decides what to
//! show for that size ([`stat_tracker::packs::layout::vision_sizes::Asker`])
//! and writes it into the status file for the app. Nothing here touches a
//! frame, the game, or the network: a size that is undeclared or that
//! cannot be checked never stops the tracker or drops a game.
//!
//! Debug builds only: `SCUFFED_DEV_FAKE_RESOLUTION=3440x1440` makes the
//! check use that size instead of the real frame's, to test the notice.
//! Release builds never read the variable ([`fake_size`] is `None` there).
//! Capture, the frames and the readers are not changed by it, and nothing
//! is written to the config. A member's "No thanks" for the fake size is
//! written to the data dir like any answer, so use a scratch data dir.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use stat_tracker::ocr_status::{VisionSizeStatus, publish_vision_size, read_status};
use stat_tracker::packs::layout::vision_sizes::{self, Asker, Decision, ScreenSize};

/// Debug-build environment variable: the capture size the notice check
/// uses in place of the real one. Not a setting, never stored.
pub(crate) const FAKE_SIZE_ENV: &str = "SCUFFED_DEV_FAKE_RESOLUTION";

/// The size of the last Tab frame grabbed, taken by [`after_capture`].
static LAST_FRAME: Mutex<Option<ScreenSize>> = Mutex::new(None);
/// This run's decisions, per data dir (one in production; tests use many).
static ASKERS: Mutex<BTreeMap<PathBuf, Asker>> = Mutex::new(BTreeMap::new());

/// Remember a grabbed Tab frame's size. Only the two numbers are kept.
pub(crate) fn note_frame(width: u32, height: u32) {
    *LAST_FRAME.lock().unwrap_or_else(|e| e.into_inner()) = Some(ScreenSize::new(width, height));
}

/// The fake size from the variable's value, if it is a size.
pub(crate) fn fake_size_from(raw: Option<&str>) -> Option<ScreenSize> {
    raw.and_then(ScreenSize::parse)
}

/// The fake size, read once. Debug builds only.
#[cfg(debug_assertions)]
fn fake_size() -> Option<ScreenSize> {
    static FAKE_SIZE: std::sync::OnceLock<Option<ScreenSize>> = std::sync::OnceLock::new();
    *FAKE_SIZE.get_or_init(|| {
        let fake = fake_size_from(std::env::var(FAKE_SIZE_ENV).ok().as_deref());
        if let Some(fake) = fake {
            tracing::warn!(target: "scuffed_stat_tracker", size = %fake, "debug build: {FAKE_SIZE_ENV} is set, the capture size notice uses this size; capture is not changed");
        }
        fake
    })
}

/// Release builds never read [`FAKE_SIZE_ENV`].
#[cfg(not(debug_assertions))]
fn fake_size() -> Option<ScreenSize> {
    None
}

/// The size the notice check uses: the fake one when set, else the frame's.
pub(crate) fn checked_size(frame: ScreenSize, fake: Option<ScreenSize>) -> ScreenSize {
    fake.unwrap_or(frame)
}

/// Run after a Tab capture finished and its result was sent. Nothing when
/// no frame was grabbed.
pub(crate) fn after_capture(data_dir: &Path) {
    after_capture_with(data_dir, fake_size());
}

/// [`after_capture`] with the fake size given, so tests need no
/// environment variable.
pub(crate) fn after_capture_with(data_dir: &Path, fake: Option<ScreenSize>) {
    let Some(frame) = LAST_FRAME.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    let size = checked_size(frame, fake);
    let mut askers = ASKERS.lock().unwrap_or_else(|e| e.into_inner());
    let asker = askers.entry(data_dir.to_path_buf()).or_default();
    check_size(asker, data_dir, size);
}

/// Decide for `size` and publish the ask or the warning, or take it down.
/// A size decided before in this run keeps what was published for it
/// while it is still due.
pub(crate) fn check_size(asker: &mut Asker, data_dir: &Path, size: ScreenSize) {
    let decision = asker.decide(
        size,
        || vision_sizes::support_in(data_dir, size),
        || vision_sizes::Asks::load(data_dir),
    );
    match decision {
        None | Some(Decision::Quiet) => {}
        Some(Decision::Ask(size)) => {
            tracing::info!(target: "scuffed_stat_tracker", size = %size, "no installed hero icon pack declares this capture size: asking; heroes may be misread or marked unsure, the game still records");
            publish(data_dir, size, false);
        }
        Some(Decision::Warn(size, reason)) => {
            tracing::warn!(target: "scuffed_stat_tracker", size = %size, reason = %reason, "could not tell if an installed hero icon pack declares this capture size; recording goes on and nothing is collected");
            publish(data_dir, size, true);
        }
        Some(Decision::Clear(size)) => {
            let shown = read_status(data_dir).and_then(|status| status.vision_size);
            if shown.is_some_and(|s| (s.width, s.height) == (size.width, size.height)) {
                tracing::info!(target: "scuffed_stat_tracker", size = %size, "capture size notice no longer due: taken down");
                publish_vision_size(data_dir, None);
            }
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
