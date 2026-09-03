# scuffed-stat-tracker-ui

Native **Iced 0.14** (wgpu) tracker GUI for the Scuffed Crew stat tracker.

- Package: `scuffed-stat-tracker-ui`
- Binary: `stat-tracker-gui` (same name the Dioxus GUI uses, so install paths stay compatible)
- Reads the daemon snapshot via `stat_tracker::storage::read_snapshot` (`live_snapshot.json`)
- Writes `StoreCommand` files under `<data_dir>/commands/` — same contract as the Dioxus GUI (`SetOutcome`, `EditMatch`, `DeleteSession`, `ResolveSegment`)
- Fetches `GET /api/public/seasons` on launch and every 30 minutes; cache is `<data_dir>/seasons.json`. Header season choice persists in `<data_dir>/ui_state.json`.
- Settings write `~/.config/scuffed-stat-tracker/config.toml` through `Config::save` (0600). Same keys as the daemon / Dioxus page.
- Capture preview is a one-shot screenshot of the selected output through the existing capture backends.
- Tracker service start/stop/restart goes through the `scuffed-stat-tracker.service` user unit when installed. PID liveness requires `/proc/<pid>/comm` to start with `scuffed-stat` — a reused PID is never signalled.
- Does **not** change the daemon, OCR, capture, sync, or store schema
- Does **not** open SurrealKV for match edits; the running daemon applies queued commands and refreshes the snapshot. Compact / delete on Settings open the store only after the service is stopped.

Design: `docs/notes/stat-tracker-gui-redesign-2026-09-02.md` rev 2 tokens + rev 3 responsive layout (branch `docs/tracker-gui-redesign`). Tokens: `radius-card` 16, `radius-inner` 12, page pad 24/32, Urbanist, role tints, `border` / `text-3` / `ok` / `danger`.

