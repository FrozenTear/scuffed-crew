# Backend and site QA baseline

Pass date: 2026-10-08. Scope is the Axum servers (`crates/server`, `crates/site-server`) and the crates they sit on (`crates/auth`, `crates/db`, `crates/chat`, `crates/types`, `crates/relay-policy`, `crates/api-client`) plus the site client in `crates/app` where it calls those servers. `crates/stat-tracker*` and the map crates were not edited. Map crates did compile as part of the workspace test command CI uses.

This pass records bugs. It does not change application behavior. Each confirmed bug has an ignored regression so `cargo test` stays green.

## Environment

- Worktree: `.claude/worktrees/grok-qa-baseline` on `cursor/backend-qa-baseline-e0cb` (off `main` at `9590e52`).
- Toolchain: `rustc 1.99.0 (b940084d7 2026-09-28)`. `libssl-dev` and `pkg-config` were installed so `openssl-sys` could build.
- CI definition: `.github/workflows/ci.yml` (`dep-guardrails`, `fmt`, `clippy`, `build-and-test`, `app-build`).
- Local server: `PORT=3030 cargo run -p scuffed-server` with `SURREALDB_URL`, `PRODUCTION`, and `ENCRYPTION_KEY` unset. That is the documented in-memory dev boot (`Database::connect_memory`, migrations, `seed_dev_data`). Compose / Podman was not used; it is the production install path and needs generated secrets.
- The process logged `Clan platform server listening on 0.0.0.0:3030`, seeded user `devadmin` / member `devmember` (admin), and wrote no `ERROR` or panic lines during the probe below.
- `dx build` (the `app-build` CI job) was not run. The community-page bug is demonstrated by a serde contract test against the live overview JSON, not by a browser click.
- Full workspace `clippy` (native workspace minus the app and stat-tracker, plus wasm `scuffed-app`) was not re-run. `cargo clippy -p scuffed-site-server -p scuffed-server --all-targets -- -D warnings` exited 0 after the test edits.

## Test suite results

Command matched the `build-and-test` job, from this worktree, with `CARGO_PROFILE_DEV_DEBUG=0`:

```text
cargo fmt --check
bash scripts/check-frontend-deps.sh
bash scripts/check-design-tokens.sh
bash scripts/test-backup-secrets.sh
bash scripts/test-restic-access.sh
cargo test --workspace --exclude scuffed-stat-tracker --exclude scuffed-stat-tracker-ui
cargo test -p scuffed-api-client --no-default-features --features native
```

Guardrail scripts and `cargo fmt --check` exited 0.

`cargo test --workspace …` exited 0. Sum of every `test result` line in that run: **838 passed, 0 failed, 6 ignored**. Nothing was skipped for a missing feature. The 6 ignored tests are the regressions that already existed when that command ran (4 in `qa_baseline`, 1 strategy bearer test, 1 community overview test). Two more ignored tests were added after that run (`missing_public_content_is_not_found_not_internal_error`, `strategy_heroes_returns_catalog`). A follow-up pass added ten more ignored tests (bugs 9–18 below). They are `#[ignore]` only. A second full workspace run was not done. Default `cargo test -p scuffed-site-server --test qa_baseline` after the follow-up exited 0 with 13 ignored. `cargo clippy -p scuffed-site-server --all-targets -- -D warnings` exited 0 after those edits.

| Target | Result |
|---|---|
| `relay-policy` bin | 19 passed |
| `scuffed-api-client` (default features) | 6 passed |
| `scuffed-app` lib | 9 passed |
| `scuffed-app` bin | 203 passed, 1 ignored |
| `scuffed-auth` | 33 passed |
| `scuffed-chat` | 51 passed |
| `scuffed-db` (+ `scuffed-rewrap` bin, 0 tests) | 98 passed |
| `scuffed-map-pipeline` lib + `tests/integration.rs` | 31 + 1 passed |
| `scuffed-map-renderer` | 0 tests |
| `scuffed-server` bin | 30 passed, 1 ignored |
| `scuffed-site-server` lib | 128 passed |
| `tests/api_integration.rs` | 151 passed |
| `tests/public_access_gaps.rs` | 6 passed |
| `tests/qa_baseline.rs` | 0 passed, 4 ignored (5 ignored after the later addition) |
| `tests/seo.rs` | 14 passed |
| `scuffed-types` | 51 passed |
| `scuffed-api-client` `--features native` (separate CI step) | 7 passed |

