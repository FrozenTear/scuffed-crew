# Main-only commits to re-implement after merging `feat/desktop-canvas-rendering`

When merging `feat/desktop-canvas-rendering` into `main` with feat winning all conflicts (`-X theirs`), the four commits below get overwritten. Each implements a feature the feat branch *also* touched, but feat's versions either have different scope or use a different protocol. This doc captures what specifically gets lost so it can be rebuilt on top of the merged branch.

## Reference commits (on `main`, prior to merge)

| Feature | Hash | Date |
|---|---|---|
| Identity / NIP-07 | `da8a777` | 2026-04-14 |
| Blog / NIP-23 articles | `385c110` | 2026-05-03 |
| Events / weekly RSVP | `5e967ef` | 2026-05-03 |
| Polls / server+frontend | `ee780c9` | 2026-05-03 |

To inspect any of them after the merge: `git show <hash>`.

---

## 1. Identity — NIP-07 browser extension flow (`da8a777`)

**What feat already has**: NIP-49 encrypted key backup. **What gets lost**: NIP-07 browser-extension login (auto-detect `window.nostr`, pubkey retrieval, challenge-response signing).

These are *complementary*, not duplicates — NIP-07 is for users with a signing extension (nos2x, Alby), NIP-49 is for encrypted local key backup. Ideally the merged identity page supports both.

**Files added**:
- `crates/app/src/pages/identity.rs` (746 lines)

**Key implementation details to rebuild**:
- `wasm_bindgen` + `web_sys` interop with `window.nostr`. Helpers: `has_nip07()`, `get_nostr_obj()`, `nip07_get_public_key()`, `nip07_sign_event()`.
- Hex → bech32 `npub1...` encoding (custom impl, ~50 lines, no external crate).
- Challenge-response link flow: GET challenge from server → sign with extension → POST signed event back to link identity to user.
- Error states for: no extension, permission denied, expired challenge, network error.
- Confirmation dialog before unlinking.
- `clipboard_write()` helper for copy-to-clipboard.
- Loading spinners and toast feedback (uses existing `use_toast()`).

**Server-side dependency**: assumes `/api/identity/challenge` and `/api/identity/link` endpoints. Check whether feat's NIP-49 work added these or if they need to be added as part of the rebuild.

---

## 2. Blog / Articles — NIP-23 schema (`385c110`)

**What feat already has**: "Blog, Wiki, and Forum community features" (broader scope). **What may differ**: NIP-23 long-form-content schema specifics, slug-based routing, pulldown-cmark rendering choice, admin CRUD page details.

**Files added**:
- `crates/types/src/org/article.rs` — `Article` struct (id, slug, title, content_markdown, summary, cover_image_url, author_member_id, published, **`nostr_event_id`** (NIP-23 link), created_at, updated_at, published_at).
- `crates/types/src/api/articles.rs` — `CreateArticleRequest`, `UpdateArticleRequest`.
- `crates/db/src/queries/articles.rs` (226 lines) — CRUD + publish/unpublish + paginated list + count.
- `crates/site-server/src/routes/articles.rs` (292 lines) — public list/get, admin list-all, create/update/publish/delete. Uses `AuditAction` audit log entries for lifecycle events.
- `crates/app/src/pages/blog.rs` — public paginated card grid.
- `crates/app/src/pages/blog_article.rs` — detail view with `pulldown-cmark` markdown rendering.
- `crates/app/src/pages/admin/articles.rs` — admin CRUD UI.

**SurrealDB migration** (added in `crates/db/src/migrations.rs`):
```surql
DEFINE TABLE article SCHEMAFULL;
DEFINE FIELD slug ON article TYPE string;
DEFINE FIELD title ON article TYPE string;
DEFINE FIELD content_markdown ON article TYPE string;
DEFINE FIELD summary ON article TYPE option<string>;
DEFINE FIELD cover_image_url ON article TYPE option<string>;
DEFINE FIELD author_member_id ON article TYPE string;
DEFINE FIELD published ON article TYPE bool DEFAULT false;
DEFINE FIELD nostr_event_id ON article TYPE option<string>;
DEFINE FIELD created_at ON article TYPE datetime DEFAULT time::now();
DEFINE FIELD updated_at ON article TYPE datetime DEFAULT time::now();
DEFINE FIELD published_at ON article TYPE option<datetime>;
```

