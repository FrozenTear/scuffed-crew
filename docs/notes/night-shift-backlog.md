# Night-Shift Backlog — queued nits & carried-forward work

Written 2026-07-17 after the v0.1.0 ship day. This is the pick-up-and-run list for
the next fleet all-nighter (claude + grok). Every item: problem, fix sketch, files,
size, and done-criteria. Protocol per `fleet-protocol.md` artifact (see §P below) +
standing rules: dual-agree before merge, all findings on fleet channel, worktrees
only, human holds the tag/merge gate when present.

Ordered by value. Items 1–2 are one branch.

---

## 1. Roster N+1 → db-level join (both call sites)  [MEDIUM value, SMALL effort]

**Problem.** Two handlers fetch a team roster then loop `get_member_safe` per
member (N+1), collapsing per-member db errors to `"Unknown"` silently:
- `crates/site-server/src/routes/roster.rs` — `enrich()` (added by PR #3, correct
  fix for the deserialization bug, but sequential lookups)
- `crates/site-server/src/routes/public.rs:~548` — same pattern inline in
  `public_team_detail` (the public, traffic-bearing team page — this is why the
  fix is worth it at all)

**Fix sketch.** One new db method traversing the `plays_on` edge in a single query:

```sql
SELECT *, meta::id(id) as id, <string>in as in, <string>out as out,
       in.display_name AS member_name, in.avatar_url AS avatar_url
FROM plays_on
WHERE out = $team_rid AND is_active = true
```

- New `Db*` struct private to `crates/db/src/queries/roster.rs`, new public type
  (e.g. `NamedRosterEntry { member_id, member_name: Option<String>, avatar_url,
  team_role, joined_at }`) in `crates/db/src/types.rs`.
