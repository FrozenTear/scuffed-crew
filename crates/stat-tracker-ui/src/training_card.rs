//! Training crops under Website account: the site's consent per kind,
//! read-only (it is changed on the site, on the profile settings page),
//! the tracker's last send warning, and "Delete my training crops", which
//! calls `DELETE /api/training-crops/mine` after a confirm step.
//!
//! The consent is read again each time Settings opens, like the stats
//! visibility line, and after a save while Settings is open when the saved
//! website and token have no read yet. The app never sets consent: there
//! is no call for it.

use iced::Task;
use iced::widget::{Column, button, column, row, text};
use iced::{Alignment, Element, Padding};
use scuffed_types::api::training_crops::TrainingCropConsent;
use stat_tracker::config::SyncConfig;
use stat_tracker::sync::{SyncClient, UploadKind};

use crate::app::{Message, TrackerApp};
use crate::form_widgets::{action_btn, hint};
use crate::settings_open::{OpenMsg, sync_key};
use crate::settings_rows::sub_heading;
use crate::settings_update::Msg;
use crate::theme::{self, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_LABEL, SIZE_META, TEXT, TEXT_2};

/// Sub-heading inside Website account.
pub(crate) const HEADING: &str = "TRAINING CROPS";

/// What the section is about, and what this version sends.
pub(crate) const ABOUT: &str = "Small pictures of what the tracker read, sent to the website so its readers can learn. Each kind stays off until you turn it on at the website. This version sends map names, your hero portrait and your own row's numbers, and nothing while Upload games is off. No names, chat or whole screens.";

/// The confirm line before the delete.
pub(crate) const CONFIRM: &str = "Delete every training crop you sent, of every kind? The website removes them for good, and crops waiting on this PC are removed too. Your choices on the website stay as they are.";

/// A message for the training crop section.
#[derive(Debug, Clone)]
pub enum TrainingMsg {
    /// The consent read for `key`: the value, or a short reason.
    ConsentRead {
        key: (String, String),
        value: Result<TrainingCropConsent, String>,
    },
    /// "Delete my training crops": show the confirm step.
    AskDelete,
    /// Cancel on the confirm step.
    CancelDelete,
    /// "Yes, delete them".
    ConfirmDelete,
    /// The delete answer: the count the website removed, or a reason.
    Deleted(Result<u32, String>),
}

impl TrainingMsg {
    /// True for the message that changes something (refused in a demo).
    pub(crate) fn acts(&self) -> bool {
        matches!(self, TrainingMsg::ConfirmDelete)
    }
}

/// The section's state, kept while Settings is open and between opens.
#[derive(Debug, Clone, Default)]
pub struct TrainingState {
    /// The last consent answer, with the URL and token it was read for.
    consent: Option<((String, String), Result<TrainingCropConsent, String>)>,
    /// The URL and token a read is running for.
    asking: Option<(String, String)>,
    confirm_delete: bool,
    deleting: bool,
    /// The last delete answer.
    deleted: Option<Result<u32, String>>,
}

impl TrainingState {
    /// The consent read for `key`, never one read for another URL or token.
    pub(crate) fn consent(
        &self,
        key: &(String, String),
    ) -> Option<&Result<TrainingCropConsent, String>> {
        self.consent
            .as_ref()
            .filter(|(k, _)| k == key)
            .map(|(_, value)| value)
    }

    /// Sign out: forget everything read for the old token.
    pub(crate) fn forget(&mut self) {
        *self = TrainingState::default();
    }

    /// The URL and token a consent read is running for.
    #[cfg(test)]
    pub(crate) fn reading_for(&self) -> Option<&(String, String)> {
        self.asking.as_ref()
    }
}

fn page(msg: TrainingMsg) -> Message {
    Message::SettingsPage(Msg::Open(OpenMsg::Training(msg)))
}

/// A client for the saved website, refusing what setup refuses.
fn client_for(key: &(String, String)) -> Option<SyncClient> {
    let base = crate::setup_guide::normalize_base(&key.0).ok()?;
    SyncClient::try_new(SyncConfig {
        server_url: base,
        token: key.1.clone(),
    })
    .ok()
}

