mod heroes;
mod history;
mod maps;
mod overview;

use dioxus::prelude::*;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use scuffed_types::api::{MemberSettingsResponse, UpdateMemberSettingsRequest};

use crate::components::{Toast, use_toast};
use crate::hooks::{use_api, use_api_with};
use crate::routes::Route;

// -- Shared data models (fetched once here, consumed by the tab modules) --

#[derive(Debug, Clone, Deserialize)]
struct PersonalStats {
    #[allow(dead_code)]
    member_id: String,
    total_matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct HeroStats {
    hero: String,
    matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
    avg_elims: f64,
    avg_deaths: f64,
    avg_damage: f64,
    avg_healing: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct MapStats {
    map_name: String,
    matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct PersonalMatch {
    #[allow(dead_code)]
    id: String,
    hero: String,
    map_name: String,
    #[allow(dead_code)]
    game_mode: String,
    role: String,
    outcome: String,
    elims: u32,
    deaths: u32,
    assists: u32,
    damage: u32,
    healing: u32,
    played_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
struct MatchPage {
    data: Vec<PersonalMatch>,
    next_cursor: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum StatsTab {
    Overview,
    Heroes,
    Maps,
    History,
}

// -- Shared helpers --

fn winrate_pct(wins: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        (wins as f64 / total as f64) * 100.0
    }
}

fn winrate_class(pct: f64) -> &'static str {
    if pct >= 55.0 {
        "stats-winrate high"
    } else if pct >= 45.0 {
        "stats-winrate mid"
    } else {
        "stats-winrate low"
    }
}

fn format_date(dt: &DateTime<Utc>) -> String {
    dt.format("%b %d, %Y").to_string()
}

/// Shared map → game mode (used by maps tab + overview mode chips).
pub(super) fn map_game_mode(name: &str) -> &'static str {
    match name {
        "Circuit Royal"
        | "Dorado"
        | "Havana"
        | "Junkertown"
        | "Rialto"
        | "Route 66"
        | "Shambali Monastery"
        | "Watchpoint: Gibraltar" => "Escort",
        "Blizzard World" | "Eichenwalde" | "Hollywood" | "King's Row" | "Midtown" | "Numbani"
        | "Paraíso" => "Hybrid",
        "Antarctic Peninsula"
        | "Busan"
        | "Ilios"
        | "Lijiang Tower"
        | "Nepal"
        | "Oasis"
        | "Samoa" => "Control",
        "Colosseo" | "Esperança" | "New Queen Street" | "Runasapi" => "Push",
        "Aatlis" | "New Junk City" | "Suravasa" => "Flashpoint",
        "Hanaoka" | "Throne of Anubis" => "Clash",
        _ => "Other",
    }
}

fn initial_density() -> &'static str {
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window()
            && let Ok(Some(storage)) = win.local_storage()
            && let Ok(Some(v)) = storage.get_item("stats-ui-density")
        {
            if v == "comfortable" {
                return "comfortable";
            }
        }
    }
    "compact"
}

fn persist_density(d: &str) {
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window()
            && let Ok(Some(storage)) = win.local_storage()
        {
            let _ = storage.set_item("stats-ui-density", d);
        }
    }
    #[cfg(not(feature = "web"))]
    let _ = d;
}

/// Fetch-failed state with a retry button (bumps the resource's refresh counter).
fn load_error_state(what: &'static str, mut refresh: Signal<u64>) -> Element {
    rsx! {
        div { class: "stats-load-error",
            p { "Couldn't load {what}." }
            button { onclick: move |_| refresh += 1, "Retry" }
        }
    }
}

