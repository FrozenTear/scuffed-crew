# Night-Shift Backlog — execution plan

**Rewritten 2026-07-24** (claude, review pass at USER request), then **corrected
same day after a 29-agent adversarial review of this document itself** — 10
findings, all folded in below (wrong file:line pointers, two fix sketches aimed
at the wrong seam, one stale "already indexed" claim, one deleted
data-preservation guard restored). Supersedes the 07-17 v0.1.0-ship-day list:
items 1–4 and 7–10 of that list are **all merged** (see §A). What remains from
it is only the USER-gated items in §3.

Every item: problem, fix plan, files, size, done-criteria. Standing rules per
`docs/fleet-protocol.md`: worktrees only (shared checkout read-only for agents —
IRON LAW), dual-agree before merge (author never merges own branch), human holds
the tag/release gate.

**⚠ Coordination change: no Memtrace on this machine tonight.** The fleet log is
unavailable, so the protocol's "all findings on the fleet channel" rule has no
transport. Substitute, in priority order: (1) **branch + PR per item** — the PR
body is the finding record; (2) **this doc** — append status inline as items
land; (3) commit bodies for dissent (record it verbatim, same as the log rule).
Git/gh was already the authority after restarts; tonight it is the *only*
authority. Do not block waiting for a channel that isn't there.

---

## §0. Baseline — verified green 2026-07-24

Confirmed on `main` @ `201c1b9` before this list was written, so a red result
tonight is *yours*, not pre-existing:

| Check | Result |
|---|---|
| `cargo fmt --check` | clean |
| clippy, CI's scope (native + wasm) | clean |
| `cargo test` (workspace − stat-tracker) | ~450 pass, 0 fail |
| `cargo test -p scuffed-app` | 39 pass |
| `check-frontend-deps.sh` / `check-design-tokens.sh` | both pass |
| Open PRs / working tree | none / clean |

**Local gotcha:** `cargo test --workspace` fails on this box — `leptonica-sys`
needs `lept.pc`, and only the runtime `libleptonica.so.6` is installed (no
headers, no `.pc`, no unversioned `.so`). Distro is **AerynOS 2026.05**. Since
stat-tracker work happens on this machine, fix it properly by installing the
leptonica + tesseract **dev** packages rather than working around it. Until
then: `cargo test --workspace --exclude scuffed-stat-tracker`.

---

## §1. Tonight's execution plan

One branch per item ID; PR body is the finding record. Author never merges own
branch — pair up: whoever implements NS2-1 reviews NS2-3, etc. Research-first
items are marked **[R]**: post the research result in the PR/draft *before*
writing the fix, so a wrong premise dies cheap.

| Order | ID | Branch suggestion | Gate |
|---|---|---|---|
| 1 | NS2-1 + NS2-2 | `ci/app-tests-and-clippy-all-targets` | measure disk before merging (see NS2-1) |
| 2 | NS2-3 **[R]** | `fix/nip05-domain` | **USER decision on canonical domain before merge** |
| 3 | NS2-4 **[R]** | `fix/nip05-wellknown-conformance` | pairs with NS2-3, same reviewer |
| 4 | NS2-5 | `fix/public-member-profile-n1` | includes the `plays_on` index |
| 5 | NS2-6 | `fix/unthrottled-public-routes` | — |
| 6 | NS2-7a | `fix/chat-error-hygiene` | small, independent |
| 7 | NS2-7b **[R]** | `feat/chat-relay-conn-reuse` | per-channel relay — read the caution in item 7 first |
| 8 | NS2-8 | `docs/fix-list-hygiene` | filler while builds run |

Items **10–12** (§3) stay USER-gated — do not start them, but do not drop them
either: if USER appears mid-shift, surface them. Item 9 is a logged non-action
(see §2 item 9 and §3 item 13), not USER-gated — nothing to wait for there.

