use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum ThemeMode {
    Light,
    Dark,
}

impl ThemeMode {
    /// The `data-theme` value on the root element.
    pub fn as_attr(self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Light => "Light",
            ThemeMode::Dark => "Dark",
        }
    }
}

/// Spacing density. The root element carries it as `data-density`, and
/// `css/semantic.css` remaps the row height and gutters for each value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Density {
    Compact,
    /// Look A ships with this one.
    #[default]
    Default,
    Comfortable,
}

impl Density {
    /// Menu order.
    pub const ALL: [Density; 3] = [Density::Compact, Density::Default, Density::Comfortable];

    /// The `data-density` value on the root element.
    pub fn as_attr(self) -> &'static str {
        match self {
            Density::Compact => "compact",
            Density::Default => "default",
            Density::Comfortable => "comfortable",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Density::Compact => "Compact",
            Density::Default => "Default",
            Density::Comfortable => "Comfortable",
        }
    }

    /// A stored value. Anything else (missing, empty, unknown) is `None`.
    pub fn from_attr(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.as_attr() == value)
    }
}

/// localStorage keys. Theme and density persist the same way: per browser.
const THEME_KEY: &str = "sc-theme";
const DENSITY_KEY: &str = "sc-density";

#[derive(Clone, Copy)]
pub struct ThemeCtx {
    pub mode: Signal<ThemeMode>,
    pub density: Signal<Density>,
}

/// Read persisted choice from localStorage, else fall back to prefers-color-scheme.
fn initial_mode() -> ThemeMode {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        if let Some(win) = web_sys::window() {
            if let Ok(Some(storage)) = win.local_storage()
                && let Ok(Some(v)) = storage.get_item(THEME_KEY)
            {
                return if v == "light" {
                    ThemeMode::Light
                } else {
                    ThemeMode::Dark
                };
            }
            if let Ok(Some(mql)) = win.match_media("(prefers-color-scheme: light)")
                && mql.matches()
            {
                return ThemeMode::Light;
            }
        }
    }
    ThemeMode::Dark
}

/// Read the persisted density from localStorage, else the look's default.
fn initial_density() -> Density {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        if let Some(win) = web_sys::window()
            && let Ok(Some(storage)) = win.local_storage()
            && let Ok(Some(v)) = storage.get_item(DENSITY_KEY)
            && let Some(density) = Density::from_attr(&v)
        {
            return density;
        }
    }
    Density::default()
}

/// Write one root attribute and persist it under `key`.
fn apply_root_attr(attr: &str, key: &str, value: &str) {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    if let Some(win) = web_sys::window() {
        if let Some(el) = win.document().and_then(|d| d.document_element()) {
            let _ = el.set_attribute(attr, value);
        }
        if let Ok(Some(storage)) = win.local_storage() {
            let _ = storage.set_item(key, value);
        }
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    let _ = (attr, key, value);
}

fn apply(mode: ThemeMode) {
    apply_root_attr("data-theme", THEME_KEY, mode.as_attr());
}

fn apply_density(density: Density) {
    apply_root_attr(DENSITY_ATTR, DENSITY_KEY, density.as_attr());
}

/// The root attribute `css/semantic.css` keys the density remap on.
pub const DENSITY_ATTR: &str = "data-density";

#[component]
pub fn ThemeProvider(children: Element) -> Element {
    let mode = use_signal(initial_mode);
    let density = use_signal(initial_density);
    use_context_provider(|| ThemeCtx { mode, density });
    use_effect(move || apply(mode()));
    use_effect(move || apply_density(density()));
    rsx! { {children} }
}

#[component]
pub fn ThemeToggle() -> Element {
    let mut ctx = use_context::<ThemeCtx>();
    let is_dark = (ctx.mode)() == ThemeMode::Dark;
    rsx! {
        button {
            class: "theme-toggle",
            "aria-label": "Toggle color theme",
            onclick: move |_| {
                let next = if is_dark { ThemeMode::Light } else { ThemeMode::Dark };
                ctx.mode.set(next);
            },
            // Inline SVG so the icon is font-independent. Dark mode shows a sun
            // (click → light); light mode shows a moon (click → dark).
            if is_dark {
                svg {
                    width: "18", height: "18", view_box: "0 0 24 24", fill: "none",
                    stroke: "currentColor", stroke_width: "2",
                    stroke_linecap: "round", stroke_linejoin: "round",
                    circle { cx: "12", cy: "12", r: "4" }
                    line { x1: "12", y1: "2", x2: "12", y2: "4" }
                    line { x1: "12", y1: "20", x2: "12", y2: "22" }
                    line { x1: "2", y1: "12", x2: "4", y2: "12" }
                    line { x1: "20", y1: "12", x2: "22", y2: "12" }
                    line { x1: "4.93", y1: "4.93", x2: "6.34", y2: "6.34" }
                    line { x1: "17.66", y1: "17.66", x2: "19.07", y2: "19.07" }
                    line { x1: "4.93", y1: "19.07", x2: "6.34", y2: "17.66" }
                    line { x1: "17.66", y1: "6.34", x2: "19.07", y2: "4.93" }
                }
            } else {
                svg {
                    width: "18", height: "18", view_box: "0 0 24 24", fill: "none",
                    stroke: "currentColor", stroke_width: "2",
                    stroke_linecap: "round", stroke_linejoin: "round",
                    path { d: "M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each density writes the attribute value the token remap is keyed on.
    #[test]
    fn density_attribute_values_match_the_token_remap() {
        let semantic = include_str!("css/semantic.css");
        let values: Vec<&str> = Density::ALL.iter().map(|d| d.as_attr()).collect();
        assert_eq!(values, ["compact", "default", "comfortable"]);
        for density in Density::ALL {
            let rule = format!("[{DENSITY_ATTR}=\"{}\"] {{", density.as_attr());
            assert!(semantic.contains(&rule), "no remap for {rule}");
            assert_eq!(Density::from_attr(density.as_attr()), Some(density));
        }
        assert_eq!(Density::default(), Density::Default);
    }

    #[test]
    fn unknown_stored_density_is_ignored() {
        for raw in ["", "Compact", "dense", "default "] {
            assert_eq!(Density::from_attr(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn density_persists_next_to_the_theme() {
        assert_eq!(THEME_KEY, "sc-theme");
        assert_eq!(DENSITY_KEY, "sc-density");
        let src = include_str!("provider.rs");
        let prod = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        assert!(prod.contains("apply_root_attr(\"data-theme\", THEME_KEY, mode.as_attr())"));
        assert!(prod.contains("apply_root_attr(DENSITY_ATTR, DENSITY_KEY, density.as_attr())"));
    }
}
