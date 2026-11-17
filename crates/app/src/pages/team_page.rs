//! Public team page: roster, the most recent match and the weekly schedule.

use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{
    EmptyCard, LoadError, PageHeader, PageShell, PageWidth, Pill, PillTone, SkeletonBars, btn_class,
};
use crate::routes::Route;

use super::article_author::{
    ARTICLE_CSS, ARTICLE_CSS_ID, Load, entry_date, initials, is_retrying, load_status,
};
use super::match_detail::{Outcome, filled, load_of, match_kind_label};
use super::public_fetch::{PublicFetch, fetch_public};
use super::{NotFoundLink, NotFoundPanel};

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct RosterMember {
    member_id: String,
    display_name: String,
    avatar_url: Option<String>,
    team_role: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct TeamRecord {
    wins: u32,
    losses: u32,
    draws: u32,
}

/// Public match row. Intentionally omits `notes` / `recorded_by`; the public
/// projection is being narrowed and those fields must not be depended on.
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct TeamMatch {
    id: String,
    opponent: String,
    score_us: Option<u32>,
    score_them: Option<u32>,
    map_name: Option<String>,
    game_mode: Option<String>,
    match_type: String,
    /// RFC3339 when played; server recent_matches only returns played rows.
    #[serde(default)]
    played_at: Option<String>,
    #[serde(default)]
    scheduled_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct TeamEvent {
    title: String,
    day_of_week: u8,
    time: String,
    timezone: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct TeamDetailData {
    #[allow(dead_code)]
    id: String,
    name: String,
    division: Option<String>,
    lore_quote: Option<String>,
    logo_url: Option<String>,
    color: Option<String>,
    game_name: Option<String>,
    #[serde(default)]
    roster: Vec<RosterMember>,
    record: TeamRecord,
    #[serde(default)]
    recent_matches: Vec<TeamMatch>,
    #[serde(default)]
    upcoming_events: Vec<TeamEvent>,
}

fn day_name(d: u8) -> &'static str {
    match d {
        0 => "Mon",
        1 => "Tue",
        2 => "Wed",
        3 => "Thu",
        4 => "Fri",
        5 => "Sat",
        6 => "Sun",
        _ => "-",
    }
}

/// Badge word for a roster role. Unknown roles keep their own word.
fn role_label(team_role: &str) -> String {
    match team_role {
        "captain" => "Captain".to_string(),
        "player" => "Player".to_string(),
        "sub" => "Substitute".to_string(),
        "coach" => "Coach".to_string(),
        other => other.replace('_', " "),
    }
}

/// The line under the team name: game, division and the record once a match
/// has been played. `None` when the team has none of them.
fn team_lead(t: &TeamDetailData) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(game) = filled(t.game_name.as_ref()) {
        parts.push(game.to_string());
    }
    if let Some(division) = filled(t.division.as_ref()) {
        parts.push(division.to_string());
    }
    let r = &t.record;
    if r.wins + r.losses + r.draws > 0 {
        parts.push(format!(
            "Record {}",
            crate::util::record_label(r.wins, r.losses, r.draws)
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// `Squad Alpha 3 to 1 Crew Beta`, or `Squad Alpha vs Crew Beta` before a
/// score is in.
fn match_title(team: &str, m: &TeamMatch) -> String {
    match Outcome::of(m.score_us, m.score_them) {
        Some((_, us, them)) => format!("{team} {us} to {them} {}", m.opponent),
        None => format!("{team} vs {}", m.opponent),
    }
}

const PAGE_CSS: &str = r#"
.team-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3);
  margin: var(--space-4) 0 0;
  min-width: 0;
}
.team-logo {
  box-sizing: border-box;
  flex: 0 0 auto;
  width: var(--tile-size-lg);
  height: var(--tile-size-lg);
  border-radius: var(--tile-radius);
  border: 1px solid var(--card-border);
  background: var(--tile-bg);
  object-fit: cover;
}
.team-lore {
  margin: 0;
  min-width: 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-md);
  font-style: italic;
  line-height: var(--leading-body);
  overflow-wrap: anywhere;
}
.team-roster {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--space-3);
  margin: 0;
  padding: 0;
  list-style: none;
  min-width: 0;
}
.team-member {
  position: relative;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
  min-height: calc(var(--density-row) * 2);
  padding: var(--card-padding);
  text-align: center;
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.team-member:has(a:hover) { border-color: var(--border-strong); }
/* The tile of the design system: initials on the sunken fill. */
.team-tile {
  box-sizing: border-box;
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  width: var(--tile-size-lg);
  height: var(--tile-size-lg);
  border-radius: var(--tile-radius);
  background: var(--tile-bg);
  color: var(--tile-fg);
  font-family: var(--tile-font);
  font-weight: var(--tile-font-weight);
  font-size: var(--text-md);
  line-height: var(--leading-ui);
  object-fit: cover;
}
.team-member h3 {
  margin: 0;
  max-width: 100%;
  font-family: var(--font-head);
  font-weight: var(--font-label-weight);
  font-size: var(--text-md);
  line-height: var(--leading-ui);
  color: var(--text);
  overflow-wrap: anywhere;
}
.team-member h3 a { color: inherit; text-decoration: none; }
.team-member h3 a:hover { text-decoration: underline; }
/* The name link covers the whole card, so the target is the card. */
.team-member h3 a::after {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: inherit;
}
.team-member h3 a:focus-visible { outline: none; }
.team-member h3 a:focus-visible::after {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
/* The empty "Recent match" card: one sentence and a next step. */
.team-no-match {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
}
.team-schedule {
  margin: 0;
  padding: 0 var(--card-padding);
  list-style: none;
  min-width: 0;
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.team-schedule li {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-1) var(--space-4);
  min-height: var(--density-row);
  padding: var(--space-2) 0;
  box-sizing: border-box;
  font-family: var(--font-body);
  font-size: var(--text-md);
  line-height: var(--leading-body);
  color: var(--text);
}
.team-schedule li + li { border-top: 1px solid var(--border); }
.team-schedule-when {
  color: var(--text-3);
  font-size: var(--text-sm);
  font-variant-numeric: tabular-nums;
}
@media (max-width: 820px) {
  /* Two columns, as on the public Members page. */
  .team-roster { grid-template-columns: repeat(2, minmax(0, 1fr)); }
}
"#;

#[component]
pub fn TeamPage(id: String) -> Element {
    let mut refresh = use_signal(|| 0u32);
    let id_clone = id.clone();
    let team = use_resource(move || {
        let id = id_clone.clone();
        let _ = refresh();
        async move {
            if id.is_empty() {
                return PublicFetch::NotFound;
            }
            fetch_public::<TeamDetailData>(&format!("/api/public/teams/{id}")).await
        }
    });
    let load = load_of(team.read().as_ref());
    let retrying = is_retrying(team.state()(), &load);
    crate::head::use_page_title(tab_label(&load));

    rsx! {
        TeamView {
            load,
            retrying,
            on_retry: move |_| refresh += 1,
        }
    }
}

/// The browser tab names the team, not the record id in the address.
fn tab_label(load: &Load<TeamDetailData>) -> Option<String> {
    match load {
        Load::Ready(t) => Some(format!("{} | Team", t.name.trim())),
        Load::Missing => Some("Team not found".to_string()),
        Load::Loading | Load::Failed => None,
    }
}

/// The page for one fetch state. Every state has one H1, and the status line
/// is always in the tree; only its text changes.
#[component]
fn TeamView(
    load: Load<TeamDetailData>,
    #[props(default = false)] retrying: bool,
    on_retry: EventHandler<()>,
) -> Element {
    let status = load_status(&load, retrying, "Loading team");
    // The not-found card is the narrow wall of the 404 page.
    let width = if load == Load::Missing {
        PageWidth::Narrow
    } else {
        PageWidth::Wide
    };

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-team_page-page-css", css: PAGE_CSS }

        PageShell { width, class: "team-page",
            {match &load {
                Load::Loading => rsx! { PageHeader { title: "Team" } },
                Load::Failed => rsx! { PageHeader { title: "Couldn't load this team" } },
                Load::Missing | Load::Ready(_) => rsx! {},
            }}
            p { class: "entry-status", role: "status", "{status}" }
            {match &load {
                Load::Loading => rsx! { SkeletonBars { rows: 3 } },
                Load::Missing => rsx! {
                    NotFoundPanel {
                        title: "Team not found",
                        message: "We could not find that team. It may have been renamed or removed.",
                        links: vec![
                            NotFoundLink { label: "Return home", to: Route::Home {} },
                            NotFoundLink { label: "Browse members", to: Route::Members {} },
                        ],
                    }
                },
                Load::Failed => rsx! {
                    LoadError {
                        message: "Check your connection and try again.",
                        on_retry: move |_| on_retry.call(()),
                        retrying,
                    }
                },
                Load::Ready(t) => render_team(t),
            }}
        }
    }
}

fn render_team(t: &TeamDetailData) -> Element {
    let latest = t.recent_matches.first();
    let earlier = t.recent_matches.get(1..).unwrap_or_default();

    rsx! {
        {render_header(t)}

        section { class: "ui-stack",
            h2 { class: "ui-section-title", "Roster" }
            if t.roster.is_empty() {
                EmptyCard {
                    title: "No players listed yet",
                    message: "This team has no public roster yet. Meet the rest of the crew on the Members page.",
                    actions: rsx! {
                        Link { to: Route::Members {}, class: btn_class(true), "Browse members" }
                    },
                }
            } else {
                ul { class: "team-roster",
                    for m in t.roster.iter() {
                        {render_roster_card(m)}
                    }
                }
            }
        }

        section { class: "ui-stack",
            h2 { class: "ui-section-title", "Recent match" }
            match latest {
                Some(m) => rsx! {
                    ul { class: "entry-list",
                        {render_match_row(&t.name, m)}
                    }
                },
                None => rsx! {
                    div { class: "entry-row team-no-match",
                        p { "No matches recorded yet. The first official or tournament result shows up here." }
                        Link { to: Route::Tournaments {}, class: btn_class(false), "See tournaments" }
                    }
                },
            }
        }

        if !earlier.is_empty() {
            section { class: "ui-stack",
                h2 { class: "ui-section-title", "Earlier matches" }
                ul { class: "entry-list",
                    for m in earlier.iter() {
                        {render_match_row(&t.name, m)}
                    }
                }
            }
        }

        if !t.upcoming_events.is_empty() {
            section { class: "ui-stack",
                h2 { class: "ui-section-title", "Schedule" }
                ul { class: "team-schedule",
                    for e in t.upcoming_events.iter() {
                        li {
                            span { "{e.title}" }
                            span { class: "team-schedule-when",
                                "{day_name(e.day_of_week)} · {e.time} {e.timezone}"
                            }
                        }
                    }
                }
            }
        }
    }
}

fn render_header(t: &TeamDetailData) -> Element {
    let lore = filled(t.lore_quote.as_ref());
    let logo = filled(t.logo_url.as_ref());
    // The team's own colour, set by an admin, rings its logo.
    let logo_style = filled(t.color.as_ref())
        .map(|c| format!("border-color: {c};"))
        .unwrap_or_default();
    let meta = (lore.is_some() || logo.is_some()).then(|| {
        rsx! {
            div { class: "team-meta",
                if let Some(url) = logo {
                    img {
                        class: "team-logo",
                        style: "{logo_style}",
                        src: "{url}",
                        alt: "{t.name} logo",
                    }
                }
                if let Some(lore) = lore {
                    p { class: "team-lore", "“{lore}”" }
                }
            }
        }
    });

    rsx! {
        PageHeader { title: t.name.clone(), lead: team_lead(t), meta }
    }
}

fn render_roster_card(m: &RosterMember) -> Element {
    let role = role_label(&m.team_role);

    rsx! {
        li { key: "{m.member_id}", class: "team-member",
            // The name is right under the tile, so neither repeats it.
            if let Some(url) = filled(m.avatar_url.as_ref()) {
                img { class: "team-tile", src: "{url}", alt: "" }
            } else {
                span { class: "team-tile", aria_hidden: "true", "{initials(&m.display_name)}" }
            }
            h3 {
                Link { to: Route::MemberProfile { id: m.member_id.clone() }, "{m.display_name}" }
            }
            Pill { tone: PillTone::Neutral, "{role}" }
        }
    }
}

fn render_match_row(team: &str, m: &TeamMatch) -> Element {
    let title = match_title(team, m);
    let when = m.played_at.as_deref().or(m.scheduled_at.as_deref());
    let mut details: Vec<String> = vec![match_kind_label(&m.match_type)];
    if let Some(map) = filled(m.map_name.as_ref()) {
        details.push(map.to_string());
    }
    if let Some(mode) = filled(m.game_mode.as_ref()) {
        details.push(mode.to_string());
    }
    let details = details.join(" · ");

    rsx! {
        li { key: "{m.id}", class: "entry-row",
            h2 {
                Link { to: Route::MatchDetail { id: m.id.clone() }, "{title}" }
            }
            p { class: "entry-meta",
                if let Some(when) = when {
                    {entry_date(when)}
                    span { aria_hidden: "true", "·" }
                }
                span { "{details}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{
        assert_sizes_from_tokens, css_rule, has_decl, render_routed,
    };

    const TEAM_ID: &str = "team:alpha9";

    fn member(n: u8, role: &str) -> RosterMember {
        RosterMember {
            member_id: format!("member:r{n}x"),
            display_name: format!("Player{n:02}"),
            avatar_url: None,
            team_role: role.to_string(),
        }
    }

    fn game(n: u8, us: u32, them: u32) -> TeamMatch {
        TeamMatch {
            id: format!("match_result:g{n}"),
            opponent: "Crew Beta".to_string(),
            score_us: Some(us),
            score_them: Some(them),
            map_name: Some("Route 66".to_string()),
            game_mode: None,
            match_type: "tournament".to_string(),
            played_at: Some(format!("2026-11-{:02}T19:00:00Z", 13 - n)),
            scheduled_at: None,
        }
    }

    fn team() -> TeamDetailData {
        TeamDetailData {
            id: TEAM_ID.to_string(),
            name: "Squad Alpha".to_string(),
            division: Some("Division 2".to_string()),
            lore_quote: None,
            logo_url: None,
            color: None,
            game_name: Some("Overwatch".to_string()),
            roster: vec![member(1, "captain"), member(3, "player"), member(7, "sub")],
            record: TeamRecord {
                wins: 1,
                losses: 0,
                draws: 0,
            },
            recent_matches: vec![game(1, 3, 1)],
            upcoming_events: Vec::new(),
        }
    }

    fn render(load: Load<TeamDetailData>) -> String {
        render_routed(move || {
            rsx! {
                TeamView { load: load.clone(), retrying: false, on_retry: move |_| {} }
            }
        })
    }

    #[test]
    fn team_has_one_h1_roster_cards_and_a_link_to_the_latest_match() {
        let html = render(Load::Ready(team()));
        assert!(!html.contains("<main"), "{html}");
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide team-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Squad Alpha</h1>"), "{html}");
        assert!(
            html.contains("<p class=\"page-lead\">Overwatch · Division 2 · Record 1W-0L</p>"),
            "{html}"
        );

        // Roster: initials tile hidden from assistive tech, display name as
        // the link to the profile, role as a badge word.
        assert_eq!(html.matches("class=\"team-member\"").count(), 3, "{html}");
        assert!(
            html.contains("<span class=\"team-tile\" aria-hidden=\"true\">PL</span>"),
            "{html}"
        );
        assert!(html.contains("href=\"/members/member:r1x\""), "{html}");
        assert!(html.contains(">Player01</a>"), "{html}");
        for role in ["Captain", "Player", "Substitute"] {
            assert!(
                html.contains(&format!(
                    "<span class=\"ui-pill ui-pill--neutral\">{role}</span>"
                )),
                "{role}: {html}"
            );
        }
        // A member id is only ever an address, never text.
        assert!(!html.contains(">member:r1x<"), "{html}");

        // Recent match: the score as words, a link to the match, the date.
        assert!(html.contains("Recent match</h2>"), "{html}");
        assert!(html.contains("href=\"/matches/match_result:g1\""), "{html}");
        assert!(html.contains(">Squad Alpha 3 to 1 Crew Beta</a>"), "{html}");
        assert!(
            html.contains("datetime=\"2026-11-12\">12 Nov 2026</time>"),
            "{html}"
        );
        assert!(html.contains("Tournament match · Route 66"), "{html}");
        assert!(!html.contains("Earlier matches"), "{html}");
        assert!(!html.contains("Schedule"), "{html}");
        // No button inside a link, no link inside a link.
        assert!(!html.contains("<button"), "{html}");
    }

    #[test]
    fn a_team_with_no_matches_keeps_the_roster_and_an_empty_recent_match_card() {
        let html = render(Load::Ready(TeamDetailData {
            recent_matches: Vec::new(),
            record: TeamRecord {
                wins: 0,
                losses: 0,
                draws: 0,
            },
            ..team()
        }));
        assert_eq!(html.matches("class=\"team-member\"").count(), 3, "{html}");
        assert!(html.contains("Recent match</h2>"), "{html}");
        assert!(html.contains("class=\"entry-row team-no-match\""), "{html}");
        assert!(html.contains("No matches recorded yet."), "{html}");
        assert!(html.contains("href=\"/tournaments\""), "{html}");
        // No record to show before the first match.
        assert!(
            html.contains("<p class=\"page-lead\">Overwatch · Division 2</p>"),
            "{html}"
        );
    }

    #[test]
    fn older_matches_and_the_schedule_keep_their_sections() {
        let html = render(Load::Ready(TeamDetailData {
            recent_matches: vec![game(1, 3, 1), game(2, 1, 2), game(3, 2, 2)],
            upcoming_events: vec![TeamEvent {
                title: "Team practice".to_string(),
                day_of_week: 3,
                time: "20:00".to_string(),
                timezone: "Europe/Oslo".to_string(),
            }],
            ..team()
        }));
        assert!(html.contains("Earlier matches</h2>"), "{html}");
        assert!(html.contains(">Squad Alpha 1 to 2 Crew Beta</a>"), "{html}");
        assert!(html.contains("href=\"/matches/match_result:g3\""), "{html}");
        assert_eq!(html.matches("class=\"entry-row\"").count(), 3, "{html}");
        assert!(html.contains("Schedule</h2>"), "{html}");
        assert!(html.contains("Team practice"), "{html}");
        assert!(html.contains("Thu · 20:00 Europe/Oslo"), "{html}");
    }

    #[test]
    fn empty_roster_says_so_and_offers_the_members_page() {
        let html = render(Load::Ready(TeamDetailData {
            roster: Vec::new(),
            ..team()
        }));
        assert!(html.contains("<h2>No players listed yet</h2>"), "{html}");
        assert!(html.contains("href=\"/members\""), "{html}");
        assert!(!html.contains("team-roster"), "{html}");
    }

    #[test]
    fn avatar_logo_and_lore_render_when_set() {
        let mut t = team();
        t.roster[0].avatar_url = Some("/uploads/p1.webp".to_string());
        t.logo_url = Some("/uploads/alpha.webp".to_string());
        t.lore_quote = Some("Push the cart".to_string());
        let html = render(Load::Ready(t));
        assert!(
            html.contains("<img class=\"team-tile\" src=\"/uploads/p1.webp\" alt=\"\""),
            "{html}"
        );
        assert!(html.contains("alt=\"Squad Alpha logo\""), "{html}");
        assert!(
            html.contains("<p class=\"team-lore\">“Push the cart”</p>"),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");

        // Nothing set: no empty meta row.
        let plain = render(Load::Ready(team()));
        assert!(!plain.contains("team-meta"), "{plain}");
    }

    #[test]
    fn loading_failed_and_missing_states() {
        let loading = render(Load::Loading);
        assert!(loading.contains("<h1>Team</h1>"), "{loading}");
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading team</p>"),
            "{loading}"
        );
        assert!(loading.contains("ui-skel-list"), "{loading}");

        let failed = render(Load::Failed);
        assert_eq!(failed.matches("<h1").count(), 1, "{failed}");
        assert!(failed.contains("load this team</h1>"), "{failed}");
        assert!(failed.contains("fetch-error__retry"), "{failed}");
        assert!(
            failed.contains("<p class=\"entry-status\" role=\"status\"></p>"),
            "{failed}"
        );

        let missing = render(Load::Missing);
        assert_eq!(missing.matches("<h1").count(), 1, "{missing}");
        assert!(missing.contains("Team not found"), "{missing}");
        assert!(missing.contains("ui-shell--narrow"), "{missing}");
        assert!(missing.contains("href=\"/members\""), "{missing}");
        for html in [&loading, &failed, &missing] {
            assert!(!html.contains("<main"), "{html}");
        }
    }

    #[test]
    fn tab_is_named_by_the_team_not_the_id() {
        assert_eq!(
            tab_label(&Load::Ready(team())).as_deref(),
            Some("Squad Alpha | Team")
        );
        assert_eq!(tab_label(&Load::Missing).as_deref(), Some("Team not found"));
        assert_eq!(tab_label(&Load::Failed), None);
    }

    #[test]
    fn labels_and_titles() {
        assert_eq!(role_label("coach"), "Coach");
        assert_eq!(role_label("team_manager"), "team manager");
        let mut m = game(1, 2, 2);
        assert_eq!(
            match_title("Squad Alpha", &m),
            "Squad Alpha 2 to 2 Crew Beta"
        );
        m.score_them = None;
        assert_eq!(match_title("Squad Alpha", &m), "Squad Alpha vs Crew Beta");
        assert_eq!(day_name(6), "Sun");
    }

    #[test]
    fn css_is_on_tokens_with_a_two_column_phone_roster() {
        assert_sizes_from_tokens(PAGE_CSS);
        let grid = css_rule(PAGE_CSS, ".team-roster {");
        assert!(
            has_decl(&grid, "grid-template-columns: repeat(4, minmax(0, 1fr))"),
            "{grid}"
        );
        let tile = css_rule(PAGE_CSS, ".team-tile {");
        for decl in [
            "width: var(--tile-size-lg)",
            "border-radius: var(--tile-radius)",
            "background: var(--tile-bg)",
            "color: var(--tile-fg)",
        ] {
            assert!(has_decl(&tile, decl), "missing {decl}: {tile}");
        }
        let card = css_rule(PAGE_CSS, ".team-member {");
        assert!(
            has_decl(&card, "min-height: calc(var(--density-row) * 2)"),
            "{card}"
        );
        let (_, phone) = PAGE_CSS
            .split_once("@media (max-width: 820px) {")
            .expect("phone block");
        assert!(
            has_decl(
                &css_rule(phone, ".team-roster {"),
                "grid-template-columns: repeat(2, minmax(0, 1fr))"
            ),
            "{phone}"
        );
    }
}