Ignored tests fail when run on purpose:

```text
cargo test -p scuffed-site-server --test qa_baseline -- --ignored --test-threads=1
cargo test -p scuffed-server --bin scuffed-server owner_bearer -- --ignored
cargo test -p scuffed-server --bin scuffed-server strategy_heroes_returns_catalog -- --ignored
cargo test -p scuffed-app --bin scuffed-app public_overview_accepts -- --ignored
```

Each assertion below is the failure from that run.

## Route surface

148 route registrations in `crates/site-server/src/lib.rs`, `crates/server/src/main.rs`, `crates/server/src/routes/strategy.rs`, and `crates/server/src/routes/ws.rs`. A path counts as covered when a non-comment string in `crates/site-server/tests`, or in a `#[cfg(test)]` module under `site-server`, `server`, or `app`, matches the route pattern. **145 paths match. 3 do not.**

| Area | Auth | Notes |
|---|---|---|
| `GET /api/health` | none, no rate limit | Liveness only. Covered. |
| `/api/auth/{provider}/login`, `callback`, `setup`, `local/login`, `local/register`, `nostr/challenge`, `nostr/verify` | public | Governor: burst 5, then 1 per 2s per IP. Covered (setup, login, register, rate limit). |
| `GET /api/dev/login` | in-memory dev only | Unregistered when `PRODUCTION` is set or `SURREALDB_URL` is non-blank. Live probe got `303` and `Set-Cookie: sc_session=…; HttpOnly`. |
| `POST /api/upload/avatar`, `POST /api/upload/image` | member | Dedicated upload governor. Avatar happy path, oversize, quota covered. |
| `/api/public/*`, `/api/calendar/*.ics`, `/.well-known/nostr.json`, `GET /api/auth/setup-status`, `GET /api/auth/providers` | none | Shared public governor (burst 40, 1 per 200ms). Overview, members, teams, matches, leaderboards, ICS, and the public-access gap tests cover this group. |
| `/api/members`, game accounts, role, reset-password, `POST /api/admin/nostr/republish-profiles` | member / officer / admin | List omits `nostr_secret_key_encrypted` (confirmed again on the live member list). Last-admin and moderation paths have tests. |
| `/api/games`, `/api/teams`, roster, channels, `POST /api/admin/teams/provision-channels` | public read; admin or officer write | Roster read is public. See bug 1. |
| `/api/events`, RSVP, attendance | mixed (public list vs member/officer writes) | Private-event leakage is covered in `public_access_gaps.rs`. |
| `/api/applications` | member submit; officer review | CAS / withdraw / last-admin races covered in `api_integration.rs`. |
| `/api/matches`, `/api/stats/*` | member, officer, or daemon token | Stats me/member/heroes/roles/maps and upload auth covered. |
| `GET /api/audit-log` | admin | Path is requested by tests. |
| `/api/moderation` | officer; lift is admin | Ban / lift / last-admin covered. |
| `/api/announcements`, `/api/polls` | public or member read; officer write | Paths are requested by tests. |
| `/api/articles` | public published; officer drafts | Slug miss is bug 8. |
| `/api/tournaments` and bracket/standings/report | public read; officer write | Covered in both integration files. |
| `/api/scrims` | member | Covered. |
| `/api/wiki` | public read; member write; officer delete | Topic miss is bug 8. |
| `/api/forum/*` | public unless `min_role`; officer for boards | List ACL covered; pagination bugs 2 and 6 are not. |
| `/api/nostr/*` including DM | member (health is lighter) | Challenge, verify, backup, import covered. Relay and `ENCRYPTION_KEY` were unset in the live process, so publish/sync could not be exercised end to end. |
| `GET /api/settings`, `PUT /api/settings`, admin seasons, Discord webhook test | GET is public; writes are admin | Settings body on the live server is org/brand fields (no webhook secret). |
| `GET /robots.txt`, `GET /sitemap.xml`, SPA fallback, `/uploads` | public | `seo.rs`. Fallback behavior is bug 5. |
| `/api/strategy/strategies`, `/{id}`, `/heroes`, `/meta`, patch notes | public reads; member create; feature flag 404 when strategies are disabled | Bearer bug 3. Heroes stub is bug 7. |
| `GET /api/strategy/strategies/mine` | member (`AuthUser`, so bearer works) | **No test request.** |
| `POST /api/chat/auth-token` | member | **No test request.** Live empty body was `422` (missing `relay_url`). |
| `POST /api/chat/send-encrypted` | officer | Unit test around channel lookup. |
| `POST /api/chat/decrypt` | member | **No test request.** Live unauthenticated call was `401`. |
| `GET /api/strategy/ws` | optional; user comes from the session cookie only | Handshake tests exist. Bearer identity is the same hole as bug 3. |

