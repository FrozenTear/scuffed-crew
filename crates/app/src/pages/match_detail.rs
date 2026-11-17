//! Public match detail page: score board, map result, VOD and replay code
//! for a single fixture.

use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{
    EmptyCard, LoadError, PageHeader, PageShell, PageWidth, Pill, PillTone, SkeletonBars, btn_class,
};
use crate::routes::Route;
use crate::util::{StoredUrl, local_time_view, stored_http_url};

use super::article_author::{
    ARTICLE_CSS, ARTICLE_CSS_ID, ArticleCrumb, Load, is_retrying, load_status, short_date,
};
use super::public_fetch::{PublicFetch, fetch_public};
use super::{NotFoundLink, NotFoundPanel};

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct MatchDetailData {
    id: String,
    team_id: String,
    opponent: String,
    #[serde(default)]
    score_us: Option<u32>,
    #[serde(default)]
    score_them: Option<u32>,
    map_name: Option<String>,
    game_mode: Option<String>,
    match_type: String,
    #[serde(default)]
    played_at: Option<String>,
    #[serde(default)]
    scheduled_at: Option<String>,
    #[serde(default)]
    vod_url: Option<String>,
    #[serde(default)]
    replay_code: Option<String>,
    team_name: String,
    game_name: Option<String>,
}

/// A public fetch as the page views take it. `None` is the first load.
pub(super) fn load_of<T: Clone>(slot: Option<&PublicFetch<T>>) -> Load<T> {
    match slot {
        None => Load::Loading,
        Some(PublicFetch::Found(value)) => Load::Ready(value.clone()),
        Some(PublicFetch::NotFound) => Load::Missing,
        Some(PublicFetch::Failed) => Load::Failed,
    }
}

/// Result of a scored match from the crew team's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    Win,
    Loss,
    Draw,
}

impl Outcome {
    pub(super) fn of(score_us: Option<u32>, score_them: Option<u32>) -> Option<(Self, u32, u32)> {
        let (us, them) = (score_us?, score_them?);
        let outcome = match us.cmp(&them) {
            std::cmp::Ordering::Greater => Outcome::Win,
            std::cmp::Ordering::Less => Outcome::Loss,
            std::cmp::Ordering::Equal => Outcome::Draw,
        };
        Some((outcome, us, them))
    }

    /// The badge word. The fill never carries the result on its own.
    const fn word(self) -> &'static str {
        match self {
            Outcome::Win => "Win",
            Outcome::Loss => "Loss",
            Outcome::Draw => "Draw",
        }
    }

    const fn tone(self) -> PillTone {
        match self {
            Outcome::Win => PillTone::Ok,
            Outcome::Loss => PillTone::Danger,
            Outcome::Draw => PillTone::Neutral,
        }
    }

    /// `Won 3 to 1`, `Lost 1 to 2`, `Drew 2 to 2`.
    fn sentence(self, us: u32, them: u32) -> String {
        let verb = match self {
            Outcome::Win => "Won",
            Outcome::Loss => "Lost",
            Outcome::Draw => "Drew",
        };
        format!("{verb} {us} to {them}")
    }
}

/// `tournament` as `Tournament match`. Unknown kinds keep their own word.
pub(super) fn match_kind_label(match_type: &str) -> String {
    match match_type {
        "official" => "Official match".to_string(),
        "tournament" => "Tournament match".to_string(),
        "scrim" => "Scrim".to_string(),
        other => other.replace('_', " "),
    }
}

/// A stored value that is present and not blank.
pub(super) fn filled(value: Option<&String>) -> Option<&str> {
    value.map(|v| v.trim()).filter(|v| !v.is_empty())
}

/// What the score board says between the two sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoardPhase {
    /// Both scores are in.
    Final,
    /// No score, and the start time passed less than [`IN_PLAY_WINDOW_MS`] ago.
    InProgress,
    /// No score, and the start time is ahead or not set.
    Scheduled,
    /// No score long after the start, or a played match with no score.
    AwaitingScore,
}

