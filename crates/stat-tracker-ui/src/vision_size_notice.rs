//! The one-time notice for a game capture size the hero icon reader has no
//! icon spots for, drawn under the Overview title (never in a game row).
//!
//! The running tracker writes the size into its status file
//! ([`stat_tracker::ocr_status::Status::vision_size`]) the first time it
//! sees it and has not asked about it before. The app reads that once a
//! second (the same pid guard as the keyboard notice), records that the ask
//! was shown, and offers three actions:
//!
//! * "Open website profile": hero icon sharing is the training crop opt-in,
//!   and members change it on the website at `/profile/edit`. The app only
//!   reads consent; it never sets it.
//! * "No thanks" and "Don't ask again": both are a no for that size, kept
//!   in the answers file
//!   ([`stat_tracker::packs::layout::vision_sizes::Asks`]), so nothing is
//!   cut or sent at that size and the ask does not come back after a
//!   restart.
//!
//! When the tracker could not tell whether the size is supported, the app
//! shows a warning line instead, with no actions. Games record either way.

use std::path::Path;

use iced::widget::{button, column, container, row, text};
use iced::{Border, Element, Padding, Task, Theme};

use crate::app::{Message, TrackerApp};
use crate::theme::{self, BORDER, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_META, TEXT, TEXT_2};
use stat_tracker::ocr_status::{Status as OcrSwitchStatus, VisionSizeStatus};
use stat_tracker::packs::layout::vision_sizes::{Answer, Asks, ScreenSize, record_answer};

/// Where members turn hero icon sharing on or off, under the site URL.
pub const PROFILE_PATH: &str = "/profile/edit";

/// The ask's second line.
pub const ASK_BODY: &str = "Heroes are read with OCR at this size and may be marked unsure. Your games still record. You can help: share hero icons from a few games by turning on hero icon sharing in your profile on the website. Nothing is cut or sent at this size unless it is on.";

pub const OPEN_PROFILE: &str = "Open website profile";
pub const NO_THANKS: &str = "No thanks";
pub const NEVER: &str = "Don't ask again";

/// The ask's first line.
pub fn ask_title(size: ScreenSize) -> String {
    format!("The hero icon reader does not support {size} yet.")
}

/// The warning when the tracker could not tell.
pub fn warn_text(size: ScreenSize) -> String {
    format!(
        "Can't tell if the hero icon reader supports {size}. Your games still record. Nothing is cut or sent at this size."
    )
}

/// A message from the notice. Each carries the size it answers.
#[derive(Debug, Clone)]
pub enum Msg {
    OpenProfile(String),
    NoThanks(ScreenSize),
    Never(ScreenSize),
}

/// What the notice shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Shown {
    #[default]
    Nothing,
    Ask(ScreenSize),
    Warn(ScreenSize),
}

/// What the Overview keeps for the notice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VisionNotice {
    /// The running tracker's last value, so the answers file is read only
    /// when it changes.
    seen: Option<VisionSizeStatus>,
    pub shown: Shown,
}

/// Feed the running tracker's value. A new ask is shown unless the member
/// already said no to that size, and is then recorded as shown.
pub fn observe_in(notice: &mut VisionNotice, data_dir: &Path, live: Option<VisionSizeStatus>) {
    if live == notice.seen {
        return;
    }
    notice.seen = live;
    notice.shown = match live {
        None => Shown::Nothing,
        Some(status) => {
            let size = ScreenSize::new(status.width, status.height);
            if status.unreadable {
                Shown::Warn(size)
            } else {
                match Asks::load(data_dir) {
                    Ok(asks) if asks.declined(size) => Shown::Nothing,
                    Ok(_) => {
                        if let Err(e) = record_answer(data_dir, size, Answer::Asked) {
                            tracing::warn!(error = %e, "failed to record the capture size ask");
                        }
                        Shown::Ask(size)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "capture size answers did not read");
                        Shown::Warn(size)
                    }
                }
            }
        }
    };
}

/// The tick's read of the status file, only from the running tracker.
pub(crate) fn observe(app: &mut TrackerApp, status: Option<&OcrSwitchStatus>) {
    let pid = app.daemon.pid;
    let live = status
        .filter(|status| pid == Some(status.pid))
        .and_then(|status| status.vision_size);
    observe_in(&mut app.overview_page.vision, &app.data_dir, live);
}

/// Keep the member's no for `size` and hide the notice.
pub fn answer_in(notice: &mut VisionNotice, data_dir: &Path, size: ScreenSize, answer: Answer) {
    if let Err(e) = record_answer(data_dir, size, answer) {
        tracing::warn!(error = %e, "failed to record the capture size answer");
    }
    notice.shown = Shown::Nothing;
}

pub fn update(app: &mut TrackerApp, msg: Msg) -> Task<Message> {
    match msg {
        Msg::OpenProfile(url) => crate::update::open_release_page(&url),
        Msg::NoThanks(size) => answer_in(
            &mut app.overview_page.vision,
            &app.data_dir,
            size,
            Answer::Dismissed,
        ),
        Msg::Never(size) => answer_in(
            &mut app.overview_page.vision,
            &app.data_dir,
            size,
            Answer::Never,
        ),
    }
    Task::none()
}

/// The profile page on the site the tracker syncs with, if one is set.
pub fn profile_url(server_url: &str) -> Option<String> {
    let base = server_url.trim().trim_end_matches('/');
    (!base.is_empty()).then(|| format!("{base}{PROFILE_PATH}"))
}