- Do NOT touch `RosterEntry` — `get_member_teams` + other callers share it
  (same reasoning PR #3 used).
- `member_name` is `Option` because a dangling edge yields `NONE`; handlers keep
  the `"Unknown"` fallback plus a `tracing::warn!` (that closes item 2 for free —
  with the join there is no per-entry fallible lookup left to mask, the warn only
  fires on genuinely dangling edges).
- Rewire `roster.rs` GET+POST (POST still enriches a single entry — either keep
  `get_member_safe` there or add a single-row variant; single lookup is fine) and
  `public.rs` roster build. Delete `enrich()` loop + the public.rs loop.

**Verify.** cargo test (db in-mem seed covers roster?), dev-mode manual: admin
teams modal + public team page both show names; clippy/fmt gates.
**Size.** ~60–120 lines, 1 implementer + 1 review cycle.
**Done when.** Both pages render names via one query each; no `get_member_safe`
loops remain in either handler; warn on dangling edge.

## 2. `ok().flatten()` masks db errors as "Unknown"  [folded into #1]

Dies naturally with the join. If #1 is deferred again, the standalone fix is a
one-line `tracing::warn!` inside `roster.rs::enrich()` — do not spend a branch on
it alone; batch with any roster.rs touch.

## 3. install.sh: desktop entry + systemd unit ignore PREFIX  [SMALL, real-user-visible]

**Problem.** `crates/stat-tracker/install.sh` writes the desktop entry to
`~/.local/share/applications` and the systemd unit to `~/.config/systemd/user`
even when `PREFIX` points elsewhere (observed twice on 07-17: grok's clean-room
and claude's bootstrap test both polluted the real HOME from throwaway prefixes).

**Fix sketch.** Derive both paths from PREFIX when PREFIX is non-default
(`$PREFIX/share/applications`, skip systemd entirely for non-default prefix), or
add `SKIP_INTEGRATION=1` env honored by install.sh + used by bootstrap smoke and
CI clean-room. Second option is simpler and test-focused — lean that way.
**Files.** `crates/stat-tracker/install.sh` (+ release workflow smoke if it should
set the var, + bootstrap.sh pass-through if needed).
**Done when.** A PREFIX=/tmp/... install leaves $HOME untouched; real install
unchanged; release validation still green.

## 4. install.sh logs to stdout  [TINY, only with #3]

Nested-context fine today (bootstrap parses nothing from install.sh), but the
bootstrap bug (fixed @ 3cd2c0c) proves the pattern is a loaded gun. Mirror the
bootstrap fix: `info()`/`warn()` → stderr. Do it in the same branch as #3.

## 5. Real-pixel frame gap — off-machine backup  [USER decision required first]

3 copies of the only two real 6v6 fixture frames, ALL on one machine
(2 repo trees + `/mnt/2TB/scuffed-backups/stat-tracker-test-data/`). Whole-box
loss = only real-pixel OCR evidence gone. Options (USER picks before agents act):
private fixture repo | git-lfs | cloud copy of the backup dir. Copyright gitignore
stays untouched regardless — frames never enter the public repo.
md5: fuzzy=60df83793378aa1401dcb3c181561af8, push=08b2993089a59c5579dfd9ca38a5d6e6.

## 6. Live OCR validation (deferred 3.3)  [USER plays, agents analyze]

Needs real OW 6v6 games on v0.1.0 (installed on USER's box 07-17 18:20). Check:
games register (07-16 incident class), name-col resolves on magenta/gold palettes,
offline/reject rates vs the 07-14 session, `stats_from_row` reject dig follow-up.
Agents' part: pull daemon logs + rejected frames after the session, compare
against `debug/rejected` expectations, post findings with IDs.

## 7. GUI-1: stat-tracker-gui crashes on Arch — libxdo.so.3 missing  [MEDIUM, field-confirmed 07-17]

**Problem (root cause confirmed 07-17).** Release v0.1.0 GUI fails on
Arch/CachyOS: `libxdo.so.3` not found. NOT a missing package — a soname skew:
Arch's xdotool 4.20260303 ships `libxdo.so.4`; the 24.04-built GUI wants
`.so.3`, which Arch can no longer provide. Same fragmentation class as the
leptonica problem the daemon already solves by bundling. Compounding it: the
GUI binary shipped with **no RUNPATH at all** (only the daemon got
`$ORIGIN/../lib`), so it can't see bundled libs even if present. Daemon
unaffected. ABI is compatible: `.so.4` symlinked as `.so.3` works.

**Workaround live on USER box (remove after real fix):** compat symlink
`~/.local/lib/compat/libxdo.so.3 -> /usr/lib/libxdo.so.4` + patchelf RUNPATH
stamp on the installed GUI binary. Verified: clean launch, no env vars.

**Fix (bundle — documenting can't fix a soname the distro no longer has):**
- `.github/workflows/stat-tracker-release.yml` GUI job: add libxdo to the
  bundled lib/ closure AND apply the same RPATH `$ORIGIN/../lib` treatment the
  daemon gets (currently missing on the GUI binary entirely).
- Add to release validation: `ldd`-clean assertion + headless GUI launch probe
  so this class can't ship unchecked again.

**Done when.** Clean-room GUI launch check on a non-Ubuntu container (or at
minimum ldd-clean assertion in release validation) passes; release notes list
host deps. Consider adding a `--smoke` GUI headless probe to the release
workflow so the NOT-CHECKED gap from 1.3 closes permanently.

## 8. LIVE-1: OCR elims misreads — TWO modes, corrupted a real game  [**HIGH**, escalated 07-18]

**Field data 07-18 (corrected).** Route 66 game, 35 captures: ~40% had
two-digit E collapsed to "1" (series 7,5,1,11,1×4,12×3,15,1×4,9×3,22×4,28×2).
Final captures read clean => registered stats CORRECT (E28 D10 A17, POTG
screenshot-verified) — `latest_per_game` healed as designed, but only because
the match didn't end during a bad stretch; the 00:06 E=1 run would have locked
in. Severity stays HIGH on frequency, not on a realized corruption (earlier
"locked in" claim was premature — game was still running). Modes:
(a) inflation — Control point-% bleed-through (93/99 tracking real point %);
(b) deflation — two-digit values collapse to "1" (thin-glyph digit drop,
~40% of captures on this game); (c) SUSPECT, unconfirmed — captures 27–29
read 9/13/19 vs real ~E19/A13/D9: possible column rotation; keep eyes open.

**Fix pre-validated on real data:** per-cell monotonic hold applied to the
07-18 series yields 7,11,12,15,22,28 — every collapse rejected, every real
progression kept. Use this session's captures as the regression fixture.

**Fix (revised).** Per-cell monotonic hold: within a game, counters never
decrease; a SINGLE-counter decrease cannot be a new game (split requires 2/3
by design) => treat as misread, keep the previous value for that cell, flag
the capture. Plus a rate-cap for mode (a) (elims jump beyond plausible rate =>
hold until corroborated). Neither can recover a truth OCR never read (E stays
7 in the 07-18 case) — consider a GUI stat-edit affordance for manual repair.

Original notes (07-17), kept for context:

**Problem.** Live session 07-17 ~23:40: one capture read E=93 where the real
scoreboard showed E=3 (all other cells exact; screenshot cross-checked).
Hypothesis: Control point-% digits bleeding through the translucent scoreboard
into the elims crop. Benign today — `latest_per_game` keeps the final snapshot
and `stats_regressed` needs 2/3 counters dropping (both defenses held; no false
split, final stats correct). Residual exposure: a misread on the game's FINAL
capture locks in the bad value.

**Fix sketch.** Delta-plausibility gate at capture accept: counter jump beyond a
sane rate vs `last_stats`/elapsed (e.g. elims > ~1/5s sustained) => hold the
capture (don't advance `last_stats`) until a following capture corroborates,
else drop it. Keep it one-sided (only inflated jumps) so real stomps never get
rejected. Test with the 07-17 session data (E 3→93→3 must be swallowed; real
progressions must pass).

**Done when.** Replaying the 07-17 captures yields no 93; synthetic
impossible-jump fixture test in CI; real-frame local test untouched.

## 9. LIVE-2: Ilios map never registered — map matching too brittle  [MED, field-observed 07-17]

**Problem.** Live Ilios game 07-17 ~23:35+: map empty in every capture.
Two readers failed independently:
- Vote screen (`detect/match_start.rs:344` `extract_map_names`): **exact
  `contains()` only, no fuzzy** — gate passed (navy_ratio 0.95, VOTE+MAP seen)
  but `maps=[]`. ILIOS = three capital I's, the most OCR-mangled name in the
  pool (`1LIOS`/`IL10S`/`|LIOS` all miss exact match).
- Scoreboard header (`parse.rs::find_map`): has fuzzy pass but
  `FUZZY_MAP_THRESHOLD=0.85` is too strict for a mangled 5-char word — empty
  ALL frames while hero names read fine. Oasis (round glyphs) read clean on
  both paths same night.
- Accolade reader (3rd chance, fires at match end) worked for Oasis but never
  fired a map read at the Ilios game's end — Ilios is permanently mapless; all
  three readers missed.
- 07-18 update: vote reader went 0-for-2 — `maps=[]` ×4 even on "ROUTE 66"
  (which the scoreboard reader then caught on the FIRST parse). The vote path
  is effectively dead as-is; prioritize scoreboard fuzzy + accolade robustness,
  treat vote candidates as a bonus.

**Fix sketch.** (a) Glyph-normalization pass before both matchers: `1→i`,
`|→i`, `l→i` (uppercase context), `0→o`; (b) reuse `fuzzy_match_map` in the
vote path instead of exact contains; (c) consider per-length threshold (short
names need more slack) but keep the King's Row false-positive guard;
(d) debug-dump the map region after N consecutive empty reads so the next
failure is diagnosable from raw pixels.

**Done when.** Synthetic mangled-name fixtures (`1LIOS`, `IL10S`, `0ASIS`,
`BUSVN`…) resolve correctly in a CI test; no false positives on player-name
fixtures; next live Ilios/Control session registers the map.

**Also feeds LIVE-1 (item 8):** same session showed E spikes 93→99 tracking
the control point % (93%/99% pre-overtime) — bleed-through confirmed
recurring, multiple spikes per game. Delta-gate priority raised: a spike on
the final capture is no longer a tail risk.

## 10. LIVE-3: hero attribution from final capture — majority vote instead  [MED, field-observed 07-18]

**Problem.** Lijiang game 07-18 ~00:22: recorded hero=Tracer role=Damage; USER
played Mizuki 89% / Juno 11% (career screen). Per-capture reads were 22×
Mizuki, 5× Tracer, 1× Domina — detection is fine; the aggregation takes the
FINAL capture's hero (latest_per_game), and the last reads were wrong.
Hypothesis for the bad reads: captures during killcam/spectate read the
killer's hero panel (USER died 8×; a Tracer was likely on the killfeed).
Role is derived from hero, so it corrupts together.

**ROOT CAUSE (completed 07-18, deeper than the sketch):** majority vote
ALREADY EXISTS — `storage::majority_hero` runs on every capture
(main.rs:1580) and voted correctly (22× Mizuki). The bug is downstream:
`set_session_hero` (storage/mod.rs:258) updates only `match_session`, while
the GUI history and the sync path read `personal_match` — unlike its siblings
`set_session_map` (:275) and the outcome repair (:300), which both write
through to `personal_match` + `synced=false`. Repair ran; nothing read it.
The bad reads themselves: USER browsing enemy rows post-match — the
panel-keyword disambiguation follows the SELECTED row's details panel by
design (all 6 bad reads timestamped after game end).

**FIX READY — parked branch `fix/tracker-hero-writethrough` @ 4499f36 (on
origin, UNMERGED per USER's diagnose-only directive):** mirrors the :275
write-through, guard on (hero OR role) mismatch to avoid churn; includes a
tokio test modeling the field case (majority-correct + synced straggler →
flip everywhere, exactly one row re-queued for sync). Tests 8/8, fmt/clippy
clean. Night shift: start at REVIEW (grok), dual-agree, merge. Note: already-
closed sessions (the Tracer game itself) only heal if they receive another
capture — they won't; either accept, hand-repair, or add a one-off backfill
in review if grok agrees it's in scope.

## 11. Retracted / non-items

- roster.rs "(public)" comment: **correct as written** (GET has no auth extractor
  by design; data already public via team pages). Claude flagged it wrongly on
  07-17 and retracted on-channel. Do not "fix".

---

## §P. Process upgrades for next fleet session (from fleet-protocol.md artifact)

Adopt at kickoff, before any work item:

1. **Message bodies ≤ ~400 chars** on the fleet log; long content lives in files/
   PRs/commits, log carries the pointer. (Our 07-17 single-thread log hit ~70KB
   and every poll overflowed tool limits.)
2. **Split threads:** `fleet::chat` (ops/presence) + `fleet::<initiative>` per
   work item; retire the monolithic `fleet::channel` for new work.
3. **Finding IDs** (`N1-roster`, `FST-N` style) so CONFIRM/REFUTE is unambiguous.
4. **Presence intents re-published each tick** (TTL ~120s) or leases for long ops.
5. **Watcher health checks + jitter (±20–30s) + backoff ladder** (3m→5m→10m,
   reset on activity, pin to base while any watcher is blind).
6. **Review rounds capped at 2**, then human.
7. **Scuffed-crew Appendix-A bindings:** repo_id `scuffed-crew`; shared checkout
   `/home/soot/github/scuffed-crew` READ-ONLY for agents; worktrees
   `.claude/worktrees/<agent>-<topic>`; SSE `/api/fleet/events` known-dead — poll
   ydoc via MCP; memdb ydoc survived 2/2 daemon restarts on 07-17 but re-derive
   state from git/gh anyway; 13:32Z 07-17 ydoc wipe remains undiagnosed.
