//! Self-hosted type: Urbanist (display), IBM Plex Sans (body) and
//! JetBrains Mono (codes). Variable woff2 files, subset to Latin, live in
//! `assets/fonts/` (converted from the design system TTFs).
//!
//! Each text family has a metric-matched local fallback (`Urbanist
//! Fallback`, `IBM Plex Sans Fallback`, named in `--font-family-*`) so the
//! swap from Arial to the web font does not shift layout. The overrides were
//! measured against Arial-metric Liberation Sans with an English letter
//! frequency sample: Urbanist at 400 and 700 (against Arial Bold), Plex at
//! 400. `size-adjust` matches the average advance; ascent and descent are the
//! font's hhea metrics divided by that adjust.

use dioxus::prelude::*;

/// Hash-free so `index.html` can preload the display face by a stable path.
static URBANIST: manganis::Asset = asset!(
    "/assets/fonts/urbanist.woff2",
    manganis::AssetOptions::builder().with_hash_suffix(false)
);
static IBM_PLEX_SANS: manganis::Asset = asset!(
    "/assets/fonts/ibm-plex-sans.woff2",
    manganis::AssetOptions::builder().with_hash_suffix(false)
);
static JETBRAINS_MONO: manganis::Asset = asset!(
    "/assets/fonts/jetbrains-mono.woff2",
    manganis::AssetOptions::builder().with_hash_suffix(false)
);

/// The path `index.html` preloads. A test holds the shell to it.
pub const DISPLAY_PRELOAD_HREF: &str = "/assets/urbanist.woff2";

/// Glyphs kept by the subset. Anything else falls through to the fallback.
const UNICODE_RANGE: &str = "U+0000-017F, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+2000-206F, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD";

/// `@font-face` rules for the three families and their fallbacks.
pub fn fonts_css() -> String {
    fonts_css_with(
        &URBANIST.to_string(),
        &IBM_PLEX_SANS.to_string(),
        &JETBRAINS_MONO.to_string(),
    )
}

fn fonts_css_with(urbanist: &str, plex: &str, mono: &str) -> String {
    format!(
        r#"@font-face {{
  font-family: "Urbanist";
  src: url("{urbanist}") format("woff2");
  font-weight: 100 900; font-style: normal; font-display: swap;
  unicode-range: {UNICODE_RANGE};
}}
@font-face {{
  font-family: "IBM Plex Sans";
  src: url("{plex}") format("woff2");
  font-weight: 100 700; font-stretch: 75% 100%; font-style: normal; font-display: swap;
  unicode-range: {UNICODE_RANGE};
}}
@font-face {{
  font-family: "JetBrains Mono";
  src: url("{mono}") format("woff2");
  font-weight: 100 800; font-style: normal; font-display: swap;
  unicode-range: {UNICODE_RANGE};
}}
@font-face {{
  font-family: "Urbanist Fallback";
  src: local("Arial"), local("ArialMT"), local("Liberation Sans"), local("Helvetica");
  font-weight: 100 500;
  size-adjust: 99.86%; ascent-override: 95.13%; descent-override: 25.04%; line-gap-override: 0%;
}}
@font-face {{
  font-family: "Urbanist Fallback";
  src: local("Arial Bold"), local("Arial-BoldMT"), local("Liberation Sans Bold"), local("Helvetica Bold");
  font-weight: 600 900;
  size-adjust: 95.69%; ascent-override: 99.27%; descent-override: 26.12%; line-gap-override: 0%;
}}
@font-face {{
  font-family: "IBM Plex Sans Fallback";
  src: local("Arial"), local("ArialMT"), local("Liberation Sans"), local("Helvetica");
  size-adjust: 101.23%; ascent-override: 101.25%; descent-override: 27.17%; line-gap-override: 0%;
}}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::{DISPLAY_PRELOAD_HREF, fonts_css_with};

    #[test]
    fn every_face_swaps_and_has_a_metric_matched_fallback() {
        let css = fonts_css_with("/u.woff2", "/p.woff2", "/m.woff2");
        assert_eq!(css.matches("font-display: swap").count(), 3);
        for family in ["\"Urbanist\"", "\"IBM Plex Sans\"", "\"JetBrains Mono\""] {
            assert!(css.contains(&format!("font-family: {family};")), "{family}");
        }
        assert_eq!(css.matches("font-family: \"Urbanist Fallback\"").count(), 2);
        assert_eq!(
            css.matches("font-family: \"IBM Plex Sans Fallback\"")
                .count(),
            1
        );
        assert_eq!(css.matches("size-adjust:").count(), 3);
        assert_eq!(css.matches("ascent-override:").count(), 3);
        assert!(css.contains("url(\"/u.woff2\") format(\"woff2\")"));
        assert!(!css.contains("googleapis"));
    }

    /// The fallback names in `@font-face` are the ones the type tokens use.
    #[test]
    fn fallback_names_match_the_family_tokens() {
        let primitives = include_str!("css/primitives.css");
        assert!(primitives.contains("\"Urbanist\", \"Urbanist Fallback\""));
        assert!(primitives.contains("\"IBM Plex Sans\", \"IBM Plex Sans Fallback\""));
    }

    #[test]
    fn shell_preloads_the_display_face() {
        let html = include_str!("../../index.html");
        let tag = format!(
            "<link rel=\"preload\" as=\"font\" type=\"font/woff2\" href=\"{DISPLAY_PRELOAD_HREF}\" crossorigin />"
        );
        assert!(html.contains(&tag), "{tag}");
        assert!(
            std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/fonts/urbanist.woff2"
            ))
            .exists()
        );
    }
}
