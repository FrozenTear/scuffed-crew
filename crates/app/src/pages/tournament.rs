//! One tournament (`d1-tournament-bracket` frames): breadcrumb, the name as
//! the H1 with a one line summary, then the bracket in its own scroller.

use dioxus::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;

use super::article_author::{
    ARTICLE_CSS, ARTICLE_CSS_ID, ArticleCrumb, Load, entry_date, is_retrying, load_status,
};
use super::not_found::{NotFoundLink, NotFoundPanel};
use super::tournaments::{
    FACTS_CSS, FACTS_CSS_ID, format_label, meta_facts, start_word, status_label, team_cap,
};
use crate::components::bracket::{
    BRACKET_STYLES, BracketMatch, BracketRound, BracketView, StandingsLoad, SwissStanding,
};
use crate::components::ui::{
    EmptyCard, LoadError, PageHeader, PageShell, PageWidth, Pill, PillTone, SkeletonBars, btn_class,
};
use crate::routes::Route;
use crate::util::pluralize;
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[allow(dead_code)]
struct TournamentDetail {
    id: String,
    name: String,
    game_id: Option<String>,
    format: String,
    status: String,
    max_teams: Option<u32>,
    best_of: u32,
    is_external: bool,
    external_url: Option<String>,
    rules: Option<String>,
    description: Option<String>,
    starts_at: Option<String>,
    ends_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Participant {
    id: String,
    team_id: Option<String>,
    external_name: Option<String>,
    #[allow(dead_code)]
    seed: Option<u32>,
    #[allow(dead_code)]
    status: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct BracketData {
    tournament: TournamentDetail,
    participants: Vec<Participant>,
    rounds: Vec<BracketRound>,
    matches: Vec<BracketMatch>,
}

#[derive(Debug, Clone, Deserialize)]
struct Team {
    id: String,
    name: String,
}

use crate::hooks::CursorPage;

/// The line under the H1, as the frame writes it: "Single elimination.
/// 4 teams. Best of 3."
fn summary_lead(format: &str, teams: usize, best_of: u32) -> String {
    let mut lead = format!("{}.", format_label(format));
    if teams == 0 {
        lead.push_str(" No teams entered yet.");
    } else {
        lead.push_str(&format!(" {}.", pluralize(teams, "team", "teams")));
    }
    if best_of > 1 {
        lead.push_str(&format!(" Best of {best_of}."));
    }
    lead
}

/// Display name of each participant, keyed by participant id. An entrant
/// with nothing to show yet has no entry and reads "To be decided".
fn participant_names(
    participants: &[Participant],
    team_names: &HashMap<String, String>,
) -> HashMap<String, String> {
    participants
        .iter()
        .filter_map(|p| {
            let name = if let Some(ext) = &p.external_name {
                ext.clone()
            } else if let Some(tid) = &p.team_id {
                team_names.get(tid).cloned().unwrap_or_else(|| tid.clone())
            } else {
                return None;
            };
            Some((p.id.clone(), name))
        })
        .collect()
}

/// Browser tab label, or `None` to keep the route's default.
fn tab_label(load: &Load<BracketData>) -> Option<String> {
    match load {
        Load::Ready(data) => Some(format!("{} | Tournaments", data.tournament.name.trim())),
        Load::Missing => Some("Tournament not found | Tournaments".to_string()),
        Load::Loading | Load::Failed => None,
    }
}

const PAGE_CSS: &str = r#"
/* Status badge, then the facts, under the lead. */
.tournament-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin: var(--space-3) 0 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
}
.tournament-about,
.tournament-rules {
  margin: 0;
  max-width: var(--content-max-narrow);
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-md);
  line-height: var(--leading-body);
  overflow-wrap: anywhere;
  white-space: pre-line;
}
.tournament-about { margin-top: var(--space-3); }
.tournament-external-text {
  color: var(--text-3);
  font-size: var(--text-sm);
  overflow-wrap: anywhere;
}
"#;

