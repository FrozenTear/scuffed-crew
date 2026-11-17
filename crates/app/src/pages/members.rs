use dioxus::prelude::*;
use serde::Deserialize;

use super::article_author::{ARTICLE_CSS, ARTICLE_CSS_ID, Load, initials, is_retrying};
use crate::components::ui::inline_confirm::focus_element_by_id_soon;
use crate::components::ui::{
    ChipButton, ChipRow, EmptyCard, HeroSelect, LoadError, PageHeader, PageShell, PageWidth, Pager,
    Pill, btn_class, page_slice, show_list_top, use_url_page,
};
use crate::routes::Route;
use crate::sample::{LIST_RULE_NOTE, list_result};
use crate::util::encode_query;
use scuffed_api_client::ApiClient;

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct PublicMember {
    id: String,
    display_name: String,
    org_role: String,
    avatar_url: Option<String>,
    /// Preferred competitive role (public API field).
    #[serde(default)]
    main_role: Option<String>,
    /// Hero-scoped performance, only present when the roster was fetched with a
    /// `?hero=` filter (contract Q2). A nested block, NOT sibling fields.
    /// Absent for members who have not played the selected hero (or when no
    /// hero filter is active), so it stays optional and serde-defaults to None.
    #[serde(default)]
    hero_scoped: Option<HeroScoped>,
}

/// Nested per-hero stat block attached to a [`PublicMember`] when the roster is
/// fetched with `?hero=`. `winrate` is a 0.0 to 1.0 fraction (multiply by 100 for
/// display).
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct HeroScoped {
    games: u32,
    /// Victories in `games`. Older servers omit the three counts.
    #[serde(default)]
    wins: u32,
    /// Defeats in `games`.
    #[serde(default)]
    losses: u32,
    /// Draws in `games`.
    #[serde(default)]
    draws: u32,
    winrate: f32,
}

#[derive(Deserialize)]
struct MembersResponse {
    data: Vec<PublicMember>,
    /// Present when more pages exist (same contract as `CursorResponse`).
    #[serde(default)]
    next_cursor: Option<String>,
}

/// Backend max page size (`PaginationParams` clamps to 1..=100).
const ROSTER_PAGE_LIMIT: u32 = 100;
/// Safety cap while walking cursors (100 x 20 = 2000 members). Large enough
/// for growth; avoids unbounded request loops.
const ROSTER_MAX_PAGES: usize = 20;

/// Cards per page: two rows of four on desktop, four rows of two on phone
/// (the Members frame, "1 to 8 of 24 members").
const MEMBERS_PAGE_SIZE: usize = 8;

/// The list section. It takes focus after a page change.
const LIST_ID: &str = "members-list";

/// Where focus goes after "Clear filters", which removes itself: the role
/// chip that is now pressed.
const CLEAR_FOCUS_ID: &str = RoleFilter::All.chip_id();

/// One roster fetch. `hero` is the filter it was fetched with, so a roster
/// that belongs to the previous selection is never drawn under the new one.
#[derive(Debug, Clone, PartialEq)]
struct Roster {
    hero: Option<String>,
    members: Vec<PublicMember>,
    /// The walk stopped at [`ROSTER_MAX_PAGES`] with a next cursor still set.
    truncated: bool,
}

/// Path of one roster request.
fn roster_path(hero: Option<&str>, cursor: Option<&str>) -> String {
    let mut path = format!("/api/public/members?limit={ROSTER_PAGE_LIMIT}");
    if let Some(h) = hero {
        path.push_str(&format!("&hero={}", encode_query(h)));
    }
    if let Some(c) = cursor {
        path.push_str(&format!("&cursor={}", encode_query(c)));
    }
    path
}

/// Fetch the roster. The endpoint is cursor paged and returns no total, so the
/// pager needs every row: walk `next_cursor` to the end, with and without a
/// hero filter (players past offset 100 still appear, HS-DR FE-1). `None` is a
/// failed request.
async fn fetch_roster(hero: Option<String>) -> Option<Roster> {
    let mut members = Vec::new();
    let mut cursor: Option<String> = None;
    let mut truncated = true;
    for _ in 0..ROSTER_MAX_PAGES {
        let path = roster_path(hero.as_deref(), cursor.as_deref());
        let page = ApiClient::web()
            .fetch::<MembersResponse>(&path)
            .await
            .ok()?;
        members.extend(page.data);
        match page.next_cursor {
            Some(c) if !c.is_empty() => cursor = Some(c),
            _ => {
                truncated = false;
                break;
            }
        }
    }
    Some(Roster {
        hero,
        members,
        truncated,
    })
}

