//! Public homepage: shell/skin driven composition of shared presentational blocks.

mod blocks;
mod css;
mod data;

use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::{ContentAlign, HomeSectionId, HomeShell, HomeSkin};

use crate::components::fetch_error;
use crate::components::ui::{PageShell, PageWidth};
use crate::hooks::CursorPage;
use crate::state::{loaded_site_settings, use_site_settings};
use crate::util::{FetchClass, classify_fetch};
use blocks::{
    EthosBlock, HeroBlock, ListPhase, LiveBlock, NewsBlock, RecruitBlock, StatsBlock, TeamsBlock,
    hero_class, live_panel_flags, teams_will_render,
};
use css::home_css_layers;
use data::{Announcement, Event, HomeTournament, Overview};

/// What `Home` paints from the settings fetch. Pending is a textless skeleton,
/// not the template homepage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HomeBody {
    Skeleton,
    Error,
    Ready,
}

fn home_body(phase: FetchClass) -> HomeBody {
    match phase {
        FetchClass::Loading => HomeBody::Skeleton,
        FetchClass::Error => HomeBody::Error,
        FetchClass::Ready => HomeBody::Ready,
    }
}

/// What the page knows about the viewer's membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewerMembership {
    /// The session check has not answered yet.
    Unknown,
    Member,
    /// A guest, or an account with no member row.
    NotMember,
}

impl ViewerMembership {
    /// No auth context (a bare mount) is a settled guest.
    fn of(auth: Option<&crate::state::AuthState>) -> Self {
        match auth {
            Some(auth) if auth.loading => ViewerMembership::Unknown,
            Some(auth) if auth.is_org_member() => ViewerMembership::Member,
            _ => ViewerMembership::NotMember,
        }
    }
}

/// `UseResourceState::Pending` stays pending even when the previous value is
/// still `Some(None)`, so a Retry reads as loading and not as the old error.
/// Otherwise `None` is still in flight, `Some(None)` failed, and `Some(Some)` settled.
fn resource_list_phase<T>(state: UseResourceState, slot: Option<&Option<T>>) -> ListPhase {
    if state == UseResourceState::Pending {
        return ListPhase::Pending;
    }
    match slot {
        None => ListPhase::Pending,
        Some(None) => ListPhase::Failed,
        Some(Some(_)) => ListPhase::Ready,
    }
}

/// Upcoming fixtures stay visible with Schedule off when the list has rows.
/// The empty panel is kept while loading, and after that only when the shell
/// keeps empty Live sections and Schedule itself is on.
fn show_next_match_panel(
    phase: ListPhase,
    has_upcoming: bool,
    live_keep_empty: bool,
    schedule_on: bool,
) -> bool {
    blocks::keep_section(phase, has_upcoming, live_keep_empty && schedule_on)
}

/// Page class on the `PageShell`. The admin alignment setting is not a page
/// class: only the hero follows it, through `hero_class`.
const PAGE_CLASS: &str = "home-page";

/// Text of the page status line: only the settings wait has something to say.
fn home_status(body: HomeBody) -> &'static str {
    match body {
        HomeBody::Skeleton => "Loading the home page",
        HomeBody::Error | HomeBody::Ready => "",
    }
}

/// The wrapper every state of Home renders through: the shell and skin
/// attributes, the page container and the status line. One template for the
/// skeleton, the error and the loaded page, so the status line is the same
/// node in all three and its text change is announced.
fn home_frame(shell: HomeShell, skin: HomeSkin, state: HomeBody, body: Element) -> Element {
    let shell_attr = shell.as_str();
    let skin_attr = skin.as_str();
    let status = home_status(state);
    let css = home_css_layers();
    rsx! {
        crate::styles::StyleLive { id: "scuffed-css-home", css }
        div {
            class: "home-wrap",
            "data-home-shell": "{shell_attr}",
            "data-home-skin": "{skin_attr}",
            p { class: "visually-hidden", role: "status", "{status}" }
            PageShell { width: PageWidth::Default, class: PAGE_CLASS, {body} }
        }
    }
}