Biggest untested areas: chat token provisioning and decrypt (both need `ENCRYPTION_KEY` and a relay to do anything real), `GET /api/strategy/strategies/mine`, and the strategy WebSocket once a bearer token is the only credential. HTTP path coverage elsewhere is broad; the holes are behavioral (pagination, roster `is_active`, SPA miss, cookie-vs-bearer).

## Live probe

Against `http://127.0.0.1:3030` after the seed finished.

| Call | Result |
|---|---|
| `GET /api/health` | `200` |
| `GET /api/auth/me`, `GET /api/members`, `GET /api/audit-log`, `GET /api/polls` with no cookie | `401` `{"error":"Authentication required"}` |
| `PUT /api/settings` with no cookie | `401` |
| `GET /api/settings`, `GET /api/games`, `GET /api/wiki`, `GET /api/announcements` | `200` JSON |
| `GET /api/public/overview` | `200`. Keys: `teams`, `games`, `events`, `announcements`, `settings`, `member_count`, `upcoming_matches`, `recent_results`. There is no `team_count` or `upcoming_events`. |
| `GET /api/members` with the dev session | `200`. Row keys include `nostr_pubkey` and `nostr_key_mode`. `nostr_secret_key_encrypted` is absent. |
| `POST /api/strategy/strategies` with the dev cookie, `visibility: private` | `201` |
| `GET` that id with the same cookie | `200` |
| `GET` that id with `Authorization: Bearer dev-session-token-do-not-use-in-production` and no cookie | `404` `{"error":"Strategy not found"}` |
| `GET /api/strategy/heroes` | `200` `{"data":[]}` |
| `GET /api/forum/threads/does-not-exist`, `GET /api/wiki/no-such-topic`, `GET /api/articles/no-such-slug` | `404` `{"error":"Internal error"}` |
| `GET /api/games/no-such`, `GET /api/teams/no-such`, `GET /api/public/members/no-such` | `404` with a specific not-found message |
| `POST /api/auth/local/login` bad JSON | `400` `text/plain` (Axum JSON parse error) |
| `POST` login `{}` | `422` `text/plain` (missing `username`) |
| `POST` login username `名前`, empty password | `401` JSON |
| `POST` login with a 5000-character username | `401` JSON, no panic |
| Repeated login after the burst | `429` |
| `OPTIONS /api/public/overview` with `Origin: http://localhost:3000` | `access-control-allow-origin: http://localhost:3000` |
| Same with `Origin: https://evil.example` | `200`, no `access-control-allow-origin` |
| `GET /api/qa-baseline-no-such-route` on this process | `404` `text/plain` `not found` |

The unknown-API live result differs from bug 5 because this process booted with no `dist/index.html`. The regression builds a dist directory that contains the shell and gets `200` `text/html`. That is the production shape (`scuffed-server` serving `dx build` output).

`POST` to an unknown `/api/…` path was `405`. `DELETE /api/health` was `405`.

## Confirmed bugs

Severity is about what a caller can observe on a deployed site. Tests are ignored so CI stays green. Run them with the commands in the test section.

### 1. High — deactivated members stay on public rosters

`GET /api/public/members/{id}` returns `404` once `member.is_active` is false. `GET /api/teams/{id}/roster`, `GET /api/public/teams/{id}`, and `roster_count` on `GET /api/public/overview` still list that member. Roster queries filter `plays_on.is_active`, and a ban sets `member.is_active = false` without dropping the edge (`crates/db/src/queries/roster.rs` `get_team_roster_named`, `crates/site-server/src/routes/roster.rs`, `crates/site-server/src/routes/public.rs`).

- Expected: a deactivated or banned member is absent from public roster payloads and does not increment `roster_count`.
- Actual: the roster array contains `member_id = membermember` after `UPDATE member:membermember SET is_active = false`.
- Test: `deactivated_member_is_absent_from_public_rosters` in `crates/site-server/tests/qa_baseline.rs`. Failure: `GET /api/teams/{id}/roster listed a deactivated member: ["membermember"]`.

### 2. Medium — forum `min_role` is applied after `LIMIT`

