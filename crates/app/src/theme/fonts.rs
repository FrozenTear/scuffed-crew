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
//!   otherwise changed here. The four Plex files are IBM's own split woff2
//!   files, byte for byte, from the npm package `@ibm/plex-sans-variable`
//!   0.2.0 (github.com/IBM/plex release `@ibm/plex-sans-variable@0.2.0`,
//!   tarball integrity `sha512-S8xew9E8CPPKFvitbu2gDtwNFVWwwhClheaIBeykDYg7DrVK3cV5Xq5MjMrWPP5Os0xyiRFjMOmtGBLzz3urTQ==`),
//!   `fonts/split/woff2/`: `IBM Plex Sans Var-Roman-Latin1.woff2`,
//!   `IBM Plex Sans Var-Roman-Latin2.woff2`,
//!   `IBM Plex Sans Var-Roman-Latin3.woff2` and
//!   `IBM Plex Sans Var-Roman-Pi.woff2`, renamed only (no spaces in URLs).
//!   Their `unicode-range` values are IBM's, from the package's
//!   `IBM Plex Sans Var-Roman.css`. `ibm-plex-sans-OFL.txt` is the
//!   package's `LICENSE.txt`, byte for byte.
//!
//! Shipped files, byte size and sha256 (`assets/fonts/`):
//! - `urbanist.woff2` 29912 B
//!   `e59b4b15d8f69eead16ab029dbb8f089257ea12950a0e94c5c8a3e4c78dd083b`
//! - `jetbrains-mono.woff2` 44500 B
//!   `24353ce2af3dd5a3ade31cedc79ef5c85314032194970c1745626677476ab8b4`
//! - `ibm-plex-sans-var-roman-latin1.woff2` 68988 B
//!   `a18d94ea83c93f0a465d0976d8bc8ecbb031225dbaf76fd201d4e724a972034f`
//! - `ibm-plex-sans-var-roman-latin2.woff2` 44752 B
//!   `53de8a1640c42f45de9483f816dfc71dad3e87c24fbd115f4c312b8eddf39545`
//! - `ibm-plex-sans-var-roman-latin3.woff2` 18576 B
//!   `f08bc033a4959fb5fbc75a678fe56d2cd72ab5664bb3e6e1a41ac8f0d881c551`
//! - `ibm-plex-sans-var-roman-pi.woff2` 23900 B
//!   `0273a6b0411acbccb52b7d55af2bae2c58b35d68d6795a28c2f4db7ab21accbc`
//! - `ibm-plex-sans-OFL.txt` 4456 B
//!   `7e6b2818edbd8f6a01ae80641cc8f16a51080d08fb4e532be3a0b6f74adb07da`
//! - `urbanist-OFL.txt` 4385 B
//!   `ee1221b1c2d08920e5f9ca764eb228dafa5c8090df9cf665373c2287b9cb8f49`
//! - `jetbrains-mono-OFL.txt` 4399 B
//!   `b2fe5e8987594e9ffd1d2ca52a2f5d73eb8335243893c5d6254b5ad69269591d`
//!
//! The crate has no sha256 dependency, so `pinned_files_are_unchanged`
//! holds each file to its byte length and a 64-bit FNV-1a checksum instead
//! of the sha256. If a file is ever replaced, update all three values for it
//! together from the new file: the byte size and sha256 in the list above,
//! and the size and FNV-1a in `pinned_files_are_unchanged`.
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
static PLEX_LATIN3: manganis::Asset = asset!(
    "/assets/fonts/ibm-plex-sans-var-roman-latin3.woff2",
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
const PLEX_LATIN3_RANGE: &str = "U+0102-0103, U+01CD-01DC, U+1EA0-1EF9, U+20AB";
const PLEX_PI_RANGE: &str = "U+0E3F, U+2000-200D, U+2015, U+2028-2029, U+202F, U+2032-2033, U+2070, U+2074-2079, U+2080-2089, U+2113, U+2116, U+2126, U+212E, U+2150-2151, U+2153-215E, U+2190-2199, U+21A9-21AA, U+21B0-21B3, U+21B6-21B7, U+21BA-21BB, U+21C4, U+21C6, U+2202, U+2206, U+220F, U+2211, U+2215, U+221A, U+221E, U+222B, U+2248, U+2260, U+2264-2265, U+25CA, U+2713, U+274C, U+2B0E-2B11, U+ECE0, U+EFCC, U+FEFF, U+FFFD";

/// The display fonts an admin may pick. A fixed list of self-hosted
/// families: never a free-text name or a URL. `--font-body` stays IBM Plex
/// Sans whichever is chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayFont {
    #[default]
    Urbanist,
    IbmPlexSans,
}

