use dioxus::prelude::*;

use crate::routes::Route;

const NOT_FOUND_CSS: &str = r#"
    .nf-page {
        min-height: 100vh;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        text-align: center;
        padding: 4rem 1.5rem;
        background: var(--bg);
        color: var(--text);
    }
    .nf-code {
        font-family: var(--font-head);
        font-size: clamp(4rem, 18vw, 6.5rem);
        line-height: 1;
        letter-spacing: 0.04em;
        color: var(--text);
        margin: 0;
    }
    .nf-title {
        font-family: var(--font-head);
        font-size: 1.25rem;
        margin: 1rem 0 0.35rem;
        color: var(--text);
    }
    .nf-msg {
        color: var(--text-2);
        font-size: 0.95rem;
        max-width: 28rem;
        margin: 0 0 1.5rem;
        line-height: 1.5;
    }
    .nf-path {
        font-family: var(--font-mono);
        font-size: 0.72rem;
        color: var(--text-3);
        letter-spacing: 0.04em;
        margin-bottom: 1.5rem;
        word-break: break-all;
    }
    .nf-link {
        display: inline-flex;
        align-items: center;
        padding: 0.55rem 1.1rem;
        background: var(--accent);
        color: var(--accent-fg);
        font-family: var(--font-mono);
        font-size: 0.72rem;
        letter-spacing: 0.1em;
        text-transform: uppercase;
        text-decoration: none;
        border-radius: var(--radius-md, 8px);
    }
    .nf-link:hover { filter: brightness(1.08); }
"#;

#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let path = if segments.is_empty() {
        String::new()
    } else {
        format!("/{}", segments.join("/"))
    };

    rsx! {
        style { {NOT_FOUND_CSS} }
        div { class: "nf-page",
            h1 { class: "nf-code", "404" }
            p { class: "nf-title", "Page not found" }
            p { class: "nf-msg",
                "That route doesn't exist. It may have moved, or the link is wrong."
            }
            if !path.is_empty() {
                p { class: "nf-path", "{path}" }
            }
            Link { to: Route::Home {}, class: "nf-link", "Return home" }
        }
    }
}
