# Forum hierarchy design (old-school boards)

**Date:** 2026-07-11  
**Status:** Implemented on main (2026-07-11) — hierarchy API + public/admin UI  
**Scope:** Local forum only (`forum_backend = local`). Nostr-backed forum remains optional/future.

---

## Goal

Replace flat string “categories” (hardcoded tabs: general / game / strategy / offtopic) with a classic clan-board structure:

```text
Category          e.g. Games, Org, Off-topic     (section header)
└── Board         e.g. Overwatch, Announcements  (threads live here)
    └── Sub-board e.g. Strategy, VODs            (optional; threads live here too)
        └── Thread
            └── Replies
```

**Example path:** Games → Overwatch → Strategy → thread  
**Without sub-board:** Games → LFG → thread  

Sub-boards are **optional**. Max depth under a category: **board → one level of sub-board** (no infinite nesting).

---

## Current state (baseline)

| Piece | Today |
|-------|--------|
| Schema | `forum_thread.category: string`, `forum_reply` |
| UI | Hardcoded `CATEGORIES` in `crates/app/src/pages/forum.rs` |
| Routes | `/forum`, `/forum/:id` |
| Features already | Pin, lock, create thread, replies, optional Nostr publish hooks |

---

## Domain model

### `forum_category`

| Field | Notes |
|-------|--------|
| `id` | Surreal record id |
| `name` | Display name |
| `slug` | URL-safe unique |
| `description` | Optional short blurb |
| `sort_order` | i32, lower first |
| `is_active` | Soft-hide without delete |

Categories **do not** hold threads. They only group boards on the index.

### `forum_board`

| Field | Notes |
|-------|--------|
| `id` | |
| `category_id` | Parent category |
| `parent_board_id` | `NONE` = top-level board; set = **sub-board** of another board in the **same** category |
| `name`, `slug` | Slug unique within parent scope (or globally unique — prefer **global** for simple URLs) |
| `description` | Shown on index / board header |
| `sort_order` | |
| `is_locked` | No new threads if true (replies policy TBD: allow or not) |
| `min_role` | Optional: `recruit` / `member` / `officer` / `admin` for posting (read policy later) |
| `is_active` | |

**Invariant:** `parent_board_id`, if set, must point to a board with `parent_board_id = NONE` (only one sub-level).

### `forum_thread` (change)

| Field | Change |
|-------|--------|
| `board_id` | **New required** FK to board (or sub-board) |
| `category` | **Deprecated** string; migrate then drop (or keep read-only during transition) |

Keep: title, content, author, pinned, locked, timestamps, nostr_event_id, is_active.

### `forum_reply`

Unchanged structurally (thread_id, author, content, …).

---

## URLs (public)

| Path | Page |
|------|------|
| `/forum` | Index: categories → boards (and nested sub-boards) with last-activity summary when cheap |
| `/forum/b/:slug` | Board or sub-board thread list |
| `/forum/t/:id` | Thread view (prefer explicit `t/` so ids never clash with slugs) |

**Migration of routes:** keep `/forum/:id` working as thread id for a deprecation period, or redirect to `/forum/t/:id`.

---

## Permissions (v1)

| Action | Who |
|--------|-----|
| Read public boards | Anyone (or logged-in only if we gate later — default: same as today) |
| Create thread / reply | Org member (same as current forum posts) |
| Create/edit/reorder categories & boards | Officer+ or admin |
| Pin / lock thread | Officer+ (existing) |
| Post in locked board | Nobody new threads; officer override optional |

`min_role` on board: enforce on create thread/reply in v1 if cheap; otherwise ship structure first and add role gates in a follow-up.

---

## Admin UX

- Admin section **Forum** (or under Settings):  
  - List categories (reorder)  
  - Per category: boards  
  - Per board: optional sub-boards  
- Create / rename / describe / soft-deactivate  
- No need for full drag-and-drop v1: up/down sort or numeric order  

---

## Public UX

**Index (`/forum`)** — old-school list:

```text
## Games
  Overwatch          12 threads   last: … by …
    └ Strategy        4 threads   last: …
  Valorant            …

## Org
  Announcements
  Applications discussion
```

**Board page** — thread table: sticky first, then by `updated_at`; New Thread if allowed.

**Thread page** — existing thread + replies UI, retargeted to `board_id` breadcrumbs:

`Forum > Games > Overwatch > Strategy > Thread title`

---

## Data migration from string categories

Map existing `forum_thread.category` values:

| Old string | Suggested board |
|------------|-----------------|
| `general` | Category **Org** → board **General** |
| `game` | Category **Games** → board **General** (or **Overwatch** if org is OW-first) |
| `strategy` | Category **Games** → board **Strategy** (or sub-board under Overwatch) |
| `offtopic` | Category **Off-topic** → board **General** |
| unknown / empty | Category **Org** → board **General** |

Bootstrap seed creates that minimal tree on first migration if empty; then `UPDATE forum_thread SET board_id = …` per mapping; stop writing `category` string on new threads.

Exact game names (Overwatch vs multi-game) can be adjusted at seed time from site settings later.

---

## API sketch (v1)

```
GET    /api/forum/tree                 # categories + boards + sub-boards
GET    /api/forum/boards/:slug         # board meta + paginated threads
POST   /api/forum/boards               # officer+ create board/sub-board
PATCH  /api/forum/boards/:id
POST   /api/forum/categories           # officer+
PATCH  /api/forum/categories/:id

GET    /api/forum/threads?board=slug   # replace category filter
POST   /api/forum/threads              # body: board_id | board slug, title, content
GET    /api/forum/threads/:id
POST   /api/forum/threads/:id/replies
PATCH  /api/forum/threads/:id/pin|lock
```

Keep existing reply/pin/lock handlers; extend create/list to `board_id`.

---

## Out of scope (explicit)

- Infinite nesting beyond board → sub-board  
- Full ACL matrix (per-group read/write)  
- Nostr as primary forum store  
- Attachments / BBCode polish  
- Unread tracking, watch lists (nice later)  

---

## Implementation order (when building)

1. Schema: `forum_category`, `forum_board`; `forum_thread.board_id`  
2. Seed + migrate string categories → boards  
3. API tree + board thread list  
4. Public index + board page + breadcrumbs  
5. Admin CRUD for categories/boards  
6. Deprecate string `category` field and hardcoded UI tabs  
7. (Optional) `min_role` + locked-board enforcement  

---

## Success criteria

1. Admin can create **Games → Overwatch → Strategy** without a deploy.  
2. Thread can live on Overwatch **or** Strategy.  
3. Index looks like a classic forum section list, not four hardcoded tabs.  
4. Existing threads still visible after migration.  
5. Local forum still works with Nostr relay offline.  

---

## Operator note

Site deploy and Nostr relay remain independent. Ship hierarchy on **local** forum first.