/// The notice, when there is one to show.
pub fn view(app: &TrackerApp) -> Option<Element<'static, Message>> {
    let profile = app
        .saved_config
        .sync
        .as_ref()
        .and_then(|sync| profile_url(&sync.server_url));
    view_for(app.overview_page.vision.shown, profile)
}

fn view_for(shown: Shown, profile: Option<String>) -> Option<Element<'static, Message>> {
    let size = match shown {
        Shown::Nothing => return None,
        Shown::Warn(size) => return Some(crate::widgets::warn_line(&warn_text(size))),
        Shown::Ask(size) => size,
    };
    let mut actions = row![].spacing(8);
    if let Some(url) = profile {
        actions = actions.push(action(OPEN_PROFILE, Msg::OpenProfile(url)));
    }
    actions = actions
        .push(action(NO_THANKS, Msg::NoThanks(size)))
        .push(action(NEVER, Msg::Never(size)));
    let body = column![
        text(ask_title(size))
            .size(SIZE_BODY)
            .font(FONT_MEDIUM)
            .color(TEXT),
        text(ASK_BODY).size(SIZE_META).color(TEXT_2),
        actions,
    ]
    .spacing(8);
    Some(
        container(body)
            .padding(12)
            .style(theme::surface_panel)
            .into(),
    )
}

/// A small outlined button whose words are its name.
fn action(label: &'static str, msg: Msg) -> Element<'static, Message> {
    button(text(label).size(12).font(FONT_SEMIBOLD).color(TEXT))
        .padding(Padding::from([4, 10]))
        .style(|_: &Theme, status| button::Style {
            background: theme::hover_tint(status),
            text_color: TEXT,
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..button::Style::default()
        })
        .on_press(Message::OverviewPage(
            crate::overview_page::Msg::VisionSize(msg),
        ))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const UW: ScreenSize = ScreenSize::new(3440, 1440);
    const WIDE_16_10: ScreenSize = ScreenSize::new(2560, 1600);

    fn live(size: ScreenSize, unreadable: bool) -> Option<VisionSizeStatus> {
        Some(VisionSizeStatus {
            width: size.width,
            height: size.height,
            unreadable,
        })
    }

    #[test]
    fn a_new_size_is_shown_once_and_recorded_as_asked() {
        let dir = tempfile::tempdir().unwrap();
        let mut notice = VisionNotice::default();
        observe_in(&mut notice, dir.path(), None);
        assert_eq!(notice.shown, Shown::Nothing);
        observe_in(&mut notice, dir.path(), live(UW, false));
        assert_eq!(notice.shown, Shown::Ask(UW));
        assert!(Asks::load(dir.path()).unwrap().was_asked(UW));
        assert!(view_for(notice.shown, None).is_some());
    }

    #[test]
    fn no_thanks_and_dont_ask_again_hide_it_and_survive_a_restart() {
        for (msg_answer, size) in [(Answer::Dismissed, UW), (Answer::Never, WIDE_16_10)] {
            let dir = tempfile::tempdir().unwrap();
            let mut notice = VisionNotice::default();
            observe_in(&mut notice, dir.path(), live(size, false));
            answer_in(&mut notice, dir.path(), size, msg_answer);
            assert_eq!(notice.shown, Shown::Nothing);
            // The same value on the next tick does not bring it back.
            observe_in(&mut notice, dir.path(), live(size, false));
            assert_eq!(notice.shown, Shown::Nothing);
            // A restarted app reads the answer from disk.
            let mut reopened = VisionNotice::default();
            observe_in(&mut reopened, dir.path(), live(size, false));
            assert_eq!(reopened.shown, Shown::Nothing, "{msg_answer:?}");
            assert!(Asks::load(dir.path()).unwrap().declined(size));
        }
    }

    #[test]
    fn an_unreadable_size_is_a_warning_with_no_actions() {
        let dir = tempfile::tempdir().unwrap();
        let mut notice = VisionNotice::default();
        observe_in(&mut notice, dir.path(), live(UW, true));
        assert_eq!(notice.shown, Shown::Warn(UW));
        assert!(
            !dir.path()
                .join(stat_tracker::packs::layout::vision_sizes::ASKS_FILE)
                .exists()
        );
        // An answers file that does not read: a warning, not an ask.
        std::fs::write(
            dir.path()
                .join(stat_tracker::packs::layout::vision_sizes::ASKS_FILE),
            b"[",
        )
        .unwrap();
        let mut notice = VisionNotice::default();
        observe_in(&mut notice, dir.path(), live(UW, false));
        assert_eq!(notice.shown, Shown::Warn(UW));
    }

    #[test]
    fn the_tracker_clearing_it_hides_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut notice = VisionNotice::default();
        observe_in(&mut notice, dir.path(), live(UW, false));
        observe_in(&mut notice, dir.path(), None);
        assert_eq!(notice.shown, Shown::Nothing);
    }

    #[test]
    fn the_profile_link_is_the_sites_profile_page() {
        assert_eq!(
            profile_url("https://crew.example/").as_deref(),
            Some("https://crew.example/profile/edit")
        );
        assert_eq!(profile_url("  "), None);
    }

    #[test]
    fn the_copy_is_short_plain_and_has_no_em_dash() {
        for line in [
            ask_title(UW),
            warn_text(UW),
            ASK_BODY.to_string(),
            OPEN_PROFILE.to_string(),
            NO_THANKS.to_string(),
            NEVER.to_string(),
        ] {
            assert!(!line.contains('\u{2014}'), "{line}");
            assert!(!line.trim().is_empty());
        }
        assert!(ask_title(UW).contains("3440x1440"));
        assert!(warn_text(UW).contains("still record"));
    }
}
