use dioxus::prelude::*;

use crate::routes::Route;

#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    rsx! {
        div {
            style: "text-align:center; padding:6rem 2rem; min-height:100vh; display:flex; flex-direction:column; align-items:center; justify-content:center;",
            h1 {
                style: "font-family:var(--font-head); font-size:6rem; color:var(--text);",
                "404"
            }
            p { style: "color:var(--text-2); margin-bottom:2rem;",
                "Page not found"
            }
            Link { to: Route::Home {}, style: "color:var(--accent);", "Return home" }
        }
    }
}
