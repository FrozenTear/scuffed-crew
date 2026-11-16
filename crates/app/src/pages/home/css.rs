//! Homepage CSS for the redesign (Direction 1, Look A).
//!
//! The page body is a `PageShell` at the default content width, so the gutter
//! and the gap between blocks come from `--space-page` and `--space-section`.
//! Every size here is a token, so `data-density` applies. Shell and skin layers
//! add overrides through the data attributes on `.home-wrap`.

/// Structure and look shared by every shell and skin.
pub const HOME_SHARED_CSS: &str = r#"
    .home-wrap { min-width: 0; }
    .home-page {
        font-family: var(--font-body);
        font-size: var(--text-md);
        line-height: var(--leading-body);
        color: var(--text);
    }

    /* --- Hero. Left by default, centred with `is-centered`. --- */
    .home-hero {
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: var(--space-4);
        padding: var(--space-section) 0;
    }
    .home-hero.is-centered {
        align-items: center;
        text-align: center;
    }
    .home-banner {
        display: inline-flex;
        align-items: center;
        gap: var(--space-2);
        min-height: var(--hit-target-min);
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-pill);
        background: var(--banner-soft-bg);
        color: var(--banner-soft-fg);
        font-family: var(--font-head);
        font-size: var(--text-xs);
        font-weight: var(--font-label-weight);
        letter-spacing: var(--font-head-tracking);
        text-transform: uppercase;
        line-height: var(--leading-ui);
        overflow-wrap: anywhere;
        text-box-trim: trim-both;
        text-box-edge: cap alphabetic;
    }
    .home-hero-title,
    .home-card-title {
        margin: 0;
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        letter-spacing: var(--font-head-tracking);
        text-transform: var(--font-head-transform);
        line-height: var(--leading-display);
        color: var(--text);
        overflow-wrap: anywhere;
    }
    .home-hero-title {
        font-size: var(--text-3xl);
        max-width: var(--content-max-narrow);
        text-box-trim: trim-both;
        text-box-edge: cap alphabetic;
    }
    /* The second title line. It is a block, so it reads as its own line. */
    .home-hero-title em {
        display: block;
        font-style: normal;
        color: var(--accent-text);
    }
    .home-hero-sub {
        margin: 0;
        max-width: var(--content-max-narrow);
        color: var(--text-3);
        font-size: var(--text-lg);
    }
    .home-actions,
    .home-action-row,
    .home-tag-row {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
    }
    .home-hero.is-centered .home-actions {
        justify-content: center;
        width: 100%;
    }

    /* Buttons are the shared `btn` (components/ui/button.rs). */
    .home-link:focus-visible,
    .home-list a:focus-visible,
    .home-team-name a:focus-visible,
    .home-team-card:focus-visible,
    .home-team-chip:focus-visible {
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }

    /* --- Stat tiles: one row at every width --- */
    .home-stats {
        display: grid;
        grid-auto-flow: column;
        grid-auto-columns: minmax(0, 1fr);
        gap: var(--space-3);
    }
    .home-stat {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--space-1);
        min-width: 0;
        padding: var(--card-padding);
        background: var(--card-bg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--card-shadow);
        text-align: center;
    }
    .home-stat-n {
        display: block;
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        font-size: var(--text-2xl);
        letter-spacing: var(--font-head-tracking);
        line-height: var(--leading-ui);
        font-variant-numeric: tabular-nums;
        color: var(--text);
    }
    .home-stat-l {
        display: block;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-ui);
    }

    /* --- Skeletons. One line box tall, so nothing moves when data lands. --- */
    .home-skel {
        display: block;
        border-radius: var(--radius-control);
        background: color-mix(in srgb, var(--text) 12%, var(--surface-sunken));
    }
    .home-skel-badge {
        width: calc(var(--space-12) * 3);
        height: var(--hit-target-min);
        border-radius: var(--radius-pill);
    }
    .home-skel-title {
        width: min(calc(var(--space-12) * 6), 80%);
        height: var(--text-3xl);
    }
    .home-skel-sub {
        width: min(calc(var(--space-12) * 9), 100%);
        height: var(--text-lg);
    }
    .home-skel-actions {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
    }
    .home-hero.is-centered .home-skel-actions { justify-content: center; }
    .home-skel-btn {
        width: calc(var(--space-12) * 2);
        height: var(--button-height);
    }
    .home-skel-stat {
        width: var(--space-6);
        height: 1lh;
        margin: 0 auto;
    }

    /* --- Sections and cards --- */
    .home-section {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
        min-width: 0;
        /* The hero's second button jumps here: land below the sticky top bar. */
        scroll-margin-top: calc(var(--nav-height-used) + var(--space-section));
    }
    .home-head { min-width: 0; }
    .home-kicker {
        margin: 0 0 var(--space-2);
        font-family: var(--font-head);
        font-weight: var(--font-label-weight);
        font-size: var(--text-xs);
        letter-spacing: var(--font-head-tracking);
        text-transform: uppercase;
        line-height: var(--leading-ui);
        color: var(--accent-text);
    }
    .home-card-title { font-size: var(--text-lg); }
    .home-body {
        margin: 0;
        max-width: var(--content-max-narrow);
        color: var(--text-3);
    }
    .home-muted {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
    }
    /* A list's status line with nothing to say: out of the layout, still in
       the tree so its next text is announced. */
    .home-list-status:empty {
        position: absolute;
        width: 1px;
        height: 1px;
        padding: 0;
        margin: -1px;
        overflow: hidden;
        clip: rect(0, 0, 0, 0);
        white-space: nowrap;
        border: 0;
    }
    .home-card {
        box-sizing: border-box;
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
        min-width: 0;
        padding: var(--card-padding);
        background: var(--card-bg);
        color: var(--card-fg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--card-shadow);
    }
    .home-card p { margin: 0; }
    .home-grid-2 {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: var(--space-4);
    }
    .home-grid-2.is-single { grid-template-columns: 1fr; }
    .home-stack {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
    }
    .home-link {
        display: inline-flex;
        align-items: center;
        align-self: flex-start;
        min-height: var(--hit-target-min);
        color: var(--accent-text);
        font-size: var(--text-md);
        text-decoration: none;
    }
    .home-link:hover { text-decoration: underline; }

    /* --- Ethos rules --- */
    .home-rules {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }
    .home-rules li {
        display: flex;
        align-items: flex-start;
        gap: var(--space-3);
        padding: var(--card-padding);
        background: var(--card-bg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
    }
    .home-rule-num {
        min-width: var(--space-6);
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        font-size: var(--text-lg);
        font-variant-numeric: tabular-nums;
        line-height: var(--leading-body);
        color: var(--accent-text);
    }

    /* --- Lists inside cards (schedule, tournaments, expectations) --- */
    .home-list {
        list-style: none;
        margin: 0;
        padding: 0;
    }
    .home-list li {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-1) var(--space-4);
        min-height: var(--density-row);
        padding: var(--space-1) 0;
        border-bottom: 1px solid var(--border);
    }
    .home-list li:last-child { border-bottom: 0; }
    .home-list a {
        display: inline-flex;
        align-items: center;
        min-height: var(--hit-target-min);
        color: var(--text);
        font-weight: var(--font-label-weight);
        text-decoration: underline;
    }
    .home-list a:hover { color: var(--accent-text); }
    .home-meta {
        color: var(--text-3);
        font-size: var(--text-sm);
        font-variant-numeric: tabular-nums;
    }
    .home-bullets {
        margin: 0;
        padding: 0 0 0 var(--space-5);
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        color: var(--text-3);
    }

    /* --- Next match and results --- */
    .home-match-vs {
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        font-size: var(--text-lg);
        line-height: var(--leading-display);
        color: var(--text);
        overflow-wrap: anywhere;
    }
    .home-match-sep {
        color: var(--text-3);
        font-weight: var(--font-label-weight);
    }
    .home-match-when {
        color: var(--text);
        font-variant-numeric: tabular-nums;
    }
    .home-results {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-3);
        padding: var(--space-3) var(--card-padding);
        background: var(--card-bg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
    }
    .home-results p { margin: 0; }
    .home-results-label {
        font-family: var(--font-head);
        font-weight: var(--font-label-weight);
        font-size: var(--text-xs);
        letter-spacing: var(--font-head-tracking);
        text-transform: uppercase;
        color: var(--text-3);
    }
    .home-results-track {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
        flex: 1;
        min-width: 0;
    }
    .home-result {
        display: inline-flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-1) var(--space-2);
        padding: var(--space-1) var(--space-3);
        border: 1px solid var(--border);
        border-radius: var(--radius-control);
        font-size: var(--text-sm);
    }
    .home-result-teams {
        color: var(--text);
        font-weight: var(--font-label-weight);
    }
    .home-result-score {
        color: var(--text);
        font-weight: var(--font-head-weight);
        font-variant-numeric: tabular-nums;
    }
    .home-result-date {
        color: var(--text-3);
        font-size: var(--text-xs);
        font-variant-numeric: tabular-nums;
    }

    /* --- Teams: table (ops hub), cards, chips --- */
    .home-team-table {
        /* name, game, roster, division, record */
        --home-team-cols: minmax(0, 1.4fr) minmax(0, 0.9fr) minmax(0, 0.5fr) minmax(0, 0.7fr) minmax(0, 0.6fr);
        background: var(--card-bg);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--card-shadow);
        overflow: hidden;
    }
    .home-team-head,
    .home-team-row {
        display: grid;
        grid-template-columns: var(--home-team-cols);
        column-gap: var(--space-4);
        align-items: center;
        min-height: var(--density-row);
        padding: var(--space-1) var(--card-padding);
    }
    .home-team-head {
        font-family: var(--font-head);
        font-weight: var(--font-label-weight);
        font-size: var(--text-xs);
        letter-spacing: var(--font-head-tracking);
        text-transform: uppercase;
        color: var(--text-3);
    }
    .home-team-row { border-top: 1px solid var(--border); }
    .home-team-head > :nth-child(n+3),
    .home-team-row > :nth-child(n+3) {
        text-align: right;
        font-variant-numeric: tabular-nums;
    }
    .home-team-row > * { min-width: 0; overflow-wrap: anywhere; }
    .home-team-name {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0 var(--space-2);
    }
    .home-team-name a {
        display: inline-flex;
        align-items: center;
        min-height: var(--hit-target-min);
        color: var(--text);
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        font-size: var(--text-lg);
        text-decoration: none;
    }
    .home-team-name a:hover {
        color: var(--accent-text);
        text-decoration: underline;
    }
    .home-team-lore {
        flex-basis: 100%;
        padding-bottom: var(--space-2);
        color: var(--text-3);
        font-size: var(--text-sm);
        font-style: italic;
    }
    .home-team-game,
    .home-team-roster,
    .home-team-div,
    .home-team-record {
        color: var(--text-3);
        font-size: var(--text-sm);
    }
    .home-team-cards {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(min(calc(var(--space-12) * 5), 100%), 1fr));
        gap: var(--space-3);
    }
    a.home-team-card {
        text-decoration: none;
        min-height: var(--hit-target-min);
    }
    a.home-team-card:hover { border-color: var(--border-strong); }
    a.home-team-card:hover .home-card-title { text-decoration: underline; }
    .home-team-card.is-forming,
    .home-team-chip.is-forming { border-style: dashed; }
    .home-team-chips {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
    }
    .home-team-chip {
        display: inline-flex;
        align-items: center;
        min-height: var(--hit-target-min);
        padding: var(--space-1) var(--space-4);
        border: 1px solid var(--border-strong);
        border-radius: var(--radius-pill);
        background: var(--card-bg);
        color: var(--text);
        font-size: var(--text-sm);
        text-decoration: none;
    }
    .home-team-chip:hover { border-color: var(--text); }

    /* --- News --- */
    .home-news-meta {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
        color: var(--text-3);
        font-size: var(--text-xs);
        font-variant-numeric: tabular-nums;
    }
    .home-news-card .home-news-meta { margin-bottom: calc(var(--space-1) * -1); }
    .home-news-body { overflow-wrap: anywhere; }

    /* --- Recruit tags --- */
    .home-tag-label {
        color: var(--text-3);
        font-size: var(--text-sm);
    }
    .home-tag {
        display: inline-flex;
        align-items: center;
        padding: var(--space-1) var(--space-2);
        border: 1px solid var(--border);
        border-radius: var(--radius-control);
        background: var(--surface);
        color: var(--text);
        font-size: var(--text-xs);
        line-height: var(--leading-body);
    }

    .home-foot {
        margin: 0;
        padding-top: var(--space-4);
        border-top: 1px solid var(--border);
        color: var(--text-3);
        font-size: var(--text-xs);
    }

    @supports not (text-box-trim: trim-both) {
        /* The display face leaves extra room under the cap height. */
        .home-banner { padding-top: calc(var(--space-2) + 0.08em); }
    }

    @media (max-width: 820px) {
        .home-grid-2 { grid-template-columns: 1fr; }
        .home-stats { gap: var(--space-2); }
        .home-stat {
            min-height: var(--hit-target-min);
            padding: var(--space-2);
        }
        .home-stat-n { font-size: var(--text-lg); }
        .home-stat-l {
            font-size: var(--text-xs);
            overflow-wrap: anywhere;
        }
        /* Name and record only. The team page has the rest. */
        .home-team-table { --home-team-cols: minmax(0, 1fr) auto; }
        .home-team-head > :nth-child(2),
        .home-team-head > :nth-child(3),
        .home-team-head > :nth-child(4),
        .home-team-row .home-team-game,
        .home-team-row .home-team-roster,
        .home-team-row .home-team-div { display: none; }
    }
