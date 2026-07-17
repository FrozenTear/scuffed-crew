use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::{Toast, use_toast};
use crate::routes::Route;
use scuffed_api_client::ApiClient;
use scuffed_types::MeResponse;

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

/// PUT /api/members/:id — plain `Option`s serialize as `null` when empty,
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

const ROLE_OPTIONS: [&str; 5] = ["", "tank", "dps", "support", "flex"];

fn none_if_empty(s: String) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

const PAGE_CSS: &str = r#"
    .profile-edit-page {
        padding: 3rem 2rem;
        max-width: 640px;
        margin: 0 auto;
    }
    .profile-edit-page h1 {
        font-family: var(--font-head);
        font-size: 1.8rem;
        color: var(--text);
        letter-spacing: 2px;
        margin: 0 0 2rem;
    }
    .profile-edit-section {
        margin-bottom: 2.5rem;
    }
    .profile-edit-section h2 {
        font-family: var(--font-head);
        font-size: 1.1rem;
        color: var(--text);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        margin: 0 0 1rem;
    }
    .profile-edit-field {
        margin-bottom: 1rem;
    }
    .profile-edit-field label {
        display: block;
        font-size: 0.75rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.05em;
        color: var(--text-3);
        margin-bottom: 0.35rem;
    }
    .profile-edit-field input,
    .profile-edit-field textarea,
    .profile-edit-field select {
        width: 100%;
        background: var(--surface-2);
        border: 1px solid var(--border);
        border-radius: 6px;
        color: var(--text);
        font-size: 0.9rem;
        padding: 0.55rem 0.7rem;
    }
    .profile-edit-field textarea {
        min-height: 90px;
        resize: vertical;
    }
    .profile-edit-hint {
        font-size: 0.72rem;
        color: var(--text-3);
        margin-top: 0.3rem;
    }
    .profile-edit-btn {
        background: var(--accent);
        color: var(--accent-fg);
        border: none;
        border-radius: 6px;
        font-weight: 700;
        font-size: 0.85rem;
        padding: 0.6rem 1.4rem;
        cursor: pointer;
    }
    .profile-edit-btn:disabled {
        opacity: 0.6;
        cursor: default;
    }
    .profile-edit-account {
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 1rem;
        margin-bottom: 1rem;
    }
    .profile-edit-account-name {
        font-family: var(--font-head);
        font-weight: 700;
        color: var(--text);
        margin-bottom: 0.75rem;
    }
    .profile-edit-account-grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
        gap: 0.75rem;
        margin-bottom: 0.75rem;
    }
    .profile-edit-status {
        color: var(--text-3);
        text-align: center;
        padding: 3rem 0;
    }
    .profile-edit-back {
        margin-top: 2rem;
        padding-top: 1.5rem;
        border-top: 1px solid var(--border);
    }
    .profile-edit-back a {
        color: var(--accent);
        text-decoration: none;
        font-size: 0.85rem;
        font-weight: 600;
    }
    .profile-edit-back a:hover {
        text-decoration: underline;
    }
"#;

