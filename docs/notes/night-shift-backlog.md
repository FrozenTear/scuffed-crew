# Night-Shift Backlog — queued work

**Rewritten 2026-07-24** (claude, review pass at USER request). Supersedes the
07-17 v0.1.0-ship-day list: **items 1–4 and 7–10 of that list are all merged**
(see §A). What remains from it is only the two USER-gated items. The rest of
this doc is new work from the 07-24 review.

Every item: problem, fix sketch, files, size, done-criteria. Standing rules per
`docs/fleet-protocol.md`: worktrees only (shared checkout read-only), dual-agree
before merge (author never merges own branch), human holds the tag/release gate.

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

## §1. Tonight's suggested order

Research-first items are marked **[R]** — they need a look before anything is
built. Items 1–2 are one branch.

1. **Item 1 + 2** — CI coverage holes. Smallest change, permanently raises the
   floor for every later item. Do this first.
2. **Item 3 [R]** — NIP-05 domain. **Confirmed broken** (USER 07-24: we do not
   own `scuffed.gg`). Research the blast radius before patching.
3. **Item 4** — `?name=_` NIP-05 spec deviation. Pairs naturally with item 3.
4. **Item 5** — `public_member_profile` N+1. ~20 lines against an existing
   tested helper; near-free.
5. **Item 6** — unthrottled unauthenticated routes.
6. **Item 7** — chat error leak + per-request relay reconnect.
7. **Item 8** — doc/branch hygiene. Good filler while builds run.
8. **Items 9–10** stay USER-gated (carried from the old list).

---

## §2. New work (2026-07-24 review)

### 1. CI never runs the app's tests  [HIGH value, TINY effort]

**Problem.** `.github/workflows/ci.yml:103` excludes `scuffed-app` from
`cargo test`. 39 tests exist and pass — undo stack, editor state, canvas tools,
brand/theme tokens — and have **never** run in CI. Verified locally:
`cargo test -p scuffed-app` → 39 passed, ~30s including build.

**Fix sketch.** Drop `--exclude scuffed-app` from the test step (the job already
installs the system deps the app needs). Keep the build-step exclusion if the
native build is the expensive part — measure first; if adding it pushes the job
over the disk/time budget the 07-19 hardening was fighting, split app tests into
their own job instead of dropping the item.
**Files.** `.github/workflows/ci.yml`.
**Done when.** A deliberately-broken assertion in `state/undo.rs` fails CI.

### 2. CI clippy has no `--all-targets` — test code is unlinted  [MEDIUM, SMALL]

**Problem.** Both clippy steps (`ci.yml:56`, `:64`) omit `--all-targets`, so no
test code is ever linted. 14 warnings are already sitting in the tree, invisible:
11 × `needless_borrow` in `crates/site-server/tests/api_integration.rs`,
2 × `field_reassign_with_default`, 1 × `len_without_is_empty` on
`ConsumedChallengeStore`. They land all at once on whoever adds the flag.

**Fix sketch.** Clear the 14 warnings first (mechanical; `--fix` handles most),
then add `--all-targets` to both steps in the same PR so the gate closes behind
the cleanup. `ConsumedChallengeStore` wants a real `is_empty`, not an `#[allow]`.
**Files.** `.github/workflows/ci.yml`, `crates/site-server/tests/api_integration.rs`,
`crates/site-server/src/challenge_store.rs`, 2 sites flagged by the run.
**Done when.** `cargo clippy --workspace --all-targets -- -D warnings` is clean
locally and enforced in CI.

### 3. NIP-05 identities point at a domain we don't own  [**HIGH** — confirmed broken]

**Problem.** `crates/site-server/src/routes/members.rs:424` publishes kind-0
profile metadata with `nip05 = <name>@scuffed.gg`. The deploy serves
`/.well-known/nostr.json` from `ow.scuffedcrew.no` (`docs/deploy.md:155`).
**USER confirmed 07-24 that `scuffed.gg` is not ours.** So a Nostr client
verifying any member identity fetches `https://scuffed.gg/.well-known/nostr.json`
— a third party's domain — and NIP-05 verification fails for every member today.