const STATS_CSS: &str = r#"
    .stats-page {
        max-width: 1100px;
        margin: 0 auto;
        padding: 2rem 1.5rem;
    }
    .stats-page h1 {
        font-family: var(--font-head);
        font-size: 1.8rem;
        color: var(--text);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        margin-bottom: 0.5rem;
    }
    .stats-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        flex-wrap: wrap;
        gap: 1rem;
        margin-bottom: 1.5rem;
    }
    .stats-header-actions {
        display: flex;
        gap: 0.5rem;
    }
    .stats-header-actions a, .stats-header-actions button {
        padding: 0.4rem 1rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        font-size: 0.8rem;
        cursor: pointer;
        transition: all 0.15s;
        text-decoration: none;
    }
    .stats-header-actions a:hover, .stats-header-actions button:hover {
        color: var(--text);
        border-color: var(--accent-soft);
    }
    .stats-tabs {
        display: flex;
        gap: 0.25rem;
        margin-bottom: 1.5rem;
        border-bottom: 1px solid var(--border);
        padding-bottom: 0;
    }
    .stats-tab {
        padding: 0.6rem 1.2rem;
        border: none;
        background: none;
        color: var(--text-2);
        font-family: var(--font-head);
        font-size: 0.9rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.03em;
        cursor: pointer;
        border-bottom: 2px solid transparent;
        margin-bottom: -1px;
        transition: color 0.15s, border-color 0.15s;
    }
    .stats-tab:hover {
        color: var(--text);
    }
    .stats-tab.active {
        color: var(--accent);
        border-bottom-color: var(--accent);
    }
    .stats-winrate {
        display: inline-block;
        padding: 0.1rem 0.4rem;
        border-radius: 4px;
        font-size: 0.8rem;
        font-weight: 600;
    }
    .stats-winrate.high { color: var(--ok); }
    .stats-winrate.mid { color: var(--warn); }
    .stats-winrate.low { color: var(--danger); }
    .outcome-win { color: var(--ok); font-weight: 600; }
    .outcome-loss { color: var(--danger); font-weight: 600; }
    .outcome-draw { color: var(--warn); font-weight: 600; }
    .match-card {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 0.75rem 1rem;
        display: grid;
        grid-template-columns: 80px 1fr 1fr auto;
        gap: 1rem;
        align-items: center;
        font-size: 0.85rem;
        transition: background 0.15s;
    }
    .match-card:hover { background: var(--surface-2); }
    .match-cards { display: flex; flex-direction: column; gap: 0.5rem; }
    .match-card .match-outcome {
        font-family: var(--font-head);
        font-size: 1rem;
        font-weight: 700;
        text-transform: uppercase;
    }
    .match-card .match-hero { color: var(--text); font-weight: 500; }
    .match-card .match-map { color: var(--text-2); font-size: 0.8rem; }
    .match-card .match-scoreline { color: var(--text-3); font-size: 0.8rem; }
    .match-card .match-date { color: var(--text-3); font-size: 0.75rem; text-align: right; }
    .stats-pagination {
        display: flex;
        justify-content: center;
        gap: 0.75rem;
        margin-top: 1.5rem;
    }
    .stats-pagination button {
        padding: 0.4rem 1rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        font-size: 0.8rem;
        cursor: pointer;
        transition: all 0.15s;
    }
    .stats-pagination button:hover:not(:disabled) {
        color: var(--text);
        border-color: var(--accent-soft);
    }
    .stats-pagination button:disabled { opacity: 0.4; cursor: not-allowed; }

    /* Summary: win-rate lead tile + secondary tiles */
    .stats-summary {
        display: grid;
        grid-template-columns: minmax(220px, 2fr) minmax(140px, 1fr);
        gap: 1rem;
        margin-bottom: 2rem;
    }
    .stat-tile {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
        text-align: center;
        display: flex;
        flex-direction: column;
        justify-content: center;
    }
    .stat-tile-value {
        font-family: var(--font-head);
        font-size: 1.9rem;
        color: var(--text);
        letter-spacing: 2px;
        line-height: 1;
    }
    .stat-tile-hero .stat-tile-value {
        font-size: 2.8rem;
        color: var(--accent);
    }
    .stat-tile-label {
        font-size: 0.75rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.05em;
        margin-top: 0.35rem;
    }
    .stat-tile-record {
        font-size: 0.9rem;
        color: var(--text-2);
        margin-top: 0.4rem;
    }

    /* Load error + retry */
    .stats-load-error {
        text-align: center;
        padding: 2rem 1rem;
        color: var(--text-2);
        font-size: 0.9rem;
    }
    .stats-load-error p { margin: 0 0 0.75rem; }
    .stats-load-error button {
        padding: 0.4rem 1rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        font-size: 0.8rem;
        cursor: pointer;
        transition: all 0.15s;
    }
    .stats-load-error button:hover {
        color: var(--text);
        border-color: var(--accent-soft);
    }

    /* Overview: role layout */
    .overview-grid {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 1.5rem;
        margin-top: 1rem;
    }
    .overview-section {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
    }
    .overview-section h3 {
        font-family: var(--font-head);
        font-size: 0.85rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        margin: 0 0 1rem;
    }
    .role-cards { display: flex; flex-direction: column; gap: 0.75rem; }
    .role-card {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        padding: 0.6rem 0.75rem;
        border-radius: 8px;
        background: var(--bg);
        border-left: 3px solid transparent;
    }
    .role-card-info { flex: 1; }
    .role-card-name {
        font-weight: 600;
        font-size: 0.85rem;
        color: var(--text);
    }
    .role-card-sub {
        font-size: 0.75rem;
        color: var(--text-3);
        margin-top: 0.15rem;
    }
    .role-card-wr {
        font-family: var(--font-head);
        font-size: 1.1rem;
        font-weight: 700;
    }
    .role-card-wr.high { color: var(--ok); }
    .role-card-wr.mid { color: var(--warn); }
    .role-card-wr.low { color: var(--danger); }

    /* Heroes: chart + table */
    .heroes-chart-section {
        margin-bottom: 1.5rem;
    }
    .section-title {
        font-family: var(--font-head);
        font-size: 0.8rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        margin-bottom: 0.75rem;
    }

    /* Maps: game mode groups */
    .map-mode-group { margin-bottom: 1.5rem; }
    .map-mode-header {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin-bottom: 0.6rem;
        padding-bottom: 0.4rem;
        border-bottom: 1px solid var(--border);
    }
    .map-mode-name {
        font-family: var(--font-head);
        font-size: 0.85rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .map-mode-agg {
        font-size: 0.75rem;
        color: var(--text-3);
        margin-left: auto;
    }

    @media (max-width: 768px) {
        .overview-grid { grid-template-columns: 1fr; }
    }
    @media (max-width: 640px) {
        .stats-summary { grid-template-columns: 1fr; }
        .match-card {
            grid-template-columns: 60px 1fr;
            gap: 0.5rem;
        }
        .match-card .match-scoreline,
        .match-card .match-date {
            grid-column: 1 / -1;
        }
    }

    /* Slim tracker row (shown once matches exist) */
    .tracker-slim {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        flex-wrap: wrap;
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 0.6rem 1rem;
        margin-bottom: 1.5rem;
        font-size: 0.85rem;
    }
    .tracker-slim-label {
        font-family: var(--font-head);
        font-size: 0.8rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.06em;
    }
    .tracker-slim-sep { color: var(--text-3); }
    .tracker-slim-note { color: var(--text-2); }
    .tracker-slim-actions {
        margin-left: auto;
        display: flex;
        gap: 0.5rem;
    }
    .tracker-slim-actions a, .tracker-slim-actions button {
        padding: 0.3rem 0.8rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        font-size: 0.8rem;
        cursor: pointer;
        transition: all 0.15s;
        text-decoration: none;
    }
    .tracker-slim-actions a:hover, .tracker-slim-actions button:hover {
        color: var(--text);
        border-color: var(--accent-soft);
    }

    /* Daemon settings panel */
    .daemon-settings {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
        margin-bottom: 1.5rem;
    }
    .daemon-settings summary {
        font-family: var(--font-head);
        font-size: 0.85rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        cursor: pointer;
        user-select: none;
        list-style: none;
        display: flex;
        align-items: center;
        gap: 0.5rem;
    }
    .daemon-settings summary::-webkit-details-marker { display: none; }
    .daemon-settings[open] summary { margin-bottom: 1rem; }
    .daemon-settings-row {
        display: flex;
        gap: 0.75rem;
        align-items: center;
        flex-wrap: wrap;
    }
    .daemon-settings-label {
        font-size: 0.8rem;
        color: var(--text-2);
        white-space: nowrap;
    }
    .daemon-settings-input {
        flex: 1;
        min-width: 160px;
        padding: 0.4rem 0.75rem;
        background: var(--bg);
        border: 1px solid var(--border);
        border-radius: 6px;
        color: var(--text);
        font-size: 0.85rem;
        font-family: var(--font-mono);
    }
    .daemon-settings-input:focus {
        outline: none;
        border-color: var(--accent-soft);
    }
    .daemon-settings-hint {
        font-size: 0.75rem;
        color: var(--text-3);
        margin-top: 0.5rem;
    }

    /* Tracker download / install panel */
    .tracker-download {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.25rem;
        margin-bottom: 1.5rem;
    }
    .tracker-download-head {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        flex-wrap: wrap;
        margin-bottom: 0.75rem;
    }
    .tracker-download-head h3 {
        font-family: var(--font-head);
        font-size: 0.85rem;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        margin: 0;
    }
    .tracker-badge {
        display: inline-block;
        padding: 0.15rem 0.55rem;
        border-radius: 4px;
        background: var(--surface-2);
        border: 1px solid var(--warn);
        color: var(--warn);
        font-size: 0.7rem;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .tracker-download p {
        font-size: 0.85rem;
        color: var(--text-2);
        margin: 0 0 0.75rem;
        line-height: 1.5;
    }
    .tracker-download code.tracker-install {
        display: block;
        background: var(--bg);
        border: 1px solid var(--border);
        border-radius: 4px;
        padding: 0.6rem 0.75rem;
        font-family: var(--font-mono);
        font-size: 0.8rem;
        color: var(--accent);
        word-break: break-all;
        margin: 0 0 0.75rem;
        user-select: all;
    }
    .tracker-download-links {
        display: flex;
        gap: 0.5rem;
        flex-wrap: wrap;
    }
    .tracker-download-links a {
        padding: 0.4rem 1rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        font-size: 0.8rem;
        text-decoration: none;
        transition: all 0.15s;
    }
    .tracker-download-links a:hover {
        color: var(--text);
        border-color: var(--accent-soft);
    }

    /* W3: filters, density, overview extras */
    .stats-filters {
        display: flex;
        flex-wrap: wrap;
        gap: 1rem 1.5rem;
        margin-bottom: 1.25rem;
        align-items: center;
    }
    .filter-group {
        display: flex;
        flex-wrap: wrap;
        gap: 0.4rem;
        align-items: center;
    }
    .filter-label {
        font-size: 0.7rem;
        text-transform: uppercase;
        letter-spacing: 0.05em;
        color: var(--text-3);
        margin-right: 0.25rem;
    }
    .filter-chip {
        padding: 0.25rem 0.65rem;
        border-radius: 999px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        font-size: 0.75rem;
        cursor: pointer;
    }
    .filter-chip.active {
        border-color: var(--accent);
        color: var(--text);
        background: color-mix(in srgb, var(--accent) 12%, var(--bg));
    }
    .filter-hint {
        font-size: 0.7rem;
        color: var(--text-3);
        margin: 0;
        width: 100%;
    }
    .stats-row-muted { opacity: 0.55; }
    .density-toggle {
        font-size: 0.75rem;
        padding: 0.3rem 0.65rem;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--bg);
        color: var(--text-2);
        cursor: pointer;
    }
    .density-toggle:hover { color: var(--text); border-color: var(--accent-soft); }
    .stats-page[data-density="comfortable"] .overview-grid { gap: 1.75rem; }
    .stats-page[data-density="comfortable"] .match-card { padding: 1rem 1.15rem; }
    .stats-page[data-density="compact"] .overview-grid { gap: 1rem; }
    .overview-form { margin-top: 1rem; }
    .mini-hero-list { display: flex; flex-direction: column; gap: 0.45rem; }
    .mini-hero-row {
        display: grid;
        grid-template-columns: 1fr auto auto;
        gap: 0.75rem;
        font-size: 0.85rem;
        padding: 0.35rem 0;
        border-bottom: 1px solid var(--border);
    }
    .mini-hero-name { color: var(--text); font-weight: 500; }
    .mini-hero-meta { color: var(--text-3); }
    .mini-hero-wr { color: var(--accent); font-weight: 600; font-variant-numeric: tabular-nums; }
    .mode-chips { display: flex; flex-wrap: wrap; gap: 0.5rem; }
    .mode-chip {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        padding: 0.55rem 0.75rem;
        border-radius: 8px;
        border: 1px solid var(--border);
        background: var(--bg);
        min-width: 4.5rem;
    }
    .mode-chip-name { font-size: 0.7rem; color: var(--text-3); text-transform: uppercase; }
    .mode-chip-wr { font-size: 1.1rem; font-weight: 700; color: var(--text); font-variant-numeric: tabular-nums; }
    .mode-chip-n { font-size: 0.7rem; color: var(--text-3); }
    .form-strip { display: flex; flex-wrap: wrap; gap: 0.35rem; }
    .form-chip {
        width: 1.6rem; height: 1.6rem;
        display: inline-flex; align-items: center; justify-content: center;
        border-radius: 4px;
        font-size: 0.7rem; font-weight: 700;
    }
    .form-chip.win { background: color-mix(in srgb, var(--ok) 25%, transparent); color: var(--ok); }
    .form-chip.loss { background: color-mix(in srgb, var(--danger) 25%, transparent); color: var(--danger); }
    .form-chip.draw { background: color-mix(in srgb, var(--text-3) 20%, transparent); color: var(--text-3); }
    .map-callouts { display: flex; flex-wrap: wrap; gap: 0.75rem; margin-bottom: 1.25rem; }
    .map-callout {
        display: flex; flex-wrap: wrap; gap: 0.5rem 0.75rem; align-items: baseline;
        padding: 0.6rem 0.9rem; border-radius: 8px; border: 1px solid var(--border);
        background: var(--surface); font-size: 0.85rem;
    }
    .map-callout.best { border-left: 3px solid var(--ok); }
    .map-callout.worst { border-left: 3px solid var(--danger); }
    .map-callout-label { font-size: 0.7rem; text-transform: uppercase; color: var(--text-3); }
    .map-callout-name { font-weight: 600; color: var(--text); }
    .map-callout-meta { color: var(--text-2); font-variant-numeric: tabular-nums; }
"#;

#[component]
pub fn Stats() -> Element {
    let stats = use_api::<PersonalStats>("/api/stats/me");
    let heroes = use_api::<Vec<HeroStats>>("/api/stats/me/heroes");
    let maps = use_api::<Vec<MapStats>>("/api/stats/me/maps");
    let server_settings = use_api::<MemberSettingsResponse>("/api/stats/settings");

    let mut tab = use_signal(|| StatsTab::Overview);
    let mut toast = use_toast();

    // Local editable state for the player_name field
    let mut player_name_input: Signal<String> = use_signal(String::new);
    let mut settings_saving = use_signal(|| false);

    // Populate input once settings load (only on first load, not on every re-render)
    let mut settings_loaded = use_signal(|| false);
    {
        let data = server_settings.data.read();
        if !settings_loaded() {
            let s = data.as_ref().and_then(|d| d.as_ref());
            if let Some(s) = s {
                let name = s.player_name.clone().unwrap_or_default();
                drop(data);
                player_name_input.set(name);
                settings_loaded.set(true);
            }
        }
    }

    let save_settings = move |_| {
        let name = player_name_input().trim().to_string();
        settings_saving.set(true);
        spawn(async move {
            let body = UpdateMemberSettingsRequest {
                player_name: if name.is_empty() { None } else { Some(name) },
            };
            match ApiClient::web()
                .put_json::<_, MemberSettingsResponse>("/api/stats/settings", &body)
                .await
            {
                Ok(_) => toast.show(Toast::success("Settings saved.")),
                Err(e) => toast.show(Toast::error(format!("Save failed: {e}"))),
            }
            settings_saving.set(false);
        });
    };

    let mut page_cursor = use_signal(|| Option::<String>::None);
    let mut cursor_history: Signal<Vec<Option<String>>> = use_signal(|| vec![None]);

    let matches = use_api_with::<MatchPage>(move || {
        let cursor = page_cursor();
        match cursor {
            Some(c) => format!("/api/stats/me/matches?limit=25&cursor={c}"),
            None => "/api/stats/me/matches?limit=25".to_string(),
        }
    });
    // Overview form strip — same endpoint, limit=10, no new backend (Q3).
    let form_matches = use_api::<MatchPage>("/api/stats/me/matches?limit=10");

    let mut density = use_signal(initial_density);
    let hero_role = use_signal(|| "All");
    let hero_sort = use_signal(|| "matches");
    let hist_outcome = use_signal(|| "all");
    let hist_role = use_signal(|| "all");

    // Progressive disclosure: once the member has tracked matches, the setup
    // chrome collapses to one slim row. Zero-match members keep the full
    // onboarding cards — that's their primary UI. While the summary is still
    // loading (or failed) we show the slim row rather than flashing onboarding.
    let has_matches = stats
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(|s| s.total_matches > 0);
    let slim_tracker = has_matches != Some(false);
    let mut setup_open = use_signal(|| false);
    let show_full_setup = !slim_tracker || setup_open();

    let tab_body = match tab() {
        StatsTab::Overview => overview::overview_tab(heroes, maps, form_matches),
        StatsTab::Heroes => heroes::heroes_tab(heroes, hero_role, hero_sort),
        StatsTab::Maps => maps::maps_tab(maps),
        StatsTab::History => history::history_tab(
            matches,
            page_cursor,
            cursor_history,
            hist_outcome,
            hist_role,
        ),
    };

    let dens = density();

    rsx! {
        style { {STATS_CSS} }
        style { {crate::styles::admin::CSS} }

        div { class: "stats-page", "data-density": dens,
            div { class: "stats-header",
                h1 { "My Stats" }
                div { class: "stats-header-actions",
                    button {
                        class: "density-toggle",
                        title: "Toggle compact / comfortable density",
                        onclick: move |_| {
                            let next = if density() == "compact" {
                                "comfortable"
                            } else {
                                "compact"
                            };
                            density.set(next);
                            persist_density(next);
                        },
                        if dens == "compact" { "Density: Compact" } else { "Density: Comfortable" }
                    }
                    Link { to: Route::StatsTokens {}, "Daemon Tokens" }
                }
            }

            if slim_tracker {
                div { class: "tracker-slim",
                    span { class: "tracker-slim-label", "Tracker" }
                    span { class: "tracker-slim-sep", "·" }
                    span { class: "tracker-slim-note", "Linux only" }
                    div { class: "tracker-slim-actions",
                        button {
                            onclick: move |_| setup_open.toggle(),
                            if setup_open() { "Hide instructions" } else { "Reinstall instructions" }
                        }
                        Link { to: Route::StatsTokens {}, "Daemon Tokens" }
                    }
                }
            }

            if show_full_setup {
                // Daemon settings (collapsible)
                details { class: "daemon-settings",
                    summary { "⚙ Daemon Settings" }
                    div { class: "daemon-settings-row",
                        label { class: "daemon-settings-label", r#for: "player-name-input", "In-game name" }
                        input {
                            id: "player-name-input",
                            class: "daemon-settings-input",
                            r#type: "text",
                            placeholder: "e.g. FROZEN",
                            value: player_name_input(),
                            oninput: move |e| player_name_input.set(e.value()),
                        }
                        button {
                            class: "btn-primary",
                            disabled: settings_saving(),
                            onclick: save_settings,
                            if settings_saving() { "Saving…" } else { "Save" }
                        }
                    }
                    p { class: "daemon-settings-hint",
                        "The daemon uses this name to find your row on replay and post-match scoreboards."
                    }
                }

                // Tracker download / install
                div { class: "tracker-download",
                    div { class: "tracker-download-head",
                        h3 { "Get the Stat Tracker" }
                        span { class: "tracker-badge", "Linux only" }
                    }
                    p {
                        "The tracker auto-captures your Overwatch scoreboards via OCR and uploads your stats here. "
                        "It runs on Linux / Wayland only — there is no Windows or macOS build."
                    }
                    code { class: "tracker-install",
                        "curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash"
                    }
                    p {
                        "After installing, start the GUI, then paste this server's URL and a "
                        Link { to: Route::StatsTokens {}, "daemon token" }
                        " to begin uploading."
                    }
                    div { class: "tracker-download-links",
                        a {
                            href: "https://github.com/FrozenTear/scuffed-crew/releases/latest",
                            target: "_blank",
                            rel: "noopener noreferrer",
                            "Latest release"
                        }
                    }
                }
            }

            // Summary (always visible)
            {
                let err = stats.error.read().clone();
                let data = stats.data.read();
                let s = data.as_ref().and_then(|d| d.as_ref());
                match s {
                    None if err.is_some() => load_error_state("stats", stats.refresh),
                    None => rsx! { p { class: "loading-state", "Loading stats..." } },
                    Some(s) if s.total_matches == 0 => rsx! {
                        p { class: "empty-state",
                            "No matches tracked yet. Install the tracker above and play — your stats land here automatically."
                        }
                    },
                    Some(s) => {
                        let wr = winrate_pct(s.wins, s.total_matches);
                        let mut record = format!("{}W–{}L", s.wins, s.losses);
                        if s.draws > 0 {
                            record.push_str(&format!("–{}D", s.draws));
                        }
                        rsx! {
                            div { class: "stats-summary",
                                div { class: "stat-tile stat-tile-hero",
                                    div { class: "stat-tile-value", "{wr:.1}%" }
                                    div { class: "stat-tile-label", "Win Rate" }
                                    div { class: "stat-tile-record", "{record}" }
                                }
                                div { class: "stat-tile",
                                    div { class: "stat-tile-value", "{s.total_matches}" }
                                    div { class: "stat-tile-label", "Matches" }
                                }
                            }
                        }
                    }
                }
            }

            // Tabs
            div { class: "stats-tabs",
                button {
                    class: if tab() == StatsTab::Overview { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| tab.set(StatsTab::Overview),
                    "Overview"
                }
                button {
                    class: if tab() == StatsTab::Heroes { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| tab.set(StatsTab::Heroes),
                    "Heroes"
                }
                button {
                    class: if tab() == StatsTab::Maps { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| tab.set(StatsTab::Maps),
                    "Maps"
                }
                button {
                    class: if tab() == StatsTab::History { "stats-tab active" } else { "stats-tab" },
                    onclick: move |_| {
                        tab.set(StatsTab::History);
                        page_cursor.set(None);
                        cursor_history.set(vec![None]);
                    },
                    "History"
                }
            }

            // Tab content
            {tab_body}
        }
    }
}