P0–P2 are on `main` (PRs #56–#58). **P4** (PR #59) is Settings, capture preview, daemon control, update banner, tray. **P3** is the companion overlay (this crate, `iced_layershell` 0.19).

There is **no** software `--preview` path and no `preview.rs`. **Robert will capture real niri window shots for Design** (empty + with-data cadence, overlay on/off).

## Run fixtures

```sh
# Empty Overview / Games / Heroes / Maps / Seasons / Settings (no season picker)
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
5. **Win rate:** the site shows `wins / total_matches`. This GUI uses `wins / (wins + losses + draws)` (unknown outcomes do not dilute — P0/P1). They match when every counted game has a decided outcome. The Seasons row appends `· N undecided` when the counts differ so the percentage is readable. **Robert to decide** whether the site should switch to decided-only (recommended by the P2 review) before P5.

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
| `--companion` | Run the layer-shell overlay only (needs niri / wlr-layer-shell). |
| `--seasons-url URL` | `GET /api/public/seasons`; cached to `<data_dir>/seasons.json`. Ignored with `--fixture`. |

Seasons URL falls back to `SCUFFED_SERVER` or `config.toml` sync URL (live mode only). Offline: cache only. No seasons → picker hidden, all time.

### Settings and tracker service

Live mode only. `--fixture` shows the Settings page but does not write `config.toml`, start/stop the service, compact, or delete data.

```sh
# same data dir the service uses (XDG default if omitted)
cargo run -p scuffed-stat-tracker-ui -- --data-dir "$HOME/.local/share/scuffed-stat-tracker"
```

1. Sidebar **Settings**.
2. **Tracker service** — Start / Stop / Restart. When `~/.config/systemd/user/scuffed-stat-tracker.service` exists, verbs go through `systemctl --user`. Otherwise Start launches `scuffed-stat-tracker` from PATH / next to this binary; Stop reads `daemon.pid` and signals only if `/proc/<pid>/comm` is the tracker.
3. **Start on login** — `systemctl --user enable --now` / `disable --now` when the unit file is installed.
4. **Capture** — pick a monitor (or Auto), then **Capture now** for a one-shot preview of what the tracker sees.
5. **Save settings** writes today's fields: monitor, in-game name (`player_name` — no `#1234` discriminator), session window, game process names, auto-detect, website URL + token, debug images. Restart the service after a save if it is running.
6. **Scoreboard reading** — Install copies `koverwatch.traineddata` if missing (`ensure_koverwatch_tessdata`). Rebuild trains it again (`regenerate_koverwatch_tessdata`).
7. **Stored data** — Compact (`LocalStore::vacuum`) or delete all local matches. Both refuse while the service is running.
8. **Update banner** (Overview + Settings) appears only when GitHub has a newer `stat-tracker-v*` release. It never downloads an installer. The "you're on" version is `SST_RELEASE_VERSION` (runtime or compile-time, set by release CI / packaging) or `scuffed-stat-tracker --version` from the installed daemon — never this crate's `0.1.0`. If none of those resolve, the banner stays hidden.
9. **Tray** — Show window / Hide window / Hide / show overlay / Quit (`tray-icon`). Left-click shows the window. The GUI runs as `iced::daemon` so the process stays alive with no window. Hide is `window::close` (destroys the surface — `Mode::Hidden` / minimize stay in niri Alt-Tab); Show is `window::open` and rebinds `window_id`. App state (screen, settings draft, overlay session hold, etc.) is kept in the daemon. Overlay hide is session-scoped: it sticks until the game ends, then the next launch auto-shows.

```sh
systemctl --user status scuffed-stat-tracker.service
systemctl --user start scuffed-stat-tracker.service
# after Save settings:
systemctl --user restart scuffed-stat-tracker.service
```

### Companion overlay (P3)

Layer-shell surface (`iced_layershell` 0.19, pinned with Iced 0.14): `Layer::Overlay`, top-right, margins 24, width 360, height to content, `KeyboardInteractivity::None`, exclusive zone 0, output = `capture_output` from config (same monitor the daemon captures). Clicks and keys pass through (`events_transparent`) so fullscreen Overwatch keeps input.

**Visibility:** auto-show while the **game process** is running (configured `game_process_names` in `/proc`). A leftover `active_game.json` does **not** count as live — the daemon keeps that file after a match. While the process is up, the file may supply a `session_id` only for an unfinished game inside `session_window_secs`; decided outcomes and stale `last_activity` are ignored. Empty process names → not running. Header / Overview health / tray **Hide / show overlay** hides it until the **process ends** — it does not reopen mid-session (new match ids keep the hold). Next launch auto-shows. The hold is `overlay_hidden_key` in `<data_dir>/ui_state.json`. Esc is N/A (`KeyboardInteractivity::None`).

The overlay is a **second process**, not a second window on `iced::daemon`:

```
stat-tracker-gui                 # iced::daemon + tray (P4)
  └── stat-tracker-gui --companion --data-dir …   # iced_layershell overlay
```

That keeps Hide/Show / zero-window tray behaviour from #59 unchanged. Two iced event loops cannot share one process (winit vs layershellev).

```sh
# Overlay only (layout / niri acceptance). Fixture treats the game as running.
cargo run -p scuffed-stat-tracker-ui -- --companion --fixture empty
cargo run -p scuffed-stat-tracker-ui -- --companion --fixture sample

# Main window; overlay auto-shows when the game is running
cargo run -p scuffed-stat-tracker-ui -- --data-dir "$HOME/.local/share/scuffed-stat-tracker"
```

Header chip **Companion showing / hidden / waiting for game**. Tray **Hide / show overlay** does the same. Hide while the game is live sticks until that process ends. `--fixture` counts as one live session so Design can hide/show for shots.

KDE blur (`org_kde_kwin_blur`) is a follow-up, not v1. The panel is translucent (`surface` @ 88%).

`iced_layershell` is the default `companion` feature. If it cannot build on a host: `--no-default-features` keeps the main window; `--companion` then prints a rebuild hint.

**Robert:** live niri acceptance — overlay sits above fullscreen Overwatch; the game keeps keyboard and pointer. Design shots: empty + with-data.

## Out of scope (later phases)

P5 remove the Dioxus `gui` feature. Compositor blur on the overlay.

Urbanist (OFL) is bundled as the labelled tracker product face — see `fonts/OFL.txt`.
