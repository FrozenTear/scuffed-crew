use crate::theme::brand::{self, BrandConfig};

/// Colours for one theme scope (dark or light). The brand accent is not here:
/// the org picks it, so it is passed in separately.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: &'static str,
    pub surface: &'static str,
    pub surface_2: &'static str,
    pub border: &'static str,
    pub text: &'static str,
    pub text_2: &'static str,
    pub text_3: &'static str,
    pub accent_fg: &'static str,
    pub ok: &'static str,
    pub warn: &'static str,
    pub danger: &'static str,
}

/// One named look: fonts, the smallest text size, which theme it starts in,
/// and a palette for each theme scope. A new template is a new `const`, not
/// new page CSS.
#[derive(Debug, Clone, Copy)]
pub struct Template {
    pub id: &'static str,
    pub label: &'static str,
    pub font_head: &'static str,
    pub font_body: &'static str,
    pub font_mono: &'static str,
    pub text_xs: &'static str,
    /// `"dark"` or `"light"`. Not read by the app yet.
    pub default_theme: &'static str,
    pub dark: Palette,
    pub light: Palette,
}

/// Today's look. `theme_css` renders this one, so nothing on the site changes.
pub const CLASSIC: Template = Template {
    id: "classic",
    label: "Classic",
    font_head: "'Space Grotesk', system-ui, sans-serif",
    font_body: "'Inter', system-ui, sans-serif",
    font_mono: "'JetBrains Mono', ui-monospace, monospace",
    text_xs: "12px",
    default_theme: "light",
    dark: Palette {
        bg: "#17171d",
        surface: "#1f1f27",
        surface_2: "#282831",
        border: "#353541",
        text: "#f4f4f8",
        text_2: "#c1c1cd",
        text_3: "#9696a3",
        accent_fg: "#ffffff",
        ok: "#46d8a4",
        warn: "#fbbf24",
        danger: "#f06a6a",
    },
    light: Palette {
        bg: "#f7f7f9",
        surface: "#ffffff",
        surface_2: "#f0f0f4",
        border: "#e3e3e9",
        text: "#16161c",
        text_2: "#545462",
        text_3: "#696975",
        accent_fg: "#ffffff",
        ok: "#087a50",
        warn: "#c2830a",
        danger: "#d63031",
    },
};

/// The tracker app's look: dark and purple. Defined but not selectable yet
/// (no setting picks it). Surface 2, border and the text greys are derived
/// from Classic dark until the tracker's own values are confirmed. Plex and
/// Urbanist are not loaded by the site yet, so they fall back to the system
/// font.
pub const APP: Template = Template {
    id: "app",
    label: "App",
    font_head: "'Urbanist', system-ui, sans-serif",
    font_body: "'IBM Plex Sans', system-ui, sans-serif",
    font_mono: "'JetBrains Mono', ui-monospace, monospace",
    text_xs: "12px",
    default_theme: "dark",
    dark: Palette {
        bg: "#121218",
        surface: "#1c1c25",
        surface_2: "#282831",
        border: "#353541",
        text: "#f4f4f8",
        text_2: "#c1c1cd",
        text_3: "#9696a3",
        accent_fg: "#ffffff",
        ok: "#46d8a4",
        warn: "#f5b84a",
        danger: "#ff5c7a",
    },
    light: CLASSIC.light,
};

/// Every template, in menu order. The first is the fallback.
pub const TEMPLATES: [Template; 2] = [CLASSIC, APP];

/// Look a template up by id. An unknown or empty id gives Classic.
pub fn template_by_id(id: &str) -> Template {
    TEMPLATES
        .iter()
        .find(|t| t.id == id.trim())
        .copied()
        .unwrap_or(CLASSIC)
}

/// Build the full design-system stylesheet (primitives + both semantic scopes)
/// for the Classic template. This is what the app renders today.
pub fn theme_css(brand: &BrandConfig) -> String {
    theme_css_for(&CLASSIC, brand)
}