"#;

/// Esports skin: upper-case display type on the same tokens. The clean skin
/// is the Look A base with no layer of its own.
pub const HOME_SKIN_ESPORTS_CSS: &str = r#"
.home-wrap[data-home-skin="esports"] .home-hero-title,
.home-wrap[data-home-skin="esports"] .ui-section-title { text-transform: uppercase; }
.home-wrap[data-home-skin="esports"] .home-kicker { letter-spacing: var(--font-label-tracking); }
"#;

/// Shell-specific composition tweaks. Every shell is the default content
/// width. What a shell changes is in Rust (`HomeShell`): section order, the
/// empty policy and the teams presentation.
pub const HOME_SHELL_CSS: &str = r#"
/* Minimal: no stat tiles. */
.home-wrap[data-home-shell="minimal"] .home-stats { display: none; }
"#;

pub fn home_css_layers() -> String {
    [HOME_SHARED_CSS, HOME_SHELL_CSS, HOME_SKIN_ESPORTS_CSS].concat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{
        assert_ink_clears_aa, assert_sizes_from_tokens, css_rule, has_decl, rules,
    };

    #[test]
    fn home_sizes_and_type_come_from_tokens() {
        // The hidden status box is the one place a 1px size is not a hairline,
        // and the audit already allows 1px.
        assert_sizes_from_tokens(&home_css_layers());
    }

    #[test]
    fn home_css_uses_no_banned_or_raw_colour_names() {
        let css = home_css_layers();
        // Split so this file does not contain the banned names itself.
        for banned in [
            ["--text", "-muted"].concat(),
            ["--text", "-secondary"].concat(),
            ["--font", "-display"].concat(),
            ["--succ", "ess"].concat(),
            ["--warn", "ing"].concat(),
            ["rg", "b("].concat(),
            ["rg", "ba("].concat(),
        ] {
            assert!(!css.contains(banned.as_str()), "{banned}");
        }
        for (selector, body) in rules(&css) {
            for decl in body.split(';') {
                let Some((_, value)) = decl.split_once(':') else {
                    continue;
                };
                assert!(!value.contains('#'), "{selector}: {decl}");
            }
        }
    }

    #[test]
    fn hero_is_left_by_default_and_centred_by_modifier() {
        let base = css_rule(HOME_SHARED_CSS, ".home-hero {");
        assert!(has_decl(&base, "align-items: flex-start"), "{base}");
        assert!(has_decl(&base, "padding: var(--space-section) 0"), "{base}");
        let centred = css_rule(HOME_SHARED_CSS, ".home-hero.is-centered {");
        assert!(has_decl(&centred, "align-items: center"), "{centred}");
        assert!(has_decl(&centred, "text-align: center"), "{centred}");
        // Cards and section heads never take the centring.
        for (selector, body) in rules(HOME_SHARED_CSS) {
            if body.contains("text-align: center") {
                assert!(
                    selector == ".home-hero.is-centered" || selector == ".home-stat",
                    "{selector} is centred"
                );
            }
        }
    }

    #[test]
    fn community_banner_uses_the_soft_banner_tokens() {
        let banner = css_rule(HOME_SHARED_CSS, ".home-banner {");
        for decl in [
            "background: var(--banner-soft-bg)",
            "color: var(--banner-soft-fg)",
            "border-radius: var(--radius-pill)",
            "font-size: var(--text-xs)",
            "text-transform: uppercase",
        ] {
            assert!(has_decl(&banner, decl), "missing {decl}: {banner}");
        }
        assert_ink_clears_aa("--banner-soft-fg", "--banner-soft-bg", 4.5);
    }

    #[test]
    fn cards_and_tiles_use_the_card_tokens() {
        for marker in [".home-card {", ".home-stat {"] {
            let rule = css_rule(HOME_SHARED_CSS, marker);
            for decl in [
                "padding: var(--card-padding)",
                "background: var(--card-bg)",
                "border: 1px solid var(--card-border)",
                "border-radius: var(--card-radius)",
            ] {
                assert!(has_decl(&rule, decl), "{marker} missing {decl}");
            }
        }
        let n = css_rule(HOME_SHARED_CSS, ".home-stat-n {");
        assert!(has_decl(&n, "font-size: var(--text-2xl)"), "{n}");
        assert!(has_decl(&n, "font-variant-numeric: tabular-nums"), "{n}");
    }

    #[test]
    fn interactive_controls_are_at_least_the_hit_target() {
        // Buttons and Retry are the shared 44px ones, with no home copy.
        for gone in [".home-btn", "fetch-error__retry"] {
            assert!(!HOME_SHARED_CSS.contains(gone), "{gone} is shared now");
        }
        for marker in [
            ".home-link {",
            ".home-list a {",
            ".home-team-name a {",
            ".home-team-chip {",
            "a.home-team-card {",
        ] {
            let rule = css_rule(HOME_SHARED_CSS, marker);
            assert!(
                has_decl(&rule, "min-height: var(--hit-target-min)"),
                "{marker}: {rule}"
            );
        }
    }

    #[test]
    fn anchored_sections_land_below_the_sticky_top_bar() {
        let section = css_rule(HOME_SHARED_CSS, ".home-section {");
        assert!(
            section
                .contains("scroll-margin-top: calc(var(--nav-height-used) + var(--space-section))"),
            "{section}"
        );
    }

    #[test]
    fn tags_follow_the_frame_recipe() {
        let tag = css_rule(HOME_SHARED_CSS, ".home-tag {");
        for decl in [
            "padding: var(--space-1) var(--space-2)",
            "font-size: var(--text-xs)",
            "line-height: var(--leading-body)",
        ] {
            assert!(has_decl(&tag, decl), "missing {decl}: {tag}");
        }
    }

    #[test]
    fn skins_and_shells_add_only_what_they_still_change() {
        // Clean is the base: no empty layer is shipped for it.
        let css = home_css_layers();
        assert!(!css.contains("data-home-skin=\"clean\""), "{css}");
        assert!(css.contains("[data-home-skin=\"esports\"] .ui-section-title"));
        assert!(css.contains("[data-home-shell=\"minimal\"] .home-stats"));
        // No size literal slips past the rem and px audit as an em.
        assert!(has_decl(
            &css_rule(HOME_SHARED_CSS, ".home-skel-stat {"),
            "width: var(--space-6)"
        ));
    }

    #[test]
    fn phone_keeps_three_tiles_in_a_row_and_stacks_the_card_grid() {
        let stats = css_rule(HOME_SHARED_CSS, ".home-stats {");
        assert!(has_decl(&stats, "grid-auto-flow: column"), "{stats}");
        let (_, phone) = HOME_SHARED_CSS
            .split_once("@media (max-width: 820px) {")
            .expect("phone block");
        assert!(
            has_decl(
                &css_rule(phone, ".home-grid-2 {"),
                "grid-template-columns: 1fr"
            ),
            "{phone}"
        );
        // The row stays a row: the phone block only tightens it.
        let phone_stats = css_rule(phone, ".home-stats {");
        assert!(!phone_stats.contains("grid-auto-flow"), "{phone_stats}");
        assert!(
            has_decl(
                &css_rule(phone, ".home-stat-l {"),
                "font-size: var(--text-xs)"
            ),
            "{phone}"
        );
    }

    #[test]
    fn body_copy_tokens_clear_text_contrast_on_page_and_card() {
        for ink in ["--text", "--text-3", "--accent-text"] {
            for fill in ["--bg", "--surface"] {
                assert_ink_clears_aa(ink, fill, 4.5);
            }
        }
        assert_ink_clears_aa("--on-accent", "--accent-solid", 4.5);
    }
}
