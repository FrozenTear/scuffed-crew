use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use super::article_author::{ARTICLE_CSS, ARTICLE_CSS_ID, Load, is_retrying};
use crate::components::ui::{LoadError, PageHeader, PageShell, PageWidth};
use crate::components::{Toast, use_toast};
use crate::routes::Route;
use crate::state::{loaded_site_settings, use_site_settings};
use crate::util::pluralize;
use scuffed_api_client::ApiClient;

/// Fields this page reads from `GET /api/public/overview`.
///
/// Mirrors `PublicOverview` in `crates/site-server/src/routes/public.rs`:
/// `member_count`, `teams` and `events` (public events only). The route also
/// sends `games`, `announcements`, `settings`, `upcoming_matches`, and
/// `recent_results`. Those are ignored here. Rows are not read field by
/// field; the tiles show `teams.len()` and `events.len()`.
#[derive(Debug, Clone, Deserialize)]
struct PublicOverview {
    member_count: u64,
    #[serde(default)]
    teams: Vec<serde::de::IgnoredAny>,
    #[serde(default)]
    events: Vec<serde::de::IgnoredAny>,
}

/// The three numbers on the tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Counts {
    members: u64,
    teams: usize,
    events: usize,
}

impl From<PublicOverview> for Counts {
    fn from(overview: PublicOverview) -> Self {
        Counts {
            members: overview.member_count,
            teams: overview.teams.len(),
            events: overview.events.len(),
        }
    }
}

#[derive(Serialize)]
struct CommunityCreateBody {
    community_id: String,
    name: String,
    description: Option<String>,
    rules: Option<String>,
    image: Option<String>,
}

/// H1 until the org name is known. A missing name stays generic so the page
/// never says "My Clan".
const TITLE_FALLBACK: &str = "Community";

/// Tile value when the counts did not load.
const NOT_AVAILABLE: &str = "Not available";

/// The H1: the org's own name once settings have loaded.
fn community_title(org_name: Option<&str>) -> String {
    org_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(TITLE_FALLBACK)
        .to_string()
}

/// The lead under the title: the org's own description when it has one, then
/// the counts as a sentence once they are known.
fn community_lead(description: Option<&str>, counts: Option<Counts>) -> String {
    let description = description.map(str::trim).filter(|text| !text.is_empty());
    let numbers = counts.map(|c| {
        format!(
            "{} on the public list, {}.",
            pluralize(c.members as usize, "member", "members"),
            pluralize(c.teams, "team", "teams"),
        )
    });
    match (description, numbers) {
        (Some(text), Some(numbers)) if text.ends_with(['.', '!', '?']) => {
            format!("{text} {numbers}")
        }
        (Some(text), Some(numbers)) => format!("{text}. {numbers}"),
        (Some(text), None) => text.to_string(),
        (None, Some(numbers)) => numbers,
        (None, None) => "Meet the crew and find a way in.".to_string(),
    }
}

/// Text of the status line: the loading words during a load or a retry.
fn counts_status(counts: &Load<Counts>, retrying: bool) -> &'static str {
    if retrying || matches!(counts, Load::Loading) {
        "Loading counts"
    } else {
        ""
    }
}

/// One way into the crew: the page, its link text and one line about it.
struct WayIn {
    to: Route,
    title: &'static str,
    blurb: &'static str,
}

fn ways_in() -> [WayIn; 4] {
    [
        WayIn {
            to: Route::Members {},
            title: "Members",
            blurb: "The public roster, with roles and ranks.",
        },
        WayIn {
            to: Route::Forum {},
            title: "Forum",
            blurb: "Boards for talk, groups, and feedback.",
        },
        WayIn {
            to: Route::Feed {},
            title: "Feed",
            blurb: "Short updates from people who are signed in.",
        },
        WayIn {
            to: Route::Polls {},
            title: "Polls",
            blurb: "Votes on times, maps, and crew questions.",
        },
    ]
}

const PAGE_CSS: &str = r#"
.community-stats {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--space-3);
  margin: 0;
  min-width: 0;
}
.community-stat {
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  min-width: 0;
  padding: var(--space-card-pad);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
  text-align: center;
}
.community-stat dt {
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
}
.community-stat dd {
  margin: 0;
  color: var(--text);
  font-family: var(--font-head);
  font-size: var(--text-xl);
  font-weight: var(--font-label-weight);
  letter-spacing: var(--font-head-tracking);
  line-height: var(--leading-ui);
  font-variant-numeric: tabular-nums;
  text-box-trim: trim-both;
  text-box-edge: cap alphabetic;
}
/* Words in place of a number: body size, muted ink. */
.community-stat dd.is-missing {
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-md);
  font-weight: var(--font-body-weight);
  letter-spacing: normal;
}
.community-officer p {
  margin: 0;
  max-width: var(--content-max-narrow);
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-md);
  line-height: var(--leading-body);
}
.community-officer .btn { align-self: flex-start; }
@media (max-width: 820px) {
  .community-stats { grid-template-columns: minmax(0, 1fr); }
  .community-stat { padding: var(--space-2); }
}
"#;

