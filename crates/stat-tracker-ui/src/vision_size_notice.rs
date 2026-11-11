//! The notice for a game capture size no installed hero icon pack
//! declares, drawn under the Overview title (never in a game row, never on
//! another screen).
//!
//! The running tracker writes the size into its status file
//! ([`stat_tracker::ocr_status::Status::vision_size`]) when it sees a size
//! the installed packs do not declare and the member has not said no. The
//! app reads that once a second (the same pid guard as the keyboard notice)
//! and, only when a sync server is set, shows the ask with four actions:
//!
//! * "Open website profile": hero icon sharing is the training crop opt-in,
//!   and members change it on the website at `/profile/edit`. The app only
//!   reads consent; it never sets it. Opening it records nothing.
//! * "No thanks": a no for that size.
//! * "Don't ask again": a no for every size.
//! * "Close": hides the ask for this run and records nothing, so it can
//!   come back after a restart until the member says no.
//!
//! A no is kept in the answers file
//! ([`stat_tracker::packs::layout::vision_sizes::Asks`]). Seeing the ask
//! records nothing. The notice is hidden only after the no is on disk; a
//! write that fails keeps it up with a line saying the answer was not saved.
//!
//! When the tracker could not tell, a warning shows instead, with or
//! without a sync server: the pack record did not read, or the answers
//! file did not read. For the answers file the member can reset it: the
//! old bytes are kept in a copy next to it, and the new file holds the
//! answers that still read in them plus a "No thanks" for the size. Games
//! record either way.

use std::path::Path;

use iced::widget::{Column, button, column, container, row, text};
use iced::{Border, Element, Padding, Task, Theme};

use crate::app::{Message, TrackerApp};
use crate::model::Screen;
use crate::theme::{self, BORDER, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_META, TEXT, TEXT_2};
use stat_tracker::ocr_status::{Status as OcrSwitchStatus, VisionSizeStatus};
use stat_tracker::packs::layout::vision_sizes::{
    ASKS_FILE, Answer, Asks, ScreenSize, record_answer, reset_unreadable, supported_sizes,
};

/// Where members turn hero icon sharing on or off, under the site URL.
pub const PROFILE_PATH: &str = "/profile/edit";

/// The ask's second line.
pub const ASK_BODY: &str = "At this size the hero icon reader may misread heroes or mark them unsure. Your games still record. You can help: share hero icons from a few games by turning on hero icon sharing in your profile on the website. Nothing is cut or sent at this size unless it is on.";

pub const OPEN_PROFILE: &str = "Open website profile";
pub const NO_THANKS: &str = "No thanks";
pub const NEVER: &str = "Don't ask again";
pub const CLOSE: &str = "Close";
pub const RESET: &str = "Reset, keep a copy";

/// Under the answers file warning: what the reset does.
pub const RESET_HINT: &str =
    "Reset keeps a copy of the old file next to it and saves a No thanks for this size.";

/// A no that did not reach the disk.
pub const ANSWER_NOT_SAVED: &str = "Your answer was not saved, so the tracker may ask again. Check that the tracker's data folder can be written, then try again.";

/// A reset that did not happen.
pub const RESET_FAILED: &str = "The file was not reset and is left as it was. Check that the tracker's data folder can be written, then try again.";

/// The ask's first line.
pub fn ask_title(size: ScreenSize) -> String {
    format!("No hero icon pack for {size} is installed yet.")
}

/// The warning when the pack record did not read.
pub fn pack_unreadable_text(size: ScreenSize) -> String {
    format!(
        "Can't tell if {size} is covered: the tracker's pack record could not be read. Your games still record. Nothing is cut or sent at this size."
    )
}

/// The warning when the answers file did not read.
pub fn answers_unreadable_text(size: ScreenSize) -> String {
    format!(
        "The file {ASKS_FILE} in the tracker's data folder could not be read. Your games still record. Nothing is cut or sent at {size} because the file could not be read, and the tracker will not overwrite it."
    )
}

/// A message from the notice. Each answer carries the size it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    OpenProfile(String),
    NoThanks(ScreenSize),
    Never(ScreenSize),
    Close(ScreenSize),
    Reset(ScreenSize),
}

