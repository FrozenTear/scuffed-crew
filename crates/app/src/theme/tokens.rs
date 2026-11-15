//! Design tokens: primitives, semantic roles, component roles and the look
//! (direction) that maps roles onto primitives, plus the brand accent roles.
//!
//! The CSS layers in `css/` are copied from the design system
//! (`design-system/tokens/*.css` and `themes/a-mint-refined.css`). The look
//! file is keyed on `[data-theme]` only, because that is how the app switches
//! theme (see `provider.rs`). Pages use semantic or component tokens, never
//! primitives or raw hex.

use crate::theme::brand::{self, AccentRoles, BrandConfig, STRATEGY_BRAND, Scheme};

const PRIMITIVES: &str = include_str!("css/primitives.css");
const SEMANTIC: &str = include_str!("css/semantic.css");
const COMPONENTS: &str = include_str!("css/components.css");
const LOOK_A: &str = include_str!("css/look_a.css");

/// One named look: a direction file that remaps the semantic roles onto
/// primitives for both theme scopes. A new look is a new CSS file and a new
/// `const`, not new page CSS.
#[derive(Debug, Clone, Copy)]
pub struct Template {
    pub id: &'static str,
    pub label: &'static str,
    /// `:root` defaults plus `[data-theme="dark"]` and `[data-theme="light"]`.
    pub css: &'static str,
}

/// Look A, Mint Refined: today's look tightened onto named roles.
pub const MINT_REFINED: Template = Template {
    id: "mint-refined",
    label: "Mint Refined",
    css: LOOK_A,
};

/// Every template, in menu order. The first is the fallback.
pub const TEMPLATES: [Template; 1] = [MINT_REFINED];

/// Look a template up by id. An unknown or empty id gives Mint Refined.
pub fn template_by_id(id: &str) -> Template {
    TEMPLATES
        .iter()
        .find(|t| t.id == id.trim())
        .copied()
        .unwrap_or(MINT_REFINED)
}

/// The full token stylesheet for the default look and one brand config.
pub fn theme_css(brand: &BrandConfig) -> String {
    theme_css_for(&MINT_REFINED, brand)
}

/// Load order: primitives, semantic, components, the look, then the derived
/// brand accent roles (they win over the look's mint defaults), then the
/// page base. The strategy scope gets the same derivation from its orange.
pub fn theme_css_for(template: &Template, brand: &BrandConfig) -> String {
    let strategy_dark = AccentRoles::derive(STRATEGY_BRAND, Scheme::Dark);
    let strategy_light = AccentRoles::derive(STRATEGY_BRAND, Scheme::Light);
    format!(
        r#"{PRIMITIVES}
{SEMANTIC}
{COMPONENTS}
{look}
/* Brand accent roles, derived in brand.rs (port of derive-accent.js). */
[data-theme="dark"] {{ {dark} }}
[data-theme="light"] {{ {light} }}
[data-theme="dark"] [data-accent="strategy"] {{ {strategy_dark} }}
[data-theme="light"] [data-accent="strategy"] {{ {strategy_light} }}

html, body {{
  /* --page-bg-* set from admin Site Settings when customized */
  background-color: var(--page-bg-color, var(--bg));
  background-image: var(--page-bg-image, none);
  background-size: cover;
  background-position: center;
  background-attachment: fixed;
  background-repeat: no-repeat;
  color: var(--text);
  font-family: var(--font-body); font-size: var(--text-md);
  font-variant-numeric: tabular-nums;
}}
"#,
        look = template.css,
        dark = brand.dark.css_decls(),
        light = brand.light.css_decls(),
        strategy_dark = strategy_dark.css_decls(),
        strategy_light = strategy_light.css_decls(),
    )
}

