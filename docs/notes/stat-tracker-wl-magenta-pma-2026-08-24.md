# Stat-tracker W/L — magenta PMA + missed EndTitle (grok → claude review)

**Date:** 2026-08-24  
**Author:** grok (Grok 4.6)  
**Repo:** scuffed-crew @ `origin/main` `1e37529` (live daemon **0.3.3**)  
**Request (USER):** tracker only records losses. Rialto tonight was also a win. Two surfaces: huge center EndTitle at game end, small top-left in the PMA lobby. Figure out a way to deal with that. **Design locked A-only after claude review (PR #43). No detector code in this branch.**

**Fleet:** `fleet::tracker-wl` (detail) + `fleet::chat` (pointer). Branch: `docs/tracker-wl-magenta-pma`.

**Do not commit** anything under `crates/stat-tracker/test-data/` or live `debug/` captures. Frames cited below live only on the host data dir.

---

## 1. What USER sees

Decided outcomes since 2026-08-20 are **defeat-only**. Last stored victory: 2026-08-19 (Ball, King’s Row, 34/5). Live unit `scuffed-stat-tracker.service` since 2026-08-21 21:31, binary 0.3.3.

Tonight (local 2026-08-24, after midnight) three sessions, all `unknown` in `matches.jsonl`. USER confirms **two were wins**.

---

## 2. Tonight’s evidence (host)

Data dir: `~/.local/share/scuffed-stat-tracker/`. Journal: `scuffed-stat-tracker.service`.

| Local | Map | Last board | Store | What the debug PNG actually is |
|---|---|---|---|---|
| 00:01–00:12 | Runasapi | Ball **21/5** | unknown | **Win.** `debug/rejected/rejected_preflight_20260824_001227.png` is PMA cards: magenta **VICTORY** top-left, Runasapi, 10:00, Frozen 21. Tab path logged `outcome=Unknown`, not adopted. |
| 00:14–00:22 | Rialto | Ball **5/2** (split sessions) | unknown | **Win (USER).** No Victory text on disk. 00:22:55 in-game scoreboard → 00:22:57 3D spawn (no title) → 00:23:02 POTG. |
| 23:48 (23rd) | Lijiang | — | **defeat** | EndTitle **DEFEAT 1 \| 2**. OCR `DEFERT`, fuzzy-matched, adopted from rejected preflight. |

Contrast that works: `result word: DEFEAT text=DEFERT context="end title"`. Zero `result word: VICTORY` since this daemon start.

### PMA crop probe (Runasapi 001227)

16:9 accolade crop as `read_result_word` does (`5,35,250,60` /1000) is **correct** — the word is in the crop.

| Path | Result |
|---|---|
| System tesseract 5.5.3 on **raw color** crop, PSM 7, A–Z | `VICTORY` |
| Rec.601 luma → Otsu (current `prepare_title`) then tess | `MATCHTIE` (white **MATCH TIME**, magenta dropped) |
| Crop mean RGB | `(44, 31, 61)` — magenta has low luma because Rec.601 weights green |

`title_crop_has_signal` **passes** (~2.6% lit). Failure is the binarizer, not the crop gate.

EndTitle (`prepare_end_title` + `opponent_ink` + `fuzzy_outcome_word`) was **never fed a victory frame** tonight. Poller skips the whole tick while Tab OCR is in flight (`main.rs` ~1276, H3 / FPS). Slow cadence is 4s × `SLOW_POLL_DIVISOR=2` = 8s; EndTitle lasted ~2s on Rialto.

---

## 3. Two holes

1. **PMA top-left (long-lived).** Magenta title is invisible to luma Otsu. Yellow DEFEAT is bright and survives. Accolade/rank still use exact `contains("VICTORY")`; only EndTitle is fuzzy. This is why Tab-on-PMA (Runasapi) still stored unknown.
2. **Huge center EndTitle (short).** Detector is fine for yellow (and is hue-locked for magenta **if it sees the frame**). Tab-on-scoreboard at match end blocks the poller for the exact window. Next Tab is spawn/POTG with no word (Rialto).

Do **not** infer W/L from a 5/2 scoreboard or from 3D spawn with no text.

---

## 4. Approaches (for claude to pick / refute)

**A. Magenta-safe PMA OCR only.** Change `prepare_title` gray to `max(R,G,B)` (or reuse EndTitle `opponent_ink`). Run `fuzzy_outcome_word` from `ocr_outcome_word`. Keep C6 `trim_to_tall_glyphs`.  
Fixes Runasapi-class. Leaves Rialto-class if USER never lands on PMA cards.

**B. Extra in-match peeks while Tab is in flight.** Catches the 2s EndTitle. Reopens the FPS/fuzzy hitch (compositor readback on the game output). USER already paid for PR-A/B to stop that.

**C. A + post-match-only full cadence (original grok rec; REFUTED — §8).** After a non-scoreboard capture while outcome is `unknown`, keep cheap outcome polls at 4s until decided or the game process is gone. Does **not** cover Rialto (blocking Tab was an accepted in-game board) and **does** re-enable H3 for the whole match (hero-select Tabs look like the same reject). Do not ship..

---

## 5. Locked design (A-only — see §8)

1. **`prepare_title`:** grayscale from `max(R,G,B)` instead of Rec.601 luma. Magenta and yellow both become ink; dark HUD stays dark. Do **not** change `prepare_end_title` (already hue-locked). Do **not** reuse `opponent_ink` here — it zeros white (`(r+b)/2−g` on 255,255,255) and `prepare_title` also feeds `scoreline_looks_present` + `read_accolade_map`.
2. **Classifier:** `ocr_outcome_word` (accolade + rank) uses `fuzzy_outcome_word` for VICTORY/DEFEAT. **DRAW** on this path: exact `contains("DRAW")` or lev≤1 — Tab has no sat-mass gate and no 2-read confirm, and `len+1 >= 4 && lev<=2` accepts 3-letter garbage (`DRA`/`RAW`/`DAG`). EndTitle keeps the existing fuzzy (gated).
3. **Cadence:** **no change.** Do not un-skip poll-during-Tab or drop the slow divisor based on a non-scoreboard reject (C is dead; see §8).
4. **Tests (no live PNGs in git):**
   - Synthetic magenta-on-dark “VICTORY” crop → `Victory` through `prepare_title_trimmed` + `fuzzy_outcome_word`.
   - Synthetic yellow-on-dark “DEFEAT” still `Defeat`.
   - Title + smaller map/time to the right still trims (C6).
   - DRAW 3-letter fragments stay `Unknown` on the accolade path.
   - Optional `#[ignore]` replay of the host PMA PNG via existing `tests/outcome_fixtures.rs` (gitignored dir) — covers tess4 vs the tess5 probe.
5. **Out of scope:** back-filling tonight’s rows; inferring W/L from stats; in-match extra screencopies; poll-cadence changes; bumping 0.3.3 in the implementation PR.

**Files (expected):** `crates/stat-tracker/src/ocr/preprocess.rs` (`prepare_title`), `crates/stat-tracker/src/detect/match_end.rs` (`ocr_outcome_word` / DRAW tightness), tests next to those. Example `probe_outcome.rs` if it still assumes luma. **Not** `main.rs` poll skip.

---

## 6. Review asks (claude)

Please CONFIRM/REFUTE with file:line. Rubric:

- **R1.** max-channel vs `opponent_ink` for `prepare_title`. Grok prefers max-channel (one line, theme-agnostic, measured on 001227). Opponent-ink is already calibrated on EndTitle magenta; duplicating it into every title crop may be cleaner or may overfit hue 300.
- **R2.** Scope C vs A-only for the first PR. A is smaller and unblocks PMA Tab. C is what actually covers Rialto *if* a title screen still appears after spawn.
- **R3.** False-positive risk: max-channel + fuzzy on the **in-game** top-left HUD crop (nameplate, “FROZEN”). `title_crop_has_signal` + stability (poll) + unique lev≤2 to VICTORY/DEFEAT/DRAW should still reject — please try to break that.
- **R4.** Must not regress yellow EndTitle `DEFERT` or C6 trim (`DEFEATJT` / empty on mixed line).
- **R5.** Poll-skip during Tab was H3 (FPS). Gating the skip on “already post-match” must not reintroduce in-match double-screencopy. If that’s fragile, say so and we ship A-only.

**READ:** this note; `match_end.rs` `prepare_end_title` / `ocr_outcome_word` / `fuzzy_outcome_word`; `preprocess.rs` `prepare_title` / `prepare_title_trimmed`; `main.rs` poll skip + `SLOW_POLL_DIVISOR`. **RUN:** nothing required for a design verdict; optional: tesseract the host PMA crop. **NOT checked:** tess4 vs system tess5 on a max-channel binary (live daemon ships tess4).

---

## 7. Local pointers (host only)

```
~/.local/share/scuffed-stat-tracker/debug/rejected/rejected_preflight_20260824_001227.png  # PMA VICTORY
~/.local/share/scuffed-stat-tracker/debug/rejected/rejected_preflight_20260823_234801.png  # EndTitle DEFEAT
~/.local/share/scuffed-stat-tracker/debug/rejected/rejected_preflight_20260824_002257.png  # Rialto 3D spawn, no title
```

---

## 8. Claude verdict (2026-08-24, PR #43 comment + `fleet::tracker-wl`)

**APPROVE note, ship A-only.** Grok ACK: C is wrong as specified.

| Ask | Verdict | Why (verified against host frames, not just the comment) |
|---|---|---|
| R1 max-channel | **CONFIRM** | 001227 luma=`MATCHT` → Unknown; max=`VICTORY`. `opponent_ink` zeros white; would break FINAL SCORE + map OCR. |
| R3 FP | **CONFIRM** | Claude: 400 crops (200 frames × accolade+rank), 0 false decided under max+fuzzy. |
| R4 yellow/C6 | **CONFIRM** | DEFEAT still reads; EndTitle untouched so `DEFERT` stands. |
| R2/R5 approach C | **REFUTE** | (1) Rialto blocker was **accepted** `accepted_20260824_002255`, not a reject — C’s gate flips after the 2s title is gone. (2) Gate fires at **match start**: `rejected_noplayerrow_20260824_000115` / `_001421` are TIME 0:00 spawn/hero-select (`Select your hero` on 001421). Sticky until finished = full-cadence + un-skip **all match** = H3 regression. (3) PMA/rank 15–20s+ at 8s cadence is enough once A can read magenta. |
| DRAW nit | **TAKE** | Tight DRAW on Tab/accolade path only. |

Probe used tess5; live daemon is tess4 — `#[ignore]` fixture replay is the coverage for that gap.

Implementation follows §5 (A-only). Author does not merge this docs PR.
