//! The 0.6 Settings page's cards and folds, after spec p6 and p7: the
//! everyday cards on the left (Player and sessions, Companion, Website
//! account, Updates) and the bodies of the ADVANCED rows other than
//! Capture and reading (Tracker service, Stored data, Website connection,
//! Troubleshooting). Rows are divided by hairlines with the control on
//! the right; switches are On/Off. Every control applies at once
//! ([`crate::settings_update`]).

use iced::widget::{Row, button, checkbox, column, container, image, row, space, text, text_input};
use iced::{Alignment, Element, Fill, Padding};

use crate::app::{Message, TrackerApp};
use crate::form_widgets::{
    REPORT_HINT, action_btn, checkbox_style, field_caption, hint, uninstall_button,
};
use crate::settings::{
    self, DEBUG_IMAGES_HELP, FIELD_SHORT, POLL_INTERVAL_HELP, PRERELEASE_UPDATES_LABEL,
    SHADOW_READER_ENV_NOTE, SHADOW_READER_HINT, SHADOW_READER_LABEL, SettingsField, SettingsToggle,
};
use crate::settings_model::url_host;
use crate::settings_rows::{
    divided, link, page_field, row_title, setting_row, sub_heading, switch, title_line,
};
use crate::settings_save::Commit;
use crate::settings_update::{Msg, apply, apply_with};
use crate::theme::{
    self, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_LABEL, SIZE_META, TEXT, TEXT_3,
};

/// Help under "Show Companion during games" (spec).
const COMPANION_HELP: &str = "A small click-through card with your live stats.";
/// What the switch gates ([`crate::companion_gate`]).
const COMPANION_LIVE_NOTE: &str = "While this is on, the bar, the tray and the shortcut hide or show it for one game. Off turns them off too.";
/// Under a website token set outside the app.
const SYNC_ENV_NOTE: &str =
    "That token is used instead of one saved here, so the website and token are locked.";
/// Under Upload games after each match (spec).
const UPLOAD_HELP: &str = "Only your own stats are sent.";
/// Sub-heading of the Sharing rows inside Website account.
const SHARING_HEADING: &str = "Sharing · both start off";
/// "Read my rank after Comp games" help.
const RANK_HELP: &str = "Saves rank, rank change and its label. Applies from the next game.";
/// "Share stat pictures" help.
const STAT_PICTURES_HELP: &str = "Uploads the grayscale stat area after each game to help train the reader. No names. Kept 30 days. Report pictures are a separate setting in Capture and reading.";
/// Help under Get alpha updates (spec).
const ALPHA_HELP: &str = "Test builds before release. They may have bugs.";
/// Under the Tracker service timing fields.
const ENTER_TO_APPLY: &str = "Press Enter to apply.";
/// Under the Website connection fields.
const SYNC_HELP: &str = "Both are needed to upload. Blank either and Apply to turn uploads off.";
/// The shortcut's own copy, as on the classic page.
const SHORTCUT_LABEL: &str = "Keyboard shortcut to hide or show the overlay";
const SHORTCUT_HELP: &str = "Works in fullscreen. Overlay is click-through.";
const SHORTCUT_FIELD_HELP: &str = "Super = Windows key. Join with +. Press Enter to apply.";

/// A boxed toggle handler, or none when the switch is disabled.
type Toggle<'a> = Option<Box<dyn Fn(bool) -> Message + 'a>>;

fn toggle<'a>(enabled: bool, f: impl Fn(bool) -> Message + 'a) -> Toggle<'a> {
    enabled.then(|| Box::new(f) as Box<dyn Fn(bool) -> Message + 'a>)
}

/// The status strip's check, the Website account's too.
pub(crate) fn check_icon(size: f32) -> Element<'static, Message> {
    image(crate::icons::handle(crate::icons::Icon::Check))
        .width(size)
        .height(size)
        .into()
}

/// Player and sessions.
pub(crate) fn player(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    divided(vec![
        crate::settings_keys::names_row(app, demo),
        crate::settings_rows::session_window_row(app),
    ])
}

