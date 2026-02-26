use super::Theme;
use super::tokens::{ColorTokens, FontTokens};

/// The Scuffed Crew clan site theme.
///
/// Purple accent, warm muted text, Bebas Neue / Rajdhani / Source Sans 3 / DM Mono.
/// Extracted from `index.html` CSS variables.
pub fn scuffed_crew_theme() -> Theme {
    Theme {
        colors: ColorTokens {
            bg_void: "#08080c",
            bg_surface: "#0e0e14",
            bg_card: "#14141e",
            bg_card_alt: "#1a1a28",
            bg_elevated: "#20202e",
            accent: "#7c3aed",
            accent_soft: "rgba(124, 58, 237, 0.15)",
            accent_glow: "rgba(124, 58, 237, 0.25)",
            accent_bright: "#a78bfa",
            danger: "#d63031",
            success: "#00c853",
            warning: "#f0b232",
            info: "#4a9eff",
            text_bright: "#f0eee8",
            text_primary: "#ccc8c0",
            text_secondary: "#807a70",
            text_muted: "#504c44",
            border: "#2a2832",
            border_light: "#363440",
        },
        fonts: FontTokens {
            display_hero: "'Bebas Neue', sans-serif",
            display: "'Rajdhani', sans-serif",
            body: "'Source Sans 3', sans-serif",
            mono: "'DM Mono', monospace",
        },
    }
}

/// Overwatch strategy app theme.
///
/// Orange accent, cool slate text, Orbitron / Exo 2 / JetBrains Mono.
/// Extracted from `assets/styles.css` CSS variables.
pub fn strategy_app_theme() -> Theme {
    Theme {
        colors: ColorTokens {
            bg_void: "#05070f",
            bg_surface: "#0a0e1a",
            bg_card: "#111827",
            bg_card_alt: "#1a2332",
            bg_elevated: "#243044",
            accent: "#ff6a00",
            accent_soft: "rgba(255, 106, 0, 0.15)",
            accent_glow: "rgba(255, 106, 0, 0.4)",
            accent_bright: "#ff9500",
            danger: "#ef4444",
            success: "#22c55e",
            warning: "#fbbf24",
            info: "#00f0ff",
            text_bright: "#ffffff",
            text_primary: "#e2e8f0",
            text_secondary: "#94a3b8",
            text_muted: "#64748b",
            border: "#1e293b",
            border_light: "#334155",
        },
        fonts: FontTokens {
            display_hero: "'Orbitron', sans-serif",
            display: "'Exo 2', sans-serif",
            body: "'Exo 2', sans-serif",
            mono: "'JetBrains Mono', monospace",
        },
    }
}
