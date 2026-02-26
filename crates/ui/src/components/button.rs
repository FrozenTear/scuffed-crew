use leptos::ev;
use leptos::prelude::*;

/// Button variant
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Ghost,
    Danger,
}

impl ButtonVariant {
    fn css_class(&self) -> &'static str {
        match self {
            ButtonVariant::Primary => "sc-btn sc-btn-primary",
            ButtonVariant::Secondary => "sc-btn sc-btn-secondary",
            ButtonVariant::Ghost => "sc-btn sc-btn-ghost",
            ButtonVariant::Danger => "sc-btn sc-btn-danger",
        }
    }
}

/// Shared button component using CSS variables from the theme.
#[component]
pub fn Button(
    #[prop(default = ButtonVariant::Primary)] variant: ButtonVariant,
    #[prop(optional)] disabled: bool,
    #[prop(optional, into)] class: String,
    #[prop(optional)] on_click: Option<Callback<ev::MouseEvent>>,
    children: Children,
) -> impl IntoView {
    let classes = format!("{} {}", variant.css_class(), class);

    view! {
        <button
            class={classes}
            disabled={disabled}
            on:click=move |ev| {
                if let Some(cb) = &on_click {
                    cb.run(ev);
                }
            }
        >
            {children()}
        </button>
    }
}

/// Inline styles for the button system (injected once).
///
/// Uses CSS custom properties set by `ThemeProvider`.
pub const BUTTON_STYLES: &str = r#"
.sc-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.6rem 1.4rem;
    border: 1px solid transparent;
    border-radius: 6px;
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 0.9rem;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    cursor: pointer;
    transition: all 0.2s ease;
    text-decoration: none;
}
.sc-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
}
.sc-btn-primary {
    background: var(--accent);
    color: var(--text-bright);
    border-color: var(--accent);
}
.sc-btn-primary:hover:not(:disabled) {
    background: var(--accent-bright);
    box-shadow: 0 0 20px var(--accent-glow);
}
.sc-btn-secondary {
    background: transparent;
    color: var(--accent-bright);
    border-color: var(--accent);
}
.sc-btn-secondary:hover:not(:disabled) {
    background: var(--accent-soft);
}
.sc-btn-ghost {
    background: transparent;
    color: var(--text-secondary);
    border-color: transparent;
}
.sc-btn-ghost:hover:not(:disabled) {
    color: var(--text-bright);
    background: var(--bg-elevated);
}
.sc-btn-danger {
    background: var(--danger);
    color: var(--text-bright);
    border-color: var(--danger);
}
.sc-btn-danger:hover:not(:disabled) {
    filter: brightness(1.2);
    box-shadow: 0 0 20px rgba(214, 48, 49, 0.3);
}
"#;