#[component]
pub fn Tournament(id: String) -> Element {
    let id_for_bracket = id.clone();
    let id_for_standings = id.clone();

    let mut refresh = use_signal(|| 0u32);
    let bracket_data = use_resource(move || {
        let id = id_for_bracket.clone();
        let _ = refresh();
        async move {
            if id.is_empty() {
                return Err("HTTP error: 404".to_string());
            }
            ApiClient::web()
                .fetch::<BracketData>(&format!("/api/tournaments/{id}/bracket"))
                .await
                .map_err(|e| e.to_string())
        }
    });

    let teams = use_resource(|| async {
        ApiClient::web()
            .fetch::<CursorPage<Team>>("/api/teams")
            .await
            .ok()
            .map(|r| r.data)
    });

    let mut standings = use_resource(move || {
        let id = id_for_standings.clone();
        async move {
            ApiClient::web()
                .fetch::<Vec<SwissStanding>>(&format!("/api/tournaments/{id}/standings"))
                .await
                .map_err(|e| e.to_string())
        }
    });

    let team_names: HashMap<String, String> = teams
        .read()
        .as_ref()
        .and_then(|t| t.as_ref())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|t| (t.id, t.name))
        .collect();
    let swiss = match standings.read().as_ref() {
        None => StandingsLoad::Loading,
        Some(Ok(rows)) => StandingsLoad::Ready(rows.clone()),
        // A restart keeps the old error until the new answer lands.
        Some(Err(_)) if standings.state()() == UseResourceState::Pending => StandingsLoad::Retrying,
        Some(Err(_)) => StandingsLoad::Failed,
    };
    let load = Load::of(bracket_data.read().as_ref());
    let retrying = is_retrying(bracket_data.state()(), &load);
    crate::head::use_page_title(tab_label(&load));

    rsx! {
        TournamentView {
            load,
            team_names,
            swiss,
            retrying,
            on_retry: move |_| refresh += 1,
            on_standings_retry: move |_| standings.restart(),
        }
    }
}

/// The page for one fetch state. Every state has one H1, and the status line
/// is always in the tree; only its text changes.
#[component]
fn TournamentView(
    load: Load<BracketData>,
    /// Team id to team name, for participants that are org teams.
    team_names: HashMap<String, String>,
    swiss: StandingsLoad,
    #[props(default = false)] retrying: bool,
    on_retry: EventHandler<()>,
    #[props(default)] on_standings_retry: Option<EventHandler<()>>,
) -> Element {
    let status = load_status(&load, retrying, "Loading tournament");
    let current = match &load {
        Load::Ready(data) => Some(data.tournament.name.clone()),
        _ => None,
    };
    // The not-found card is the narrow wall of the 404 page.
    let width = if load == Load::Missing {
        PageWidth::Narrow
    } else {
        PageWidth::Wide
    };

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-tournament-page-css", css: PAGE_CSS }
        crate::styles::StyleOnce { id: FACTS_CSS_ID, css: FACTS_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-tournament-bracket-styles", css: BRACKET_STYLES }

        PageShell { width, class: "tournament-page",
            // The not-found card has its own way back, so a one item trail
            // above it would only repeat it.
            if load != Load::Missing {
                ArticleCrumb { to: Route::Tournaments {}, section: "Tournaments", current }
            }
            {match &load {
                // The name is not known yet, so the page is named by its kind.
                Load::Loading => rsx! { PageHeader { title: "Tournament" } },
                Load::Failed => rsx! { PageHeader { title: "Couldn't load this tournament" } },
                Load::Missing => rsx! {},
                Load::Ready(data) => rsx! { TournamentHeader { data: data.clone() } },
            }}
            p { class: "entry-status", role: "status", "{status}" }
            {match &load {
                Load::Loading => rsx! { SkeletonBars { rows: 3 } },
                Load::Missing => rsx! {
                    NotFoundPanel {
                        title: "Tournament not found",
                        message: "This tournament does not exist or is not public yet.",
                        links: vec![
                            NotFoundLink { label: "Back to tournaments", to: Route::Tournaments {} },
                            NotFoundLink { label: "Return home", to: Route::Home {} },
                        ],
                    }
                },
                // The server answers 500, not 404, for some unknown ids, so
                // this state cannot tell a wrong link from an outage. The
                // message covers both and the trail above leads back.
                Load::Failed => rsx! {
                    LoadError {
                        message: "The link may be wrong, or the server may be having trouble. Check the address or try again.",
                        on_retry: move |_| on_retry.call(()),
                        retrying,
                    }
                },
                Load::Ready(data) => rsx! {
                    TournamentBody {
                        data: data.clone(),
                        team_names: team_names.clone(),
                        swiss: swiss.clone(),
                        on_standings_retry,
                    }
                },
            }}
        }
    }
}