/// The role chips. A member's `main_role` is free text set on the profile
/// form ("tank", "dps", "support", "flex").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoleFilter {
    All,
    Tank,
    Damage,
    Support,
    Flex,
}

impl RoleFilter {
    const CHIPS: [RoleFilter; 5] = [
        RoleFilter::All,
        RoleFilter::Tank,
        RoleFilter::Damage,
        RoleFilter::Support,
        RoleFilter::Flex,
    ];

    const fn label(self) -> &'static str {
        match self {
            RoleFilter::All => "All roles",
            RoleFilter::Tank => "Tank",
            RoleFilter::Damage => "Damage",
            RoleFilter::Support => "Support",
            RoleFilter::Flex => "Flex",
        }
    }

    /// Id of the chip: the field name plus the value.
    const fn chip_id(self) -> &'static str {
        match self {
            RoleFilter::All => "members-role-all",
            RoleFilter::Tank => "members-role-tank",
            RoleFilter::Damage => "members-role-damage",
            RoleFilter::Support => "members-role-support",
            RoleFilter::Flex => "members-role-flex",
        }
    }

    /// The role a stored `main_role` names, if it is one of the known four.
    fn of(main_role: Option<&str>) -> Option<RoleFilter> {
        match main_role?.trim().to_ascii_lowercase().as_str() {
            "tank" => Some(RoleFilter::Tank),
            "dps" | "damage" => Some(RoleFilter::Damage),
            "support" => Some(RoleFilter::Support),
            "flex" => Some(RoleFilter::Flex),
            _ => None,
        }
    }

    fn admits(self, m: &PublicMember) -> bool {
        self == RoleFilter::All || RoleFilter::of(m.main_role.as_deref()) == Some(self)
    }
}

/// Role line on a card: the chip's word for a known role, the stored text
/// otherwise, nothing when unset.
fn role_text(main_role: Option<&str>) -> Option<String> {
    let raw = main_role?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(match RoleFilter::of(Some(raw)) {
        Some(role) => role.label().to_string(),
        None => raw.to_string(),
    })
}

