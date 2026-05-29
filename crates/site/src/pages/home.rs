use crate::components::{Divider, Nav};
use crate::sections::{
    About, Announcements, Comms, Footer, Hero, Recruit, Schedule, Teams, Tournaments,
};
use leptos::prelude::*;

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