**Routes registered**: `/blog`, `/blog/:slug`, admin `/admin/articles`. Nav link in public layout + admin sidebar.

---

## 3. Events — weekly calendar + RSVP (`5e967ef`)

**What feat already has**: "Events page with RSVP and Scrim Board with scheduling" (superset). **What may differ**: 7-day weekly grid layout, responsive 7/2/1 column breakpoints, RSVP toast feedback, exact UI styling.

**Files added**:
- `crates/app/src/pages/events.rs` (392 lines) — pure UI page.

**Frontend behavior**:
- 7-day weekly grid (Mon–Sun), recurring events shown by `day_of_week`.
- Event card fields: title, time, timezone, duration_minutes, is_recurring badge.
- RSVP buttons: Going / Maybe / Can't, only for authenticated members.
- Live RSVP summary counts fetched from `GET /api/events/:id/rsvp-summary` returning `{event_id, yes_count, maybe_count, no_count}`.
- Toast feedback via `use_toast()` after RSVP success/failure.
- Responsive: 7-col desktop, 2-col tablet (≤900px), 1-col mobile (≤480px).

**Routes**: `/events` registered. Nav links added to desktop and mobile menus.

**Server-side dependency**: `GET /api/events`, `POST /api/events/:id/rsvp` with `{status: "going"|"maybe"|"no"}`, `GET /api/events/:id/rsvp-summary`. Verify whether feat's events implementation exposes these — may need to bridge.

---

## 4. Polls — server routes + DB queries + frontend (`ee780c9`)

**What feat already has**: polls schema, types, server routes, components, page (3 commits). **What may differ**: query implementation specifics, viewer-vote tracking, audit log integration.

This is the most likely to be a near-duplicate. **Worth diffing feat's `crates/db/src/queries/polls.rs` and `crates/site-server/src/routes/polls.rs` against main's versions before assuming feat's work is complete.**

**Files added by main's commit**:
- `crates/db/src/queries/polls.rs` (278 lines) — `create_poll`, `list_polls`, `get_poll`, `get_poll_results`, `vote`, `get_member_poll_votes`. Uses SurrealDB `DbPoll` and `DbPollVote` tables.
- `crates/site-server/src/routes/polls.rs` (309 lines) — `GET /api/polls`, `GET /api/polls/:id` (returns `PollDetailResponse` with poll + results + viewer_votes), `POST /api/polls` (officer-only), `POST /api/polls/:id/vote` (member-only).
- `crates/app/src/components/poll/{mod,poll_card,poll_create}.rs` — UI components.
- `crates/app/src/pages/polls.rs` — public polls page.

**SurrealDB schema** (in `crates/db/src/migrations.rs`):
```surql
DEFINE TABLE poll SCHEMAFULL;
DEFINE FIELD title ON poll TYPE string;
DEFINE FIELD description ON poll TYPE option<string>;
DEFINE FIELD options ON poll TYPE array;
DEFINE FIELD options.* ON poll TYPE string;
DEFINE FIELD close_at ON poll TYPE option<datetime>;
DEFINE FIELD allow_multiple ON poll TYPE bool DEFAULT false;
DEFINE FIELD created_by ON poll TYPE string;
DEFINE FIELD is_active ON poll TYPE bool DEFAULT true;

DEFINE TABLE poll_vote SCHEMAFULL;
DEFINE FIELD poll_id ON poll_vote TYPE string;
DEFINE FIELD member_id ON poll_vote TYPE string;
DEFINE FIELD option_index ON poll_vote TYPE int ASSERT $value >= 0;
DEFINE INDEX poll_vote_unique_idx ON poll_vote
    COLUMNS poll_id, member_id, option_index UNIQUE;
```

**Notable details**: `PollDetailResponse` exposes `viewer_votes: Vec<u32>`, derived from current authenticated user's `member_id`. Audit log entries on poll create/close/delete.

---

## Suggested re-implementation order after the merge

1. **Polls** first — most likely a near-duplicate of feat's; just diff and reconcile, probably small.
2. **Events** next — likely the smallest delta (UI styling + 7-day grid layout if feat's already covers RSVP).
3. **Blog NIP-23 fields** — verify feat's article/blog tables have `nostr_event_id` and slug-based routing; add if missing.
4. **Identity NIP-07** last — additive on top of feat's NIP-49 page; the most genuinely-new code to write.

For each: check `git show <hash>` first, then diff against feat's current implementation in the merged tree.