/// Companion: the saved "Show Companion during games" switch, then the
/// keyboard shortcut control as the classic page has it, applying on its
/// box and on Enter.
pub(crate) fn companion(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let on = app.page_state.companion_during_games;
    let show = setting_row(
        row_title("Show Companion during games"),
        vec![hint(COMPANION_HELP), hint(COMPANION_LIVE_NOTE)],
        switch(
            on,
            toggle(!demo, |v| {
                Message::SettingsPage(Msg::CompanionDuringGames(v))
            }),
        ),
    );

    let mut boxed = checkbox(app.settings.overlay_hotkey_enabled)
        .label(SHORTCUT_LABEL)
        .size(16)
        .text_size(SIZE_BODY)
        .style(checkbox_style);
    if !demo {
        boxed = boxed.on_toggle(|v| {
            apply_with(
                Message::SettingsToggle(SettingsToggle::OverlayHotkey, v),
                Commit::Shortcut,
            )
        });
    }
    let mut shortcut = column![boxed, hint(SHORTCUT_HELP)].spacing(6);
    if app.settings.overlay_hotkey_enabled {
        let mut input = text_input(crate::hotkey::DEFAULT_BIND, &app.settings.overlay_hotkey)
            .padding(Padding::from([8, 10]))
            .size(SIZE_BODY)
            .style(theme::text_input_style)
            .width(Fill);
        if !demo {
            input = input
                .on_input(|v| Message::SettingsText(SettingsField::OverlayHotkey, v))
                .on_submit(Message::SettingsPage(Msg::Commit(Commit::Shortcut)));
        }
        shortcut = shortcut.push(
            column![
                field_caption("Shortcut"),
                container(input).width(Fill).max_width(FIELD_SHORT),
                hint(SHORTCUT_FIELD_HELP),
            ]
            .spacing(4),
        );
    }
    divided(vec![show, shortcut.into()])
}

/// The line under a refused token. Under `SCUFFED_TOKEN` the Website
/// connection fields are locked, so it points to the variable instead.
fn rejected_token_line(token_from_env: bool) -> &'static str {
    if token_from_env {
        "The website refused the token from SCUFFED_TOKEN. Set a new one there, then start the app again."
    } else {
        "The website refused the token. Paste a new one under Advanced, Website connection."
    }
}

/// Website account: the connection, uploads, and the Sharing rows.
pub(crate) fn website_account(app: &TrackerApp) -> Element<'_, Message> {
    let sync_on = app.sync_on();
    let rejected = crate::tracker_status::sync_token_rejected(app);
    let env_vars = crate::settings_env::sync_env(app.env_overlay());
    let mut connection = column![].spacing(4);
    match app.saved_config.sync.as_ref() {
        Some(sync) if sync_on => {
            // The check only while the website takes the token.
            let mark: Element<'_, Message> = if rejected {
                image(crate::icons::handle(crate::icons::Icon::Warn))
                    .width(14)
                    .height(14)
                    .into()
            } else {
                check_icon(14.0)
            };
            connection = connection.push(crate::account_card::signed_in(
                app,
                url_host(&sync.server_url),
                mark,
                env_vars.is_some(),
            ));
            if let Some(line) = crate::account_card::visibility_line(app) {
                connection = connection.push(line);
            }
        }
        _ => {
            connection = connection.push(
                text("Not connected. Games stay on your PC.")
                    .size(SIZE_BODY)
                    .font(FONT_SEMIBOLD)
                    .color(TEXT),
            );
            // Where to connect, while the Upload row that says it is hidden.
            if let Some(note) = crate::settings_keys::not_connected_note(rejected) {
                connection = connection.push(hint(note));
            }
        }
    }
    if rejected {
        connection = connection.push(
            text(rejected_token_line(env_vars.is_some()))
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(theme::WARN),
        );
    }
    if let Some(vars) = env_vars {
        connection = connection.push(crate::capture_reading::lock_line(vars));
    }

    // One pointer to Website connection: the refusal line names it; else,
    // with no website, the note under "Not connected" (while the Upload row
    // is hidden) or the Upload row's own note does.
    let demo = app.fixture.is_some();
    let uploads = crate::settings_keys::upload_row(app, demo, UPLOAD_HELP, !rejected);
    // The Sharing heading only while a sharing row shows.
    let shared = crate::settings_keys::sharing_rows(app, demo, RANK_HELP, STAT_PICTURES_HELP);
    let mut sharing = column![].spacing(12);
    if !shared.is_empty() {
        sharing = sharing
            .push(sub_heading(SHARING_HEADING))
            .push(divided(shared));
    }
    let sharing = sharing.push(crate::account_card::what_leaves_link());
    let mut rows = vec![connection.into()];
    rows.extend(uploads);
    rows.push(sharing.into());
    divided(rows)
}

