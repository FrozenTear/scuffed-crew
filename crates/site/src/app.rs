use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_meta::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};
use serde::Deserialize;
use scuffed_ui::{scuffed_crew_theme, ThemeProvider};
use scuffed_ui::components::toast::{ToastContainer, ToastState};
use scuffed_ui::components::button::BUTTON_STYLES;
use scuffed_ui::components::card::CARD_STYLES;
use scuffed_ui::components::nav::NAV_STYLES;
use scuffed_ui::components::toast::TOAST_STYLES;
use scuffed_ui::components::badge::BADGE_STYLES;
use scuffed_ui::components::modal::MODAL_STYLES;
use scuffed_ui::components::avatar::AVATAR_STYLES;
use scuffed_ui::components::auth_buttons::AUTH_BUTTONS_STYLES;
use scuffed_auth::client::api;

use crate::pages::{ApplyPage, HomePage, MemberProfilePage, MembersPage, NewsPage, PropagandaPage, TournamentPage, TournamentsPage};

/// Response from /api/auth/me
#[derive(Debug, Clone, Deserialize)]
struct MeResponse {
    user: MeUser,
    member: Option<MeMember>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MeUser {
    #[allow(dead_code)]
    pub id: String,
    #[allow(dead_code)]
    pub username: String,
    #[allow(dead_code)]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MeMember {
    pub id: String,
    pub org_role: String,
    pub display_name: String,
}

/// Site-wide auth state provided via context.
#[derive(Clone, Copy)]
pub struct SiteAuth {
    pub user: RwSignal<Option<MeUser>>,
    pub member: RwSignal<Option<MeMember>>,
    pub loading: RwSignal<bool>,
}

impl SiteAuth {
    pub fn new() -> Self {
        Self {
            user: RwSignal::new(None),
            member: RwSignal::new(None),
            loading: RwSignal::new(true),
        }
    }

    pub fn is_logged_in(&self) -> bool {
        self.user.get().is_some()
    }

    pub fn is_member(&self) -> bool {
        self.member.get().is_some()
    }

    fn fetch(&self) {
        let user_sig = self.user;
        let member_sig = self.member;
        let loading_sig = self.loading;
        spawn_local(async move {
            loading_sig.set(true);
            match api::fetch_json::<MeResponse>("/api/auth/me").await {
                Ok(resp) => {
                    user_sig.set(Some(resp.user));
                    member_sig.set(resp.member);
                }
                Err(_) => {
                    user_sig.set(None);
                    member_sig.set(None);
                }
            }
            loading_sig.set(false);
        });
    }
}

pub fn use_site_auth() -> SiteAuth {
    expect_context::<SiteAuth>()
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    provide_context(ToastState::new());

    let auth = SiteAuth::new();
    auth.fetch();
    provide_context(auth);

    let theme = scuffed_crew_theme();

    // Collect all scuffed-ui component styles
    let component_styles = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        BUTTON_STYLES, CARD_STYLES, NAV_STYLES, TOAST_STYLES,
        BADGE_STYLES, MODAL_STYLES, AVATAR_STYLES, AUTH_BUTTONS_STYLES,
    );

    view! {
        <Stylesheet href="/assets/styles/main.css"/>
        <Stylesheet href="/assets/styles/propaganda.css"/>
        <Title text="The Scuffed Crew \u{2014} Gaming Org"/>
        <Meta name="description" content="A multi-game gaming org built on old-school clan principles."/>
        <Meta name="viewport" content="width=device-width, initial-scale=1.0"/>
        <Style>{component_styles}</Style>

        <ThemeProvider theme=theme>
            <Router>
                <Routes fallback=|| view! { <NotFound/> }>
                    <Route path=path!("/") view=HomePage/>
                    <Route path=path!("/news") view=NewsPage/>
                    <Route path=path!("/members") view=MembersPage/>
                    <Route path=path!("/members/:id") view=MemberProfilePage/>
                    <Route path=path!("/apply") view=ApplyPage/>
                    <Route path=path!("/tournaments") view=TournamentsPage/>
                    <Route path=path!("/tournaments/:id") view=TournamentPage/>
                    <Route path=path!("/propaganda") view=PropagandaPage/>
                </Routes>
                <ToastContainer/>
            </Router>
        </ThemeProvider>
    }
}

#[component]
fn NotFound() -> impl IntoView {
    view! {
        <div style="text-align: center; padding: 6rem 2rem; min-height: 100vh; display: flex; flex-direction: column; align-items: center; justify-content: center;">
            <h1 style="font-family: 'Bebas Neue', sans-serif; font-size: 6rem; color: var(--text-bright);">"404"</h1>
            <p style="color: var(--text-secondary); margin-bottom: 2rem;">"Page not found"</p>
            <a href="/" style="color: var(--accent);">"Return home"</a>
        </div>
    }
}
