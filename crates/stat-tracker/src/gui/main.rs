mod daemon;
mod history;
mod preview;
mod settings;
mod stats;
mod status;
mod style;
mod tray;

use dioxus::desktop::tao::dpi::LogicalSize;
use dioxus::desktop::{Config as DesktopConfig, WindowBuilder};
use dioxus::prelude::*;

fn main() {
    let _ = gtk::init();
    let _tray = tray::try_create_tray();
    if let Some(ref handle) = _tray {
        let quit_id = handle.quit_id.clone();
        std::thread::spawn(move || {
            loop {
                if tray::poll_quit(&quit_id) {
                    std::process::exit(0);
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        });
    }

    let window = WindowBuilder::new()
        .with_title("Scuffed Stat Tracker")
        .with_inner_size(LogicalSize::new(1240.0, 860.0));
    dioxus::LaunchBuilder::desktop()
        .with_cfg(DesktopConfig::new().with_window(window).with_menu(None))
        .launch(app);
}

#[derive(Clone, Routable, PartialEq)]
enum Route {
    #[route("/")]
    Dashboard {},
    #[route("/matches")]
    Matches {},
    #[route("/stats")]
    Stats {},
    #[route("/settings")]
    Settings {},
    // Diagnostics view, reachable from Settings (not in the nav).
    #[route("/preview")]
    Preview {},
}

#[component]
fn Dashboard() -> Element {
    // Diagnostics: SST_VIEW=matches|stats|settings|preview jumps straight to
    // that view at startup — used by screenshot tooling for UI review.
    use_hook(|| {
        static APPLIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !APPLIED.swap(true, std::sync::atomic::Ordering::SeqCst)
            && let Ok(view) = std::env::var("SST_VIEW")
        {
            let nav = navigator();
            match view.as_str() {
                "matches" => nav.push(Route::Matches {}),
                "stats" => nav.push(Route::Stats {}),
                "settings" => nav.push(Route::Settings {}),
                "preview" => nav.push(Route::Preview {}),
                _ => None,
            };
        }
    });
    rsx! {
        div { class: "app",
            Nav {}
            status::StatusPanel {}
        }
    }
}

#[component]
fn Matches() -> Element {
    rsx! {
        div { class: "app",
            Nav {}
            history::MatchesPanel {}
        }
    }
}

#[component]
fn Stats() -> Element {
    rsx! {
        div { class: "app",
            Nav {}
            stats::StatsPanel {}
        }
    }
}

#[component]
fn Settings() -> Element {
    rsx! {
        div { class: "app",
            Nav {}
            settings::SettingsPanel {}
        }
    }
}

#[component]
fn Preview() -> Element {
    rsx! {
        div { class: "app",
            Nav {}
            preview::PreviewPanel {}
        }
    }
}

#[component]
fn Nav() -> Element {
    rsx! {
        nav { class: "nav",
            h1 { class: "logo", "Scuffed Stat Tracker" }
            div { class: "nav-links",
                Link { to: Route::Dashboard {}, "Dashboard" }
                Link { to: Route::Matches {}, "Matches" }
                Link { to: Route::Stats {}, "Stats" }
                Link { to: Route::Settings {}, "Settings" }
            }
        }
    }
}

fn app() -> Element {
    rsx! {
        style { {style::CSS} }
        Router::<Route> {}
    }
}
