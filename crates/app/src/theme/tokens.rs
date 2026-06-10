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
}}

html, body {{
  background: var(--bg); color: var(--text);
  font-family: var(--font-body); font-size: var(--text-base);
}}
"#,
        accent_d = brand.accent_dark,
        accent_l = brand.accent_light,
        soft_d = brand.accent_soft_dark,
        soft_l = brand.accent_soft_light,
    )
}

/// Convenience for the app root.
pub fn theme_css_current() -> String {
    theme_css(&brand::current())
}

#[cfg(test)]
mod tests {
    use super::{BrandConfig, theme_css};

    #[test]
    fn emits_both_theme_scopes_and_uses_brand_accent() {
        let brand = BrandConfig::SCUFFED;
        let css = theme_css(&brand);
        assert!(css.contains("[data-theme=\"dark\"]"));
        assert!(css.contains("[data-theme=\"light\"]"));
        assert!(css.contains(brand.accent_dark));
        assert!(css.contains(brand.accent_light));
        assert!(css.contains("--bg:"));
        assert!(css.contains("--text-2:"));
        assert!(css.contains("--space-1:"));
    }
}
