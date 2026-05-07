# The Scuffed Crew

Gaming org community site. Rust monorepo: Dioxus 0.7 app + legacy Leptos frontends + Axum backends + SurrealDB.

## Architecture

```
crates/
  app/           # Primary Dioxus 0.7 WASM app (strategy editor, admin, chat)
  server/        # Axum backend for Dioxus app (REST + WebSocket)
  types/         # Shared types between app and server
  api-client/    # HTTP client (web + native)
  site/          # Legacy Leptos 0.8 CSR site (trunk build → dist/)
  admin/         # Legacy Leptos 0.8 CSR admin SPA (trunk build → dist/admin/)
  site-server/   # Legacy Axum server for site/admin (API + static files)
  db/            # SurrealDB client, migrations, queries
  auth/          # OAuth, sessions, crypto (shared crate)
  ui/            # Legacy Leptos 0.8 shared UI components (scuffed-ui)
```

## SurrealDB Gotchas

- **`$token` is a reserved variable.** Never use `$token` as a bind parameter name. Use `$tok` or similar.
- **chrono DateTime<Utc> does NOT serialize correctly for SCHEMAFULL tables.** SurrealDB rejects chrono's string-serialized datetime (`"2026-02-27T..."`) because it expects a native datetime type. Use `surrealdb::types::Datetime` (aliased as `SurrealDatetime`) in all `Db*` structs. Convert to/from chrono in the conversion layer:
  - Rust → DB: `SurrealDatetime::from(Utc::now())`
  - DB → Rust: `db.field.into()` (implements `From<SurrealDatetime> for DateTime<Utc>`)
  - Optional: `db.field.map(|d| d.into())`
- **Raw SurrealQL datetimes work fine:** `time::now()`, `time::now() + 365d`, etc.
- **We use SurrealDB v3 only (never v2).** `type::thing()` does NOT work. Use `RecordId` bindings instead: `.bind(("rid", surrealdb::RecordId::from(("table", id))))` and `$rid` in the query. For `RELATE`: `RELATE $member_rid -> edge -> $team_rid`.

## Dev Mode

- `SURREALDB_URL` unset → in-memory database with auto-seeded dev data (user=devadmin, role=admin)
- Visit `/api/dev/login` to set the session cookie, then go to `/admin/`
- Admin SPA `dev-noauth` feature bypasses frontend auth guards but NOT server-side extractors
- Build admin: `cd crates/admin && trunk build --features dev-noauth`
- Run server: `PORT=3030 cargo run -p scuffed-site-server`

## Conventions

- DB internal structs (`Db*`) are private to `crates/db/src/queries/` — public types live in `crates/db/src/types.rs`
- Auth extractors: `OrgMember` (any member) → `OfficerUser` (officer+) → `AdminUser` (admin only)
- Refresh pattern in admin: `RwSignal<u32>` as `LocalResource` dependency, increment after mutations
- Toast feedback for all user-facing mutations: `use_toast().show(Toast::success(...))`
- Audit log: fire-and-forget after successful mutations, log error but don't fail the request

## Research Strategy: Wave-Based Multi-Agent

For complex research tasks, use a wave-based approach:

**Wave 1 — Broad landscape.** Launch 3-5 agents in parallel covering different angles of the same question (e.g., platform survey, competitor analysis, gap analysis, community patterns). Synthesize all results into a unified briefing with deduplicated, prioritized findings before proceeding.

**Wave 2+ — Deep dives.** Based on Wave 1 findings, launch targeted agents that go deep on specific implementation questions identified as high-priority. Each subsequent wave narrows focus based on what the previous wave surfaced.

**Principles:**
- Act as team leader: dispatch, wait, synthesize, redirect
- Agents run in parallel (background) — never duplicate their work in the main thread
- Between waves, present a consolidated summary to the user and let them steer
- Save key decisions/constraints to memory as they emerge
- Each agent gets a focused brief with clear scope and "RESEARCH ONLY" instruction

## Brand

- Brand color: `#7c3aed` (purple)
- Age requirement: 16+
- Tone: direct, no-drama, no politics