/// Updates: the version and Check now, then the alpha switch.
pub(crate) fn updates(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let version = crate::settings_model::strip_version(app.installed_version.as_deref());
    let mut help: Vec<Element<'_, Message>> = Vec::new();
    match &app.update {
        Some(info) => {
            help.push(
                text(format!("Version {} is available.", info.latest))
                    .size(SIZE_LABEL)
                    .font(FONT_SEMIBOLD)
                    .color(theme::ACCENT)
                    .into(),
            );
            let install = settings::install_row(
                app.package_probe(),
                Some(info.latest.as_str()),
                app.installed_version.as_deref(),
            );
            help.push(hint(install.line));
            if install.copy && !demo {
                help.push(link("Copy command", Some(Message::CopyUpdateCmd)));
            }
        }
        // After a failed Check now only the failure line shows.
        None if app.page_state.update_check_failed_at.is_some() => {}
        None => help.push(crate::account_card::up_to_date_line(
            app.page_state.update_checked_at.as_deref(),
        )),
    }
    if let Some(at) = &app.page_state.update_check_failed_at {
        help.push(
            text(format!(
                "Could not check for updates at {at}. Try again later."
            ))
            .size(SIZE_LABEL)
            .font(FONT_MEDIUM)
            .color(theme::WARN)
            .into(),
        );
    }
    let checking = app.page_state.update_checking;
    let mut check = button(
        text(if checking { "Checking…" } else { "Check now" })
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
    )
    .padding(Padding::from([7, 14]))
    .style(theme::ghost_btn());
    if !demo && !checking {
        check = check.on_press(Message::SettingsPage(Msg::CheckUpdates));
    }
    let version_row = setting_row(
        row![
            text("Version")
                .size(SIZE_BODY)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
            text(version).size(SIZE_BODY).font(FONT_BOLD).color(TEXT),
        ]
        .spacing(6)
        .into(),
        help,
        check.into(),
    );
    let alpha = setting_row(
        row_title(PRERELEASE_UPDATES_LABEL),
        vec![hint(ALPHA_HELP)],
        switch(
            app.settings.prerelease_updates,
            toggle(!demo, |v| {
                apply(Message::SettingsToggle(
                    SettingsToggle::PrereleaseUpdates,
                    v,
                ))
            }),
        ),
    );
    divided(vec![version_row, alpha])
}

