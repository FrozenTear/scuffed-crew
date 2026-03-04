# Dioxus Frontend Cleanup — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Eliminate type duplication, fetch boilerplate, signal explosion, and CSS repetition across the Dioxus admin frontend.

**Architecture:** Four incremental, independently-shippable steps that each reduce one category of duplication. Each step builds on the previous but can be committed and verified alone. The cleanup moves API request/response types into the shared `scuffed-types` crate, extracts a `use_api` hook to replace 23 boilerplate patterns, introduces a `ModalController<T>` to cut modal signal counts in half, and consolidates CSS into a module system.

**Tech Stack:** Rust, Dioxus 0.7, serde, chrono, scuffed-types crate, scuffed-api-client crate

---

## Step 1: Consolidate Types

Move all request DTOs from admin pages into `crates/types/src/api/`, and replace local response types with imports from `scuffed_types::org`. This step has zero logic changes — only moving struct definitions and updating imports.

### Background

Currently, 12 admin pages define 43 local structs. Many are duplicated across pages (e.g., `Game` appears in 5 files, `Member` in 6 files). Response types are simplified versions of org types (using `String` instead of `DateTime<Utc>`, `String` instead of enums). Request DTOs (Serialize-only) exist nowhere in the shared crate.

**Key decision:** Response types in org/ use proper types (`OrgRole` enum, `DateTime<Utc>`). The local page types use `String` for everything. Since the org types already derive Serialize + Deserialize with appropriate serde attributes, and the API returns all fields, pages can deserialize into the full org types directly. Display formatting changes are minor (OrgRole implements Display, DateTime<Utc> implements Display).

### Task 1.1: Create `api/` module structure in types crate

**Files:**
- Move: `crates/types/src/api.rs` → `crates/types/src/api/mod.rs`
- Create: `crates/types/src/api/members.rs`
- Create: `crates/types/src/api/tournaments.rs`
- Create: `crates/types/src/api/teams.rs`
- Create: `crates/types/src/api/games.rs`
- Create: `crates/types/src/api/announcements.rs`
- Create: `crates/types/src/api/events.rs`
- Create: `crates/types/src/api/moderation.rs`
- Create: `crates/types/src/api/applications.rs`
- Create: `crates/types/src/api/matches.rs`
- Create: `crates/types/src/api/settings.rs`

**Step 1: Convert api.rs to api/mod.rs**

Move existing `crates/types/src/api.rs` to `crates/types/src/api/mod.rs` and add submodule declarations. Keep existing types (ApiError, ApiSuccess, PaginatedResponse) in mod.rs.

```rust
// crates/types/src/api/mod.rs
pub mod members;
pub mod tournaments;
pub mod teams;
pub mod games;
pub mod announcements;
pub mod events;
pub mod moderation;
pub mod applications;
pub mod matches;
pub mod settings;

pub use members::*;
pub use tournaments::*;
pub use teams::*;
pub use games::*;
pub use announcements::*;
pub use events::*;
pub use moderation::*;
pub use applications::*;
pub use matches::*;
pub use settings::*;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiSuccess<T> {
    pub data: T,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}
```

**Step 2: Create request DTO files**

Each file contains only Serialize request types — the response types stay in `org/`.

```rust
// crates/types/src/api/members.rs
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ChangeRoleRequest {
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToggleActiveRequest {
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateGameAccountRequest {
    pub game_id: String,
    pub account_name: String,
    pub account_id: Option<String>,
}
```

```rust
// crates/types/src/api/tournaments.rs
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CreateTournamentRequest {
    pub name: String,
    pub format: String,
    pub game_id: Option<String>,
    pub max_teams: Option<u32>,
    pub starts_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusChangeRequest {
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddParticipantRequest {
    pub team_id: Option<String>,
    pub external_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchReportRequest {
    pub score_a: u32,
    pub score_b: u32,
    pub winner_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_codes: Option<Vec<String>>,
}
```

```rust
// crates/types/src/api/teams.rs
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CreateTeamRequest {
    pub name: String,
    pub game_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub division: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddRosterMemberRequest {
    pub member_id: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateRosterRoleRequest {
    pub role: String,
}
```

```rust
// crates/types/src/api/games.rs
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CreateGameRequest {
    pub name: String,
    pub abbreviation: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateGameRequest {
    pub name: Option<String>,
    pub abbreviation: Option<Option<String>>,
}
```

