//! Brand-configurable accent colors.
//!
//! Product default is a neutral purple. Live sites override via SiteSettings
//! (`brand_accent_dark` / `brand_accent_light`). Homepage presets may suggest
//! accents when applied in admin.
//!
//! Every other accent role (`--accent-text`, `--accent-solid`, `--on-accent`,
//! `--accent-soft`) is derived from the brand hex by [`AccentRoles::derive`],
//! a port of the design system's `derive-accent.js`, including its contrast
//! lock and the mint lock for the live brand.
//!
//! Port source: the design-system package (`site-redesign/design-system/`),
//! which is not vendored in this repo. Files and sha256 the port matches:
//! - `tokens/derive-accent.js`
//!   `adf946e17cdebd1f0c753527a8964af7e056a206ffabf85932d1b7d4049c21a5`
//! - `tokens.md`
//!   `c224092174343293c88ab6fd94821989b009445886aab86cc744a280e4b877b8`

/// Dark ink for a fill that white text does not clear 4.5:1 on.
/// Same value as `--neutral-900` and Look A dark `--bg`.
pub const INK: &str = "#17171d";
const WHITE: &str = "#ffffff";
const MINT_DARK: &str = "#46d8a4";
const MINT_LIVE_LIGHT: &str = "#0ea66e";
const MINT_LIGHT_SOLID: &str = "#087a50";
const MINT_LIGHT_TEXT_DEEP: &str = "#007046";

/// Look A text surfaces in derivation order: `--bg`, `--surface`,
/// `--surface-raised`, `--surface-sunken`. A test holds these to
/// `css/look_a.css`.
pub const DARK_SURFACES: [&str; 4] = ["#17171d", "#1f1f27", "#282831", "#282831"];
pub const LIGHT_SURFACES: [&str; 4] = ["#f7f7f9", "#ffffff", "#ffffff", "#f0f0f4"];

/// Strategy planner brand (`--orange-strategy`). `[data-accent="strategy"]`
/// sets only this hex; every other accent role is derived from it.
pub const STRATEGY_BRAND: &str = "#ff7a1a";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Dark,
    Light,
}

impl Scheme {
    /// Alpha of the raw brand in `--accent-soft`.
    fn soft_alpha(self) -> f64 {
        match self {
            Scheme::Dark => 0.17,
            Scheme::Light => 0.1,
        }
    }

    fn surfaces(self) -> [&'static str; 4] {
        match self {
            Scheme::Dark => DARK_SURFACES,
            Scheme::Light => LIGHT_SURFACES,
        }
    }
}

/// The accent roles one theme scope gets from one brand hex. This is a port
/// of `derive-accent.js` from the design system and must stay in step with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccentRoles {
    /// `--accent`: the raw brand. Not for 12px text or light-theme bars.
    pub accent: String,
    /// `--accent-text`: 4.5:1 on every text surface and on the soft fill.
    pub text: String,
    /// `--accent-solid`: solid fills (primary buttons, the mark, skip link).
    pub solid: String,
    /// `--on-accent`: white only when white clears 4.5:1 on the solid.
    pub on_accent: String,
    /// `--accent-soft`: the brand at 17% (dark) or 10% (light), composited
    /// onto `--surface` as an opaque hex so contrast is stable on any parent.
    pub soft: String,
}