#[component]
pub fn Community() -> Element {
    let mut refresh = use_signal(|| 0u32);
    let overview = use_resource(move || {
        let _ = refresh();
        async move {
            ApiClient::web()
                .fetch::<PublicOverview>("/api/public/overview")
                .await
                .map(Counts::from)
                .map_err(|e| crate::client_error::logged_message("community overview", &e))
        }
    });

    let site_settings = use_site_settings();
    let resolved = site_settings.resolved.read();
    let settings = loaded_site_settings(resolved.as_ref());
    let org_name = settings.map(|s| s.org_name.clone());
    let description = settings.map(|s| s.site_description.clone());

    let me = use_resource(|| async {
        ApiClient::web()
            .fetch::<scuffed_types::MeResponse>("/api/auth/me")
            .await
            .ok()
    });

    let me_data = me.read();
    let is_officer = me_data
        .as_ref()
        .and_then(|d| d.as_ref())
        .and_then(|m| m.member.as_ref())
        .map(|member| matches!(member.org_role.as_str(), "officer" | "admin"))
        .unwrap_or(false);

    let counts = Load::of(overview.read().as_ref());
    let retrying = is_retrying(overview.state()(), &counts);

    rsx! {
        CommunityView {
            org_name,
            description,
            counts,
            retrying,
            is_officer,
            on_retry: move |_| refresh += 1,
        }
    }
}

/// The page for one fetch state. The status line is always in the tree; only
/// its text changes.
#[component]
fn CommunityView(
    org_name: Option<String>,
    description: Option<String>,
    counts: Load<Counts>,
    /// A Retry is running.
    #[props(default = false)]
    retrying: bool,
    is_officer: bool,
    on_retry: EventHandler<()>,
) -> Element {
    let loaded = match &counts {
        Load::Ready(c) => Some(*c),
        _ => None,
    };
    let title = community_title(org_name.as_deref());
    let lead = community_lead(description.as_deref(), loaded);
    let status = counts_status(&counts, retrying);
    let failed = matches!(counts, Load::Failed(_) | Load::Missing);
    let tiles = [
        ("Members", loaded.map(|c| c.members.to_string())),
        ("Teams", loaded.map(|c| c.teams.to_string())),
        ("Open events", loaded.map(|c| c.events.to_string())),
    ];
    // The officer action publishes under the org's real name, so it waits
    // for settings.
    let publish_as = org_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string);

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-community-page-css", css: PAGE_CSS }

        PageShell { width: PageWidth::Default, class: "community-page",
            PageHeader { title, lead }

            section { class: "ui-stack", aria_label: "Crew in numbers",
                p { class: "entry-status", role: "status", "{status}" }
                dl { class: "community-stats",
                    for (label, value) in tiles {
                        div { key: "{label}", class: "community-stat",
                            dt { "{label}" }
                            {match value {
                                Some(value) => rsx! { dd { "{value}" } },
                                None if failed => rsx! { dd { class: "is-missing", {NOT_AVAILABLE} } },
                                None => rsx! { dd { class: "is-missing", "Loading" } },
                            }}
                        }
                    }
                }
                if failed {
                    LoadError {
                        message: counts.failure_line("Couldn't load the counts."),
                        on_retry: move |_| on_retry.call(()),
                        retrying,
                    }
                }
            }

            section { class: "ui-stack", aria_labelledby: "community-ways-title",
                h2 { class: "ui-section-title", id: "community-ways-title", "Find a way in" }
                div { class: "entry-list",
                    for way in ways_in() {
                        article { key: "{way.title}", class: "entry-row",
                            h2 {
                                Link { to: way.to, "{way.title}" }
                            }
                            p { "{way.blurb}" }
                        }
                    }
                }
            }

            if is_officer {
                if let Some(name) = publish_as {
                    OfficerCommunityActions { org_name: name }
                }
            }
        }
    }
}