Same root: `docs/external-clients.md:14,24,59` instructs members to add
`wss://relay.scuffed.gg` as their relay. That is a domain someone else can
register and point anywhere. Treat the doc as part of this fix, not a follow-up.

**[R] Research first (before patching).**
- How many members already have a published kind-0 with the bad `nip05`? Those
  events are on relays and immutable — a fix must **republish**, not just change
  the format going forward. Scope the republish path before writing code.
- Does anything else derive identity from that string (`crates/chat`,
  `crates/app/src/pages/identity.rs:384`, relay-policy allowlist)?
- Confirm the intended canonical domain with USER — `ow.scuffedcrew.no` today,
  but a future apex would change the answer, and republishing twice is worse
  than waiting a day for the decision.

**Fix sketch.** Derive the NIP-05 domain from config, not a literal.
`state.oauth_config.redirect_base_url` already holds the canonical origin and is
already used this way by `routes/calendar.rs:28-32` (strips the scheme) — reuse
that, or promote it to a proper settings field per P2-4. Then republish kind-0
for affected members. Fix `identity.rs:384` (display) and `external-clients.md`
(relay URL) in the same branch.
**Files.** `crates/site-server/src/routes/members.rs`,
`crates/app/src/pages/identity.rs`, `docs/external-clients.md`,
`crates/chat/src/nostr/events.rs:555,573` (test fixtures — check before changing).
**Size.** Small code, medium blast radius. 1 implementer + 1 review, USER
decision on the domain before merge.
**Done when.** No hardcoded `scuffed.gg` outside tests; a published kind-0 for a
test member verifies against the live `.well-known/nostr.json`; P2-4 in
`docs/website-review-fix-list.md` updated.

### 4. `/.well-known/nostr.json?name=_` returns every member  [MEDIUM]

**Problem.** `crates/site-server/src/routes/nostr.rs:105` treats `_` as a
wildcard: `if requested_name == "_" || requested_name == nip05_name`. In NIP-05,
`_` is the **root identifier for the domain itself**, not "all names". Two more
quirks in the same handler: a request with no `name` param returns an empty set
silently (rather than the conventional full-or-root response), and every request
runs `list_nostr_identities()` — a `LIMIT 2000` member scan — regardless of what
was asked for.

**Not a data leak** — `nostr_pubkey` is already in the public member projection
(`public.rs:271-288` `member_to_public`) and that endpoint is rate-limited. This
is a spec-conformance + wasted-work item, not a disclosure. Do not file it as one.

**Fix sketch.** Make `_` resolve to the org's own root identity (or 404 if none
is configured); push the name filter into the query so a lookup fetches one row
instead of 2000. Decide deliberately what a bare `/.well-known/nostr.json` with
no `name` should return and document it.
**Files.** `crates/site-server/src/routes/nostr.rs`, `crates/db/src/queries/members.rs`.
**Done when.** `?name=_` no longer enumerates; a single-name lookup does a
single-row query; behavior matches NIP-05 and is covered by tests.

### 5. `public_member_profile` N+1 over every team  [MEDIUM value, SMALL effort]

**Problem.** `crates/site-server/src/routes/public.rs:439-468` fetches all teams,
then every team's **full roster**, to find the teams one member is on — on a
public, traffic-bearing endpoint. `Database::get_member_teams()`
(`crates/db/src/queries/roster.rs:169`) already does exactly this in one indexed
query against `plays_on`, and currently has **zero callers**.

Note: the old backlog item 1 claimed `get_member_teams` had callers worth
protecting. It does not — that reasoning is stale, don't inherit it.

**Fix sketch.** Replace the loop with `get_member_teams(&id)`, then resolve team
names from the already-fetched `list_teams()` (or add a named variant mirroring
`get_team_roster_named`, which is the pattern the team-roster fix already used).
**Files.** `crates/site-server/src/routes/public.rs`, possibly
`crates/db/src/queries/roster.rs`.
**Done when.** The public member page renders identical team lists via one roster
query; no per-team roster loop remains; test covers a member on 2 teams and a
member on none.

### 6. Unauthenticated routes outside every rate limiter  [MEDIUM]

