//! Edit profile (`/profile/edit`), the `d1-profile-edit` frame.
//!
//! This file also holds the pieces the other account pages share (account
//! keys, my reports, device link): the account card and form CSS, the save
//! line and the page-level wait, sign-in wall and failure card.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::member_gate::Wall;
use crate::components::ui::{
    FieldError, PageHeader, PageShell, PageWidth, SkeletonBars, btn_class,
};
use crate::components::{SaveState, Toast, is_http_status, use_toast};
use crate::pages::display_name::{DisplayNameAudience, name_field_after_edit, settle_name_field};
use crate::routes::Route;
use crate::state::auth::{AuthState, use_auth};
use scuffed_api_client::ApiClient;
use scuffed_types::MeResponse;
use scuffed_types::api::{MemberSettingsResponse, StatsVisibility};

use crate::stats_privacy;

/// Prefill source: the public profile projection (flattened member + accounts).
#[derive(Debug, Clone, Deserialize)]
struct ProfilePrefill {
    display_name: String,
    bio: Option<String>,
    #[serde(default)]
    main_role: Option<String>,
    #[serde(default)]
    twitch: Option<String>,
    #[serde(default)]
    twitter: Option<String>,
    #[serde(default)]
    game_accounts: Vec<AccountPrefill>,
}

#[derive(Debug, Clone, Deserialize)]
struct AccountPrefill {
    game_id: String,
    account_name: String,
    account_id: Option<String>,
    #[serde(default)]
    rank: Option<String>,
    #[serde(default)]
    sr: Option<u32>,
    #[serde(default)]
    role: Option<String>,
}

/// PUT /api/members/:id. Plain `Option`s serialize as `null` when empty,
/// which the server's double-Option reads as an explicit clear.
#[derive(Serialize)]
struct UpdateProfileBody {
    display_name: String,
    bio: Option<String>,
    main_role: Option<String>,
    twitch: Option<String>,
    twitter: Option<String>,
}

#[derive(Serialize)]
struct UpsertAccountBody {
    game_id: String,
    account_name: String,
    account_id: Option<String>,
    rank: Option<String>,
    sr: Option<u32>,
    role: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct AccountEdit {
    game_id: String,
    account_name: String,
    account_id: Option<String>,
    rank: String,
    sr: String,
    role: String,
}

/// Stored value and the word shown for it. The empty value clears the role.
const ROLE_OPTIONS: [(&str, &str); 5] = [
    ("", "Not set"),
    ("tank", "Tank"),
    ("dps", "Damage"),
    ("support", "Support"),
    ("flex", "Flex"),
];

const NAME_ID: &str = "profile-display-name";
const NAME_ERROR_ID: &str = "profile-display-name-error";
const ROLE_ID: &str = "profile-main-role";
const BIO_ID: &str = "profile-bio";
const TWITCH_ID: &str = "profile-twitch";
const TWITCH_HINT_ID: &str = "profile-twitch-hint";
const TWITTER_ID: &str = "profile-twitter";
const TWITTER_HINT_ID: &str = "profile-twitter-hint";
const SAVE_ID: &str = "profile-save";
const SAVE_STATUS_ID: &str = "profile-save-status";
const VISIBILITY_ID: &str = "profile-stats-visibility";
const VISIBILITY_HINT_ID: &str = "profile-stats-visibility-hint";
const VISIBILITY_SAVE_ID: &str = "profile-stats-visibility-save";
const VISIBILITY_STATUS_ID: &str = "profile-stats-visibility-status";

const NAME_REQUIRED: &str = "Enter a display name.";
const PROFILE_SAVED: &str = "Profile saved.";

fn none_if_empty(s: String) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// A piece of an element id built from data: letters and digits kept,
/// everything else a dash.
fn id_part(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Id of one field of a game account row: game id plus field name.
fn account_field_id(game_id: &str, field: &str) -> String {
    format!("profile-account-{}-{field}", id_part(game_id))
}

/// The field ids of one game account card, from its game id.
#[derive(Debug, Clone, PartialEq)]
struct AccountIds {
    title: String,
    rank: String,
    sr: String,
    role: String,
    save: String,
    status: String,
}

impl AccountIds {
    fn new(game_id: &str) -> Self {
        Self {
            title: account_field_id(game_id, "title"),
            rank: account_field_id(game_id, "rank"),
            sr: account_field_id(game_id, "sr"),
            role: account_field_id(game_id, "role"),
            save: account_field_id(game_id, "save"),
            status: account_field_id(game_id, "status"),
        }
    }
}

/// The account card, field and save line shared by the account pages
/// (`d1-profile-edit`, `d1-identity`): a card padded `--space-section`,
/// labels `--space-2` above their controls, `--space-5` from one field to the
/// next, checkbox rows `--space-3` apart, and the actions `--space-5` under
/// the last field. On phone the actions stack at full width.
pub(crate) const ACCOUNT_CSS: &str = r#"
.account-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-stack);
    min-width: 0;
    margin: 0;
}
.account-card {
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    min-width: 0;
    margin: 0;
    padding: var(--space-section);
    background: var(--card-bg);
    color: var(--card-fg);
    border: 1px solid var(--card-border);
    border-radius: var(--card-radius);
    box-shadow: var(--card-shadow);
}
/* The card spaces its children with `gap`, never sibling margins: a child
   that resets its own margin (a field, a fieldset, a form) would cancel a
   sibling margin and sit flush on the next one. A checkbox pair pulls back
   to --space-3. */
