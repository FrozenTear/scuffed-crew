use leptos::prelude::*;

/// Card component with optional hover glow effect.
#[component]
pub fn Card(
    #[prop(optional)] hover_glow: bool,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let glow_class = if hover_glow { " sc-card-glow" } else { "" };
    let classes = format!("sc-card{} {}", glow_class, class);

    view! {
        <div class={classes}>
            {children()}
        </div>
    }
}

pub const CARD_STYLES: &str = r#"
.sc-card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.5rem;
    transition: border-color 0.3s ease, box-shadow 0.3s ease;
}
.sc-card-glow:hover {
    border-color: var(--accent);
    box-shadow: 0 0 30px var(--accent-glow);
}
"#;