/// The H1, the summary line, the status badge with the facts, the
/// description and the external link.
#[component]
fn TournamentHeader(data: BracketData) -> Element {
    let t = &data.tournament;
    let lead = summary_lead(&t.format, data.participants.len(), t.best_of);
    let status_word = status_label(&t.status).to_string();
    let registering = t.status == "registration";
    let cap = team_cap(t.max_teams);
    let start = start_word(&t.status);
    let starts = t.starts_at.clone();
    let is_external = t.is_external;
    let about = t
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string);
    let url = t.external_url.as_deref().map(str::trim).unwrap_or_default();
    let href = crate::util::http_href(url).map(str::to_string);
    let url_text = (href.is_none() && !url.is_empty()).then(|| url.to_string());

    // The facts after the badge; each but the last carries the dot after it.
    let mut facts: Vec<Element> = Vec::new();
    if registering {
        facts.push(rsx! { span { "Registration open" } });
    }
    if let Some(cap) = cap {
        facts.push(rsx! { span { "{cap}" } });
    }
    if let Some(date) = starts.as_deref() {
        facts.push(rsx! { span { "{start} " {entry_date(date)} } });
    }
    if is_external {
        facts.push(rsx! { span { "External" } });
    }
    if let Some(text) = url_text {
        facts.push(rsx! { span { class: "tournament-external-text", "{text}" } });
    }

    rsx! {
        PageHeader {
            title: t.name.clone(),
            lead,
            meta: rsx! {
                p { class: "tournament-meta",
                    Pill { tone: PillTone::Neutral, "{status_word}" }
                    {meta_facts(facts)}
                }
                if let Some(about) = about {
                    p { class: "tournament-about", "{about}" }
                }
            },
            actions: href.map(|href| rsx! {
                a {
                    href: "{href}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    class: btn_class(false),
                    "Open the tournament site"
                    span { class: "visually-hidden", " (opens in a new tab)" }
                }
            }),
        }
    }
}