.account-card > * { margin: 0; }
.account-card > .account-check + .account-check { margin-top: calc(var(--space-3) - var(--space-5)); }
.account-sub-title {
    font-family: var(--font-head);
    font-weight: var(--font-head-weight);
    letter-spacing: var(--font-head-tracking);
    line-height: var(--leading-display);
    font-size: var(--text-lg);
    color: var(--text);
    overflow-wrap: anywhere;
}
.account-lead {
    color: var(--text-3);
    font-family: var(--font-body);
    font-size: var(--text-md);
    line-height: var(--leading-body);
}
.account-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
}
.account-field > .field-input { margin: 0; }
.account-fact {
    margin: 0;
    color: var(--text);
    font-family: var(--font-body);
    font-size: var(--text-md);
    line-height: var(--leading-body);
    overflow-wrap: anywhere;
}
.account-mono { font-family: var(--font-mono); overflow-wrap: anywhere; }
.account-check {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    min-height: var(--hit-target-min);
    color: var(--text);
    font-family: var(--font-body);
    font-size: var(--text-md);
    line-height: var(--leading-body);
    cursor: pointer;
}
.account-check input[type="checkbox"] {
    flex: 0 0 auto;
    width: calc(var(--space-4) + var(--space-1) / 2);
    height: calc(var(--space-4) + var(--space-1) / 2);
    margin: 0;
    accent-color: var(--accent-solid);
    cursor: pointer;
}
.account-card :is(input, select, textarea, button, summary, a):focus-visible {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
}
.account-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
}
.account-status {
    margin: 0;
    color: var(--text-3);
    font-family: var(--font-body);
    font-size: var(--text-sm);
    line-height: var(--leading-body);
}
.account-status.is-error { color: var(--loss); }
/* No text: out of the layout, still in the tree so the next change is
   announced. */
.account-status:empty {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
}
/* The inline confirm strip sits where the card's other actions do, with
   44px buttons on the button tokens. */
.account-card .inline-confirm__actions { justify-content: flex-start; flex-wrap: wrap; }
.account-card .inline-confirm__hint { text-align: left; }
.account-card .inline-confirm__cancel,
.account-card .inline-confirm__go {
    min-height: var(--hit-target-min);
    padding: var(--space-2) var(--button-pad-x);
    font-family: var(--button-font);
    font-size: var(--button-font-size);
    font-weight: var(--button-font-weight);
    line-height: var(--leading-ui);
    border-radius: var(--button-radius);
}
.account-wait {
    margin: 0;
    color: var(--text-3);
    font-family: var(--font-body);
    font-size: var(--text-md);
    line-height: var(--leading-body);
}
@media (max-width: 820px) {
    .account-actions { flex-direction: column; align-items: stretch; }
    .account-actions .btn { width: 100%; }
}
"#;

pub(crate) const ACCOUNT_CSS_ID: &str = "scuffed-css-pages-profile_settings-account-css";

/// Profile edit only: the game account rows.
const PAGE_CSS: &str = r#"
.profile-edit-page .profile-account + .profile-account { margin-top: var(--space-5); }
.profile-account-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--space-4);
    min-width: 0;
}
@media (max-width: 820px) {
    .profile-account-grid { grid-template-columns: minmax(0, 1fr); gap: var(--space-5); }
}
"#;

/// The save line and the toast for a failed profile save. The line logs the
/// raw error (`SaveState::failed`); the toast does not, so one failure is
/// one console line (QA round 7 on 748).
fn profile_save_failure(err: &scuffed_api_client::ClientError) -> (SaveState, Toast) {
    (
        SaveState::failed("Could not save your profile", err),
        Toast::error_logged_elsewhere("Save failed", err),
    )
}

/// [`profile_save_failure`] for a game account row: one console line.
fn account_save_failure(err: &scuffed_api_client::ClientError) -> (SaveState, Toast) {
    (
        SaveState::failed("Could not save the game account", err),
        Toast::error_logged_elsewhere("Save failed", err),
    )
}

/// The save line of an account form: one `role="status"` paragraph that is
/// always rendered, so a change of its text is announced. A failure keeps the
/// server's reason (see [`SaveState::failed`]).
#[component]
pub(crate) fn AccountStatus(id: String, state: SaveState) -> Element {
    let text = state.text();
    rsx! {
        p {
            id,
            class: if state.is_error() { "account-status is-error" } else { "account-status" },
            role: "status",
            "{text}"
        }
    }
}

/// What an account page shows before its own content.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AccountView {
    /// The page's data is on its way.
    Loading,
    /// Not signed in.
    SignIn,
    /// Signed in without an org membership.
    Apply,
    /// The load failed; the server's reason.
    Failed(String),
    /// The page's own content.
    Ready,
}

/// The view the session alone decides: a guest gets the sign-in wall and a
/// signed-in non-member the membership wall. `None` while the session is
/// still checked, or for a member.
pub(crate) fn account_view_from_auth(auth: &AuthState) -> Option<AccountView> {
    if auth.loading {
        None
    } else if !auth.is_logged_in() {
        Some(AccountView::SignIn)
    } else if !auth.is_org_member() {
        Some(AccountView::Apply)
    } else {
        None
    }
}

