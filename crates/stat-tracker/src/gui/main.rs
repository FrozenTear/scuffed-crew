mod daemon;
mod preview;
mod settings;
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
    #[route("/settings")]
    Settings {},
    #[route("/preview")]
    Preview {},
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
