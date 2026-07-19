# DR1 P1 — ADMIN lane findings

Reviewer: claude (Opus) · Date: 2026-07-19 · Scope: crates/site-server admin REST
(EXCL applications/members-role/ban [ACCT] and extractor internals [AUTH]).

Format: ID | SEVERITY | file:line | claim | failure scenario | fix direction | CONFIRMED/SUSPECTED

---

DR1-ADMIN-001 | HIGH | routes/uploads.rs:23-140 + uploads.rs:70-101 + lib.rs:451-452 |
Avatar/image uploads have no per-user quota, no cleanup of replaced files, and no
rate limit → disk-exhaustion DoS by any authenticated member. |
`POST /api/upload/avatar` is gated only by `OrgMember` (recruit+). Each call writes a
new `{uuid}.{ext}` file (uploads.rs:95) and NOTHING deletes the previous one when a
member changes avatar; the GovernorLayer rate limiter is attached only to `auth_routes`
(lib.rs:76) so upload routes are unthrottled. A single accepted member can loop
2 MB writes and fill the disk / inode table, taking the site down (DB + SPA served
from same host). Files are never GC'd even in normal use → unbounded growth. |
Add a per-member upload rate limit (governor or DB token bucket) + a max-files/quota
per member, delete the prior avatar on replace, and consider a reaper for orphaned
uploads. | CONFIRMED

DR1-ADMIN-002 | MED | routes/teams.rs:77 (create=AdminUser) vs :123 (update=OfficerUser) |
Team create requires Admin but update requires only Officer — privilege asymmetry;
plus there is no DELETE route at all. |
An Officer cannot create or remove a team but CAN rename it, change its `game_id`,
color, division, and lore. The gating tier for "structural team object" is inconsistent
with `games.rs` (both create+update = Admin). An officer could repurpose an existing
team (rename + swap game) to route around the admin-only create gate. |
Pick one tier for the team object lifecycle — most consistent with games.rs is
`AdminUser` on both create and update (and add an admin DELETE). | CONFIRMED

DR1-ADMIN-003 | MED | uploads.rs:70-101 |
`save_upload` sniffs magic bytes (good) but stores the raw client bytes with no
re-encode and no pixel/dimension cap → pixel-flood / decompression-bomb served to
clients. |
The 2 MB/5 MB caps bound the on-disk byte size but NOT decoded dimensions: a ~few-hundred-KB
PNG/GIF/WebP can declare a gigapixel canvas. The server never decodes (so the server
itself is safe), but it stores and serves the raw bytes via `ServeDir` (lib.rs:496);
any browser/thumbnailer that renders the avatar can be forced to allocate huge bitmaps
(client-side memory DoS). No server-side transcode means a crafted file also keeps its
original (possibly non-conforming) payload. |
Decode + re-encode server-side (e.g. `image` crate with a max width/height and
`limits()` for pixel budget) before persisting; reject on decode failure or dimension
overflow. | CONFIRMED

DR1-ADMIN-004 | MED | routes/forum.rs:261-356 |
Forum structural mutations (create/update category, create/update board) fire NO
audit() — only pin/lock are audited. |
`create_category`, `update_category`, `create_board`, `update_board` are Officer-level
changes to forum structure (including `is_locked`/`is_active` toggles on boards and
`min_role` visibility gating downstream) yet leave no audit trail. CLAUDE.md requires
fire-and-forget audit after successful mutations. An officer restructuring/hiding boards
is invisible in `/api/audit-log`. |
Add `audit(...)` after each successful forum category/board create+update, mirroring
pin/lock. | CONFIRMED

DR1-ADMIN-005 | MED | routes/tournaments.rs:576-594 |
`report_match` does not validate that `winner_id` is one of the match's two
participants before advancing it. | SUSPECTED (db-layer `report_tournament_match` may
guard; not read — DB lane). The route takes `body.winner_id: String` and immediately
`set_match_participant(next_id, next_slot, &body.winner_id)` (line 597-603) to seed the
next round. If an officer supplies a `winner_id` that is neither `participant_a_id` nor
`participant_b_id` (typo or arbitrary id), the bracket advances a non-participant, and
the loser-elimination branch (610-647) then treats participant_a as the loser. Corrupts
tournament state. |
Validate `winner_id ∈ {participant_a_id, participant_b_id}` in the handler (or confirm
the db query rejects it) and return 400 otherwise. | SUSPECTED

DR1-ADMIN-006 | LOW | routes/uploads.rs:23-140 |
Neither `upload_avatar` nor `upload_image` fires audit(). |
File writes to disk (officer image uploads especially) leave no audit record of who
uploaded what. Lower value than config mutations but still a coverage hole flagged by
convention. |
Audit both upload paths with actor id + resulting URL. | CONFIRMED

DR1-ADMIN-007 | LOW | routes/leaderboards.rs:189-228 |
`admin_create_season` (AdminUser) is unaudited. |
Creating a season — which reshapes public leaderboard windows and can flip
`is_current` — leaves no audit entry. |
Add audit() after `create_season`. | CONFIRMED

DR1-ADMIN-008 | LOW | routes/attendance.rs:28-59 |
`batch_mark_attendance` (Officer) writes a batch of attendance records with no audit. |
An officer can bulk-set/overwrite any members' attendance status for an occurrence with
no trail. Bulk mutation, accountability-relevant. |
Audit the batch (event id + occurrence + count), or per-entry. | CONFIRMED