#[component]
fn OfficerCommunityActions(org_name: String) -> Element {
    let mut toasts = use_toast();
    let mut publishing = use_signal(|| false);
    let name_for_publish = org_name.clone();

    let publish_community = move |_| {
        if publishing() {
            return;
        }
        let name = name_for_publish.clone();
        // Set before the task starts, so a second click in the same frame is
        // ignored too.
        publishing.set(true);
        spawn(async move {
            let body = CommunityCreateBody {
                community_id: "scuffed-crew".to_string(),
                name: name.clone(),
                description: Some(format!(
                    "{name}. Competitive gaming community on Nostr. Your keys, your identity."
                )),
                rules: Some(
                    "1. Be respectful\n2. No drama, no politics\n3. Age 16+\n4. Have fun"
                        .to_string(),
                ),
                image: None,
            };

            match ApiClient::web()
                .post_json::<_, serde_json::Value>("/api/nostr/community", &body)
                .await
            {
                Ok(_) => {
                    toasts.show(Toast::success("Community definition published to relay"));
                }
                Err(e) => {
                    toasts.show(Toast::error_from(
                        "Couldn't publish the community definition.",
                        &e,
                    ));
                }
            }
            publishing.set(false);
        });
    };

    let busy = publishing();

    rsx! {
        section { class: "ui-stack community-officer", aria_labelledby: "community-officer-title",
            h2 { class: "ui-section-title", id: "community-officer-title", "Officer tools" }
            p {
                "Publish the crew's name, description and house rules to the relay, so other apps can show them."
            }
            // Never natively disabled: a disabled button drops keyboard focus.
            button {
                r#type: "button",
                class: "btn is-ghost",
                aria_disabled: if busy { "true" },
                onclick: publish_community,
                if busy { "Publishing..." } else { "Publish community to relay" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    const NET: &str = crate::client_error::NETWORK_ERROR_MESSAGE;
    use super::super::article_author::test_support::render_routed;
    use super::*;
    use crate::components::ToastProvider;

    const COUNTS: Counts = Counts {
        members: 8,
        teams: 2,
        events: 0,
    };

    fn render(
        org_name: Option<&'static str>,
        counts: Load<Counts>,
        retrying: bool,
        is_officer: bool,
    ) -> String {
        let html = render_routed(move || {
            rsx! {
                ToastProvider {
                    CommunityView {
                        org_name: org_name.map(str::to_string),
                        description: Some("An EMEA Overwatch crew.".to_string()),
                        counts: counts.clone(),
                        retrying,
                        is_officer,
                        on_retry: move |_| {},
                    }
                }
            }
        });
        // The page body without the toast regions the probe adds around it.
        html.split_once("<div class=\"toast-container\"")
            .map_or(html.clone(), |(page, _)| page.to_string())
    }

    #[test]
    fn title_omits_a_name_until_settings_load() {
        assert_eq!(community_title(None), "Community");
        assert_eq!(community_title(Some("  ")), "Community");
        assert_eq!(community_title(Some("Night Owls")), "Night Owls");
        for pending in [community_title(None), community_lead(None, None)] {
            assert!(!pending.contains("My Clan"), "{pending}");
        }
    }

    #[test]
    fn lead_is_the_description_then_the_counts() {
        assert_eq!(
            community_lead(Some("An EMEA Overwatch crew."), Some(COUNTS)),
            "An EMEA Overwatch crew. 8 members on the public list, 2 teams."
        );
        assert_eq!(
            community_lead(Some(" An EMEA Overwatch crew "), Some(COUNTS)),
            "An EMEA Overwatch crew. 8 members on the public list, 2 teams."
        );
        let one = Counts {
            members: 1,
            teams: 1,
            events: 0,
        };
        assert_eq!(
            community_lead(Some("  "), Some(one)),
            "1 member on the public list, 1 team."
        );
        assert_eq!(
            community_lead(Some("An EMEA Overwatch crew."), None),
            "An EMEA Overwatch crew."
        );
        assert_eq!(
            community_lead(None, None),
            "Meet the crew and find a way in."
        );
    }

    #[test]
    fn hub_has_the_org_title_three_tiles_and_four_ways_in() {
        let html = render(Some("Test Crew"), Load::Ready(COUNTS), false, false);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Test Crew</h1>"), "{html}");
        assert!(!html.contains("<main"), "{html}");
        assert!(
            html.contains(
                "<p class=\"page-lead\">An EMEA Overwatch crew. 8 members on the public list, 2 teams.</p>"
            ),
            "{html}"
        );
        for tile in [
            "<dt>Members</dt><dd>8</dd>",
            "<dt>Teams</dt><dd>2</dd>",
            "<dt>Open events</dt><dd>0</dd>",
        ] {
            assert!(html.contains(tile), "{tile}: {html}");
        }
        assert!(
            html.contains("<p class=\"entry-status\" role=\"status\"></p>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<h2 class=\"ui-section-title\" id=\"community-ways-title\">Find a way in</h2>"
            ),
            "{html}"
        );
        assert_eq!(html.matches("class=\"entry-row\"").count(), 4, "{html}");
        for (href, title) in [
            ("/members", "Members"),
            ("/forum", "Forum"),
            ("/feed", "Feed"),
            ("/polls", "Polls"),
        ] {
            assert!(
                html.contains(&format!("href=\"{href}\""))
                    && html.contains(&format!(">{title}</a>")),
                "{href}: {html}"
            );
        }
        // Plain language: no protocol explainer, no officer tools for a guest.
        for word in ["Nostr", "NIP", "keypair", "My Clan", "Officer tools"] {
            assert!(!html.contains(word), "{word}: {html}");
        }
        assert!(!html.contains("<button"), "{html}");
    }

    #[test]
    fn loading_and_failed_counts() {
        let loading = render(None, Load::Loading, false, false);
        assert!(loading.contains("<h1>Community</h1>"), "{loading}");
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading counts</p>"),
            "{loading}"
        );
        assert_eq!(
            loading
                .matches("<dd class=\"is-missing\">Loading</dd>")
                .count(),
            3,
            "{loading}"
        );
        assert!(!loading.contains("Not available"), "{loading}");
        assert!(
            loading.contains("<p class=\"page-lead\">An EMEA Overwatch crew.</p>"),
            "{loading}"
        );

        let failed = render(
            Some("Test Crew"),
            Load::Failed(NET.to_string()),
            false,
            false,
        );
        assert_eq!(
            failed
                .matches("<dd class=\"is-missing\">Not available</dd>")
                .count(),
            3,
            "{failed}"
        );
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(
            failed.contains(
                "Couldn&#39;t load the counts. Could not reach the server. Check your connection and try again."
            ),
            "{failed}"
        );
        assert!(failed.contains(">Retry</button>"), "{failed}");
        // The ways in do not depend on the counts.
        assert_eq!(failed.matches("class=\"entry-row\"").count(), 4, "{failed}");

        let retrying = render(
            Some("Test Crew"),
            Load::Failed(NET.to_string()),
            true,
            false,
        );
        assert!(retrying.contains(">Loading counts</p>"), "{retrying}");
        assert!(retrying.contains(">Retrying...</button>"), "{retrying}");
        assert!(!retrying.contains(" disabled"), "{retrying}");

        for html in [&loading, &failed, &retrying] {
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
            assert!(!html.contains("<main"), "{html}");
            assert_eq!(
                html.matches("class=\"entry-status\" role=\"status\"")
                    .count(),
                1,
                "{html}"
            );
        }
    }

    #[test]
    fn officer_gets_a_named_publish_button_once_the_org_name_is_known() {
        let html = render(Some("Test Crew"), Load::Ready(COUNTS), false, true);
        assert!(
            html.contains(
                "<h2 class=\"ui-section-title\" id=\"community-officer-title\">Officer tools</h2>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<button type=\"button\" class=\"btn is-ghost\">Publish community to relay</button>"
            ),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");

        // No name yet: nothing to publish under.
        let pending = render(None, Load::Ready(COUNTS), false, true);
        assert!(!pending.contains("Officer tools"), "{pending}");
    }

    #[test]
    fn page_css_stacks_the_tiles_on_phone_and_uses_tokens() {
        assert!(!PAGE_CSS.contains("rem"), "fixed sizes in the page CSS");
        let phone = PAGE_CSS
            .split_once("@media (max-width: 820px) {")
            .expect("phone block")
            .1;
        assert!(
            phone.contains(".community-stats { grid-template-columns: minmax(0, 1fr); }"),
            "{phone}"
        );
    }

    fn parse_overview(json: serde_json::Value) -> PublicOverview {
        serde_json::from_value(json).expect("overview payload")
    }

    /// Hand-written sample of `PublicOverview` in
    /// `crates/site-server/src/routes/public.rs`. The app crate does not depend
    /// on the server crate, so this is not built by serializing that type.
    /// Field names match the server struct. Nested objects follow `Team` (flattened
    /// into `TeamOverview`), `TeamRecord`, `Game`, `Event`, `Announcement`,
    /// `SiteSettings` in `crates/db/src/types.rs`, and `UpcomingMatch` /
    /// `RecentResult` in `crates/types`.
    fn server_shaped_overview() -> serde_json::Value {
        serde_json::json!({
            "teams": [
                {
                    "id": "team-1",
                    "name": "Night Owls",
                    "game_id": "ow2",
                    "color": null,
                    "division": "Open",
                    "lore_quote": null,
                    "logo_url": null,
                    "is_active": true,
                    "created_at": "2026-01-15T00:00:00Z",
                    "roster_count": 5,
                    "record": { "wins": 3, "losses": 1, "draws": 0 }
                },
                {
                    "id": "team-2",
                    "name": "Day Hawks",
                    "game_id": "ow2",
                    "color": null,
                    "division": null,
                    "lore_quote": "Hold the high ground.",
                    "logo_url": null,
                    "is_active": true,
                    "created_at": "2026-02-01T00:00:00Z",
                    "roster_count": 4,
                    "record": { "wins": 0, "losses": 2, "draws": 1 }
                }
            ],
            "games": [
                {
                    "id": "ow2",
                    "name": "Overwatch",
                    "abbreviation": "OW",
                    "is_active": true,
                    "created_at": "2026-01-01T00:00:00Z"
                }
            ],
            "events": [
                {
                    "id": "event-1",
                    "title": "Scrim night",
                    "day_of_week": 2,
                    "time": "19:00",
                    "timezone": "UTC",
                    "duration_minutes": 120,
                    "is_recurring": true,
                    "team_id": "team-1",
                    "created_by": "member-1",
                    "is_active": true,
                    "is_public": true
                }
            ],
            "announcements": [
                {
                    "id": "ann-1",
                    "title": "Welcome",
                    "content": "Season starts soon.",
                    "author_id": "member-1",
                    "pinned": true,
                    "is_active": true,
                    "created_at": "2026-03-01T12:00:00Z",
                    "updated_at": "2026-03-01T12:00:00Z"
                }
            ],
            "settings": {
                "id": "settings",
                "org_name": "Night Owls",
                "site_description": "Competitive gaming community",
                "recruitment_open": true,
                "strategies_enabled": true,
                "officers_can_edit_teams": false,
                "recruitment_message": "Apply in the form.",
                "min_age": 16,
                "forum_backend": "local",
                "extra_relay_urls": "",
                "home_shell": "ops_hub",
                "home_skin": "clean",
                "public_layout": "hub",
                "homepage_json": "{}",
                "nav_json": "{}",
                "page_bg_color": "",
                "page_bg_image_url": "",
                "brand_accent_dark": "",
                "brand_accent_light": "",
                "updated_at": "2026-03-01T12:00:00Z"
            },
            "member_count": 12,
            "upcoming_matches": [
                {
                    "id": "match-1",
                    "team_id": "team-1",
                    "team_name": "Night Owls",
                    "game_name": "Overwatch",
                    "opponent": "Rivals",
                    "match_type": "official",
                    "scheduled_at": "2026-04-01T18:00:00Z"
                }
            ],
            "recent_results": [
                {
                    "id": "match-0",
                    "team_id": "team-1",
                    "team_name": "Night Owls",
                    "opponent": "Rivals",
                    "score_us": 2,
                    "score_them": 1,
                    "outcome": "win",
                    "match_type": "official",
                    "played_at": "2026-03-20T18:00:00Z"
                }
            ]
        })
    }

    #[test]
    fn server_shaped_overview_parses_member_and_team_counts() {
        let overview = parse_overview(server_shaped_overview());
        assert_eq!(overview.member_count, 12);
        assert_eq!(overview.teams.len(), 2);
        assert_eq!(
            Counts::from(overview),
            Counts {
                members: 12,
                teams: 2,
                events: 1,
            }
        );
    }

    #[test]
    fn empty_teams_list_counts_as_zero() {
        let mut payload = server_shaped_overview();
        payload["teams"] = serde_json::json!([]);
        payload["member_count"] = serde_json::json!(4);
        let overview = parse_overview(payload);
        assert_eq!(overview.member_count, 4);
        assert_eq!(overview.teams.len(), 0);
    }

    #[test]
    fn missing_teams_field_defaults_to_zero() {
        let mut payload = server_shaped_overview();
        payload.as_object_mut().expect("object").remove("teams");
        let overview = parse_overview(payload);
        assert_eq!(overview.member_count, 12);
        assert_eq!(overview.teams.len(), 0);
    }

    #[test]
    fn missing_events_field_counts_as_zero_open_events() {
        let mut payload = server_shaped_overview();
        payload.as_object_mut().expect("object").remove("events");
        assert_eq!(Counts::from(parse_overview(payload)).events, 0);
    }
}
