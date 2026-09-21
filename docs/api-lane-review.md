# API / backend lane review

**Reviewer:** Cursor cloud agent (review-only)  
**Repo:** https://github.com/FrozenTear/scuffed-crew  
**SHA reviewed:** `e9fbe432b26cca57c9c4fe16f0cd1aa342289fa8` (`origin/main`, 2026-09-02)  
**Scope:** Axum (`scuffed-server`, `site-server`), SurrealDB (`crates/db`), auth/sessions, Nostr chat backend, deploy (`Containerfile`, `compose.yml`, `scripts/install.sh`, `docs/deploy.md`).  
**Out of scope:** Dioxus `crates/app`, OCR `crates/stat-tracker` (except where they consume these APIs).  
**No merges. No functional patches.**

**Post-review status (2026-09-02):** After this review SHA, merged [PR #50](https://github.com/FrozenTear/scuffed-crew/pull/50) addressed **F-API-001** (forum `min_role` list leak + fail-closed thread/reply ACL) and **F-API-002** (first-boot `/api/auth/setup` TOCTOU via `bootstrap_lock` CAS). Those two findings below are historical; they stay in the doc but are **resolved on `main`**. No other findings were re-audited in this rebase.

---

## Verdict (Evidence Law)

| | |
|---|---|
| **READ** | Router/auth/crypto/session/DB client/migrations, extractors, membership policy, chat/WS/Nostr, uploads, rate limits, compose/Containerfile/install/deploy docs, at SHA above. |
| **RAN** | `git rev-parse`, `git fetch origin main`, targeted ripgrep. No live HTTP, no cargo test/clippy. |
| **COMPILED** | Nothing. |
| **DID NOT CHECK** | Live `:3030`, production VPS, runtime Surreal race of concurrent `/api/auth/setup`, whether any live org actually uses restricted forum boards or the strategy WS, GHCR image contents vs this SHA. |

A finding that only type-checked is not a finding. Every item below points at a path/symbol on this SHA.

---

## 1. Executive summary

The production API is in much better shape than a greenfield Axum+Surreal app: session tokens are hashed, OAuth CSRF is constant-time, rate-limit keys refuse spoofed `X-Forwarded-For`, membership transitions are CAS'd, last-admin is policy-enforced, remote DB refuses to boot without `PRODUCTION` + `ENCRYPTION_KEY` + a distinct scoped app password, and several prior DR1/NS2 items (NIP-05 omit-unvalidated-domain, `_` not wildcard, public governor widening, `plays_on` indexes, B5 error hygiene on chat) are actually in the tree.

The remaining holes that can bite a real deploy are: (1) forum `min_role` is still bypassable via the unfiltered thread list (the "A1/H4 fixed" claim is incomplete), (2) first-boot `/api/auth/setup` is check-then-act and can mint two admins, (3) officer encrypted chat cannot run because `provision_team_channels` has zero callers, (4) compose injects empty `ALLOWED_ORIGINS` and that disables the `REDIRECT_BASE_URL` fallback used by strategy WebSocket origin checks, (5) many unauthenticated GETs (wiki, articles, tournaments including drafts, forum, settings, …) sit outside every governor, and (6) `/api/health` plus the Surreal `version` probe cannot tell you the stack is actually serving data.

Nothing in this pass is a "skip the next deploy" Critical on a correctly-run `install.sh` box with no restricted forum boards. The High items should be the next API tickets.

---

## 2. Severity-ranked findings

### High

#### F-API-001 — Forum `min_role` bypass via unfiltered thread list — **RESOLVED on main via #50**
- **Where:** `crates/site-server/src/routes/forum.rs` `list_threads` (`:401–449`); `crates/db/src/queries/forum.rs` `list_forum_threads` (`:631–668`).
- **What:** `forum_tree` and `get_board` hide/enforce `min_role`. `get_thread` also calls `enforce_board_access` when a board row exists. **`GET /api/forum/threads` with no `board` (or with only `category`) does not.** The DB path is `SELECT * FROM forum_thread WHERE is_active = true …` — every active thread, including officer/admin boards. Title + body metadata of the thread row come back to anonymous callers.
- **Why it matters:** `docs/security-quality-review-fix-list.md` marks A1/H4 "forum `min_role` never enforced" as **fixed**. The tree is fixed; the list is not. Anyone who can hit the API learns officer-board thread titles (and whatever fields `ForumThread` serializes) without logging in.
- **Residual:** `get_thread` (`forum.rs:468–485`) skips the ACL when `board_id` is missing or `get_forum_board` returns `Ok(None)` (dangling/legacy row) and still returns replies.
- **Fix direction:** After fetch, drop threads whose board fails `enforce_board_access`. Never serve a thread whose board cannot be resolved. Add a test: restricted board + anonymous `GET /api/forum/threads` returns zero of those rows.
- **Confidence:** high (code path is unconditional).

#### F-API-002 — First-boot `/api/auth/setup` is TOCTOU — **RESOLVED on main via #50**
- **Where:** `crates/site-server/src/routes/auth.rs` `setup` (`:490–580`); `crates/db/src/queries/members.rs` `has_any_member` (`:610–623`) / `create_member` (`:115–158`).
- **What:** Gate is `has_any_member() == false`, then `create_local_user` + `create_member(..., OrgRole::Admin)`. There is no CAS / single-row lock / "only while member count is 0" write. `member_user_idx` is UNIQUE on `user_id`, not "at most one bootstrap admin".
- **Why it matters:** Two concurrent POSTs with different usernames both observe an empty member table and both become admins. The window is first-boot only (members are never hard-deleted, so the endpoint stays closed after the first successful create) — but that is exactly the window a newly-exposed VPS is on the public internet waiting for the operator to open a browser.
- **Fix direction:** One-row bootstrap lock (`CREATE ONLY` a sentinel, or `CREATE member …` under a unique constraint that fails the loser with 409), or serialize setup on a process mutex *and* a DB unique. Do not re-open this endpoint when all admins are suspended (already correct — keep `has_any_member`).
- **Confidence:** high on the race; exploit requires winning the first-boot window.

#### F-API-003 — Officer encrypted chat cannot execute (dead provisioning)
- **Where:** `crates/chat/src/provisioning.rs` `provision_team_channels` (only definition); `crates/server/src/routes/chat.rs` `send_encrypted` (`:241–262`) reads `team_channel` by `group_id`. Grep across `crates/`: **zero call sites** besides the export in `chat/src/lib.rs:23`.
- **What:** Night-shift backlog NS2-7 already recorded this. Still true on this SHA. Nothing creates `team_channel` rows, so `send_encrypted` 404s at channel lookup. `TODO(Phase 2c)` on `chat.rs:337` is still there.
- **Why it matters:** The HTTP surface advertises a working officer gift-wrap path. Relays, NIP-29 groups, and client UI that assume channels exist will fail in production. Optimizing relay connection reuse (NS2-7b) remains wasted until this is wired.
- **Fix direction:** Call `provision_team_channels` from team create/update (and a one-shot backfill). Decide whether team chat is Phase-2 unfinished or a lost refactor — do not build more on `team_channel` until that call exists.
- **Confidence:** high.

#### F-API-004 — Empty `ALLOWED_ORIGINS` disables CORS/WS fallback
- **Where:** `compose.yml:77` `ALLOWED_ORIGINS: ${ALLOWED_ORIGINS:-}`; `crates/site-server/src/state.rs` `OAuthConfig::from_env` (`:312–314`); `crates/server/src/routes/ws.rs` `ws_origin_allowed` (`:61–66`).
- **What:** Fallback to `redirect_base_url` only runs when the env var is **unset** (`Err`). Compose always *sets* the var. Empty string → `allowed_origins = [""]`. Browser `Origin: https://ow.scuffedcrew.no` does not match `""` → strategy WS returns **403**. Fresh `install.sh` writes `ALLOWED_ORIGINS=${REDIRECT_BASE_URL}` (`scripts/install.sh:88`). The harden-existing-secrets block (`:114–129`) does **not** append `ALLOWED_ORIGINS`.
- **Why it matters:** Same-origin XHR still works (CORS is not required). The strategy editor WebSocket does not: it is origin-checked. An older `secrets.env` upgraded in place, or a compose-only bring-up, silently breaks collab.
- **Fix direction:** Treat blank `ALLOWED_ORIGINS` as unset (fall back to `REDIRECT_BASE_URL`). Have `install.sh` / `update.sh` `ensure_secret_key ALLOWED_ORIGINS`. Do not treat `Ok("")` as a configured allow-list.
- **Confidence:** high on the code; impact is "strategy WS from a browser" on deploys missing the key.

---

### Medium

#### F-API-005 — Unauthenticated GETs still outside every governor (NS2-6 incomplete)
- **Where:** `crates/site-server/src/lib.rs` `public_routes` (`:123–173`) covers `/api/public/*`, ICS, `/.well-known/nostr.json`, setup-status, providers. **Not** covered, and handlers take no auth extractor:
  - `/api/settings` (`settings.rs:91`)
  - `/api/wiki`, `/api/wiki/{topic}`, `/api/wiki/{topic}/revisions`
  - `/api/articles`, `/api/articles/{slug}`
  - `/api/tournaments`, `/api/tournaments/{id}` (+ bracket/standings/matches/participants reads)
  - `/api/teams`, `/api/teams/{id}`, `/api/teams/{id}/roster`
  - `/api/games`, `/api/games/{id}`
  - `/api/events` (anonymous → public events only)
  - `/api/forum/tree`, boards, threads
  - `/api/teams/{id}/matches` (anonymous → public rows)
- **Why:** NS2-6 claimed "every remaining unauthenticated route". The comment at `lib.rs:108–114` is honest about sharing one bucket, but the merge list is not complete. Same amplification class as the public governor.
- **Fix:** Fold these into `public_routes` (or a second shared group). Keep `/api/health` ungoverned (already documented).
- **Confidence:** high.

#### F-API-006 — Tournament drafts are publicly enumerable
- **Where:** `crates/site-server/src/routes/tournaments.rs` `list_tournaments` (`:75–98`) — no auth. Query `status=draft` is accepted (`:80`). `list_tournaments_paginated` with `status=None` returns all statuses.
- **Why:** Internal names, rules, dates of unpublished cups leak. Public site already has `/api/public/*` for the intended public surface.
- **Fix:** Default-filter out `draft` (and maybe `archived`) for anonymous callers; require `OfficerUser` to list drafts.
- **Confidence:** high.

#### F-API-007 — Health is liveness-only; Surreal "health" is `version`
- **Where:** `crates/site-server/src/routes/health.rs` — `StatusCode::OK`, no DB. `Containerfile:41–42` HEALTHCHECK curls that. `compose.yml:21–26` Surreal probe is `CMD /surreal version`. `depends_on` is start-not-ready (`:59–60`). `docs/deploy.md:100–102` says compose "does **not** healthcheck Surreal" — **stale vs current compose.yml**.
- **Why:** Orchestrator can mark the app healthy while Surreal is down or migrations have not finished. `version` succeeds if the binary is on disk, not if RocksDB opened. Container `start-period=10s` is tight if bootstrap DEFINE + migrate is slow.
- **Fix:** Split `/api/health` (liveness, no DB) vs `/api/ready` (SELECT 1 / `INFO FOR DB` with a short timeout). Point the container HEALTHCHECK at ready, or keep liveness and add a compose readiness probe. Replace Surreal `version` with a real query or drop the false-green check. Update `docs/deploy.md`.
- **Confidence:** high.

#### F-API-008 — `PRODUCTION` is parsed three different ways
- **Where:**
  - `crates/auth/src/env_flags.rs` `is_production_env` — empty/0/false/no/off = off; any other non-empty = on.
  - `crates/server/src/main.rs:53` HSTS: `std::env::var("PRODUCTION").is_ok()` — **empty string is on**.
  - `crates/server/src/routes/ws.rs:69–73` `is_production` — only `1/true/TRUE/yes/YES`. `PRODUCTION=on` (which `is_production_env` accepts) does **not** require Origin on WS.
- **Why:** Cookie Secure, HSTS, remote-DB policy, and WS Origin use different classifiers. `compose.yml:101` defaults `PRODUCTION` to empty; `install.sh` writes `PRODUCTION=1` so the blessed path is fine. Manual/compose-only deploys hit the seams.
- **Fix:** One helper (`is_production_env`) everywhere, including HSTS and WS.
- **Confidence:** high on inconsistency; impact depends on env spelling.

#### F-API-009 — Strategy WS broadcasts before persist
- **Where:** `crates/server/src/routes/ws.rs` element/phase handlers (`:296–311` and siblings). `try_spawn_persist` fires a background write; `broadcast` runs immediately.
- **Why:** Peers see edits that can fail to land. Rejoin reloads from DB and the room diverges. Not a privilege bug (`can_edit_strategy` is checked) — integrity.
- **Fix:** Persist-then-broadcast, or broadcast a tentative event and confirm/rollback. At minimum surface persist failure to the originating socket.
- **Confidence:** high.

#### F-API-010 — `/api/chat/auth-token` signs NIP-42 AUTH for any `relay_url`
- **Where:** `crates/server/src/routes/chat.rs` `provision_auth_token` (`:81–84`, `:123–126`). Client supplies `relay_url` + `challenge`.
- **Why:** Any org member with server-managed keys can obtain a server-signed AUTH event for an attacker-controlled relay, proving control of the org identity to that relay. Bound by `OrgMember` + encryption required.
- **Fix:** Allow-list `state.relay_url` + `site_settings.extra_relay_urls`. Reject others.
- **Confidence:** high on the behavior; Medium because the caller is already a member.

#### F-API-011 — DM handlers still interpolate `{e}` to clients
- **Where:** `crates/site-server/src/routes/nostr.rs` `dm_send` `:1626` (`Failed to build gift wrap: {e}`), `:1673` (`Failed to store sent message: {e}`). Chat routes were cleaned (NS2-7a / B5); this file was not.
- **Why:** Internal crypto/DB text on a member-facing API.
- **Fix:** Mirror `internal_err` / generic client string; keep detail in `tracing`.
- **Confidence:** high.

#### F-API-012 — Daemon stats upload is unthrottled and unbounded
- **Where:** `POST /api/stats/upload` (`stats.rs:22`) uses `DaemonUser`, not the upload `GovernorLayer` (`lib.rs:99–102`). `upsert_personal_matches` (`personal_stats.rs:93–99`) loops one UPSERT per match with no batch cap. `scuffed-server` body limit is 10 MB (`server/src/main.rs:250`).
- **Why:** A stolen daemon token (no expiry — F-API-017) can write an arbitrary number of rows per request and retry freely. Different threat from the image-upload disk fill (DR1-ADMIN-001).
- **Fix:** Cap `body.matches.len()` (e.g. 200). Put the route on a governor (IP or token-hash). Consider token TTL / last-used alerts.
- **Confidence:** high.

#### F-API-013 — NIP-05 still scans up to 2000 members per request
- **Where:** `nostr.rs` `nostr_json` (`:109`); `members.rs` `list_nostr_identities` (`:456–460`, `LIMIT 2000`). NS2-4a fixed `_` / empty-name enumeration; the scan remains (accepted in the backlog if NS2-6 covers the route — it does).
- **Why:** Cost, not disclosure. Fine at current org size; still the hottest anonymous scan.
- **Fix:** Stored `nip05_name` + index (NS2-4b), or cache the map. Keep the governor.
- **Confidence:** high.

#### F-API-014 — Relay profile: root DB creds; policy plugin not in compose
- **Where:** `compose.yml` `strfry` (`:29–49`) gets `SURREALDB_USER/PASSWORD` root. `crates/relay-policy` is a standalone stdin plugin (`relay-policy/src/main.rs`); not referenced by `compose.yml` / `relay/Containerfile` in this pass. CLAUDE.md: "not yet in deploy".
- **Why:** Compromised strfry = root Surreal. Without the policy plugin, the relay is not enforcing the member-pubkey allowlist this repo implemented.
- **Fix:** Give strfry a read-only/scoped user. Wire `relay-policy` into the relay image if the relay profile is going to production. Until then, treat `--profile relay` as incomplete.
- **Confidence:** high on creds; medium on "is the plugin supposed to be live yet" (docs say future).

#### F-API-015 — No readiness, metrics, or request IDs
- **Where:** `health.rs`; `server/src/main.rs` tracing is `EnvFilter` + fmt only; no Prometheus/OTLP; `TraceLayer` without a request-id propagator.
- **Why:** You cannot distinguish "process up" from "DB serving" (F-API-007) or follow a 500 across logs.
- **Fix:** `/api/ready`, a request-id middleware, and one metrics endpoint (even a tiny `metrics-exporter-prometheus`) before the next production fire.
- **Confidence:** high.

---

### Low

#### F-API-016 — No Content-Security-Policy
- **Where:** `crates/server/src/main.rs` `security_headers` (`:22–60`) sets nosniff, DENY frame, XSS=0, referrer, permissions, conditional HSTS. No CSP. `site-server` binary has **none** of these (prod image runs `scuffed-server` — `Containerfile:23,36`).
- **Fix:** Conservative CSP for the SPA (`default-src 'self'`, wasm/script allowances as needed). Ignore `site-server` or delete the unused binary entrypoint if it is no longer shipped.

#### F-API-017 — Daemon tokens never expire
- **Where:** `migrations.rs:667–676` — `is_active`, no `expires_at`. `validate_daemon_token` only checks `is_active`.
- **Fix:** Optional TTL + rotate UX. Revoke-on-password-change / ban already exists via member checks on the extractor.

#### F-API-018 — CSRF for cookie POSTs is SameSite=Lax only
- **Where:** `crates/auth/src/server/session.rs` `build_session_cookie` — `HttpOnly`, `Secure` (prod/release), `SameSite=Lax`. OAuth state has a real CSRF cookie + constant-time compare. No synchronizer token on `/api/*` mutations.
- **Why:** Adequate for a first-party SPA on a single site. Not adequate if a second site is ever added to `ALLOWED_ORIGINS` as a full peer, or if a browser ignores Lax.
- **Fix:** Stay Lax; add `__Host-` prefix + `Path=/` (already `/`). If you add a second origin, add double-submit or custom header.

#### F-API-019 — Per-process challenge + Nostr rate-limit stores
- **Where:** `challenge_store.rs` (documented multi-instance caveat); `nostr_rate_limit.rs` same. Fine for single-container install.sh. Replay of a captured kind-22242 within the freshness window can succeed on another replica.
- **Fix:** Only if you scale out: unique `consumed_challenge` row or Redis.

#### F-API-020 — `NIP05_DOMAIN` not hardened into existing secrets
- **Where:** `install.sh` first-run does not write `NIP05_DOMAIN`. NS2-3a correctly omits `nip05` when unset (`state.rs` `nip05_domain_from_env`). Kind-0 events on a public deploy still ship without a verifiable identity until the operator sets the domain (NS2-3b, USER-gated).
- **Fix:** Prompt in install; document on the first-boot admin screen.

#### F-API-021 — `BOOTSTRAP_ADMIN_*` stays in the container env
- **Where:** `server/src/main.rs:113–137` applies reset + session revoke (DR1-AUTH-003 — good), then only logs "remove from env". Compose keeps passing the vars (`compose.yml:103–105`).
- **Fix:** Fail boot if the flag is still set after a successful apply, or require a one-shot migrate-style job.

#### F-API-022 — `DEFINE USER` interpolates the app password
- **Where:** `crates/db/src/client.rs` `ensure_database_app_user` (`:344–351`). Documented: escape `\` and `'`; no control characters. `install.sh` uses `openssl rand -base64 32` (safe).
- **Fix:** Keep the documented charset. If Surreal ever accepts a bound password, switch.

#### F-API-023 — Capped member-name maps (NS2-9, logged)
- **Where:** `list_members_paginated(500, 0)` used as a name dictionary. Past 500 members, admin lists silently lose names. Not a tonight bug; do not "fix" with a full-table load.
- **Fix:** Page-scoped join (`enrich_audit_actor_names` pattern) when a fourth copy appears.

#### F-API-024 — Wiki + revisions are fully public
- **Where:** `wiki.rs:33–76, 189–221` — comments say "public". Revisions include prior markdown.
- **Why:** Fine if the wiki is a public knowledge base. Wrong if anyone ever treats it as officer docs.
- **Fix:** Product call. If internal pages exist, add `min_role` like forum boards.

---

### Info

- `/uploads` is unauthenticated by design (`lib.rs:569`). Upload path sniffs magic bytes (`uploads.rs:48–64`) and UUID-names files — good. No directory listing assumed (ServeDir).
- Cookie auth + Bearer session tokens share `get_session_user` (`extractor.rs:56–76`). Daemon tokens are a separate table/extractor. Good split.
- `OptionalOrgMember` maps every `OrgMember` error to `None` (`extractors.rs:97–100`). Fail-closed for private data (anonymous sees only public events/matches). A 500 on member lookup looks like "logged out" — acceptable.
- `scuffed-server` binds `0.0.0.0:3000` in-container; compose publishes `127.0.0.1:${HOST_PORT}` only. Blessed path is not internet-direct.
- Chat `send_encrypted` still returns `format!("Channel '{}' not found")` (`chat.rs:259`) — existence oracle for group ids, Low/Info given channels are unprovisioned.

---

## 3. Production blockers vs nice-to-haves

**Blockers before you treat the API as "done" for a public hostname**

1. **F-API-001** — **resolved on `main` via #50.** Historical: close the forum list leak if any board has `min_role` (or you plan to add one).
2. **F-API-002** — **resolved on `main` via #50.** Historical: CAS/lock setup before the next fresh install on a public IP.
3. **F-API-004** — blank `ALLOWED_ORIGINS` handling + harden existing `secrets.env`, or strategy WS is a landmine.
4. **F-API-007 + F-API-015** — readiness that touches Surreal; fix the stale deploy.md sentence. You cannot operate what you cannot probe.
5. **F-API-003** — either wire channel provisioning or hide/disable the chat encrypt routes so the contract matches reality.
6. Confirm `NIP05_DOMAIN` / `ALLOWED_ORIGINS` / `PRODUCTION=1` on the live box (install.sh does the last; the first two are easy to miss).

**Not blockers (do next, do not stall a ship)**

- F-API-005/006 governors + draft visibility  
- F-API-008 unify `PRODUCTION`  
- F-API-009 persist-then-broadcast  
- F-API-010 relay allow-list on AUTH  
- F-API-011 leftover `{e}` on DM  
- F-API-012/017 daemon upload cap + TTL  
- F-API-013 NIP-05 indexed lookup  
- F-API-014 relay-policy + non-root strfry  
- F-API-016 CSP  
- F-API-018–024 hygiene  

**Already closed on this SHA (do not redo)**

NS2-3a omit invalid NIP-05 · NS2-4a `_` is not a wildcard · NS2-5 `get_member_teams` + `plays_on` indexes · NS2-6 governor on ICS/well-known/setup-status · DR1-AUTH-001 trusted-proxy keys · DR1-AUTH-002 dummy Argon2 · DR1-AUTH-003 bootstrap session revoke · DR1-ACCT-003 setup gated on `has_any_member` · B5 chat error hygiene (chat.rs only) · `DEFINE USER OVERWRITE` idempotent bootstrap · `SURREALDB_APP_PASSWORD` must differ from root in prod.

---

## 4. Notable strengths worth keeping

- **Fail-closed remote boot:** `assert_remote_production_policy` + required `ENCRYPTION_KEY` + scoped EDITOR app user + no `root/root` in prod (`crates/db/src/client.rs`). `NOSTR_CHALLENGE_SECRET` panic outside dev (`server/src/main.rs:155–166`).
- **Session model:** BLAKE3-hashed tokens (`auth/src/crypto.rs`), unique index, per-user cap 10, hourly cleanup, revoke on ban/deactivate/password reset.
- **Authn/authz extractors:** `OrgMember` / `OfficerUser` / `AdminUser` / `DaemonUser` fail closed on DB errors; suspended/inactive rejected (`extractors.rs`).
- **Membership policy as a pure module** (`membership_policy.rs`) + CAS application status (`update_application_status`) + last-admin + "destructive side effects after CAS" (CLAUDE.md / DR1-ACCT-001). Keep this shape.
- **Rate-limit spoof resistance:** `TrustedProxyIpKeyExtractor` right-to-left XFF, tests included (`rate_limit.rs`).
- **Crypto:** AES-256-GCM + AAD domain separation + keyring + encryption_required aligned with remote DB (`crypto.rs` DR1-AUTH-004).
- **Uploads:** magic-byte sniff, declared-type mismatch reject, UUID names, dedicated governor.
- **NIP-05:** validate/reject loopback and `scuffed.gg`-class mistakes; republish is admin + `NIP05_REPUBLISH_ENABLED=1` + dry-run unless `confirm`.
- **Nostr secret hygiene:** list projections omit `nostr_secret_key_encrypted`; full row loaded only for signing (`chat.rs` `load_member_with_secret`).
- **Deploy defaults:** loopback publish, `secrets.env` mode 600, distinct app password, `PRODUCTION=1` from install.sh, image from GHCR rather than compile-on-VPS.
- **Security headers on the unified binary** (minus CSP) and HSTS only when production-ish.

---

## Suggested ticket order (no patches in this PR)

1. F-API-001 forum list ACL + test — **done on `main` via #50**  
2. F-API-002 setup CAS — **done on `main` via #50**  
3. F-API-004 blank-origins = unset  
4. F-API-007 `/api/ready` + deploy.md  
5. F-API-003 provision or hide chat encrypt  
6. F-API-005/006 public governor + draft filter  
7. The Medium leftovers as a hygiene batch  

---

## Protocol note

Cloud-agent git instructions say `git checkout -b` in the shared checkout. Repo law (`docs/agent-protocol.md` IRON LAW) forbids that. This review lives on `cursor/api-lane-review-ac68` via `git worktree add .claude/worktrees/cursor-api-lane-review`. Shared `main` HEAD was not moved. No merge.
