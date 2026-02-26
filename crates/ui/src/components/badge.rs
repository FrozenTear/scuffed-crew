use leptos::prelude::*;

/// Badge variant
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BadgeVariant {
    #[default]
    Default,
    Success,
    Warning,
    Danger,
    Info,
}

impl BadgeVariant {
    fn css_class(&self) -> &'static str {
        match self {
            BadgeVariant::Default => "sc-badge",
            BadgeVariant::Success => "sc-badge sc-badge-success",
            BadgeVariant::Warning => "sc-badge sc-badge-warning",
            BadgeVariant::Danger => "sc-badge sc-badge-danger",
            BadgeVariant::Info => "sc-badge sc-badge-info",
        }
    }
}

/// A small status badge / pill.
#[component]
pub fn StatusBadge(
    #[prop(default = BadgeVariant::Default)] variant: BadgeVariant,
    children: Children,
) -> impl IntoView {
    view! {
        <span class={variant.css_class()}>
            {children()}
        </span>
    }
}

pub const BADGE_STYLES: &str = r#"
.sc-badge {
    display: inline-flex;
    align-items: center;
    padding: 0.15rem 0.6rem;
    border-radius: 99px;
    font-family: var(--font-display);
    font-size: 0.7rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    background: var(--bg-elevated);
    color: var(--text-secondary);
    border: 1px solid var(--border);
}
.sc-badge-success { color: var(--success); border-color: var(--success); }
.sc-badge-warning { color: var(--warning); border-color: var(--warning); }
.sc-badge-danger { color: var(--danger); border-color: var(--danger); }
.sc-badge-info { color: var(--info); border-color: var(--info); }
"#;
