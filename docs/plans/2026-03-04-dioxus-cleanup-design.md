# Dioxus Frontend Cleanup — Design

**Date:** 2026-03-04
**Status:** Approved
**Approach:** Bottom-Up Extraction (4 incremental steps)

## Context

The Dioxus 0.7 migration is functionally complete (66 source files, all admin pages, strategy editor with collab, desktop support). Four systemic issues reduce developer velocity:

1. **Signal explosion** — 20+ signals per admin page for modal lifecycle
2. **CSS duplication** — inline CSS strings repeated across 15+ pages
3. **Type duplication** — request/response structs defined locally in pages instead of shared
4. **Fetch boilerplate** — same `use_resource` + refresh + `.ok()` pattern at ~30 call sites

Additionally, Dioxus VDOM interference with native inputs (date picker) was discovered and needs a principled approach.

## Step 1: Consolidate Types

Move all request/response DTOs to `crates/types/src/api/`:

```
crates/types/src/api/
├── mod.rs              # re-exports
├── members.rs          # ChangeRole, ToggleActive, MemberListItem
├── tournaments.rs      # CreateTournament, StatusChange, BracketData
├── teams.rs            # CreateTeam, etc.
├── announcements.rs    # CreateAnnouncement, etc.
└── strategy.rs         # CreateStrategyRequest, UpdateStrategyRequest
```

**Rules:**
- Domain types (`Member`, `Tournament`) stay in `org/` and `strategy.rs`
- API-specific types (create/update requests, list responses) go in `api/`
- Pages import from `scuffed_types::api::*` instead of defining local structs
- Backend can import the same types for handler signatures

**Changes:** Delete local struct definitions in pages, replace with imports. No logic changes.

## Step 2: Custom `use_api` Hook

New file `crates/app/src/hooks.rs`:

```rust
pub struct ApiResource<T> {
    pub data: Resource<Option<T>>,
    pub refresh: Signal<u64>,
    pub error: Signal<Option<String>>,
}

impl<T> ApiResource<T> {
    pub fn reload(&mut self) { self.refresh += 1; }
}

pub fn use_api<T: DeserializeOwned + 'static>(
    url: impl Fn() -> String + 'static,
) -> ApiResource<T>
```

**Usage:**
```rust
// Before (5 lines, no error handling):
let mut refresh = use_signal(|| 0u64);
let data = use_resource(move || async move {
    let _ = refresh();
    ApiClient::web().fetch::<Vec<Member>>("/api/members").await.ok()
});

// After (1 line, with error tracking):
let mut members = use_api::<Vec<Member>>(|| "/api/members".into());
members.reload(); // after mutations
```

**Changes:** Replace ~30 `use_resource` + refresh signal pairs across all pages.

## Step 3: Modal Controller Abstraction

New file `crates/app/src/hooks/modal.rs`:

```rust
#[derive(Clone, Copy)]
pub struct ModalController<T: Clone + 'static> {
    pub open: Signal<bool>,
    pub target: Signal<Option<T>>,
    pub submitting: Signal<bool>,
}

impl<T: Clone + 'static> ModalController<T> {
    pub fn new() -> Self { ... }
    pub fn show(&mut self, item: T) { ... }
    pub fn show_empty(&mut self) { ... }
    pub fn close(&mut self) { ... }
    pub fn submit<F, Fut>(&mut self, f: F) { ... }
}
```

**Impact:** Cuts signal count roughly in half on complex pages. Standardizes modal lifecycle (open/close/submit/reset). Form-specific fields remain as individual signals.

**Usage:**
```rust
// Before (~8 signals per modal):
let mut role_open = use_signal(|| false);
let mut role_target: Signal<Option<Member>> = use_signal(|| None);
let mut role_submitting = use_signal(|| false);

// After (1 controller):
let mut role_modal = ModalController::<Member>::new();
```

## Step 4: CSS Module System

Reorganize CSS into `crates/app/src/styles/`:

```
styles/
├── mod.rs          # re-exports
├── theme.rs        # CSS variables, resets (from current theme.rs)
├── common.rs       # Shared: buttons, pills, animations, responsive helpers
├── admin.rs        # Admin: data tables, form modals, toolbar (from admin_shared.rs)
├── public.rs       # Public: section headers, nav, footer, cards
└── strategy.rs     # Strategy: toolbar, panels, canvas overlays
```

**Rules:**
- Each module exports `pub const CSS: &str`
- Layout components inject base styles once — pages don't repeat them
- Page-specific styles stay inline but should be minimal
- Same CSS-in-Rust approach (no Tailwind, no external files)

**Changes:** Extract common patterns from inline CSS, move to modules. Delete duplicated CSS from individual pages.

## Execution Order

Each step is independently shippable:

1. **Types** — lowest risk, highest clarity improvement
2. **use_api hook** — low risk, kills the most boilerplate
3. **ModalController** — medium risk, biggest signal reduction
4. **CSS modules** — medium effort, long-term maintenance payoff

## Out of Scope

- Service layer / domain services (Approach B — deferred)
- Tailwind or external CSS tooling
- Desktop auth flow (bearer token persistence)
- Loading skeletons / Suspense
- Undo/redo testing
