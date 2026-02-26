use leptos::prelude::*;

#[component]
pub fn Nav() -> impl IntoView {
    let (menu_open, set_menu_open) = signal(false);

    let hamburger_class = move || {
        if menu_open.get() {
            "nav-hamburger open"
        } else {
            "nav-hamburger"
        }
    };

    let overlay_class = move || {
        if menu_open.get() {
            "nav-overlay open"
        } else {
            "nav-overlay"
        }
    };

    let close_menu = move |_| set_menu_open.set(false);

    view! {
        <nav>
            <a href="/" class="nav-mark">
                <div class="nav-icon">"SC"</div>
                <span>"The Scuffed Crew"</span>
            </a>
            <ul class="nav-links">
                <li><a href="#about">"About"</a></li>
                <li><a href="#teams">"Teams"</a></li>
                <li><a href="#comms">"Comms"</a></li>
                <li><a href="#schedule">"Schedule"</a></li>
                <li><a href="#join" class="nav-cta">"Join Us"</a></li>
            </ul>
            <button
                class=hamburger_class
                aria-label="Toggle menu"
                on:click=move |_| set_menu_open.update(|v| *v = !*v)
            >
                <span></span><span></span><span></span>
            </button>
        </nav>

        <div class=overlay_class>
            <a href="#about" on:click=close_menu>"About"</a>
            <a href="#teams" on:click=close_menu>"Teams"</a>
            <a href="#comms" on:click=close_menu>"Comms"</a>
            <a href="#schedule" on:click=close_menu>"Schedule"</a>
            <a href="#join" class="btn btn-primary" style="margin-top:1rem;" on:click=close_menu>"Join Us"</a>
        </div>
    }
}
