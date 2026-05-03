mod settings;
mod status;
mod style;

use dioxus::prelude::*;

fn main() {
    dioxus::launch(app);
}

#[derive(Clone, Routable, PartialEq)]
enum Route {
    #[route("/")]
    Home {},
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
fn Settings() -> Element {
    rsx! {
        div { class: "app",
            Nav {}
            settings::SettingsPanel {}
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