```rust
// crates/types/src/api/announcements.rs
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CreateAnnouncementRequest {
    pub title: String,
    pub content: String,
    pub is_pinned: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateAnnouncementRequest {
    pub title: String,
    pub content: String,
    pub is_pinned: bool,
}
```

```rust
// crates/types/src/api/events.rs
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CreateEventRequest {
    pub title: String,
    pub day_of_week: u8,
    pub time: String,
    pub timezone: String,
    pub is_recurring: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttendanceEntry {
    pub member_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchAttendanceRequest {
    pub date: String,
    pub entries: Vec<AttendanceEntry>,
}
```

```rust
// crates/types/src/api/moderation.rs
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CreateModerationRequest {
    pub member_id: String,
    pub action_type: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}
```

```rust
// crates/types/src/api/applications.rs
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct PatchApplicationRequest {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer_notes: Option<String>,
}
```

```rust
// crates/types/src/api/matches.rs
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MatchPayload {
    pub team_id: String,
    pub opponent: String,
    pub score_us: u32,
    pub score_them: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_name: Option<String>,
    pub match_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub played_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}
```

```rust
// crates/types/src/api/settings.rs
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct UpdateSettingsRequest {
    pub org_name: Option<String>,
    pub description: Option<String>,
    pub welcome_message: Option<String>,
}
```

**Step 3: Verify types crate compiles**

Run: `cargo check --package scuffed-types`
Expected: Success with no errors.

**Step 4: Commit**

```bash
git add crates/types/src/api/
git commit -m "feat(types): add API request DTOs to shared types crate"
```

### Task 1.2: Migrate admin pages to use shared types

This is the largest task — update all 12 admin pages to import types instead of defining them locally. Do this page by page, verifying compilation after each.

**Important:** The pages currently use simplified response types with `String` fields (e.g., `org_role: String`, `created_at: String`). The org types use proper types (`OrgRole`, `DateTime<Utc>`). Pages can use the org types directly since:
- The API returns all fields → all struct fields get populated
- `OrgRole` and other enums implement `Display` → `{member.org_role}` still works in RSX
- `DateTime<Utc>` implements `Display` → `{member.joined_at}` shows the ISO 8601 string

