//! The site's navigation model: three fixed groups (Community, Compete,
//! Strategy), the account entries and the admin sub-nav.
//!
//! The desktop mega menu, the phone drawer, the tab bar and the footer all
//! render from these lists, so a page added here shows up in every one of them.

use crate::routes::Route;

/// One of the three top bar groups. The slug is the stable part of the
/// element ids for that group's disclosure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NavGroupId {
    Community,
    Compete,
    Strategy,
}

impl NavGroupId {
    pub(crate) const ALL: [NavGroupId; 3] = [
        NavGroupId::Community,
        NavGroupId::Compete,
        NavGroupId::Strategy,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            NavGroupId::Community => "Community",
            NavGroupId::Compete => "Compete",
            NavGroupId::Strategy => "Strategy",
        }
    }

    /// Id of the top bar button that opens this group's panel.
    pub(crate) fn toggle_id(self) -> &'static str {
        match self {
            NavGroupId::Community => "nav-group-community-toggle",
            NavGroupId::Compete => "nav-group-compete-toggle",
            NavGroupId::Strategy => "nav-group-strategy-toggle",
        }
    }

    /// Id of this group's panel, the target of `aria-controls`.
    pub(crate) fn panel_id(self) -> &'static str {
        match self {
            NavGroupId::Community => "nav-group-community-panel",
            NavGroupId::Compete => "nav-group-compete-panel",
            NavGroupId::Strategy => "nav-group-strategy-panel",
        }
    }
}

/// One link. `id` matches the `NAV_CATALOG` id where the page has one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NavEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub route: Route,
}

fn entry(id: &'static str, label: &'static str, route: Route) -> NavEntry {
    NavEntry { id, label, route }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NavGroup {
    pub id: NavGroupId,
    pub entries: Vec<NavEntry>,
}

impl NavGroup {
    /// Where the phone tab for this group goes: the group's first page.
    pub(crate) fn landing(&self) -> Option<Route> {
        self.entries.first().map(|entry| entry.route.clone())
    }
}

/// What decides which entries a visitor sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NavContext {
    pub signed_in: bool,
    /// Officer or admin: the tier `AdminLayout` lets in.
    pub admin_access: bool,
    /// Full admin: sees the admin-only pages inside the admin section.
    pub is_admin: bool,
    /// `GET /api/settings`.strategies_enabled (default on).
    pub strategies_enabled: bool,
    /// False only after relay health says the relay is off.
    pub relay_on: bool,
}

/// The three groups, in top bar order. A group with no visible entries is
/// left out, so the bar never shows a button that opens an empty panel.
pub(crate) fn nav_groups(ctx: NavContext) -> Vec<NavGroup> {
    let mut community = vec![
        entry("members", "Members", Route::Members {}),
        entry("forum", "Forum", Route::Forum {}),
        entry("news", "News", Route::News {}),
        entry("blog", "Blog", Route::Blog {}),
        entry("wiki", "Wiki", Route::Wiki {}),
    ];
    if ctx.signed_in {
        community.push(entry("feed", "Feed", Route::Feed {}));
        community.push(entry("polls", "Polls", Route::Polls {}));
        if ctx.relay_on {
            community.push(entry("chat", "Chat", Route::TeamChat {}));
        }
        community.push(entry("community", "Community hub", Route::Community {}));
    }

    let compete = vec![
        entry("tournaments", "Tournaments", Route::Tournaments {}),
        entry("events", "Events", Route::Events {}),
        entry("scrims", "Scrims", Route::Scrims {}),
        entry("leaderboards", "Leaderboards", Route::Leaderboards {}),
        entry("stats", "Stats", Route::Stats {}),
    ];

    let mut strategy = Vec::new();
    if ctx.strategies_enabled {
        strategy.push(entry("strategy", "Planner", Route::StrategyBrowse {}));
        if ctx.signed_in {
            strategy.push(entry("strategy_my", "My strategies", Route::StrategyMy {}));
        }
        strategy.push(entry("strategy_heroes", "Heroes", Route::StrategyHeroes {}));
        strategy.push(entry("strategy_meta", "Meta", Route::StrategyMeta {}));
    }
    // Patch Notes is one public page. It stays when Strategies is off.
    strategy.push(entry("patch_notes", "Patch Notes", Route::PatchNotes {}));

    [
        (NavGroupId::Community, community),
        (NavGroupId::Compete, compete),
        (NavGroupId::Strategy, strategy),
    ]
    .into_iter()
    .filter(|(_, entries)| !entries.is_empty())
    .map(|(id, entries)| NavGroup { id, entries })
    .collect()
}

