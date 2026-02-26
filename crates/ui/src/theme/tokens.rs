/// Color tokens used across the theme system.
///
/// Every value is a CSS color string (hex, rgba, etc.) that gets injected
/// as a CSS custom property by `ThemeProvider`.
#[derive(Debug, Clone)]
pub struct ColorTokens {
    pub bg_void: &'static str,
    pub bg_surface: &'static str,
    pub bg_card: &'static str,
    pub bg_card_alt: &'static str,
    pub bg_elevated: &'static str,
    pub accent: &'static str,
    pub accent_soft: &'static str,
    pub accent_glow: &'static str,
    pub accent_bright: &'static str,
    pub danger: &'static str,
    pub success: &'static str,
    pub warning: &'static str,
    pub info: &'static str,
    pub text_bright: &'static str,
    pub text_primary: &'static str,
    pub text_secondary: &'static str,
    pub text_muted: &'static str,
    pub border: &'static str,
    pub border_light: &'static str,
}

/// Font tokens used across the theme system.
///
/// Each value is a CSS `font-family` declaration.
#[derive(Debug, Clone)]
pub struct FontTokens {
    /// Hero/display headings (largest)
    pub display_hero: &'static str,
    /// Section headings
    pub display: &'static str,
    /// Body text
    pub body: &'static str,
    /// Code / monospace
    pub mono: &'static str,
}