`list_forum_threads` applies SQL `LIMIT`/`START`, then drops rows the caller cannot see (`crates/site-server/src/routes/forum.rs` `list_threads`, `crates/db/src/queries/forum.rs`). A newer officer-only thread occupies the only slot of `?limit=1`. The anonymous caller drops it and receives an empty page, so the older public thread never appears.

- Expected: `limit=1` still returns a public thread when the newest row is restricted, and the restricted title is absent.
- Actual: `{"threads":[],"total":0}`.
- Test: `forum_list_does_not_hide_public_threads_behind_restricted_ones` in `crates/site-server/tests/qa_baseline.rs`.

The existing `forum_unfiltered_list_hides_restricted_and_orphan_threads` test still passes. It checks that a restricted row is hidden when the page is large enough to include the public row. It does not catch this hole.

### 3. Medium — private strategy GET ignores `Authorization: Bearer`

`POST /api/strategy/strategies` takes `AuthUser`, which accepts the bearer token. `GET /api/strategy/strategies/{id}` calls `try_get_user`, which reads only the session cookie (`crates/server/src/routes/strategy.rs`). A private strategy the token just created is `404`.

- Expected: the owner bearer receives `200` and the strategy body.
- Actual: `404` `{"error":"Strategy not found"}`. The same id with the cookie is `200` (unit test and live probe).
- Test: `owner_bearer_can_read_private_strategy` in `crates/server/src/routes/strategy.rs`.

`GET /api/strategy/ws` resolves the user with `get_user_from_cookie` in `crates/server/src/routes/ws.rs`. `GET /api/strategy/meta` uses the same `try_get_user` cookie lookup for the personal block. Those two paths were not given their own failing tests.

### 4. Medium — Community stats never render

`Community` in `crates/app/src/pages/community.rs` fetches `GET /api/public/overview` into a `PublicOverview` that requires `team_count` and `upcoming_events`. The route returns `teams` and `events` arrays plus `member_count` (`crates/site-server/src/routes/public.rs`). `fetch` fails, `.ok()` swallows it, and the members/teams block is omitted. The live overview body has the server shape.

- Expected: the client accepts the live payload and can show `member_count` and a team count.
- Actual: `missing field team_count`.
- Test: `public_overview_accepts_the_live_overview_payload` in `crates/app/src/pages/community.rs`.

The homepage reads `teams` directly. This mismatch is the community page.

### 5. Medium — unknown `GET /api/*` is the SPA shell

`spa_service` treats a missing multi-segment path as a client route when `index.html` exists (`classify_spa_route` / `is_static_miss_path` in `crates/site-server/src/routes/seo.rs`). `/api/…` is multi-segment and has no static extension, so an unregistered GET is `200` `text/html`.

- Expected: `404` with a non-HTML body.
- Actual: `200`, `content-type: text/html; charset=utf-8`, body is the shell (`SPA-SHELL-MARKER` in the test).
- Test: `unknown_api_get_is_json_404` in `crates/site-server/tests/qa_baseline.rs`.

On the live process, which had no shell at boot, the same URL was `404` `text/plain`. `POST` to an unknown API path was `405`.

### 6. Low — forum `total` is the page length

`list_threads` sets `total` to `items.len()` after the page is filtered (`crates/site-server/src/routes/forum.rs`).

- Expected: two threads and `?limit=1` yield `total: 2` and one row.
- Actual: `total: 1` with a single thread (`Second`).
- Test: `forum_thread_total_counts_every_match` in `crates/site-server/tests/qa_baseline.rs`.

### 7. Medium — strategy heroes page is fed an empty list

`list_heroes` in `crates/server/src/routes/strategy.rs` is `Json(json!({ "data": [] }))`. `StrategyHeroes` in `crates/app/src/pages/strategy/heroes.rs` renders that `data` array (name, role, abilities, health).

- Expected: the catalog is non-empty so the page can list heroes.
- Actual: `200` `{"data":[]}` (live probe and the test).
- Test: `strategy_heroes_returns_catalog` in `crates/server/src/routes/strategy.rs`.

### 8. Low — missing forum, wiki, and article rows say "Internal error"

Status is `404`. The body is `{"error":"Internal error"}`.

- `get_forum_thread` maps every `Err`, including `DbError::NotFound`, to that body (`crates/site-server/src/routes/forum.rs`).
- `get_wiki_page` maps `NotFound` to `404` and still sets the message to `Internal error` (`crates/site-server/src/routes/wiki.rs`).
- `article_not_found` does the same (`crates/site-server/src/routes/articles.rs`).