/// Account links for a signed-in person, in menu order. Sign out is a button
/// and Admin has its own gate, so neither is in this list.
pub(crate) fn account_entries(ctx: NavContext) -> Vec<NavEntry> {
    if !ctx.signed_in {
        return Vec::new();
    }
    let mut entries = vec![entry("profile", "Profile", Route::ProfileSettings {})];
    if ctx.relay_on {
        entries.push(entry("dm", "Direct messages", Route::DmInbox {}));
    }
    entries.push(entry(
        "identity",
        crate::pages::ACCOUNT_KEYS,
        Route::IdentitySettings {},
    ));
    entries.push(entry("reports", "My reports", Route::MyReports {}));
    entries.push(entry("link", "Link a device", Route::LinkDevice {}));
    entries
}

/// Account links for a visitor who is not signed in.
pub(crate) fn guest_entries() -> Vec<NavEntry> {
    vec![
        entry("apply", "Apply", Route::Apply {}),
        entry("login", "Log in", Route::Login {}),
    ]
}

/// The Admin entry is for people the admin section lets in, and nobody else.
pub(crate) fn shows_admin_entry(ctx: NavContext) -> bool {
    ctx.signed_in && ctx.admin_access
}

/// The admin sub-nav, in the order of the old admin sidebar. Seasons, Relay,
/// Audit log and Settings are for full admins, as before.
pub(crate) fn admin_entries(ctx: NavContext) -> Vec<NavEntry> {
    if !shows_admin_entry(ctx) {
        return Vec::new();
    }
    let mut entries = vec![
        entry("admin", "Dashboard", Route::AdminDashboard {}),
        entry("admin_members", "Members", Route::AdminMembers {}),
        entry("admin_teams", "Teams", Route::AdminTeams {}),
        entry(
            "admin_applications",
            "Applications",
            Route::AdminApplications {},
        ),
        entry("admin_forum", "Forum", Route::AdminForum {}),
        // Moderation is OfficerUser-gated on the server, so every officer sees it.
        entry("admin_moderation", "Moderation", Route::AdminModeration {}),
        entry("admin_games", "Games", Route::AdminGames {}),
        entry(
            "admin_announcements",
            "Announcements",
            Route::AdminAnnouncements {},
        ),
        entry("admin_articles", "Articles", Route::AdminArticles {}),
        entry(
            "admin_patch_notes",
            "Patch Notes",
            Route::AdminPatchNotes {},
        ),
        entry("admin_schedule", "Schedule", Route::AdminSchedule {}),
        entry("admin_matches", "Matches", Route::AdminMatches {}),
        entry(
            "admin_tournaments",
            "Tournaments",
            Route::AdminTournaments {},
        ),
        entry("admin_reports", "Reports", Route::AdminReports {}),
        entry("admin_hero_icons", "Hero icons", Route::AdminHeroIcons {}),
    ];
    if ctx.is_admin {
        entries.push(entry("admin_seasons", "Seasons", Route::AdminSeasons {}));
        entries.push(entry("admin_relay", "Relay", Route::AdminRelay {}));
        entries.push(entry(
            "admin_audit_log",
            "Audit log",
            Route::AdminAuditLog {},
        ));
        entries.push(entry("admin_settings", "Settings", Route::AdminSettings {}));
    }
    entries
}