impl AccentRoles {
    /// Derive every accent role from a normalized `#rrggbb` brand.
    ///
    /// Dark: solid is the brand; text is the brand when it clears 4.5:1 on
    /// every text surface, otherwise it is lightened in OKLCH until it does.
    /// Light: solid is the brand darkened in OKLCH until 4.5:1 on the four
    /// surfaces; text starts there and darkens further until it also clears
    /// the soft fills. Mint lock: `#46d8a4` / `#0ea66e` keep the site values.
    pub fn derive(brand: &str, scheme: Scheme) -> Self {
        let brand = normalize_hex(brand).unwrap_or_else(|| "#000000".into());
        let surfaces = scheme.surfaces();
        let alpha = scheme.soft_alpha();
        let soft = composite(&brand, alpha, surfaces[1]);
        let mut text_surfaces: Vec<String> = surfaces.iter().map(|s| s.to_string()).collect();
        for base in &surfaces[..3] {
            text_surfaces.push(composite(&brand, alpha, base));
        }
        let text_surfaces: Vec<&str> = text_surfaces.iter().map(String::as_str).collect();
        let ink_on = |solid: &str| {
            if contrast_ratio(WHITE, solid) >= 4.5 {
                WHITE
            } else {
                INK
            }
        };

        let (solid, text, on) = match scheme {
            Scheme::Light if brand == MINT_DARK || brand == MINT_LIVE_LIGHT => {
                let text = if passes_all(MINT_LIGHT_SOLID, &text_surfaces, 4.5) {
                    MINT_LIGHT_SOLID.to_string()
                } else if passes_all(MINT_LIGHT_TEXT_DEEP, &text_surfaces, 4.5) {
                    MINT_LIGHT_TEXT_DEEP.to_string()
                } else {
                    adjust_until(MINT_LIGHT_SOLID, &text_surfaces, 4.5, Toward::Dark)
                };
                (MINT_LIGHT_SOLID.to_string(), text, WHITE)
            }
            Scheme::Dark if brand == MINT_DARK => {
                let text = if passes_all(MINT_DARK, &text_surfaces, 4.5) {
                    MINT_DARK.to_string()
                } else {
                    adjust_until(MINT_DARK, &text_surfaces, 4.5, Toward::Light)
                };
                (MINT_DARK.to_string(), text, INK)
            }
            Scheme::Dark => {
                let text = if passes_all(&brand, &text_surfaces, 4.5) {
                    brand.clone()
                } else {
                    adjust_until(&brand, &text_surfaces, 4.5, Toward::Light)
                };
                let on = ink_on(&brand);
                (brand.clone(), text, on)
            }
            Scheme::Light => {
                let solid = adjust_until(&brand, &surfaces, 4.5, Toward::Dark);
                let text = if passes_all(&solid, &text_surfaces, 4.5) {
                    solid.clone()
                } else {
                    adjust_until(&solid, &text_surfaces, 4.5, Toward::Dark)
                };
                let on = ink_on(&solid);
                (solid, text, on)
            }
        };

        Self {
            accent: brand,
            text,
            solid,
            on_accent: on.to_string(),
            soft,
        }
    }

    /// The five accent declarations for one scope.
    pub fn css_decls(&self) -> String {
        format!(
            "--brand-accent: {a}; --accent: {a}; --accent-text: {t}; --accent-solid: {s}; --on-accent: {o}; --accent-soft: {soft};",
            a = self.accent,
            t = self.text,
            s = self.solid,
            o = self.on_accent,
            soft = self.soft,
        )
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BrandConfig {
    /// Brand hex for dark mode.
    pub accent_dark: String,
    /// Brand hex for light mode.
    pub accent_light: String,
    /// Accent roles derived from `accent_dark` for the dark scope.
    pub dark: AccentRoles,
    /// Accent roles derived from `accent_light` for the light scope.
    pub light: AccentRoles,
}

impl BrandConfig {
    /// Product default accents (not clan-specific).
    pub fn product_default() -> Self {
        Self::from_accents("#8f73ff", "#6d4aff")
    }

    /// Build from primary dark/light hex accents; every other role is derived.
    pub fn from_accents(accent_dark: &str, accent_light: &str) -> Self {
        let dark = normalize_hex(accent_dark).unwrap_or_else(|| "#8f73ff".into());
        let light = normalize_hex(accent_light).unwrap_or_else(|| dark.clone());
        Self {
            dark: AccentRoles::derive(&dark, Scheme::Dark),
            light: AccentRoles::derive(&light, Scheme::Light),
            accent_dark: dark,
            accent_light: light,
        }
    }

    /// Neutral accent while public settings are still unknown.
    ///
    /// The product purple is a real brand. Painting it before settings arrive
    /// flashes the wrong accent on orgs that chose something else. This gray
    /// stays at least 3:1 against both theme backgrounds and against the dark
    /// `--border` used by a focused field. The boot mark in `index.html`,
    /// `assets/favicon.svg`, and [`org_favicon_data_uri`] use the same hex.
    pub fn pending() -> Self {
        Self::from_accents("#808088", "#808088")
    }

    /// Resolve settings fields: empty → product default.
    pub fn from_settings(accent_dark: &str, accent_light: &str) -> Self {
        let d = accent_dark.trim();
        let l = accent_light.trim();
        if d.is_empty() && l.is_empty() {
            return Self::product_default();
        }
        let dark = if d.is_empty() { "#8f73ff" } else { d };
        let light = if l.is_empty() { dark } else { l };
        Self::from_accents(dark, light)
    }
}

/// Product-default purple for callers that are not the public boot path.
///
/// Unloaded public settings use [`BrandConfig::pending`], not this. This is
/// the installed accent, not a placeholder.
pub fn current() -> BrandConfig {
    BrandConfig::product_default()
}

/// SVG favicon for one org. Letters come from the org name; the fill is the
/// pending gray so every clan does not share the same mark color and initials.
pub fn org_favicon_data_uri(initials: &str) -> String {
    let letters: String = initials
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(2)
        .collect();
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect width='32' height='32' rx='6' fill='%23808088'/><text x='16' y='22' text-anchor='middle' font-family='system-ui,sans-serif' font-size='14' font-weight='700' fill='%23ffffff'>{letters}</text></svg>"
    );
    format!("data:image/svg+xml,{svg}")
}

/// `None` leaves the shell's static icon (settings still unknown).
/// A non-empty org name always gets a data-URI mark, even when the initials
/// are `CL`. A settled blank name keeps the neutral asset.
pub fn runtime_favicon_href(settled_org_name: Option<&str>) -> Option<String> {
    match settled_org_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        None if settled_org_name.is_none() => None,
        None => Some("/assets/favicon.svg".to_string()),
        Some(name) => Some(org_favicon_data_uri(&scuffed_types::org_initials(name))),
    }
}

