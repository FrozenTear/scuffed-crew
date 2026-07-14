# The Scuffed Crew

Gaming org community site. Rust monorepo: Dioxus 0.7 app + Axum backends + SurrealDB. (Legacy Leptos frontends removed 2026-06.)

## Architecture

```
crates/
  app/           # Dioxus 0.7 WASM app (site, admin, strategy editor, chat) → dx build → dist/
  server/        # Unified Axum binary: site-server routes + strategy WebSocket + chat (scuffed-server)
  site-server/   # Core REST API library + main (sessions, members, tournaments, uploads, SPA fallback)
  types/         # Shared types between app and server
  api-client/    # HTTP client (web + native)
  db/            # SurrealDB client, migrations, queries
  auth/          # OAuth, sessions, crypto (shared crate)
  chat/          # Nostr chat backend (relay client, NIP-44/59 crypto)
  stat-tracker/  # Overwatch stat tracker (OCR capture daemon + optional Dioxus desktop GUI)
  map-pipeline/  # Map asset processing tooling
  map-renderer/  # Map rendering tooling
  relay-policy/  # Nostr relay policy plugin (NIP-42/29, rate limits) — standalone, not yet in deploy (future work)
```

Production = `scuffed-server` serving `dist/` (built by `dx build` from crates/app). See `scripts/build.sh` and `Containerfile`.

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
- `/api/dev/login` sets the session cookie (route only registered in dev mode), then go to `/admin/`
- Run app: `cd crates/app && dx serve` (or `dx build` then serve `dist/` via the server)
- Run server: `PORT=3030 cargo run -p scuffed-server`

## Production / VPS

- Prefer `./scripts/install.sh` (generates `data/secrets.env`, free `HOST_PORT`, Podman Compose)
- First visit: create admin account in the browser (no Discord required)
- Full runbook: `docs/deploy.md`
- Optional later: systemd Quadlet (not required for first install)

## Conventions

- DB internal structs (`Db*`) are private to `crates/db/src/queries/` — public types live in `crates/db/src/types.rs`
- Auth extractors: `OrgMember` (any member) → `OfficerUser` (officer+) → `AdminUser` (admin only)
- Refresh pattern in admin: list resources depend on a refresh counter signal; increment it after mutations (e.g. `members.refresh += 1`)
- Toast feedback for all user-facing mutations: `use_toast().show(Toast::success(...))`
- Audit log: fire-and-forget after successful mutations, log error but don't fail the request

### Membership policy

- Pure rules in `crates/site-server/src/membership_policy.rs`
- **Actionable admin** = active admin not suspended/banned (use for last-admin + setup)
- Policy denials: **403** authz, **400** invalid state, **409** CAS / last-admin race / dup apply
- After demote/deactivate/suspend/ban of an actionable admin: `assert_has_actionable_admin` + compensate
- Application transitions: membership side effects **before** CAS status write
- Submit: if `count_open_applications > 1` after insert, delete the new row and 409
- Applicants self-withdraw via `POST /api/applications/mine/withdraw` (pending/trial only)
- Ban deactivates; lift does **not** re-activate (see `docs/notes/moderation.md`)

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