DR1-ADMIN-009 | LOW | settings.rs:168-190, forum.rs:261-333/463-600, polls.rs:79-124, articles.rs:104-140, wiki.rs:86-120 |
No length/size caps on user-supplied string and collection fields across admin/officer/member
mutations. |
`org_name`, `site_description`, `recruitment_message`, `homepage_json`, `nav_json`,
`extra_relay_urls` (settings); forum category/board `name`/`description`, thread/reply
`title`/`content`; poll `title`/`description` and unbounded `options` count/length;
article `title`/`content_markdown`; wiki `content_markdown` — all passed through with
only `.trim().is_empty()` checks. A member/officer can store multi-MB strings (bounded
only by the global 6 MB body limit) or a poll with a huge option list → storage bloat and
slow reads. Rendering is Dioxus-escaped so XSS risk is low; this is a resource/robustness
gap. |
Add explicit max-length constants per field and a max option count; reject with 400. | CONFIRMED

DR1-ADMIN-010 | LOW | routes/uploads.rs:70-77 & 130-137, uploads.rs:7-32 |
Upload handlers map every `UploadError` to `400 "Internal error"`, discarding the
specific (helpful, already-authored) messages and mis-coding IoError. |
`save_upload` returns rich `Display` messages (too-large MB, invalid type), but the
handler does `map_err(|_e| (BAD_REQUEST, "Internal error"))`. A too-large file returns
400 with an unhelpful body; a filesystem `IoError` (server fault) is also reported as a
client 400 instead of 500. |
Match on `UploadError`: 413/400 with the real message for size/type, 500 for `IoError`. | CONFIRMED

DR1-ADMIN-011 | LOW | routes/forum.rs:513-520 |
`create_thread` decides FORBIDDEN-on-locked-board by substring-matching the db error
text (`msg.contains("locked")`). |
Fragile: if the db error wording changes, a locked-board post silently becomes a 500
instead of 403. Control-flow on error strings is brittle. |
Return a typed error (e.g. `DbError::Locked`) from the query and match on the variant. | CONFIRMED

DR1-ADMIN-012 | LOW | lib.rs:59-76, 92-504 |
Rate limiting (GovernorLayer) is applied ONLY to `auth_routes`; every other
mutation route (uploads, forum posts/replies, wiki, poll votes, scrims, rsvps, stats
upload) is unthrottled. |
Enables spam/abuse: avatar-upload disk fill (see -001), forum/wiki content flooding,
poll-vote hammering. The auth burst limit does not protect the app surface. |
Apply a global (or per-authenticated-user) governor layer to the main router, or add
targeted limits on write-heavy member routes. | CONFIRMED

DR1-ADMIN-013 | NIT | articles.rs:78-92, 162-175, 219-232, 266-279, 317-330 |
NotFound branches return the correct 404 status but with body `error: "Internal error"`. |
Misleading client message ("Internal error" for a 404). Cosmetic; no security impact. |
Use a "not found" message in the NotFound arm. | CONFIRMED

DR1-ADMIN-014 | NIT | routes/dev.rs:56-64 |
`dev_login` builds its session cookie without secure/same_site/max_age. |
Informational only: route is registered exclusively when `SURREALDB_URL` is unset
(lib.rs:87-90) AND the handler re-checks the same env (dev.rs:20-26) — double-gated, so
this cookie never exists in a real deploy. Confirms the double-gate holds; not a prod risk. |
None required; optionally set the flags for parity. | CONFIRMED (not a vuln)

DR1-ADMIN-015 | NIT | routes/scrims.rs:65-98, 101-139, 149-213 |
Recon's "scrims mutate at OrgMember(!)" concern is MITIGATED. |
`create_scrim`/`update_scrim_status` extract `OrgMember` but then call
`authorize_scrim_team` which requires Officer+ OR on-team-roster, returning 403
otherwise. Authz is correct; recorded to close the recon suspicion. Status validated to
a fixed allowlist (400 on invalid) and NotFound distinguished. | CONFIRMED (no bug)

---

## Audit-coverage summary (every admin/officer mutation vs audit())

AUDITED (good): teams create/update, games create/update, settings update, tournaments
(all 9), articles (all 5), announcements (3), events (3), roster (3), matches (2),
scrims (2), polls create/deactivate, wiki create/update/delete, forum pin/lock,
stats upload/token-create/token-revoke.

UNAUDITED holes (officer/admin/config-relevant):
- upload_avatar, upload_image (-006)
- admin_create_season (-007)
- forum create_category / update_category / create_board / update_board (-004)
- batch_mark_attendance (-008)
- test_discord_webhook (integrations.rs:19, admin outbound side-effect) — LOW, note only

UNAUDITED but acceptable (member self-service, non-config): forum create_thread /
create_reply, poll vote/unvote, rsvp_event, stats update_member_settings.

## Authz map verdict
No unauthenticated or under-privileged MUTATION route found (no missing-extractor CRIT).
Only asymmetry of note: teams create=Admin vs update=Officer (-002). games symmetric
(Admin/Admin). All officer/member mutation routes carry an extractor; scrims/attendance
add correct in-handler self/roster checks.

## Injection / traversal
No user input interpolated into SurrealQL in any ADMIN route (bind-params only; queries
live in db lane). Upload filenames are server-generated UUIDs (uploads.rs:95) — no path
traversal from client. Settings sanitizers (settings.rs:12-63) correctly reject CSS/URL
injection for bg color/image/brand accent. matches.rs VOD host allowlist + replay-code
charset validation are solid.