/// Accept `#rgb` / `#rrggbb` / bare hex → lowercase `#rrggbb`.
pub fn normalize_hex(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let hex = s.strip_prefix('#').unwrap_or(s);
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 => {
            let mut out = String::from("#");
            for c in hex.chars() {
                out.push(c);
                out.push(c);
            }
            Some(out.to_ascii_lowercase())
        }
        6 => Some(format!("#{}", hex.to_ascii_lowercase())),
        _ => None,
    }
}

/// WCAG AA for text. The brand settings contrast lock uses the same floor
/// the derivation aims for.
pub const AA_TEXT: f64 = 4.5;

/// WCAG AA for non-text shapes (1.4.11). A primary button or the nav mark
/// is an accent fill with no border, so the fill itself is its edge.
pub const AA_NON_TEXT: f64 = 3.0;

/// The live contrast check for one theme scope: the three pairs the brand
/// settings page shows, measured on the derived roles.
#[derive(Clone, Debug, PartialEq)]
pub struct SchemeCheck {
    pub scheme: Scheme,
    pub roles: AccentRoles,
    /// `--on-accent` on `--accent-solid` (a primary button).
    pub ink_on_accent: f64,
    /// `--accent-text` on the page background `--bg`.
    pub accent_on_bg: f64,
    /// `--accent-solid` on the surface it stands out from least: the page
    /// background or a card. Below [`AA_NON_TEXT`] a primary button has no
    /// visible edge.
    pub fill_on_surface: f64,
}

impl SchemeCheck {
    fn new(roles: AccentRoles, scheme: Scheme) -> Self {
        let ink_on_accent = contrast_ratio(&roles.on_accent, &roles.solid);
        let accent_on_bg = contrast_ratio(&roles.text, scheme.surfaces()[0]);
        let fill_on_surface = scheme
            .surfaces()
            .iter()
            .map(|surface| contrast_ratio(&roles.solid, surface))
            .fold(f64::INFINITY, f64::min);
        Self {
            scheme,
            roles,
            ink_on_accent,
            accent_on_bg,
            fill_on_surface,
        }
    }

    pub fn fill_passes(&self) -> bool {
        self.fill_on_surface >= AA_NON_TEXT
    }

    pub fn ink_passes(&self) -> bool {
        self.ink_on_accent >= AA_TEXT
    }

    pub fn text_passes(&self) -> bool {
        self.accent_on_bg >= AA_TEXT
    }

    pub fn passes(&self) -> bool {
        self.ink_passes() && self.text_passes() && self.fill_passes()
    }

    fn theme_word(&self) -> &'static str {
        match self.scheme {
            Scheme::Dark => "dark",
            Scheme::Light => "light",
        }
    }
}

