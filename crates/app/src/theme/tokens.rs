use crate::theme::brand::{self, BrandConfig};

/// Build the full design-system stylesheet (primitives + both semantic scopes)
/// from a brand config. The ONLY place raw color/scale literals live.
pub fn theme_css(brand: &BrandConfig) -> String {
    format!(
        r#"
:root {{
  --font-head: 'Space Grotesk', system-ui, sans-serif;
  --font-body: 'Inter', system-ui, sans-serif;
  --font-mono: 'JetBrains Mono', ui-monospace, monospace;

  --text-xs: 11px;  --text-sm: 12.5px; --text-base: 14px; --text-lg: 15.5px;
  --text-xl: 18px;  --text-2xl: 21px;  --text-3xl: 30px;

  --space-1: 4px;  --space-2: 8px;  --space-3: 12px; --space-4: 16px;
  --space-6: 24px; --space-8: 32px; --space-12: 48px;

  --radius-sm: 7px; --radius-md: 9px; --radius-lg: 12px; --radius-pill: 999px;

  --overlay: rgba(0,0,0,0.6);

  /* Chart palette (dark values; light scope overrides). CVD-validated against
     the dark surface (#1f1f27) — see docs/notes/stats-ui-w2-validation.md.
     Snap-tuned to the dark lightness band; re-run the validator if edited. */
  --chart-1: #8f73ff; --chart-2: #15ac7d; --chart-3: #b98a02; --chart-4: #ca474c; --chart-5: #089fd7; --chart-6: #984ab2;
  /* Winrate bars: two-pole diverging encoding (W5a). Cool up-pole (> 50%),
     warm down-pole (< 50%); exactly-50% and sub-min-games rows take the
     neutral --text-3 midpoint. Chart poles, NOT status tokens. Pair
     CVD-validated vs both surfaces — see docs/notes/stats-ui-w2-validation.md
     (W5a section); re-run the validator if edited. */
  --chart-wr-up: #089fd7; --chart-wr-down: #aa5000;
}}

[data-theme="dark"] {{
  --bg: #17171d; --surface: #1f1f27; --surface-2: #282831; --border: #353541;
  --text: #f4f4f8; --text-2: #c1c1cd; --text-3: #9696a3;
  --accent: {accent_d}; --accent-fg: #ffffff; --accent-soft: {soft_d};
  --ok: #46d8a4; --warn: #fbbf24; --danger: #f06a6a;
}}

[data-theme="light"] {{
  --bg: #f7f7f9; --surface: #ffffff; --surface-2: #f0f0f4; --border: #e3e3e9;
  --text: #16161c; --text-2: #545462; --text-3: #83838f;
  --accent: {accent_l}; --accent-fg: #ffffff; --accent-soft: {soft_l};
  --ok: #0ea66e; --warn: #c2830a; --danger: #d63031;
  /* CVD-validated against #ffffff — see docs/notes/stats-ui-w2-validation.md. */
  --chart-1: #6d4aff; --chart-2: #0ea66e; --chart-3: #c2830a; --chart-4: #d63031; --chart-5: #0284c7; --chart-6: #7405c3;
  --chart-wr-up: #0284c7; --chart-wr-down: #843900;
}}

html, body {{
  /* --page-bg-* set from admin Site Settings when customized */
  background-color: var(--page-bg-color, var(--bg));
  background-image: var(--page-bg-image, none);
  background-size: cover;
  background-position: center;
  background-attachment: fixed;
  background-repeat: no-repeat;
  color: var(--text);
  font-family: var(--font-body); font-size: var(--text-base);
}}

[data-accent="strategy"] {{ --accent: #ff7a1a; --accent-soft: rgba(255,122,26,.16); }}
"#,
        accent_d = brand.accent_dark.as_str(),
        accent_l = brand.accent_light.as_str(),
        soft_d = brand.accent_soft_dark.as_str(),
        soft_l = brand.accent_soft_light.as_str(),
    )
}

/// Concrete colors for the strategy canvas (2D context needs literal strings, not CSS vars).
pub const CANVAS_BG: &str = "#14141c";
pub const CANVAS_TILE_PLACEHOLDER: &str = "#2a2a3e";
pub const CANVAS_GRID_LOADING: &str = "#333";
pub const CANVAS_TEXT_LOADING: &str = "#666";
pub const STRATEGY_ACCENT: &str = "#ff7a1a";
pub const CANVAS_SELECTION_COLOR: &str = "#00ff00";
pub const CANVAS_BADGE_BG: &str = "rgba(0,0,0,0.7)";
pub const CANVAS_WHITE: &str = "#fff";
pub const HP_SMALL_FILL: &str = "#ffeb3b";
pub const HP_SMALL_STROKE: &str = "#ffc107";
pub const HP_LARGE_FILL: &str = "#ff9800";
pub const HP_LARGE_STROKE: &str = "#f57c00";
pub const HP_GLOW: &str = "rgba(255,255,255,0.5)";
pub const CANVAS_MARKER_BORDER: &str = "#fff";

/// Dark-theme page background (matches `[data-theme="dark"] --bg`).
pub const BG_DARK: &str = "#17171d";
/// Product-default brand accents (matches `BrandConfig::product_default`).
pub const BRAND_ACCENT_DARK: &str = "#8f73ff";
pub const BRAND_ACCENT_LIGHT: &str = "#6d4aff";
/// Browser chrome `theme-color` meta (dark shell).
pub const THEME_COLOR: &str = BG_DARK;

/// Convenience for the app root.
pub fn theme_css_current() -> String {
    theme_css(&brand::current())
}

#[cfg(test)]
mod tests {
    use super::{BrandConfig, theme_css};

    #[test]
    fn emits_both_theme_scopes_and_uses_brand_accent() {
        let brand = BrandConfig::product_default();
        let css = theme_css(&brand);
        assert!(css.contains("[data-theme=\"dark\"]"));
        assert!(css.contains("[data-theme=\"light\"]"));
        assert!(css.contains(brand.accent_dark.as_str()));
        assert!(css.contains(brand.accent_light.as_str()));
        assert!(css.contains("--bg:"));
        assert!(css.contains("--text-2:"));
        assert!(css.contains("--space-1:"));
        assert!(css.contains("[data-accent=\"strategy\"]"));
    }
}