impl DisplayFont {
    /// Menu order. The first is the default.
    pub const ALL: [DisplayFont; 2] = [DisplayFont::Urbanist, DisplayFont::IbmPlexSans];

    /// The value stored in settings.
    pub fn id(self) -> &'static str {
        match self {
            DisplayFont::Urbanist => "urbanist",
            DisplayFont::IbmPlexSans => "ibm-plex-sans",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DisplayFont::Urbanist => "Urbanist",
            DisplayFont::IbmPlexSans => "IBM Plex Sans",
        }
    }

    /// The primitive family token (`css/primitives.css`).
    pub fn family_var(self) -> &'static str {
        match self {
            DisplayFont::Urbanist => "var(--font-family-urbanist)",
            DisplayFont::IbmPlexSans => "var(--font-family-ibm-plex-sans)",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|font| font.id() == id.trim())
    }

    /// A stored value. Empty, missing or unknown gives the default.
    pub fn from_setting(value: Option<&str>) -> Self {
        value.and_then(Self::from_id).unwrap_or_default()
    }

    /// The value a save stores for this font. The default is stored as `""`
    /// ("use the default"), the same as the accent's empty default, so a
    /// reset never pins an explicit id. Other fonts store their [`Self::id`].
    pub fn setting_value(self) -> &'static str {
        if self == Self::default() {
            ""
        } else {
            self.id()
        }
    }

    /// The font the site uses: the typed `brand_font_display` of loaded
    /// settings. Settings not loaded yet give the default.
    pub fn for_settings(settings: Option<&scuffed_types::SiteSettings>) -> Self {
        Self::from_setting(settings.map(|s| s.brand_font_display.as_str()))
    }

    /// `--brand-font-display` for one scope (a preview card).
    pub fn css_decl(self) -> String {
        format!("--brand-font-display: {};", self.family_var())
    }

    /// The site-wide override, placed after the token stylesheet. Empty for
    /// the default, which `css/semantic.css` already sets.
    pub fn root_css(self) -> String {
        match self {
            DisplayFont::Urbanist => String::new(),
            other => format!(":root {{ {} }}", other.css_decl()),
        }
    }
}

/// Served font and licence URLs, as `dx` resolves them.
struct FontUrls<'a> {
    urbanist: &'a str,
    plex: [&'a str; 4],
    mono: &'a str,
    licences: [&'a str; 3],
}

/// `@font-face` rules for the three families and their fallbacks.
pub fn fonts_css() -> String {
    let (urbanist, latin1, latin2, latin3, pi, mono) = (
        URBANIST.to_string(),
        PLEX_LATIN1.to_string(),
        PLEX_LATIN2.to_string(),
        PLEX_LATIN3.to_string(),
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
        plex: [&latin1, &latin2, &latin3, &pi],
        mono: &mono,
        licences: [&urbanist_ofl, &plex_ofl, &mono_ofl],
    })
}

