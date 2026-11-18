//! Tournaments list (`d1-tournaments` frames): one card per tournament with a
//! linked title, the description, and a meta line that starts with the status
//! as a word in a badge.

use dioxus::prelude::*;
use serde::Deserialize;

use super::article_author::{
    ARTICLE_CSS, ARTICLE_CSS_ID, Load, entry_date, is_retrying, list_status,
};
use crate::components::ui::{
    EmptyCard, LoadError, PageHeader, PageShell, PageWidth, Pill, PillTone, SkeletonBars, btn_class,
};
use crate::hooks::CursorPage;
use crate::routes::Route;
use crate::state::use_auth;
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Tournament {
    id: String,
    name: String,
    #[allow(dead_code)]
    game_id: Option<String>,
    format: String,
    status: String,
    max_teams: Option<u32>,
    is_external: bool,
    #[allow(dead_code)]
    external_url: Option<String>,
    description: Option<String>,
    starts_at: Option<String>,
    #[allow(dead_code)]
    created_at: String,
}

/// Format in sentence case, as the frames write it ("Single elimination").
pub(super) fn format_label(f: &str) -> &str {
    match f {
        "single_elim" => "Single elimination",
        "double_elim" => "Double elimination",
        "round_robin" => "Round robin",
        "swiss" => "Swiss",
        _ => f,
    }
}

/// The word in the status badge. A tournament that takes registrations has
/// not started, so it reads "Upcoming" as in the frames; the meta line adds
/// that registration is open.
pub(super) fn status_label(s: &str) -> &str {
    match s {
        "draft" => "Draft",
        "registration" => "Upcoming",
        "in_progress" => "In progress",
        "completed" => "Completed",
        "archived" => "Archived",
        _ => s,
    }
}

/// "Started" once play has begun, "Starts" before. The API has no clock
/// field for this, so it follows the status.
pub(super) fn start_word(status: &str) -> &'static str {
    match status {
        "in_progress" | "completed" | "archived" => "Started",
        _ => "Starts",
    }
}

/// "8 team cap", or nothing when the tournament has no cap.
pub(super) fn team_cap(max_teams: Option<u32>) -> Option<String> {
    max_teams
        .filter(|n| *n > 0)
        .map(|n| format!("{n} team cap"))
}

/// The staff sentence is only for officers and admins. Visitors get the public line.
fn tournaments_empty_copy(show_staff_hint: bool) -> &'static str {
    if show_staff_hint {
        "When the org runs a bracket or open cup, it shows up here. Officers add them under Admin → Tournaments."
    } else {
        "When the org runs a bracket or open cup, it shows up here."
    }
}

/// Characters of a description shown on a card.
const DESC_PREVIEW_CHARS: usize = 120;

/// The card's description: trimmed, cut at [`DESC_PREVIEW_CHARS`] with "..."
/// when longer. `None` for a missing or blank description.
fn desc_preview(description: Option<&str>) -> Option<String> {
    let text = description?.trim();
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= DESC_PREVIEW_CHARS {
        return Some(text.to_string());
    }
    let cut: String = text.chars().take(DESC_PREVIEW_CHARS).collect();
    Some(format!("{}...", cut.trim_end()))
}

const PAGE_CSS: &str = r#"
/* The badge leads the meta line; the facts follow it. */
.tournaments-page .entry-row .entry-meta { margin-top: var(--space-2); }
"#;

/// `StyleOnce` id for [`FACTS_CSS`].
pub(super) const FACTS_CSS_ID: &str = "scuffed-css-pages-tournament-facts";

/// A fact and the dot after it are one flex item, so when the meta line
/// wraps on a phone the dot stays with its fact: no line starts with a dot
/// and no dot is left on a line of its own.
pub(super) const FACTS_CSS: &str = r#"
.meta-fact {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}
"#;

/// The facts after the status badge, each followed by a dot except the last.
pub(super) fn meta_facts(facts: Vec<Element>) -> Element {
    let last = facts.len().saturating_sub(1);
    rsx! {
        for (i, fact) in facts.into_iter().enumerate() {
            span { class: "meta-fact",
                {fact}
                if i < last {
                    span { class: "entry-sep", aria_hidden: "true", "·" }
                }
            }
        }
    }
}

/// The rows and whether the server has more than this first page.
type ListSlot = (Vec<Tournament>, bool);