Where pages do string comparisons like `org_role == "admin"`, change to `org_role == OrgRole::Admin` (or use the enum's `is_at_least()` method).

Where pages display formatted dates, use `member.joined_at.format("%Y-%m-%d").to_string()` or `member.joined_at.to_string()`.

**Special case — Dashboard:** The dashboard uses minimal types with only `id` fields for counting. Keep these local since they intentionally don't need full types, and serde will ignore extra JSON fields.

**Step 1: Migrate `admin/games.rs` (simplest page, 3 local types)**

Delete the local `Game`, `CreateGame`, `UpdateGame` structs. Replace with:

```rust
use scuffed_types::{Game, api::{CreateGameRequest, UpdateGameRequest}};
```

Update usages:
- `CreateGame { name, abbreviation: abbr }` → `CreateGameRequest { name, abbreviation: abbr }`
- `UpdateGame { name: Some(name), ... }` → `UpdateGameRequest { name: Some(name), ... }`
- `Game` type annotations stay the same (same name)

Note: The local `Game` has `{ id, name, abbreviation }` while org `Game` may have more fields — verify the org type has at minimum these fields. If the org type is missing, keep the local type or add the missing fields to org.

Run: `cargo check --package scuffed-app`

**Step 2: Migrate `admin/announcements.rs` (3 local types)**

Delete local `Announcement`, `CreateAnnouncement`, `UpdateAnnouncement`. Replace with:

```rust
use scuffed_types::{Announcement, api::{CreateAnnouncementRequest, UpdateAnnouncementRequest}};
```

Adjust: `created_at` may change from `String` to `DateTime<Utc>`. Update display in RSX if needed.

Run: `cargo check --package scuffed-app`

**Step 3: Migrate `admin/applications.rs` (2 local types)**

Delete local `Application`, `PatchApplication`. Replace with:

```rust
use scuffed_types::{Application, api::PatchApplicationRequest};
```

Run: `cargo check --package scuffed-app`

**Step 4: Migrate `admin/moderation.rs` (4 local types)**

Delete local `ModerationAction`, `ModerationResponse`, `Member`, `CreateModeration`. Replace with:

```rust
use scuffed_types::{Member, ModerationAction, PaginatedResponse, api::CreateModerationRequest};
```

Replace `ModerationResponse` usage with `PaginatedResponse<ModerationAction>` if field names match. If the server returns different field names, create a `ModerationListResponse` in api/moderation.rs instead.

Run: `cargo check --package scuffed-app`

**Step 5: Migrate `admin/members.rs` (most complex, 8 local types)**

Delete local `Member`, `GameAccount`, `Game`, `ModerationAction`, `AttendanceStats`, `ChangeRole`, `ToggleActive`, `CreateGameAccount`. Replace with:

```rust
use scuffed_types::{
    Member, GameAccount, Game, ModerationAction,
    api::{ChangeRoleRequest, ToggleActiveRequest, CreateGameAccountRequest},
};
```

For `AttendanceStats` — check if it exists in org/. If not, create it in `crates/types/src/org/member.rs` or keep local.

Update enum comparisons:
- `member.org_role == "admin"` → `member.org_role.to_string() == "admin"` or add match arms

Run: `cargo check --package scuffed-app`

**Step 6: Migrate `admin/teams.rs` (7 local types)**

Delete local `Team`, `Game`, `RosterEntry`, `Member`, `CreateTeam`, `AddRosterMember`, `UpdateRosterRole`. Replace with:

```rust
use scuffed_types::{Team, Game, RosterEntry, Member, api::{CreateTeamRequest, AddRosterMemberRequest, UpdateRosterRoleRequest}};
```

Run: `cargo check --package scuffed-app`

**Step 7: Migrate `admin/schedule.rs` (6 local types)**

Delete local `Event`, `Team`, `Member`, `CreateEvent`, `AttendancePayload`, `AttendanceEntry`. Replace with:

```rust
use scuffed_types::{Event, Team, Member, api::{CreateEventRequest, BatchAttendanceRequest, AttendanceEntry}};
```

Run: `cargo check --package scuffed-app`

**Step 8: Migrate `admin/matches.rs` (3 local types)**

Delete local `Team`, `MatchResult`, `MatchPayload`. Replace with:

```rust
use scuffed_types::{Team, MatchResult, api::MatchPayload};
```

Run: `cargo check --package scuffed-app`

**Step 9: Migrate `admin/tournaments.rs` (11 local types)**

This is the most complex page. Delete local `Tournament`, `Game`, `BracketData`, `Participant`, `Round`, `BracketMatch`, `Member`, `CreateTournament`, `StatusChange`, `AddParticipant`, `MatchReport`. Replace with:

```rust
use scuffed_types::{
    Tournament, Game, TournamentBracket, TournamentParticipant,
    TournamentRound, TournamentMatch, Member,
    api::{CreateTournamentRequest, StatusChangeRequest, AddParticipantRequest, MatchReportRequest},
};
```

Note: The local types use different names from org types:
- `BracketData` → `TournamentBracket`
- `Participant` → `TournamentParticipant`
- `Round` → `TournamentRound`
- `BracketMatch` → `TournamentMatch`

Update all usages to the new names. This includes type annotations in signals, function parameters, and RSX.

Run: `cargo check --package scuffed-app`

**Step 10: Migrate `admin/settings.rs` and `admin/audit_log.rs`**

Settings:
```rust
use scuffed_types::{SiteSettings, api::UpdateSettingsRequest};
```

Audit log:
```rust
use scuffed_types::{AuditLogEntry, PaginatedResponse};
```

Run: `cargo check --package scuffed-app`

**Step 11: Migrate remaining pages (public + strategy)**

Check `pages/apply.rs`, `pages/tournaments.rs`, `pages/tournament.rs`, `pages/members.rs`, `pages/member_profile.rs`, and strategy pages for any local types that should use shared imports.

Run: `cargo check --package scuffed-app`

**Step 12: Final verification**

Run: `cargo check --package scuffed-app --features web`
Run: `cargo check --package scuffed-app --features desktop --no-default-features`

Both should compile with no errors.

**Step 13: Commit**

```bash
git add crates/app/src/pages/ crates/types/
git commit -m "refactor: replace local page types with shared scuffed_types imports

Eliminates 43 local struct definitions across 12 admin pages.
Response types use org module, request DTOs use new api module."
```

---

## Step 2: Custom `use_api` Hook

Replace 23 `use_resource` + refresh signal patterns with a single `use_api<T>` hook.

### Task 2.1: Create hooks module with `use_api`

**Files:**
- Create: `crates/app/src/hooks/mod.rs`
- Create: `crates/app/src/hooks/api.rs`
- Modify: `crates/app/src/main.rs` (add `mod hooks;`)

**Step 1: Create the hook**

```rust
// crates/app/src/hooks/mod.rs
mod api;
pub use api::*;
```

```rust
// crates/app/src/hooks/api.rs
use dioxus::prelude::*;
use serde::de::DeserializeOwned;
use scuffed_api_client::ApiClient;

/// A resource that fetches data from an API endpoint with built-in refresh support.
#[derive(Clone, Copy)]
pub struct ApiResource<T: 'static> {
    pub data: Resource<Option<T>>,
    pub refresh: Signal<u64>,
}

impl<T: 'static> ApiResource<T> {
    /// Trigger a reload of the resource.
    pub fn reload(&mut self) {
        self.refresh += 1;
    }

    /// Read the current data (convenience wrapper).
    pub fn read(&self) -> ReadableRef<Signal<Option<Option<T>>>> {
        self.data.read()
    }
}

/// Fetch data from a static API endpoint with automatic refresh support.
///
/// Usage:
/// ```rust
/// let members = use_api::<Vec<Member>>("/api/members");
/// // later: members.reload();
/// ```
pub fn use_api<T: DeserializeOwned + 'static>(url: &'static str) -> ApiResource<T> {
    let refresh = use_signal(|| 0u64);
    let data = use_resource(move || async move {
        let _ = refresh();
        ApiClient::web().fetch::<T>(url).await.ok()
    });
    ApiResource { data, refresh }
}