/// Read the consent again (Settings just opened). A demo asks nothing.
pub(crate) fn ask_consent(app: &mut TrackerApp) -> Option<Task<Message>> {
    if app.fixture.is_some() {
        return None;
    }
    let key = sync_key(app)?;
    let state = &mut app.page_state.opened.training;
    if state.asking.as_ref() == Some(&key) {
        return None;
    }
    let client = client_for(&key)?;
    state.asking = Some(key.clone());
    Some(Task::perform(
        async move {
            client
                .fetch_training_consent()
                .await
                .map_err(|e| e.plain_text())
        },
        move |value| {
            page(TrainingMsg::ConsentRead {
                key: key.clone(),
                value,
            })
        },
    ))
}

/// A settings save worked while Settings is open: when the saved website
/// and token have no consent read yet (signed in, a new token, Upload
/// games turned on), read it now, so "Reading your choices" never stays
/// with no read running. A key already read is not asked again.
pub(crate) fn ask_consent_if_unread(app: &mut TrackerApp) -> Option<Task<Message>> {
    let key = sync_key(app)?;
    if app.page_state.opened.training.consent(&key).is_some() {
        return None;
    }
    ask_consent(app)
}

/// Delete every own training crop on the website, then the crops waiting
/// on this PC. The website's count, or a reason. The delete time is
/// recorded first for the tracker's send pass (see
/// `stat_tracker::sync::mark_training_delete`).
pub(crate) async fn delete_everywhere(
    client: SyncClient,
    data_dir: std::path::PathBuf,
) -> Result<u32, String> {
    // First, so a send pass running now drops what it holds and removes
    // again what it stores while this delete runs.
    if let Err(error) = stat_tracker::sync::mark_training_delete(&data_dir) {
        tracing::warn!(%error, "could not record the training crop delete");
    }
    let deleted = client
        .delete_own_training_crops()
        .await
        .map_err(|e| e.plain_text())?;
    stat_tracker::sync::clear_training_outbox(&data_dir);
    Ok(deleted)
}

/// Handle one [`TrainingMsg`].
pub(crate) fn update(app: &mut TrackerApp, msg: TrainingMsg) -> Task<Message> {
    let state = &mut app.page_state.opened.training;
    match msg {
        TrainingMsg::ConsentRead { key, value } => {
            if state.asking.as_ref() == Some(&key) {
                state.asking = None;
                state.consent = Some((key, value));
            }
        }
        TrainingMsg::AskDelete => {
            state.confirm_delete = true;
            state.deleted = None;
        }
        TrainingMsg::CancelDelete => state.confirm_delete = false,
        TrainingMsg::ConfirmDelete => {
            if state.deleting {
                return Task::none();
            }
            state.confirm_delete = false;
            let Some(client) = sync_key(app).as_ref().and_then(client_for) else {
                let state = &mut app.page_state.opened.training;
                state.deleted = Some(Err("Connect to the website first.".into()));
                return Task::none();
            };
            let state = &mut app.page_state.opened.training;
            state.deleting = true;
            state.deleted = None;
            return Task::perform(delete_everywhere(client, app.data_dir.clone()), |result| {
                page(TrainingMsg::Deleted(result))
            });
        }
        TrainingMsg::Deleted(result) => {
            state.deleting = false;
            state.deleted = Some(result);
        }
    }
    Task::none()
}

/// One read-only line per kind: "Map names: on".
pub(crate) fn consent_lines(consent: &TrainingCropConsent) -> Vec<String> {
    UploadKind::ALL
        .into_iter()
        .map(|kind| {
            let on = if kind.consented(consent) { "on" } else { "off" };
            format!("{}: {on}", kind.label())
        })
        .collect()
}

/// Where the consent is changed.
pub(crate) fn where_to_change(server_url: &str) -> String {
    let base = server_url.trim().trim_end_matches('/');
    format!("Change these on the website, at {base}/profile/edit. The tracker only reads them.")
}