#[component]
pub fn Tournaments() -> Element {
    let auth = use_auth();
    let mut refresh = use_signal(|| 0u32);
    let tournaments = use_resource(move || {
        let _ = refresh();
        async move {
            ApiClient::web()
                .fetch::<CursorPage<Tournament>>("/api/tournaments")
                .await
                .map(|page| (page.data, page.next_cursor.is_some()))
                .map_err(|e| e.to_string())
        }
    });

    let slot: Load<ListSlot> = Load::of(tournaments.read().as_ref());
    let (load, capped) = match slot {
        Load::Loading => (Load::Loading, false),
        Load::Failed => (Load::Failed, false),
        Load::Missing => (Load::Missing, false),
        Load::Ready((list, capped)) => (Load::Ready(visible(list)), capped),
    };
    let retrying = is_retrying(tournaments.state()(), &load);

    rsx! {
        TournamentsView {
            load,
            capped,
            retrying,
            show_staff: auth().is_officer_or_above(),
            on_retry: move |_| refresh += 1,
        }
    }
}

/// Drafts are not listed here, for staff either: they live under Admin.
fn visible(list: Vec<Tournament>) -> Vec<Tournament> {
    list.into_iter().filter(|t| t.status != "draft").collect()
}

/// The page for one fetch state. The status line is always in the tree; only
/// its text changes.
#[component]
fn TournamentsView(
    load: Load<Vec<Tournament>>,
    /// The server has more tournaments than the page it returned.
    #[props(default = false)]
    capped: bool,
    #[props(default = false)] retrying: bool,
    #[props(default = false)] show_staff: bool,
    on_retry: EventHandler<()>,
) -> Element {
    let status = list_status(
        &load,
        retrying,
        capped,
        "Loading tournaments",
        "tournaments",
    );

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-tournaments-page-css", css: PAGE_CSS }
        crate::styles::StyleOnce { id: FACTS_CSS_ID, css: FACTS_CSS }

        PageShell { width: PageWidth::Wide, class: "tournaments-page",
            PageHeader {
                title: "Tournaments",
                lead: "Open cups and in-house brackets for the crew.",
            }
            section { class: "ui-stack", aria_label: "All tournaments",
                p { class: "entry-status", role: "status", "{status}" }
                {match &load {
                    Load::Loading => rsx! { SkeletonBars { rows: 3 } },
                    Load::Failed | Load::Missing => rsx! {
                        LoadError {
                            message: "Couldn't load the tournaments. Check your connection and try again.",
                            on_retry: move |_| on_retry.call(()),
                            retrying,
                        }
                    },
                    Load::Ready(list) if list.is_empty() => rsx! {
                        EmptyCard {
                            title: "No tournaments yet",
                            message: tournaments_empty_copy(show_staff),
                            actions: rsx! {
                                Link { to: Route::Events {}, class: btn_class(true),
                                    "See upcoming events"
                                }
                                if show_staff {
                                    Link { to: Route::AdminTournaments {}, class: btn_class(false),
                                        "Open tournament admin"
                                    }
                                }
                            },
                        }
                    },
                    Load::Ready(list) => rsx! {
                        div { class: "entry-list",
                            for t in list.iter() {
                                Fragment { key: "{t.id}", {render_tournament_card(t)} }
                            }
                        }
                    },
                }}
            }
        }
    }
}