/// Fetch data from a dynamic API endpoint with automatic refresh support.
///
/// Usage:
/// ```rust
/// let roster = use_api_with::<Vec<RosterEntry>, _>(move || format!("/api/teams/{}/roster", team_id()));
/// ```
pub fn use_api_with<T: DeserializeOwned + 'static>(
    url: impl Fn() -> String + 'static,
) -> ApiResource<T> {
    let refresh = use_signal(|| 0u64);
    let data = use_resource(move || {
        let url = url();
        async move {
            let _ = refresh();
            ApiClient::web().fetch::<T>(&url).await.ok()
        }
    });
    ApiResource { data, refresh }
}
```

**Step 2: Register module in main.rs**

Add `mod hooks;` to `crates/app/src/main.rs` after the existing module declarations.

**Step 3: Verify compilation**

Run: `cargo check --package scuffed-app`
Expected: Success (hook is defined but not yet used).

**Step 4: Commit**

```bash
git add crates/app/src/hooks/ crates/app/src/main.rs
git commit -m "feat(app): add use_api and use_api_with hooks for API fetching"
```

### Task 2.2: Migrate pages to use `use_api`

Replace `use_resource` + refresh signal patterns in all admin pages. Work through pages from simplest to most complex.

**Pattern replacement:**

Before:
```rust
let mut refresh = use_signal(|| 0u64);
let games = use_resource(move || async move {
    let _ = refresh();
    ApiClient::web().fetch::<Vec<Game>>("/api/games").await.ok()
});
// later: refresh += 1;
```

After:
```rust
let mut games = use_api::<Vec<Game>>("/api/games");
// later: games.reload();
```

For dynamic URLs:
```rust
// Before:
let mut refresh = use_signal(|| 0u64);
let matches = use_resource(move || {
    let team_id = selected_team().clone();
    async move {
        let _ = refresh();
        if let Some(id) = team_id {
            ApiClient::web().fetch::<Vec<MatchResult>>(&format!("/api/teams/{id}/matches")).await.ok()
        } else {
            None
        }
    }
});

