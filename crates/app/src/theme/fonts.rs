//! Self-hosted type: Urbanist (display), IBM Plex Sans (body) and
//! JetBrains Mono (codes). Variable woff2 files live in `assets/fonts/`.
//!
//! Licensing (SIL OFL 1.1): every woff2 keeps its copyright and licence
//! name records (IDs 0, 13, 14, and 7 where the source has one), and the
//! three licence texts ship next to the fonts in `dist/assets/` through
//! `asset!()`.
//!
//! - Urbanist and JetBrains Mono declare no Reserved Font Name, so they ship
//!   as Latin subsets of the design system TTFs (`Urbanist-VariableFont_wght.ttf`,
//!   `JetBrainsMono-VariableFont_wght.ttf`), made with fontTools 4.68:
//!   `pyftsubset <ttf> --unicodes=<UNICODE_RANGE> --layout-features='*'
//!   --name-IDs='*' --flavor=woff2`.
//! - IBM Plex Sans reserves the name "Plex", so it is never subset or
//!   otherwise changed here. The three Plex files are IBM's own split woff2
//!   files, byte for byte, from the npm package `@ibm/plex-sans-variable`
//!   0.2.0 (github.com/IBM/plex release `@ibm/plex-sans-variable@0.2.0`),
//!   `fonts/split/woff2/`: `IBM Plex Sans Var-Roman-Latin1.woff2`,
//!   `IBM Plex Sans Var-Roman-Latin2.woff2` and
//!   `IBM Plex Sans Var-Roman-Pi.woff2`, renamed only (no spaces in URLs).
//!   Their `unicode-range` values are IBM's, from the package's
//!   `IBM Plex Sans Var-Roman.css`. `ibm-plex-sans-OFL.txt` is the
//!   package's `LICENSE.txt`, byte for byte.
//!
//! Each text family has a metric-matched local fallback (`Urbanist
//! Fallback`, `IBM Plex Sans Fallback`, named in `--font-family-*`) so the
//! swap from Arial to the web font does not shift layout. The overrides were
//! measured against Arial-metric Liberation Sans with an English letter
//! frequency sample: Urbanist at 400 and 700 (against Arial Bold), Plex at
//! 400. `size-adjust` matches the average advance; ascent and descent are the
//! font's hhea metrics divided by that adjust. IBM's Plex build has the same
//! hhea metrics and advances as the design system TTF they were measured on.

use dioxus::prelude::*;

/// Hash-free so `index.html` can preload the display face by a stable path.
static URBANIST: manganis::Asset = asset!(
    "/assets/fonts/urbanist.woff2",
    manganis::AssetOptions::builder().with_hash_suffix(false)
);
static PLEX_LATIN1: manganis::Asset = asset!(
    "/assets/fonts/ibm-plex-sans-var-roman-latin1.woff2",
    manganis::AssetOptions::builder().with_hash_suffix(false)
);
static PLEX_LATIN2: manganis::Asset = asset!(
    "/assets/fonts/ibm-plex-sans-var-roman-latin2.woff2",
    manganis::AssetOptions::builder().with_hash_suffix(false)
);
static PLEX_PI: manganis::Asset = asset!(
    "/assets/fonts/ibm-plex-sans-var-roman-pi.woff2",
    manganis::AssetOptions::builder().with_hash_suffix(false)
);
static JETBRAINS_MONO: manganis::Asset = asset!(
    "/assets/fonts/jetbrains-mono.woff2",
    manganis::AssetOptions::builder().with_hash_suffix(false)
);

/// OFL 1.1 condition 2: the licence travels with every copy of the fonts.
static URBANIST_OFL: manganis::Asset = asset!(
    "/assets/fonts/urbanist-OFL.txt",
    manganis::AssetOptions::builder().with_hash_suffix(false)
);
static IBM_PLEX_SANS_OFL: manganis::Asset = asset!(
    "/assets/fonts/ibm-plex-sans-OFL.txt",
    manganis::AssetOptions::builder().with_hash_suffix(false)
);
static JETBRAINS_MONO_OFL: manganis::Asset = asset!(
    "/assets/fonts/jetbrains-mono-OFL.txt",
    manganis::AssetOptions::builder().with_hash_suffix(false)
);