/// What the notice shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Shown {
    #[default]
    Nothing,
    Ask(ScreenSize),
    /// The pack record did not read.
    PackUnreadable(ScreenSize),
    /// The answers file did not read.
    AnswersUnreadable(ScreenSize),
}

/// The last click that did not reach the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Answer,
    Reset,
}

/// What the Overview keeps for the notice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VisionNotice {
    /// The running tracker's last value, so the answers file is read only
    /// when it changes.
    seen: Option<VisionSizeStatus>,
    pub shown: Shown,
    pub failed: Option<Failure>,
}

/// What to show for the tracker's value. Reads the answers file, and the
/// pack record for a warning. Writes nothing. An ask is kept even with no
/// sync server set: [`shown_on`] hides it until one is, so a server set
/// later in the same run shows the waiting ask.
fn shown_for(data_dir: &Path, live: Option<VisionSizeStatus>) -> Shown {
    let Some(status) = live else {
        return Shown::Nothing;
    };
    let size = ScreenSize::new(status.width, status.height);
    match Asks::load(data_dir) {
        Err(e) => {
            tracing::warn!(error = %e, "capture size answers did not read");
            Shown::AnswersUnreadable(size)
        }
        // The tracker's warning, with the answers file fine: the pack
        // record. If that reads now too, the tracker looks again.
        Ok(_) if status.unreadable => match supported_sizes(data_dir) {
            Err(_) => Shown::PackUnreadable(size),
            Ok(_) => Shown::Nothing,
        },
        Ok(asks) if asks.declined(size) => Shown::Nothing,
        Ok(_) => Shown::Ask(size),
    }
}

/// Feed the running tracker's value. A new ask is kept unless the member
/// already said no to that size; [`shown_on`] draws it only when a sync
/// server is set. Nothing is written.
pub fn observe_in(notice: &mut VisionNotice, data_dir: &Path, live: Option<VisionSizeStatus>) {
    if live == notice.seen {
        return;
    }
    notice.seen = live;
    notice.failed = None;
    notice.shown = shown_for(data_dir, live);
}

/// The running tracker's value: `None` unless the status file is from the
/// tracker that runs now (`daemon_pid`).
pub(crate) fn live_vision_size(
    status: Option<&OcrSwitchStatus>,
    daemon_pid: Option<u32>,
) -> Option<VisionSizeStatus> {
    status
        .filter(|status| daemon_pid == Some(status.pid))
        .and_then(|status| status.vision_size)
}

/// The tick's read of the status file, only from the running tracker.
pub(crate) fn observe(app: &mut TrackerApp, status: Option<&OcrSwitchStatus>) {
    let live = live_vision_size(status, app.daemon.pid);
    observe_in(&mut app.overview_page.vision, &app.data_dir, live);
}

/// Keep the member's no for `size`. The ask is hidden only once the no is
/// on disk, and only when it is the ask for `size` (or any ask, for "Don't
/// ask again"), so a late click for an older size leaves a newer ask up.
/// A failed write keeps the notice and says the answer was not saved.
pub fn answer_in(notice: &mut VisionNotice, data_dir: &Path, size: ScreenSize, answer: Answer) {
    if let Err(e) = record_answer(data_dir, size, answer) {
        tracing::warn!(error = %e, "failed to record the capture size answer");
        notice.failed = Some(Failure::Answer);
        return;
    }
    notice.failed = None;
    let hides = match notice.shown {
        Shown::Ask(shown) => shown == size || answer == Answer::Never,
        _ => false,
    };
    if hides {
        notice.shown = Shown::Nothing;
    }
}

/// Hide the ask for `size` for this run. Records nothing.
pub fn close_in(notice: &mut VisionNotice, size: ScreenSize) {
    if notice.shown == Shown::Ask(size) {
        notice.shown = Shown::Nothing;
        notice.failed = None;
    }
}