/// Concrete colors for the strategy canvas. The 2D context takes literal
/// strings, so the static planner colours live here and a test holds them to
/// the `--canvas-*` primitives in `css/primitives.css`. The planner field
/// stays dark in both themes.
pub const CANVAS_BG: &str = "#14141c";
pub const CANVAS_TILE_PLACEHOLDER: &str = "#2a2a3e";
pub const CANVAS_GRID_LOADING: &str = "#333333";
pub const CANVAS_TEXT_LOADING: &str = "#666666";
pub const STRATEGY_ACCENT: &str = brand::STRATEGY_BRAND;
/// Selection follows the brand and theme (`--sel-ring`, the solid accent), so
/// the canvas reads it from computed style instead of a literal.
pub const CANVAS_SELECTION_TOKEN: &str = "--canvas-selection";
pub const CANVAS_BADGE_BG: &str = "rgba(0,0,0,0.7)";
pub const CANVAS_WHITE: &str = "#ffffff";
pub const HP_SMALL_FILL: &str = "#ffeb3b";
pub const HP_SMALL_STROKE: &str = "#ffc107";
pub const HP_LARGE_FILL: &str = "#ff9800";
pub const HP_LARGE_STROKE: &str = "#f57c00";
pub const HP_GLOW: &str = "rgba(255,255,255,0.5)";
pub const CANVAS_MARKER_BORDER: &str = "#ffffff";
/// Hero chip fill for an id that is not on the roster. A neutral grey, so the
/// team ring around it stays visible.
pub const CANVAS_UNKNOWN_HERO_FILL: &str = "#6b7280";
/// Hero chip ring on a light fill when the team colour matches the fill.
pub const CANVAS_CHIP_RING_DARK: &str = "#111827";

/// Dark-theme page background (matches `[data-theme="dark"] --bg`).
pub const BG_DARK: &str = brand::DARK_SURFACES[0];
/// Product-default brand accents (matches `BrandConfig::product_default`).
pub const BRAND_ACCENT_DARK: &str = "#8f73ff";
pub const BRAND_ACCENT_LIGHT: &str = "#6d4aff";

/// Convenience for the app root.
pub fn theme_css_current() -> String {
    theme_css(&brand::current())
}