/// Licence files, relative to the crate root. A test holds each one to the
/// OFL 1.1 text.
pub const LICENCE_FILES: [&str; 3] = [
    "assets/fonts/urbanist-OFL.txt",
    "assets/fonts/ibm-plex-sans-OFL.txt",
    "assets/fonts/jetbrains-mono-OFL.txt",
];

/// The path `index.html` preloads. A test holds the shell to it.
pub const DISPLAY_PRELOAD_HREF: &str = "/assets/urbanist.woff2";

/// Glyphs kept by the Urbanist and JetBrains Mono subsets. Anything else
/// falls through to the fallback.
const UNICODE_RANGE: &str = "U+0000-017F, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+2000-206F, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD";

/// IBM's ranges for its Plex split files (`IBM Plex Sans Var-Roman.css`).
const PLEX_LATIN1_RANGE: &str = "U+0000, U+000D, U+0020-007E, U+00A0-00FF, U+0131, U+0152-0153, U+02C6, U+02DA, U+02DC, U+2013-2014, U+2018-201A, U+201C-201E, U+2020-2022, U+2026, U+2030, U+2039-203A, U+2044, U+20AC, U+2122, U+2212, U+FB01-FB02";
const PLEX_LATIN2_RANGE: &str = "U+0100-0101, U+0104-0130, U+0132-0151, U+0154-017F, U+018F, U+0192, U+01A0-01A1, U+01AF-01B0, U+01FA-01FF, U+0218-021B, U+0237, U+0259, U+1E80-1E85, U+1E9E, U+20A1, U+20A4, U+20A6, U+20A8-20AA, U+20AD-20AE, U+20B1-20B2, U+20B4-20B5, U+20B8-20BA, U+20BD, U+20BF";
const PLEX_PI_RANGE: &str = "U+0E3F, U+2000-200D, U+2015, U+2028-2029, U+202F, U+2032-2033, U+2070, U+2074-2079, U+2080-2089, U+2113, U+2116, U+2126, U+212E, U+2150-2151, U+2153-215E, U+2190-2199, U+21A9-21AA, U+21B0-21B3, U+21B6-21B7, U+21BA-21BB, U+21C4, U+21C6, U+2202, U+2206, U+220F, U+2211, U+2215, U+221A, U+221E, U+222B, U+2248, U+2260, U+2264-2265, U+25CA, U+2713, U+274C, U+2B0E-2B11, U+ECE0, U+EFCC, U+FEFF, U+FFFD";

