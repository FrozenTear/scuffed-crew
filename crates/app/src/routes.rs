use dioxus::prelude::*;

use crate::head::HeadLayout;
use crate::layouts::{AdminLayout, PublicLayout, StrategyLayout};
use crate::pages::*;

#[derive(Clone, Routable, Debug, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    // One head for every route. It rewrites the existing title and meta tags
    // so navigation does not leave a stale or second copy.
    #[layout(HeadLayout)]
    // First-boot + login (minimal chrome via public layout)
    #[layout(PublicLayout)]
        #[route("/setup")]
        Setup {},
        #[route("/login")]
        Login {},
        #[route("/link")]
        LinkDevice {},
        #[route("/")]
        Home {},
        #[route("/members")]
        Members {},
        #[route("/members/:id")]
        MemberProfile { id: String },
        #[route("/teams/:id")]
        TeamPage { id: String },
        #[route("/matches/:id")]
        MatchDetail { id: String },
        #[route("/news")]
        News {},
        #[route("/apply")]
        Apply {},
        #[route("/tournaments")]
        Tournaments {},
        #[route("/tournaments/:id")]
        Tournament { id: String },
        #[route("/identity")]
        IdentitySettings {},
        #[route("/profile/edit")]
        ProfileSettings {},
        #[route("/reports")]
        MyReports {},
        #[route("/dm")]
        DmInbox {},
        #[route("/dm/:peer_pubkey")]
        DmThread { peer_pubkey: String },
        #[route("/chat")]
        TeamChat {},
        #[route("/community")]
        Community {},
        #[route("/feed")]
        Feed {},
        #[route("/polls")]
        Polls {},
        #[route("/scrims")]
        Scrims {},
        #[route("/events")]
        Events {},
        #[route("/blog")]
        Blog {},
        #[route("/blog/:slug")]
        BlogPost { slug: String },
        #[route("/wiki")]
        Wiki {},
        #[route("/wiki/:topic")]
        WikiPage { topic: String },
        #[route("/forum")]
        Forum {},
        #[route("/forum/b/:slug")]
        ForumBoardPage { slug: String },
        #[route("/forum/t/:id")]
        ForumThread { id: String },
        #[route("/leaderboards")]
        Leaderboards {},
        #[route("/stats")]
        Stats {},
        #[route("/stats/tokens")]
        StatsTokens {},
        #[route("/stats/member/:id")]
        StatsMember { id: String },
        #[route("/stats/match/:id")]
        StatsMatch { id: String },
        #[route("/patch-notes")]
        PatchNotes {},
        #[redirect("/strategies", || Route::StrategyBrowse {})]
        #[redirect("/matches", || Route::Home {})]
        #[route("/:..segments")]
        NotFound { segments: Vec<String> },
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
        #[route("/admin/seasons")]
        AdminSeasons {},
        #[route("/admin/tournaments")]
        AdminTournaments {},
        #[route("/admin/announcements")]
        AdminAnnouncements {},
        #[route("/admin/articles")]
        AdminArticles {},
        #[route("/admin/patch-notes")]
        AdminPatchNotes {},
        #[route("/admin/forum")]
        AdminForum {},
        #[route("/admin/audit-log")]
        AdminAuditLog {},
        #[route("/admin/moderation")]
        AdminModeration {},
        #[route("/admin/reports")]
        AdminReports {},
        #[route("/admin/relay")]
        AdminRelay {},
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
}

#[cfg(test)]
mod tests {
    use super::Route;
    use std::str::FromStr;

    #[test]
    fn unknown_paths_fall_through_to_the_catch_all() {
        for (path, segments) in [
            ("/nope", vec!["nope"]),
            ("/nope/deeper", vec!["nope", "deeper"]),
            ("/admin/nope", vec!["admin", "nope"]),
            ("/strategy/nope", vec!["strategy", "nope"]),
        ] {
            assert_eq!(
                Route::from_str(path).unwrap(),
                Route::NotFound {
                    segments: segments.into_iter().map(String::from).collect()
                },
                "{path}"
            );
        }
    }

    #[test]
    fn static_and_dynamic_routes_still_win_over_the_catch_all() {
        for path in [
            "/",
            "/members",
            "/members/abc",
            "/matches/abc",
            "/leaderboards",
            "/stats",
            "/stats/tokens",
            "/stats/match/abc",
            "/patch-notes",
            "/admin",
            "/admin/members",
            "/strategy",
            "/strategy/editor/abc",
            "/login",
        ] {
            let route = Route::from_str(path).unwrap_or_else(|e| panic!("{path}: {e:?}"));
            assert!(
                !matches!(route, Route::NotFound { .. }),
                "{path} fell into the catch-all"
            );
        }
    }

    #[test]
    fn strategies_and_matches_redirect_to_real_pages() {
        assert_eq!(
            Route::from_str("/strategies").unwrap(),
            Route::StrategyBrowse {}
        );
        assert_eq!(Route::from_str("/matches").unwrap(), Route::Home {});
    }

    #[test]
    fn patch_notes_has_public_and_strategy_routes() {
        assert_eq!(
            Route::from_str("/patch-notes").unwrap(),
            Route::PatchNotes {}
        );
        assert_eq!(
            Route::from_str("/strategy/patch-notes").unwrap(),
            Route::StrategyPatchNotes {}
        );
        assert_eq!(Route::PatchNotes {}.to_string(), "/patch-notes");
        assert_eq!(
            Route::StrategyPatchNotes {}.to_string(),
            "/strategy/patch-notes"
        );
    }

    #[test]
    fn public_patch_notes_is_not_under_strategy_layout_path() {
        assert_eq!(Route::PatchNotes {}.to_string(), "/patch-notes");
        assert!(!Route::PatchNotes {}.to_string().starts_with("/strategy"));
    }

    #[test]
    fn report_routes_parse() {
        assert_eq!(Route::from_str("/reports").unwrap(), Route::MyReports {});
        assert_eq!(Route::MyReports {}.to_string(), "/reports");
        assert_eq!(
            Route::from_str("/admin/reports").unwrap(),
            Route::AdminReports {}
        );
        assert_eq!(Route::AdminReports {}.to_string(), "/admin/reports");
    }

    #[test]
    fn stats_match_route_parses() {
        assert_eq!(
            Route::from_str("/stats/match/m1").unwrap(),
            Route::StatsMatch { id: "m1".into() }
        );
        assert_eq!(
            Route::StatsMatch { id: "m1".into() }.to_string(),
            "/stats/match/m1"
        );
        assert_eq!(Route::from_str("/stats").unwrap(), Route::Stats {});
    }

    #[test]
    fn link_route_drops_a_code_query() {
        let parsed = Route::from_str("/link?code=ABCD").expect("link route");
        assert_eq!(parsed, Route::LinkDevice {});
        assert_eq!(parsed.to_string(), "/link");
        assert!(!parsed.to_string().contains('?'));
        assert!(!parsed.to_string().contains("ABCD"));
    }
}
