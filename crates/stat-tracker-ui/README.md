# scuffed-stat-tracker-ui

Native **Iced 0.14** (wgpu) tracker GUI for the Scuffed Crew stat tracker.

- Package: `scuffed-stat-tracker-ui`
- Binary: `stat-tracker-gui` (same name the Dioxus GUI uses, so install paths stay compatible)
- Reads the daemon snapshot via `stat_tracker::storage::read_snapshot` (`live_snapshot.json`)
- Writes `StoreCommand` files under `<data_dir>/commands/` — same contract as the Dioxus GUI (`SetOutcome`, `EditMatch`, `DeleteSession`, `ResolveSegment`)
- Fetches `GET /api/public/seasons` on launch and every 30 minutes; cache is `<data_dir>/seasons.json`. Header season choice persists in `<data_dir>/ui_state.json`.
- Does **not** change the daemon, OCR, capture, sync, or store schema
- Does **not** open SurrealKV; the running daemon applies queued commands and refreshes the snapshot

Design: `docs/notes/stat-tracker-gui-redesign-2026-09-02.md` rev 3 (branch `docs/tracker-gui-redesign`). Tokens: `radius-card` 16, `radius-inner` 12, page pad 24/32, Urbanist, role tints, `border` / `text-3` / `ok` / `danger`.

P0 Overview is on `main` (PR #56). P1 Games / Heroes / Maps + StoreCommand writes + rev 3 responsive layout is on `main` (PR #57). This crate is **P2**: Seasons screen, cache, persistence, aggregation parity.

There is **no** software `--preview` path and no `preview.rs`. Robert will capture real niri window shots (empty + sample + live) for Design.

## Run fixtures

```sh
# Empty Overview / Games / Heroes / Maps / Seasons (no season picker)
cargo run -p scuffed-stat-tracker-ui -- --fixture empty

# Sample matches + seasons (picker stays; defaults to the current season)
cargo run -p scuffed-stat-tracker-ui -- --fixture sample
```

`--fixture` never fetches `GET /api/public/seasons`. It writes a demo `live_snapshot.json` (and sample `seasons.json`) under a temp dir, then reads it back through `storage::read_snapshot`.

Override the dir if you want to inspect command files / `ui_state.json` afterwards:

```sh
cargo run -p scuffed-stat-tracker-ui -- --fixture sample --data-dir /tmp/sst-ui-sample
ls /tmp/sst-ui-sample/commands/   # after using Games → expand a card → Victory / Edit / Delete
cat /tmp/sst-ui-sample/ui_state.json
```

## Live (daemon running)

```sh
cargo run -p scuffed-stat-tracker-ui
# or pin the same data dir the daemon uses:
cargo run -p scuffed-stat-tracker-ui -- --data-dir "$HOME/.local/share/scuffed-stat-tracker"
```

### Seasons

- **Screen:** sidebar **Seasons** lists each server season (name, UTC window, current marker) plus an **All time** row, each with games / W–L–D / win rate. Selecting a row sets the header season switch. Read-only — no admin CRUD in the desktop app (`/admin/seasons` on the website).
- **Fetch:** live mode GETs `/api/public/seasons` on launch and every 30 minutes. Written to `<data_dir>/seasons.json`. Offline uses that cache and shows **Last refreshed … · cached** on the Seasons screen.
- **Selection:** default = the season marked `is_current`, else all time. Persisted in `<data_dir>/ui_state.json`. No seasons → picker hidden (same as the website).
- **Window:** `starts_at <= played_at < ends_at` (UTC), matching `crates/db/src/queries/personal_stats.rs` (`played_at >= $season_start AND played_at < $season_end`).

### How to verify season numbers against the website

Acceptance: for a **synced** account, season totals on this screen equal My Stats on the site.

1. Sync so daemon `live_snapshot.json` rows with `synced: true` match server `personal_match` for that member. The desktop aggregate also includes **unsynced** local games — subtract those (or sync first) before comparing.
2. `GET /api/public/seasons` — same list the GUI caches. Pick a season id (or omit for all time).
3. Website: **My Stats** (or `/stats/member/:id`) → same season in the picker. Or:

   ```sh
   curl -sS "$SCUFFED_SERVER/api/public/seasons"
   # session cookie required:
   curl -sS "$SCUFFED_SERVER/api/stats/me?season=$SEASON_ID"
   # or /api/stats/member/$MEMBER_ID?season=$SEASON_ID
   ```

4. Compare **games** (`total_matches`), **wins**, **losses**, **draws** to the Seasons-screen row. A game on `ends_at` belongs to the *next* season (half-open).
5. **Win rate:** the site shows `wins / total_matches`. This GUI uses `wins / (wins + losses + draws)` (unknown outcomes do not dilute — P0/P1). They match when every counted game has a decided outcome.

### StoreCommand round-trip

1. Start the tracker daemon as usual (`stat-tracker` / systemd unit). It owns SurrealKV and polls `<data_dir>/commands/`.
2. Start this GUI against the same `--data-dir` (or the config/XDG default).
3. Open **Games**, click a card so it expands, then:
   - **Victory / Defeat / Draw** → `StoreCommand::SetOutcome`
   - **Edit stats** → change a field → **Save corrections** → `StoreCommand::EditMatch`
   - **Delete session** (click twice) → `StoreCommand::DeleteSession`
   - Hero timeline **Confirm / Dismiss** → `StoreCommand::ResolveSegment` (`confirm` / `dismiss`)
4. The GUI writes `cmd_<nanos>_<pid>.json` via `storage::queue_command` (tmp + rename).
5. The daemon applies the file, deletes it, and rewrites `live_snapshot.json`. The GUI polls mtime at 1 Hz and refreshes.

Inspect a queued file before the daemon eats it:

```sh
# stop the daemon briefly, or copy quickly
cat "$HOME/.local/share/scuffed-stat-tracker/commands/"cmd_*.json
```

Example payload (same tagged JSON the Dioxus GUI wrote):

```json
{"op":"set_outcome","session_id":"sess-t1","outcome":"defeat"}
```

Fixture mode still writes those files (under the fixture `--data-dir`); there is no daemon to apply them unless you point `--data-dir` at a live tree.

### Flags

| Flag | Meaning |
|------|---------|
| `--data-dir PATH` | Daemon data dir (default: config / XDG; temp dir when `--fixture` is set) |
| `--fixture empty\|sample` | Write a demo `live_snapshot.json` and read it back through `storage::read_snapshot` |
| `--seasons-url URL` | `GET /api/public/seasons`; cached to `<data_dir>/seasons.json`. Ignored with `--fixture`. |

Seasons URL falls back to `SCUFFED_SERVER` or `config.toml` sync URL (live mode only). Offline: cache only. No seasons → picker hidden, all time.

## Out of scope (later phases)

P3 companion overlay, P4 settings/tray, P5 remove the Dioxus `gui` feature.

Urbanist (OFL) is bundled as the labelled tracker product face — see `fonts/OFL.txt`.
