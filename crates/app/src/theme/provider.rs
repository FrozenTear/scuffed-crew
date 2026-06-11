use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum ThemeMode {
    Light,
    Dark,
}

impl ThemeMode {
    #[cfg_attr(not(feature = "web"), allow(dead_code))]
    fn as_attr(self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }
}

#[derive(Clone, Copy)]
pub struct ThemeCtx {
    pub mode: Signal<ThemeMode>,
}

/// Read persisted choice from localStorage, else fall back to prefers-color-scheme.
fn initial_mode() -> ThemeMode {
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window() {
            if let Ok(Some(storage)) = win.local_storage()
                && let Ok(Some(v)) = storage.get_item("sc-theme")
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

fn apply(mode: ThemeMode) {
    #[cfg(feature = "web")]
    if let Some(win) = web_sys::window() {
        if let Some(el) = win.document().and_then(|d| d.document_element()) {
            let _ = el.set_attribute("data-theme", mode.as_attr());
        }
        if let Ok(Some(storage)) = win.local_storage() {
            let _ = storage.set_item("sc-theme", mode.as_attr());
        }
    }
    #[cfg(not(feature = "web"))]
    let _ = mode;
}

#[component]
pub fn ThemeProvider(children: Element) -> Element {
    let mode = use_signal(initial_mode);
    use_context_provider(|| ThemeCtx { mode });
    use_effect(move || apply(mode()));
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
