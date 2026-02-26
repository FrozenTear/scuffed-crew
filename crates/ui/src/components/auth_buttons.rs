use leptos::prelude::*;
use scuffed_auth::AuthProvider;

/// Login buttons for the configured auth providers.
///
/// Each button links to the corresponding OAuth login endpoint.
#[component]
pub fn AuthButtons(
    /// Which providers to show login buttons for
    providers: Vec<AuthProvider>,
    /// Base path for auth endpoints (e.g. "/api/auth")
    #[prop(default = "/api/auth".to_string())]
    base_path: String,
) -> impl IntoView {
    view! {
        <div class="sc-auth-buttons">
            {providers.into_iter().map(|p| {
                let href = format!("{}/{}/login", base_path, p);
                let label = format!("Sign in with {}", provider_display_name(p));
                let class = format!("sc-btn sc-auth-btn sc-auth-{}", p);
                view! {
                    <a href={href} class={class}>
                        {provider_icon(p)}
                        {label}
                    </a>
                }
            }).collect_view()}
        </div>
    }
}

fn provider_display_name(p: AuthProvider) -> &'static str {
    match p {
        AuthProvider::Discord => "Discord",
        AuthProvider::Google => "Google",
        AuthProvider::Matrix => "Matrix",
    }
}

fn provider_icon(p: AuthProvider) -> &'static str {
    match p {
        AuthProvider::Discord => "\u{1f3ae}", // game controller
        AuthProvider::Google => "\u{1f310}",   // globe
        AuthProvider::Matrix => "\u{1f5a7}",   // networked computers
    }
}

pub const AUTH_BUTTONS_STYLES: &str = r#"
.sc-auth-buttons {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    width: 100%;
    max-width: 320px;
}
.sc-auth-btn {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem 1.25rem;
    border-radius: 8px;
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 0.9rem;
    text-decoration: none;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    transition: all 0.2s;
    border: 1px solid var(--border);
    color: var(--text-bright);
    background: var(--bg-card);
}
.sc-auth-btn:hover {
    border-color: var(--accent);
    background: var(--bg-elevated);
}
.sc-auth-discord { border-left: 3px solid #5865F2; }
.sc-auth-google { border-left: 3px solid #4285F4; }
.sc-auth-matrix { border-left: 3px solid #0DBD8B; }
"#;