// After:
let mut matches = use_api_with::<Vec<MatchResult>, _>(move || {
    match selected_team() {
        Some(id) => format!("/api/teams/{id}/matches"),
        None => String::new(), // empty URL returns None
    }
});
```

**Step 1: Migrate simple pages (1 resource each)**

Files to update (in order):
- `admin/games.rs` — 1 resource: `games`
- `admin/announcements.rs` — 1 resource: `announcements`
- `admin/applications.rs` — 1 resource: `applications`
- `admin/moderation.rs` — 1 resource: `actions`
- `admin/audit_log.rs` — 1 resource (may use pagination differently — check before migrating)
- `pages/apply.rs` — 1 resource: `my_app`
- `strategy/my_strategies.rs` — 1 resource: `strategies`

For each: add `use crate::hooks::use_api;`, delete the `refresh` signal and `use_resource` block, replace with `use_api`, change `refresh += 1` to `.reload()`.

Run after each: `cargo check --package scuffed-app`

**Step 2: Migrate multi-resource pages**

Pages with multiple resources sharing one refresh signal:
- `admin/members.rs` — 2 resources (members, games) + 1 sub-resource (game accounts)
- `admin/teams.rs` — 3 resources (teams, games, members) + 1 sub-resource (roster)
- `admin/schedule.rs` — 3 resources (events, teams, members)
- `admin/matches.rs` — 1 dynamic resource (depends on selected team)
- `admin/tournaments.rs` — 3 resources (tournaments, games, members) + 1 sub-resource (detail)

For pages with shared refresh signals (where one `refresh += 1` reloads multiple resources): each `use_api` call gets its own internal refresh, so `.reload()` needs to be called on each. Create a small helper closure:

```rust
let mut tournaments = use_api::<Vec<Tournament>>("/api/tournaments");
let mut games = use_api::<Vec<Game>>("/api/games");
let mut members = use_api::<Vec<Member>>("/api/members");

// After mutations, reload all:
let reload_all = move || {
    tournaments.reload();
    games.reload();
    members.reload();
};
```

For sub-resources that load conditionally (roster, game accounts, detail): use `use_api_with` with a dynamic URL that changes when the target changes.

Run after each page: `cargo check --package scuffed-app`

**Step 3: Final verification and commit**

Run: `cargo check --package scuffed-app --features web`

```bash
git add crates/app/src/pages/ crates/app/src/hooks/
git commit -m "refactor: replace use_resource + refresh patterns with use_api hook

Replaces 23 use_resource + refresh signal pairs across 13 files."
```

---

## Step 3: Modal Controller Abstraction

Introduce `ModalController<T>` to replace the repetitive modal signal pattern (open + target + submitting = 3 signals per modal → 1 controller).

### Task 3.1: Create ModalController

**Files:**
- Create: `crates/app/src/hooks/modal.rs`
- Modify: `crates/app/src/hooks/mod.rs` (add module)

**Step 1: Implement ModalController**

```rust
// crates/app/src/hooks/modal.rs
use dioxus::prelude::*;

/// Controls the lifecycle of a modal that operates on a target of type T.
///
/// Replaces the common pattern of:
///   let mut open = use_signal(|| false);
///   let mut target: Signal<Option<T>> = use_signal(|| None);
///   let mut submitting = use_signal(|| false);
#[derive(Clone, Copy)]
pub struct ModalController<T: Clone + 'static> {
    pub open: Signal<bool>,
    pub target: Signal<Option<T>>,
    pub submitting: Signal<bool>,
}

impl<T: Clone + 'static> ModalController<T> {
    /// Create a new modal controller. Call this in your component body.
    pub fn new() -> Self {
        Self {
            open: use_signal(|| false),
            target: use_signal(|| None),
            submitting: use_signal(|| false),
        }
    }

    /// Open the modal targeting a specific item (for edit/delete).
    pub fn show(&mut self, item: T) {
        self.target.set(Some(item));
        self.open.set(true);
    }

    /// Open the modal with no target (for create).
    pub fn show_empty(&mut self) {
        self.target.set(None);
        self.open.set(true);
    }

    /// Close the modal and reset state.
    pub fn close(&mut self) {
        self.open.set(false);
        self.submitting.set(false);
    }

    /// Check if the modal is currently open.
    pub fn is_open(&self) -> bool {
        (self.open)()
    }

    /// Check if the modal is currently submitting.
    pub fn is_submitting(&self) -> bool {
        (self.submitting)()
    }

    /// Get the current target (if any).
    pub fn get_target(&self) -> Option<T> {
        (self.target)()
    }

    /// Mark as submitting.
    pub fn start_submit(&mut self) {
        self.submitting.set(true);
    }

    /// Mark as done submitting.
    pub fn end_submit(&mut self) {
        self.submitting.set(false);
    }
}
```

**Step 2: Export from hooks module**

Add to `crates/app/src/hooks/mod.rs`:

```rust
mod modal;
pub use modal::*;
```

**Step 3: Verify compilation**

Run: `cargo check --package scuffed-app`

**Step 4: Commit**

```bash
git add crates/app/src/hooks/modal.rs crates/app/src/hooks/mod.rs
git commit -m "feat(app): add ModalController abstraction for modal signal management"
```

### Task 3.2: Migrate admin pages to use ModalController

Work through pages from simplest to most complex. Each page typically has 2-3 modals (create/edit form, delete confirm, sometimes a detail view).

**Pattern replacement for FormModal usage:**

Before (games.rs):
```rust
let mut modal_open = use_signal(|| false);
let mut submitting = use_signal(|| false);
let mut editing_id: Signal<Option<String>> = use_signal(|| None);