#[component]
pub fn ProfileSettings() -> Element {
    let mut toast = use_toast();

    let mut member_id = use_signal(String::new);
    let mut display_name = use_signal(String::new);
    let mut bio = use_signal(String::new);
    let mut main_role = use_signal(String::new);
    let mut twitch = use_signal(String::new);
    let mut twitter = use_signal(String::new);
    let mut accounts = use_signal(Vec::<AccountEdit>::new);
    let mut loaded = use_signal(|| false);
    let mut load_error = use_signal(|| None::<String>);
    let mut saving = use_signal(|| false);

    use_future(move || async move {
        let me = match ApiClient::web().fetch::<MeResponse>("/api/auth/me").await {
            Ok(me) => me,
            Err(_) => {
                load_error.set(Some("You must be logged in to edit your profile.".into()));
                return;
            }
        };
        let Some(m) = me.member else {
            load_error.set(Some("Only org members have an editable profile.".into()));
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
                loaded.set(true);
            }
            Err(e) => load_error.set(Some(format!("Couldn't load your profile: {e}"))),
        }
    });

    let on_save_profile = move |_| {
        let name = display_name().trim().to_string();
        if name.is_empty() {
            toast.show(Toast::error("Display name is required."));
            return;
        }
        saving.set(true);
        spawn(async move {
            let body = UpdateProfileBody {
                display_name: display_name().trim().to_string(),
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
                Ok(_) => toast.show(Toast::success("Profile saved.")),
                Err(e) => toast.show(Toast::error(format!("Save failed: {e}"))),
            }
            saving.set(false);
        });
    };

    let mut on_save_account = move |idx: usize| {
        let Some(acc) = accounts.with(|a| a.get(idx).cloned()) else {
            return;
        };
        let sr = match acc.sr.trim() {
            "" => None,
            raw => match raw.parse::<u32>() {
                Ok(v) => Some(v),
                Err(_) => {
                    toast.show(Toast::error("SR must be a number."));
                    return;
                }
            },
        };
        saving.set(true);
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
                Ok(_) => toast.show(Toast::success("Game account saved.")),
                Err(e) => toast.show(Toast::error(format!("Save failed: {e}"))),
            }
            saving.set(false);
        });
    };

    rsx! {
        style { {PAGE_CSS} }

        main { class: "profile-edit-page",
            h1 { "Edit Profile" }

            if let Some(err) = load_error() {
                p { class: "profile-edit-status", "{err}" }
            } else if !loaded() {
                p { class: "profile-edit-status", "Loading..." }
            } else {
                div { class: "profile-edit-section",
                    h2 { "Profile" }
                    div { class: "profile-edit-field",
                        label { "Display name" }
                        input {
                            r#type: "text",
                            value: display_name(),
                            maxlength: 64,
                            oninput: move |e| display_name.set(e.value()),
                        }
                    }
                    div { class: "profile-edit-field",
                        label { "Bio" }
                        textarea {
                            value: bio(),
                            maxlength: 500,
                            oninput: move |e| bio.set(e.value()),
                        }
                    }
                    div { class: "profile-edit-field",
                        label { "Main role" }
                        select {
                            value: main_role(),
                            onchange: move |e| main_role.set(e.value()),
                            for r in ROLE_OPTIONS.iter() {
                                option { value: "{r}", selected: main_role() == *r,
                                    if r.is_empty() { "—" } else { "{r}" }
                                }
                            }
                        }
                    }
                    div { class: "profile-edit-field",
                        label { "Twitch" }
                        input {
                            r#type: "text",
                            value: twitch(),
                            maxlength: 32,
                            placeholder: "handle, not URL",
                            oninput: move |e| twitch.set(e.value()),
                        }
                        p { class: "profile-edit-hint", "Just the handle — no https://, no @." }
                    }
                    div { class: "profile-edit-field",
                        label { "Twitter / X" }
                        input {
                            r#type: "text",
                            value: twitter(),
                            maxlength: 32,
                            placeholder: "handle, not URL",
                            oninput: move |e| twitter.set(e.value()),
                        }
                    }
                    button {
                        class: "profile-edit-btn",
                        disabled: saving(),
                        onclick: on_save_profile,
                        "Save profile"
                    }
                }

                if !accounts.read().is_empty() {
                    div { class: "profile-edit-section",
                        h2 { "Game Accounts" }
                        for (idx, acc) in accounts.read().iter().enumerate() {
                            div { class: "profile-edit-account", key: "{acc.game_id}-{idx}",
                                div { class: "profile-edit-account-name", "{acc.account_name}" }
                                div { class: "profile-edit-account-grid",
                                    div { class: "profile-edit-field",
                                        label { "Rank" }
                                        input {
                                            r#type: "text",
                                            value: "{acc.rank}",
                                            maxlength: 64,
                                            placeholder: "e.g. Diamond 2",
                                            oninput: move |e| accounts.with_mut(|a| {
                                                if let Some(row) = a.get_mut(idx) { row.rank = e.value(); }
                                            }),
                                        }
                                    }
                                    div { class: "profile-edit-field",
                                        label { "SR" }
                                        input {
                                            r#type: "number",
                                            value: "{acc.sr}",
                                            min: 0,
                                            oninput: move |e| accounts.with_mut(|a| {
                                                if let Some(row) = a.get_mut(idx) { row.sr = e.value(); }
                                            }),
                                        }
                                    }
                                    div { class: "profile-edit-field",
                                        label { "Role" }
                                        select {
                                            value: "{acc.role}",
                                            onchange: move |e| accounts.with_mut(|a| {
                                                if let Some(row) = a.get_mut(idx) { row.role = e.value(); }
                                            }),
                                            for r in ROLE_OPTIONS.iter() {
                                                option { value: "{r}", selected: acc.role == *r,
                                                    if r.is_empty() { "—" } else { "{r}" }
                                                }
                                            }
                                        }
                                    }
                                }
                                button {
                                    class: "profile-edit-btn",
                                    disabled: saving(),
                                    onclick: move |_| on_save_account(idx),
                                    "Save account"
                                }
                            }
                        }
                    }
                }

                div { class: "profile-edit-back",
                    Link {
                        to: Route::MemberProfile { id: member_id() },
                        "View your public profile"
                    }
                }
            }
        }
    }
}