/// The line after a delete.
pub(crate) fn deleted_line(result: &Result<u32, String>) -> String {
    match result {
        Ok(1) => "Deleted 1 training crop from the website.".to_string(),
        Ok(n) => format!("Deleted {n} training crops from the website."),
        Err(reason) => format!("Could not delete your training crops. {reason}"),
    }
}

fn warn_line(copy: String) -> Element<'static, Message> {
    text(copy)
        .size(SIZE_LABEL)
        .font(FONT_MEDIUM)
        .color(theme::WARN)
        .into()
}

/// The section, while uploads are on with a saved website. `None` when
/// signed out.
pub(crate) fn section(app: &TrackerApp) -> Option<Element<'_, Message>> {
    let key = sync_key(app)?;
    let state = &app.page_state.opened.training;
    let demo = app.fixture.is_some();
    let mut body: Column<'_, Message> = column![sub_heading(HEADING), hint(ABOUT)].spacing(6);
    match state.consent(&key) {
        Some(Ok(consent)) => {
            for line in consent_lines(consent) {
                body = body.push(
                    text(line)
                        .size(SIZE_LABEL)
                        .font(FONT_SEMIBOLD)
                        .color(TEXT_2),
                );
            }
        }
        Some(Err(reason)) => {
            body = body.push(hint(format!("Could not read your choices. {reason}")));
        }
        None if !demo => body = body.push(hint("Reading your choices from the website…")),
        None => {}
    }
    body = body.push(hint(where_to_change(&key.0)));
    if let Some(warning) = app.daemon.training_warning.clone() {
        body = body.push(warn_line(warning));
    }
    if state.confirm_delete {
        body = body.push(
            text(CONFIRM)
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(theme::DANGER),
        );
    }
    if state.deleting {
        body = body.push(hint("Deleting…"));
    }
    match &state.deleted {
        Some(result @ Ok(_)) => body = body.push(hint(deleted_line(result))),
        Some(result @ Err(_)) => body = body.push(warn_line(deleted_line(result))),
        None => {}
    }
    if !demo && !state.deleting {
        let actions = if state.confirm_delete {
            row![
                action_btn("Cancel", false, page(TrainingMsg::CancelDelete)),
                button(
                    text("Yes, delete them")
                        .size(SIZE_META)
                        .font(FONT_SEMIBOLD)
                        .color(TEXT),
                )
                .padding(Padding::from([8, 16]))
                .style(theme::danger_btn(true))
                .on_press(page(TrainingMsg::ConfirmDelete)),
            ]
        } else {
            row![
                button(
                    text("Delete my training crops")
                        .size(SIZE_META)
                        .font(FONT_SEMIBOLD)
                        .color(theme::DANGER),
                )
                .padding(Padding::from([8, 16]))
                .style(theme::danger_btn(false))
                .on_press(page(TrainingMsg::AskDelete))
            ]
        };
        body = body.push(actions.spacing(8).align_y(Alignment::Center));
    }
    Some(body.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_in_app() -> (tempfile::TempDir, TrackerApp) {
        let (dir, mut app) = crate::fixtures::test_app();
        app.fixture = None;
        app.saved_config.sync = Some(SyncConfig {
            server_url: "https://crew.example".into(),
            token: "tok".into(),
        });
        (dir, app)
    }

    fn key() -> (String, String) {
        ("https://crew.example".into(), "tok".into())
    }

    fn send(app: &mut TrackerApp, msg: TrainingMsg) {
        let _ = app.update(page(msg));
    }

    fn read(app: &mut TrackerApp, value: Result<TrainingCropConsent, String>) {
        app.page_state.opened.training.asking = Some(key());
        send(app, TrainingMsg::ConsentRead { key: key(), value });
    }

    #[test]
    fn consent_is_shown_per_kind_read_only_with_where_to_change_it() {
        let consent = TrainingCropConsent {
            hero_icon: false,
            stat_digits: true,
            map_name: true,
        };
        assert_eq!(
            consent_lines(&consent),
            ["Hero icons: off", "Stat digits: on", "Map names: on"]
        );
        for url in ["https://crew.example", "https://crew.example/"] {
            let line = where_to_change(url);
            assert!(
                line.contains("https://crew.example/profile/edit."),
                "{line}"
            );
        }
    }

    #[test]
    fn the_copy_has_no_dashes() {
        let mut copy = vec![
            ABOUT.to_string(),
            CONFIRM.to_string(),
            where_to_change("https://a.example"),
            deleted_line(&Ok(1)),
            deleted_line(&Ok(3)),
            deleted_line(&Err("x".into())),
        ];
        copy.extend(consent_lines(&TrainingCropConsent::default()));
        for line in copy {
            assert!(
                !line.contains('\u{2014}') && !line.contains('\u{2013}'),
                "{line}"
            );
        }
    }

    #[test]
    fn a_read_counts_only_for_the_token_it_was_asked_for() {
        let (_dir, mut app) = signed_in_app();
        let consent = TrainingCropConsent {
            map_name: true,
            ..TrainingCropConsent::default()
        };
        read(&mut app, Ok(consent));
        let state = &app.page_state.opened.training;
        assert_eq!(state.consent(&key()), Some(&Ok(consent)));
        let other = ("https://crew.example".to_string(), "other".to_string());
        assert_eq!(state.consent(&other), None);
        // An answer nobody waits for is dropped.
        send(
            &mut app,
            TrainingMsg::ConsentRead {
                key: other.clone(),
                value: Ok(TrainingCropConsent::default()),
            },
        );
        assert_eq!(app.page_state.opened.training.consent(&other), None);
        // Sign out forgets it.
        let _ = crate::settings_open::update(&mut app, OpenMsg::SignOut, |_| Task::none());
        assert_eq!(app.page_state.opened.training.consent(&key()), None);
    }

    #[test]
    fn delete_asks_first_then_shows_the_count_or_the_error() {
        let (_dir, mut app) = signed_in_app();
        assert!(section(&app).is_some());
        send(&mut app, TrainingMsg::AskDelete);
        assert!(app.page_state.opened.training.confirm_delete);
        send(&mut app, TrainingMsg::CancelDelete);
        assert!(!app.page_state.opened.training.confirm_delete);
        assert!(
            !app.page_state.opened.training.deleting,
            "cancel deletes nothing"
        );

        send(&mut app, TrainingMsg::AskDelete);
        send(&mut app, TrainingMsg::ConfirmDelete);
        assert!(app.page_state.opened.training.deleting);
        assert!(!app.page_state.opened.training.confirm_delete);
        send(&mut app, TrainingMsg::Deleted(Ok(12)));
        let state = &app.page_state.opened.training;
        assert!(!state.deleting);
        assert_eq!(
            deleted_line(state.deleted.as_ref().unwrap()),
            "Deleted 12 training crops from the website."
        );
        send(
            &mut app,
            TrainingMsg::Deleted(Err("The website had a problem (HTTP 500).".into())),
        );
        let line = deleted_line(app.page_state.opened.training.deleted.as_ref().unwrap());
        assert!(line.starts_with("Could not delete"), "{line}");
        assert!(line.contains("500"), "{line}");
    }

    #[test]
    fn a_demo_deletes_nothing_and_signed_out_shows_no_section() {
        let (_dir, mut app) = signed_in_app();
        app.fixture = crate::fixtures::test_app().1.fixture;
        assert!(app.fixture.is_some());
        send(&mut app, TrainingMsg::ConfirmDelete);
        assert!(!app.page_state.opened.training.deleting);
        assert!(TrainingMsg::ConfirmDelete.acts());
        assert!(!TrainingMsg::AskDelete.acts());

        app.saved_config.sync = None;
        assert!(section(&app).is_none());
    }

    #[test]
    fn the_trackers_warning_is_read_for_the_section() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            crate::daemon::initial_view(dir.path()).training_warning,
            None
        );
        let status = dir.path().join("training_crops").join("status.json");
        std::fs::create_dir_all(status.parent().unwrap()).unwrap();
        std::fs::write(
            &status,
            r#"{"warning":"Training crops: could not send them.","at":"2026-11-10T18:00:00Z"}"#,
        )
        .unwrap();
        let view = crate::daemon::initial_view(dir.path());
        assert_eq!(
            view.training_warning.as_deref(),
            Some("Training crops: could not send them.")
        );
        let again = crate::daemon::refresh_view(dir.path(), &view);
        assert_eq!(again.training_warning, view.training_warning);
    }

    fn png() -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(200, 30));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// One loopback answer; returns the request line.
    fn answer_once(response: String) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let thread = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let n = std::io::Read::read(&mut sock, &mut buf).unwrap_or(0);
            let _ = std::io::Write::write_all(&mut sock, response.as_bytes());
            String::from_utf8_lossy(&buf[..n])
                .lines()
                .next()
                .unwrap_or_default()
                .to_string()
        });
        (url, thread)
    }

    fn http(status: u16, body: &str) -> String {
        format!(
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn queue_one(dir: &std::path::Path) {
        let consent = dir.join("training_crops").join("consent.json");
        std::fs::create_dir_all(consent.parent().unwrap()).unwrap();
        std::fs::write(
            &consent,
            r#"{"hero_icon":false,"stat_digits":false,"map_name":true}"#,
        )
        .unwrap();
        let token = stat_tracker::sync::random_token().unwrap();
        let meta = stat_tracker::sync::map_name_meta(&token, 1920, 1080);
        assert!(stat_tracker::sync::keep_training_crop(
            dir,
            UploadKind::MapName,
            meta,
            png()
        ));
    }

    #[tokio::test]
    async fn delete_calls_the_site_then_clears_the_local_outbox() {
        let dir = tempfile::tempdir().unwrap();
        queue_one(dir.path());
        let (url, server) = answer_once(http(200, r#"{"deleted":4}"#));
        let client = client_for(&(url, "tok".into())).unwrap();
        let got = delete_everywhere(client, dir.path().to_path_buf()).await;
        assert_eq!(got, Ok(4));
        assert_eq!(
            server.join().unwrap(),
            "DELETE /api/training-crops/mine HTTP/1.1"
        );
        assert!(!stat_tracker::sync::has_queued_crops(dir.path()));
    }

    /// A failed delete says why. The app leaves the waiting crops alone,
    /// but the delete marker is already written, so the tracker's next
    /// pass drops every crop kept before it and sends none of them (see
    /// `crops_kept_before_a_delete_are_dropped_not_sent` in the tracker).
    #[tokio::test]
    async fn a_failed_delete_says_why_and_marks_the_waiting_crops_for_the_next_pass_to_drop() {
        let dir = tempfile::tempdir().unwrap();
        queue_one(dir.path());
        std::thread::sleep(std::time::Duration::from_millis(5));
        let (url, server) = answer_once(http(503, r#"{"error":"hero_icons_disabled"}"#));
        let client = client_for(&(url, "tok".into())).unwrap();
        let got = delete_everywhere(client, dir.path().to_path_buf()).await;
        server.join().unwrap();
        let reason = got.unwrap_err();
        assert!(reason.contains("not taking training crops"), "{reason}");
        assert!(
            stat_tracker::sync::has_queued_crops(dir.path()),
            "still on disk until the next pass"
        );
        let marker: i64 = std::fs::read_to_string(dir.path().join("training_crops/deleted_at"))
            .expect("the delete marker")
            .trim()
            .parse()
            .unwrap();
        let outbox = dir.path().join("training_crops/outbox/map_name");
        for entry in std::fs::read_dir(outbox).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            let kept: i64 = name.split('-').next().unwrap().parse().unwrap();
            assert!(kept < marker, "{name} was kept before the delete");
        }
    }
}