// open:
editing_id.set(Some(game.id));
modal_open.set(true);

// close:
modal_open.set(false);

// submit:
submitting.set(true);
// ...
submitting.set(false);
modal_open.set(false);

// RSX:
FormModal { open: modal_open(), submitting: submitting(), ... }
```

After:
```rust
let mut modal = ModalController::<String>::new(); // T = editing ID type

// open for edit:
modal.show(game.id.clone());

// open for create:
modal.show_empty();

// close:
modal.close();

// submit:
modal.start_submit();
// ...
modal.end_submit();
modal.close();

// RSX:
FormModal { open: modal.is_open(), submitting: modal.is_submitting(), ... }
```

**Pattern replacement for ConfirmDialog:**

Before:
```rust
let mut delete_open = use_signal(|| false);
let mut delete_target: Signal<Option<Tournament>> = use_signal(|| None);

// open:
delete_target.set(Some(tournament.clone()));
delete_open.set(true);

// RSX:
ConfirmDialog { open: delete_open(), ... }
```

After:
```rust
let mut delete_modal = ModalController::<Tournament>::new();

// open:
delete_modal.show(tournament.clone());

// RSX:
ConfirmDialog { open: delete_modal.is_open(), ... }
```

**Step 1: Migrate simple pages**

In order of complexity:
1. `admin/games.rs` — 1 form modal → 1 ModalController<String> (saves 2 signals)
2. `admin/announcements.rs` — 1 form modal + 1 confirm → 2 ModalControllers (saves 4 signals)
3. `admin/applications.rs` — 1 confirm dialog → 1 ModalController (saves 1 signal)
4. `admin/moderation.rs` — 1 form modal + 1 confirm → 2 ModalControllers (saves 4 signals)
5. `admin/matches.rs` — 1 form modal → 1 ModalController (saves 2 signals)

Run after each: `cargo check --package scuffed-app`

**Step 2: Migrate complex pages**

6. `admin/schedule.rs` — 2 form modals + 1 confirm → 3 ModalControllers (saves 6 signals)
7. `admin/teams.rs` — 1 form modal + 1 confirm + 1 detail → 3 ModalControllers (saves 6+ signals)
8. `admin/members.rs` — 1 form modal + 2 confirms + 3 detail views → 6 ModalControllers (saves 12+ signals)
9. `admin/tournaments.rs` — 2 form modals + 2 confirms → 4 ModalControllers (saves 8+ signals)

For detail view modals (members page mod history, stats, accounts), ModalController<T> where T is the target member. The `data` and `loading` signals remain separate since they're view-specific.

Run after each: `cargo check --package scuffed-app`

**Step 3: Final verification and commit**

Run: `cargo check --package scuffed-app --features web`

```bash
git add crates/app/src/pages/admin/ crates/app/src/hooks/
git commit -m "refactor: replace modal signal patterns with ModalController