/// The view for a failed load: a 401 is the sign-in wall, anything else the
/// failure card with the server's reason.
pub(crate) fn account_view_from_error(reason: &str) -> AccountView {
    if is_http_status(reason, 401) {
        AccountView::SignIn
    } else {
        AccountView::Failed(reason.to_string())
    }
}

/// Text of the page status line. Empty for the walls: the card says it.
pub(crate) fn account_status_text(view: &AccountView, noun: &str) -> String {
    match view {
        AccountView::Loading => format!("Loading {noun}"),
        AccountView::Failed(reason) => failure_sentence(noun, reason),
        AccountView::SignIn | AccountView::Apply | AccountView::Ready => String::new(),
    }
}

fn failure_sentence(noun: &str, reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        format!("Couldn't load {noun}.")
    } else {
        format!("Couldn't load {noun}. {reason}")
    }
}

/// Everything an account page shows until its content is ready: an always
/// rendered `role="status"` line (visible while loading, spoken only next to
/// a card), then skeleton bars, the sign-in wall, the membership wall or the
/// failure card with Retry. `noun` names the data ("your profile"), `task`
/// what the page is for ("edit your profile"), `return_path` where Log in
/// comes back to.
#[component]
pub(crate) fn AccountPending(
    view: AccountView,
    noun: String,
    task: String,
    return_path: String,
    #[props(default)] on_retry: EventHandler<()>,
) -> Element {
    let status = account_status_text(&view, &noun);
    let status_class = if view == AccountView::Loading {
        "account-wait"
    } else {
        "visually-hidden"
    };
    let login_href = crate::pages::stat_reports::login_return_href(&return_path);
    let remember = return_path.clone();
    let card = match view {
        AccountView::Loading => rsx! { SkeletonBars { rows: 4 } },
        AccountView::SignIn => rsx! {
            Wall {
                title: "Log in required",
                lead: "Log in to {task}.",
                actions: rsx! {
                    a {
                        href: "{login_href}",
                        class: btn_class(true),
                        onclick: move |_| crate::pages::remember_login_return(&remember),
                        "Log in"
                    }
                    Link { to: Route::Home {}, class: btn_class(false), "Return home" }
                },
            }
        },
        AccountView::Apply => rsx! {
            Wall {
                title: "Membership required",
                lead: "You need to be an org member to {task}.",
                actions: rsx! {
                    Link { to: Route::Apply {}, class: btn_class(true), "Apply" }
                    Link { to: Route::Home {}, class: btn_class(false), "Return home" }
                },
            }
        },
        AccountView::Failed(reason) => rsx! {
            Wall {
                title: "Couldn't load {noun}",
                lead: failure_sentence(&noun, &reason),
                actions: rsx! {
                    button {
                        r#type: "button",
                        class: btn_class(true),
                        onclick: move |_| on_retry.call(()),
                        "Retry"
                    }
                    Link { to: Route::Home {}, class: btn_class(false), "Return home" }
                },
            }
        },
        AccountView::Ready => rsx! {},
    };
    rsx! {
        p { id: "account-page-status", class: status_class, role: "status", "{status}" }
        {card}
    }
}

/// What the profile load came back with.
#[derive(Clone, Debug, PartialEq)]
enum ProfileLoad {
    Loading,
    NotMember,
    Failed(String),
    Ready,
}

fn profile_view(auth: &AuthState, load: &ProfileLoad) -> AccountView {
    if let Some(view) = account_view_from_auth(auth) {
        return view;
    }
    match load {
        ProfileLoad::Loading => AccountView::Loading,
        ProfileLoad::NotMember => AccountView::Apply,
        ProfileLoad::Failed(err) => account_view_from_error(err),
        ProfileLoad::Ready => AccountView::Ready,
    }
}