/// The short Admin list in the phone drawer. The sub-nav on admin pages has
/// the rest.
pub(crate) fn drawer_admin_entries(ctx: NavContext) -> Vec<NavEntry> {
    admin_entries(ctx)
        .into_iter()
        .filter(|entry| {
            matches!(
                entry.id,
                "admin" | "admin_members" | "admin_applications" | "admin_settings"
            )
        })
        .collect()
}

/// Which part of the site a route belongs to. Drives the current state of the
/// top bar groups, the phone tabs and the Admin entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    Home,
    Group(NavGroupId),
    Account,
    Admin,
    /// First-run setup and the not-found page sit outside every section.
    None,
}

/// Every route is placed by hand. The match has no wildcard, so a new route
/// does not compile until it has a section.
pub(crate) fn section_of(route: &Route) -> Section {
    use NavGroupId::{Community, Compete, Strategy};
    match route {
        Route::Home {} => Section::Home,
        Route::Members {}
        | Route::MemberProfile { .. }
        | Route::TeamPage { .. }
        | Route::News {}
        | Route::TeamChat {}
        | Route::Community {}
        | Route::Feed {}
        | Route::Polls {}
        | Route::Blog {}
        | Route::BlogPost { .. }
        | Route::Wiki {}
        | Route::WikiPage { .. }
        | Route::Forum {}
        | Route::ForumBoardPage { .. }
        | Route::ForumThread { .. } => Section::Group(Community),
        Route::MatchDetail { .. }
        | Route::Tournaments {}
        | Route::Tournament { .. }
        | Route::Scrims {}
        | Route::Events {}
        | Route::Leaderboards {}
        | Route::Stats {}
        | Route::StatsTokens {}
        | Route::StatsMember { .. }
        | Route::StatsMatch { .. } => Section::Group(Compete),
        Route::PatchNotes {}
        | Route::StrategyBrowse {}
        | Route::StrategyMy {}
        | Route::StrategyHeroes {}
        | Route::StrategyMeta {}
        | Route::StrategyPatchNotes {}
        | Route::StrategyEditorNew {}
        | Route::StrategyEditor { .. } => Section::Group(Strategy),
        Route::Login {}
        | Route::LinkDevice {}
        | Route::Apply {}
        | Route::IdentitySettings {}
        | Route::ProfileSettings {}
        | Route::MyReports {}
        | Route::DmInbox {}
        | Route::DmThread { .. } => Section::Account,
        Route::AdminDashboard {}
        | Route::AdminMembers {}
        | Route::AdminMemberMatches { .. }
        | Route::AdminGames {}
        | Route::AdminTeams {}
        | Route::AdminSchedule {}
        | Route::AdminApplications {}
        | Route::AdminMatches {}
        | Route::AdminSeasons {}
        | Route::AdminTournaments {}
        | Route::AdminAnnouncements {}
        | Route::AdminArticles {}
        | Route::AdminPatchNotes {}
        | Route::AdminForum {}
        | Route::AdminAuditLog {}
        | Route::AdminModeration {}
        | Route::AdminReports {}
        | Route::AdminHeroIcons {}
        | Route::AdminRelay {}
        | Route::AdminSettings {} => Section::Admin,
        Route::Setup {} | Route::NotFound { .. } => Section::None,
    }
}

