use leptos::prelude::*;
use leptos_meta::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};
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

use crate::pages::{HomePage, PropagandaPage};

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    provide_context(ToastState::new());

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