/// Both theme scopes for one brand config. Save is blocked unless every
/// text pair reaches [`AA_TEXT`] and the fill reaches [`AA_NON_TEXT`].
#[derive(Clone, Debug, PartialEq)]
pub struct AccentCheck {
    pub dark: SchemeCheck,
    pub light: SchemeCheck,
}

impl AccentCheck {
    pub fn for_brand(brand: &BrandConfig) -> Self {
        Self {
            dark: SchemeCheck::new(brand.dark.clone(), Scheme::Dark),
            light: SchemeCheck::new(brand.light.clone(), Scheme::Light),
        }
    }

    pub fn passes(&self) -> bool {
        self.dark.passes() && self.light.passes()
    }

    /// One short sentence for the first failing pair, or `None` when every
    /// pair passes.
    pub fn fail_reason(&self) -> Option<String> {
        for check in [&self.dark, &self.light] {
            let theme = check.theme_word();
            if !check.text_passes() {
                return Some(format!(
                    "This accent does not reach 4.5:1 on the {theme} page background."
                ));
            }
            if !check.ink_passes() {
                return Some(format!(
                    "Ink on this accent does not reach 4.5:1 in the {theme} preview."
                ));
            }
            if !check.fill_passes() {
                return Some(format!(
                    "Buttons in this accent do not reach 3:1 against the {theme} page and cards, so they lose their edge."
                ));
            }
        }
        None
    }
}

/// A ratio as the contrast lines print it: one decimal, rounded. A pair
/// just under its floor is rounded down instead, so a failing pair never
/// reads as `4.5:1` (text) or `3.0:1` (fill).
pub fn format_ratio(ratio: f64) -> String {
    let rounded = (ratio * 10.0).round() / 10.0;
    let just_under = |floor: f64| ratio < floor && rounded >= floor;
    let shown = if just_under(AA_TEXT) || just_under(AA_NON_TEXT) {
        (ratio * 10.0).floor() / 10.0
    } else {
        rounded
    };
    format!("{shown:.1}:1")
}

/// An accent the contrast lock blocks: in dark neither white nor the dark
/// ink reaches 4.5:1 on it.
#[cfg(test)]
pub const TEST_BLOCKED_ACCENT: &str = "#d35400";
/// Text that is not a hex colour the server accepts, for the hex field.
#[cfg(test)]
pub const TEST_INVALID_ACCENTS: [&str; 4] = ["mint", "#12", "#46d8a4ff", "url(x)"];
/// A short hex in upper case with stray spaces: accepted and normalized.
#[cfg(test)]
pub const TEST_SHORT_ACCENT: &str = " #ABC ";
/// The live mint brand, for tests that need a passing accent.
#[cfg(test)]
pub const TEST_MINT_ACCENT: &str = MINT_DARK;
/// A second passing accent that is not the mint and not in any token file.
#[cfg(test)]
pub const TEST_OTHER_ACCENT: &str = "#ff5fa2";
/// An accent as dark as the dark page: every text pair passes, but a
/// primary button in it has no edge, so the fill pair blocks it.
#[cfg(test)]
pub const TEST_EDGELESS_ACCENT: &str = "#000000";
/// Accents for contrast sweeps: the extremes, the page colours, and the
/// accents QA 694 measured (pale yellow, sky, lilac, mint, the defaults).
#[cfg(test)]
pub const TEST_SWEEP_ACCENTS: [&str; 11] = [
    "#000000", "#ffffff", "#ffe680", "#46d8a4", "#38bdf8", "#c084fc", "#8f73ff", "#6d4aff",
    "#ff0000", "#17171d", "#f7f7f9",
];

type Rgb = (u8, u8, u8);

fn parse_rgb(hex: &str) -> Rgb {
    let h = hex.trim().trim_start_matches('#');
    let byte = |i: usize| {
        h.get(i..i + 2)
            .and_then(|b| u8::from_str_radix(b, 16).ok())
            .unwrap_or(0)
    };
    (byte(0), byte(2), byte(4))
}

/// Round and clamp each channel, as `toHex` in `derive-accent.js` does.
fn to_hex(r: f64, g: f64, b: f64) -> String {
    let h = |n: f64| n.round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", h(r), h(g), h(b))
}

fn srgb_to_linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f64) -> f64 {
    let c = c.clamp(0.0, 1.0);
    let v = if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0).round()
}