/// Pending and error share one wrapper so the error branch can be mounted in tests.
/// Default shell, skin and alignment match a fresh install.
/// `reason` is the settings fetch's mapped error, said under the heading.
fn pending_home(body: HomeBody, reason: &str, refresh: Signal<u32>) -> Element {
    let inner = if body == HomeBody::Error {
        let reason = reason.to_string();
        rsx! { HeroSettingsError { reason, refresh } }
    } else {
        rsx! { HeroSkeleton {} }
    };
    home_frame(HomeShell::default(), HomeSkin::default(), body, inner)
}

/// Whether the page offers its apply actions (the hero's first button and the
/// recruit block). An org member has nothing to apply for: /apply only tells
/// them they are already in.
///
/// While the session check runs the viewer is unknown, and unknown is not
/// "guest": the actions stay out until the answer is in. Showing them first
/// and taking them away from a member made the page jump on every load (the
/// Recruit block is the first section of the recruit_landing shell). The rest
/// of the page does not wait for the session.
fn offers_apply(recruitment_open: bool, viewer: ViewerMembership) -> bool {
    recruitment_open && viewer == ViewerMembership::NotMember
}

#[component]
pub fn Home() -> Element {
    let site_settings = use_site_settings();
    let viewer = {
        let auth = try_use_context::<Signal<crate::state::AuthState>>();
        let auth = auth.as_ref().map(|auth| auth.read());
        ViewerMembership::of(auth.as_deref())
    };
    // Refresh ticks are read in the sync part of each resource, not inside the
    // future, so Retry restarts that fetch without subscribing to its result.
    let overview_refresh = use_signal(|| 0u32);
    let overview = use_resource(move || {
        let _tick = overview_refresh();
        async move {
            ApiClient::web()
                .fetch::<Overview>("/api/public/overview")
                .await
                .ok()
        }
    });
    let announcements_refresh = use_signal(|| 0u32);
    let announcements = use_resource(move || {
        let _tick = announcements_refresh();
        async move {
            ApiClient::web()
                .fetch::<CursorPage<Announcement>>("/api/announcements")
                .await
                .ok()
                .map(|r| r.data)
        }
    });
    let tournaments_refresh = use_signal(|| 0u32);
    let tournaments_res = use_resource(move || {
        let _tick = tournaments_refresh();
        async move {
            ApiClient::web()
                .fetch::<CursorPage<HomeTournament>>("/api/tournaments")
                .await
                .ok()
                .map(|r| r.data)
        }
    });
    let events_refresh = use_signal(|| 0u32);
    let events = use_resource(move || {
        let _tick = events_refresh();
        async move {
            ApiClient::web()
                .fetch::<CursorPage<Event>>("/api/events")
                .await
                .ok()
                .map(|r| r.data)
        }
    });
    let resolved = site_settings.resolved.read();
    let settings_fetch = classify_fetch(resolved.as_ref());
    let loaded = loaded_site_settings(resolved.as_ref());

    // Default homepage copy is a template for new installs. Painting it before
    // settings arrive flashes that template, then swaps in the real org.
    // Pending stays a textless skeleton. A failed load stays an error.
    let body = home_body(settings_fetch);
    let Some(settings) = loaded.filter(|_| body == HomeBody::Ready) else {
        // A parsed `#sc-settings` block is Ready above and uses that org's
        // shell and skin.
        let reason = match resolved.as_ref() {
            Some(Err(reason)) => reason.clone(),
            _ => String::new(),
        };
        return pending_home(body, &reason, site_settings.refresh);
    };

    let content = crate::copy::plain_homepage(&settings.homepage);
    let home_shell: HomeShell = settings.home_shell;
    let home_skin: HomeSkin = settings.home_skin;
    let recruitment_open = offers_apply(settings.recruitment_open, viewer);

    // Resolve list data for blocks (Home owns resources).
    // Pending keeps a section mounted (loading / skeleton) even when the shell
    // hides empty lists, so the page does not grow when the fetch lands.
    let events_phase = resource_list_phase(events.state()(), events.read().as_ref());
    let event_list = events
        .read()
        .as_ref()
        .and_then(|e| e.as_ref())
        .cloned()
        .unwrap_or_default();
    let tourneys_phase =
        resource_list_phase(tournaments_res.state()(), tournaments_res.read().as_ref());
    let tourney_list = tournaments_res
        .read()
        .as_ref()
        .and_then(|t| t.as_ref())
        .cloned()
        .unwrap_or_default();
    let live_tournaments: Vec<HomeTournament> = tourney_list
        .iter()
        .filter(|t| t.status == "registration" || t.status == "in_progress")
        .take(5)
        .cloned()
        .collect();
    let news_phase = resource_list_phase(announcements.state()(), announcements.read().as_ref());
    let news_list = announcements
        .read()
        .as_ref()
        .and_then(|a| a.as_ref())
        .cloned()
        .unwrap_or_default();
    let upcoming_phase = resource_list_phase(overview.state()(), overview.read().as_ref());
    let overview_data = overview.read().as_ref().and_then(|o| o.as_ref()).cloned();

    let upcoming_matches = overview_data
        .as_ref()
        .map(|o| o.upcoming_matches.clone())
        .unwrap_or_default();
    let recent_results = overview_data
        .as_ref()
        .map(|o| o.recent_results.clone())
        .unwrap_or_default();

    let has_events = !event_list.is_empty();
    let has_tourneys = !live_tournaments.is_empty();
    let has_upcoming = !upcoming_matches.is_empty();
    let has_results = !recent_results.is_empty();
    let teams_empty = overview_data
        .as_ref()
        .map(|o| o.teams.is_empty())
        .unwrap_or(true);

    let live_keep_empty = home_shell.show_when_empty(HomeSectionId::Live);
    let (show_schedule, show_tourneys) = live_panel_flags(
        live_keep_empty,
        content.sections.schedule,
        content.sections.tournaments,
        events_phase,
        has_events,
        tourneys_phase,
        has_tourneys,
    );
    let show_next_match = show_next_match_panel(
        upcoming_phase,
        has_upcoming,
        live_keep_empty,
        content.sections.schedule,
    );
    // The ticker is reserved while overview is in flight, then hidden when the
    // list is empty or the fetch failed.
    let show_results = blocks::keep_section(upcoming_phase, has_results, false);

    let show_teams = teams_will_render(
        content.sections.teams,
        upcoming_phase,
        teams_empty,
        home_shell.show_when_empty(HomeSectionId::Teams),
    );
    // Secondary CTA only when Teams block will render, including while that
    // list is still loading so the button does not pop in later.
    let show_secondary_cta = show_teams;

    let show_news = content.sections.news
        && blocks::keep_section(
            news_phase,
            !news_list.is_empty(),
            home_shell.show_when_empty(HomeSectionId::News),
        );
    let metrics_pending = upcoming_phase == ListPhase::Pending;

    let (metric_squads, metric_members, metric_games) = {
        match overview_data.as_ref() {
            Some(data) => {
                let with_roster = data.teams.iter().filter(|t| t.roster_count > 0).count();
                let squads = if with_roster > 0 {
                    Some(with_roster)
                } else {
                    None
                };
                let members = (data.member_count > 0).then_some(data.member_count);
                let games = (!data.games.is_empty()).then_some(data.games.len());
                (squads, members, games)
            }
            None => (None, None, None),
        }
    };

    let show_stats = metrics_pending || metric_squads.or(metric_members).or(metric_games).is_some();
    let section_order = home_shell.section_order();
    let teams_presentation = home_shell.teams_presentation();

    // Secondary CTA: ethos when shown, otherwise squads (recruit landing).
    let secondary_href = if content.sections.ethos {
        "#ethos".to_string()
    } else {
        "#squads".to_string()
    };

    let body = rsx! {
        HeroBlock {
            content: content.clone(),
            recruitment_open,
            show_secondary_cta,
            secondary_href,
        }
        // Not mounted when empty: an empty flex child would still
        // take a `--space-section` gap.
        if show_stats {
            StatsBlock {
                metric_squads,
                metric_members,
                metric_games,
                metrics_pending,
            }
        }
        for id in section_order.iter().copied() {
            {
                match id {
                    HomeSectionId::Ethos if content.sections.ethos => rsx! {
                        EthosBlock { content: content.clone() }
                    },
                    HomeSectionId::Live
                        if show_schedule || show_tourneys || show_next_match || show_results =>
                    {
                        rsx! {
                            LiveBlock {
                                content: content.clone(),
                                events: event_list.clone(),
                                events_phase,
                                live_tournaments: live_tournaments.clone(),
                                tourneys_phase,
                                upcoming_matches: upcoming_matches.clone(),
                                upcoming_phase,
                                recent_results: recent_results.clone(),
                                show_schedule,
                                show_tourneys,
                                show_next_match,
                                show_results,
                                retry_events: events_refresh,
                                retry_tourneys: tournaments_refresh,
                                retry_overview: overview_refresh,
                            }
                        }
                    },
                    HomeSectionId::Teams if show_teams => rsx! {
                        TeamsBlock {
                            content: content.clone(),
                            overview: overview_data.clone(),
                            phase: upcoming_phase,
                            presentation: teams_presentation,
                            retry: overview_refresh,
                        }
                    },
                    HomeSectionId::News if show_news => rsx! {
                        NewsBlock {
                            content: content.clone(),
                            announcements: news_list.clone(),
                            phase: news_phase,
                            retry: announcements_refresh,
                        }
                    },
                    HomeSectionId::Recruit if content.sections.recruit && recruitment_open => rsx! {
                        RecruitBlock { content: content.clone() }
                    },
                    _ => rsx! {},
                }
            }
        }

        if !content.footer_note.trim().is_empty() {
            p { class: "home-foot", "{content.footer_note}" }
        }
    };
    home_frame(home_shell, home_skin, HomeBody::Ready, body)
}