/// Bracket (or the empty card while there is none), then the rules.
#[component]
fn TournamentBody(
    data: BracketData,
    team_names: HashMap<String, String>,
    swiss: StandingsLoad,
    #[props(default)] on_standings_retry: Option<EventHandler<()>>,
) -> Element {
    let t = &data.tournament;
    let names = participant_names(&data.participants, &team_names);
    let ids: Vec<String> = data.participants.iter().map(|p| p.id.clone()).collect();
    let entrants = data.participants.len();
    let entered = pluralize(entrants, "team is", "teams are");
    let rules = t
        .rules
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_string);
    let back = rsx! {
        Link { to: Route::Tournaments {}, class: btn_class(false), "Back to tournaments" }
    };

    rsx! {
        if !data.matches.is_empty() {
            BracketView {
                name: t.name.clone(),
                format: t.format.clone(),
                rounds: data.rounds.clone(),
                matches: data.matches.clone(),
                participant_names: names,
                participant_ids: ids,
                swiss_standings: swiss,
                on_standings_retry,
            }
        } else if entrants == 0 {
            EmptyCard {
                title: "No teams entered yet",
                message: "Teams show up here once they are entered. The bracket follows when the field is set.",
                actions: back,
            }
        } else {
            EmptyCard {
                title: "Bracket not drawn yet",
                message: "{entered} entered. The bracket shows up here once it is drawn.",
                actions: back,
            }
        }

        if let Some(rules) = rules {
            section { class: "ui-stack",
                h2 { class: "ui-section-title", "Rules" }
                p { class: "tournament-rules", "{rules}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::article_author::test_support::render_routed;
    use super::*;
    use crate::components::ui::test_support::assert_sizes_from_tokens;

    fn participant(id: &str, team: Option<&str>, external: Option<&str>) -> Participant {
        Participant {
            id: id.to_string(),
            team_id: team.map(str::to_string),
            external_name: external.map(str::to_string),
            seed: None,
            status: "active".to_string(),
        }
    }

    fn round(id: &str, number: u32) -> BracketRound {
        BracketRound {
            id: id.to_string(),
            round_number: number,
            stage: "main".to_string(),
        }
    }

    fn game(
        id: &str,
        round: &str,
        pos: u32,
        a: &str,
        b: &str,
        won: Option<(u32, u32)>,
    ) -> BracketMatch {
        BracketMatch {
            id: id.to_string(),
            round_id: round.to_string(),
            bracket_position: pos,
            participant_a_id: Some(a.to_string()),
            participant_b_id: Some(b.to_string()),
            score_a: won.map(|s| s.0),
            score_b: won.map(|s| s.1),
            winner_id: won.map(|_| a.to_string()),
            status: if won.is_some() {
                "completed"
            } else {
                "pending"
            }
            .to_string(),
            replay_codes: Vec::new(),
        }
    }

    /// The frame's Test Cup: four teams, both semi-finals played.
    fn test_cup() -> BracketData {
        BracketData {
            tournament: TournamentDetail {
                id: "cup1".to_string(),
                name: "Test Cup".to_string(),
                game_id: None,
                format: "single_elim".to_string(),
                status: "in_progress".to_string(),
                max_teams: Some(4),
                best_of: 3,
                is_external: false,
                external_url: None,
                rules: Some("Best of three.\nNo coaching mid map.".to_string()),
                description: Some("Winner stays on for the final.".to_string()),
                starts_at: Some("2026-11-12T18:00:00Z".to_string()),
                ends_at: None,
            },
            participants: vec![
                participant("p:a", Some("team:alpha"), None),
                participant("p:b", None, Some("Crew Beta")),
                participant("p:c", Some("team:gamma"), None),
                participant("p:d", None, Some("Crew Delta")),
            ],
            rounds: vec![round("r:1", 1), round("r:2", 2)],
            matches: vec![
                game("m:1", "r:1", 0, "p:a", "p:b", Some((2, 0))),
                game("m:2", "r:1", 1, "p:c", "p:d", Some((2, 1))),
                game("m:3", "r:2", 0, "p:a", "p:c", None),
            ],
        }
    }

    fn team_names() -> HashMap<String, String> {
        [("team:alpha", "Squad Alpha"), ("team:gamma", "Squad Gamma")]
            .into_iter()
            .map(|(id, name)| (id.to_string(), name.to_string()))
            .collect()
    }

    fn render_state(load: Load<BracketData>, retrying: bool) -> String {
        render_routed(move || {
            rsx! {
                TournamentView {
                    load: load.clone(),
                    team_names: team_names(),
                    swiss: StandingsLoad::Ready(Vec::new()),
                    retrying,
                    on_retry: move |_| {},
                }
            }
        })
    }

    fn render(load: Load<BracketData>) -> String {
        render_state(load, false)
    }

    #[test]
    fn ready_page_has_crumb_one_h1_lead_and_no_main() {
        let html = render(Load::Ready(test_cup()));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide tournament-page\""),
            "{html}"
        );
        assert!(
            html.contains(
                "<nav class=\"article-crumb\" aria-label=\"Breadcrumb\"><a href=\"/tournaments\">Tournaments</a>\
                 <span aria-hidden=\"true\">/</span><span aria-current=\"page\">Test Cup</span></nav>"
            ),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Test Cup</h1>"), "{html}");
        assert!(
            html.contains("<p class=\"page-lead\">Single elimination. 4 teams. Best of 3.</p>"),
            "{html}"
        );
        assert!(!html.contains("<main"), "{html}");
        assert!(
            html.contains("<p class=\"entry-status\" role=\"status\"></p>"),
            "{html}"
        );
    }

    #[test]
    fn meta_line_leads_with_the_status_word() {
        let html = render(Load::Ready(test_cup()));
        assert!(
            html.contains(
                "<p class=\"tournament-meta\"><span class=\"ui-pill ui-pill--neutral\">In progress</span>\
                 <span class=\"meta-fact\"><span>4 team cap</span><span class=\"entry-sep\" aria-hidden=\"true\">·</span></span>\
                 <span class=\"meta-fact\"><span>Started <time class=\"entry-date\" datetime=\"2026-11-12\">12 Nov 2026</time></span></span></p>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"tournament-about\">Winner stays on for the final.</p>"),
            "{html}"
        );
        assert!(!html.contains(">Live<"), "{html}");
    }

    #[test]
    fn bracket_shows_names_round_labels_and_the_named_scroller() {
        let html = render(Load::Ready(test_cup()));
        assert!(html.contains("aria-label=\"Test Cup bracket\""), "{html}");
        assert!(html.contains("tabindex=\"0\""), "{html}");
        for head in ["Semi-finals", "Final", "Champion"] {
            assert!(
                html.contains(&format!("<h2 class=\"visually-hidden\">{head}</h2>")),
                "{head}: {html}"
            );
        }
        // Team names from the team list and from external entrants.
        assert!(
            html.contains("Squad Alpha<span class=\"visually-hidden\">, winner</span>"),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"slot-team\">Crew Beta</p>"),
            "{html}"
        );
        assert!(html.contains("Won 2 to 0"), "{html}");
        assert!(html.contains("Won 2 to 1"), "{html}");
        for raw in ["p:a", "m:1", "r:1"] {
            assert!(!html.contains(raw), "raw id {raw} leaked: {html}");
        }
        // Rules follow the bracket under their own H2.
        let bracket = html.find("bracket-clip").expect("bracket");
        let rules = html
            .find("<h2 class=\"ui-section-title\">Rules</h2>")
            .expect("rules");
        assert!(bracket < rules, "{html}");
        assert!(html.contains("class=\"tournament-rules\""), "{html}");
    }

    #[test]
    fn participant_names_prefer_the_external_name_then_the_team() {
        let list = vec![
            participant("p:a", Some("team:alpha"), None),
            participant("p:b", Some("team:alpha"), Some("Crew Beta")),
            participant("p:c", Some("team:unknown"), None),
            participant("p:d", None, None),
        ];
        let names = participant_names(&list, &team_names());
        assert_eq!(names.get("p:a").map(String::as_str), Some("Squad Alpha"));
        assert_eq!(names.get("p:b").map(String::as_str), Some("Crew Beta"));
        // Unchanged: a team the list does not know falls back to its id.
        assert_eq!(names.get("p:c").map(String::as_str), Some("team:unknown"));
        // Nothing to show yet: the slot reads "To be decided".
        assert_eq!(names.get("p:d"), None);
    }

    #[test]
    fn no_entrants_uses_the_empty_card() {
        let mut cup = test_cup();
        cup.participants.clear();
        cup.rounds.clear();
        cup.matches.clear();
        cup.tournament.status = "registration".to_string();
        let html = render(Load::Ready(cup));
        assert!(
            html.contains("<section class=\"empty-card\"><h2>No teams entered yet</h2>"),
            "{html}"
        );
        assert!(html.contains(">Back to tournaments</a>"), "{html}");
        assert!(
            html.contains(
                "<p class=\"page-lead\">Single elimination. No teams entered yet. Best of 3.</p>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "ui-pill--neutral\">Upcoming</span><span class=\"meta-fact\"><span>Registration open</span>\
                 <span class=\"entry-sep\" aria-hidden=\"true\">·</span></span>"
            ),
            "{html}"
        );
        assert!(html.contains("Starts <time"), "{html}");
        assert!(!html.contains("bracket-clip"), "{html}");
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
    }

    #[test]
    fn entrants_without_a_bracket_say_how_many_are_in() {
        let mut cup = test_cup();
        cup.rounds.clear();
        cup.matches.clear();
        let html = render(Load::Ready(cup.clone()));
        assert!(html.contains("<h2>Bracket not drawn yet</h2>"), "{html}");
        assert!(
            html.contains("<p>4 teams are entered. The bracket"),
            "{html}"
        );

        cup.participants.truncate(1);
        let one = render(Load::Ready(cup));
        assert!(one.contains("<p>1 team is entered. The bracket"), "{one}");
    }

    #[test]
    fn missing_id_is_a_not_found_card_with_a_way_back() {
        let html = render(Load::Missing);
        assert!(
            html.contains("class=\"ui-shell ui-shell--narrow tournament-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("Tournament not found"), "{html}");
        assert!(html.contains("href=\"/tournaments\""), "{html}");
        assert!(html.contains(">Back to tournaments</a>"), "{html}");
        assert!(!html.contains("article-crumb"), "{html}");
        assert!(!html.contains("<main"), "{html}");
    }

    #[test]
    fn the_empty_id_and_a_404_both_read_as_missing() {
        // What the page stores for an empty id and what the client reports
        // for a 404 answer.
        for err in ["HTTP error: 404", "HTTP error 404: Tournament not found"] {
            let slot: Result<BracketData, String> = Err(err.to_string());
            assert_eq!(Load::of(Some(&slot)), Load::Missing, "{err}");
        }
        // A 500 is a failure the page offers to retry.
        let slot: Result<BracketData, String> = Err("HTTP error 500: Internal error".to_string());
        assert_eq!(Load::of(Some(&slot)), Load::Failed);
    }

    #[test]
    fn failed_load_has_plain_words_retry_and_the_way_back() {
        let html = render(Load::Failed);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("load this tournament</h1>"), "{html}");
        assert!(html.contains("role=\"alert\""), "{html}");
        assert!(html.contains("The link may be wrong"), "{html}");
        assert!(html.contains(">Retry</button>"), "{html}");
        assert!(
            html.contains("<a href=\"/tournaments\">Tournaments</a>"),
            "{html}"
        );
        // No raw client or server error text.
        assert!(!html.contains("HTTP error"), "{html}");

        let retrying = render_state(Load::Failed, true);
        assert!(retrying.contains(">Retrying...</button>"), "{retrying}");
        assert!(
            retrying.contains("<p class=\"entry-status\" role=\"status\">Loading tournament</p>"),
            "{retrying}"
        );
    }

    #[test]
    fn loading_is_labelled_inside_the_chrome() {
        let html = render(Load::Loading);
        assert!(html.contains("<h1>Tournament</h1>"), "{html}");
        assert!(
            html.contains("<p class=\"entry-status\" role=\"status\">Loading tournament</p>"),
            "{html}"
        );
        assert!(html.contains("ui-skel-row"), "{html}");
        assert!(
            html.contains("<a href=\"/tournaments\">Tournaments</a></nav>"),
            "{html}"
        );
    }

    #[test]
    fn external_link_is_a_named_safe_link_or_plain_text() {
        let mut cup = test_cup();
        cup.tournament.is_external = true;
        cup.tournament.external_url = Some("https://example.com/cup".to_string());
        let html = render(Load::Ready(cup.clone()));
        assert!(
            html.contains(
                "<a href=\"https://example.com/cup\" target=\"_blank\" rel=\"noopener noreferrer\""
            ),
            "{html}"
        );
        assert!(
            html.contains(">Open the tournament site<span class=\"visually-hidden\"> (opens in a new tab)</span></a>"),
            "{html}"
        );
        assert!(html.contains("<span>External</span>"), "{html}");

        cup.tournament.external_url = Some("javascript:alert(1)".to_string());
        let unsafe_url = render(Load::Ready(cup));
        assert!(!unsafe_url.contains("href=\"javascript"), "{unsafe_url}");
        assert!(
            unsafe_url
                .contains("<span class=\"tournament-external-text\">javascript:alert(1)</span>"),
            "{unsafe_url}"
        );
    }

    #[test]
    fn tab_label_names_the_tournament() {
        assert_eq!(
            tab_label(&Load::Ready(test_cup())).as_deref(),
            Some("Test Cup | Tournaments")
        );
        assert_eq!(
            tab_label(&Load::Missing).as_deref(),
            Some("Tournament not found | Tournaments")
        );
        assert_eq!(tab_label(&Load::Loading), None);
        assert_eq!(tab_label(&Load::Failed), None);
    }

    #[test]
    fn summary_lead_follows_the_frame_wording() {
        assert_eq!(
            summary_lead("single_elim", 4, 1),
            "Single elimination. 4 teams."
        );
        assert_eq!(summary_lead("swiss", 1, 3), "Swiss. 1 team. Best of 3.");
        assert_eq!(
            summary_lead("round_robin", 0, 1),
            "Round robin. No teams entered yet."
        );
    }

    #[test]
    fn page_css_uses_tokens() {
        assert_sizes_from_tokens(PAGE_CSS);
    }
}