/// Served font and licence URLs, as `dx` resolves them.
struct FontUrls<'a> {
    urbanist: &'a str,
    plex: [&'a str; 3],
    mono: &'a str,
    licences: [&'a str; 3],
}

/// `@font-face` rules for the three families and their fallbacks.
pub fn fonts_css() -> String {
    let (urbanist, latin1, latin2, pi, mono) = (
        URBANIST.to_string(),
        PLEX_LATIN1.to_string(),
        PLEX_LATIN2.to_string(),
        PLEX_PI.to_string(),
        JETBRAINS_MONO.to_string(),
    );
    let (urbanist_ofl, plex_ofl, mono_ofl) = (
        URBANIST_OFL.to_string(),
        IBM_PLEX_SANS_OFL.to_string(),
        JETBRAINS_MONO_OFL.to_string(),
    );
    fonts_css_with(&FontUrls {
        urbanist: &urbanist,
        plex: [&latin1, &latin2, &pi],
        mono: &mono,
        licences: [&urbanist_ofl, &plex_ofl, &mono_ofl],
    })
}

fn fonts_css_with(urls: &FontUrls) -> String {
    let FontUrls {
        urbanist,
        plex: [latin1, latin2, pi],
        mono,
        licences: [urbanist_ofl, plex_ofl, mono_ofl],
    } = urls;
    let plex_face = |src: &str, range: &str| {
        format!(
            r#"@font-face {{
  font-family: "IBM Plex Sans";
  src: url("{src}") format("woff2");
  font-weight: 100 700; font-style: normal; font-display: swap;
  unicode-range: {range};
}}
"#
        )
    };
    let plex = [
        plex_face(latin1, PLEX_LATIN1_RANGE),
        plex_face(latin2, PLEX_LATIN2_RANGE),
        plex_face(pi, PLEX_PI_RANGE),
    ]
    .concat();
    format!(
        r#"/* Fonts under the SIL Open Font License 1.1: Urbanist {urbanist_ofl}, IBM Plex Sans {plex_ofl}, JetBrains Mono {mono_ofl} */
@font-face {{
  font-family: "Urbanist";
  src: url("{urbanist}") format("woff2");
  font-weight: 100 900; font-style: normal; font-display: swap;
  unicode-range: {UNICODE_RANGE};
}}
{plex}@font-face {{
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
    use super::{DISPLAY_PRELOAD_HREF, FontUrls, LICENCE_FILES, fonts_css_with};

    const URLS: FontUrls<'static> = FontUrls {
        urbanist: "/u.woff2",
        plex: ["/p1.woff2", "/p2.woff2", "/p3.woff2"],
        mono: "/m.woff2",
        licences: ["/u-OFL.txt", "/p-OFL.txt", "/m-OFL.txt"],
    };

    fn crate_path(rel: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
    }

    #[test]
    fn every_face_swaps_and_has_a_metric_matched_fallback() {
        let css = fonts_css_with(&URLS);
        assert_eq!(css.matches("font-display: swap").count(), 5);
        for family in ["\"Urbanist\"", "\"IBM Plex Sans\"", "\"JetBrains Mono\""] {
            assert!(css.contains(&format!("font-family: {family};")), "{family}");
        }
        assert_eq!(css.matches("font-family: \"IBM Plex Sans\";").count(), 3);
        assert_eq!(css.matches("font-family: \"Urbanist Fallback\"").count(), 2);
        assert_eq!(
            css.matches("font-family: \"IBM Plex Sans Fallback\"")
                .count(),
            1
        );
        assert_eq!(css.matches("size-adjust:").count(), 3);
        assert_eq!(css.matches("ascent-override:").count(), 3);
        for url in [
            "/u.woff2",
            "/p1.woff2",
            "/p2.woff2",
            "/p3.woff2",
            "/m.woff2",
        ] {
            assert!(
                css.contains(&format!("url(\"{url}\") format(\"woff2\")")),
                "{url}"
            );
        }
        assert!(!css.contains("googleapis"));
        // IBM's file declares its own wdth range; only the default is used.
        assert!(!css.contains("font-stretch"));
    }

    /// The OFL texts are `asset!()`s (so `dx` copies them into `dist/assets`),
    /// the font CSS names them, and each is the real OFL 1.1.
    #[test]
    fn licence_texts_ship_with_the_fonts() {
        let src = include_str!("fonts.rs");
        let css = fonts_css_with(&URLS);
        for url in URLS.licences {
            assert!(css.contains(url), "{url}");
        }
        for rel in LICENCE_FILES {
            assert!(
                src.contains(&format!("asset!(\n    \"/{rel}\"")),
                "{rel} is not an asset!()"
            );
            let text = std::fs::read_to_string(crate_path(rel)).expect(rel);
            assert!(text.contains("SIL OPEN FONT LICENSE Version 1.1"), "{rel}");
            assert!(text.contains("Copyright"), "{rel}");
        }
        let plex = std::fs::read_to_string(crate_path(LICENCE_FILES[1])).unwrap();
        assert!(plex.contains("Reserved Font Name \"Plex\""));
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