/// How long after its start an unscored match counts as in play. The API has
/// no status field, so the board reads the clock.
const IN_PLAY_WINDOW_MS: f64 = 6.0 * 60.0 * 60.0 * 1000.0;

fn board_phase(
    scored: bool,
    played: bool,
    scheduled_ms: Option<f64>,
    now_ms: Option<f64>,
) -> BoardPhase {
    if scored {
        return BoardPhase::Final;
    }
    if played {
        return BoardPhase::AwaitingScore;
    }
    match (scheduled_ms, now_ms) {
        (Some(start), Some(now)) if now >= start => {
            if now - start < IN_PLAY_WINDOW_MS {
                BoardPhase::InProgress
            } else {
                BoardPhase::AwaitingScore
            }
        }
        _ => BoardPhase::Scheduled,
    }
}

impl BoardPhase {
    const fn label(self) -> &'static str {
        match self {
            BoardPhase::Final => "to",
            BoardPhase::InProgress => "In progress",
            BoardPhase::Scheduled => "Scheduled",
            BoardPhase::AwaitingScore => "Score not recorded",
        }
    }
}

/// The browser clock and the scheduled start, both as epoch milliseconds.
#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn clock(scheduled_at: Option<&str>) -> (Option<f64>, Option<f64>) {
    let start = scheduled_at
        .map(js_sys::Date::parse)
        .filter(|ms| ms.is_finite());
    (start, Some(js_sys::Date::now()))
}

/// Native builds (unit tests) have no browser clock: an unscored match reads
/// as scheduled.
#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
fn clock(_scheduled_at: Option<&str>) -> (Option<f64>, Option<f64>) {
    (None, None)
}

/// `2026-11-12 20:00 UTC+1` (the app's local stamp) as
/// `12 Nov 2026, 20:00 UTC+1`, the date order the frames use. Text that is
/// not a stamp passes through.
fn frame_stamp(stamp: &str) -> String {
    match (short_date(stamp), stamp.get(10..)) {
        (Some((_, date)), Some(rest)) if !rest.trim().is_empty() => {
            format!("{date}, {}", rest.trim())
        }
        (Some((_, date)), _) => date,
        _ => stamp.to_string(),
    }
}

const PAGE_CSS: &str = r#"
.match-lead time { white-space: nowrap; }
.match-board {
  box-sizing: border-box;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  gap: var(--space-5);
  min-width: 0;
  padding: var(--space-5);
  text-align: center;
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
.match-side { min-width: 0; }
.match-side h2 {
  margin: 0;
  font-family: var(--font-head);
  font-weight: var(--font-head-weight);
  letter-spacing: var(--font-head-tracking);
  font-size: var(--text-xl);
  line-height: var(--leading-display);
  color: var(--text);
  overflow-wrap: anywhere;
}
.match-side h2 a { color: inherit; text-decoration: none; }
.match-side h2 a:hover { text-decoration: underline; }
.match-side h2 a:focus-visible {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
.match-num {
  margin: var(--space-4) 0 0;
  font-family: var(--font-head);
  font-size: var(--text-2xl);
  font-weight: var(--font-label-weight);
  font-variant-numeric: tabular-nums;
  line-height: var(--leading-display);
  color: var(--text);
}
.match-num.is-win { color: var(--win); }
.match-mid {
  margin: 0;
  font-family: var(--font-body);
  font-size: var(--text-md);
  line-height: var(--leading-body);
  color: var(--text);
}
.match-maps {
  display: grid;
  gap: var(--space-3);
  margin: 0;
  padding: 0;
  list-style: none;
  min-width: 0;
}
/* Map name and score line on the left, the result badge on the right. */
.match-map {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: var(--space-3) var(--space-4);
  align-items: start;
}
.match-map h3 {
  margin: 0 0 var(--space-2);
  font-family: var(--font-head);
  font-size: var(--text-lg);
  font-weight: var(--font-label-weight);
  letter-spacing: var(--font-head-tracking);
  line-height: var(--leading-ui);
  color: var(--text);
  overflow-wrap: anywhere;
}
.match-media {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3) var(--space-5);
}
.match-replay {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-2);
}
.match-replay code {
  font-family: var(--font-mono);
  font-size: var(--text-md);
  color: var(--text);
  overflow-wrap: anywhere;
}
@media (max-width: 820px) {
  /* The board stacks: side, the word between, side. */
  .match-board { flex-direction: column; }
  /* The badge drops under the score line and hugs its word. */
  .match-map { grid-template-columns: minmax(0, 1fr); }
  .match-map .ui-pill { justify-self: start; }
}
"#;

