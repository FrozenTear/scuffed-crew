# scuffed-stat-tracker-ui

Native **Iced 0.14** (wgpu) tracker GUI for the Scuffed Crew stat tracker.

- Package: `scuffed-stat-tracker-ui`
- Binary: `stat-tracker-gui` (the only desktop UI entry; install paths and the `.desktop` file stay this name)
- Reads the daemon snapshot via `stat_tracker::storage::read_snapshot` (`live_snapshot.json`)
- Writes `StoreCommand` files under `<data_dir>/commands/` (`SetOutcome`, `EditMatch`, `DeleteSession`, `ResolveSegment`)
- Fetches `GET /api/public/seasons` on launch and every 30 minutes; cache is `<data_dir>/seasons.json`. The season choice (the season control on the stats screens) persists in `<data_dir>/ui_state.json`.
- Settings write `~/.config/scuffed-stat-tracker/config.toml` through `Config::save` (0600). Same keys as the daemon.
- Capture preview is a one-shot screenshot of the selected output through the existing capture backends.
- Tracker service start/stop/restart goes through the `scuffed-stat-tracker.service` user unit when installed. PID liveness compares `/proc/<pid>/exe` to `scuffed-stat-tracker` (and the pid file's start time when present). A reused PID is never signalled.
- Does **not** change the daemon, OCR, capture, sync, or store schema
- Does **not** open SurrealKV for match edits; the running daemon applies queued commands and refreshes the snapshot. Compact / delete on Settings open the store only after the service is stopped.

Design: `docs/notes/stat-tracker-gui-redesign-2026-09-02.md` rev 2 tokens + rev 3 responsive layout (branch `docs/tracker-gui-redesign`). Tokens: `radius-card` 16, `radius-inner` 12, page pad 24/32, Urbanist, role tints, `border` / `text-3` / `ok` / `danger`.

0.6 Design: the Scuffed Tracker 0.6 redesign spec FrozenTear approved (variant A, top tabs). The 0.6 layout puts the tabs across the top (Overview, Games, Heroes, Maps, Seasons, Settings) with the tracker status and a "need a look" chip at the right, and a new Settings page. Every tab has its 0.6 screen since 0.6.0-alpha.1.

P0 to P5 are on `main` (PRs #56 to #61). **v0.4.0** is the first Iced-only tracker
release (Overview / Games / Heroes / Maps / Seasons / Settings + companion
overlay; Dioxus GUI removed). Binary name stays `stat-tracker-gui`.

There is **no** software `--preview` path and no `preview.rs`. **FrozenTear will capture real niri window shots for Design** (empty + with-data cadence, overlay on/off).

## Layout

The 0.6 top tabs layout is the default. `SCUFFED_TRACKER_LAYOUT=classic` starts the 0.5 layout instead: the left sidebar, the season and companion controls in the header, and the old Settings cards with **Save settings**. The variable is read once at start; any other value (or none) gives the top tabs, and a value that is set but is not `classic` or `tabs` is logged as a warning.

```sh
SCUFFED_TRACKER_LAYOUT=classic cargo run -p scuffed-stat-tracker-ui -- --fixture sample
```

## Run fixtures

```sh
# Empty Overview / Games / Heroes / Maps / Seasons / Settings (no season picker)
cargo run -p scuffed-stat-tracker-ui -- --fixture empty

# Sample matches + seasons (picker stays; defaults to the current season)
cargo run -p scuffed-stat-tracker-ui -- --fixture sample

# 857 games over three seasons, for the 0.6 Maps and Seasons shots
cargo run -p scuffed-stat-tracker-ui -- --fixture full
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

- **Screen:** the **Seasons** tab (sidebar **Seasons** in the classic layout) lists each server season (name, UTC window, current marker) plus an **All time** row, each with games / W-L-D / win rate. Selecting a row sets the season switch. Read-only, no admin CRUD in the desktop app (`/admin/seasons` on the website).
- **Fetch:** live mode GETs `/api/public/seasons` on launch and every 30 minutes. Written to `<data_dir>/seasons.json`. Offline uses that cache and shows **Last refreshed … · cached** on the Seasons screen.
- **Selection:** default = the season marked `is_current`, else all time. Persisted in `<data_dir>/ui_state.json`. No seasons → picker hidden (same as the website).
- **Window:** `starts_at <= played_at < ends_at` (UTC), matching `crates/db/src/queries/personal_stats.rs` (`played_at >= $season_start AND played_at < $season_end`).

### How to verify season numbers against the website

Acceptance: for a **synced** account, season totals on this screen equal My Stats on the site.

1. Sync so daemon `live_snapshot.json` rows with `synced: true` match server `personal_match` for that member. The desktop aggregate also includes **unsynced** local games, subtract those (or sync first) before comparing.
2. `GET /api/public/seasons`: same list the GUI caches. Pick a season id (or omit for all time).
3. Website: **My Stats** (or `/stats/member/:id`) → same season in the picker. Or:

   ```sh
   curl -sS "$SCUFFED_SERVER/api/public/seasons"
   # session cookie required:
   curl -sS "$SCUFFED_SERVER/api/stats/me?season=$SEASON_ID"
   # or /api/stats/member/$MEMBER_ID?season=$SEASON_ID
   # stored-role aggregates (same session auth and ?season= window as /heroes):
   curl -sS "$SCUFFED_SERVER/api/stats/me/roles?season=$SEASON_ID"
   # or /api/stats/member/$MEMBER_ID/roles?season=$SEASON_ID
   ```

   `/roles` returns a JSON array of `{ role, matches, wins, losses, draws, avg_elims, avg_deaths, avg_damage, avg_healing }`, ordered by matches descending, then role ascending. Each row is grouped by the role the tracker uploaded: its manual correction if made, else the detected role. `""` is its own row when that stored role is empty.

4. Compare **games** (`total_matches`), **wins**, **losses**, **draws** to the Seasons-screen row. A game on `ends_at` belongs to the *next* season (half-open).
5. **Win rate:** the site shows `wins / total_matches`. This GUI uses `wins / (wins + losses + draws)` (unknown outcomes do not dilute, P0/P1). They match when every counted game has a decided outcome. The Seasons row appends `· N undecided` when the counts differ so the percentage is readable. **FrozenTear to decide** whether the site should switch to decided-only (recommended by the P2 review) before P5.

### StoreCommand round-trip

1. Start the tracker daemon as usual (`stat-tracker` / systemd unit). It owns SurrealKV and polls `<data_dir>/commands/`.
2. Start this GUI against the same `--data-dir` (or the config/XDG default).
3. Open **Games** and click a row so it opens, then:
   - Result **Win / Loss / Draw** → `StoreCommand::SetOutcome`
   - **Edit** → change a field → **Save corrections** → `StoreCommand::EditMatch` (the map must be one the reader knows; a missing map or hero holds the upload until it is set)
   - Hero timeline **Confirm / Dismiss**, and **Undo** on a dismissed run → `StoreCommand::ResolveSegment` (`confirm` / `dismiss`, and `clear` for Undo)
   - Under the 0.6 top tabs the open row ends with **Delete game**, which opens a confirm strip in place of the actions (Cancel first, Esc cancels); deleting writes `StoreCommand::DeleteSession`, and its tombstone goes up on the next sync if the game was uploaded. Under the classic sidebar layout the card still has **Victory / Defeat / Draw** and **Delete session** (click twice) → `StoreCommand::DeleteSession`.
4. The GUI writes `cmd_<nanos>_<pid>.json` via `storage::queue_command` (tmp + rename).
5. The daemon applies the file, deletes it, and rewrites `live_snapshot.json`. The GUI polls mtime at 1 Hz and refreshes.

Inspect a queued file before the daemon eats it:

```sh
# stop the daemon briefly, or copy quickly
cat "$HOME/.local/share/scuffed-stat-tracker/commands/"cmd_*.json
```

Example payload (same tagged JSON the daemon applies):

```json
{"op":"set_outcome","session_id":"sess-t1","outcome":"defeat"}
```

Fixture mode still writes those files (under the fixture `--data-dir`); there is no daemon to apply them unless you point `--data-dir` at a live tree. A fixture run in its default temp dir starts with no queued commands; one with a `--data-dir` you pass keeps the queued commands there.

### Flags

| Flag | Meaning |
|------|---------|
| `--data-dir PATH` | Daemon data dir (default: config / XDG; temp dir when `--fixture` is set) |
| `--fixture empty\|sample\|games\|full` | Write a demo `live_snapshot.json` and read it back through `storage::read_snapshot`. `games` is the Games tab set: every row and panel state, with scoreboard pictures. `full`: 857 games for the Maps and Seasons shots. |
| `--companion` | Run the layer-shell overlay only (needs niri / wlr-layer-shell). |
| `--seasons-url URL` | `GET /api/public/seasons`; cached to `<data_dir>/seasons.json`. Ignored with `--fixture`. |

Seasons URL falls back to `SCUFFED_SERVER` or `config.toml` sync URL (live mode only). Offline: cache only. No seasons → picker hidden, all time.

### Settings and tracker service

Live mode only. `--fixture` shows the Settings page but does not write `config.toml`, start/stop the service, compact, or delete data.

```sh
# same data dir the service uses (XDG default if omitted)
cargo run -p scuffed-stat-tracker-ui -- --data-dir "$HOME/.local/share/scuffed-stat-tracker"
```

1. The **Settings** tab (sidebar **Settings** in the classic layout).
   The 0.6 page opens with a status strip (tracker state, Version, Last capture, how many changes wait for a restart, and **Restart tracker** or **Start tracker**), then **Run setup again** and the cards: Player and sessions, Companion, Website account and Updates, with the ADVANCED rows (Capture and reading, Tracker service, Stored data, Website connection, Troubleshooting) beside them on a wide window and under them on a narrow one.
2. **Tracker service**: Start / Stop / Restart (on the 0.6 page Start and Restart are on the status strip). When `~/.config/systemd/user/scuffed-stat-tracker.service` exists, verbs go through `systemctl --user`. Otherwise Start launches `scuffed-stat-tracker` from PATH / next to this binary; Stop reads `daemon.pid` and signals only if `/proc/<pid>/comm` is the tracker.
3. **Start on login**: `systemctl --user enable --now` / `disable --now` when the unit file is installed.
4. **Capture**: pick a monitor (or Auto), then **Capture now** for a one-shot preview of what the tracker sees.
5. On the 0.6 page every change is written to `config.toml` as you make it, with no Save button: switches, chips, dropdowns and radios at once, typed numbers on Enter, Website connection with its own **Apply**. Troubleshooting holds **Save debug images from each capture** (`debug_ocr`), **Save pre-game screens** (`debug_save_pregame_screens`, off by default) and the extra number reader. Changes the running tracker reads only at start are tagged **After restart** and counted on the strip until the tracker restarts; the page never restarts it for you. A save edits `config.toml` in place: comments, key order and keys this build does not know stay (a key a save removes takes only its own line; the comment lines above it move onto the entry below, and go with it only when it was the last entry of its table or the entry below is a dotted key, an implicit table such as `[foo.bar]` with no `[foo]`, or an array of tables). Settings shows the values in effect, with the `--token` / `SCUFFED_TOKEN`, `--server` / `SCUFFED_SERVER`, `STAT_TRACKER_DEBUG_OCR` and `STAT_TRACKER_OCR_THREADS` overrides on top (each such row says **Set outside the app**), but a save never writes an override into the file. A change that is refused (a cleartext website URL, a typed number that does not parse) leaves the file and the control as they were and says why. **Show Companion during games** is stored in `<data_dir>/ui_state.json` (`companion_during_games`); off, the Companion never starts and the bar switch, the tray item and the shortcut are off too. The classic layout has no control for it and ignores it. In the classic layout **Save settings** writes today's fields: monitor, in-game name (`player_name`, no `#1234` discriminator), session window, game process names, auto-detect, website URL + token, debug images. **Companion** shortcut is stored in `<data_dir>/ui_state.json` (not `config.toml` / not `StoreCommand`). Restart the service after a change if it is running.
6. **Scoreboard reading**: Install puts back the `koverwatch.traineddata` that ships with the release (`restore_bundled_koverwatch`, from `PREFIX/share/scuffed-stat-tracker/tessdata` or `/usr/share/scuffed-stat-tracker/tessdata`). With no bundled copy it shows the reinstall command. It never trains on the host.
7. **Stored data**: **Compact stored data** (`LocalStore::vacuum`) or **Delete all local match data**. Delete removes the match store, the match log (`matches.jsonl`), `live_snapshot.json`, the hero-timeline backup (`stats.surrealkv.pre-timeline-*`, made when 0.6.0-alpha.1.1 first starts; only the newest is kept), and `screencast-restore-token` with its `.tmp`, so the next screen share asks for permission again. Settings (`config.toml`), debug images and the logs (`daemon.log`, `perf.log`) stay, and nothing on the website is deleted. Both refuse while the service is running.
8. **Update banner** (Overview + Settings) appears only when GitHub has a newer `stat-tracker-v*` release. It never downloads an installer. The "you're on" version is `SST_RELEASE_VERSION` (runtime or compile-time, set by release CI / packaging) or `scuffed-stat-tracker --version` from the installed daemon, never this crate's `0.1.0`. If none of those resolve, the banner stays hidden.
9. **Tray**: Show window / Hide window / Hide / show overlay / Quit (`tray-icon`) when Ayatana AppIndicator (or `libappindicator3`) is on the host. Left-click shows the window. The GUI runs as `iced::daemon` so the process stays alive with no window. Hide is `window::close` (destroys the surface, `Mode::Hidden` / minimize stay in niri Alt-Tab); Show is `window::open` and rebinds `window_id`. App state (screen, settings draft, overlay session hold, etc.) is kept in the daemon. Overlay hide is session-scoped: it sticks until the game ends, then the next launch auto-shows. If the system lib is missing, the window still starts (warning + toast); Hide-to-tray does not work and closing the window quits. Optional packages: Debian/Ubuntu `libayatana-appindicator3-1`, Fedora `libayatana-appindicator-gtk3`, Arch `libayatana-appindicator`. AerynOS may not ship it.

```sh
systemctl --user status scuffed-stat-tracker.service
systemctl --user start scuffed-stat-tracker.service
# after a change tagged After restart (or Save settings in the classic layout):
systemctl --user restart scuffed-stat-tracker.service
```

Start through the unit does not inherit the GUI's `WAYLAND_DISPLAY` /
`DISPLAY`. `install.sh` rewrites `ExecStart` to the real `$PREFIX/bin`
binary and adds `scuffed-stat-tracker-session.service`, which refreshes
`~/.config/scuffed-stat-tracker/session.env` from the compositor before
each start. Without that, Sway/Hyprland come up as `CaptureBackend::None`.
Reinstall if the unit on disk still says `ExecStart=%h/.local/bin/scuffed-stat-tracker`.

### Companion overlay (P3)

Layer-shell surface (`iced_layershell` 0.19, pinned with Iced 0.14): `Layer::Overlay`, top-right, margins 24, width 360, height to content, `KeyboardInteractivity::None`, exclusive zone 0, output = `capture_output` from config (same monitor the daemon captures). Clicks and keys pass through (`events_transparent`) so fullscreen Overwatch keeps input.

**Visibility:** auto-show while the **game process** is running, unless **Show Companion during games** is off on the 0.6 Settings page (then it never starts, and the chip, the tray item and the shortcut do nothing but say so; the classic layout ignores that setting) (configured `game_process_names` in `/proc`). A leftover `active_game.json` does **not** count as live, the daemon keeps that file after a match. While the process is up, the file may supply a `session_id` only for an unfinished game inside `session_window_secs`; decided outcomes and stale `last_activity` are ignored. Empty process names → not running. The companion chip / Overview health / tray **Hide / show overlay** hides it until the **process ends**: it does not reopen mid-session (new match ids keep the hold). Next launch auto-shows. The hold is `overlay_hidden_key` in `<data_dir>/ui_state.json`. Esc is N/A (`KeyboardInteractivity::None`).

**Show / hide shortcut:** Settings → **Companion**. Default **Super+Shift+C**, on. The overlay cannot receive keys (`KeyboardInteractivity::None`), so the main GUI process reads `/dev/input` (same evdev path as Tab capture, not X11 `XGrabKey`). Same `input` group / seat `uaccess` as the daemon. Add it with `sudo usermod -aG input "$USER"`, then log out and back in, a new terminal does not pick up the group. The shortcut is an explicit user action: it toggles visibility **now** through the same OverlayHold as the tray button. Hide sticks until the game ends; pressing it again mid-session shows the overlay. While **Show Companion during games** is off the shortcut is not listened to. The current bind is a one-line footer on the overlay when the shortcut is on. Bind is stored in `<data_dir>/ui_state.json` (`overlay_hotkey`, `overlay_hotkey_enabled`).

Bind format: `Super+Shift+C` (modifiers joined with `+`, then one key). Super aliases: `Meta`, `Win`. Also `Ctrl`, `Alt`, `Shift`. Key: `A` to `Z`, `0` to `9`, `F1` to `F12`, `Tab`, `Space`, `Esc`, `Enter`. At least one modifier is required. Empty field saves as Super+Shift+C.

**Why not the GlobalShortcuts portal?** niri (FrozenTear’s CachyOS / AerynOS) has no xdg-desktop-portal GlobalShortcuts backend. A portal bind would not fire there. This path does not prompt for a portal permission.

**Optional niri bind** (only if the in-app shortcut does not fire; do not use both or one press will hide then show):

```kdl
binds {
    Super+Shift+C { spawn-sh "touch ~/.local/share/scuffed-stat-tracker/companion_toggle"; }
}
```

The GUI notices `companion_toggle` mtime on its 1s tick and runs the same toggle. That file is **not** a `StoreCommand` (do not put it under `commands/`).

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

The companion chip **Companion showing / hidden / waiting for game** sits in the controls on the stats screens (in the header in the classic layout). With **Show Companion during games** off it reads **Companion off in Settings** and cannot be pressed. Tray **Hide / show overlay** does the same. Hide while the game is live sticks until that process ends. `--fixture` counts as one live session so Design can hide/show for shots.

KDE blur (`org_kde_kwin_blur`) is a follow-up, not v1. The panel is translucent (`surface` @ 88%).

`iced_layershell` is the default `companion` feature. If it cannot build on a host: `--no-default-features` keeps the main window; `--companion` then prints a rebuild hint.

**FrozenTear:** live niri acceptance: overlay sits above fullscreen Overwatch; the game keeps keyboard and pointer. Design shots: empty + with-data.

## Out of scope (later)

Compositor blur on the overlay (`org_kde_kwin_blur`).

Urbanist (OFL) is bundled as the labelled tracker product face, see `fonts/OFL.txt`.

Icons in `assets/`: the tab icons (`assets/tab-icons/`, white alpha masks the app tints) were rasterised once at 2x from the stroke SVG paths in the 0.6 redesign spec, drawn for this project; that spec is not in this repository, so five of them are kept as committed images and cannot be redrawn here. `assets/draw_icons.py` draws the overview tab icon and the warning, check, lock and chevron icons. The root `.gitignore` ignores `*.png` except under `assets/`, and a test checks that every file the GUI compiles in is tracked by git. No third-party icon set is used; they fall under this repository's licence.
