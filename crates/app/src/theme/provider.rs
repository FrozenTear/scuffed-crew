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
            if is_dark { "\u{2600}" } else { "\u{1f319}" }
        }
    }
}
