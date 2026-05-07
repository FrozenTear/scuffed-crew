mod daemon;
mod history;
mod preview;
mod progression;
mod settings;
mod stats;
mod status;
mod style;
mod tray;

use dioxus::prelude::*;

fn main() {
    let _ = gtk::init();
    let _tray = tray::try_create_tray();
    if let Some(ref handle) = _tray {
        let quit_id = handle.quit_id.clone();
        std::thread::spawn(move || loop {
            if tray::poll_quit(&quit_id) {
                std::process::exit(0);
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        });
    }

    dioxus::launch(app);
}

#[derive(Clone, Routable, PartialEq)]
enum Route {
    #[route("/")]
    Home {},
    #[route("/history")]
    History {},
    #[route("/stats")]
    Stats {},
    #[route("/progression")]
    Progression {},
    #[route("/preview")]
    Preview {},
    #[route("/settings")]
    Settings {},
}

#[component]
fn Home() -> Element {
    rsx! {
        div { class: "app",
            Nav {}
            status::StatusPanel {}
        }
    }
}

#[component]
fn History() -> Element {
    rsx! {
        div { class: "app",
            Nav {}
            history::HistoryPanel {}
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
fn Progression() -> Element {
    rsx! {
        div { class: "app",
            Nav {}
            progression::ProgressionPanel {}
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
                Link { to: Route::Home {}, "Status" }
                Link { to: Route::History {}, "Matches" }
                Link { to: Route::Stats {}, "Stats" }
                Link { to: Route::Progression {}, "Progression" }
                Link { to: Route::Preview {}, "Preview" }
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
