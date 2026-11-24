//! Capture pieces both Settings pages draw: the classic Capture card
//! ([`crate::settings`]) and the 0.6 Capture and reading row
//! ([`crate::capture_reading`]). The route rules (chips, notes, labels)
//! stay in [`crate::settings`]; this holds what the two pages show around
//! them, so a fix to one shows on both.

use iced::Element;
use iced::widget::{Column, text};
use stat_tracker::capture::CaptureBackend;
use stat_tracker::config::Config;

use crate::app::{Message, TrackerApp};
use crate::settings::{NoteTone, SessionKind, capture_route_notes, show_kde_direct};
use crate::theme::{self, FONT_MEDIUM, SIZE_META, TEXT_3};

/// Under the capture preview.
pub(crate) const PREVIEW_CAPTION: &str = "What the tracker sees. The scoreboard should be visible.";

/// Before the GUI has found a capture backend.
const LOOKING_FOR_BACKEND: &str = "Looking for a capture method…";

/// The line under the capture chips that says what Automatic does here.
pub(crate) fn capture_route_help(session: SessionKind) -> &'static str {
    if session == SessionKind::X11 {
        "This is an X11 session. Automatic captures the screen directly with X11. Older screenshot is the previous portal."
    } else {
        "Automatic uses the screen stream on KDE and GNOME, and direct capture on other desktops. Screen stream forces that stream. Older screenshot is the previous portal."
    }
}

/// The backend after "Detected:".
pub(crate) fn backend_label(backend: Option<CaptureBackend>) -> &'static str {
    match backend {
        None => LOOKING_FOR_BACKEND,
        Some(CaptureBackend::Wayshot) => "Wayland",
        Some(CaptureBackend::X11) => "X11",
        Some(CaptureBackend::Portal) => "Desktop portal",
        Some(CaptureBackend::Kwin) => "KDE direct (KWin)",
        Some(CaptureBackend::None) => "None available",
    }
}

/// The Capture now button's label: still looking, capturing, or ready.
pub(crate) fn capture_button_label(backend_known: bool, capturing: bool) -> &'static str {
    if !backend_known {
        LOOKING_FOR_BACKEND
    } else if capturing {
        "Capturing…"
    } else {
        "Capture now"
    }
}

/// Capture now can be pressed: a backend is known and no capture runs.
pub(crate) fn can_capture(app: &TrackerApp) -> bool {
    app.backend.is_some() && !app.capturing
}

/// The note when `STAT_TRACKER_CAPTURE` (the process environment or an
/// environment.d file) overrides the chosen method.
pub(crate) fn capture_env_note() -> Option<String> {
    Config::capture_env_note(
        std::env::var(stat_tracker::config::CAPTURE_ENV)
            .ok()
            .as_deref(),
        &stat_tracker::config::environment_d_capture_files(),
    )
}

/// KDE direct's help goes with its chip, and only where it can work.
pub(crate) fn shows_kde_direct_help(app: &TrackerApp) -> bool {
    app.session != SessionKind::X11 && show_kde_direct(&app.desktop, app.settings.capture_route)
}

/// The lines under the capture method ([`capture_route_notes`]) pushed
/// onto `body`: warnings in amber, the rest muted.
pub(crate) fn push_route_notes<'a>(
    mut body: Column<'a, Message>,
    app: &TrackerApp,
) -> Column<'a, Message> {
    for (note, tone) in capture_route_notes(
        &app.desktop,
        app.session,
        app.settings.capture_route,
        app.kde_direct.as_ref(),
        app.ocr_switch.capture_notice.as_deref(),
    ) {
        body = body.push(
            text(note)
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(match tone {
                    NoteTone::Info => TEXT_3,
                    NoteTone::Warn => theme::WARN,
                }),
        );
    }
    body
}

/// The last capture's error, in red.
pub(crate) fn preview_error(app: &TrackerApp) -> Option<Element<'static, Message>> {
    app.preview_error.as_ref().map(|err| {
        text(err.clone())
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(theme::DANGER)
            .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_button_says_what_capture_is_doing() {
        assert_eq!(capture_button_label(false, false), LOOKING_FOR_BACKEND);
        assert_eq!(capture_button_label(false, true), LOOKING_FOR_BACKEND);
        assert_eq!(capture_button_label(true, true), "Capturing…");
        assert_eq!(capture_button_label(true, false), "Capture now");
    }

    #[test]
    fn backend_labels_name_each_backend() {
        assert_eq!(backend_label(None), LOOKING_FOR_BACKEND);
        assert_eq!(
            backend_label(Some(CaptureBackend::Kwin)),
            "KDE direct (KWin)"
        );
        assert_eq!(backend_label(Some(CaptureBackend::None)), "None available");
    }
}
