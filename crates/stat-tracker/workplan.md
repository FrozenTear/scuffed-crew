# Stat Tracker — Parallel Work Plan (Fable + Grok)

**Source backlog:** `review.md` (canonical — item IDs below refer to it)
**Model:** two agents in parallel on disjoint file sets, hard checkpoint between phases, mandatory cross-review of every diff.

## Split rationale

- **Fable lane = correctness-critical, `main.rs`-centric.** The sync contract and session state machine are where subtle-but-wrong implementations are cheap to write and expensive to discover (wedged sync, misattributed outcomes). This lane also concentrates *all* `main.rs` edits in one agent so there is exactly one writer of the hardest file.
- **Grok lane = precisely-specified mechanical wins in GUI/OCR/capture.** The backlog already contains the design for these (gate this, cache that, memoize this) — execution speed dominates, verification risk is low, and none of it touches `main.rs`, `parse.rs`, or the sync path.

## File ownership (hard boundary — do not cross without checkpoint)

| Owner | Files |
|---|---|
| **Fable** | `src/main.rs`, `src/parse.rs`, `src/sync/`, `src/storage/mod.rs` (queries/session/sync fns), `src/detect/mod.rs` (`MatchOutcome`), `crates/types/src/api/stats.rs`, `crates/db/` (migrations + `personal_stats.rs`), server route for stats upload |
| **Grok** | `src/gui/**`, `src/ocr/**`, `src/capture/**`, `src/detect/hero_portrait.rs` (portrait geometry fn only), new `src/stats.rs` (extracted compute_stats), `Cargo.toml` dep trims |
| **Shared — coordinate** | `src/config.rs` (Grok adds the debug-OCR flag only; Fable adds nothing in Phase 1), `src/lib.rs` (module registrations — trivial, resolve at merge) |

GUI outcome-string parsing (`gui/stats.rs:108`, `gui/history.rs`, `gui/status.rs`) is **frozen in Phase 1**: Fable introduces typed `MatchOutcome` in the lib; Grok adopts it in the GUI in Phase 2. Grok must not "fix" those literals independently.

---

## Phase 1 (parallel, ~independent branches)

### Fable — branch `stat-tracker/fix-sync-contract`

Order matters (each step builds on the previous):