/// Tracker service: its state and Stop, Start on login, and match
/// watching with its timings. Start and Restart are in the strip.
pub(crate) fn tracker_service(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let d = &app.daemon;
    let busy = app.daemon_busy;
    let mut state_help = vec![hint(if d.service_installed {
        "Runs as a login service."
    } else {
        "No login service installed. Start launches the tracker directly."
    })];
    if let Some(pid) = d.pid {
        state_help.push(hint(format!("Process {pid}")));
    }
    if busy {
        state_help.push(hint("Working…"));
    }
    // The same words as the strip, so the two never disagree.
    let status_words = crate::tracker_status::strip_label(d.running() || app.fixture.is_some());
    let state: Element<'_, Message> = if d.running() && !demo && !busy {
        action_btn(
            "Stop",
            false,
            Message::Daemon(crate::daemon::DaemonVerb::Stop),
        )
    } else {
        text("Start and Restart are at the top.")
            .size(SIZE_LABEL)
            .font(FONT_MEDIUM)
            .color(TEXT_3)
            .into()
    };
    let status = setting_row(title_line(status_words, false, &[]), state_help, state);
    let login = setting_row(
        row_title("Start on login"),
        vec![hint("Starts the tracker when you log in.")],
        switch(
            d.autostart,
            toggle(!demo && !busy && d.service_installed, |_| {
                Message::ToggleAutostart
            }),
        ),
    );
    let watch = setting_row(
        title_line("Watch for match start and end", false, &[]),
        vec![],
        switch(
            app.settings.auto_detect_enabled,
            toggle(!demo, |v| {
                apply(Message::SettingsToggle(SettingsToggle::AutoDetect, v))
            }),
        ),
    );
    let mut rows = vec![status, login, watch];
    if app.settings.auto_detect_enabled {
        rows.push(setting_row(
            row_title("Check every (seconds)"),
            vec![hint(POLL_INTERVAL_HELP), hint(ENTER_TO_APPLY)],
            page_field(
                "auto",
                &app.settings.poll_interval_secs,
                SettingsField::PollInterval,
                Some(Commit::Timings),
                false,
                demo,
            ),
        ));
        rows.push(setting_row(
            row_title("Wait after detection (seconds)"),
            vec![hint(ENTER_TO_APPLY)],
            page_field(
                "120",
                &app.settings.cooldown_secs,
                SettingsField::Cooldown,
                Some(Commit::Timings),
                false,
                demo,
            ),
        ));
    }
    divided(rows)
}

/// Website connection: website URL and token, with their own Apply. A
/// token from outside the app locks both fields and Apply, so no save can
/// write the outside values into config.toml.
pub(crate) fn website_connection(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let env_vars = crate::settings_env::sync_env(app.env_overlay());
    let locked = demo || env_vars.is_some();
    let saved = app.saved_config.sync.as_ref();
    let typed_differs = saved.map_or(
        !app.settings.sync_url.trim().is_empty() || !app.settings.sync_token.trim().is_empty(),
        |s| {
            app.settings.sync_url.trim() != s.server_url
                || app.settings.sync_token.trim() != s.token
        },
    );
    let mut url_help = vec![];
    if let Some(warning) = settings::sync_url_problem(&app.settings.sync_url) {
        url_help.push(
            text(warning)
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(theme::DANGER)
                .into(),
        );
    }
    let url = setting_row(
        row_title("Website URL"),
        url_help,
        page_field(
            "https://your-site.com",
            &app.settings.sync_url,
            SettingsField::SyncUrl,
            Some(Commit::Sync),
            false,
            locked,
        ),
    );
    let mut token_help = vec![hint("Paste the token from the website.")];
    if let Some(vars) = env_vars {
        token_help.push(crate::capture_reading::lock_line(vars));
        token_help.push(hint(SYNC_ENV_NOTE));
        token_help.push(hint(crate::capture_reading::SERVICE_MAY_DIFFER));
    }
    let token = setting_row(
        row_title("Account token"),
        token_help,
        page_field(
            "token",
            &app.settings.sync_token,
            SettingsField::SyncToken,
            Some(Commit::Sync),
            true,
            locked,
        ),
    );
    let mut apply_btn = button(text("Apply").size(SIZE_META).font(FONT_BOLD).color(TEXT))
        .padding(Padding::from([7, 16]))
        .style(crate::settings_page::primary_btn(typed_differs && !locked));
    if typed_differs && !locked {
        apply_btn = apply_btn.on_press(Message::SettingsPage(Msg::Commit(Commit::Sync)));
    }
    let footer = Row::new()
        .spacing(12)
        .align_y(Alignment::Center)
        .push(container(hint(SYNC_HELP)).width(Fill))
        .push(apply_btn);
    divided(vec![url, token, footer.into()])
}

