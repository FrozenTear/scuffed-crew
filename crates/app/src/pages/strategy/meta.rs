use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;

// --- Types ---

#[derive(Debug, Clone, Deserialize)]
struct MetaResponse {
    updated: String,
    source: String,
    heroes: Vec<HeroMeta>,
}

#[derive(Debug, Clone, Deserialize)]
struct HeroMeta {
    id: String,
    name: String,
    role: String,
    portrait_url: String,
    pickrate: f64,
    winrate: f64,
}

// --- Sort state ---

#[derive(Debug, Clone, Copy, PartialEq)]
enum SortColumn {
    Name,
    Role,
    Pickrate,
    Winrate,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    fn toggle(self) -> Self {
        match self {
            SortDir::Asc => SortDir::Desc,
            SortDir::Desc => SortDir::Asc,
        }
    }

    fn indicator(self) -> &'static str {
        match self {
            SortDir::Asc => "\u{25b2}",
            SortDir::Desc => "\u{25bc}",
        }
    }
}

// --- Role helpers ---

fn role_color(role: &str) -> &'static str {
    match role {
        "Tank" | "tank" => "#3b82f6",
        "Damage" | "damage" => "#ef4444",
        "Support" | "support" => "#22c55e",
        _ => "#94a3b8",
    }
}

fn role_bg(role: &str) -> &'static str {
    match role {
        "Tank" | "tank" => "rgba(59, 130, 246, 0.12)",
        "Damage" | "damage" => "rgba(239, 68, 68, 0.12)",
        "Support" | "support" => "rgba(34, 197, 94, 0.12)",
        _ => "rgba(148, 163, 184, 0.12)",
    }
}

fn winrate_bar_color(wr: f64) -> &'static str {
    if wr >= 52.0 {
        "#22c55e"
    } else if wr <= 48.0 {
        "#ef4444"
    } else {
        "#94a3b8"
    }
}

fn canonical_role(role: &str) -> &str {
    match role.to_lowercase().as_str() {
        "tank" => "Tank",
        "damage" => "Damage",
        "support" => "Support",
        _ => role,
    }
}

// --- Sorting ---

fn sort_heroes(heroes: &[HeroMeta], col: SortColumn, dir: SortDir) -> Vec<HeroMeta> {
    let mut sorted = heroes.to_vec();
    sorted.sort_by(|a, b| {
        let ord = match col {
            SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortColumn::Role => a.role.to_lowercase().cmp(&b.role.to_lowercase()),
            SortColumn::Pickrate => a.pickrate.partial_cmp(&b.pickrate).unwrap_or(std::cmp::Ordering::Equal),
            SortColumn::Winrate => a.winrate.partial_cmp(&b.winrate).unwrap_or(std::cmp::Ordering::Equal),
        };
        match dir {
            SortDir::Asc => ord,
            SortDir::Desc => ord.reverse(),
        }
    });
    sorted
}

// --- Role summary stats ---

struct RoleSummary {
    role: String,
    count: usize,
    avg_pickrate: f64,
    avg_winrate: f64,
}

fn compute_role_summaries(heroes: &[HeroMeta]) -> Vec<RoleSummary> {
    let roles = ["Tank", "Damage", "Support"];
    roles
        .iter()
        .map(|&r| {
            let matching: Vec<&HeroMeta> = heroes
                .iter()
                .filter(|h| canonical_role(&h.role) == r)
                .collect();
            let count = matching.len();
            let (avg_pr, avg_wr) = if count > 0 {
                let pr: f64 = matching.iter().map(|h| h.pickrate).sum::<f64>() / count as f64;
                let wr: f64 = matching.iter().map(|h| h.winrate).sum::<f64>() / count as f64;
                (pr, wr)
            } else {
                (0.0, 0.0)
            };
            RoleSummary {
                role: r.to_string(),
                count,
                avg_pickrate: avg_pr,
                avg_winrate: avg_wr,
            }
        })
        .collect()
}

// --- CSS ---