/// The members to list, in order. A hero filter keeps only members who have
/// played the hero, sorted by games then win rate; the role chip narrows
/// either list.
fn visible_members(
    members: &[PublicMember],
    hero_active: bool,
    role: RoleFilter,
) -> Vec<&PublicMember> {
    let mut list: Vec<&PublicMember> = members
        .iter()
        .filter(|m| role.admits(m) && (!hero_active || m.hero_scoped.is_some()))
        .collect();
    if hero_active {
        list.sort_by(|a, b| match (&a.hero_scoped, &b.hero_scoped) {
            (Some(a), Some(b)) => b.games.cmp(&a.games).then_with(|| {
                b.winrate
                    .partial_cmp(&a.winrate)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            _ => std::cmp::Ordering::Equal,
        });
    }
    list
}

/// Text of the always-rendered status line.
fn roster_status(load: &Load<Roster>, retrying: bool) -> String {
    match load {
        _ if retrying => "Loading members".to_string(),
        Load::Loading => "Loading members".to_string(),
        Load::Ready(roster) if roster.truncated => {
            format!("Showing the first {} members.", roster.members.len())
        }
        _ => String::new(),
    }
}

const PAGE_CSS: &str = r#"
.members-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  min-width: 0;
}
.members-hero {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}
/* The frame's hero label is plain body text beside the select. */
.members-hero .field-label {
  font-family: var(--font-body);
  font-size: var(--text-md);
  font-weight: var(--font-body-weight);
}
/* The frame's select: strong edge on the surface, body text. */
.members-hero .ui-field {
  box-sizing: border-box;
  width: auto;
  max-width: 100%;
  min-height: var(--hit-target-min);
  background: var(--surface);
  border-color: var(--border-strong);
  font-size: var(--text-md);
  font-weight: var(--font-body-weight);
}
.members-hero .ui-field:focus { border-color: var(--accent-solid); }
/* The frame puts a section gap between the grid and the pager. */
.members-page .pager-target > .pager {
  margin-top: calc(var(--space-section) - var(--space-stack));
}
.members-note {
  margin: 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-body);
}
.member-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--space-3);
  min-width: 0;
}
.member-card {
  position: relative;
  box-sizing: border-box;
  min-width: 0;
  min-height: calc(var(--density-row) * 2);
  padding: var(--card-padding);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-2);
  text-align: center;
}
.member-card:has(.member-name a:hover) { border-color: var(--border-strong); }
/* Loading placeholder: the card outline at its resting size. */
.member-card.is-loading { min-height: calc(var(--density-row) * 4); }
.member-tile {
  flex: 0 0 auto;
  width: var(--tile-size-lg);
  height: var(--tile-size-lg);
  border-radius: var(--tile-radius);
  background: var(--tile-bg);
  color: var(--tile-fg);
  font-family: var(--tile-font);
  font-weight: var(--tile-font-weight);
  font-size: var(--text-md);
  line-height: var(--leading-ui);
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
}
.member-tile img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.member-name {
  margin: 0;
  max-width: 100%;
  font-family: var(--font-head);
  font-weight: var(--font-label-weight);
  font-size: var(--text-md);
  line-height: var(--leading-ui);
  color: var(--text);
  overflow-wrap: anywhere;
}
.member-name a {
  color: inherit;
  text-decoration: none;
}
/* The name link covers the whole card, so the target is the card. */
.member-name a::after {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: var(--card-radius);
}
.member-name a:hover { text-decoration: underline; }
.member-name a:focus-visible { outline: none; }
.member-name a:focus-visible::after {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
.member-role,
.member-hero-stat {
  margin: 0;
  max-width: 100%;
  font-family: var(--font-body);
  line-height: var(--leading-body);
  overflow-wrap: anywhere;
}
/* The frame's role line keeps a paragraph's margin above and below. */
.member-role {
  margin-block: var(--space-3);
  color: var(--text);
  font-size: var(--text-md);
}
.member-hero-stat {
  color: var(--text);
  font-size: var(--text-sm);
  font-variant-numeric: tabular-nums;
}
.member-hero-stat.muted { color: var(--text-3); }
@media (max-width: 820px) {
  .members-toolbar {
    flex-direction: column;
    align-items: stretch;
  }
  .members-hero .ui-field { font-size: var(--text-lg); }
  .member-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
}
"#;

#[component]
pub fn Members() -> Element {
    // `None` = "All heroes" (no filter). Drives both the HeroSelect value and
    // the resource re-fetch below.
    let mut hero = use_signal(|| None::<String>);
    let mut role = use_signal(|| RoleFilter::All);
    // The page lives in the address (`?page=2`), as on the blog list.
    let page = use_url_page();

    // Reading `hero()` inside the closure (before `async move`) makes the
    // resource reactive: it re-fetches whenever the selection changes.
    let mut members = use_resource(move || {
        let hero = hero();
        async move { fetch_roster(hero).await }
    });

    let fetched = Load::of_api(members.read().as_ref());
    let retrying = is_retrying(members.state()(), &fetched);
    // `use_resource` keeps the last roster while the next one is in flight.
    let load = match fetched {
        Load::Ready(roster) if roster.hero != hero() => Load::Loading,
        other => other,
    };
    // A new filter starts at the first page.
    let first_page = move || {
        if page.get() != 1 {
            page.go(1);
        }
    };

    rsx! {
        MembersIndex {
            load,
            hero: hero(),
            role: role(),
            page: page.get(),
            retrying,
            on_hero: move |next| {
                hero.set(next);
                first_page();
            },
            on_role: move |next| {
                role.set(next);
                first_page();
            },
            on_clear: move |_| {
                role.set(RoleFilter::All);
                hero.set(None);
                first_page();
            },
            on_retry: move |_| members.restart(),
            on_page: move |next| {
                page.go(next);
                show_list_top(LIST_ID);
            },
        }
    }
}

/// The page for one fetch state. The status line and the pager are always in
/// the tree; only their text changes.
#[component]
fn MembersIndex(
    load: Load<Roster>,
    hero: Option<String>,
    role: RoleFilter,
    page: usize,
    /// A Retry is running.
    #[props(default = false)]
    retrying: bool,
    on_hero: EventHandler<Option<String>>,
    on_role: EventHandler<RoleFilter>,
    /// Back to "All roles" and "All heroes".
    on_clear: EventHandler<()>,
    on_retry: EventHandler<()>,
    on_page: EventHandler<usize>,
) -> Element {
    let status = roster_status(&load, retrying);
    let hero_active = hero.is_some();
    let filtered = hero_active || role != RoleFilter::All;
    let visible = match &load {
        Load::Ready(roster) => visible_members(&roster.members, hero_active, role),
        _ => Vec::new(),
    };
    let total = visible.len();

    rsx! {
        // The shared list status line (`.entry-status`) lives in the article CSS.
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-members-page-css", css: PAGE_CSS }

        PageShell { width: PageWidth::Default, class: "members-page",
            PageHeader {
                title: "Members",
                lead: "The public roster. Filter by role, then page through the full list.",
            }
            div { class: "members-toolbar",
                ChipRow { label: "Filter by role", scroll: true,
                    for chip in RoleFilter::CHIPS {
                        ChipButton {
                            key: "{chip.chip_id()}",
                            class: "chip",
                            id: chip.chip_id(),
                            label: chip.label(),
                            pressed: chip == role,
                            onclick: move |_| on_role.call(chip),
                        }
                    }
                }
                div { class: "members-hero",
                    label { class: "field-label", r#for: "members-hero", "Hero" }
                    HeroSelect {
                        id: "members-hero".to_string(),
                        name: "members-hero".to_string(),
                        value: hero.clone(),
                        onchange: move |v| on_hero.call(v),
                    }
                }
            }
            section {
                id: LIST_ID,
                class: "ui-stack pager-target",
                tabindex: "-1",
                aria_label: "Members",
                p { class: "entry-status", role: "status", "{status}" }
                {match &load {
                    Load::Loading => rsx! {
                        div { class: "member-grid", aria_hidden: "true",
                            for i in 0..MEMBERS_PAGE_SIZE {
                                div { key: "{i}", class: "member-card is-loading" }
                            }
                        }
                    },
                    Load::Failed | Load::Missing => rsx! {
                        LoadError {
                            message: "Couldn't load the members. Check your connection and try again.",
                            on_retry: move |_| on_retry.call(()),
                            retrying,
                        }
                    },
                    Load::Ready(_) if visible.is_empty() && filtered => rsx! {
                        EmptyCard {
                            title: "No members match this filter",
                            message: "Try another role or hero, or clear the filters to see everyone.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    id: "members-clear-filters",
                                    class: btn_class(false),
                                    onclick: move |_| {
                                        on_clear.call(());
                                        focus_element_by_id_soon(CLEAR_FOCUS_ID.to_string());
                                    },
                                    "Clear filters"
                                }
                            },
                        }
                    },
                    Load::Ready(_) if visible.is_empty() => rsx! {
                        EmptyCard {
                            title: "No members yet",
                            message: "Check back soon, or be the first to join.",
                            actions: rsx! {
                                Link { to: Route::Apply {}, class: btn_class(true), "Apply to join" }
                            },
                        }
                    },
                    Load::Ready(_) => rsx! {
                        if hero_active {
                            p { class: "members-note", "{LIST_RULE_NOTE}" }
                        }
                        div { class: "member-grid",
                            for m in page_slice(&visible, page, MEMBERS_PAGE_SIZE).iter().copied() {
                                {render_member_card(m, hero.as_deref())}
                            }
                        }
                    },
                }}
                Pager {
                    id: "members-pager",
                    page,
                    total,
                    page_size: MEMBERS_PAGE_SIZE,
                    noun: "members",
                    noun_singular: "member",
                    on_change: move |next| on_page.call(next),
                }
            }
        }
    }
}

