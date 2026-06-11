//! The single source of brand-configurable values. Hardcoded to Scuffed Crew
//! defaults today; a later sub-project swaps `current()` to read /api/settings.
//! Nothing else in the app should hardcode brand colors — they consume this.

#[derive(Clone, Copy, PartialEq)]
pub struct BrandConfig {
    /// Accent in dark mode (hex).
    pub accent_dark: &'static str,
    /// Accent in light mode (hex).
    pub accent_light: &'static str,
    /// Soft accent fill, dark mode (rgba).
    pub accent_soft_dark: &'static str,
    /// Soft accent fill, light mode (rgba).
    pub accent_soft_light: &'static str,
}

impl BrandConfig {
    pub const SCUFFED: Self = Self {
        accent_dark: "#8f73ff",
        accent_light: "#6d4aff",
        accent_soft_dark: "rgba(143,115,255,.17)",
        accent_soft_light: "rgba(109,74,255,.10)",
    };
}

/// The active brand config. Single call-site that the settings sub-project rewires.
pub fn current() -> BrandConfig {
    BrandConfig::SCUFFED
}