const PAGE_CSS: &str = r#"
    .meta-page {
        padding: 2rem;
        max-width: 1100px;
        margin: 0 auto;
    }
    .meta-page-title {
        font-family: var(--font-display-hero);
        font-size: 2.2rem;
        color: var(--text-bright);
        letter-spacing: 2px;
        text-transform: uppercase;
        margin: 0 0 1.5rem;
    }
    .meta-role-cards {
        display: grid;
        grid-template-columns: repeat(3, 1fr);
        gap: 1rem;
        margin-bottom: 1.5rem;
    }
    .meta-role-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 1rem 1.25rem;
        display: flex;
        flex-direction: column;
        gap: 0.35rem;
    }
    .meta-role-card-title {
        font-family: var(--font-display);
        font-weight: 700;
        font-size: 1rem;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .meta-role-card-stat {
        font-size: 0.8rem;
        color: var(--text-secondary);
    }
    .meta-role-card-stat strong {
        color: var(--text-bright);
        font-weight: 600;
    }
    .meta-filters {
        display: flex;
        gap: 0.5rem;
        margin-bottom: 1.25rem;
        flex-wrap: wrap;
    }
    .meta-chip {
        padding: 0.3rem 0.75rem;
        border-radius: 999px;
        font-size: 0.78rem;
        font-weight: 600;
        border: 1px solid var(--border);
        background: var(--bg-card);
        color: var(--text-secondary);
        cursor: pointer;
        transition: all 0.15s;
        text-transform: uppercase;
        letter-spacing: 0.03em;
    }
    .meta-chip:hover {
        border-color: var(--border-light);
        color: var(--text-primary);
    }
    .meta-chip.active {
        border-color: var(--accent);
        background: var(--accent-soft);
        color: var(--accent-bright);
    }
    .meta-table-wrap {
        overflow-x: auto;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--bg-card);
    }
    .meta-table {
        width: 100%;
        border-collapse: collapse;
        font-size: 0.85rem;
    }
    .meta-table th {
        padding: 0.65rem 0.75rem;
        text-align: left;
        font-family: var(--font-display);
        font-weight: 700;
        font-size: 0.75rem;
        text-transform: uppercase;
        letter-spacing: 0.06em;
        color: var(--text-muted);
        border-bottom: 1px solid var(--border);
        cursor: pointer;
        user-select: none;
        white-space: nowrap;
        transition: color 0.15s;
    }
    .meta-table th:hover {
        color: var(--text-secondary);
    }
    .meta-table th.sorted {
        color: var(--accent-bright);
    }
    .meta-table th .sort-ind {
        margin-left: 0.3rem;
        font-size: 0.65rem;
    }
    .meta-table td {
        padding: 0.5rem 0.75rem;
        border-bottom: 1px solid var(--border);
        vertical-align: middle;
    }
    .meta-table tr:last-child td {
        border-bottom: none;
    }
    .meta-table tr:hover td {
        background: var(--bg-card-alt);
    }
    .meta-hero-cell {
        display: flex;
        align-items: center;
        gap: 0.6rem;
    }
    .meta-hero-portrait {
        width: 28px;
        height: 28px;
        border-radius: 4px;
        object-fit: cover;
        background: var(--bg-surface);
        flex-shrink: 0;
    }
    .meta-hero-name {
        color: var(--text-bright);
        font-weight: 500;
    }
    .meta-role-pill {
        display: inline-block;
        font-size: 0.65rem;
        padding: 0.1rem 0.5rem;
        border-radius: 999px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.03em;
    }
    .meta-bar-cell {
        display: flex;
        align-items: center;
        gap: 0.5rem;
    }
    .meta-bar-value {
        min-width: 42px;
        text-align: right;
        font-family: var(--font-mono);
        font-size: 0.8rem;
        color: var(--text-primary);
    }
    .meta-bar-track {
        flex: 1;
        height: 6px;
        background: var(--bg-surface);
        border-radius: 3px;
        overflow: hidden;
        min-width: 60px;
    }
    .meta-bar-fill {
        height: 100%;
        border-radius: 3px;
        transition: width 0.3s ease;
    }
    .meta-footer {
        margin-top: 1rem;
        font-size: 0.75rem;
        color: var(--text-muted);
        text-align: right;
    }
    .meta-loading, .meta-empty {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem 0;
    }
    @media (max-width: 640px) {
        .meta-role-cards {
            grid-template-columns: 1fr;
        }
    }
"#;

// --- Component ---

#[component]
pub fn StrategyMeta() -> Element {
    let meta_data = use_resource(|| async {
        ApiClient::web()
            .fetch::<MetaResponse>("/api/strategy/meta")
            .await
            .ok()
    });

    let mut role_filter = use_signal(|| "All".to_string());
    let mut sort_col = use_signal(|| SortColumn::Pickrate);
    let mut sort_dir = use_signal(|| SortDir::Desc);

    rsx! {
        style { {PAGE_CSS} }

        div { class: "meta-page",
            h1 { class: "meta-page-title", "Competitive Meta" }

            {
                let data = meta_data.read();
                let data = data.as_ref().and_then(|d| d.as_ref());

                match data {
                    None => rsx! { p { class: "meta-loading", "Loading meta data..." } },
                    Some(resp) if resp.heroes.is_empty() => rsx! {
                        p { class: "meta-empty", "No hero data available." }
                    },
                    Some(resp) => {
                        let summaries = compute_role_summaries(&resp.heroes);
                        let current_filter = (role_filter)();
                        let filtered: Vec<HeroMeta> = if current_filter == "All" {
                            resp.heroes.clone()
                        } else {
                            resp.heroes
                                .iter()
                                .filter(|h| canonical_role(&h.role) == current_filter)
                                .cloned()
                                .collect()
                        };
                        let sorted = sort_heroes(&filtered, (sort_col)(), (sort_dir)());
                        let active_col = (sort_col)();
                        let active_dir = (sort_dir)();
                        let updated = resp.updated.clone();
                        let source = resp.source.clone();

                        rsx! {
                            // Role summary cards
                            div { class: "meta-role-cards",
                                for s in summaries.iter() {
                                    {render_role_card(s)}
                                }
                            }

                            // Filter chips
                            div { class: "meta-filters",
                                for label in ["All", "Tank", "Damage", "Support"] {
                                    {render_filter_chip(label, &current_filter, &mut role_filter)}
                                }
                            }

                            if sorted.is_empty() {
                                p { class: "meta-empty", "No heroes match the selected filter." }
                            } else {
                                // Table
                                div { class: "meta-table-wrap",
                                    table { class: "meta-table",
                                        thead {
                                            tr {
                                                {render_th("Name", SortColumn::Name, active_col, active_dir, sort_col, sort_dir)}
                                                {render_th("Role", SortColumn::Role, active_col, active_dir, sort_col, sort_dir)}
                                                {render_th("Pickrate", SortColumn::Pickrate, active_col, active_dir, sort_col, sort_dir)}
                                                {render_th("Winrate", SortColumn::Winrate, active_col, active_dir, sort_col, sort_dir)}
                                            }
                                        }
                                        tbody {
                                            for hero in sorted.iter() {
                                                {render_hero_row(hero)}
                                            }
                                        }
                                    }
                                }
                            }

                            // Footer attribution
                            p { class: "meta-footer",
                                "Data from {source} \u{2014} Updated {updated}"
                            }
                        }
                    }
                }
            }
        }
    }
}