/// WCAG relative luminance of a `#rrggbb` colour.
pub fn relative_luminance(hex: &str) -> f64 {
    let (r, g, b) = parse_rgb(hex);
    0.2126 * srgb_to_linear(r) + 0.7152 * srgb_to_linear(g) + 0.0722 * srgb_to_linear(b)
}

/// WCAG contrast ratio between two `#rrggbb` colours.
pub fn contrast_ratio(fg: &str, bg: &str) -> f64 {
    let l1 = relative_luminance(fg);
    let l2 = relative_luminance(bg);
    let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

fn passes_all(hex: &str, surfaces: &[&str], min: f64) -> bool {
    surfaces.iter().all(|s| contrast_ratio(hex, s) >= min)
}

/// `fg` at `alpha` over `bg`, as an opaque hex. The same mix as CSS
/// `color-mix(in srgb, fg alpha, bg)`.
pub(crate) fn composite(fg: &str, alpha: f64, bg: &str) -> String {
    let f = parse_rgb(fg);
    let b = parse_rgb(bg);
    let mix = |f: u8, b: u8| f64::from(f) * alpha + f64::from(b) * (1.0 - alpha);
    to_hex(mix(f.0, b.0), mix(f.1, b.1), mix(f.2, b.2))
}

struct Oklch {
    l: f64,
    c: f64,
    h: f64,
}

fn hex_to_oklch(hex: &str) -> Oklch {
    let (r, g, b) = parse_rgb(hex);
    let (r, g, b) = (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b));
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
    let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
    let lab_l = 0.2104542553 * l_ + 0.793617785 * m_ - 0.0040720468 * s_;
    let lab_a = 1.9779984951 * l_ - 2.428592205 * m_ + 0.4505937099 * s_;
    let lab_b = 0.0259040371 * l_ + 0.7827717662 * m_ - 0.808675766 * s_;
    let mut h = lab_b.atan2(lab_a).to_degrees();
    if h < 0.0 {
        h += 360.0;
    }
    Oklch {
        l: lab_l,
        c: (lab_a * lab_a + lab_b * lab_b).sqrt(),
        h,
    }
}

fn oklch_to_hex(l: f64, c: f64, h: f64) -> String {
    let rad = h.to_radians();
    let (a, b) = (c * rad.cos(), c * rad.sin());
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.291485548 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3;
    let g = -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3;
    let b2 = -0.0041960863 * l3 - 0.7034186147 * m3 + 1.707614701 * s3;
    to_hex(linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b2))
}

#[derive(Clone, Copy)]
enum Toward {
    Light,
    Dark,
}

