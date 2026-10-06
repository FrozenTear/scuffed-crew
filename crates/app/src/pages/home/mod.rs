//! Public homepage — shell/skin driven composition of shared presentational blocks.

mod blocks;
mod css;
mod data;

use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::{HomeSectionId, HomeShell, HomeSkin, org_initials};

use crate::components::fetch_error;
use crate::hooks::CursorPage;
use crate::state::{loaded_site_settings, use_site_settings};
use crate::util::{FetchClass, classify_fetch};
use blocks::{
    EthosBlock, HeroBlock, ListPhase, LiveBlock, NewsBlock, RecruitBlock, TeamsBlock,
    live_panel_flags, teams_will_render,
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

/// `UseResourceState::Pending` stays pending even when the previous value is
/// still `Some(None)`, so Retry shows "Loading…" instead of the error.
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

/// Pending and error share one wrapper so the error branch can be mounted in tests.
/// Default shell and skin match a fresh install. The 128px rail note lives in `css.rs`.
fn pending_home(body: HomeBody, refresh: Signal<u32>) -> Element {
    let shell_attr = HomeShell::default().as_str();
    let skin_attr = HomeSkin::default().as_str();
    let css = home_css_layers();
    rsx! {
        style { "{css}" }
        div {
            class: "home-wrap",
            "data-home-shell": "{shell_attr}",
            "data-home-skin": "{skin_attr}",
            if body == HomeBody::Error {
                HeroSettingsError { refresh }
            } else {
                HeroSkeleton {}
            }
        }
    }
}

#[component]
pub fn Home() -> Element {
    let site_settings = use_site_settings();
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
        // shell and skin. Width numbers for this default wrapper are in css.rs.
        return pending_home(body, site_settings.refresh);
    };

    let content = settings.homepage.clone();
    let home_shell: HomeShell = settings.home_shell;
    let home_skin: HomeSkin = settings.home_skin;
    let initials = org_initials(&settings.org_name);
    let recruitment_open = settings.recruitment_open;

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

    let home_class = format!("home {}", content.content_align.css_class());
    let shell_attr = home_shell.as_str();
    let skin_attr = home_skin.as_str();
    let css = home_css_layers();
    let section_order = home_shell.section_order();
    let teams_presentation = home_shell.teams_presentation();

    // Secondary CTA: ethos when shown, otherwise squads (recruit landing).
    let secondary_href = if content.sections.ethos {
        "#ethos".to_string()
    } else {
        "#squads".to_string()
    };

    rsx! {
        style { "{css}" }
        div {
            class: "home-wrap",
            "data-home-shell": "{shell_attr}",
            "data-home-skin": "{skin_attr}",
            // Full-bleed hero sits outside the constrained body column.
            HeroBlock {
                content: content.clone(),
                initials: initials.clone(),
                recruitment_open,
                show_secondary_cta,
                secondary_href,
                metric_squads,
                metric_members,
                metric_games,
                metrics_pending,
            }
            div { class: "{home_class}",
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
            }
        }
    }
}

/// Textless hero. Bars use the loaded hero's type scale.
///
/// The parent `.home-wrap` sets `data-home-shell` so the rail width matches the
/// loaded page (`pending_home` uses the default shell). `data-home-skin` is set
/// for consistency; no skin rule affects this skeleton. Width numbers live next
/// to `.home-skel` in css.rs. `aria-busy` is omitted: it suppresses the
/// `role="status"` announcement.
#[component]
fn HeroSkeleton() -> Element {
    rsx! {
        header {
            class: "home-hero",
            span { class: "home-skel-status", role: "status", "Loading…" }
            div { class: "home-hero-rail",
                div { class: "home-hero-inner",
                    div { class: "home-skel home-skel-badge", aria_hidden: "true" }
                    div { class: "home-skel home-skel-title", aria_hidden: "true" }
                    div { class: "home-skel home-skel-title home-skel-title-short", aria_hidden: "true" }
                    div { class: "home-skel home-skel-sub", aria_hidden: "true" }
                    div { class: "home-skel-actions", aria_hidden: "true",
                        div { class: "home-skel home-skel-btn" }
                        div { class: "home-skel home-skel-btn" }
                    }
                }
            }
        }
    }
}

/// Settings failed. Same hero chrome, no template copy, retry refetches settings.
#[component]
fn HeroSettingsError(refresh: Signal<u32>) -> Element {
    rsx! {
        header { class: "home-hero",
            div { class: "home-hero-rail",
                div { class: "home-hero-inner",
                    {fetch_error(
                        "Couldn't load this page. Check your connection and try again.",
                        refresh,
                    )}
                }
            }
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
    fn pending_hero_is_a_textless_skeleton() {
        let html = render(HeroSkeleton);
        assert!(html.contains("home-hero"), "{html}");
        assert!(
            html.contains("class=\"home-skel home-skel-badge\""),
            "{html}"
        );
        assert!(html.contains("home-skel-title"), "{html}");
        assert!(!html.contains("aria-busy"), "{html}");
        assert!(html.contains("role=\"status\""), "{html}");
        assert!(html.contains("Loading…"), "{html}");
        assert!(!html.contains("aria-label"), "{html}");
        assert_no_template_copy(&html);
    }

    #[test]
    fn home_body_follows_the_settings_phase() {
        assert_eq!(home_body(FetchClass::Loading), HomeBody::Skeleton);
        assert_eq!(home_body(FetchClass::Error), HomeBody::Error);
        assert_eq!(home_body(FetchClass::Ready), HomeBody::Ready);
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
        assert!(html.contains("Loading…"), "{html}");
        assert!(!html.contains("aria-busy"), "{html}");
        assert_no_template_copy(&html);
    }

    #[test]
    fn error_home_mounts_the_error_branch_not_the_template() {
        fn view() -> Element {
            let refresh = use_signal(|| 0u32);
            pending_home(HomeBody::Error, refresh)
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
    }

    #[test]
    fn failed_settings_hero_is_an_error_not_the_template() {
        fn view() -> Element {
            let refresh = use_signal(|| 0u32);
            rsx! { HeroSettingsError { refresh } }
        }
        let html = render(view);
        assert!(
            html.contains("load this page") && html.contains("try again"),
            "{html}"
        );
        assert!(html.contains("Retry"), "{html}");
        assert_no_template_copy(&html);
    }
}
