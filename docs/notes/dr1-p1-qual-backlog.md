# DR-1 P1 QUAL — Prioritized Refactor Backlog

Lane: QUAL (metrics/structure only — no security judgments). Author: claude (Opus). Date: 2026-07-19.
Source inventory: `docs/notes/dr1-p0-targets-claude.md` (QUAL section). Seams verified by reading the actual code.
Ranking = impact × safety. OVERNIGHT-OK / PARK-MORNING per grok rule A4 (multi-file god-refactors wait for USER morning; only tiny, get_impact-verified blast lands overnight).

NOTE on overnight scope: even single-file "tiny" refactors in `crates/site-server/routes/*` and `crates/db` are marked PARK-MORNING when those files are under active edit by the AUTH/ACCT/ADMIN/DB fix lanes — a QUAL refactor merged overnight would collide with in-flight security branches. stat-tracker and its own crate are NOT under P1 security waves (grok A3), so single-file stat-tracker refactors can land overnight without collision risk (behavior-preservation + green tests still required).

---

## CORRECTIONS TO THE P0 INVENTORY (verified false positives — do NOT spend refactor budget here)

- **`compute_stats` is NOT forked.** `crates/stat-tracker/src/gui/stats.rs:4` does `use stat_tracker::stats::{HeroMapBreakdown, compute_stats};` — the GUI imports the single shared `compute_stats` (stats.rs:117). The memtrace entry "compute_stats cc32 @ gui/stats.rs:111" is mis-attributed: line 111 is the body of the `StatsPanel` Dioxus render component (rsx! inflates cc), not a second aggregation fn. There is exactly ONE compute_stats. No dedup work exists here. (This was the single largest claimed "dedup win" — it is a non-bug.)
- **`detect/*/open()` repetition does NOT exist.** Only two `open()` fns in the whole crate: `detect/mod.rs:111 MultiKeyboardStream::open` (evdev) and `storage/mod.rs:147 LocalStore::open` (SurrealKV). Unrelated. No cluster.
- **`find_hero` / `fuzzy_match_hero` "4-6 near-identical copies"** is an AST-node artifact. There is one `find_hero` (parse.rs:285) and one `fuzzy_match_hero` (parse.rs:356). The real parallel pair is `fuzzy_match_hero` vs `fuzzy_match_map` (parse.rs:589) — genuine but low-value (see DR1-QUAL-007).

---

## RANKED BACKLOG

DR1-QUAL-NNN | file:line | metric | proposed refactor | blast | overnight? | before→after

**DR1-QUAL-001** | stat-tracker/src/main.rs:1272-1378 | part of handle_capture cc53/459 | Extract the `spawn_blocking` frame-analysis closure verbatim into a named free fn `analyze_frame(matcher, scoreboard-inputs...) -> FrameAnalysisOutcome`. It already returns a clean enum and owns its inputs by move — a mechanical lift. | tiny (single file, private, moves a closure to a fn; stat-tracker not under P1 waves) | **OVERNIGHT-OK** | handle_capture cc53→~38; new analyze_frame cc~22

**DR1-QUAL-002** | stat-tracker/src/main.rs:1247 | handle_capture cc53/459 (workspace worst) | After 001, split the persistence half into seams: `decide_split()` (1525-1546), gate-apply + hold-logging (1548-1578) folded into existing `capture_gate`, `persist_capture()` (session create/append + insert_match, 1596-1638), `record_capture_diagnostics()` (majority-hero refresh 1649-1657 + empty-map dump 1661-1678), portrait auto-collect (1497-1515). handle_capture becomes an ~80-line orchestrator. | small (single file; multiple seams; depends on 001) | **PARK-MORNING** (multi-seam god-fn) | handle_capture cc53→~15; +4 helpers cc≤10 each