/// One roster card. The name is the link and covers the card; the tile is
/// decoration next to the name, so it is hidden from assistive tech.
fn render_member_card(m: &PublicMember, hero: Option<&str>) -> Element {
    let role = role_text(m.main_role.as_deref());
    // Hero-scoped stat line, only when a hero is selected AND this member has
    // played it (contract Q2 nested block present).
    let hero_stat = hero
        .zip(m.hero_scoped.as_ref())
        .map(|(h, hs)| hero_stat_line(h, hs));

    rsx! {
        article { key: "{m.id}", class: "member-card",
            div { class: "member-tile", aria_hidden: "true",
                if let Some(url) = m.avatar_url.as_ref().filter(|u| !u.is_empty()) {
                    img { src: "{url}", alt: "", loading: "lazy" }
                } else {
                    "{initials(&m.display_name)}"
                }
            }
            h2 { class: "member-name",
                Link { to: Route::MemberProfile { id: m.id.clone() }, "{m.display_name}" }
            }
            Pill { "{m.org_role}" }
            if let Some(role) = role {
                p { class: "member-role", "{role}" }
            }
            if let Some(stat) = &hero_stat {
                p { class: stat.class, "{stat.text}" }
            }
        }
    }
}

/// Hero stat line on a member card under the hero filter.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HeroStatLine {
    text: String,
    class: &'static str,
}