/// Every custom property a stylesheet resolves to for one theme scope, with
/// `var()` chains followed. Applies `:root`, `[data-theme]` and
/// `[data-theme="<theme>"]` rules in source order (they share specificity).
#[cfg(test)]
pub(crate) fn resolved_tokens(css: &str, theme: &str) -> std::collections::HashMap<String, String> {
    let mut text = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        text.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .split_once("*/")
            .map_or("", |(_, tail)| tail);
    }
    text.push_str(rest);

    let scoped = format!("[data-theme=\"{theme}\"]");
    let mut raw = std::collections::HashMap::new();
    for block in text.split('}') {
        let Some((selector, body)) = block.split_once('{') else {
            continue;
        };
        let applies = selector
            .split(',')
            .map(str::trim)
            .any(|s| s == ":root" || s == "[data-theme]" || s == scoped);
        if !applies {
            continue;
        }
        for decl in body.split(';') {
            if let Some((name, value)) = decl.split_once(':')
                && name.trim().starts_with("--")
            {
                raw.insert(name.trim().to_string(), value.trim().to_string());
            }
        }
    }

    fn resolve(
        raw: &std::collections::HashMap<String, String>,
        value: &str,
        depth: usize,
    ) -> String {
        let Some(inner) = value.strip_prefix("var(").and_then(|v| v.strip_suffix(')')) else {
            return value.to_string();
        };
        let (name, fallback) = inner.split_once(',').unwrap_or((inner, ""));
        match raw.get(name.trim()) {
            Some(next) if depth < 32 => resolve(raw, next, depth + 1),
            _ => fallback.trim().to_string(),
        }
    }
    raw.iter()
        .map(|(k, v)| (k.clone(), resolve(&raw, v, 0)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        BrandConfig, CANVAS_BG, CANVAS_TILE_PLACEHOLDER, HP_LARGE_FILL, HP_LARGE_STROKE,
        HP_SMALL_FILL, HP_SMALL_STROKE, MINT_REFINED, STRATEGY_ACCENT, TEMPLATES, resolved_tokens,
        template_by_id, theme_css, theme_css_for,
    };
    use crate::theme::brand::{DARK_SURFACES, LIGHT_SURFACES, contrast_ratio};

    const SURFACES: [&str; 4] = ["--bg", "--surface", "--surface-raised", "--surface-sunken"];

    fn live_mint() -> BrandConfig {
        BrandConfig::from_accents("#46d8a4", "#0ea66e")
    }

    fn get<'a>(map: &'a std::collections::HashMap<String, String>, name: &str) -> &'a str {
        map.get(name)
            .unwrap_or_else(|| panic!("missing {name}"))
            .as_str()
    }

    #[test]
    fn emits_every_layer_and_both_brand_scopes() {
        let brand = BrandConfig::product_default();
        let css = theme_css(&brand);
        for needle in [
            "--neutral-900:",
            "--accent-text:",
            "--button-primary-fg: var(--on-accent)",
            "[data-theme=\"dark\"] {",
            "[data-theme=\"light\"] {",
            "[data-theme=\"dark\"] [data-accent=\"strategy\"]",
            "[data-theme=\"light\"] [data-accent=\"strategy\"]",
            "--space-1:",
            "font-size: var(--text-md)",
        ] {
            assert!(css.contains(needle), "missing {needle}");
        }
        assert!(css.contains(&brand.dark.css_decls()));
        assert!(css.contains(&brand.light.css_decls()));
        assert!(
            !css.contains("data-direction=\"a\"]["),
            "look A keys on data-theme only"
        );
    }

    #[test]
    fn smallest_text_token_is_twelve_px() {
        let css = theme_css(&live_mint());
        for theme in ["dark", "light"] {
            let tokens = resolved_tokens(&css, theme);
            assert_eq!(get(&tokens, "--text-xs"), "12px");
            assert_eq!(get(&tokens, "--text-sm"), "13px");
            assert_eq!(get(&tokens, "--text-md"), "14px");
            assert_eq!(get(&tokens, "--radius-control"), "8px");
            assert_eq!(get(&tokens, "--radius-card"), "12px");
        }
    }

    /// Look A values from the semantic token table in `tokens.md`.
    #[test]
    fn look_a_matches_the_token_table() {
        let css = theme_css(&live_mint());
        let table: [(&str, &str, &str); 19] = [
            ("--bg", "#17171d", "#f7f7f9"),
            ("--surface", "#1f1f27", "#ffffff"),
            ("--surface-raised", "#282831", "#ffffff"),
            ("--surface-sunken", "#282831", "#f0f0f4"),
            ("--border", "#353541", "#e3e3e9"),
            ("--border-strong", "#72727f", "#888896"),
            ("--text", "#f4f4f8", "#16161c"),
            ("--text-muted", "#c1c1cd", "#545462"),
            ("--text-subtle", "#9696a3", "#696975"),
            ("--text-disabled", "#777784", "#888896"),
            ("--accent-text", "#46d8a4", "#087a50"),
            ("--accent-solid", "#46d8a4", "#087a50"),
            ("--on-accent", "#17171d", "#ffffff"),
            ("--win", "#46d8a4", "#087a50"),
            ("--loss", "#f06a6a", "#c81d24"),
            ("--warn", "#fbbf24", "#9b5f00"),
            ("--info", "#089fd7", "#0072b3"),
            ("--focus-ring", "#46d8a4", "#087a50"),
            ("--on-win", "#17171d", "#ffffff"),
        ];
        let dark = resolved_tokens(&css, "dark");
        let light = resolved_tokens(&css, "light");
        for (name, d, l) in table {
            assert_eq!(get(&dark, name), d, "dark {name}");
            assert_eq!(get(&light, name), l, "light {name}");
        }
    }

    /// The derivation in brand.rs uses the same four surfaces the look sets.
    #[test]
    fn brand_surfaces_match_look_a() {
        let css = theme_css(&live_mint());
        for (theme, expected) in [("dark", DARK_SURFACES), ("light", LIGHT_SURFACES)] {
            let tokens = resolved_tokens(&css, theme);
            let got: Vec<&str> = SURFACES.iter().map(|n| get(&tokens, n)).collect();
            assert_eq!(got, expected, "{theme}");
        }
    }

    /// Text roles clear AA on every text surface, disabled text and the
    /// strong border clear 3:1, and the derived accent pairs hold for the
    /// live mint, the product purple and the pending gray.
    #[test]
    fn every_scope_clears_contrast_floors() {
        for brand in [
            live_mint(),
            BrandConfig::product_default(),
            BrandConfig::pending(),
        ] {
            let css = theme_css(&brand);
            for theme in ["dark", "light"] {
                let tokens = resolved_tokens(&css, theme);
                for bg in SURFACES.iter().map(|n| get(&tokens, n)) {
                    for (name, min) in [
                        ("--text", 4.5),
                        ("--text-muted", 4.5),
                        ("--text-subtle", 4.5),
                        ("--accent-text", 4.5),
                        ("--link", 4.5),
                        ("--win", 4.5),
                        ("--loss", 4.5),
                        ("--warn", 4.5),
                        ("--info", 4.5),
                        ("--text-disabled", 3.0),
                        ("--border-strong", 3.0),
                        ("--focus-ring", 3.0),
                        ("--accent-solid", 3.0),
                    ] {
                        let fg = get(&tokens, name);
                        let ratio = contrast_ratio(fg, bg);
                        assert!(ratio >= min, "{theme} {name} {fg} on {bg} = {ratio:.2}");
                    }
                }
                for (fg, bg) in [
                    ("--on-accent", "--accent-solid"),
                    ("--skip-fg", "--skip-bg"),
                    ("--on-win", "--win"),
                    ("--on-loss", "--loss"),
                    ("--accent-text", "--accent-soft"),
                    ("--text", "--accent-soft"),
                    ("--win", "--win-soft"),
                    ("--loss", "--loss-soft"),
                    ("--warn", "--warn-soft"),
                    ("--info", "--info-soft"),
                    ("--text-muted", "--neutral-soft"),
                ] {
                    let (f, b) = (get(&tokens, fg), get(&tokens, bg));
                    let ratio = contrast_ratio(f, b);
                    assert!(ratio >= 4.5, "{theme} {fg} {f} on {bg} {b} = {ratio:.2}");
                }
            }
        }
    }

    /// Soft fills are opaque hex so computed style stays rgb().
    #[test]
    fn soft_fills_are_opaque() {
        let css = theme_css(&BrandConfig::product_default());
        for theme in ["dark", "light"] {
            let tokens = resolved_tokens(&css, theme);
            for name in [
                "--accent-soft",
                "--win-soft",
                "--loss-soft",
                "--warn-soft",
                "--info-soft",
                "--neutral-soft",
            ] {
                let value = get(&tokens, name);
                assert!(
                    value.len() == 7 && value.starts_with('#'),
                    "{theme} {name} = {value}"
                );
            }
        }
    }

    #[test]
    fn canvas_constants_match_the_primitives() {
        let css = theme_css(&live_mint());
        let tokens = resolved_tokens(&css, "dark");
        for (name, value) in [
            ("--canvas-bg", CANVAS_BG),
            ("--canvas-tile-placeholder", CANVAS_TILE_PLACEHOLDER),
            ("--canvas-hp-small-fill", HP_SMALL_FILL),
            ("--canvas-hp-small-stroke", HP_SMALL_STROKE),
            ("--canvas-hp-large-fill", HP_LARGE_FILL),
            ("--canvas-hp-large-stroke", HP_LARGE_STROKE),
            ("--orange-strategy", STRATEGY_ACCENT),
        ] {
            assert_eq!(get(&tokens, name), value, "{name}");
        }
        assert_eq!(
            get(&tokens, "--canvas-selection"),
            get(&tokens, "--accent-solid")
        );
    }

    /// The default look renders through `theme_css` and is pinned by a
    /// snapshot, so a refactor cannot move a byte of the sheet unnoticed.
    #[test]
    fn default_template_css_is_pinned() {
        let css = theme_css(&BrandConfig::product_default());
        assert_eq!(css, include_str!("mint_refined.golden.css"));
        assert_eq!(
            css,
            theme_css_for(&MINT_REFINED, &BrandConfig::product_default())
        );
    }

    #[test]
    fn unknown_template_ids_fall_back_to_mint_refined() {
        assert_eq!(template_by_id(" mint-refined ").id, "mint-refined");
        assert_eq!(template_by_id("").id, "mint-refined");
        assert_eq!(template_by_id("classic").id, "mint-refined");
    }

    #[test]
    fn template_ids_are_unique_and_mint_refined_is_first() {
        assert_eq!(TEMPLATES[0].id, MINT_REFINED.id);
        let mut ids: Vec<&str> = TEMPLATES.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), TEMPLATES.len());
    }
}