Games, teams, and public member misses use a specific message (`Game not found`, and so on). Live checks matched that split.

- Expected: `404` whose `error` string is a not-found message.
- Actual: `{"error":"Internal error"}`.
- Test: `missing_public_content_is_not_found_not_internal_error` in `crates/site-server/tests/qa_baseline.rs`. The run fails on the forum URL first. Wiki and article were confirmed with curl on the live server.

## Follow-up confirmations

A second pass re-read public reads, authorization, and validation. The roster leak (bug 1) and the forum `LIMIT` hole (bugs 2 and 6) are the same defects already listed. These additional ones failed under `--ignored`.

### 9. Medium — any member can read a deactivated profile by id

`GET /api/members` omits inactive rows unless the caller is officer+ and passes `include_inactive=true`. `GET /api/members/{id}` returns `get_member_safe` with no `is_active` check (`crates/site-server/src/routes/members.rs`). The public profile 404s the same id.

- Expected: a recruit receives `404`.
- Actual: `200` with bio, role, and `is_active: false`.
- Test: `recruit_cannot_read_deactivated_member_by_id` in `crates/site-server/tests/qa_baseline.rs`.

### 10. Medium — a deactivated team stays public by id

`list_teams` is `WHERE is_active = true`. `get_team` is a raw select, and `public_team_detail` 404s only when the row is missing (`crates/site-server/src/routes/public.rs`, `crates/db/src/queries/teams.rs`).

- Expected: `GET /api/public/teams/alpha` is `404` after `is_active = false`. Overview already omits the team.
- Actual: `200` with `"is_active": false`, roster, and record.
- Test: `inactive_team_is_absent_from_public_detail`.

### 11. Medium — the public win/loss record counts private scrims

`recent_matches` on the public team page keeps `is_public` and drops scrims. `get_team_record` counts every completed `match_result` for the team (`crates/db/src/queries/matches.rs`).

- Expected: one public official win and one private scrim loss yield `wins: 1`, `losses: 0`.
- Actual: `wins: 1`, `losses: 1`. The match list in the same body contains only the official win.
- Test: `public_record_ignores_private_and_scrim_results`.

### 12. Medium — deactivating a forum board does not hide its threads

The tree and slug lookup require `forum_board.is_active = true`. `list_threads` / `get_thread` load the board with a raw select and then only enforce `min_role` (`crates/site-server/src/routes/forum.rs`, `crates/db/src/queries/forum.rs`).

- Expected: after `is_active = false`, `GET /api/forum/threads/{id}` is `404` and the unfiltered list omits the thread.
- Actual: `200` with the title and content, and the board object in that body has `"is_active": false`.
- Test: `deactivated_forum_board_hides_its_threads`.

### 13. Medium — accepting an application clears a ban's deactivation

A ban sets `is_active` false. `submit_application` allows the user through because they are not active. `ensure_member_for_application` then sets `is_active` true with no moderation check (`crates/site-server/src/routes/applications.rs`). The public profile treats `is_active` as the gate, so the banned member's bio is public again. `OrgMember` routes still 403 while the ban row exists. Lift is documented to leave the member inactive.

- Expected: `GET /api/public/members/{id}` stays `404` after the application is accepted.
- Actual: `200` with `org_role: recruit` and the bio.
- Test: `accepting_application_does_not_reactivate_a_ban`.

### 14. Medium — login does not use the register username rules

`validate_local_username` (1–32 characters, `[A-Za-z0-9_-]`) is used by register and setup. `local_login` trims, lowercases, and looks the string up (`crates/site-server/src/routes/auth.rs`). A miss still runs the dummy Argon2 verify and stores the string in the lockout map.

- Expected: a 33-character username is `400`.
- Actual: `401` `{"error":"invalid username or password"}`.
- Test: `login_rejects_usernames_register_would_reject`.

### 15. High — replacing an avatar can delete another member's upload

`PUT /api/members/{id}` stores `avatar_url` with no check that the path belongs to that member. The next `POST /api/upload/avatar` deletes the previous value when it starts with `/uploads/` and has no `.` or `..` segment (`crates/site-server/src/routes/uploads.rs` `delete_local_upload`).

- Expected: `images/victim/secret.png` is still on disk after the attacker uploads a new avatar.
- Actual: the file is removed.
- Test: `avatar_replace_does_not_delete_another_members_file`.

### 16. Medium — event `time` is not a clock time

