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
    EthosBlock, HeroBlock, LiveBlock, NewsBlock, RecruitBlock, TeamsBlock, live_panel_flags,
    teams_will_render,
};
use css::home_css_layers;
use data::{Announcement, Event, HomeTournament, Overview};

#[component]
pub fn Home() -> Element {
    let site_settings = use_site_settings();
    let overview = use_resource(|| async {
        ApiClient::web()
            .fetch::<Overview>("/api/public/overview")
            .await
            .ok()
    });
    let announcements = use_resource(|| async {
        ApiClient::web()
            .fetch::<CursorPage<Announcement>>("/api/announcements")
            .await
            .ok()
            .map(|r| r.data)
    });
    let tournaments_res = use_resource(|| async {
        ApiClient::web()
            .fetch::<CursorPage<HomeTournament>>("/api/tournaments")
            .await
            .ok()
            .map(|r| r.data)
    });
    let events = use_resource(|| async {
        ApiClient::web()
            .fetch::<CursorPage<Event>>("/api/events")
            .await
            .ok()
            .map(|r| r.data)
    });

    let (settings_fetch, loaded) = {
        let slot = site_settings.resource.read();
        (
            classify_fetch(slot.as_ref()),
            loaded_site_settings(slot.as_ref()).cloned(),
        )
    };

    // Default homepage copy ("Your Clan", "Gaming clan", …) is a template for
    // new installs. Painting it before settings arrive flashes that template,
    // then swaps in the real org. Pending stays a textless skeleton. A failed
    // load stays an error, not the template.
    let Some(settings) = loaded else {
        let css = home_css_layers();
        return rsx! {
            style { "{css}" }
            div { class: "home-wrap",
                if settings_fetch == FetchClass::Error {
                    HeroSettingsError { refresh: site_settings.refresh }
                } else {
                    HeroSkeleton {}
                }
            }
        };
    };

    let content = settings.homepage.clone();
    let home_shell: HomeShell = settings.home_shell;
    let home_skin: HomeSkin = settings.home_skin;
    // `org_initials` of an empty name is "CL", which is still a fake mark.
    let initials = org_initials(&settings.org_name);
    let recruitment_open = settings.recruitment_open;

    // Resolve list data for blocks (Home owns resources).
    let event_list = events
        .read()
        .as_ref()
        .and_then(|e| e.as_ref())
        .cloned()
        .unwrap_or_default();
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
    let news_list = announcements
        .read()
        .as_ref()
        .and_then(|a| a.as_ref())
        .cloned()
        .unwrap_or_default();
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

    let (show_schedule, show_tourneys) = live_panel_flags(
        home_shell.show_when_empty(HomeSectionId::Live),
        content.sections.schedule,
        content.sections.tournaments,
        has_events,
        has_tourneys,
    );
    // Match widgets show when we have data (or shell keeps empty Live sections).
    let live_keep_empty = home_shell.show_when_empty(HomeSectionId::Live);
    let show_next_match = has_upcoming || (live_keep_empty && content.sections.schedule);
    let show_results = has_results;

    let show_teams = teams_will_render(
        content.sections.teams,
        teams_empty,
        home_shell.show_when_empty(HomeSectionId::Teams),
    );
    // Secondary CTA only when Teams block will render.
    let show_secondary_cta = show_teams;

    let show_news = content.sections.news
        && (!news_list.is_empty() || home_shell.show_when_empty(HomeSectionId::News));

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
                                        live_tournaments: live_tournaments.clone(),
                                        upcoming_matches: upcoming_matches.clone(),
                                        recent_results: recent_results.clone(),
                                        show_schedule,
                                        show_tourneys,
                                        show_next_match,
                                        show_results,
                                    }
                                }
                            },
                            HomeSectionId::Teams if show_teams => rsx! {
                                TeamsBlock {
                                    content: content.clone(),
                                    overview: overview_data.clone(),
                                    presentation: teams_presentation,
                                }
                            },
                            HomeSectionId::News if show_news => rsx! {
                                NewsBlock {
                                    content: content.clone(),
                                    announcements: news_list.clone(),
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

/// Textless hero. Bars use the loaded hero's type scale so the swap keeps the rail.
#[component]
fn HeroSkeleton() -> Element {
    rsx! {
        header {
            class: "home-hero",
            aria_busy: "true",
            aria_label: "Loading",
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
        for needle in [
            "My Clan",
            "Your",
            "Clan",
            "Gaming clan",
            "Edit this copy",
            "Apply to join",
            "How we play",
        ] {
            assert!(!html.contains(needle), "{needle} in {html}");
        }
    }

    #[test]
    fn pending_hero_is_a_textless_skeleton() {
        let html = render(HeroSkeleton);
        assert!(html.contains("home-hero"), "{html}");
        assert!(html.contains("home-skel-title"), "{html}");
        assert!(html.contains("aria-busy"), "{html}");
        assert_no_template_copy(&html);
    }

    #[test]
    fn failed_settings_hero_is_an_error_not_the_template() {
        fn view() -> Element {
            let refresh = use_signal(|| 0u32);
            rsx! { HeroSettingsError { refresh } }
        }
        let html = render(view);
        // dioxus-ssr escapes the apostrophe in "Couldn't".
        assert!(
            html.contains("load this page") && html.contains("try again"),
            "{html}"
        );
        assert!(html.contains("Retry"), "{html}");
        assert_no_template_copy(&html);
    }
}