/// Troubleshooting: debug images, the extra number reader, then Report a
/// problem, About and Uninstall. Demo draws them without actions.
pub(crate) fn troubleshooting(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    // STAT_TRACKER_DEBUG_OCR turns them on whatever the file says, so the
    // row shows On and locks, as OCR workers does.
    let env_debug = app.env_overlay().debug_ocr;
    let mut debug_help = vec![hint(DEBUG_IMAGES_HELP)];
    if env_debug {
        debug_help.push(crate::capture_reading::lock_line(
            crate::settings_env::DEBUG_OCR_ENV,
        ));
        debug_help.push(hint(crate::capture_reading::SERVICE_MAY_DIFFER));
    }
    let debug = setting_row(
        row_title("Save debug images from each capture"),
        debug_help,
        switch(
            app.settings.debug_ocr || env_debug,
            toggle(!demo && !env_debug, |v| {
                apply(Message::SettingsToggle(SettingsToggle::DebugOcr, v))
            }),
        ),
    );
    let locked = app.settings.shadow.locked;
    let mut shadow_help = vec![hint(SHADOW_READER_HINT)];
    if locked {
        shadow_help.push(hint(SHADOW_READER_ENV_NOTE));
    }
    let shadow = setting_row(
        title_line(SHADOW_READER_LABEL, false, &[]),
        shadow_help,
        switch(
            app.settings.displayed_reader().file_flag(),
            toggle(!demo && settings::shadow_toggle_interactive(locked), |v| {
                apply(Message::SettingsToggle(SettingsToggle::ShadowRecognizer, v))
            }),
        ),
    );
    let report = |label: &'static str, msg: Message| -> Element<'static, Message> {
        let mut b = button(text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT))
            .padding(Padding::from([7, 14]))
            .style(theme::ghost_btn());
        if !demo {
            b = b.on_press(msg);
        }
        b.into()
    };
    let uninstall = uninstall_button(
        (!demo).then_some(Message::OpenUninstall),
        Padding::from([7, 14]),
    );
    let actions = column![
        hint(REPORT_HINT),
        Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(report(
                "Report a problem",
                Message::Report(crate::report::ReportMessage::Open(None)),
            ))
            .push(action_btn("About", false, Message::OpenAbout))
            .push(space().width(Fill))
            .push(uninstall),
    ]
    .spacing(8);
    divided(vec![debug, shadow, actions.into()])
}

/// Stored data keeps the classic body: its buttons are the same.
pub(crate) fn stored_data(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    settings::data_body(app, demo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_has_no_plan_codes() {
        for line in [
            COMPANION_HELP,
            COMPANION_LIVE_NOTE,
            SYNC_ENV_NOTE,
            UPLOAD_HELP,
            SHARING_HEADING,
            RANK_HELP,
            STAT_PICTURES_HELP,
            crate::account_card::WHAT_LEAVES_LINK,
            ALPHA_HELP,
            ENTER_TO_APPLY,
            SYNC_HELP,
            SHORTCUT_LABEL,
            SHORTCUT_HELP,
            SHORTCUT_FIELD_HELP,
        ] {
            assert!(!line.contains("S11"), "{line}");
        }
        assert!(
            crate::account_card::WHAT_LEAVES_LINK.ends_with(" ›"),
            "spec p6"
        );
    }

    /// Under SCUFFED_TOKEN the Website connection fields are locked, so
    /// the refused-token line names the variable, not the fields.
    #[test]
    fn a_refused_outside_token_points_to_the_variable() {
        let env = rejected_token_line(true);
        assert!(env.contains("SCUFFED_TOKEN"), "{env}");
        assert!(!env.contains("Website connection"), "{env}");
        assert!(rejected_token_line(false).contains("Website connection"));
        for line in [env, rejected_token_line(false)] {
            assert!(
                !line.contains('\u{2014}') && !line.contains('\u{2013}'),
                "{line}"
            );
        }
    }
}