// --- Render helpers ---

fn render_role_card(s: &RoleSummary) -> Element {
    let color = role_color(&s.role);
    let border_style = format!("border-left: 3px solid {color};");

    rsx! {
        div { class: "meta-role-card", style: "{border_style}",
            span {
                class: "meta-role-card-title",
                style: "color: {color};",
                "{s.role}"
            }
            span { class: "meta-role-card-stat",
                strong { "{s.count}" }
                " heroes"
            }
            span { class: "meta-role-card-stat",
                "Avg pick "
                strong { "{s.avg_pickrate:.1}%" }
                " \u{00b7} Win "
                strong { "{s.avg_winrate:.1}%" }
            }
        }
    }
}

fn render_filter_chip(label: &str, current: &str, signal: &mut Signal<String>) -> Element {
    let is_active = label == current;
    let class = if is_active { "meta-chip active" } else { "meta-chip" };
    let label_owned = label.to_string();
    let mut sig = *signal;

    rsx! {
        button {
            class: "{class}",
            onclick: move |_| sig.set(label_owned.clone()),
            "{label}"
        }
    }
}

fn render_th(
    label: &str,
    col: SortColumn,
    active_col: SortColumn,
    active_dir: SortDir,
    mut sort_col_sig: Signal<SortColumn>,
    mut sort_dir_sig: Signal<SortDir>,
) -> Element {
    let is_sorted = col == active_col;
    let class = if is_sorted { "sorted" } else { "" };

    rsx! {
        th {
            class: "{class}",
            onclick: move |_| {
                if (sort_col_sig)() == col {
                    sort_dir_sig.set((sort_dir_sig)().toggle());
                } else {
                    sort_col_sig.set(col);
                    sort_dir_sig.set(SortDir::Desc);
                }
            },
            "{label}"
            if is_sorted {
                span { class: "sort-ind", "{active_dir.indicator()}" }
            }
        }
    }
}

fn render_hero_row(hero: &HeroMeta) -> Element {
    let role_display = canonical_role(&hero.role);
    let color = role_color(&hero.role);
    let bg = role_bg(&hero.role);
    let wr_color = winrate_bar_color(hero.winrate);

    // Pickrate bar: scale so 25% = full width
    let pr_width = format!("{:.1}%", (hero.pickrate / 25.0 * 100.0).min(100.0));
    // Winrate bar: scale 40-60% range to 0-100% width
    let wr_width = format!("{:.1}%", ((hero.winrate - 40.0) / 20.0 * 100.0).clamp(0.0, 100.0));

    rsx! {
        tr {
            td {
                div { class: "meta-hero-cell",
                    img {
                        class: "meta-hero-portrait",
                        src: "{hero.portrait_url}",
                        alt: "{hero.name}",
                    }
                    span { class: "meta-hero-name", "{hero.name}" }
                }
            }
            td {
                span {
                    class: "meta-role-pill",
                    style: "color: {color}; background: {bg};",
                    "{role_display}"
                }
            }
            td {
                div { class: "meta-bar-cell",
                    span { class: "meta-bar-value", "{hero.pickrate:.1}%" }
                    div { class: "meta-bar-track",
                        div {
                            class: "meta-bar-fill",
                            style: "width: {pr_width}; background: var(--accent);",
                        }
                    }
                }
            }
            td {
                div { class: "meta-bar-cell",
                    span { class: "meta-bar-value", "{hero.winrate:.1}%" }
                    div { class: "meta-bar-track",
                        div {
                            class: "meta-bar-fill",
                            style: "width: {wr_width}; background: {wr_color};",
                        }
                    }
                }
            }
        }
    }
}