/// Textless hero in the loaded hero's layout: banner, title, lead, actions.
///
/// `pending_home` sets the default shell, skin and alignment on the wrappers
/// so the skeleton matches a fresh install. The page has no title to show
/// yet, so its one H1 is the page's name, for assistive tech only. The
/// loading words are the page status line in `home_frame`. `aria-busy` is
/// omitted: it suppresses the `role="status"` announcement.
#[component]
fn HeroSkeleton() -> Element {
    let class = hero_class(ContentAlign::default());
    rsx! {
        section { class: "{class}",
            h1 { class: "visually-hidden", "Home" }
            div { class: "home-skel home-skel-badge", aria_hidden: "true" }
            div { class: "home-skel home-skel-title", aria_hidden: "true" }
            div { class: "home-skel home-skel-sub", aria_hidden: "true" }
            div { class: "home-skel-actions", aria_hidden: "true",
                div { class: "home-skel home-skel-btn" }
                div { class: "home-skel home-skel-btn" }
            }
        }
    }
}

/// Settings failed. Same hero box, no template copy, retry refetches settings.
/// The failure is the page's H1, so the page still has a heading.
#[component]
fn HeroSettingsError(
    /// The settings fetch's mapped error (QA on 748: not a fixed
    /// "check your connection" for every failure).
    #[props(default)]
    reason: String,
    refresh: Signal<u32>,
) -> Element {
    let message = crate::client_error::reason_or_fallback(&reason);
    let class = hero_class(ContentAlign::default());
    rsx! {
        section { class: "{class}",
            h1 { class: "home-hero-title", "Couldn't load this page" }
            {fetch_error(&message, refresh)}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(root: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn assert_no_template_copy(html: &str) {
        let template = scuffed_types::HomepageContent::default();
        for needle in [
            template.hero_badge.as_str(),
            template.hero_title.as_str(),
            template.hero_title_accent.as_str(),
            template.hero_sub.as_str(),
            template.ethos_title.as_str(),
            template.cta_primary.as_str(),
        ] {
            assert!(!html.contains(needle), "{needle} in {html}");
        }
    }

    #[test]
    fn apply_actions_are_for_non_members_while_recruitment_is_open() {
        use ViewerMembership::{Member, NotMember, Unknown};
        assert!(offers_apply(true, NotMember));
        assert!(
            !offers_apply(true, Member),
            "a member is not asked to apply"
        );
        // Still checking the session: unknown is not treated as a guest.
        assert!(!offers_apply(true, Unknown));
        for viewer in [NotMember, Member, Unknown] {
            assert!(!offers_apply(false, viewer));
        }
        // No auth context is a settled guest; a loading one is unknown.
        assert_eq!(ViewerMembership::of(None), NotMember);
        assert_eq!(
            ViewerMembership::of(Some(&crate::state::AuthState::new())),
            Unknown
        );
        assert_eq!(ViewerMembership::of(Some(&viewer(None))), NotMember);
        assert_eq!(
            ViewerMembership::of(Some(&viewer(Some(scuffed_types::OrgRole::Recruit)))),
            Member
        );
    }

    #[test]
    fn pending_hero_is_a_textless_skeleton() {
        let html = render(HeroSkeleton);
        assert!(html.contains("home-hero"), "{html}");
        assert!(
            html.contains("class=\"home-skel home-skel-badge\""),
            "{html}"
        );
        assert!(html.contains("home-skel-title"), "{html}");
        assert!(!html.contains("aria-busy"), "{html}");
        assert!(!html.contains("aria-label"), "{html}");
        // One H1 even with no title to show: the page's name, not drawn.
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(
            html.contains("<h1 class=\"visually-hidden\">Home</h1>"),
            "{html}"
        );
        // The loading words are the page status line, not part of the hero.
        assert!(!html.contains("role=\"status\""), "{html}");
        assert_no_template_copy(&html);
    }

    #[test]
    fn status_line_is_one_node_in_every_state_and_only_its_text_changes() {
        assert_eq!(home_status(HomeBody::Skeleton), "Loading the home page");
        assert_eq!(home_status(HomeBody::Error), "");
        assert_eq!(home_status(HomeBody::Ready), "");
        fn loading() -> Element {
            let refresh = use_signal(|| 0u32);
            pending_home(HomeBody::Skeleton, "", refresh)
        }
        fn failed() -> Element {
            let refresh = use_signal(|| 0u32);
            pending_home(
                HomeBody::Error,
                crate::client_error::NETWORK_ERROR_MESSAGE,
                refresh,
            )
        }
        fn ready() -> Element {
            home_frame(
                HomeShell::default(),
                HomeSkin::default(),
                HomeBody::Ready,
                rsx! { h1 { "Loaded" } },
            )
        }
        let line = |text: &str| {
            format!(
                "data-home-skin=\"clean\"><p class=\"visually-hidden\" role=\"status\">{text}</p><div class=\"ui-shell home-page\">"
            )
        };
        let loading = render(loading);
        assert!(
            loading.contains(&line("Loading the home page")),
            "{loading}"
        );
        let failed = render(failed);
        assert!(failed.contains(&line("")), "{failed}");
        let ready = render(ready);
        assert!(ready.contains(&line("")), "{ready}");
        // Every state has exactly one H1.
        for html in [&loading, &failed, &ready] {
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
        }
        assert!(
            failed.contains("<h1 class=\"home-hero-title\">Couldn&#39;t load this page</h1>"),
            "{failed}"
        );
    }

    #[test]
    fn home_body_follows_the_settings_phase() {
        assert_eq!(home_body(FetchClass::Loading), HomeBody::Skeleton);
        assert_eq!(home_body(FetchClass::Error), HomeBody::Error);
        assert_eq!(home_body(FetchClass::Ready), HomeBody::Ready);
    }

    /// Home with settings already loaded (recruitment open, default copy)
    /// for the given viewer.
    fn render_home_for(auth: crate::state::AuthState) -> String {
        const SETTINGS: &str = r#"{"id":"site","org_name":"Test Crew","site_description":"desc","recruitment_open":true,"recruitment_message":"msg","min_age":16,"forum_backend":"local","extra_relay_urls":"","updated_at":"2026-10-06T00:00:00Z"}"#;
        crate::state::set_test_embedded_json(Some(SETTINGS.to_string()));
        let html = crate::components::ui::test_support::render_routed(move || {
            use_context_provider(|| Signal::new(auth.clone()));
            crate::state::provide_site_settings();
            rsx! { Home {} }
        });
        crate::state::set_test_embedded_json(None);
        html
    }

    fn viewer(role: Option<scuffed_types::OrgRole>) -> crate::state::AuthState {
        crate::state::AuthState {
            user: Some(scuffed_types::UserInfo {
                id: "user:1".into(),
                username: "Player01".into(),
                avatar_url: None,
                role,
            }),
            loading: false,
        }
    }

    #[test]
    fn a_member_gets_no_apply_actions_and_a_guest_gets_both() {
        let cta = scuffed_types::HomepageContent::default().cta_primary;
        let apply_link = "href=\"/apply\"";
        let recruit = "<section id=\"recruit\"";

        let guest = render_home_for(crate::state::AuthState {
            user: None,
            loading: false,
        });
        assert!(guest.contains(apply_link), "{guest}");
        assert!(guest.contains(&format!(">{cta}</a>")), "{guest}");
        assert!(guest.contains(recruit), "{guest}");
        // Logged in with no membership can still apply.
        let visitor = render_home_for(viewer(None));
        assert!(visitor.contains(apply_link), "{visitor}");
        assert!(visitor.contains(recruit), "{visitor}");

        for role in [
            scuffed_types::OrgRole::Recruit,
            scuffed_types::OrgRole::Member,
            scuffed_types::OrgRole::Admin,
        ] {
            let member = render_home_for(viewer(Some(role)));
            // The loaded page, not the skeleton.
            assert!(member.contains("class=\"home-hero-title\""), "{member}");
            assert!(!member.contains("home-skel-title"), "{member}");
            assert!(!member.contains(apply_link), "{role:?}: {member}");
            assert!(!member.contains(recruit), "{role:?}: {member}");
            assert_eq!(member.matches("<h1").count(), 1, "{member}");
        }
    }

    #[test]
    fn apply_actions_stay_out_while_the_viewer_is_unknown() {
        // Session check still running. The page is painted (it does not wait
        // for the session), but the apply actions are not: they only appear
        // once the viewer is known not to be a member, so they are never
        // shown to a member and then taken away.
        let html = render_home_for(crate::state::AuthState::new());
        assert!(html.contains("class=\"home-hero-title\""), "{html}");
        assert!(!html.contains("home-skel-title"), "{html}");
        assert!(!html.contains("href=\"/apply\""), "{html}");
        assert!(!html.contains("id=\"recruit\""), "{html}");
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
    }

    #[test]
    fn loading_home_mounts_without_template_copy() {
        fn view() -> Element {
            crate::state::provide_site_settings();
            rsx! { Home {} }
        }
        let html = render(view);
        assert!(
            html.contains(
                "class=\"home-wrap\" data-home-shell=\"ops_hub\" data-home-skin=\"clean\""
            ),
            "{html}"
        );
        assert!(
            html.contains("class=\"home-skel home-skel-badge\""),
            "{html}"
        );
        assert!(html.contains("Loading the home page"), "{html}");
        assert!(!html.contains("aria-busy"), "{html}");
        assert_no_template_copy(&html);
    }

    #[test]
    fn error_home_mounts_the_error_branch_not_the_template() {
        fn view() -> Element {
            let refresh = use_signal(|| 0u32);
            pending_home(
                HomeBody::Error,
                crate::client_error::NETWORK_ERROR_MESSAGE,
                refresh,
            )
        }
        let html = render(view);
        assert!(
            html.contains(
                "class=\"home-wrap\" data-home-shell=\"ops_hub\" data-home-skin=\"clean\""
            ),
            "{html}"
        );
        assert!(
            html.contains("load this page") && html.contains("try again"),
            "{html}"
        );
        assert!(html.contains("Retry"), "{html}");
        assert!(
            !html.contains("class=\"home-skel home-skel-badge\""),
            "{html}"
        );
        assert_no_template_copy(&html);
    }

    #[test]
    fn pending_and_error_bodies_sit_in_the_default_width_shell() {
        fn loading() -> Element {
            let refresh = use_signal(|| 0u32);
            pending_home(HomeBody::Skeleton, "", refresh)
        }
        fn failed() -> Element {
            let refresh = use_signal(|| 0u32);
            pending_home(
                HomeBody::Error,
                crate::client_error::NETWORK_ERROR_MESSAGE,
                refresh,
            )
        }
        for html in [render(loading), render(failed)] {
            // The shell owns `<main>`; the page body is a default-width div.
            assert!(
                html.contains("<div class=\"ui-shell home-page\">"),
                "{html}"
            );
            assert!(html.contains("<section class=\"home-hero\">"), "{html}");
            assert!(!html.contains("<main"), "{html}");
            assert!(!html.contains("<header"), "{html}");
            assert!(!html.contains("ui-shell--"), "{html}");
        }
    }

    #[test]
    fn alignment_is_a_hero_class_not_a_page_class() {
        // No CSS reads `align-left` or `align-center`; the setting reaches
        // the page only as the hero's `is-centered`.
        assert_eq!(PAGE_CLASS, "home-page");
        assert_eq!(hero_class(ContentAlign::Left), "home-hero");
        assert_eq!(hero_class(ContentAlign::Center), "home-hero is-centered");
        let css = home_css_layers();
        assert!(
            css.contains(".home-hero.is-centered"),
            "the hero rule exists"
        );
        for dead in [".align-left", ".align-center"] {
            assert!(!css.contains(dead), "{dead}");
        }
    }

    #[test]
    fn list_phase_distinguishes_pending_failure_and_ready() {
        assert_eq!(
            resource_list_phase(UseResourceState::Pending, None::<&Option<()>>),
            ListPhase::Pending
        );
        assert_eq!(
            resource_list_phase(UseResourceState::Pending, Some(&None::<()>)),
            ListPhase::Pending
        );
        assert_eq!(
            resource_list_phase(UseResourceState::Ready, Some(&None::<()>)),
            ListPhase::Failed
        );
        assert_eq!(
            resource_list_phase(UseResourceState::Ready, Some(&Some(()))),
            ListPhase::Ready
        );
    }

    #[test]
    fn next_match_stays_visible_when_schedule_is_off() {
        let lean = scuffed_types::HomepageSections::lean();
        assert!(!lean.schedule);
        assert!(show_next_match_panel(
            ListPhase::Ready,
            true,
            false,
            lean.schedule
        ));
        assert!(show_next_match_panel(
            ListPhase::Pending,
            false,
            false,
            lean.schedule
        ));
        assert!(!show_next_match_panel(
            ListPhase::Ready,
            false,
            false,
            lean.schedule
        ));
        assert!(!show_next_match_panel(
            ListPhase::Failed,
            false,
            false,
            lean.schedule
        ));
        assert!(show_next_match_panel(ListPhase::Ready, false, true, true));
        // Ready, empty, shell keeps empty Live sections, Schedule off.
        // Dropping `&& schedule_on` makes this true.
        assert!(!show_next_match_panel(ListPhase::Ready, false, true, false));
    }

    #[test]
    fn failed_settings_hero_is_an_error_not_the_template() {
        fn view() -> Element {
            let refresh = use_signal(|| 0u32);
            rsx! {
                HeroSettingsError {
                    reason: crate::client_error::NETWORK_ERROR_MESSAGE.to_string(),
                    refresh,
                }
            }
        }
        let html = render(view);
        assert!(
            html.contains("load this page") && html.contains("try again"),
            "{html}"
        );
        assert!(html.contains("Retry"), "{html}");
        assert_no_template_copy(&html);
    }

    #[test]
    fn home_normalises_settings_copy_before_blocks() {
        let src = include_str!("mod.rs");
        let needle = ["plain_homepage", "(&settings.homepage)"].concat();
        assert!(src.matches(needle.as_str()).count() == 1);
    }
}