Create rejects control characters in `time` and then stores the string. The public ICS builder does `hour * 60 + minute + duration` in `u32` (`crates/site-server/src/calendar.rs`). `71582789:00` overflows `u32`. Debug builds panic on that overflow. Release builds wrap.

- Expected: `POST /api/events` with that time is `400`.
- Actual: `201` and the row stores `"time": "71582789:00"`.
- Test: `event_time_must_be_a_clock_time`. The test stops at create so the ICS handler is not invoked.

### 17. Medium — login lockout fails open when the map is full

`record_failure_at` returns without inserting once 8192 non-idle usernames are tracked (`crates/site-server/src/login_lockout.rs`). A username that was not already in the map never locks. The per-IP auth governor still applies.

- Expected: five failures on a new username lock it.
- Actual: `retry_after_at` stays empty.
- Test: `full_map_still_locks_a_new_username` in `crates/site-server/src/login_lockout.rs`.

### 18. Medium — `X-Real-IP` selects the rate-limit bucket when `X-Forwarded-For` is absent

From a trusted peer, a parsed `X-Forwarded-For` wins. If that header is missing, `X-Real-IP` is the key (`crates/site-server/src/rate_limit.rs`). Caddy sets `X-Forwarded-For`, so browser traffic through the published proxy does not hit this. A client that reaches the process with a trusted peer and no `X-Forwarded-For` can rotate buckets.

- Expected: peer `127.0.0.1` plus `X-Real-IP: 203.0.113.50` and no `X-Forwarded-For` keys the bucket as `127.0.0.1`.
- Actual: the key is `203.0.113.50`.
- Test: `trusted_peer_without_xff_ignores_x_real_ip` in `crates/site-server/src/rate_limit.rs`.

## Confirmed in source, no separate regression

These match the code. A failing test was not added because the handler returns before the interesting branch without a relay, or the check needs an open WebSocket.

- **High.** `GET /api/strategy/ws` compares `global_connection_count()` to the cap before upgrade, and that counter increments only inside `join_room` (`crates/server/src/routes/ws.rs`, `crates/server/src/collab/room.rs`). A socket that never joins, including an anonymous `JoinRoom`, is not counted. `Ping` resets the idle timer.
- **Medium.** `GET /api/nostr/feed` treats the caller as an officer from role alone. It does not use `OrgMember`, so `is_active` and an active suspension are ignored. If `list_officer_group_ids` errors, `unwrap_or_default` makes the officer-group set empty, and an empty set skips the `h`-tag filter (`crates/site-server/src/routes/nostr.rs`).
- **Medium.** `POST /api/nostr/post` copies the client `group_id` into the `h` tag with no role check. The feed treats those tags as officer-only. The handler returns `400` until the member has a server-managed key, and `503` until a relay is configured, so this pass did not publish an event.
- **Known gap.** `POST /api/nostr/export-backup` wraps the server-held secret with the password in the body. It does not require the account password. The handler comment already calls this out as a step-up reauth follow-up.

## Not filed as defects

- Auth rate limit works. After the burst, `POST /api/auth/local/login` returned `429`.
- CORS does not reflect `https://evil.example`. `http://localhost:3000` is allowed. `access-control-allow-credentials` is true.
- The authenticated member list does not include `nostr_secret_key_encrypted`. Public settings on overview are org and brand fields.
- Axum JSON extractor failures are `text/plain` (`400` for malformed JSON, `422` for a missing field) rather than the `{error}` JSON envelope used by handlers. Observed, no regression added.
- `role_meets_min` treats an unrecognized `min_role` as unrestricted for a logged-in caller. The function comment says that is the rule, and the HTTP board API does not write the column.
- Anonymous tournament match and bracket payloads include `notes` and `replay_codes`. Public org-match detail strips `notes`. Bracket notes may be public commentary, so this was not filed.
- `GET /api/nostr/health` is unauthenticated and includes `relay_url`. `extra_relay_urls` and `forum_backend` are already on anonymous `GET /api/settings`.
- A garbage `cursor` on `GET /api/announcements` returned `200` and an empty page. Not chased further.
- `GET /api/strategy/meta` anonymous body has an empty `heroes` array and no `personal` block. Personal stats use the cookie lookup from bug 3. Not given a separate test.
- Nostr publish, DM sync, and chat decrypt were not driven against a relay. The live process had no `ENCRYPTION_KEY` and no `NOSTR_RELAY_URL`.