/// Build the stylesheet for one template and one brand config. The ONLY place
/// raw color/scale literals live.
///
/// Light `--text-3` and `--ok` are normal-size text. They must clear WCAG AA
/// 4.5:1 on `--bg`, `--surface`, and `--surface-2` (the darkest light surface).
/// Dark `--accent-fg` on the product accent is a brand pair: it clears the 3:1
/// large-text/UI floor and misses 4.5:1 for normal text. The light accent with
/// the same white foreground clears 4.5:1. Brand purple is left as-is.
pub fn theme_css_for(template: &Template, brand: &BrandConfig) -> String {
    let d = &template.dark;
    let l = &template.light;
    format!(
        r#"
:root {{
  --font-head: {font_head};
  --font-body: {font_body};
  --font-mono: {font_mono};

  --text-xs: {text_xs};  --text-sm: 12.5px; --text-base: 14px; --text-lg: 15.5px;
  --text-xl: 18px;  --text-2xl: 21px;  --text-3xl: 30px;

  --space-1: 4px;  --space-2: 8px;  --space-3: 12px; --space-4: 16px;
  --space-6: 24px; --space-8: 32px; --space-12: 48px;

  --radius-sm: 7px; --radius-md: 9px; --radius-lg: 12px; --radius-pill: 999px;

  --overlay: rgba(0,0,0,0.6);

  /* Chart palette (dark values; light scope overrides). CVD-validated against
     the dark surface (--surface). See docs/notes/stats-ui-w2-validation.md.
     Snap-tuned to the dark lightness band; re-run the validator if edited. */
  --chart-1: #8f73ff; --chart-2: #15ac7d; --chart-3: #b98a02; --chart-4: #ca474c; --chart-5: #089fd7; --chart-6: #984ab2;
  /* Winrate bars: two-pole diverging encoding (W5a). Cool up-pole (> 50%),
     warm down-pole (< 50%); exactly-50% and sub-min-games rows take the
     neutral --text-3 midpoint. Chart poles, NOT status tokens. Pair
     CVD-validated vs both surfaces. See docs/notes/stats-ui-w2-validation.md
     (W5a section); re-run the validator if edited. */
  --chart-wr-up: #089fd7; --chart-wr-down: #aa5000;
}}

[data-theme="dark"] {{
  --bg: {d_bg}; --surface: {d_surface}; --surface-2: {d_surface_2}; --border: {d_border};
  --text: {d_text}; --text-2: {d_text_2}; --text-3: {d_text_3};
  --accent: {accent_d}; --accent-fg: {d_accent_fg}; --accent-soft: {soft_d};
  --ok: {d_ok}; --warn: {d_warn}; --danger: {d_danger};
}}

[data-theme="light"] {{
  --bg: {l_bg}; --surface: {l_surface}; --surface-2: {l_surface_2}; --border: {l_border};
  --text: {l_text}; --text-2: {l_text_2}; --text-3: {l_text_3};
  --accent: {accent_l}; --accent-fg: {l_accent_fg}; --accent-soft: {soft_l};
  --ok: {l_ok}; --warn: {l_warn}; --danger: {l_danger};
  /* CVD-validated against #ffffff. See docs/notes/stats-ui-w2-validation.md. */
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
        font_head = template.font_head,
        font_body = template.font_body,
        font_mono = template.font_mono,
        text_xs = template.text_xs,
        d_bg = d.bg,
        d_surface = d.surface,
        d_surface_2 = d.surface_2,
        d_border = d.border,
        d_text = d.text,
        d_text_2 = d.text_2,
        d_text_3 = d.text_3,
        d_accent_fg = d.accent_fg,
        d_ok = d.ok,
        d_warn = d.warn,
        d_danger = d.danger,
        l_bg = l.bg,
        l_surface = l.surface,
        l_surface_2 = l.surface_2,
        l_border = l.border,
        l_text = l.text,
        l_text_2 = l.text_2,
        l_text_3 = l.text_3,
        l_accent_fg = l.accent_fg,
        l_ok = l.ok,
        l_warn = l.warn,
        l_danger = l.danger,
        accent_d = brand.accent_dark.as_str(),
        accent_l = brand.accent_light.as_str(),
        soft_d = brand.accent_soft_dark.as_str(),
        soft_l = brand.accent_soft_light.as_str(),
    )
}

/// Concrete colors for the strategy canvas (2D context needs literal strings, not CSS vars).
pub const CANVAS_BG: &str = "#14141c";
pub const CANVAS_TILE_PLACEHOLDER: &str = "#2a2a3e";
pub const CANVAS_GRID_LOADING: &str = "#333333";
pub const CANVAS_TEXT_LOADING: &str = "#666666";
pub const STRATEGY_ACCENT: &str = "#ff7a1a";
pub const CANVAS_SELECTION_COLOR: &str = "#00ff00";
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
pub const BG_DARK: &str = "#17171d";
/// Product-default brand accents (matches `BrandConfig::product_default`).
pub const BRAND_ACCENT_DARK: &str = "#8f73ff";
pub const BRAND_ACCENT_LIGHT: &str = "#6d4aff";

/// Convenience for the app root.
pub fn theme_css_current() -> String {
    theme_css(&brand::current())
}

#[cfg(test)]
pub(crate) fn scope_decls(css: &str, scope: &str) -> std::collections::HashMap<String, String> {
    let start = css.find(scope).unwrap_or_else(|| panic!("missing {scope}"));
    let after = &css[start + scope.len()..];
    let open = after.find('{').expect("scope brace");
    let close = after.find('}').expect("scope end");
    let mut map = std::collections::HashMap::new();
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

#[cfg(test)]
pub(crate) fn contrast_ratio(fg: &str, bg: &str) -> f64 {
    let l1 = relative_luminance(fg);
    let l2 = relative_luminance(bg);
    let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
pub(crate) fn relative_luminance(hex: &str) -> f64 {
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

#[cfg(test)]
pub(crate) fn parse_hex(hex: &str) -> (u8, u8, u8) {
    let hex = hex.trim().trim_start_matches('#');
    assert_eq!(hex.len(), 6, "{hex}");
    let n = u32::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("bad hex {hex}"));
    (
        ((n >> 16) & 0xff) as u8,
        ((n >> 8) & 0xff) as u8,
        (n & 0xff) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::{BrandConfig, contrast_ratio, scope_decls, theme_css};

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
    fn smallest_text_token_is_twelve_px() {
        let css = theme_css(&BrandConfig::product_default());
        assert!(css.contains("--text-xs: 12px;"), "{css}");
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

    use super::{APP, CLASSIC, TEMPLATES, template_by_id, theme_css_for};

    /// The refactor must not move one byte of the stylesheet the site renders.
    /// The golden file was generated from `theme_css(&BrandConfig::product_default())`
    /// before the template data existed.
    #[test]
    fn classic_template_css_is_unchanged() {
        let css = theme_css(&BrandConfig::product_default());
        assert_eq!(css, include_str!("classic.golden.css"));
        assert_eq!(
            css,
            theme_css_for(&CLASSIC, &BrandConfig::product_default())
        );
    }

    #[test]
    fn unknown_template_ids_fall_back_to_classic() {
        assert_eq!(template_by_id("app").id, "app");
        assert_eq!(template_by_id(" classic ").id, "classic");
        assert_eq!(template_by_id("").id, "classic");
        assert_eq!(template_by_id("nope").id, "classic");
    }

    #[test]
    fn template_ids_are_unique_and_classic_is_first() {
        assert_eq!(TEMPLATES[0].id, CLASSIC.id);
        let mut ids: Vec<&str> = TEMPLATES.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), TEMPLATES.len());
    }

    /// Every template keeps the same AA floor Classic is held to, in both scopes.
    #[test]
    fn every_template_text_clears_wcag_aa() {
        for template in TEMPLATES {
            let css = theme_css_for(&template, &BrandConfig::product_default());
            for scope in ["[data-theme=\"dark\"]", "[data-theme=\"light\"]"] {
                let decls = scope_decls(&css, scope);
                for text in ["--text", "--text-2", "--text-3", "--ok"] {
                    for surface in ["--bg", "--surface", "--surface-2"] {
                        let fg = decls.get(text).expect(text);
                        let bg = decls.get(surface).expect(surface);
                        let ratio = contrast_ratio(fg, bg);
                        assert!(
                            ratio >= 4.5,
                            "{} {scope} {text} {fg} on {surface} {bg} = {ratio:.2}",
                            template.id
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn app_template_is_dark_with_the_app_palette_and_a_twelve_px_floor() {
        assert_eq!(APP.default_theme, "dark");
        let css = theme_css_for(&APP, &BrandConfig::product_default());
        let dark = scope_decls(&css, "[data-theme=\"dark\"]");
        assert_eq!(dark.get("--bg").map(String::as_str), Some("#121218"));
        assert_eq!(dark.get("--surface").map(String::as_str), Some("#1c1c25"));
        assert_eq!(dark.get("--danger").map(String::as_str), Some("#ff5c7a"));
        assert!(css.contains("--text-xs: 12px;"), "{css}");
    }
}
