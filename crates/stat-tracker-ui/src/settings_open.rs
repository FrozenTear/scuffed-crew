//! What opening the 0.6 Settings page fetches, off the UI thread: the
//! monitor list, the size of the stored data (the Stored data summary,
//! spec p6) and the name the website knows the saved token by (Website
//! account, "Signed in as"). The answers land in [`OpenState`]. Sign out
//! and the "What leaves your PC" dialog are handled here too.

use iced::Task;

use crate::app::{Message, TrackerApp};
use crate::setup_guide::{TokenCheckResult, check_token};

/// A message for the pieces Settings fetches when it opens.
#[derive(Debug, Clone)]
pub enum OpenMsg {
    /// The stored data's size in bytes.
    StoreSized(u64),
    /// The website answered for the server URL and token in `key`: the
    /// display name, or `None` when it did not name one.
    AccountChecked {
        key: (String, String),
        name: Option<String>,
    },
    /// Sign out under Website account: clear the saved token.
    SignOut,
    /// Open (`true`) or close the "What leaves your PC" dialog.
    WhatLeaves(bool),
}

impl OpenMsg {
    /// True for a message that changes a setting.
    pub(crate) fn acts(&self) -> bool {
        matches!(self, OpenMsg::SignOut)
    }
}

/// What the page fetched when it last opened.
#[derive(Debug, Clone, Default)]
pub struct OpenState {
    /// Stored data size in bytes, once measured.
    pub store_bytes: Option<u64>,
    /// The website's name for the saved token, with the URL and token it
    /// was asked for, so a changed token is never shown an old name.
    account: Option<((String, String), String)>,
    /// The URL and token a check is running for.
    asking: Option<(String, String)>,
    /// The "What leaves your PC" dialog is open.
    pub what_leaves_open: bool,
}

impl OpenState {
    /// The website's name for `key`, when the last answer was for it.
    pub fn account_name(&self, key: &(String, String)) -> Option<&str> {
        self.account
            .as_ref()
            .filter(|(k, _)| k == key)
            .map(|(_, name)| name.as_str())
    }
}

/// The saved server URL and token while uploads are on.
pub(crate) fn sync_key(app: &TrackerApp) -> Option<(String, String)> {
    app.saved_config
        .sync
        .as_ref()
        .filter(|_| app.sync_on())
        .map(|s| (s.server_url.clone(), s.token.clone()))
}

/// Settings just opened: list the outputs, measure the stored data, and
/// ask the website for the account's name. A demo asks nothing.
pub fn on_open(app: &mut TrackerApp) -> Task<Message> {
    let mut tasks = Vec::new();
    if let Some(backend) = app.backend {
        tasks.push(Task::perform(
            crate::capture::list_outputs(backend),
            Message::OutputsReady,
        ));
    }
    tasks.push(resize(app));
    if app.fixture.is_none()
        && let Some(key) = sync_key(app)
        && app.page_state.opened.account_name(&key).is_none()
        && app.page_state.opened.asking.as_ref() != Some(&key)
        // The same check as setup's: never a cleartext or bad URL.
        && let Ok(base) = crate::setup_guide::normalize_base(&key.0)
    {
        app.page_state.opened.asking = Some(key.clone());
        tasks.push(Task::perform(
            check_token(base, key.1.clone()),
            move |result| {
                let name = match result {
                    Ok(TokenCheckResult::Connected { display_name }) => Some(display_name),
                    _ => None,
                };
                Message::SettingsPage(crate::settings_update::Msg::Open(OpenMsg::AccountChecked {
                    key: key.clone(),
                    name,
                }))
            },
        ));
    }
    Task::batch(tasks)
}

/// Measure the stored data again, after Settings opens, Compact or Delete.
pub fn resize(app: &TrackerApp) -> Task<Message> {
    let dir = app.data_dir.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                stat_tracker::storage::maintain::store_size_bytes(&dir)
            })
            .await
            .unwrap_or(0)
        },
        |bytes| {
            Message::SettingsPage(crate::settings_update::Msg::Open(OpenMsg::StoreSized(
                bytes,
            )))
        },
    )
}

/// Handle one [`OpenMsg`]. Sign out goes through the page's own save
/// (`save_sync`), as a blank token and Apply under Website connection do.
pub fn update(
    app: &mut TrackerApp,
    msg: OpenMsg,
    save_sync: impl FnOnce(&mut TrackerApp) -> Task<Message>,
) -> Task<Message> {
    let state = &mut app.page_state.opened;
    match msg {
        OpenMsg::StoreSized(bytes) => state.store_bytes = Some(bytes),
        OpenMsg::AccountChecked { key, name } => {
            if state.asking.as_ref() == Some(&key) {
                state.asking = None;
            }
            state.account = name.map(|n| (key, n));
        }
        OpenMsg::WhatLeaves(open) => state.what_leaves_open = open,
        OpenMsg::SignOut => {
            state.account = None;
            app.settings.sync_token.clear();
            return save_sync(app);
        }
    }
    Task::none()
}

/// "38.2 MB", for the Stored data summary.
pub fn size_line(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_line_is_megabytes_with_one_decimal() {
        assert_eq!(size_line(38_240_000), "38.2 MB");
        assert_eq!(size_line(0), "0.0 MB");
    }

    #[test]
    fn a_name_is_shown_only_for_the_token_it_was_asked_for() {
        let key = ("https://a.example".to_string(), "t1".to_string());
        let state = OpenState {
            account: Some((key.clone(), "FrozenTear".into())),
            ..OpenState::default()
        };
        assert_eq!(state.account_name(&key), Some("FrozenTear"));
        let other = ("https://a.example".to_string(), "t2".to_string());
        assert_eq!(state.account_name(&other), None);
    }

    #[test]
    fn only_sign_out_changes_a_setting() {
        assert!(OpenMsg::SignOut.acts());
        assert!(!OpenMsg::WhatLeaves(true).acts());
        assert!(!OpenMsg::StoreSized(1).acts());
    }
}