/// Step OKLCH lightness by 0.004 (at least once) until `hex` clears `min` on
/// every surface, keeping chroma and hue. Gives up after 250 steps.
fn adjust_until(hex: &str, surfaces: &[&str], min: f64, toward: Toward) -> String {
    let lch = hex_to_oklch(hex);
    let step = match toward {
        Toward::Light => 0.004,
        Toward::Dark => -0.004,
    };
    let mut l = lch.l;
    for _ in 0..250 {
        l = (l + step).clamp(0.0, 1.0);
        let candidate = oklch_to_hex(l, lch.c, lch.h);
        if passes_all(&candidate, surfaces, min) {
            return candidate;
        }
    }
    oklch_to_hex(l, lch.c, lch.h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_settings_empty_uses_product_default() {
        let b = BrandConfig::from_settings("", "");
        assert_eq!(b.accent_dark, "#8f73ff");
    }

    #[test]
    fn pending_accent_is_not_the_product_purple() {
        let pending = BrandConfig::pending();
        let product = BrandConfig::product_default();
        assert_ne!(pending.accent_dark, product.accent_dark);
        assert_ne!(pending.accent_light, product.accent_light);
        assert_ne!(pending.accent_dark, "#8f73ff");
        assert_ne!(pending.accent_light, "#6d4aff");
        assert_eq!(pending.accent_dark, pending.accent_light);
    }

    #[test]
    fn pending_accent_meets_ui_contrast_on_both_themes() {
        let pending = BrandConfig::pending();
        // `--border` is the field outline a focused input sits next to.
        let dark = DARK_SURFACES.iter().copied().chain(["#353541"]);
        let light = LIGHT_SURFACES.iter().copied().chain(["#e3e3e9"]);
        for (roles, surfaces) in [
            (&pending.dark, dark.collect::<Vec<_>>()),
            (&pending.light, light.collect::<Vec<_>>()),
        ] {
            for bg in &surfaces {
                for fg in [&roles.accent, &roles.solid] {
                    let ratio = contrast_ratio(fg, bg);
                    assert!(ratio >= 3.0, "{fg} on {bg} = {ratio:.2}");
                }
            }
            // `--border` is not a text surface; text clears AA on the other four.
            for bg in &surfaces[..4] {
                let ratio = contrast_ratio(&roles.text, bg);
                assert!(ratio >= 4.5, "text {} on {bg} = {ratio:.2}", roles.text);
            }
        }
        // The favicon draws white initials on the raw pending gray.
        let accent = pending.accent_dark.as_str();
        let white_on_pending = contrast_ratio("#ffffff", accent);
        assert!(
            white_on_pending >= 3.0,
            "white on {accent} = {white_on_pending:.2}"
        );
        let boot = include_str!("../../index.html");
        assert!(
            boot.contains(accent),
            "boot mark must use the same gray as BrandConfig::pending"
        );
    }

    #[test]
    fn favicon_assets_use_the_pending_gray_including_cl_initials() {
        let pending = BrandConfig::pending();
        let svg = include_str!("../../assets/favicon.svg");
        assert!(
            svg.contains(pending.accent_dark.as_str()),
            "favicon.svg fill must match BrandConfig::pending"
        );
        let encoded = pending.accent_dark.replacen('#', "%23", 1);
        let marked = org_favicon_data_uri("CL");
        assert!(marked.contains(&encoded), "{marked}");
        assert!(runtime_favicon_href(None).is_none());
        assert_eq!(
            runtime_favicon_href(Some("")),
            Some("/assets/favicon.svg".to_string())
        );
        assert_eq!(
            runtime_favicon_href(Some("   ")),
            Some("/assets/favicon.svg".to_string())
        );
        let clan = runtime_favicon_href(Some("Clan League")).expect("mark");
        assert!(clan.starts_with("data:image/svg+xml,"), "{clan}");
        assert!(clan.contains(&encoded), "{clan}");
        assert!(clan.contains("CL"), "{clan}");
        let bangs = runtime_favicon_href(Some("!!!")).expect("punctuation still has a name");
        assert!(bangs.starts_with("data:image/svg+xml,"), "{bangs}");
    }

    #[test]
    fn from_accents_derives_opaque_soft() {
        let b = BrandConfig::from_accents("#ff0000", "#cc0000");
        assert_eq!(b.accent_dark, "#ff0000");
        assert_eq!(b.dark.accent, "#ff0000");
        // 17% of #ff0000 over dark --surface #1f1f27, and 10% of #cc0000 over
        // white, both as opaque hex (no rgba).
        assert_eq!(b.dark.soft, "#451a20");
        assert_eq!(b.light.soft, "#fae6e6");
    }

    /// Outputs of `derive-accent.js` `derive(brand, theme, lookASurfaces)`,
    /// captured with node from the design system copy. The Rust port must
    /// match them byte for byte.
    /// Rows: brand, scheme, then text, solid, on-accent and soft.
    const JS_REFERENCE: &[&str] = &[
        "#46d8a4 dark #46d8a4 #46d8a4 #17171d #263e3c",
        "#46d8a4 light #087a50 #087a50 #ffffff #edfbf6",
        "#0ea66e dark #2fb77d #0ea66e #17171d #1c3633",
        "#0ea66e light #087a50 #087a50 #ffffff #e7f6f1",
        "#8f73ff dark #aa91ff #8f73ff #17171d #322d4c",
        "#8f73ff light #7251db #7454de #ffffff #f4f1ff",
        "#6d4aff dark #9f8aff #6d4aff #ffffff #2c264c",
        "#6d4aff light #6944fa #6c49fe #ffffff #f0edff",
        "#808088 dark #a1a1a9 #808088 #17171d #2f2f37",
        "#808088 light #6a6a72 #6d6d75 #ffffff #f2f2f3",
        "#ff7a1a dark #ff8227 #ff7a1a #17171d #452e25",
        "#ff7a1a light #bf3f00 #c24200 #ffffff #fff2e8",
        "#ffff00 dark #ffff00 #ffff00 #17171d #454520",
        "#ffff00 light #777000 #777000 #ffffff #ffffe6",
        "#1a1a1a dark #8f8f8f #1a1a1a #ffffff #1e1e25",
        "#1a1a1a light #191919 #191919 #ffffff #e8e8e8",
        "#ff0000 dark #ff614d #ff0000 #17171d #451a20",
        "#ff0000 light #cf0000 #dc0000 #ffffff #ffe6e6",
        "#cc0000 dark #ff5847 #cc0000 #ffffff #3c1a20",
        "#cc0000 light #cb0000 #cb0000 #ffffff #fae6e6",
    ];

    #[test]
    fn derivation_matches_derive_accent_js() {
        for row in JS_REFERENCE {
            let cols: Vec<&str> = row.split_whitespace().collect();
            let scheme = if cols[1] == "dark" {
                Scheme::Dark
            } else {
                Scheme::Light
            };
            let roles = AccentRoles::derive(cols[0], scheme);
            let got = [
                roles.accent.as_str(),
                roles.text.as_str(),
                roles.solid.as_str(),
                roles.on_accent.as_str(),
                roles.soft.as_str(),
            ];
            assert_eq!(got, [cols[0], cols[2], cols[3], cols[4], cols[5]], "{row}");
        }
    }

    #[test]
    fn mint_brand_is_locked() {
        let live = BrandConfig::from_accents("#46d8a4", "#0ea66e");
        assert_eq!(live.dark.solid, "#46d8a4");
        assert_eq!(live.dark.on_accent, INK);
        assert_eq!(live.dark.text, "#46d8a4");
        assert_eq!(live.light.solid, "#087a50");
        assert_eq!(live.light.text, "#087a50");
        assert_eq!(live.light.on_accent, "#ffffff");
        assert!(contrast_ratio("#ffffff", "#46d8a4") < 2.0);
    }

    /// A low-contrast brand is adjusted, never shipped as-is: yellow on a
    /// light surface is darkened until 4.5:1, and every derived pair holds
    /// for the brands listed here, including the product purple and the
    /// strategy orange. Not for every brand: in dark, `--on-accent` falls
    /// back to dark ink without a check, so for a dark solid with relative
    /// luminance between about 0.1833 and 0.2148 neither white nor the ink
    /// reaches 4.5:1 (`#d35400`: ink 4.28:1, white 4.17:1).
    /// `derive-accent.js` has the same gap; the fix belongs there first.
    #[test]
    fn low_contrast_accent_is_adjusted_to_aa() {
        let yellow = AccentRoles::derive("#ffff00", Scheme::Light);
        assert!(contrast_ratio("#ffff00", "#ffffff") < 1.1);
        assert_ne!(yellow.solid, "#ffff00");
        assert!(!yellow.soft.starts_with("rgba"), "{}", yellow.soft);
        for brand in [
            "#ffff00",
            "#8f73ff",
            "#6d4aff",
            STRATEGY_BRAND,
            "#46d8a4",
            "#808088",
        ] {
            for scheme in [Scheme::Dark, Scheme::Light] {
                let roles = AccentRoles::derive(brand, scheme);
                assert_eq!(roles.soft.len(), 7, "opaque #rrggbb soft: {}", roles.soft);
                let ink = contrast_ratio(&roles.on_accent, &roles.solid);
                assert!(ink >= 4.5, "{brand} {scheme:?} ink {ink:.2}");
                let mut surfaces: Vec<String> =
                    scheme.surfaces().iter().map(|s| s.to_string()).collect();
                surfaces.push(roles.soft.clone());
                for bg in &surfaces {
                    let text = contrast_ratio(&roles.text, bg);
                    assert!(text >= 4.5, "{brand} {scheme:?} text on {bg} {text:.2}");
                }
                if scheme == Scheme::Light {
                    for bg in scheme.surfaces() {
                        let solid = contrast_ratio(&roles.solid, bg);
                        assert!(solid >= 4.5, "{brand} light solid on {bg} {solid:.2}");
                    }
                }
            }
        }
    }

    /// The numbers on the brand settings frame for the live mint.
    #[test]
    fn contrast_check_matches_the_mint_frame() {
        let check = AccentCheck::for_brand(&BrandConfig::from_settings(MINT_DARK, MINT_DARK));
        assert!(check.passes());
        assert!(check.fail_reason().is_none());
        assert_eq!(format_ratio(check.dark.ink_on_accent), "9.9:1");
        assert_eq!(format_ratio(check.dark.accent_on_bg), "9.9:1");
        assert_eq!(format_ratio(check.light.ink_on_accent), "5.4:1");
        assert_eq!(format_ratio(check.light.accent_on_bg), "5.0:1");
    }

    #[test]
    fn contrast_check_blocks_an_accent_no_ink_clears() {
        let brand = BrandConfig::from_settings(TEST_BLOCKED_ACCENT, TEST_BLOCKED_ACCENT);
        let check = AccentCheck::for_brand(&brand);
        assert!(!check.passes());
        assert!(!check.dark.ink_passes());
        assert!(check.dark.text_passes());
        assert!(check.light.passes());
        assert_eq!(
            check.fail_reason().as_deref(),
            Some("Ink on this accent does not reach 4.5:1 in the dark preview.")
        );
    }

    #[test]
    fn product_default_and_pending_pass_the_contrast_check() {
        assert!(AccentCheck::for_brand(&BrandConfig::product_default()).passes());
        assert!(AccentCheck::for_brand(&BrandConfig::pending()).passes());
    }

    #[test]
    fn a_failing_ratio_never_prints_as_the_floor() {
        assert_eq!(format_ratio(4.49), "4.4:1");
        assert_eq!(format_ratio(4.28), "4.3:1");
        assert_eq!(format_ratio(9.87), "9.9:1");
        assert_eq!(format_ratio(4.5), "4.5:1");
        assert_eq!(format_ratio(21.0), "21.0:1");
        // The fill floor too: a fill just under 3:1 never reads as 3.0:1.
        assert_eq!(format_ratio(2.97), "2.9:1");
        assert_eq!(format_ratio(3.0), "3.0:1");
    }

    /// QA 694 nit 4: an accent that matches the dark page passed the text
    /// pairs, but the primary button and the nav mark had no edge against
    /// the page (1.0:1). The lock now checks the fill against every surface.
    #[test]
    fn contrast_check_blocks_a_fill_with_no_edge() {
        for accent in ["#000000", "#17171d", "#1f1f27", "#282831"] {
            let check = AccentCheck::for_brand(&BrandConfig::from_settings(accent, accent));
            assert!(check.dark.text_passes(), "{accent} text is lifted");
            assert!(check.dark.ink_passes(), "{accent} white ink clears");
            assert!(
                !check.dark.fill_passes(),
                "{accent} {}",
                check.dark.fill_on_surface
            );
            assert!(check.light.passes(), "{accent} light solid clears 4.5:1");
            assert!(!check.passes(), "{accent}");
            assert_eq!(
                check.fail_reason().as_deref(),
                Some(
                    "Buttons in this accent do not reach 3:1 against the dark page and cards, so they lose their edge."
                ),
                "{accent}"
            );
        }
    }

    /// The fill pair is measured on the least distinct surface, and the
    /// accents the site ships keep a visible edge on all of them.
    #[test]
    fn fill_pair_is_the_lowest_surface_ratio() {
        for brand in [
            BrandConfig::product_default(),
            BrandConfig::pending(),
            BrandConfig::from_settings(MINT_DARK, MINT_DARK),
            BrandConfig::from_settings(TEST_OTHER_ACCENT, TEST_OTHER_ACCENT),
        ] {
            let check = AccentCheck::for_brand(&brand);
            for (scheme, surfaces) in [(&check.dark, DARK_SURFACES), (&check.light, LIGHT_SURFACES)]
            {
                let lowest = surfaces
                    .iter()
                    .map(|s| contrast_ratio(&scheme.roles.solid, s))
                    .fold(f64::INFINITY, f64::min);
                assert_eq!(scheme.fill_on_surface, lowest);
                assert!(scheme.fill_passes(), "{:?} {lowest:.2}", scheme.scheme);
            }
        }
    }

    #[test]
    fn contrast_formula_anchors() {
        let black_on_white = contrast_ratio("#000000", "#ffffff");
        assert!((black_on_white - 21.0).abs() < 0.05, "{black_on_white}");
        let same = contrast_ratio("#ffffff", "#ffffff");
        assert!((same - 1.0).abs() < 0.001, "{same}");
    }
}
