use crate::theme::brand::{self, BrandConfig};

/// Build the full design-system stylesheet (primitives + both semantic scopes)
/// from a brand config. The ONLY place raw color/scale literals live.
///
/// Light `--text-3` and `--ok` are normal-size text. They must clear WCAG AA
/// 4.5:1 on `--bg`, `--surface`, and `--surface-2` (the darkest light surface).
/// Dark `--accent-fg` on the product accent is a brand pair: it clears the 3:1
/// large-text/UI floor and misses 4.5:1 for normal text. The light accent with
/// the same white foreground clears 4.5:1. Brand purple is left as-is.
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
  --text: #16161c; --text-2: #545462; --text-3: #696975;
  --accent: {accent_l}; --accent-fg: #ffffff; --accent-soft: {soft_l};
  --ok: #087a50; --warn: #c2830a; --danger: #d63031;
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
    use std::collections::HashMap;

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

    #[test]
    fn contrast_formula_anchors() {
        let black_on_white = contrast_ratio("#000000", "#ffffff");
        assert!((black_on_white - 21.0).abs() < 0.05, "{black_on_white}");
        let same = contrast_ratio("#ffffff", "#ffffff");
        assert!((same - 1.0).abs() < 0.001, "{same}");
    }

    /// Light muted text and success green are normal-size text on the light
    /// surfaces they actually sit on (`--bg`, `--surface`, `--surface-2`).
    #[test]
    fn light_text_tokens_meet_wcag_aa() {
        let css = theme_css(&BrandConfig::product_default());
        let light = scope_decls(&css, "[data-theme=\"light\"]");
        let text3 = light.get("--text-3").expect("--text-3");
        let ok = light.get("--ok").expect("--ok");
        for surface in ["--bg", "--surface", "--surface-2"] {
            let bg = light.get(surface).expect(surface);
            let text_ratio = contrast_ratio(text3, bg);
            let ok_ratio = contrast_ratio(ok, bg);
            assert!(
                text_ratio >= 4.5,
                "light --text-3 {text3} on {surface} {bg} = {text_ratio:.2}"
            );
            assert!(
                ok_ratio >= 4.5,
                "light --ok {ok} on {surface} {bg} = {ok_ratio:.2}"
            );
        }
    }

    /// Dark body/muted/success text already clears AA. White on the product
    /// dark accent clears the 3:1 large-text/UI floor and not 4.5:1 normal
    /// text; white on the light accent does clear 4.5:1. Brand purple stays.
    #[test]
    fn dark_text_and_accent_foreground_pairing() {
        let brand = BrandConfig::product_default();
        let css = theme_css(&brand);
        let dark = scope_decls(&css, "[data-theme=\"dark\"]");
        let text3 = dark.get("--text-3").expect("--text-3");
        let ok = dark.get("--ok").expect("--ok");
        for surface in ["--bg", "--surface", "--surface-2"] {
            let bg = dark.get(surface).expect(surface);
            let text_ratio = contrast_ratio(text3, bg);
            let ok_ratio = contrast_ratio(ok, bg);
            assert!(
                text_ratio >= 4.5,
                "dark --text-3 {text3} on {surface} {bg} = {text_ratio:.2}"
            );
            assert!(
                ok_ratio >= 4.5,
                "dark --ok {ok} on {surface} {bg} = {ok_ratio:.2}"
            );
        }
        let dark_accent = contrast_ratio("#ffffff", &brand.accent_dark);
        let light_accent = contrast_ratio("#ffffff", &brand.accent_light);
        assert!(
            dark_accent >= 3.0,
            "white on {} = {dark_accent:.2}",
            brand.accent_dark
        );
        assert!(
            light_accent >= 4.5,
            "white on {} = {light_accent:.2}",
            brand.accent_light
        );
    }

    fn scope_decls(css: &str, scope: &str) -> HashMap<String, String> {
        let start = css.find(scope).unwrap_or_else(|| panic!("missing {scope}"));
        let after = &css[start + scope.len()..];
        let open = after.find('{').expect("scope brace");
        let close = after.find('}').expect("scope end");
        let mut map = HashMap::new();
        for part in after[open + 1..close].split(';') {
            let Some((key, value)) = part.split_once(':') else {
                continue;
            };
            let key = key.trim();
            if let Some(name) = key.strip_prefix("--") {
                if name.contains("--") {
                    continue;
                }
                let hex = value.split_whitespace().next().unwrap_or("").trim();
                if hex.starts_with('#') {
                    map.insert(format!("--{name}"), hex.to_string());
                }
            }
        }
        map
    }

    fn contrast_ratio(fg: &str, bg: &str) -> f64 {
        let l1 = relative_luminance(fg);
        let l2 = relative_luminance(bg);
        let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn relative_luminance(hex: &str) -> f64 {
        let (r, g, b) = parse_hex(hex);
        let lin = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    fn parse_hex(hex: &str) -> (u8, u8, u8) {
        let hex = hex.trim().trim_start_matches('#');
        assert_eq!(hex.len(), 6, "{hex}");
        let n = u32::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("bad hex {hex}"));
        (
            ((n >> 16) & 0xff) as u8,
            ((n >> 8) & 0xff) as u8,
            (n & 0xff) as u8,
        )
    }
}
