use leptos::prelude::*;
use serde::Deserialize;

use scuffed_auth::client::api::fetch_json_list;

use crate::components::SectionHeader;

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
pub fn Announcements() -> impl IntoView {
    let announcements = LocalResource::new(|| async {
        fetch_json_list::<Announcement>("/api/announcements").await.ok()
    });

    view! {
        <section id="news">
            <SectionHeader
                label="// Latest News"
                title="Announcements"
                color="purple"
                description="What's happening in the crew."
            />

            <div class="news-grid" data-reveal="">
                {move || {
                    let list = announcements.get().flatten().unwrap_or_default();
                    // Show up to 3 latest announcements
                    list.into_iter().take(3).map(|a| {
                        let date = a.created_at.chars().take(10).collect::<String>();
                        let pin_badge = if a.pinned { Some(view! { <span class="news-pin">"Pinned"</span> }) } else { None };
                        view! {
                            <article class="news-card" data-reveal="">
                                <div class="news-meta">
                                    <time>{date}</time>
                                    {pin_badge}
                                </div>
                                <h3 class="news-title">{a.title}</h3>
                                <p class="news-body">{a.content}</p>
                            </article>
                        }
                    }).collect_view()
                }}
            </div>

            <div class="news-more" data-reveal="">
                <a href="/news" class="btn btn-secondary">"View All News"</a>
            </div>
        </section>
    }
}