#[component]
pub fn ProfileSettings() -> Element {
    let auth = use_auth();
    let mut toast = use_toast();

    let mut member_id = use_signal(String::new);
    let mut display_name = use_signal(String::new);
    let mut bio = use_signal(String::new);
    let mut main_role = use_signal(String::new);
    let mut twitch = use_signal(String::new);
    let mut twitter = use_signal(String::new);
    let mut accounts = use_signal(Vec::<AccountEdit>::new);
    let mut load = use_signal(|| ProfileLoad::Loading);
    let mut load_refresh = use_signal(|| 0u64);
    let mut name_error = use_signal(|| None::<String>);
    let mut save_state = use_signal(SaveState::default);
    let mut account_saving = use_signal(|| false);
    // Which account row the save line speaks for, and what it says.
    let mut account_state = use_signal(|| None::<(String, SaveState)>);
    let mut sr_invalid = use_signal(|| None::<String>);
    // None until GET /api/stats/settings answers.
    let mut stats_visibility = use_signal(|| None::<StatsVisibility>);
    let mut stats_failed = use_signal(|| None::<String>);
    let mut stats_refresh = use_signal(|| 0u64);
    let mut visibility_state = use_signal(SaveState::default);

    use_resource(move || async move {
        let _generation = stats_refresh();
        // A guest gets the sign-in wall; asking the API would only 401.
        // Reading auth here reruns the fetch once the session is known.
        if !auth.read().is_logged_in() {
            return;
        }
        match ApiClient::web()
            .fetch::<MemberSettingsResponse>("/api/stats/settings")
            .await
        {
            Ok(s) => {
                stats_failed.set(None);
                stats_visibility.set(Some(s.stats_visibility));
            }
            Err(e) => stats_failed.set(Some(crate::client_error::logged_message(
                "stats settings",
                &e,
            ))),
        }
    });

    use_resource(move || async move {
        let _generation = load_refresh();
        let me = match ApiClient::web().fetch::<MeResponse>("/api/auth/me").await {
            Ok(me) => me,
            Err(e) => {
                load.set(ProfileLoad::Failed(crate::client_error::logged_message(
                    "profile me",
                    &e,
                )));
                return;
            }
        };
        let Some(m) = me.member else {
            load.set(ProfileLoad::NotMember);
            return;
        };
        member_id.set(m.id.clone());
        match ApiClient::web()
            .fetch::<ProfilePrefill>(&format!("/api/public/members/{}", m.id))
            .await
        {
            Ok(p) => {
                display_name.set(p.display_name);
                bio.set(p.bio.unwrap_or_default());
                main_role.set(p.main_role.unwrap_or_default());
                twitch.set(p.twitch.unwrap_or_default());
                twitter.set(p.twitter.unwrap_or_default());
                accounts.set(
                    p.game_accounts
                        .into_iter()
                        .map(|a| AccountEdit {
                            game_id: a.game_id,
                            account_name: a.account_name,
                            account_id: a.account_id,
                            rank: a.rank.unwrap_or_default(),
                            sr: a.sr.map(|v| v.to_string()).unwrap_or_default(),
                            role: a.role.unwrap_or_default(),
                        })
                        .collect(),
                );
                load.set(ProfileLoad::Ready);
            }
            Err(e) => load.set(ProfileLoad::Failed(crate::client_error::logged_message(
                "profile load",
                &e,
            ))),
        }
    });

    let on_save_profile = move |evt: FormEvent| {
        evt.prevent_default();
        if save_state() == SaveState::Saving {
            return;
        }
        let typed_at_submit = display_name();
        name_error.set(None);
        if typed_at_submit.trim().is_empty() {
            name_error.set(Some(NAME_REQUIRED.to_string()));
            save_state.set(SaveState::Failed(
                "Profile not saved. Enter a display name.".to_string(),
            ));
            toast.show(Toast::error(NAME_REQUIRED));
            crate::components::ui::inline_confirm::focus_element_by_id(NAME_ID);
            return;
        }
        save_state.set(SaveState::Saving);
        spawn(async move {
            let body = UpdateProfileBody {
                display_name: typed_at_submit.trim().to_string(),
                bio: none_if_empty(bio()),
                main_role: none_if_empty(main_role()),
                twitch: none_if_empty(twitch()),
                twitter: none_if_empty(twitter()),
            };
            let path = format!("/api/members/{}", member_id());
            match ApiClient::web()
                .put_json::<_, serde_json::Value>(&path, &body)
                .await
            {
                Ok(_) => {
                    save_state.set(SaveState::Saved(PROFILE_SAVED.to_string()));
                    toast.show(Toast::success(PROFILE_SAVED));
                }
                Err(e) => {
                    let settled = settle_name_field(
                        &typed_at_submit,
                        &display_name(),
                        &e,
                        DisplayNameAudience::Editor,
                    );
                    display_name.set(settled.value);
                    name_error.set(settled.error.map(str::to_string));
                    let (line, failure_toast) = profile_save_failure(&e);
                    save_state.set(line);
                    if settled.keep_existing_failure {
                        toast.show(failure_toast);
                    }
                }
            }
        });
    };

    let on_save_visibility = move |evt: FormEvent| {
        evt.prevent_default();
        if visibility_state() == SaveState::Saving {
            return;
        }
        let Some(choice) = stats_visibility() else {
            return;
        };
        visibility_state.set(SaveState::Saving);
        spawn(async move {
            // A PUT without player_name clears it, so read the stored name
            // right before saving. The in-game name is edited on My Stats.
            let stored = ApiClient::web()
                .fetch::<MemberSettingsResponse>("/api/stats/settings")
                .await;
            let result = match stored {
                Ok(current) => {
                    let body = stats_privacy::settings_body(current.player_name, choice);
                    ApiClient::web()
                        .put_json::<_, MemberSettingsResponse>("/api/stats/settings", &body)
                        .await
                }
                Err(e) => Err(e),
            };
            match result {
                Ok(saved) => {
                    stats_visibility.set(Some(saved.stats_visibility));
                    visibility_state.set(SaveState::Saved("Stats visibility saved.".to_string()));
                    toast.show(Toast::success("Stats visibility saved."));
                }
                Err(e) => {
                    let text = stats_privacy::save_error(&e);
                    visibility_state.set(SaveState::Failed(text.clone()));
                    toast.show(Toast::error(text));
                }
            }
        });
    };

    let mut on_save_account = move |idx: usize| {
        if account_saving() {
            return;
        }
        let Some(acc) = accounts.with(|a| a.get(idx).cloned()) else {
            return;
        };
        let game_id = acc.game_id.clone();
        let sr = match acc.sr.trim() {
            "" => None,
            raw => match raw.parse::<u32>() {
                Ok(v) => Some(v),
                Err(_) => {
                    sr_invalid.set(Some(game_id.clone()));
                    account_state.set(Some((
                        game_id.clone(),
                        SaveState::Failed("SR must be a number.".to_string()),
                    )));
                    toast.show(Toast::error("SR must be a number."));
                    return;
                }
            },
        };
        sr_invalid.set(None);
        account_saving.set(true);
        account_state.set(Some((game_id.clone(), SaveState::Saving)));
        spawn(async move {
            let body = UpsertAccountBody {
                game_id: acc.game_id,
                account_name: acc.account_name.trim().to_string(),
                account_id: acc.account_id,
                rank: none_if_empty(acc.rank),
                sr,
                role: none_if_empty(acc.role),
            };
            let path = format!("/api/members/{}/game-accounts", member_id());
            match ApiClient::web()
                .put_json::<_, serde_json::Value>(&path, &body)
                .await
            {
                Ok(_) => {
                    account_state.set(Some((
                        game_id,
                        SaveState::Saved("Game account saved.".to_string()),
                    )));
                    toast.show(Toast::success("Game account saved."));
                }
                Err(e) => {
                    let (line, failure_toast) = account_save_failure(&e);
                    account_state.set(Some((game_id, line)));
                    toast.show(failure_toast);
                }
            }
            account_saving.set(false);
        });
    };

    let view = profile_view(&auth(), &load());
    let saving = save_state() == SaveState::Saving;
    let name_err = name_error().unwrap_or_default();
    let name_has_error = !name_err.is_empty();
    let visibility_busy = visibility_state() == SaveState::Saving;

    // One keyed card per game account. Worked out here, so the `for` body is
    // the keyed form itself (a key inside a `{ let ..; rsx! {} }` body does
    // not key the loop).
    let account_rows: Vec<(usize, AccountEdit, AccountIds, SaveState, bool)> = accounts
        .read()
        .iter()
        .cloned()
        .enumerate()
        .map(|(idx, acc)| {
            let ids = AccountIds::new(&acc.game_id);
            let row_state = account_state()
                .filter(|(game, _)| *game == acc.game_id)
                .map(|(_, state)| state)
                .unwrap_or_default();
            let sr_bad = sr_invalid().as_deref() == Some(acc.game_id.as_str());
            (idx, acc, ids, row_state, sr_bad)
        })
        .collect();

    let header_actions = (view == AccountView::Ready).then(|| {
        rsx! {
            Link {
                to: Route::MemberProfile { id: member_id() },
                class: btn_class(false),
                "View your public profile"
            }
        }
    });

    rsx! {
        crate::styles::StyleOnce { id: ACCOUNT_CSS_ID, css: ACCOUNT_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-profile_settings-page-css", css: PAGE_CSS }

        PageShell { width: PageWidth::Narrow, class: "profile-edit-page",
            PageHeader {
                title: "Edit profile",
                lead: "This is what members and guests see on your public profile.",
                actions: header_actions,
            }

            if view != AccountView::Ready {
                AccountPending {
                    view: view.clone(),
                    noun: "your profile",
                    task: "edit your profile",
                    return_path: "/profile/edit",
                    on_retry: move |_| {
                        load.set(ProfileLoad::Loading);
                        load_refresh += 1;
                    },
                }
            } else {
                section { class: "account-section", aria_labelledby: "profile-heading",
                    h2 { id: "profile-heading", class: "ui-section-title", "Profile" }
                    form { class: "account-card", onsubmit: on_save_profile,
                        div { class: "account-field",
                            label { class: "field-label", r#for: NAME_ID, "Display name" }
                            input {
                                id: NAME_ID,
                                class: "field-input",
                                r#type: "text",
                                autocomplete: "nickname",
                                maxlength: 64,
                                value: display_name(),
                                aria_invalid: if name_has_error { "true" },
                                aria_describedby: if name_has_error { NAME_ERROR_ID },
                                oninput: move |e: FormEvent| {
                                    let (next, cleared) = name_field_after_edit(e.value());
                                    display_name.set(next);
                                    name_error.set(cleared.map(str::to_string));
                                },
                            }
                            FieldError { id: NAME_ERROR_ID, text: name_err.clone(), role: "status".to_string() }
                        }
                        div { class: "account-field",
                            label { class: "field-label", r#for: ROLE_ID, "Shown role" }
                            select {
                                id: ROLE_ID,
                                class: "field-input",
                                value: main_role(),
                                onchange: move |e| main_role.set(e.value()),
                                for (value, word) in ROLE_OPTIONS {
                                    option { key: "{word}", value: value, selected: main_role() == value, "{word}" }
                                }
                            }
                        }
                        div { class: "account-field",
                            label { class: "field-label", r#for: BIO_ID, "About" }
                            textarea {
                                id: BIO_ID,
                                class: "field-input",
                                maxlength: 500,
                                value: bio(),
                                oninput: move |e| bio.set(e.value()),
                            }
                        }
                        div { class: "account-field",
                            label { class: "field-label", r#for: TWITCH_ID, "Twitch" }
                            input {
                                id: TWITCH_ID,
                                class: "field-input",
                                r#type: "text",
                                maxlength: 32,
                                placeholder: "handle",
                                aria_describedby: TWITCH_HINT_ID,
                                value: twitch(),
                                oninput: move |e| twitch.set(e.value()),
                            }
                            p { id: TWITCH_HINT_ID, class: "field-hint", "Just the handle. No https://, no @." }
                        }
                        div { class: "account-field",
                            label { class: "field-label", r#for: TWITTER_ID, "Twitter / X" }
                            input {
                                id: TWITTER_ID,
                                class: "field-input",
                                r#type: "text",
                                maxlength: 32,
                                placeholder: "handle",
                                aria_describedby: TWITTER_HINT_ID,
                                value: twitter(),
                                oninput: move |e| twitter.set(e.value()),
                            }
                            p { id: TWITTER_HINT_ID, class: "field-hint", "Just the handle. No https://, no @." }
                        }
                        div { class: "account-actions",
                            button {
                                id: SAVE_ID,
                                r#type: "submit",
                                class: btn_class(true),
                                aria_disabled: if saving { "true" },
                                aria_describedby: SAVE_STATUS_ID,
                                "Save profile"
                            }
                            AccountStatus { id: SAVE_STATUS_ID, state: save_state() }
                        }
                    }
                }

                section { class: "account-section", aria_labelledby: "profile-stats-heading",
                    h2 { id: "profile-stats-heading", class: "ui-section-title", "Stats privacy" }
                    if let Some(choice) = stats_visibility() {
                        form { class: "account-card", onsubmit: on_save_visibility,
                            div { class: "account-field",
                                label { class: "field-label", r#for: VISIBILITY_ID, "Who can see my stats" }
                                select {
                                    id: VISIBILITY_ID,
                                    class: "field-input",
                                    aria_describedby: VISIBILITY_HINT_ID,
                                    value: choice.as_str(),
                                    onchange: move |e| {
                                        if let Some(next) = stats_privacy::parse(&e.value()) {
                                            stats_visibility.set(Some(next));
                                        }
                                    },
                                    for v in stats_privacy::OPTIONS {
                                        option { key: "{v.as_str()}", value: v.as_str(), selected: choice == v,
                                            "{stats_privacy::label(v)}"
                                        }
                                    }
                                }
                                p { id: VISIBILITY_HINT_ID, class: "field-hint", "{stats_privacy::hint(choice)}" }
                            }
                            div { class: "account-actions",
                                button {
                                    id: VISIBILITY_SAVE_ID,
                                    r#type: "submit",
                                    class: btn_class(true),
                                    aria_disabled: if visibility_busy { "true" },
                                    aria_describedby: VISIBILITY_STATUS_ID,
                                    "Save visibility"
                                }
                                AccountStatus { id: VISIBILITY_STATUS_ID, state: visibility_state() }
                            }
                        }
                    } else if let Some(reason) = stats_failed() {
                        div { class: "account-card",
                            p { class: "account-status is-error", role: "status",
                                "Couldn't load your stats visibility. {reason}"
                            }
                            div { class: "account-actions",
                                button {
                                    r#type: "button",
                                    class: btn_class(false),
                                    onclick: move |_| {
                                        stats_failed.set(None);
                                        stats_refresh += 1;
                                    },
                                    "Retry"
                                }
                            }
                        }
                    } else {
                        div { class: "account-card",
                            p { class: "account-wait", role: "status", "Loading your stats visibility" }
                        }
                    }
                }

                super::hero_icon_settings::TrainingCropSettings { member_id: member_id() }

                if !accounts.read().is_empty() {
                    section { class: "account-section", aria_labelledby: "profile-accounts-heading",
                        h2 { id: "profile-accounts-heading", class: "ui-section-title", "Game accounts" }
                        for (idx, acc, ids, row_state, sr_bad) in account_rows {
                            form {
                                key: "{acc.game_id}",
                                class: "account-card profile-account",
                                aria_labelledby: "{ids.title}",
                                onsubmit: move |evt: FormEvent| {
                                    evt.prevent_default();
                                    on_save_account(idx);
                                },
                                h3 { id: "{ids.title}", class: "account-sub-title", "{acc.account_name}" }
                                div { class: "profile-account-grid",
                                    div { class: "account-field",
                                        label { class: "field-label", r#for: "{ids.rank}", "Rank" }
                                        input {
                                            id: "{ids.rank}",
                                            class: "field-input",
                                            r#type: "text",
                                            value: "{acc.rank}",
                                            maxlength: 64,
                                            placeholder: "Diamond 2",
                                            oninput: move |e| accounts.with_mut(|a| {
                                                if let Some(row) = a.get_mut(idx) { row.rank = e.value(); }
                                            }),
                                        }
                                    }
                                    div { class: "account-field",
                                        label { class: "field-label", r#for: "{ids.sr}", "SR" }
                                        input {
                                            id: "{ids.sr}",
                                            class: "field-input",
                                            r#type: "number",
                                            value: "{acc.sr}",
                                            min: 0,
                                            aria_invalid: if sr_bad { "true" },
                                            aria_describedby: if sr_bad { "{ids.status}" },
                                            oninput: move |e| accounts.with_mut(|a| {
                                                if let Some(row) = a.get_mut(idx) { row.sr = e.value(); }
                                            }),
                                        }
                                    }
                                    div { class: "account-field",
                                        label { class: "field-label", r#for: "{ids.role}", "Role" }
                                        select {
                                            id: "{ids.role}",
                                            class: "field-input",
                                            value: "{acc.role}",
                                            onchange: move |e| accounts.with_mut(|a| {
                                                if let Some(row) = a.get_mut(idx) { row.role = e.value(); }
                                            }),
                                            for (value, word) in ROLE_OPTIONS {
                                                option { key: "{word}", value: value, selected: acc.role == value, "{word}" }
                                            }
                                        }
                                    }
                                }
                                div { class: "account-actions",
                                    button {
                                        id: "{ids.save}",
                                        r#type: "submit",
                                        class: btn_class(true),
                                        aria_disabled: if row_state == SaveState::Saving { "true" },
                                        aria_describedby: "{ids.status}",
                                        aria_label: "Save account {acc.account_name}",
                                        "Save account"
                                    }
                                    AccountStatus { id: ids.status.clone(), state: row_state.clone() }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::admin_shared::test_page::render_admin_route as render_route;
    use crate::components::member_gate::test_route::wall;
    use crate::components::ui::test_support::assert_sizes_from_tokens;
    use scuffed_types::OrgRole;

    fn auth(role: Option<OrgRole>, loading: bool) -> AuthState {
        AuthState {
            user: role.map(|role| scuffed_types::UserInfo {
                id: "u1".into(),
                username: "Player01".into(),
                avatar_url: None,
                role: Some(role),
            }),
            loading,
        }
    }

    #[test]
    fn game_account_cards_are_keyed_by_the_loop_body() {
        // Vision nit 6 on PR 706: the key sat inside a `{ let ..; rsx! {} }`
        // body, which does not key the loop.
        let src = include_str!("profile_settings.rs");
        let prod = src.split("mod tests").next().expect("production half");
        let start = prod
            .find("for (idx, acc, ids, row_state, sr_bad) in account_rows {")
            .expect("account loop");
        let head = &prod[start..start + 200];
        assert!(
            head.contains("{\n                            form {\n                                key: \"{acc.game_id}\","),
            "{head}"
        );
        assert_eq!(
            AccountIds::new("game:ow").status,
            account_field_id("game:ow", "status")
        );
    }

    #[test]
    fn a_guest_gets_the_sign_in_wall_not_a_bare_sentence() {
        let html = render_route("/profile/edit", None);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(html.contains(">Edit profile</h1>"), "{html}");
        let card = wall(&html);
        assert!(card.contains("Log in required"), "{card}");
        assert!(card.contains("Log in to edit your profile."), "{card}");
        assert!(
            card.contains("href=\"/login?return=/profile/edit\""),
            "{card}"
        );
        assert!(!html.contains("You must be logged in"), "{html}");
        assert!(html.contains("ui-shell ui-shell--narrow profile-edit-page"));
    }

    #[test]
    fn a_member_sees_a_labelled_wait_inside_the_page() {
        let html = render_route("/profile/edit", Some(OrgRole::Member));
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(
            html.contains(
                "<p id=\"account-page-status\" class=\"account-wait\" role=\"status\">Loading your profile</p>"
            ),
            "{html}"
        );
        assert!(html.contains("ui-skel-list"), "{html}");
        assert!(!html.contains("<section class=\"wall\""), "{html}");
    }

    #[test]
    fn the_view_follows_the_session_then_the_load() {
        let member = auth(Some(OrgRole::Member), false);
        assert_eq!(
            profile_view(&auth(None, false), &ProfileLoad::Ready),
            AccountView::SignIn
        );
        assert_eq!(
            profile_view(&auth(Some(OrgRole::Recruit), false), &ProfileLoad::Loading),
            AccountView::Loading,
            "a recruit has a role, so the load decides"
        );
        assert_eq!(
            profile_view(&member, &ProfileLoad::NotMember),
            AccountView::Apply
        );
        assert_eq!(
            profile_view(&member, &ProfileLoad::Failed("HTTP error 401: no".into())),
            AccountView::SignIn
        );
        assert_eq!(
            profile_view(&member, &ProfileLoad::Failed("HTTP error 500: boom".into())),
            AccountView::Failed("HTTP error 500: boom".into())
        );
        assert_eq!(
            profile_view(&auth(None, true), &ProfileLoad::Loading),
            AccountView::Loading
        );
    }

    #[test]
    fn a_failure_keeps_the_server_reason_and_offers_retry() {
        fn view() -> Element {
            rsx! {
                AccountPending {
                    view: AccountView::Failed("HTTP error 500: database offline".into()),
                    noun: "your profile",
                    task: "edit your profile",
                    return_path: "/profile/edit",
                }
            }
        }
        let html = crate::components::ui::test_support::render_routed(view);
        assert!(
            html.contains("Couldn&#39;t load your profile. HTTP error 500: database offline")
                || html.contains("Couldn't load your profile. HTTP error 500: database offline"),
            "{html}"
        );
        assert!(html.contains(">Retry</button>"), "{html}");
        assert_eq!(html.matches("role=\"status\"").count(), 1, "{html}");
    }

    #[test]
    fn the_save_line_is_always_rendered() {
        fn idle() -> Element {
            rsx! { AccountStatus { id: "x-status", state: SaveState::Idle } }
        }
        fn failed() -> Element {
            rsx! {
                AccountStatus {
                    id: "x-status",
                    state: SaveState::failed(
                        "Could not save your profile",
                        &scuffed_api_client::ClientError::http(409, r#"{"error":"taken"}"#, None),
                    ),
                }
            }
        }
        // Issue 728: the server is down. Chromium's raw error is the whole
        // wasm stack; the line says one short sentence instead.
        fn offline() -> Element {
            let raw = format!(
                "JsValue(TypeError: Failed to fetch\nTypeError: Failed to fetch\n{})",
                "    at imports.wbg.__wbg_fetch_f1 (http://127.0.0.1:3815/assets/scuffed-app-dxh1.js:1:61234)\n"
                    .repeat(12)
            );
            rsx! {
                AccountStatus {
                    id: "x-status",
                    state: SaveState::failed(
                        "Could not save your profile",
                        &scuffed_api_client::ClientError::Network(raw),
                    ),
                }
            }
        }
        let html = crate::components::ui::test_support::render(idle);
        assert_eq!(
            html,
            "<p id=\"x-status\" class=\"account-status\" role=\"status\"></p>"
        );
        let html = crate::components::ui::test_support::render(failed);
        assert!(html.contains("account-status is-error"), "{html}");
        assert!(html.contains("HTTP error 409: taken"), "{html}");

        let html = crate::components::ui::test_support::render(offline);
        assert_eq!(
            html,
            "<p id=\"x-status\" class=\"account-status is-error\" role=\"status\">Could not save your profile. Could not reach the server. Check your connection and try again.</p>"
        );
        for leak in ["JsValue", "TypeError", "Failed to fetch", "http://", ".js:"] {
            assert!(!html.contains(leak), "{leak} in {html}");
        }

        // The save handler builds the line and the toast from the error
        // itself, so neither can show the raw text.
        let prod = include_str!("profile_settings.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("prod part");
        // QA round 7 on 748: the line and the toast come from one helper
        // that maps the error itself; the pin follows it there.
        assert!(prod.contains("SaveState::failed(\"Could not save your profile\", err),"));
        assert!(prod.contains(
            "let (line, failure_toast) = profile_save_failure(&e);\n                    save_state.set(line);"
        ));
        assert!(!prod.contains("Save failed: {e}"));
    }

    /// QA round 7 on 748: a failed profile save and a failed game account
    /// save each log the raw error exactly once (the save line logs, the
    /// toast does not), and both still show the line and the toast.
    #[test]
    fn a_failed_profile_or_account_save_logs_once() {
        use crate::components::toast::ToastLevel;
        use scuffed_api_client::ClientError;
        let take = crate::util::take_logged;
        let errors = [
            ClientError::Network("JsValue(TypeError: Failed to fetch)".into()),
            ClientError::Deserialize("missing field `display_name` at line 1 column 212".into()),
            ClientError::http(409, r#"{"error":"name taken"}"#, None),
            ClientError::http(502, "", None),
        ];
        for err in &errors {
            let _ = take();
            let (line, toast) = profile_save_failure(err);
            let logged = take();
            assert_eq!(logged.len(), 1, "profile save {err:?}: {logged:?}");
            assert!(
                logged[0].starts_with("Could not save your profile: "),
                "{logged:?}"
            );
            assert!(line.is_error());
            assert!(
                line.text().starts_with("Could not save your profile. "),
                "{}",
                line.text()
            );
            assert!(toast.level == ToastLevel::Error);
            assert!(
                toast.message.starts_with("Save failed: "),
                "{}",
                toast.message
            );
            assert!(
                toast
                    .message
                    .ends_with(&crate::client_error::user_message(err)),
                "{}",
                toast.message
            );

            let (line, toast) = account_save_failure(err);
            let logged = take();
            assert_eq!(logged.len(), 1, "account save {err:?}: {logged:?}");
            assert!(
                logged[0].starts_with("Could not save the game account: "),
                "{logged:?}"
            );
            assert!(line.is_error());
            assert!(
                line.text().starts_with("Could not save the game account. "),
                "{}",
                line.text()
            );
            assert!(toast.level == ToastLevel::Error);
            assert!(
                toast.message.starts_with("Save failed: "),
                "{}",
                toast.message
            );
        }

        // Both save handlers use the helpers and add no log of their own.
        let prod = include_str!("profile_settings.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("prod part");
        assert_eq!(prod.matches("profile_save_failure(&e)").count(), 1);
        assert_eq!(prod.matches("account_save_failure(&e)").count(), 1);
        assert!(!prod.contains("Toast::error_from("), "a second log");
    }

    #[test]
    fn ids_come_from_data() {
        assert_eq!(
            account_field_id("game:overwatch", "sr"),
            "profile-account-game-overwatch-sr"
        );
        assert_ne!(account_field_id("a", "rank"), account_field_id("b", "rank"));
    }

    #[test]
    fn page_css_uses_tokens() {
        assert_sizes_from_tokens(ACCOUNT_CSS);
        // Sibling margins lose to a child's own `margin: 0` (same
        // specificity, later rule), so the card must space with `gap`.
        assert!(!ACCOUNT_CSS.contains("* + *"), "card spacing must use gap");
        for decl in [
            "padding: var(--space-section)",
            "gap: var(--space-5);",
            ".account-card > .account-check + .account-check { margin-top: calc(var(--space-3) - var(--space-5)); }",
            "gap: var(--space-2)",
            "min-height: var(--hit-target-min)",
        ] {
            assert!(ACCOUNT_CSS.contains(decl), "{decl}");
        }
        // PAGE_CSS has no type of its own; check it for units only.
        for word in PAGE_CSS.split(|c: char| c.is_whitespace() || c == ';' || c == ')') {
            assert!(!word.ends_with("rem"), "{word}");
            assert!(!word.ends_with("px") || word == "820px", "{word}");
        }
    }

    #[test]
    fn no_em_dash_and_no_stray_reports_link() {
        let src = include_str!("profile_settings.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert!(!prod.contains('\u{2014}') && !prod.contains('\u{2013}'));
        assert!(!prod.contains("Route::MyReports"));
        assert!(!prod.contains("main {"));
    }
}