fn fonts_css_with(urls: &FontUrls) -> String {
    let FontUrls {
        urbanist,
        plex: [latin1, latin2, latin3, pi],
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
        plex_face(latin3, PLEX_LATIN3_RANGE),
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
        plex: ["/p1.woff2", "/p2.woff2", "/p3.woff2", "/p4.woff2"],
        mono: "/m.woff2",
        licences: ["/u-OFL.txt", "/p-OFL.txt", "/m-OFL.txt"],
    };

    /// The picker offers only families this crate self-hosts.
    #[test]
    fn display_fonts_are_self_hosted_families() {
        use super::DisplayFont;
        let primitives = include_str!("css/primitives.css");
        let faces = fonts_css_with(&URLS);
        for font in DisplayFont::ALL {
            let token = font
                .family_var()
                .trim_start_matches("var(")
                .trim_end_matches(')');
            assert!(primitives.contains(&format!("{token}:")), "{token}");
            assert!(
                faces.contains(&format!("font-family: \"{}\"", font.label())),
                "no @font-face for {}",
                font.label()
            );
            assert_eq!(DisplayFont::from_id(font.id()), Some(font));
        }
        assert_eq!(DisplayFont::default(), DisplayFont::ALL[0]);
        assert_eq!(DisplayFont::default().root_css(), "");
        assert_eq!(
            DisplayFont::IbmPlexSans.root_css(),
            ":root { --brand-font-display: var(--font-family-ibm-plex-sans); }"
        );
    }

    /// A stored value is an id from the list. Names, stacks and URLs are
    /// never read as a font.
    #[test]
    fn unknown_display_font_values_fall_back_to_the_default() {
        use super::DisplayFont;
        for raw in [
            "",
            "Comic Sans MS",
            "IBM Plex Sans",
            "https://fonts.example/x.woff2",
            "url(x)",
        ] {
            assert_eq!(DisplayFont::from_id(raw), None, "{raw}");
            assert_eq!(DisplayFont::from_setting(Some(raw)), DisplayFont::Urbanist);
        }
        assert_eq!(DisplayFont::from_setting(None), DisplayFont::Urbanist);
        assert_eq!(
            DisplayFont::from_setting(Some("ibm-plex-sans")),
            DisplayFont::IbmPlexSans
        );
    }

    fn crate_path(rel: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
    }

    #[test]
    fn every_face_swaps_and_has_a_metric_matched_fallback() {
        let css = fonts_css_with(&URLS);
        assert_eq!(css.matches("font-display: swap").count(), 6);
        for family in ["\"Urbanist\"", "\"IBM Plex Sans\"", "\"JetBrains Mono\""] {
            assert!(css.contains(&format!("font-family: {family};")), "{family}");
        }
        assert_eq!(css.matches("font-family: \"IBM Plex Sans\";").count(), 4);
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
            "/p4.woff2",
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

    /// `fonts_css_with` puts each Plex URL slot (the dummy `/p1` to `/p4`
    /// URLs here, Latin1, Latin2, Latin3 and Pi in that order) in its own
    /// `@font-face` with IBM's `unicode-range` for that split. The ranges
    /// are literal copies of `IBM Plex Sans Var-Roman.css` in
    /// `@ibm/plex-sans-variable` 0.2.0, not the constants above, so a swapped
    /// range constant or a swapped face fails here. Which real file fills
    /// each slot is set in `fonts_css`, which this test does not call.
    #[test]
    fn plex_faces_carry_ibm_ranges_for_their_file() {
        let css = fonts_css_with(&URLS);
        let ibm = [
            (
                "/p1.woff2",
                "U+0000, U+000D, U+0020-007E, U+00A0-00FF, U+0131, U+0152-0153, U+02C6, U+02DA, U+02DC, U+2013-2014, U+2018-201A, U+201C-201E, U+2020-2022, U+2026, U+2030, U+2039-203A, U+2044, U+20AC, U+2122, U+2212, U+FB01-FB02",
            ),
            (
                "/p2.woff2",
                "U+0100-0101, U+0104-0130, U+0132-0151, U+0154-017F, U+018F, U+0192, U+01A0-01A1, U+01AF-01B0, U+01FA-01FF, U+0218-021B, U+0237, U+0259, U+1E80-1E85, U+1E9E, U+20A1, U+20A4, U+20A6, U+20A8-20AA, U+20AD-20AE, U+20B1-20B2, U+20B4-20B5, U+20B8-20BA, U+20BD, U+20BF",
            ),
            ("/p3.woff2", "U+0102-0103, U+01CD-01DC, U+1EA0-1EF9, U+20AB"),
            (
                "/p4.woff2",
                "U+0E3F, U+2000-200D, U+2015, U+2028-2029, U+202F, U+2032-2033, U+2070, U+2074-2079, U+2080-2089, U+2113, U+2116, U+2126, U+212E, U+2150-2151, U+2153-215E, U+2190-2199, U+21A9-21AA, U+21B0-21B3, U+21B6-21B7, U+21BA-21BB, U+21C4, U+21C6, U+2202, U+2206, U+220F, U+2211, U+2215, U+221A, U+221E, U+222B, U+2248, U+2260, U+2264-2265, U+25CA, U+2713, U+274C, U+2B0E-2B11, U+ECE0, U+EFCC, U+FEFF, U+FFFD",
            ),
        ];
        for (url, range) in ibm {
            let src = format!("src: url(\"{url}\") format(\"woff2\");");
            let blocks: Vec<&str> = css
                .split("@font-face {")
                .filter(|block| block.contains(&src))
                .collect();
            assert_eq!(blocks.len(), 1, "{url} is in one @font-face");
            let block = blocks[0];
            assert!(
                block.contains("font-family: \"IBM Plex Sans\";"),
                "{url} family"
            );
            assert!(
                block.contains(&format!("unicode-range: {range};\n")),
                "{url} has IBM's unicode-range for its split"
            );
            assert_eq!(block.matches("unicode-range:").count(), 1, "{url}");
        }
    }

    /// Source wiring and contents only: each OFL text is declared as an
    /// `asset!()` in this file, the font CSS comment names the licence URLs
    /// it is given, and each file reads as OFL 1.1. Whether `dx` copies the
    /// texts into `dist/assets` is not checked here; the bodies are pinned
    /// in `pinned_files_are_unchanged`.
    #[test]
    fn licence_texts_are_assets_and_read_as_ofl() {
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

    /// 64-bit FNV-1a. A cheap stand-in for the sha256 in the module doc,
    /// since the crate has no sha256 dependency.
    fn fnv1a64(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    /// Every shipped font and licence file (the Urbanist and JetBrains Mono
    /// subsets, the four IBM Plex woff2 files and the three OFL texts), byte
    /// length and FNV-1a over the actual files (sha256 for each is in the
    /// module doc).
    #[test]
    fn pinned_files_are_unchanged() {
        let pins: [(&str, &[u8], usize, u64); 9] = [
            (
                "urbanist.woff2",
                include_bytes!("../../assets/fonts/urbanist.woff2"),
                29_912,
                0x7622_b50d_6214_cd3f,
            ),
            (
                "jetbrains-mono.woff2",
                include_bytes!("../../assets/fonts/jetbrains-mono.woff2"),
                44_500,
                0xa99d_cc8b_62d0_a2f5,
            ),
            (
                "ibm-plex-sans-var-roman-latin1.woff2",
                include_bytes!("../../assets/fonts/ibm-plex-sans-var-roman-latin1.woff2"),
                68_988,
                0x9626_e415_1ab1_0e46,
            ),
            (
                "ibm-plex-sans-var-roman-latin2.woff2",
                include_bytes!("../../assets/fonts/ibm-plex-sans-var-roman-latin2.woff2"),
                44_752,
                0x7ff5_f36a_1238_354f,
            ),
            (
                "ibm-plex-sans-var-roman-latin3.woff2",
                include_bytes!("../../assets/fonts/ibm-plex-sans-var-roman-latin3.woff2"),
                18_576,
                0xe83d_e5c1_0f88_43f6,
            ),
            (
                "ibm-plex-sans-var-roman-pi.woff2",
                include_bytes!("../../assets/fonts/ibm-plex-sans-var-roman-pi.woff2"),
                23_900,
                0x8481_74cc_6b02_6341,
            ),
            (
                "ibm-plex-sans-OFL.txt",
                include_bytes!("../../assets/fonts/ibm-plex-sans-OFL.txt"),
                4_456,
                0x51e1_9100_183d_8879,
            ),
            (
                "urbanist-OFL.txt",
                include_bytes!("../../assets/fonts/urbanist-OFL.txt"),
                4_385,
                0x7d4f_af69_887b_5fa6,
            ),
            (
                "jetbrains-mono-OFL.txt",
                include_bytes!("../../assets/fonts/jetbrains-mono-OFL.txt"),
                4_399,
                0x8d16_d710_6322_164b,
            ),
        ];
        for (name, bytes, len, sum) in pins {
            assert_eq!(bytes.len(), len, "{name} length");
            assert_eq!(fnv1a64(bytes), sum, "{name} checksum");
        }
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