**DR1-QUAL-003** | stat-tracker/src/main.rs:99-230 | main() cc26/226 | Extract the CLI-flag short-circuit block (--version/--help/--generate-tessdata/--list-outputs/--vacuum, ~130 lines of early returns) into `fn dispatch_cli_flags() -> Option<anyhow::Result<()>>` (Some ⇒ handled, exit), and the DaemonCtx assembly (270-323) into `build_daemon_ctx(config)`. main() shrinks to load-config → dispatch → build → run_loop. | tiny (single file; stat-tracker not under P1 waves) | **OVERNIGHT-OK** | main cc26→~9

**DR1-QUAL-004** | stat-tracker/src/main.rs:686 | run_loop 561 lines (longest fn in workspace after god-components) | Bundle the ~12 mutable loop locals into a `LoopState` struct, then extract each `tokio::select!` arm into an async method: `on_tab()`, `on_capture_result()`, `on_poll_tick()`, `on_cmd_tick()`. Behavior-preserving but requires the state struct first. | small (single file; needs state-struct plumbing) | **PARK-MORNING** (largest fn; touches concurrency state) | run_loop 561→~120 orchestrator; 4 handlers ~80-140 each

**DR1-QUAL-005** | stat-tracker/src/stats.rs:117 | compute_stats cc39/200 | Extract the three finalize+sort tails into `finalize_heroes(acc)`, `finalize_roles(map)`, `finalize_maps(map)`, `finalize_hero_maps(acc)`, and the rolling-WR loop into `rolling_winrate(matches)`. The accumulation loop stays. Heavily unit-tested (tests mod at :320). | tiny (single file, private; strong test coverage; not under P1 waves) | **OVERNIGHT-OK** | compute_stats cc39→~18; helpers cc≤8

**DR1-QUAL-006** | db/queries/tournaments.rs (1637) | god-file; generate_double_elim_bracket 230 lines | Split the bracket-generation algorithms (`generate_single_elim`/`double_elim`/`round_robin`/`swiss_round` + private `update_match_next`/`update_match_loser_next`, lines ~915-1512) into a `queries/tournaments/brackets.rs` submodule; keep CRUD in the parent. Pure code-motion of impl methods. | medium (single crate, but methods on same impl → module split; DB lane actively reviewing) | **PARK-MORNING** (god-file split + lane collision) | file 1637→~950 + ~690; no per-fn cc change (organizational)

**DR1-QUAL-007** | parse.rs:356 & parse.rs:589 | fuzzy_match_hero cc~9 + fuzzy_match_map cc~9 | Collapse the two into one generic `fuzzy_best(text, candidates: &[&str], threshold) -> Option<String>` (identical window/levenshtein loop; only the table + threshold + log message differ). | tiny (single file, both private) | **OVERNIGHT-OK** (but low value: ~35 lines saved; stat-tracker not under waves) | 2 fns→1 generic; net −35 lines

**DR1-QUAL-008** | stat-tracker/src/storage/mod.rs (1307) | god-file (verify) | Likely mixes LocalStore CRUD + snapshot export + GUI command queue + match-log append. Candidate split into `storage/store.rs` / `storage/snapshot.rs` / `storage/commands.rs`. Confirm responsibility boundaries before splitting. | small-medium (single file/crate; not under waves) | **PARK-MORNING** (god-file split; needs responsibility audit) | file 1307→3×~430 (organizational)

**DR1-QUAL-009** | site-server/routes/{applications,moderation,members,...}.rs | internal_err/bad_request/conflict trio re-declared per route file | Consolidate the copy-pasted error-helper trio into one `routes/error_helpers.rs` (or extend existing) and import. Mechanical, but touches ~6 files across ACCT/ADMIN lanes. | small per file, but multi-file across actively-reviewed lanes | **PARK-MORNING** (merge-collision risk with in-flight security branches) | no cc change; removes ~5 dup blocks

**DR1-QUAL-010** | site-server/routes/{members,profile,game-account}.rs | `double_option` deserializer + omit/null handling repeated | Hoist the shared `double_option` + `normalize_optional_handle` helpers into one module imported by all three. | small per file; multi-file in ACCT lane | **PARK-MORNING** (ACCT lane active) | no cc change; −~3 dup blocks