fn render_tournament_card(t: &Tournament) -> Element {
    let desc = desc_preview(t.description.as_deref());
    let registering = t.status == "registration";
    let cap = team_cap(t.max_teams);
    let start = start_word(&t.status);

    let mut facts: Vec<Element> = vec![rsx! { span { "{format_label(&t.format)}" } }];
    if registering {
        facts.push(rsx! { span { "Registration open" } });
    }
    if let Some(cap) = cap {
        facts.push(rsx! { span { "{cap}" } });
    }
    if let Some(date) = t.starts_at.as_deref() {
        facts.push(rsx! { span { "{start} " {entry_date(date)} } });
    }
    if t.is_external {
        facts.push(rsx! { span { "External" } });
    }

    rsx! {
        article { key: "{t.id}", class: "entry-row",
            h2 {
                Link { to: Route::Tournament { id: t.id.clone() }, "{t.name}" }
            }
            if let Some(desc) = desc {
                p { "{desc}" }
            }
            p { class: "entry-meta",
                Pill { tone: PillTone::Neutral, "{status_label(&t.status)}" }
                {meta_facts(facts)}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::article_author::test_support::render_routed;
    use super::*;

    fn cup(n: usize, status: &str) -> Tournament {
        Tournament {
            id: format!("cup{n}"),
            name: format!("Test Cup {n}"),
            game_id: None,
            format: "single_elim".to_string(),
            status: status.to_string(),
            max_teams: Some(8),
            is_external: false,
            external_url: None,
            description: Some("Four teams. Winner stays on for the final.".to_string()),
            starts_at: Some("2026-11-12T18:00:00Z".to_string()),
            created_at: "2026-11-01T10:00:00Z".to_string(),
        }
    }

    fn render_state(
        load: Load<Vec<Tournament>>,
        show_staff: bool,
        capped: bool,
        retrying: bool,
    ) -> String {
        render_routed(move || {
            rsx! {
                TournamentsView {
                    load: load.clone(),
                    capped,
                    retrying,
                    show_staff,
                    on_retry: move |_| {},
                }
            }
        })
    }

    fn render(load: Load<Vec<Tournament>>) -> String {
        render_state(load, false, false, false)
    }

    #[test]
    fn visitors_do_not_see_the_staff_tournaments_hint() {
        let visitor = tournaments_empty_copy(false);
        assert!(visitor.contains("shows up here"));
        assert!(!visitor.contains("Officers"));
        assert!(!visitor.contains("Admin"));
        let staff = tournaments_empty_copy(true);
        assert!(staff.contains("Officers add them under Admin"));
        assert!(staff.starts_with(visitor));
    }

    #[test]
    fn page_is_wide_with_one_h1_and_no_main() {
        let html = render(Load::Ready(vec![cup(1, "in_progress")]));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide tournaments-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Tournaments</h1>"), "{html}");
        assert!(
            html.contains(
                "<p class=\"page-lead\">Open cups and in-house brackets for the crew.</p>"
            ),
            "{html}"
        );
        assert!(!html.contains("<main"), "{html}");
    }

    #[test]
    fn card_has_linked_title_description_and_a_status_word() {
        let html = render(Load::Ready(vec![cup(1, "in_progress")]));
        assert!(
            html.contains("<h2><a href=\"/tournaments/cup1\">Test Cup 1</a></h2>"),
            "{html}"
        );
        assert!(
            html.contains("<p>Four teams. Winner stays on for the final.</p>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<p class=\"entry-meta\"><span class=\"ui-pill ui-pill--neutral\">In progress</span>\
                 <span class=\"meta-fact\"><span>Single elimination</span>"
            ),
            "{html}"
        );
        assert!(html.contains("<span>8 team cap</span>"), "{html}");
        assert!(
            html.contains(
                "Started <time class=\"entry-date\" datetime=\"2026-11-12\">12 Nov 2026</time>"
            ),
            "{html}"
        );
        // One link per card and no button inside it.
        assert_eq!(html.matches("<a ").count(), 1, "{html}");
        assert!(!html.contains("<button"), "{html}");
    }

    #[test]
    fn every_status_the_data_has_is_a_word() {
        for (status, word) in [
            ("registration", "Upcoming"),
            ("in_progress", "In progress"),
            ("completed", "Completed"),
            ("archived", "Archived"),
            ("draft", "Draft"),
        ] {
            assert_eq!(status_label(status), word);
            let html = render(Load::Ready(vec![cup(1, status)]));
            assert!(
                html.contains(&format!("ui-pill--neutral\">{word}</span>")),
                "{html}"
            );
        }
        // An unknown status still shows its text, never an empty badge.
        assert_eq!(status_label("paused"), "paused");

        let upcoming = render(Load::Ready(vec![cup(1, "registration")]));
        assert!(
            upcoming.contains("<span>Registration open</span>"),
            "{upcoming}"
        );
        assert!(upcoming.contains("Starts <time"), "{upcoming}");
    }

    #[test]
    fn optional_facts_drop_out_cleanly() {
        let mut bare = cup(1, "completed");
        bare.description = Some("   ".to_string());
        bare.max_teams = None;
        bare.starts_at = None;
        bare.is_external = true;
        let html = render(Load::Ready(vec![bare]));
        assert!(!html.contains("<p></p>"), "{html}");
        assert!(!html.contains("team cap"), "{html}");
        assert!(!html.contains("<time"), "{html}");
        assert!(html.contains("<span>External</span>"), "{html}");
        assert_eq!(html.matches("entry-sep").count(), 1, "{html}");
    }

    #[test]
    fn long_description_is_cut_with_three_dots() {
        let long = "word ".repeat(40);
        let cut = desc_preview(Some(&long)).expect("has text");
        assert!(cut.ends_with("..."), "{cut}");
        assert!(cut.chars().count() <= DESC_PREVIEW_CHARS + 3, "{cut}");
        assert_eq!(
            desc_preview(Some("Short one.")).as_deref(),
            Some("Short one.")
        );
        assert_eq!(desc_preview(Some("  ")), None);
        assert_eq!(desc_preview(None), None);
    }

    #[test]
    fn drafts_are_not_listed() {
        let list = visible(vec![cup(1, "draft"), cup(2, "registration")]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "cup2");
    }

    #[test]
    fn empty_state_says_what_is_missing_and_offers_a_next_step() {
        let guest = render(Load::Ready(Vec::new()));
        assert!(guest.contains("<h2>No tournaments yet</h2>"), "{guest}");
        assert!(guest.contains("href=\"/events\""), "{guest}");
        assert!(guest.contains(">See upcoming events</a>"), "{guest}");
        assert!(!guest.contains("Officers"), "{guest}");
        assert!(!guest.contains("/admin"), "{guest}");
        assert!(!guest.contains("entry-list"), "{guest}");

        let staff = render_state(Load::Ready(Vec::new()), true, false, false);
        assert!(staff.contains("Officers add them under Admin"), "{staff}");
        assert!(staff.contains(">Open tournament admin</a>"), "{staff}");
    }

    #[test]
    fn loading_and_failed_states_keep_the_chrome() {
        let loading = render(Load::Loading);
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading tournaments</p>"),
            "{loading}"
        );
        assert!(loading.contains("ui-skel-row"), "{loading}");
        assert!(!loading.contains("No tournaments yet"), "{loading}");

        let failed = render(Load::Failed);
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(failed.contains("load the tournaments."), "{failed}");
        assert!(failed.contains(">Retry</button>"), "{failed}");

        let retrying = render_state(Load::Failed, false, false, true);
        assert!(retrying.contains(">Retrying...</button>"), "{retrying}");
        assert!(retrying.contains("aria-disabled=\"true\""), "{retrying}");
        assert!(!retrying.contains(" disabled"), "{retrying}");

        // The live region and the H1 exist in every state.
        let empty = render(Load::Ready(Vec::new()));
        for html in [&loading, &failed, &retrying, &empty] {
            assert_eq!(
                html.matches("class=\"entry-status\" role=\"status\"")
                    .count(),
                1,
                "{html}"
            );
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
            assert!(!html.contains("<main"), "{html}");
        }
        assert!(
            empty.contains("<p class=\"entry-status\" role=\"status\"></p>"),
            "{empty}"
        );
    }

    #[test]
    fn a_cut_list_says_how_many_it_shows() {
        let html = render_state(
            Load::Ready(vec![cup(1, "completed"), cup(2, "completed")]),
            false,
            true,
            false,
        );
        assert!(
            html.contains(
                "<p class=\"entry-status\" role=\"status\">Showing the newest 2 tournaments.</p>"
            ),
            "{html}"
        );
    }

    #[test]
    fn page_css_uses_tokens() {
        for unit in ["px", "rem", "em;"] {
            assert!(!PAGE_CSS.contains(unit), "{unit} in the page css");
            assert!(!FACTS_CSS.contains(unit), "{unit} in the facts css");
        }
    }

    /// Browser pass at 390: the dots were flex items of their own, so a
    /// wrapped meta line started with a dot or left one at a line end alone.
    /// Each dot now trails its fact inside one group.
    #[test]
    fn each_dot_trails_its_fact_so_a_wrapped_line_never_starts_with_one() {
        let html = render(Load::Ready(vec![cup(1, "registration")]));
        let sep = "<span class=\"entry-sep\" aria-hidden=\"true\">·</span>";
        for fact in [
            "<span>Single elimination</span>",
            "<span>Registration open</span>",
            "<span>8 team cap</span>",
        ] {
            assert!(
                html.contains(&format!("<span class=\"meta-fact\">{fact}{sep}</span>")),
                "{fact}: {html}"
            );
        }
        // The last fact has no dot after it, and no group starts with one.
        assert!(
            html.contains("12 Nov 2026</time></span></span></p>"),
            "{html}"
        );
        assert!(
            !html.contains("<span class=\"meta-fact\"><span class=\"entry-sep\""),
            "{html}"
        );
        assert_eq!(html.matches("entry-sep").count(), 3, "{html}");
        assert!(
            html.contains("ui-pill--neutral\">Upcoming</span><span class=\"meta-fact\">"),
            "{html}"
        );
        let group = crate::components::ui::test_support::css_rule(FACTS_CSS, ".meta-fact {");
        assert!(group.contains("display: inline-flex"), "{group}");
    }
}
