use leptos::prelude::*;

#[component]
pub fn Footer() -> impl IntoView {
    view! {
        <footer>
            <div class="footer-mark">"The Scuffed Crew"</div>
            <div class="footer-links">
                <a href="/#about">"About"</a>
                <a href="/#teams">"Teams"</a>
                <a href="/#schedule">"Schedule"</a>
                <a href="/#join">"Join"</a>
            </div>
            <div class="footer-copy">"\u{00A9} 2026 The Scuffed Crew"</div>
            <div class="footer-privacy">"No tracking \u{00B7} No cookies \u{00B7} No ads"</div>
        </footer>
    }
}