/// True when `entry` is the page being shown, or the list page a detail page
/// hangs under (a member's matches under Admin Members, and so on).
pub(crate) fn is_current(entry: &Route, current: &Route) -> bool {
    if entry == current {
        return true;
    }
    matches!(
        (entry, current),
        (Route::AdminMembers {}, Route::AdminMemberMatches { .. })
            | (Route::Members {}, Route::MemberProfile { .. })
            | (Route::Blog {}, Route::BlogPost { .. })
            | (Route::Wiki {}, Route::WikiPage { .. })
            | (
                Route::Forum {},
                Route::ForumBoardPage { .. } | Route::ForumThread { .. }
            )
            | (Route::Tournaments {}, Route::Tournament { .. })
            | (
                Route::Stats {},
                Route::StatsTokens {} | Route::StatsMember { .. } | Route::StatsMatch { .. }
            )
            | (Route::PatchNotes {}, Route::StrategyPatchNotes {})
            | (
                Route::StrategyBrowse {},
                Route::StrategyEditorNew {} | Route::StrategyEditor { .. }
            )
            | (Route::DmInbox {}, Route::DmThread { .. })
    )
}

/// Direct messages, team chat and the strategy editor are full-height tools.
/// The shell leaves the footer off so the tool can use the height.
pub(crate) fn is_tool_route(route: &Route) -> bool {
    matches!(
        route,
        Route::DmInbox {}
            | Route::DmThread { .. }
            | Route::TeamChat {}
            | Route::StrategyEditorNew {}
            | Route::StrategyEditor { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::prelude::Routable;
    use scuffed_types::NAV_CATALOG;

    const GUEST: NavContext = NavContext {
        signed_in: false,
        admin_access: false,
        is_admin: false,
        strategies_enabled: true,
        relay_on: true,
    };
    const MEMBER: NavContext = NavContext {
        signed_in: true,
        ..GUEST
    };
    const OFFICER: NavContext = NavContext {
        admin_access: true,
        ..MEMBER
    };
    const ADMIN: NavContext = NavContext {
        is_admin: true,
        ..OFFICER
    };

    fn ids(entries: &[NavEntry]) -> Vec<&'static str> {
        entries.iter().map(|entry| entry.id).collect()
    }

    fn group(ctx: NavContext, id: NavGroupId) -> Vec<&'static str> {
        nav_groups(ctx)
            .into_iter()
            .find(|group| group.id == id)
            .map(|group| ids(&group.entries))
            .unwrap_or_default()
    }

    #[test]
    fn guest_groups_match_the_public_frames_plus_wiki() {
        assert_eq!(
            group(GUEST, NavGroupId::Community),
            ["members", "forum", "news", "blog", "wiki"]
        );
        assert_eq!(
            group(GUEST, NavGroupId::Compete),
            ["tournaments", "events", "scrims", "leaderboards", "stats"]
        );
        assert_eq!(
            group(GUEST, NavGroupId::Strategy),
            [
                "strategy",
                "strategy_heroes",
                "strategy_meta",
                "patch_notes"
            ]
        );
    }

    #[test]
    fn signed_in_groups_add_the_member_pages() {
        assert_eq!(
            group(MEMBER, NavGroupId::Community),
            [
                "members",
                "forum",
                "news",
                "blog",
                "wiki",
                "feed",
                "polls",
                "chat",
                "community"
            ]
        );
        assert_eq!(
            group(MEMBER, NavGroupId::Strategy),
            [
                "strategy",
                "strategy_my",
                "strategy_heroes",
                "strategy_meta",
                "patch_notes"
            ]
        );
    }

    #[test]
    fn the_bar_is_always_the_same_three_groups_in_order() {
        for ctx in [GUEST, MEMBER, OFFICER, ADMIN] {
            let order: Vec<NavGroupId> = nav_groups(ctx).iter().map(|g| g.id).collect();
            assert_eq!(order, NavGroupId::ALL);
        }
    }

    #[test]
    fn chat_lives_in_community_and_dms_live_in_the_account_menu() {
        assert!(group(MEMBER, NavGroupId::Community).contains(&"chat"));
        assert!(!ids(&account_entries(MEMBER)).contains(&"chat"));
        assert!(ids(&account_entries(MEMBER)).contains(&"dm"));
        for id in NavGroupId::ALL {
            assert!(!group(MEMBER, id).contains(&"dm"), "{id:?}");
        }
    }

    #[test]
    fn chat_and_dms_leave_when_the_relay_is_off() {
        let off = NavContext {
            relay_on: false,
            ..MEMBER
        };
        assert!(!group(off, NavGroupId::Community).contains(&"chat"));
        assert!(!ids(&account_entries(off)).contains(&"dm"));
        assert!(group(off, NavGroupId::Community).contains(&"feed"));
        assert!(ids(&account_entries(off)).contains(&"reports"));
    }

    #[test]
    fn strategies_off_keeps_only_patch_notes_in_the_strategy_group() {
        let off = NavContext {
            strategies_enabled: false,
            ..MEMBER
        };
        assert_eq!(group(off, NavGroupId::Strategy), ["patch_notes"]);
        let groups = nav_groups(off);
        let strategy = groups
            .iter()
            .find(|g| g.id == NavGroupId::Strategy)
            .expect("strategy group");
        assert_eq!(strategy.landing(), Some(Route::PatchNotes {}));
        assert_eq!(
            group(off, NavGroupId::Compete),
            group(MEMBER, NavGroupId::Compete),
            "the flag does not touch Stats or Leaderboards"
        );
    }

    #[test]
    fn phone_tabs_land_on_each_groups_first_page() {
        let landings: Vec<Option<Route>> =
            nav_groups(GUEST).iter().map(NavGroup::landing).collect();
        assert_eq!(
            landings,
            [
                Some(Route::Members {}),
                Some(Route::Tournaments {}),
                Some(Route::StrategyBrowse {})
            ]
        );
    }

    #[test]
    fn patch_notes_entry_is_the_public_page() {
        let entry = nav_groups(GUEST)
            .into_iter()
            .flat_map(|group| group.entries)
            .find(|entry| entry.id == "patch_notes")
            .expect("patch notes entry");
        assert_eq!(entry.route, Route::PatchNotes {});
    }

    #[test]
    fn every_catalog_page_has_a_place_in_a_group() {
        let all: Vec<&str> = nav_groups(ADMIN)
            .into_iter()
            .flat_map(|group| group.entries)
            .map(|entry| entry.id)
            .collect();
        for item in NAV_CATALOG {
            assert!(
                all.contains(&item.id),
                "catalog page `{}` is missing from the grouped nav",
                item.id
            );
        }
    }

    #[test]
    fn entry_ids_are_unique_so_element_ids_built_from_them_are_too() {
        let mut all: Vec<&str> = nav_groups(ADMIN)
            .into_iter()
            .flat_map(|group| group.entries)
            .map(|entry| entry.id)
            .collect();
        all.extend(ids(&account_entries(ADMIN)));
        all.extend(ids(&guest_entries()));
        all.extend(ids(&admin_entries(ADMIN)));
        let count = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), count);
    }

    #[test]
    fn admin_entry_is_only_for_people_the_admin_section_lets_in() {
        assert!(!shows_admin_entry(GUEST));
        assert!(!shows_admin_entry(MEMBER));
        assert!(shows_admin_entry(OFFICER));
        assert!(shows_admin_entry(ADMIN));
        // A stale role flag without a session shows nothing.
        assert!(!shows_admin_entry(NavContext {
            signed_in: false,
            ..ADMIN
        }));
        assert!(admin_entries(MEMBER).is_empty());
        assert!(admin_entries(GUEST).is_empty());
        assert!(drawer_admin_entries(MEMBER).is_empty());
    }

    #[test]
    fn officers_do_not_see_the_admin_only_pages() {
        let officer = ids(&admin_entries(OFFICER));
        for id in [
            "admin_seasons",
            "admin_relay",
            "admin_audit_log",
            "admin_settings",
        ] {
            assert!(!officer.contains(&id), "{id}");
            assert!(ids(&admin_entries(ADMIN)).contains(&id), "{id}");
        }
        assert!(officer.contains(&"admin_moderation"));
        assert_eq!(
            ids(&drawer_admin_entries(OFFICER)),
            ["admin", "admin_members", "admin_applications"]
        );
        assert_eq!(
            ids(&drawer_admin_entries(ADMIN)),
            [
                "admin",
                "admin_members",
                "admin_applications",
                "admin_settings"
            ]
        );
    }

    /// Static routes a signed-in admin cannot reach from the nav, with the
    /// reason each one is allowed to be missing.
    fn not_linked_on_purpose(route: &Route) -> bool {
        matches!(
            route,
            // First-run only. A finished setup redirects to Log in.
            Route::Setup {}
            // Guest pages. A signed-in person has no Apply or Log in.
            | Route::Login {}
            | Route::Apply {}
            // Same page as Patch Notes. The nav links the public one.
            | Route::StrategyPatchNotes {}
            // Opened from the New strategy buttons on Planner and My strategies.
            | Route::StrategyEditorNew {}
            // Opened from the Stats page and the device link page.
            | Route::StatsTokens {}
        )
    }

    #[test]
    fn every_static_route_is_reachable_from_the_nav() {
        let mut linked: Vec<Route> = vec![Route::Home {}];
        linked.extend(
            nav_groups(ADMIN)
                .into_iter()
                .flat_map(|group| group.entries)
                .map(|entry| entry.route),
        );
        linked.extend(account_entries(ADMIN).into_iter().map(|e| e.route));
        linked.extend(admin_entries(ADMIN).into_iter().map(|e| e.route));

        let routes = Route::static_routes();
        assert!(routes.len() > 40, "static routes: {}", routes.len());
        for route in routes {
            if not_linked_on_purpose(&route) {
                continue;
            }
            assert!(
                linked.contains(&route),
                "{route} is not reachable from the grouped nav, the account menu or the admin sub-nav"
            );
        }

        let guest: Vec<Route> = guest_entries().into_iter().map(|e| e.route).collect();
        assert!(guest.contains(&Route::Login {}));
        assert!(guest.contains(&Route::Apply {}));
    }

    #[test]
    fn sections_follow_the_groups() {
        for group in nav_groups(ADMIN) {
            for entry in &group.entries {
                assert_eq!(
                    section_of(&entry.route),
                    Section::Group(group.id),
                    "{}",
                    entry.id
                );
            }
        }
        for entry in admin_entries(ADMIN) {
            assert_eq!(section_of(&entry.route), Section::Admin, "{}", entry.id);
        }
        for entry in account_entries(ADMIN) {
            assert_eq!(section_of(&entry.route), Section::Account, "{}", entry.id);
        }
        assert_eq!(section_of(&Route::Home {}), Section::Home);
        assert_eq!(
            section_of(&Route::NotFound { segments: vec![] }),
            Section::None
        );
    }

    #[test]
    fn detail_pages_keep_their_list_entry_current() {
        assert!(is_current(&Route::Members {}, &Route::Members {}));
        assert!(is_current(
            &Route::AdminMembers {},
            &Route::AdminMemberMatches { id: "m1".into() }
        ));
        assert!(is_current(
            &Route::PatchNotes {},
            &Route::StrategyPatchNotes {}
        ));
        assert!(!is_current(&Route::Members {}, &Route::Forum {}));
        assert!(!is_current(
            &Route::AdminDashboard {},
            &Route::AdminMembers {}
        ));
    }

    #[test]
    fn tool_routes_are_the_full_height_pages() {
        assert!(is_tool_route(&Route::TeamChat {}));
        assert!(is_tool_route(&Route::DmInbox {}));
        assert!(is_tool_route(&Route::StrategyEditorNew {}));
        assert!(!is_tool_route(&Route::Home {}));
        assert!(!is_tool_route(&Route::StrategyBrowse {}));
    }
}