#[component]
pub fn MatchDetail(id: String) -> Element {
    let mut refresh = use_signal(|| 0u32);
    let match_id = id.clone();
    let data = use_resource(move || {
        let id = match_id.clone();
        let _ = refresh();
        async move {
            if id.is_empty() {
                return PublicFetch::NotFound;
            }
            fetch_public::<MatchDetailData>(&format!("/api/public/matches/{id}")).await
        }
    });
    let load = load_of(data.read().as_ref());
    let retrying = is_retrying(data.state()(), &load);
    crate::head::use_page_title(tab_label(&load));
    let (scheduled_ms, now_ms) = match &load {
        Load::Ready(m) => clock(m.scheduled_at.as_deref()),
        _ => (None, None),
    };

    rsx! {
        MatchView {
            load,
            scheduled_ms,
            now_ms,
            retrying,
            on_retry: move |_| refresh += 1,
        }
    }
}

/// The browser tab names the match by its two sides, not by the record id.
fn tab_label(load: &Load<MatchDetailData>) -> Option<String> {
    match load {
        Load::Ready(m) => Some(format!(
            "{} vs {} | Match",
            m.team_name.trim(),
            m.opponent.trim()
        )),
        Load::Missing => Some("Match not found".to_string()),
        Load::Loading | Load::Failed => None,
    }
}