/// Small-sample rule on the hero stat line: the win rate from 10 decided games, the
/// record below that, muted under 3 games. `winrate` is a 0.0 to 1.0
/// fraction.
fn hero_stat_line(hero: &str, hs: &HeroScoped) -> HeroStatLine {
    let result = list_result(hs.games, hs.wins, hs.losses, hs.draws, hs.winrate);
    let games = hs.games;
    let text = if result.is_rate {
        format!("{games} games · {} WR on {hero}", result.text)
    } else {
        format!("{games} games · {} on {hero}", result.text)
    };
    let class = if result.muted {
        "member-hero-stat muted"
    } else {
        "member-hero-stat"
    };
    HeroStatLine { text, class }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{
        assert_sizes_from_tokens, css_rule, has_decl, render_routed,
    };

    fn line(games: u32, wins: u32, losses: u32, draws: u32, winrate: f32) -> HeroStatLine {
        let hs = HeroScoped {
            games,
            wins,
            losses,
            draws,
            winrate,
        };
        hero_stat_line("Ana", &hs)
    }

    #[test]
    fn hero_stat_line_follows_the_small_sample_rule() {
        let two = line(2, 1, 1, 0, 0.5);
        assert_eq!(two.text, "2 games · 1W-1L on Ana");
        assert_eq!(two.class, "member-hero-stat muted");

        let nine = line(9, 5, 3, 1, 5.0 / 9.0);
        assert_eq!(nine.text, "9 games · 5W-3L-1D on Ana");
        assert_eq!(nine.class, "member-hero-stat");
        assert!(!nine.text.contains('%'), "{}", nine.text);

        let ten = line(10, 6, 4, 0, 0.6);
        assert_eq!(ten.text, "10 games · 60% WR on Ana");
        assert_eq!(ten.class, "member-hero-stat");
    }

    #[test]
    fn hero_scoped_reads_the_record_and_older_blocks() {
        let new: HeroScoped =
            serde_json::from_str(r#"{"games":5,"wins":3,"losses":1,"draws":1,"winrate":0.6}"#)
                .unwrap();
        assert_eq!(
            hero_stat_line("Ana", &new).text,
            "5 games · 3W-1L-1D on Ana"
        );

        let old: HeroScoped = serde_json::from_str(r#"{"games":5,"winrate":0.6}"#).unwrap();
        assert_eq!((old.wins, old.losses, old.draws), (0, 0, 0));
        assert_eq!(hero_stat_line("Ana", &old).text, "5 games · 3W on Ana");
    }

    fn member(n: usize, main_role: Option<&str>) -> PublicMember {
        PublicMember {
            id: format!("member:{n:02}"),
            display_name: format!("Player{n:02}"),
            org_role: "member".to_string(),
            avatar_url: None,
            main_role: main_role.map(str::to_string),
            hero_scoped: None,
        }
    }

    fn roster(members: Vec<PublicMember>) -> Roster {
        Roster {
            hero: None,
            members,
            truncated: false,
        }
    }

    fn crew(count: usize) -> Vec<PublicMember> {
        let roles = [
            Some("tank"),
            Some("dps"),
            Some("support"),
            Some("flex"),
            None,
        ];
        (1..=count)
            .map(|n| member(n, roles[(n - 1) % roles.len()]))
            .collect()
    }

    #[derive(Clone, PartialEq)]
    struct View {
        load: Load<Roster>,
        hero: Option<String>,
        role: RoleFilter,
        page: usize,
        retrying: bool,
    }

    fn view(load: Load<Roster>) -> View {
        View {
            load,
            hero: None,
            role: RoleFilter::All,
            page: 1,
            retrying: false,
        }
    }

    fn render(view: View) -> String {
        render_routed(move || {
            let view = view.clone();
            rsx! {
                MembersIndex {
                    load: view.load,
                    hero: view.hero,
                    role: view.role,
                    page: view.page,
                    retrying: view.retrying,
                    on_hero: move |_| {},
                    on_role: move |_| {},
                    on_clear: move |_| {},
                    on_retry: move |_| {},
                    on_page: move |_| {},
                }
            }
        })
    }

    const LIST_OPEN: &str = "<section id=\"members-list\" class=\"ui-stack pager-target\" tabindex=\"-1\" aria-label=\"Members\">";

    #[test]
    fn every_state_has_one_h1_no_main_and_the_live_regions() {
        let ready = render(view(Load::Ready(roster(crew(3)))));
        let empty = render(view(Load::Ready(roster(Vec::new()))));
        let loading = render(view(Load::Loading));
        let failed = render(view(Load::Failed));
        for html in [&ready, &empty, &loading, &failed] {
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
            assert!(html.contains("<h1>Members</h1>"), "{html}");
            assert!(!html.contains("<main"), "{html}");
            assert!(html.contains(LIST_OPEN), "{html}");
            assert!(
                html.contains("<p class=\"entry-status\" role=\"status\">"),
                "{html}"
            );
            assert!(html.contains("id=\"members-pager-meta\""), "{html}");
            // The filters stay in place in every state.
            assert!(
                html.contains("role=\"group\" aria-label=\"Filter by role\""),
                "{html}"
            );
            assert!(html.contains("id=\"members-hero\""), "{html}");
        }
    }

    #[test]
    fn filters_are_named_controls() {
        let mut v = view(Load::Ready(roster(crew(3))));
        v.role = RoleFilter::Tank;
        let html = render(v);
        assert!(
            html.contains(
                "<button type=\"button\" id=\"members-role-all\" aria-pressed=\"false\" class=\"chip\">All roles</button>"
            ),
            "{html}"
        );
        assert!(
            html.contains(
                "<button type=\"button\" id=\"members-role-tank\" aria-pressed=\"true\" class=\"chip active\">Tank</button>"
            ),
            "{html}"
        );
        for label in ["Damage", "Support", "Flex"] {
            assert!(html.contains(&format!(">{label}</button>")), "{html}");
        }
        assert_eq!(html.matches("aria-pressed=\"true\"").count(), 1, "{html}");
        // The visible label points at the select, and the select has a name.
        assert!(
            html.contains("<label class=\"field-label\" for=\"members-hero\">Hero</label>"),
            "{html}"
        );
        assert!(html.contains("aria-label=\"Hero\""), "{html}");
    }

    #[test]
    fn card_is_an_article_with_a_linked_h2_and_no_nested_controls() {
        let mut admin = member(1, Some("dps"));
        admin.org_role = "admin".to_string();
        let html = render(view(Load::Ready(roster(vec![admin, member(2, None)]))));
        assert_eq!(html.matches("<article class=\"member-card\">").count(), 2);
        assert!(
            html.contains("<div class=\"member-tile\" aria-hidden=\"true\">P1</div>"),
            "{html}"
        );
        // Player01 and Player02 get different tiles (the shared avatar rule).
        assert!(
            html.contains("<div class=\"member-tile\" aria-hidden=\"true\">P2</div>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<h2 class=\"member-name\"><a href=\"/members/member:01\">Player01</a></h2>"
            ),
            "{html}"
        );
        // Every org role wears the same neutral badge, as the frame draws it.
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--neutral\">admin</span>"),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"member-role\">Damage</p>"),
            "{html}"
        );
        // No role set: no role line.
        assert_eq!(html.matches("class=\"member-role\"").count(), 1, "{html}");
        // The only link in a card is its name, and it holds no button.
        let grid = html.split("class=\"member-grid\"").nth(1).unwrap();
        let grid = grid.split("id=\"members-pager\"").next().unwrap();
        assert_eq!(grid.matches("<a ").count(), 2, "{grid}");
        assert!(!grid.contains("<button"), "{grid}");
    }

    #[test]
    fn avatar_image_is_decorative_next_to_the_name() {
        let mut m = member(1, Some("tank"));
        m.avatar_url = Some("/uploads/player01.png".to_string());
        let html = render(view(Load::Ready(roster(vec![m]))));
        assert!(
            html.contains("<img src=\"/uploads/player01.png\" alt=\"\" loading=\"lazy\"/>"),
            "{html}"
        );
        assert!(html.contains(">1 member</p>"), "{html}");
    }

    #[test]
    fn long_roster_is_paged_eight_at_a_time() {
        let first = render(view(Load::Ready(roster(crew(24)))));
        assert_eq!(first.matches("class=\"member-card\"").count(), 8, "{first}");
        assert!(first.contains(">1 to 8 of 24 members</p>"), "{first}");
        assert!(first.contains("id=\"members-pager-page-3\""), "{first}");

        let mut v = view(Load::Ready(roster(crew(20))));
        v.page = 3;
        let last = render(v);
        assert_eq!(last.matches("class=\"member-card\"").count(), 4, "{last}");
        assert!(last.contains(">Player20</a>"), "{last}");
        assert!(last.contains(">17 to 20 of 20 members</p>"), "{last}");
    }

    #[test]
    fn role_chip_narrows_the_list_and_the_count() {
        let mut v = view(Load::Ready(roster(crew(10))));
        v.role = RoleFilter::Damage;
        let html = render(v);
        // Players 2 and 7 have the stored role "dps".
        assert_eq!(html.matches("class=\"member-card\"").count(), 2, "{html}");
        assert!(html.contains(">Player02</a>"), "{html}");
        assert!(html.contains(">Player07</a>"), "{html}");
        assert!(html.contains(">2 members</p>"), "{html}");
    }

    #[test]
    fn role_names_map_from_the_stored_text() {
        assert_eq!(RoleFilter::of(Some("dps")), Some(RoleFilter::Damage));
        assert_eq!(RoleFilter::of(Some(" Tank ")), Some(RoleFilter::Tank));
        assert_eq!(RoleFilter::of(Some("flex")), Some(RoleFilter::Flex));
        assert_eq!(RoleFilter::of(Some("shotcaller")), None);
        assert_eq!(RoleFilter::of(None), None);
        assert_eq!(role_text(Some("support")).as_deref(), Some("Support"));
        assert_eq!(role_text(Some("shotcaller")).as_deref(), Some("shotcaller"));
        assert_eq!(role_text(Some("  ")), None);
        // An unset or unknown role shows under "All roles" only.
        let odd = member(1, Some("shotcaller"));
        assert!(RoleFilter::All.admits(&odd));
        for chip in &RoleFilter::CHIPS[1..] {
            assert!(!chip.admits(&odd), "{chip:?}");
        }
    }

    fn played(n: usize, games: u32, winrate: f32) -> PublicMember {
        let mut m = member(n, Some("support"));
        m.hero_scoped = Some(HeroScoped {
            games,
            wins: 0,
            losses: 0,
            draws: 0,
            winrate,
        });
        m
    }

    #[test]
    fn hero_filter_keeps_players_of_the_hero_sorted_by_games_then_rate() {
        let members = vec![
            member(1, Some("tank")),
            played(2, 12, 0.5),
            played(3, 30, 0.4),
            played(4, 12, 0.75),
        ];
        let order: Vec<&str> = visible_members(&members, true, RoleFilter::All)
            .iter()
            .map(|m| m.display_name.as_str())
            .collect();
        assert_eq!(order, ["Player03", "Player04", "Player02"]);
        // Without a hero the roster order is kept.
        assert_eq!(visible_members(&members, false, RoleFilter::All).len(), 4);

        let mut v = view(Load::Ready(Roster {
            hero: Some("Ana".to_string()),
            members,
            truncated: false,
        }));
        v.hero = Some("Ana".to_string());
        let html = render(v);
        assert_eq!(html.matches("class=\"member-card\"").count(), 3, "{html}");
        assert!(
            html.contains(&format!("<p class=\"members-note\">{LIST_RULE_NOTE}</p>")),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"member-hero-stat\">30 games · 40% WR on Ana</p>"),
            "{html}"
        );
    }

    #[test]
    fn empty_states_say_what_is_missing_and_offer_a_next_step() {
        let none = render(view(Load::Ready(roster(Vec::new()))));
        assert!(none.contains("<h2>No members yet</h2>"), "{none}");
        assert!(none.contains("href=\"/apply\""), "{none}");
        assert!(none.contains(">Apply to join</a>"), "{none}");
        assert!(!none.contains("Clear filters"), "{none}");

        let mut v = view(Load::Ready(roster(vec![member(1, Some("tank"))])));
        v.role = RoleFilter::Support;
        let filtered = render(v);
        assert!(
            filtered.contains("<h2>No members match this filter</h2>"),
            "{filtered}"
        );
        assert!(
            filtered.contains("id=\"members-clear-filters\""),
            "{filtered}"
        );
        assert!(filtered.contains(">Clear filters</button>"), "{filtered}");
        assert!(!filtered.contains("class=\"member-card\""), "{filtered}");
    }

    #[test]
    fn clear_filters_hands_focus_to_the_all_roles_chip() {
        // The button removes itself; focus goes to a control that stays.
        let mut v = view(Load::Ready(roster(vec![member(1, Some("tank"))])));
        v.role = RoleFilter::Support;
        let filtered = render(v);
        assert!(
            filtered.contains(&format!("id=\"{CLEAR_FOCUS_ID}\"")),
            "the focus target is on the page: {filtered}"
        );
        assert_eq!(CLEAR_FOCUS_ID, "members-role-all");
        let prod = include_str!("members.rs")
            .split("mod tests")
            .next()
            .unwrap();
        let button = prod
            .split("id: \"members-clear-filters\"")
            .nth(1)
            .expect("clear button");
        let click = &button[..button.find("\"Clear filters\"").expect("label")];
        assert!(
            click.contains("focus_element_by_id_soon(CLEAR_FOCUS_ID.to_string())"),
            "{click}"
        );
    }

    #[test]
    fn loading_failed_and_retrying_states() {
        let loading = render(view(Load::Loading));
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading members</p>"),
            "{loading}"
        );
        assert!(
            loading.contains("<div class=\"member-grid\" aria-hidden=\"true\">"),
            "{loading}"
        );
        assert_eq!(
            loading.matches("class=\"member-card is-loading\"").count(),
            MEMBERS_PAGE_SIZE,
            "{loading}"
        );
        assert!(!loading.contains("No members"), "{loading}");

        // A failed fetch is an error with a retry, not an endless "Loading".
        let failed = render(view(Load::Failed));
        assert!(failed.contains("role=\"alert\""), "{failed}");
        assert!(
            failed.contains("load the members. Check your connection"),
            "{failed}"
        );
        assert!(failed.contains(">Retry</button>"), "{failed}");
        assert!(!failed.contains("Loading"), "{failed}");

        let mut v = view(Load::Failed);
        v.retrying = true;
        let retrying = render(v);
        assert!(
            retrying.contains("<p class=\"entry-status\" role=\"status\">Loading members</p>"),
            "{retrying}"
        );
        assert!(retrying.contains(">Retrying...</button>"), "{retrying}");
    }

    #[test]
    fn a_cut_off_roster_says_so() {
        let mut r = roster(crew(3));
        assert_eq!(roster_status(&Load::Ready(r.clone()), false), "");
        r.truncated = true;
        assert_eq!(
            roster_status(&Load::Ready(r), false),
            "Showing the first 3 members."
        );
    }

    #[test]
    fn roster_requests_ask_for_full_pages_and_escape_the_filter() {
        assert_eq!(roster_path(None, None), "/api/public/members?limit=100");
        let path = roster_path(Some("Soldier: 76"), Some("a b"));
        assert!(
            path.starts_with("/api/public/members?limit=100&hero=Soldier"),
            "{path}"
        );
        assert!(path.contains("&cursor="), "{path}");
        assert!(!path.contains(' '), "{path}");
    }

    #[test]
    fn page_css_uses_tokens_and_the_frame_grid() {
        assert_sizes_from_tokens(PAGE_CSS);
        let grid = css_rule(PAGE_CSS, ".member-grid {");
        assert!(
            has_decl(&grid, "grid-template-columns: repeat(4, minmax(0, 1fr))"),
            "{grid}"
        );
        assert!(has_decl(&grid, "gap: var(--space-3)"), "{grid}");
        let phone = PAGE_CSS
            .split_once("@media (max-width: 820px) {")
            .unwrap()
            .1;
        assert!(
            has_decl(
                &css_rule(phone, ".member-grid {"),
                "grid-template-columns: repeat(2, minmax(0, 1fr))"
            ),
            "{phone}"
        );
        let card = css_rule(PAGE_CSS, ".member-card {");
        for decl in [
            "padding: var(--card-padding)",
            "background: var(--card-bg)",
            "border: 1px solid var(--card-border)",
            "border-radius: var(--card-radius)",
        ] {
            assert!(has_decl(&card, decl), "{decl}: {card}");
        }
        let select = css_rule(PAGE_CSS, ".members-hero .ui-field {");
        assert!(
            has_decl(&select, "min-height: var(--hit-target-min)"),
            "{select}"
        );
        assert!(
            has_decl(&select, "border-color: var(--border-strong)"),
            "{select}"
        );
        assert!(has_decl(&select, "background: var(--surface)"), "{select}");
        let label = css_rule(PAGE_CSS, ".members-hero .field-label {");
        assert!(has_decl(&label, "font-size: var(--text-md)"), "{label}");
        let pager = css_rule(PAGE_CSS, ".members-page .pager-target > .pager {");
        assert!(
            has_decl(
                &pager,
                "margin-top: calc(var(--space-section) - var(--space-stack))"
            ),
            "{pager}"
        );
        // The role line keeps the frame's paragraph margin.
        assert!(
            has_decl(
                &css_rule(PAGE_CSS, ".member-role {"),
                "margin-block: var(--space-3)"
            ),
            "{PAGE_CSS}"
        );
        // The status line is the shared list one, not a fork.
        assert!(!PAGE_CSS.contains("-status"), "{PAGE_CSS}");
        assert!(!PAGE_CSS.contains('\u{2014}') && !PAGE_CSS.contains('\u{2013}'));
    }
}
