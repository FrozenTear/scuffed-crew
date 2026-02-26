use leptos::prelude::*;

/// User avatar with image and fallback initials.
#[component]
pub fn UserAvatar(
    /// The user's avatar URL (if any)
    #[prop(optional, into)]
    src: Option<String>,
    /// The user's display name (used for fallback initials)
    #[prop(into)]
    name: String,
    /// Size in pixels (default: 32)
    #[prop(default = 32)]
    size: u32,
) -> impl IntoView {
    let initials = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    let size_px = format!("{}px", size);
    let font_size = format!("{}px", size / 3);

    view! {
        <div
            class="sc-avatar"
            style:width={size_px.clone()}
            style:height={size_px}
            style:font-size={font_size}
        >
            {match src {
                Some(url) => view! {
                    <img src={url} alt={name} class="sc-avatar-img" />
                }.into_any(),
                None => view! {
                    <span class="sc-avatar-initials">{initials}</span>
                }.into_any(),
            }}
        </div>
    }
}

pub const AVATAR_STYLES: &str = r#"
.sc-avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    overflow: hidden;
    background: var(--accent-soft);
    color: var(--accent-bright);
    font-family: var(--font-display);
    font-weight: 700;
    flex-shrink: 0;
}
.sc-avatar-img {
    width: 100%;
    height: 100%;
    object-fit: cover;
}
"#;