**Problem.** The HS-DR P1 public governor (`lib.rs:107-141`) covers
`/api/public/*` only. Still unauthenticated **and** unthrottled:
- `/api/calendar/all.ics` and `/api/calendar/team/{id}` — full `list_events()`
  plus a settings read per hit, and `Cache-Control: public, max-age=3600` only
  helps if something upstream actually caches.
- `/.well-known/nostr.json` — the 2000-row scan from item 4.
- `/api/auth/setup-status`, `/api/auth/providers` — cheap, but free probes.

Same amplification class the public governor was added to close. Both calendar
handlers correctly filter `is_public` (checked) — this is cost, not exposure.

**Fix sketch.** Extend the existing `public_governor_config` group to cover the
calendar + well-known + setup-status/providers routes, or add a second group with
a looser budget if 5/s is wrong for an ICS feed that calendar clients poll.
Consider whether Caddy edge caching is the better lever for the ICS specifically.
**Files.** `crates/site-server/src/lib.rs` (+ `docs/deploy.md` if Caddy changes).
**Done when.** No unauthenticated route sits outside a governor group; an ICS
poll loop from one IP gets throttled; normal calendar-client refresh still works.

### 7. Chat: internal errors to clients + per-request relay reconnect  [MEDIUM]

**Problem — two issues, one file.**
(a) `crates/server/src/routes/chat.rs` returns internal error text to clients:
`format!("Encryption failed: {e}")` (:328), `"Decryption failed: {e}"` (:447),
`"Failed to provision auth event: {e}"` (:65, :114). The B5 fix applied the
log-internally / generic-to-client pattern to tournaments, forum and members but
never reached chat.
(b) `send_encrypted` (:336) opens a **fresh relay WebSocket per request**, then
publishes gift wraps **sequentially**, then disconnects — latency scales with
officer count. The `TODO(Phase 2c)` on that line already names the fix.

**Fix sketch.** (a) is independent and small — log the detail, return a generic
message, mirror B5 exactly. (b) is the real work: hold a `RelayClient` in shared
state (`AppState` already carries `relay_url` and `dm_events`; `dm_subscriber.rs`
already maintains a persistent connection — reuse that seam rather than inventing
a second one) and publish concurrently. **Split these into two branches** — (a)
should not wait on (b).
**Files.** `crates/server/src/routes/chat.rs`, `crates/site-server/src/state.rs`,
`crates/site-server/src/dm_subscriber.rs`.
**Done when.** (a) no `{e}` reaches a chat client, detail still in logs.
(b) one message to N officers uses one connection and one round-trip batch;
the "502 if any gift-wrap publish fails" guarantee from B7 is preserved.

### 8. Doc + branch hygiene  [LOW, TINY — good filler]

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

### 9. Systemic: full-table loads in list handlers  [LOW now, watch it]

`list_applications` (`applications.rs:148`) and `list_moderation`
(`moderation.rs:270`) each load the **entire member table** per request to build
a name map; applications additionally does a per-row `get_user` for
never-provisioned applicants, where a single DB error fails the whole page.

Correct and fine at org scale — **not a bug, do not "fix" it tonight.** Logged
because the pattern was introduced twice in one week and the third copy is when
it becomes real. If a fourth list needs names, build the shared
page-scoped-join helper instead (`crates/db/src/queries/audit_log.rs`
`enrich_audit_actor_names` is the good pattern to copy — it resolves only the ids
on the current page).

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

Old item 8 suggested a GUI stat-edit affordance as the durable answer. Still
unbuilt; still the right call if manual repairs keep recurring.

### 13. Retracted / non-items — do not "fix"

- `roster.rs` "(public)" comment: **correct as written** (GET has no auth
  extractor by design; data already public via team pages). Flagged wrongly on
  07-17 and retracted.
- Item 9 above (full-table list loads): logged, deliberately **not** actioned.

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
   protected paths, policy overrides, and the item-3 domain decision.
7. **Worktrees only** — `.claude/worktrees/<agent>-<topic>`. Shared checkout
   `~/github/scuffed-crew` stays read-only for agents (IRON LAW).
8. **Append status to this doc as items land** — it is the durable state now.