Cuts ~45 signals across 9 admin pages by consolidating
open/target/submitting into ModalController<T>."
```

---

## Step 4: CSS Module System

Reorganize CSS into `crates/app/src/styles/` modules. Extract shared patterns from `ADMIN_SHARED_CSS` and inline page CSS into reusable modules.

### Task 4.1: Create styles module structure

**Files:**
- Create: `crates/app/src/styles/mod.rs`
- Create: `crates/app/src/styles/common.rs`
- Create: `crates/app/src/styles/admin.rs`
- Create: `crates/app/src/styles/public.rs`
- Create: `crates/app/src/styles/strategy.rs`
- Modify: `crates/app/src/main.rs` (add `mod styles;`)

**Step 1: Create styles module with common CSS**

Extract cross-cutting patterns into `common.rs`:

```rust
// crates/app/src/styles/mod.rs
pub mod common;
pub mod admin;
pub mod public;
pub mod strategy;
```

```rust
// crates/app/src/styles/common.rs
/// Shared CSS for buttons, pills, animations, and responsive helpers.
/// Injected once by layout components — pages should not repeat these.
pub const CSS: &str = r#"
    /* Buttons */
    .btn-primary {
        display: inline-flex; align-items: center; gap: 0.4rem; padding: 0.5rem 1.2rem;
        border-radius: 6px; background: var(--accent); color: white; border: none;
        font-size: 0.85rem; font-weight: 600; cursor: pointer; transition: all 0.2s;
        text-transform: uppercase; letter-spacing: 0.03em;
    }
    .btn-primary:hover { filter: brightness(1.15); box-shadow: 0 0 15px var(--accent-glow); }
    .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }

    .btn-secondary {
        padding: 0.5rem 1rem; border-radius: 6px; background: var(--bg-surface);
        border: 1px solid var(--border); color: var(--text-secondary); font-size: 0.85rem;
        cursor: pointer; transition: all 0.15s;
    }
    .btn-secondary:hover { color: var(--text-bright); }

    .btn-danger { background: #ef4444; }
    .btn-danger:hover { background: #dc2626; }

    /* Status / Role pills */
    .status-pill {
        display: inline-block; padding: 0.15rem 0.5rem; border-radius: 999px;
        font-size: 0.65rem; font-weight: 700; text-transform: uppercase; letter-spacing: 0.04em;
    }
    .status-pill.pending { background: #f59e0b33; color: #fbbf24; }
    .status-pill.active, .status-pill.accepted { background: #10b98133; color: #34d399; }
    .status-pill.inactive, .status-pill.rejected { background: #ef444433; color: #f87171; }
    .status-pill.trial { background: #3b82f633; color: #60a5fa; }
    .status-pill.draft { background: #6b728033; color: #9ca3af; }
    .status-pill.registration { background: #8b5cf633; color: #a78bfa; }
    .status-pill.completed { background: #10b98133; color: #34d399; }
    .status-pill.in_progress { background: #f59e0b33; color: #fbbf24; }
    .status-pill.withdrawn { background: #6b728033; color: #9ca3af; }

    .role-pill {
        display: inline-block; padding: 0.15rem 0.5rem; border-radius: 999px;
        font-size: 0.65rem; font-weight: 700; text-transform: uppercase; letter-spacing: 0.04em;
    }
    .role-pill.admin { background: #ef444433; color: #f87171; }
    .role-pill.officer { background: #f9731633; color: #f97316; }
    .role-pill.member { background: #7c3aed33; color: #a78bfa; }
    .role-pill.recruit { background: #6b728033; color: #9ca3af; }

    /* Empty state */
    .empty-state { color: var(--text-muted); text-align: center; padding: 3rem 1rem; font-size: 0.9rem; }

    /* Loading */
    .loading-state { color: var(--text-muted); padding: 2rem; font-size: 0.9rem; }

    /* Animations */
    @keyframes fade-in { from { opacity: 0; } to { opacity: 1; } }
    @keyframes slide-up { from { transform: translateY(10px); opacity: 0; } to { transform: translateY(0); opacity: 1; } }
"#;
```

**Step 2: Create admin CSS module**

Move admin-specific patterns from `ADMIN_SHARED_CSS` to `admin.rs`:

```rust
// crates/app/src/styles/admin.rs
/// Admin-specific CSS: data tables, form modals, toolbar, pagination.
/// Injected by AdminLayout — admin pages inherit these styles.
pub const CSS: &str = r#"
    /* Data Table */
    .data-table { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
    .data-table th { ... }
    .data-table td { ... }
    .data-table tr:hover td { background: var(--bg-card); }

    /* Row actions */
    .row-actions { display: flex; gap: 0.35rem; flex-wrap: wrap; }
    .row-btn { ... }

    /* Admin toolbar */
    .admin-toolbar { ... }

    /* Form Modal */
    .form-modal-overlay { ... }
    .form-modal { ... }
    .form-modal-header { ... }
    .form-modal-body { ... }
    .form-modal-footer { ... }

    /* Form Fields */
    .form-field { ... }
    .form-label { ... }
    .form-input, .form-select, .form-textarea { ... }

    /* Form Grid */
    @media (min-width: 480px) {
        .form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; }
        .form-grid .span-full { grid-column: 1 / -1; }
    }

    /* Summary Cards */
    .summary-cards { ... }
    .summary-card { ... }

    /* Pagination */
    .pagination { ... }

    /* Confirm Dialog */
    .confirm-body { ... }
"#;
```

(Fill in the `...` sections from the current `ADMIN_SHARED_CSS` content.)

**Step 3: Create public and strategy CSS stubs**

```rust
// crates/app/src/styles/public.rs
/// Public site CSS: section headers, nav overrides, footer, cards.
/// Injected by PublicLayout.
pub const CSS: &str = r#"
    /* Will be populated when extracting from public page CSS */
"#;
```

```rust
// crates/app/src/styles/strategy.rs
/// Strategy section CSS: shared toolbar, panel, canvas overlay patterns.
/// Injected by StrategyLayout.
pub const CSS: &str = r#"
    /* Will be populated when extracting from strategy component CSS */
"#;
```

**Step 4: Register module and verify**

Add `mod styles;` to `crates/app/src/main.rs`.

Run: `cargo check --package scuffed-app`

**Step 5: Commit**

```bash
git add crates/app/src/styles/ crates/app/src/main.rs
git commit -m "feat(app): add CSS module system with common, admin, public, strategy modules"
```

### Task 4.2: Wire CSS modules into layout components

**Step 1: Inject common CSS in App root**

In `crates/app/src/main.rs`, add common CSS alongside theme:

```rust
style { {theme::THEME_CSS} }
style { {styles::common::CSS} }
```

**Step 2: Inject admin CSS in AdminLayout**

In `crates/app/src/layouts/admin.rs`, replace `ADMIN_SHARED_CSS` injection:

```rust
// Before: style { {ADMIN_SHARED_CSS} }
// After:
style { {crate::styles::admin::CSS} }
```

**Step 3: Remove per-page `style { {ADMIN_SHARED_CSS} }` blocks**

Every admin page currently has `style { {ADMIN_SHARED_CSS} }` in its RSX. Remove these since the layout now injects the CSS once.

Files to update:
- `admin/games.rs`
- `admin/announcements.rs`
- `admin/applications.rs`
- `admin/moderation.rs`
- `admin/members.rs`
- `admin/teams.rs`
- `admin/schedule.rs`
- `admin/matches.rs`
- `admin/tournaments.rs`
- `admin/dashboard.rs`
- `admin/settings.rs`
- `admin/audit_log.rs`

Run: `cargo check --package scuffed-app`

**Step 4: Update admin_shared.rs**

Remove `ADMIN_SHARED_CSS` from `admin_shared.rs`. Keep only the components (DataTable, FormModal, ConfirmDialog, StatusPill, RolePill, SummaryCard). The CSS now lives in `styles/admin.rs` and `styles/common.rs`.

Run: `cargo check --package scuffed-app`

**Step 5: Commit**

```bash
git add crates/app/src/
git commit -m "refactor: wire CSS modules into layouts, remove per-page CSS injection

Admin CSS injected once in AdminLayout instead of in every page.
Common CSS (pills, buttons, animations) injected in App root."
```

### Task 4.3: Extract duplicated CSS from pages (stretch goal)

This task is lower priority and can be done incrementally:

1. Extract shared button patterns from strategy pages (`.btn-create` → use `.btn-primary`)
2. Extract duplicated panel header CSS from strategy components into `styles/strategy.rs`
3. Extract color swatch patterns (toolbar.rs + properties_panel.rs) into shared CSS
4. Consider extracting the monolithic `HOME_CSS` into logical sections

Each extraction: move CSS to the appropriate styles module, remove from original file, verify compilation.

This task is left as a stretch goal since the impact is lower than Steps 1-3.

---

## Execution Order Summary

| Step | Risk | Impact | Est. Files Changed |
|------|------|--------|-------------------|
| 1. Consolidate Types | Low | High clarity | 14 |
| 2. use_api Hook | Low | High boilerplate reduction | 15 |
| 3. ModalController | Medium | High signal reduction | 11 |
| 4. CSS Modules | Medium | Medium maintenance | 16 |

Each step is independently shippable and verifiable with `cargo check`.

---

## Verification Checklist

After all steps:
- [ ] `cargo check --package scuffed-types` passes
- [ ] `cargo check --package scuffed-app --features web` passes
- [ ] `cargo check --package scuffed-app --features desktop --no-default-features` passes
- [ ] `cargo run -p scuffed-server` starts without errors
- [ ] All admin pages load and function correctly in browser
- [ ] Modals open, close, and submit correctly
- [ ] Data tables display correctly
- [ ] Refresh after mutations works