/// Reset the answers file that did not read, keeping a copy of it and a
/// no for `size`. A failed reset keeps the warning and says so.
pub fn reset_in(notice: &mut VisionNotice, data_dir: &Path, size: ScreenSize) {
    match reset_unreadable(data_dir, size) {
        Ok(copy) => {
            if let Some(copy) = copy {
                tracing::info!(copy = %copy.display(), "capture size answers reset; the old file was kept");
            }
            notice.failed = None;
            if notice.shown == Shown::AnswersUnreadable(size) {
                notice.shown = Shown::Nothing;
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to reset the capture size answers");
            notice.failed = Some(Failure::Reset);
        }
    }
}

pub fn update(app: &mut TrackerApp, msg: Msg) -> Task<Message> {
    update_with(app, msg, crate::update::open_release_page);
    Task::none()
}

/// [`update`] with the link opener given, so tests open no browser.
pub(crate) fn update_with(app: &mut TrackerApp, msg: Msg, open: impl FnOnce(&str)) {
    let notice = &mut app.overview_page.vision;
    let data_dir = &app.data_dir;
    match msg {
        Msg::OpenProfile(url) => open(&url),
        Msg::NoThanks(size) => answer_in(notice, data_dir, size, Answer::Dismissed),
        Msg::Never(size) => answer_in(notice, data_dir, size, Answer::Never),
        Msg::Close(size) => close_in(notice, size),
        Msg::Reset(size) => reset_in(notice, data_dir, size),
    }
}

/// The profile page on the site the tracker syncs with, if one is set: the
/// site's origin with the path `/profile/edit`, no query, no fragment.
/// `None` for an empty or invalid URL, or one that is not http(s).
pub fn profile_url(server_url: &str) -> Option<String> {
    let raw = server_url.trim();
    if raw.is_empty() {
        return None;
    }
    let mut url = url::Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return None;
    }
    url.set_path(PROFILE_PATH);
    url.set_query(None);
    url.set_fragment(None);
    let _ = url.set_username("");
    let _ = url.set_password(None);
    Some(url.into())
}

/// What the notice shows on the current screen: nothing off Overview, and
/// no ask without a sync server.
pub(crate) fn shown_on(app: &TrackerApp) -> Shown {
    if app.screen != Screen::Overview {
        return Shown::Nothing;
    }
    match app.overview_page.vision.shown {
        Shown::Ask(_) if app.saved_config.sync.is_none() => Shown::Nothing,
        shown => shown,
    }
}

/// The notice, when there is one to show on this screen.
pub fn view(app: &TrackerApp) -> Option<Element<'static, Message>> {
    let profile = app
        .saved_config
        .sync
        .as_ref()
        .and_then(|sync| profile_url(&sync.server_url));
    view_for(shown_on(app), app.overview_page.vision.failed, profile)
}

fn view_for(
    shown: Shown,
    failed: Option<Failure>,
    profile: Option<String>,
) -> Option<Element<'static, Message>> {
    let mut body: Column<'static, Message> = match shown {
        Shown::Nothing => return None,
        Shown::PackUnreadable(size) => {
            column![crate::widgets::warn_line(&pack_unreadable_text(size))]
        }
        Shown::AnswersUnreadable(size) => column![
            crate::widgets::warn_line(&answers_unreadable_text(size)),
            text(RESET_HINT).size(SIZE_META).color(TEXT_2),
            row![action(RESET, Msg::Reset(size))].spacing(8),
        ],
        Shown::Ask(size) => {
            let mut actions = row![].spacing(8);
            if let Some(url) = profile {
                actions = actions.push(action(OPEN_PROFILE, Msg::OpenProfile(url)));
            }
            actions = actions
                .push(action(NO_THANKS, Msg::NoThanks(size)))
                .push(action(NEVER, Msg::Never(size)))
                .push(action(CLOSE, Msg::Close(size)));
            column![
                text(ask_title(size))
                    .size(SIZE_BODY)
                    .font(FONT_MEDIUM)
                    .color(TEXT),
                text(ASK_BODY).size(SIZE_META).color(TEXT_2),
                actions,
            ]
        }
    }
    .spacing(8);
    match failed {
        Some(Failure::Answer) => body = body.push(crate::widgets::warn_line(ANSWER_NOT_SAVED)),
        Some(Failure::Reset) => body = body.push(crate::widgets::warn_line(RESET_FAILED)),
        None => {}
    }
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
#[path = "tests_vision_size_notice.rs"]
mod tests;
