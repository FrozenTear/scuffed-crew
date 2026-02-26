pub mod tokens;
pub mod presets;

use leptos::prelude::*;
use tokens::{ColorTokens, FontTokens};

/// Complete theme definition for an app.
#[derive(Debug, Clone)]
pub struct Theme {
    pub colors: ColorTokens,
    pub fonts: FontTokens,
}

/// Generate a CSS `<style>` block that sets CSS custom properties from the theme.
pub fn generate_css_vars(theme: &Theme) -> String {
    let c = &theme.colors;
    let f = &theme.fonts;
    format!(
        ":root {{\
\n  --bg-void: {};\
\n  --bg-surface: {};\
\n  --bg-card: {};\
\n  --bg-card-alt: {};\
\n  --bg-elevated: {};\
\n  --accent: {};\
\n  --accent-soft: {};\
\n  --accent-glow: {};\
\n  --accent-bright: {};\
\n  --danger: {};\
\n  --success: {};\
\n  --warning: {};\
\n  --info: {};\
\n  --text-bright: {};\
\n  --text-primary: {};\
\n  --text-secondary: {};\
\n  --text-muted: {};\
\n  --border: {};\
\n  --border-light: {};\
\n  --font-display-hero: {};\
\n  --font-display: {};\
\n  --font-body: {};\
\n  --font-mono: {};\
\n}}",
        c.bg_void,
        c.bg_surface,
        c.bg_card,
        c.bg_card_alt,
        c.bg_elevated,
        c.accent,
        c.accent_soft,
        c.accent_glow,
        c.accent_bright,
        c.danger,
        c.success,
        c.warning,
        c.info,
        c.text_bright,
        c.text_primary,
        c.text_secondary,
        c.text_muted,
        c.border,
        c.border_light,
        f.display_hero,
        f.display,
        f.body,
        f.mono,
    )
}

/// Provides the theme to the component tree via Leptos context
/// and injects CSS custom properties into the DOM.
#[component]
pub fn ThemeProvider(theme: Theme, children: Children) -> impl IntoView {
    let css = generate_css_vars(&theme);
    provide_context(theme);

    view! {
        <style>{css}</style>
        {children()}
    }
}

/// Hook to access the current theme from context.
pub fn use_theme() -> Theme {
    expect_context::<Theme>()
}