1. **Sync contract (P0 #1 = C1 + C2 + M4):**
   - `session_id` (or client-generated match id) added to `StatsUploadEntry`; server upsert per session; relax/handle the `unknown` ASSERT server-side or filter client-side (`get_unsynced` excludes `outcome = 'unknown'`).
   - Replace the dead `contains("unique")` guard with structured `IndexExists` matching in `bulk_insert_personal_matches`.
   - Map/outcome corrections become server upserts (dedup key must not include mutable fields).
   - `reqwest` client timeout (30s); move `try_sync` off the select loop (spawned task).
   - `mark_synced` by record ids, not count (M7 — same code, do it here).
2. **Thin enablement slice (P1 #6):** `MatchOutcome`: `Display`/`FromStr`/serde in the lib (delete `outcome_str`/`outcome_from_str`); `FrameAnalysis` struct replacing the 9-tuple. No behavior change — pure typing.
3. **Session-boundary fixes (P1 #7, #10):** staleness bound on unfinished sessions (M1); persist/recover open-session skeleton (M10); pending-outcome apply-or-clear on poller open + streak resets + mid-game Tab banner guard (M5); session create/insert half-failure handling (M6); zero-output startup panic (m1 — it's in `main.rs`).
4. **Wrong-data fixes (P1 #8, #9):** map-vote as candidates only + canonicalization through the MAPS table (M2); Ana/HAVANA word-boundary + tie-break fix (M3) — both in `parse.rs`/`main.rs`.
5. **Un-serialize the daemon loop (P0 #3 = P1) + lazy full `recognize()` (P3):** spawn `handle_capture` with single-flight, results via channel; make the full-image OCR conditional. Done last because it restructures `handle_capture`, which steps 2–4 edit.

### Grok — branch `stat-tracker/perf-gui-lifecycle`

No internal ordering constraints; suggested by value:

1. **C3 force-clear guard (P0 #2):** disable clear while daemon runs (or stop→clear→restart) + confirm dialog. `gui/settings.rs`, `gui/daemon.rs`.
2. **Gate debug OCR PNGs (P0 #4 = P2):** env/config flag (add to `config.rs`), save already-computed intermediates when enabled, debug dir passed from caller. `ocr/mod.rs`, `ocr/preprocess.rs`.
3. **Calibration cache + early-exit (P4):** `ocr/mod.rs` only.
4. **Wayshot connection reuse (P5):** `capture/wayshot.rs`.
5. **GUI store-handle caching + snapshot mtime skip (P10) and shared `use_live_matches()` hook (A6):** `gui/*.rs`.
6. **Settings "restart required" toast (M8)** + real capture-backend label on dashboard (A9 item): `gui/settings.rs`, `gui/status.rs`.
7. **GUI memoization sweep (P12):** `use_memo` on MatchesPanel, DaemonCard set-on-change + `tokio::process`, tray blocking recv, `use_resource` for Wayland calls.
8. **Extract `compute_stats` to lib + unit tests (A7):** new `src/stats.rs`; GUI calls it. (Keep outcome strings as-is until Phase 2.)
9. **Shared portrait geometry fn (A2, geometry half):** add `portrait_rect(dims, row_idx, team_size)` to `hero_portrait.rs` with tests. Do **not** update the `main.rs` call site — Fable wires it in during step 3 of their lane (it's inside `handle_capture`).

### Explicitly deferred to Phase 2 (conflict-bound)

- **P6 single-RGB-per-poll** — changes `detect/match_start.rs`/`match_end.rs` signatures *and* the `run_loop` poll arm; must land after Fable's loop restructure.
- **GUI adoption of typed `MatchOutcome`** — after Fable's lib change merges.
- **P7 pass-crop/team-size-once** — depends on Fable's `handle_capture` restructure.
- **P11 snapshot debounce** — `storage/mod.rs` + `main.rs` call sites; Fable's files, low urgency.
- **A4 anyhow migration, A1 full `DaemonCtx` split, P2-tier hygiene** — after both branches merge.

---

## Merge & review protocol

1. **Merge order: Fable first** (heavy `main.rs` diff), then Grok rebases — GUI/OCR/capture files won't conflict; `config.rs`/`lib.rs` resolve trivially.
2. **Mandatory cross-review before each merge:** Grok reviews Fable's diff, Fable reviews Grok's diff. This exercise demonstrated each catches confirmed issues the other misses — cross-review is the cheapest insurance in this plan. Reviews check against `review.md` item specs *and* the "What is already good (do not casually undo)" section.
3. **Checkpoint after Phase 1 merge:** re-run the full test suite + `#[ignore]`d fixture replays locally; then plan Phase 2 (deferred items above).
4. **Verification gates:**
   - Fable lane: dev-mode server round-trip (in-memory DB, `/api/dev/login`) proving multi-Tab game → exactly one server row, `unknown` never uploaded, back-fill updates (not duplicates) the server row, retry after simulated partial upload doesn't wedge.
   - Grok lane: before/after timing of a Tab capture via `examples/extract.rs` or the profile tool (expect multi-second improvement from P2+P4 alone); manual GUI pass for clear-data guard + settings toast.
   - Both: live game-test checklist (evening session) covers the merged result.

## Notes

- Neither lane blocks the other; the only cross-lane dependency (portrait geometry fn ↔ call site) is handled by Grok exporting the function and Fable wiring it.
- If Grok finishes early, P13 micro items (`OnceLock` for `tessdata_lang`, `encode_png` by-ref, `detect_column_offset` crop-first, release-profile LTO, dep trims) are all in Grok-owned files and safe to pull forward. `read_result_word` brightness pre-gate (P8) is in `detect/match_end.rs` — **not** safe to pull forward (Fable file, Phase 2).
- Server-side changes (types/db/server) are Fable-only; do not parallelize schema edits.
