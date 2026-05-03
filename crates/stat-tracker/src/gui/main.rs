mod daemon;
mod preview;
mod settings;
mod status;
mod style;
mod tray;

use dioxus::prelude::*;

fn main() {
    let _tray = tray::try_create_tray();
    if let Some(ref handle) = _tray {
        let show_id = handle.show_id.clone();
        let quit_id = handle.quit_id.clone();
        std::thread::spawn(move || loop {
            if let Some(action) = tray::poll_tray_events(&show_id, &quit_id) {
                match action {
                    tray::TrayAction::Quit => std::process::exit(0),
                    tray::TrayAction::ShowWindow => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
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
