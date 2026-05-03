use dioxus::prelude::*;

use crate::layouts::{AdminLayout, PublicLayout, StrategyLayout};
use crate::pages::*;

#[derive(Clone, Routable, Debug, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    // Public site (top nav + footer)
    #[layout(PublicLayout)]
        #[route("/")]
        Home {},
        #[route("/members")]
        Members {},
        #[route("/members/:id")]
        MemberProfile { id: String },
        #[route("/news")]
        News {},
        #[route("/polls")]
        Polls {},
        #[route("/apply")]
        Apply {},
        #[route("/tournaments")]
        Tournaments {},
        #[route("/tournaments/:id")]
        Tournament { id: String },
        #[route("/events")]
        Events {},
        #[route("/identity")]
        Identity {},
        #[route("/blog")]
        Blog {},
        #[route("/blog/:slug")]
        BlogArticle { slug: String },
    #[end_layout]

    // Admin panel (sidebar layout, auth guarded)
    #[layout(AdminLayout)]
        #[route("/admin")]
        AdminDashboard {},
        #[route("/admin/members")]
        AdminMembers {},
        #[route("/admin/games")]
        AdminGames {},
        #[route("/admin/teams")]
        AdminTeams {},
        #[route("/admin/schedule")]
        AdminSchedule {},
        #[route("/admin/applications")]
        AdminApplications {},
        #[route("/admin/matches")]
        AdminMatches {},
        #[route("/admin/tournaments")]
        AdminTournaments {},
        #[route("/admin/announcements")]
        AdminAnnouncements {},
        #[route("/admin/articles")]
        AdminArticles {},
        #[route("/admin/audit-log")]
        AdminAuditLog {},
        #[route("/admin/moderation")]
        AdminModeration {},
        #[route("/admin/settings")]
        AdminSettings {},
    #[end_layout]

    // Strategy section (strategy nav layout)
    #[layout(StrategyLayout)]
        #[route("/strategy")]
        StrategyBrowse {},
        #[route("/strategy/my")]
        StrategyMy {},
        #[route("/strategy/heroes")]
        StrategyHeroes {},
        #[route("/strategy/meta")]
        StrategyMeta {},
        #[route("/strategy/patch-notes")]
        StrategyPatchNotes {},
        #[route("/strategy/editor")]
        StrategyEditorNew {},
        #[route("/strategy/editor/:id")]
        StrategyEditor { id: String },
    #[end_layout]

    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}