**DR1-QUAL-011** | app/pages/strategy/editor.rs:420 (EditorLayout 905) · app/components/strategy/map_canvas.rs:186 (MapCanvas 746) · app/pages/admin/settings.rs:13 (AdminSettings 748) | frontend god-components | Behavior-preserving sub-component extraction (rsx! line-count inflation, not true cc). Each needs prop plumbing + signal threading — high review surface. | large (per-component; big diffs, prop wiring) | **PARK-MORNING** (grok A4 explicit) | line-count only; ~900→orchestrator + N child components

**DR1-QUAL-012** | crypto.rs:346 `is_strict_production_crypto` vs env_flags.rs:8 `is_production_env` | duplicated prod-gate match arms | Potential dedup, BUT the two may intentionally diverge (strict-crypto vs general prod gate) and both are security-gating. Do NOT dedup blind — hand to AUTH lane to confirm arms are truly identical before merging. | small but security-sensitive | **PARK-MORNING** (AUTH-owned; correctness-sensitive) | −~10 lines IF arms confirmed identical

**DR1-QUAL-013** | db/src/types.rs (1213) | long-but-cohesive (pure type defs + Display impls) | Optional split into `types/{tournament,member,audit,stats}.rs` with re-exports. Low value: it's cohesive, not a god-file; the only cost is scroll length. | large (re-export churn, imported everywhere) | **PARK-MORNING** (low priority; leave unless USER wants it) | organizational only

---

## OVERSIZED FILES — top 8 verdict (god-file = SPLIT vs long-but-cohesive = LEAVE)

| rank | file | lines | verdict | covered by |
|---|---|---|---|---|
| 1 | stat-tracker/src/main.rs | 2181 | GOD-FILE (daemon loop + capture + CLI + helpers) | 001-004 (fn-level) |
| 2 | db/queries/tournaments.rs | 1637 | GOD-FILE (bracket algos vs CRUD) | 006 |
| 3 | app/pages/strategy/editor.rs | 1565 | GOD-COMPONENT | 011 |
| 4 | app/components/strategy/map_canvas.rs | 1452 | GOD-COMPONENT | 011 |
| 5 | stat-tracker/storage/mod.rs | 1307 | GOD-FILE (verify) | 008 |
| 6 | db/src/types.rs | 1213 | LONG-BUT-COHESIVE (pure types) — LEAVE | 013 (opt) |
| 7 | app/pages/stats.rs | 1080 | LONG-BUT-COHESIVE (rsx page) — LEAVE | — |
| 8 | app/pages/home/css.rs | 1018 | LONG-BUT-COHESIVE (CSS-in-Rust string) — LEAVE | — |

(nostr.rs 1839 excluded — NOSTR lane owns content.)

---

## CONVENTION DRIFT (systemic patterns — QUAL in scope; individual instances belong to other lanes)

- **Per-file error-helper trio** (internal_err/bad_request/conflict) copy-pasted across route files → DR1-QUAL-009. Systemic, not one-off.
- **double_option / optional-handle normalization** duplicated across member/profile/game-account routes → DR1-QUAL-010.
- **Prod-gate logic** duplicated (crypto vs env_flags) → DR1-QUAL-012 (security-sensitive; AUTH confirms).
- Audit-log coverage gaps, missing-toast, refresh-counter misuse: the P0 inventory found these are enumerated per-route in the ADMIN/FRONT lanes; as a PATTERN they are individually-tracked findings, not a single refactor — no systemic QUAL refactor item beyond flagging that audit() is opt-in-per-route (no enforced wrapper), which is the root cause of the coverage holes. Optional future item: a `mutating_route!` macro / middleware that makes audit non-optional — LARGE, PARK.

---

## SUMMARY

- Total items: **13** (+3 P0 false-positive corrections that remove work).
- OVERNIGHT-OK: **4** (001, 003, 005, 007) — all single-file stat-tracker fn extractions, not under P1 security waves.
- PARK-MORNING: **9** (002, 004, 006, 008, 009, 010, 011, 012, 013).
- Highest value: 001/002 (handle_capture, the cc53 workspace worst), 003 (main), 004 (run_loop).
