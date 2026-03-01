use leptos::prelude::*;
use crate::components::{Nav, Divider};
use crate::sections::{Hero, About, Teams, Comms, Schedule, Announcements, Tournaments, Recruit, Footer};

#[component]
pub fn HomePage() -> impl IntoView {
    view! {
        <Nav/>
        <Hero/>
        <Divider/>
        <About/>
        <Divider/>
        <Teams/>
        <Divider/>
        <Announcements/>
        <Divider/>
        <Tournaments/>
        <Divider/>
        <Comms/>
        <Divider/>
        <Schedule/>
        <Divider/>
        <Recruit/>
        <Footer/>
    }
}