**Hard rules for tonight** (restored + new, violations are shift-stoppers):
- **Do NOT clear, reset, or re-seed the local stat-tracker store**
  (`stats.surrealkv` under the daemon's data dir). It holds the ONLY copies of
  the Route 66 (35 caps), Lijiang, and Ilios capture series — real-pixel
  regression material that exists nowhere in git (only the Havana series was
  extracted, as the numeric fixture in `capture_gate.rs:669-769`). Item 10's
  off-machine backup has not happened yet. This guard was in the 07-17 doc and
  it still binds.
- No tags, releases, force-push, data deletion, protected paths, policy
  overrides. Human-only.
- Kind-0 / NIP-05 **republish is USER-gated** (item 3) — prepare the branch,
  do not publish events to relays tonight.

---

## §2. New work (2026-07-24 review, corrected same day)

### 1. CI never runs the app's tests  [NS2-1 — HIGH value, TINY effort]

**Problem.** `.github/workflows/ci.yml:103` excludes `scuffed-app` from
`cargo test`. 39 tests exist and pass — undo stack, editor state, canvas tools,
brand/theme tokens — and have **never** run in CI. Verified locally:
`cargo test -p scuffed-app` → 39 passed, ~30s including build.

**Plan.** Drop `--exclude scuffed-app` from the test step. Be aware:
`cargo test` **compiles the crate it tests**, so the build-step exclusion at
`ci.yml:100` stops saving anything the moment the test step includes the app —
keeping it is a no-op, not a mitigation. The real question is whether the
native scuffed-app build pushes the Build & Test job over the runner disk
budget that the 07-19 hardening (`CARGO_PROFILE_DEV_DEBUG: "0"`, the
`rm -rf` free-space step) exists to protect. **Measure first**: run the job in
a draft PR and check `df` headroom. If it doesn't fit, put app tests in their
own job (own cache key) instead of squeezing them in.
**Files.** `.github/workflows/ci.yml`.
**Done when.** A deliberately-broken assertion in `state/undo.rs` fails CI, and
the Build & Test job completes with disk headroom recorded in the PR.

### 2. CI clippy has no `--all-targets` — test code is unlinted  [NS2-2 — MEDIUM, SMALL]

**Problem.** Both clippy steps (`ci.yml:56`, `:64`) omit `--all-targets`, so no
test code is ever linted. 14 warnings are already sitting in the tree, invisible:
11 × `needless_borrow` in `crates/site-server/tests/api_integration.rs`,
2 × `field_reassign_with_default`, 1 × `len_without_is_empty` on
`ConsumedChallengeStore`. They land all at once on whoever adds the flag.

**Plan.** Clear the 14 warnings first (mechanical; `--fix` handles most), then
add `--all-targets` to both steps in the same PR so the gate closes behind the
cleanup. `ConsumedChallengeStore` wants a real `is_empty`, not an `#[allow]`.
**Files.** `.github/workflows/ci.yml`, `crates/site-server/tests/api_integration.rs`,
`crates/site-server/src/challenge_store.rs`, 2 sites flagged by the run.
**Done when.** `cargo clippy --workspace --all-targets -- -D warnings` is clean
locally and enforced in CI.

### 3. NIP-05 identities point at a domain we don't own  [NS2-3 — **HIGH**, confirmed broken]

**Problem.** `crates/site-server/src/routes/members.rs:424` publishes kind-0
profile metadata with `nip05 = <name>@scuffed.gg`. The deploy serves
`/.well-known/nostr.json` from `ow.scuffedcrew.no` (`docs/deploy.md:155`).
**USER confirmed 07-24 that `scuffed.gg` is not ours.** So a Nostr client
verifying any member identity fetches a third party's domain — NIP-05
verification fails for every member today, and could be *hijacked* by whoever
registers the domain.

Same root: `docs/external-clients.md:14,24,59` instructs members to add
`wss://relay.scuffed.gg` as their relay. Treat the doc as part of this fix.

**[R] Research first (before patching).**
- How many members already have a published kind-0 with the bad `nip05`? Those
  events are on relays and immutable — the fix must **republish**, not just
  change the format going forward. Scope the republish path before writing code.
- Does anything else derive identity from that string (`crates/chat`,
  `crates/app/src/pages/identity.rs:384`, relay-policy allowlist)?
- Confirm the canonical domain with USER — `ow.scuffedcrew.no` today, but a
  future apex changes the answer, and republishing twice is worse than waiting.

**Plan.** Derive the NIP-05 domain from configuration — but **NOT** by blindly
reusing `state.oauth_config.redirect_base_url`: that value defaults to
`http://localhost:3000` (`state.rs:90`) / `http://127.0.0.1:3000`
(`compose.yml:76`) and the installer's public-URL prompt accepts blank. A naive
reuse would publish immutable `name@127.0.0.1:3000` identities to public relays
on any default-configured deploy — worse than today. Required shape:
- Publish kind-0 `nip05` **only when** a valid, non-loopback, non-IP public
  domain is configured (dedicated setting per P2-4, or a validated derivation
  from `redirect_base_url` that refuses loopback/private/blank). No valid
  domain → omit the `nip05` field entirely; everything else about the kind-0
  still publishes.
- Then republish kind-0 for affected members — **USER-gated, not tonight**
  (see §1 hard rules); land the code, stage the republish as a command/flag.
- Fix `identity.rs:384` (display) and `external-clients.md` (relay URL) in the
  same branch.

**Files (corrected pointers).**
- `crates/site-server/src/routes/members.rs:416-425` (the publish path)
- `crates/app/src/pages/identity.rs:384` (display string)
- `docs/external-clients.md:14,24,59` (relay URL instructions)
- Test fixtures asserting the domain: `crates/chat/src/nostr/events.rs:727,738`
  (`testuser@scuffed.gg`) and
  `crates/site-server/tests/api_integration.rs:1511,1538`
  (`wss://relay.scuffed.gg`). NOTE: `events.rs:555,573` are `logo.png` avatar
  URLs in an unrelated test — cosmetic third-party-domain references worth
  sweeping too, but they are NOT the NIP-05 fixtures.

**Size.** Small code, medium blast radius. 1 implementer + 1 review, USER
decision on the domain + republish before merge.
**Done when.** No hardcoded `scuffed.gg` outside deliberately-kept fixtures; a
default-configured dev deploy publishes kind-0 **without** a `nip05` field
(test this case explicitly); a properly-configured deploy's `nip05` verifies
against its own `.well-known/nostr.json`; P2-4 in
`docs/website-review-fix-list.md` updated.

### 4. `/.well-known/nostr.json?name=_` returns every member  [NS2-4 — MEDIUM]

**Problem.** `crates/site-server/src/routes/nostr.rs:105` treats `_` as a
wildcard: `if requested_name == "_" || requested_name == nip05_name`. In NIP-05,
`_` is the **root identifier for the domain itself**, not "all names". Two more
quirks: a request with no `name` param returns an empty set silently, and every
request runs `list_nostr_identities()` — a `LIMIT 2000` member scan — regardless
of what was asked.

**Not a data leak** — `nostr_pubkey` is already in the public member projection
(`public.rs:271-288` `member_to_public`) and that endpoint is rate-limited. This
is spec conformance + wasted work, not disclosure. Do not file it as one.

**Plan.** Two parts, and mind the constraint the first sketch of this item
missed: the NIP-05 name is **computed in Rust** (`normalize_nip05_name`,
`nostr.rs:67-73` — lowercase, strip to `[a-z0-9_]`) and exists in **no DB
column**, so the filter cannot be "pushed into the query" as-is. A naive
`WHERE display_name = $name` would break every member whose display name has
spaces/capitals ("Frozen Tear" → `frozentear`) — a regression from today's
working path. Choose one:
- **(a) Cheap, tonight-sized:** keep the in-Rust filter, fix only the `_`
  semantics (root identity or 404, never enumerate) and the empty-`name`
  behavior. The 2000-row scan stays but is bounded and rate-limitable via
  NS2-6. Document the scan as accepted.
- **(b) Right, bigger:** add a stored `nip05_name` column, written through on
  every display-name change (migration + backfill via `normalize_nip05_name`,
  plus a `DEFINE INDEX`), then filter in the query. Touches member-update paths;
  only do this tonight if (a) lands first and time remains.

**Files.** `crates/site-server/src/routes/nostr.rs`; (b) additionally
`crates/db/src/migrations.rs`, `crates/db/src/queries/members.rs` and the
member-update write paths.
**Done when.** `?name=_` no longer enumerates; a single-name lookup for a member
with a spaced/capitalized display name still verifies (regression test);
empty-`name` behavior is deliberate and documented.

### 5. `public_member_profile` N+1 over every team  [NS2-5 — MEDIUM value, SMALL effort]

**Problem.** `crates/site-server/src/routes/public.rs:439-468` fetches all teams,
then every team's **full roster**, to find the teams one member is on — on a
public, traffic-bearing endpoint. `Database::get_member_teams()`
(`crates/db/src/queries/roster.rs:169`) already does this in one query against
`plays_on`, and currently has **zero callers** (verified 07-24).

**Two corrections to the first sketch of this item:**
- `plays_on` is **unindexed** — `migrations.rs:101` defines the relation table
  but no `DEFINE INDEX` covers `in` or `out`, so `get_member_teams` is an edge-
  table scan, not "one indexed query". Fine at current scale, but since the
  point of this item is a public-endpoint hot path, **add the index in the same
  branch**: `DEFINE INDEX IF NOT EXISTS plays_on_in_idx ON plays_on COLUMNS in;`
  (and consider `out` for the team-roster direction while there).
- The old backlog's "Do NOT touch `RosterEntry`" note is **still valid** — the
  07-24 rewrite wrongly retracted it. `RosterEntry` is the return type of
  `get_team_roster`, which has four live callers: `public.rs:62` (overview
  roster counts), `public.rs:451` (this very loop), `chat/provisioning.rs:113`
  (channel provisioning), `server/routes/chat.rs:267` (officer fan-out).
  **Do not reshape `RosterEntry`** (no new required fields). If team names are
  needed, resolve them from the already-fetched `list_teams()` or add a separate
  named variant (the `get_team_roster_named` pattern).

**Plan.** Replace the loop with `get_member_teams(&id)`; resolve team names from
`list_teams()`; add the `plays_on` index; leave `RosterEntry` untouched.
**Files.** `crates/site-server/src/routes/public.rs`,
`crates/db/src/migrations.rs`, possibly `crates/db/src/queries/roster.rs`.
**Done when.** Public member page renders identical team lists via one roster
query; no per-team roster loop remains; `plays_on(in)` is indexed; tests cover a
member on 2 teams and on none; the four `get_team_roster` call sites compile
untouched.

### 6. Unauthenticated routes outside every rate limiter  [NS2-6 — MEDIUM]

**Problem.** The HS-DR P1 public governor (`lib.rs:107-141`) covers
`/api/public/*` only. Still unauthenticated **and** unthrottled:
- `/api/calendar/all.ics` and `/api/calendar/team/{id}` — full `list_events()`
  plus a settings read per hit; `Cache-Control: public, max-age=3600` only helps
  if something upstream actually caches.
- `/.well-known/nostr.json` — the 2000-row scan from item 4.
- `/api/auth/setup-status`, `/api/auth/providers` — cheap, but free probes.

Same amplification class the public governor was added to close. Both calendar
handlers correctly filter `is_public` (checked) — this is cost, not exposure.

**Plan.** Extend the existing `public_governor_config` group to cover calendar +
well-known + setup-status/providers, or add a second group with a looser budget
if 5/s is wrong for an ICS feed calendar clients poll. Consider Caddy edge
caching as the better lever for the ICS specifically.
**Files.** `crates/site-server/src/lib.rs` (+ `docs/deploy.md` if Caddy changes).
**Done when.** No unauthenticated route sits outside a governor group; an ICS
poll loop from one IP gets throttled; normal calendar-client refresh still works.

### 7. Chat: internal errors to clients + per-request relay reconnect  [NS2-7 — MEDIUM]

**Problem — two issues, one file. Split into two branches; (a) must not wait on (b).**

**(a) Error hygiene — small, safe.** `crates/server/src/routes/chat.rs` returns
internal error text to clients: `format!("Encryption failed: {e}")` (:328),
`"Decryption failed: {e}"` (:447), `"Failed to provision auth event: {e}"`
(:65, :114). The B5 fix applied log-internally/generic-to-client to
tournaments, forum and members but never reached chat. Mirror B5 exactly.

**(b) Connection reuse — [R], the first sketch of this item aimed at the wrong
seam.** `send_encrypted` (`chat.rs:194`) opens a fresh relay WebSocket per
request (`RelayClient::new`, `:338`) and publishes gift wraps sequentially. But
note `chat.rs:337`: it publishes to
**`channel.relay_url` — a per-channel DB column** (`db/src/types.rs:777`), NOT the
process-wide `AppState.relay_url` / `NOSTR_RELAY_URL`. The `dm_subscriber`
persistent connection is built from the env URL (`dm_subscriber.rs:132`), so
"just reuse dm_subscriber's connection" would silently publish every channel's
gift wraps to the wrong relay — delivery succeeds locally, recipients on the
channel's actual relay never see the message, and the B7 502-guard never trips.
Correct shape: a small **connection cache keyed by relay URL**
(`HashMap<String, RelayClient>` behind a mutex in shared state, lazily
connected, with reconnect-on-error), so each channel keeps talking to its own
relay. Research first: whether `RelayClient` tolerates concurrent publishes,
and whether channels in practice all share one relay URL today (if provably
yes and enforced, a simpler design is defensible — document the invariant).
**Files.** (a) `crates/server/src/routes/chat.rs` only. (b) also
`crates/site-server/src/state.rs` (the unified server has no state module of
its own — it imports `scuffed_site_server::state::AppState`,
`crates/server/src/main.rs:13`) and `crates/chat/src/nostr/relay.rs`.
**Done when.** (a) no `{e}` reaches a chat client; detail still in logs.
(b) one message to N officers reuses a connection **to that channel's relay**;
concurrent publishes; B7's "502 if any gift-wrap publish fails" preserved; a
test covers two channels with different relay URLs.

### 8. Doc + branch hygiene  [NS2-8 — LOW, TINY — good filler]

- `docs/website-review-fix-list.md` lists **P2-2 (raw hex CI guard) as open**.
  It is **done and CI-enforced** — `scripts/check-design-tokens.sh` runs in the
  `dep-guardrails` job and there are currently **0** raw hex literals outside
  `theme/`. Mark it done.
- Same doc, P2-1 says "~71 one-off `PAGE_CSS` blocks". It is **74** now — the
  migration is slowly losing ground. Update the number; consider a guard that
  fails on *new* blocks rather than requiring the full migration.
- **32 remote branches**, nearly all merged. Prune. Note
  `fix/tracker-hero-writethrough` is already gone from origin and its fix **is**
  in main (verified) — the old backlog's "parked, UNMERGED" note is stale, no
  work was lost.

### 9. Systemic: capped member loads in list handlers  [NS2-9 — LOW now, watch it]

`list_applications` (`applications.rs:148`) and `list_moderation`
(`moderation.rs:270`) build name maps from `list_members()` — which is **not** a
full-table load: it is `list_members_paginated(500, 0)`
(`members.rs:180-182`), a silent 500-row cap. The failure mode at scale is
therefore **wrong output, not cost**: past 500 active members, every later
member falls out of the name map and admin lists silently render fallback/blank
names. Applications additionally does a per-row `get_user` for never-provisioned
applicants, where one DB error fails the whole page.

Fine at org scale tonight — **not a work item, do not "fix" it.** Logged because
the pattern was introduced twice in one week and the third copy is when it
becomes real. If a fourth list needs names, build the shared page-scoped-join
helper instead (`crates/db/src/queries/audit_log.rs` `enrich_audit_actor_names`
is the good pattern — it resolves only the ids on the current page, so no cap
can bite). If the org ever approaches 500 members, this graduates to a bug.

---

## §3. Carried forward — USER-gated (unchanged from 07-17)

### 10. Real-pixel frame gap — off-machine backup  [USER decision required]

3 copies of the only two real 6v6 fixture frames, ALL on one machine (2 repo
trees + `/mnt/2TB/scuffed-backups/stat-tracker-test-data/`). Whole-box loss =
only real-pixel OCR evidence gone. Options (USER picks before agents act):
private fixture repo | git-lfs | cloud copy of the backup dir. Copyright
gitignore stays untouched regardless — frames never enter the public repo.
md5: fuzzy=`60df83793378aa1401dcb3c181561af8`, push=`08b2993089a59c5579dfd9ca38a5d6e6`.

### 11. Live OCR validation  [USER plays, agents analyze]

Now against **v0.3.1** (the 07-17 list said v0.1.0 — three minor versions of gate
and map-matching work have landed since; see §A). Check: games register,
name-col resolves on magenta/gold palettes, offline/reject rates, and whether the
capture gate's monotonic-hold + rate-cap actually swallow the 9X inflation and
tail-clip classes in the field. Agents' part: pull daemon logs + rejected frames
after a session, compare against `debug/rejected` expectations, post findings as
a PR or an append to this doc.

### 12. Manual stat repairs — still no affordance  [needs decision]

There is **no stat-edit path in the app** (checked 07-24: no `stat_edit` /
`repair_session` anywhere in `crates/stat-tracker/src`). The two queued repairs
below therefore still need either a direct store edit or a new affordance — and
I could not verify from the repo whether they were ever applied. **Confirm status
before acting; do not re-apply blindly.**
- Havana Victory `b1b263e994d1f7f8`: D=5→6, HLG=234→2341
- Lijiang Defeat (07-18 00:22): hero Tracer→Mizuki, role Damage→Support

**⚠ BINDING (restored from the 07-17 doc): any store access for these repairs
must be additive-only. Do NOT clear, reset, re-seed, or bulk-rewrite the local
store — it holds the only copies of the Route 66 / Lijiang / Ilios capture
series (see §1 hard rules). Take a file-level copy of `stats.surrealkv` before
any direct edit.**

Old item 8 suggested a GUI stat-edit affordance as the durable answer. Still
unbuilt; still the right call if manual repairs keep recurring.

### 13. Retracted / non-items — do not "fix"

- `roster.rs` "(public)" comment: **correct as written** (GET has no auth
  extractor by design; data already public via team pages). Flagged wrongly on
  07-17 and retracted.
- Item 9 above (capped member-name loads): logged, deliberately **not** actioned.
- The 07-24 rewrite's own retraction of the `RosterEntry` warning was itself
  wrong and has been re-retracted — the warning stands (see item 5).

---

## §A. Closed since the 07-17 list — do not redo

Verified merged on `main` @ `201c1b9`, 2026-07-24:

| Old # | Item | Evidence |
|---|---|---|
| 1 | Roster N+1 → db join | `get_team_roster_named` live in `roster.rs:80` + `public.rs:637` |
| 2 | `ok().flatten()` masking | died with the join, as designed |
| 3 | install.sh PREFIX pollution | `SKIP_INTEGRATION` honored, `install.sh:23-27,96-97` |
| 4 | install.sh logs to stdout | `info`/`warn`/`error` all `>&2`, `install.sh:19-21` |
| 7 | GUI libxdo bundle + RUNPATH | bundled + `patchelf --set-rpath` + clean-room `ldd` gate, `stat-tracker-release.yml:163-196` |
| 8 | OCR elims misreads (gate) | `crates/stat-tracker/src/capture_gate.rs` — monotonic hold + rate cap + corroboration |
| 9 | Ilios / map matching | `normalize_ocr_glyphs` + per-length fuzzy threshold in **both** readers (`parse.rs:384`, `match_start.rs:350`) |
| 10 | Hero write-through | in main — `storage/mod.rs:435-442` writes `personal_match` + `synced=false` |

Also landed since: DR1 P0–P2 (acct/auth/admin/nostr/db), trust P0–P2, audit
`target_label` read-time join, roster role PUT response, relay health honesty
(F-AUI-001/002/003/004), CI disk hardening. stat-tracker is at **v0.3.1**.

Dependabot (noted 07-24, low urgency, NOT tonight-work): `glib 0.18.5`
unsoundness (needs 0.20+, i.e. a wry/tao bump — lands with the Dioxus 0.8 work
in `dioxus-0.8-alpha-notes.md`) and `rand 0.7.3` via `phf_generator` (build-time
codegen only). The serious one, SurrealDB CVE-2026-49997 (edge-delete permission
bypass — directly relevant to `plays_on`), is already **fixed**.

---

## §P. Process (adjusted for no-Memtrace)

1. **PR-per-item is the finding record.** No fleet channel tonight — the PR body
   carries problem/fix/verification. Keep bodies short; link code, don't paste it.
2. **Finding IDs** (`NS2-1` … `NS2-9` for §2 items) so CONFIRM/REFUTE is
   unambiguous across agents and sessions.
3. **Dual-agree before merge; author never merges own branch.** Unchanged, and
   more important without a channel to catch mistakes in flight.
4. **Review rounds capped at 2**, then leave the branch parked with the
   disagreement recorded in the commit body, and move on. Do not stall.
5. **Correctness objections always block.** Approach/priority disagreements
   resolve by: shrink the claim → prefer reversible → measure with fixtures
   instead of arguing → smaller blast radius. Record dissent verbatim.
6. **Human-only, no exceptions:** tags/releases, force-push, data deletion,
   protected paths, policy overrides, the item-3 domain decision + kind-0
   republish, and anything touching the local stat-tracker store beyond
   additive edits (§1 hard rules).
7. **Worktrees only** — `.claude/worktrees/<agent>-<topic>`. Shared checkout
   `~/github/scuffed-crew` stays read-only for agents (IRON LAW).
8. **Append status to this doc as items land** — it is the durable state now.
