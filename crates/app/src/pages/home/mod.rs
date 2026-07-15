//! Public homepage — shell/skin driven composition of shared presentational blocks.

mod blocks;
mod css;
mod data;

use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::{HomeSectionId, HomeShell, HomeSkin, SiteSettings, org_initials};

use crate::hooks::CursorPage;
use blocks::{
    EthosBlock, HeroBlock, LiveBlock, NewsBlock, RecruitBlock, TeamsBlock, live_panel_flags,
    teams_will_render,
};
use css::home_css_layers;
use data::{Announcement, Event, HomeTournament, Overview};

#[component]
pub fn Home() -> Element {
    let settings = use_resource(|| async {
        ApiClient::web()
            .fetch::<SiteSettings>("/api/settings")
            .await
            .ok()
    });
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

    let content = settings
        .read()
        .as_ref()
        .and_then(|s| s.as_ref())
        .map(|s| s.homepage.clone())
        .unwrap_or_default();
    let home_shell: HomeShell = settings
        .read()
        .as_ref()
        .and_then(|s| s.as_ref())
        .map(|s| s.home_shell)
        .unwrap_or(HomeShell::OpsHub);
    let home_skin: HomeSkin = settings
        .read()
        .as_ref()
        .and_then(|s| s.as_ref())
        .map(|s| s.home_skin)
        .unwrap_or(HomeSkin::Clean);
    let org_name = settings
        .read()
        .as_ref()
        .and_then(|s| s.as_ref())
        .map(|s| s.org_name.clone())
        .unwrap_or_else(|| "My Clan".into());
    let initials = org_initials(&org_name);
    let recruitment_open = settings
        .read()
        .as_ref()
        .and_then(|s| s.as_ref())
        .map(|s| s.recruitment_open)
        .unwrap_or(true);

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

    let has_events = !event_list.is_empty();
    let has_tourneys = !live_tournaments.is_empty();
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
                            HomeSectionId::Live if show_schedule || show_tourneys => rsx! {
                                LiveBlock {
                                    content: content.clone(),
                                    events: event_list.clone(),
                                    live_tournaments: live_tournaments.clone(),
                                    show_schedule,
                                    show_tourneys,
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