/// The page for one fetch state. Every state has one H1, and the status line
/// is always in the tree; only its text changes.
#[component]
fn MatchView(
    load: Load<MatchDetailData>,
    /// Scheduled start and the current time, epoch milliseconds.
    #[props(default)]
    scheduled_ms: Option<f64>,
    #[props(default)] now_ms: Option<f64>,
    #[props(default = false)] retrying: bool,
    on_retry: EventHandler<()>,
) -> Element {
    let status = load_status(&load, retrying, "Loading match");
    // The not-found card is the narrow wall of the 404 page.
    let width = if load == Load::Missing {
        PageWidth::Narrow
    } else {
        PageWidth::Wide
    };

    rsx! {
        crate::styles::StyleOnce { id: ARTICLE_CSS_ID, css: ARTICLE_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-match_detail-page-css", css: PAGE_CSS }

        PageShell { width, class: "match-page",
            {match &load {
                Load::Loading => rsx! { PageHeader { title: "Match" } },
                Load::Failed => rsx! { PageHeader { title: "Couldn't load this match" } },
                Load::Missing | Load::Ready(_) => rsx! {},
            }}
            p { class: "entry-status", role: "status", "{status}" }
            {match &load {
                Load::Loading => rsx! { SkeletonBars { rows: 3 } },
                Load::Missing => rsx! {
                    NotFoundPanel {
                        title: "Match not found",
                        message: "We could not find that match. It may be private or removed.",
                        links: vec![
                            NotFoundLink { label: "See tournaments", to: Route::Tournaments {} },
                            NotFoundLink { label: "Return home", to: Route::Home {} },
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
                Load::Ready(m) => render_match(m, scheduled_ms, now_ms),
            }}
        }
    }
}

fn render_match(m: &MatchDetailData, scheduled_ms: Option<f64>, now_ms: Option<f64>) -> Element {
    let result = Outcome::of(m.score_us, m.score_them);
    let phase = board_phase(
        result.is_some(),
        m.played_at.is_some(),
        scheduled_ms,
        now_ms,
    );
    let team_route = Route::TeamPage {
        id: m.team_id.clone(),
    };
    let title = format!("{} vs {}", m.team_name, m.opponent);
    let kind = match filled(m.game_name.as_ref()) {
        Some(game) => format!("{game} · {}", match_kind_label(&m.match_type)),
        None => match_kind_label(&m.match_type),
    };
    let when = m
        .played_at
        .as_deref()
        .or(m.scheduled_at.as_deref())
        .map(local_time_view)
        .map(|view| (frame_stamp(&view.text), view.datetime.zip(view.title)));
    let score_line = match result {
        Some((outcome, us, them)) => outcome.sentence(us, them),
        None => "Not played yet".to_string(),
    };
    let map = filled(m.map_name.as_ref());
    let mode = filled(m.game_mode.as_ref());
    let vod = m.vod_url.as_deref().and_then(stored_http_url);
    let replay = filled(m.replay_code.as_ref());

    rsx! {
        ArticleCrumb {
            to: team_route.clone(),
            section: m.team_name.clone(),
            current: "This match".to_string(),
        }
        PageHeader {
            title,
            meta: rsx! {
                p { class: "page-lead match-lead",
                    "{kind}"
                    if let Some((stamp, machine)) = when {
                        " · "
                        if let Some((datetime, utc)) = machine {
                            time { datetime: "{datetime}", title: "{utc}", "{stamp}" }
                        } else {
                            span { "{stamp}" }
                        }
                    }
                }
            },
        }

        div { class: "match-board",
            div { class: "match-side",
                h2 { Link { to: team_route.clone(), "{m.team_name}" } }
                if let Some((outcome, us, _)) = result {
                    p {
                        class: if outcome == Outcome::Win { "match-num is-win" } else { "match-num" },
                        "{us}"
                    }
                }
            }
            p { class: "match-mid", "{phase.label()}" }
            div { class: "match-side",
                h2 { "{m.opponent}" }
                if let Some((outcome, _, them)) = result {
                    p {
                        class: if outcome == Outcome::Loss { "match-num is-win" } else { "match-num" },
                        "{them}"
                    }
                }
            }
        }

        section { class: "ui-stack",
            h2 { class: "ui-section-title", "Maps" }
            if map.is_none() && result.is_none() {
                EmptyCard {
                    title: "No maps yet",
                    message: "The map and its score show up here once the match is recorded.",
                    actions: rsx! {
                        Link { to: team_route.clone(), class: btn_class(true), "Open {m.team_name}" }
                    },
                }
            } else {
                ul { class: "match-maps",
                    li { class: "entry-row match-map",
                        div {
                            h3 { {map.unwrap_or("Map not recorded")} }
                            p {
                                "{score_line}"
                                if let Some(mode) = mode {
                                    " · {mode}"
                                }
                            }
                        }
                        if let Some((outcome, _, _)) = result {
                            Pill { tone: outcome.tone(), {outcome.word()} }
                        }
                    }
                }
            }
        }

        if vod.is_some() || replay.is_some() {
            section { class: "ui-stack",
                h2 { class: "ui-section-title", "Watch and replay" }
                div { class: "entry-row match-media",
                    if let Some(vod) = vod {
                        match vod {
                            StoredUrl::Link(href) => rsx! {
                                a {
                                    class: btn_class(false),
                                    href: "{href}",
                                    target: "_blank",
                                    rel: "noopener noreferrer",
                                    "Watch the VOD"
                                    span { class: "visually-hidden", " (opens in a new tab)" }
                                }
                            },
                            StoredUrl::Plain(text) => rsx! {
                                p { "VOD: {text}" }
                            },
                        }
                    }
                    if let Some(code) = replay {
                        p { class: "match-replay",
                            "Replay code"
                            code { "{code}" }
                        }
                    }
                }
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

    const MATCH_ID: &str = "match_result:m7q2";
    const TEAM_ID: &str = "team:alpha9";
    const HOUR_MS: f64 = 60.0 * 60.0 * 1000.0;

    fn played() -> MatchDetailData {
        MatchDetailData {
            id: MATCH_ID.to_string(),
            team_id: TEAM_ID.to_string(),
            opponent: "Crew Beta".to_string(),
            score_us: Some(3),
            score_them: Some(1),
            map_name: Some("Route 66".to_string()),
            game_mode: Some("Escort".to_string()),
            match_type: "tournament".to_string(),
            played_at: Some("2026-11-12T19:00:00Z".to_string()),
            scheduled_at: None,
            vod_url: None,
            replay_code: None,
            team_name: "Squad Alpha".to_string(),
            game_name: Some("Overwatch".to_string()),
        }
    }

    fn upcoming() -> MatchDetailData {
        MatchDetailData {
            score_us: None,
            score_them: None,
            map_name: None,
            game_mode: None,
            played_at: None,
            scheduled_at: Some("2026-11-12T19:00:00Z".to_string()),
            ..played()
        }
    }

    fn render_at(
        load: Load<MatchDetailData>,
        scheduled_ms: Option<f64>,
        now_ms: Option<f64>,
    ) -> String {
        render_routed(move || {
            rsx! {
                MatchView {
                    load: load.clone(),
                    scheduled_ms,
                    now_ms,
                    retrying: false,
                    on_retry: move |_| {},
                }
            }
        })
    }

    fn render(load: Load<MatchDetailData>) -> String {
        render_at(load, None, None)
    }

    #[test]
    fn played_match_has_crumb_one_h1_board_and_a_worded_map_row() {
        let html = render(Load::Ready(played()));
        assert!(!html.contains("<main"), "{html}");
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide match-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Squad Alpha vs Crew Beta</h1>"), "{html}");
        assert!(
            html.contains("<nav class=\"article-crumb\" aria-label=\"Breadcrumb\">"),
            "{html}"
        );
        assert!(html.contains("href=\"/teams/team:alpha9\""), "{html}");
        assert!(
            html.contains("<span aria-current=\"page\">This match</span>"),
            "{html}"
        );
        // Kind and date line, date in the frame order.
        assert!(html.contains("Overwatch · Tournament match"), "{html}");
        assert!(
            html.contains("<time datetime=\"2026-11-12T19:00:00Z\""),
            "{html}"
        );
        assert!(html.contains("12 Nov 2026, 19:00 UTC</time>"), "{html}");
        // Board: both sides, the winning number marked, "to" between.
        assert!(
            html.contains("<p class=\"match-num is-win\">3</p>"),
            "{html}"
        );
        assert!(html.contains("<p class=\"match-num\">1</p>"), "{html}");
        assert!(html.contains("<p class=\"match-mid\">to</p>"), "{html}");
        // Map row: the result is a word in the badge and in the score line.
        assert!(html.contains("<h3>Route 66</h3>"), "{html}");
        assert!(html.contains("Won 3 to 1"), "{html}");
        assert!(html.contains("Escort"), "{html}");
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--ok\">Win</span>"),
            "{html}"
        );
    }

    #[test]
    fn a_loss_and_a_draw_are_words_not_only_fills() {
        let lost = render(Load::Ready(MatchDetailData {
            score_us: Some(1),
            score_them: Some(2),
            ..played()
        }));
        assert!(
            lost.contains("<span class=\"ui-pill ui-pill--danger\">Loss</span>"),
            "{lost}"
        );
        assert!(lost.contains("Lost 1 to 2"), "{lost}");
        // The opponent's number carries the mark, never ours.
        assert!(lost.contains("<p class=\"match-num\">1</p>"), "{lost}");
        assert!(
            lost.contains("<p class=\"match-num is-win\">2</p>"),
            "{lost}"
        );

        let drew = render(Load::Ready(MatchDetailData {
            score_us: Some(2),
            score_them: Some(2),
            ..played()
        }));
        assert!(
            drew.contains("<span class=\"ui-pill ui-pill--neutral\">Draw</span>"),
            "{drew}"
        );
        assert!(drew.contains("Drew 2 to 2"), "{drew}");
        assert!(!drew.contains("is-win"), "{drew}");
    }

    #[test]
    fn board_phase_follows_the_score_and_the_clock() {
        let start = 1_000_000.0;
        assert_eq!(board_phase(true, true, None, None), BoardPhase::Final);
        assert_eq!(
            board_phase(false, false, Some(start), Some(start - HOUR_MS)),
            BoardPhase::Scheduled
        );
        assert_eq!(
            board_phase(false, false, Some(start), Some(start)),
            BoardPhase::InProgress
        );
        assert_eq!(
            board_phase(false, false, Some(start), Some(start + 2.0 * HOUR_MS)),
            BoardPhase::InProgress
        );
        assert_eq!(
            board_phase(false, false, Some(start), Some(start + 6.0 * HOUR_MS)),
            BoardPhase::AwaitingScore
        );
        assert_eq!(
            board_phase(false, true, None, Some(start)),
            BoardPhase::AwaitingScore
        );
        // No start time or no clock: nothing says it began.
        assert_eq!(
            board_phase(false, false, None, Some(start)),
            BoardPhase::Scheduled
        );
        assert_eq!(
            board_phase(false, false, Some(start), None),
            BoardPhase::Scheduled
        );
    }

    #[test]
    fn a_match_in_play_says_in_progress_on_the_board() {
        let start = 1_000_000.0;
        let html = render_at(Load::Ready(upcoming()), Some(start), Some(start + HOUR_MS));
        assert!(
            html.contains("<p class=\"match-mid\">In progress</p>"),
            "{html}"
        );
        assert!(!html.contains("match-num"), "{html}");
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        // No map and no score yet: the empty card says so and offers a step.
        assert!(html.contains("<h2>No maps yet</h2>"), "{html}");
        assert!(html.contains("Open Squad Alpha"), "{html}");
        assert!(!html.contains("ui-pill"), "{html}");

        let before = render_at(Load::Ready(upcoming()), Some(start), Some(start - HOUR_MS));
        assert!(
            before.contains("<p class=\"match-mid\">Scheduled</p>"),
            "{before}"
        );
    }

    #[test]
    fn a_score_without_a_map_still_gets_a_worded_row() {
        let html = render(Load::Ready(MatchDetailData {
            map_name: Some("  ".to_string()),
            game_mode: None,
            ..played()
        }));
        assert!(html.contains("<h3>Map not recorded</h3>"), "{html}");
        assert!(html.contains(">Win</span>"), "{html}");
    }

    #[test]
    fn media_section_only_with_a_vod_or_a_replay_code() {
        let none = render(Load::Ready(played()));
        assert!(!none.contains("Watch and replay"), "{none}");

        let html = render(Load::Ready(MatchDetailData {
            vod_url: Some("https://example.com/vod/1".to_string()),
            replay_code: Some("AB12CD".to_string()),
            ..played()
        }));
        assert!(html.contains("Watch and replay"), "{html}");
        assert!(
            html.contains("href=\"https://example.com/vod/1\""),
            "{html}"
        );
        assert!(html.contains("rel=\"noopener noreferrer\""), "{html}");
        assert!(html.contains("(opens in a new tab)"), "{html}");
        assert!(html.contains("<code>AB12CD</code>"), "{html}");

        let unsafe_url = render(Load::Ready(MatchDetailData {
            vod_url: Some("javascript:alert(1)".to_string()),
            ..played()
        }));
        assert!(!unsafe_url.contains("href=\"javascript"), "{unsafe_url}");
    }

    #[test]
    fn loading_failed_and_missing_states() {
        let loading = render(Load::Loading);
        assert!(loading.contains("<h1>Match</h1>"), "{loading}");
        assert!(
            loading.contains("<p class=\"entry-status\" role=\"status\">Loading match</p>"),
            "{loading}"
        );
        assert!(loading.contains("ui-skel-list"), "{loading}");

        let failed = render(Load::Failed);
        assert_eq!(failed.matches("<h1").count(), 1, "{failed}");
        assert!(failed.contains("load this match</h1>"), "{failed}");
        assert!(failed.contains("fetch-error__retry"), "{failed}");
        // The live region stays in the tree with no text.
        assert!(
            failed.contains("<p class=\"entry-status\" role=\"status\"></p>"),
            "{failed}"
        );

        let missing = render(Load::Missing);
        assert_eq!(missing.matches("<h1").count(), 1, "{missing}");
        assert!(missing.contains("Match not found"), "{missing}");
        assert!(missing.contains("ui-shell--narrow"), "{missing}");
        assert!(missing.contains("href=\"/tournaments\""), "{missing}");
        assert!(!missing.contains("article-crumb"), "{missing}");
        for html in [&loading, &failed, &missing] {
            assert!(!html.contains("<main"), "{html}");
        }
    }

    #[test]
    fn tab_is_named_by_the_two_sides_not_the_id() {
        assert_eq!(
            tab_label(&Load::Ready(played())).as_deref(),
            Some("Squad Alpha vs Crew Beta | Match")
        );
        assert_eq!(
            tab_label(&Load::Missing).as_deref(),
            Some("Match not found")
        );
        assert_eq!(tab_label(&Load::Loading), None);
        let html = render(Load::Ready(played()));
        assert!(!html.contains(MATCH_ID), "raw match id leaked: {html}");
    }

    #[test]
    fn stamp_uses_the_frame_date_order_and_keeps_the_zone() {
        assert_eq!(
            frame_stamp("2026-11-12 20:00 UTC+1"),
            "12 Nov 2026, 20:00 UTC+1"
        );
        assert_eq!(frame_stamp("2026-11-12"), "12 Nov 2026");
        assert_eq!(frame_stamp("soon"), "soon");
    }

    #[test]
    fn kind_labels_and_outcomes() {
        assert_eq!(match_kind_label("official"), "Official match");
        assert_eq!(match_kind_label("tournament"), "Tournament match");
        assert_eq!(match_kind_label("show_match"), "show match");
        assert_eq!(Outcome::of(Some(3), None), None);
        assert_eq!(Outcome::of(Some(3), Some(1)), Some((Outcome::Win, 3, 1)));
    }

    #[test]
    fn css_is_on_tokens_and_the_board_stacks_on_a_phone() {
        assert_sizes_from_tokens(PAGE_CSS);
        let board = css_rule(PAGE_CSS, ".match-board {");
        for decl in [
            "background: var(--card-bg)",
            "border: 1px solid var(--card-border)",
            "border-radius: var(--card-radius)",
            "padding: var(--space-5)",
        ] {
            assert!(has_decl(&board, decl), "missing {decl}: {board}");
        }
        let (_, phone) = PAGE_CSS
            .split_once("@media (max-width: 820px) {")
            .expect("phone block");
        assert!(
            has_decl(&css_rule(phone, ".match-board {"), "flex-direction: column"),
            "{phone}"
        );
        assert!(
            has_decl(
                &css_rule(phone, ".match-map {"),
                "grid-template-columns: minmax(0, 1fr)"
            ),
            "{phone}"
        );
    }
}
