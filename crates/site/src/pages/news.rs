use leptos::prelude::*;
use serde::Deserialize;

use scuffed_auth::client::api::fetch_json;

use crate::components::Nav;
use crate::sections::Footer;

#[derive(Debug, Clone, Deserialize)]
struct Announcement {
    #[allow(dead_code)]
    id: String,
    title: String,
    content: String,
    pinned: bool,
    created_at: String,
}

#[component]
pub fn NewsPage() -> impl IntoView {
    let announcements = LocalResource::new(|| async {
        fetch_json::<Vec<Announcement>>("/api/announcements").await.ok()
    });

    view! {
        <Nav/>
        <main class="news-page">
            <h1 class="news-page-title">"News & Announcements"</h1>

            {move || match announcements.get().flatten() {
                None => view! { <p class="news-loading">"Loading..."</p> }.into_any(),
                Some(list) if list.is_empty() => view! { <p class="news-empty">"No announcements yet."</p> }.into_any(),
                Some(list) => view! {
                    <div class="news-list">
                        {list.into_iter().map(|a| {
                            let date = a.created_at.chars().take(10).collect::<String>();
                            let card_class = if a.pinned { "news-card pinned" } else { "news-card" };
                            let pin_badge = if a.pinned { Some(view! { <span class="news-pin">"Pinned"</span> }) } else { None };
                            view! {
                                <article class=card_class>
                                    <div class="news-meta">
                                        <time>{date}</time>
                                        {pin_badge}
                                    </div>
                                    <h2 class="news-title">{a.title}</h2>
                                    <p class="news-body">{a.content}</p>
                                </article>
                            }
                        }).collect_view()}
                    </div>
                }.into_any(),
            }}
        </main>
        <Footer/>
    }
}
