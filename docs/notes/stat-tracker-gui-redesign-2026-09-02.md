# Stat Tracker desktop GUI redesign — design document

Status: **proposal for review** (claude, 2026-09-02). Owner: USER. Reviewer: grok.
Mockups: gallery https://claude.ai/code/artifact/39b79ec2-d70c-4697-bf3c-728eab98858c (the "Your pick" section at the top is this design; 30 alternatives below it).

## 1. Decisions already made (USER, 2026-09-02)

| Decision | Choice | Why |
|---|---|---|
| Toolkit | Leave Dioxus. **Native only, no web view.** Rebuild on **Iced 0.14** (wgpu). | Dioxus desktop felt limiting; the native Dioxus renderer is not there yet. Iced is the most mature GPU-rendered Rust toolkit; Elm-style state fits the daemon-snapshot model. |
| Direction | **Combined 12 Hero Gallery + 20 Match Cards + 10 Companion** | One card grammar for games and heroes; the companion is the same cards shrunk into an overlay. |
| Seasons | Included in the redesign. Every stats view switches **All time / one season**. | Server side shipped in PR #48; the desktop GUI has no season concept today. |
| Companion overlay | **wlr-layer-shell** surface on the overlay layer (above the fullscreen game). Translucent panel now; request compositor blur (niri 26.04+, KWin, Hyprland) as a follow-up. | USER already ships this pattern in `noctalia-stream-addonsx/rust/render` (smithay-client-toolkit, `Layer::Overlay`, `KeyboardInteractivity::None`, exclusive zone 0). |

Rejected: Tauri / any web front end (USER: native only). egui (look ceiling), Slint (own markup + licence), GPUI/Xilem (too early), Freya (Dioxus-based).

## 2. Goals and non-goals

Goals
- A native desktop app that answers "how am I doing this season?" in one glance and shows tonight's games as the primary object.
- Total-or-per-season everywhere, with the same semantics as the website (`played_at` in `[starts_at, ends_at)`).
- A companion overlay usable while the game is fullscreen.
- Keep the daemon untouched. The UI remains a separate process that reads the daemon's snapshot and queues commands, exactly as today.
- Feature parity with the current GUI before the Dioxus GUI is removed: dashboard/status, matches list with corrections, stats, settings (config.toml), capture preview, daemon start/stop, update banner, tray.

Non-goals
- No changes to OCR, capture, sync, or the store schema.
- No new server endpoints. Seasons come from `GET /api/public/seasons`.
- No Windows/macOS (unchanged: Linux/Wayland first, X11 via existing capture backends).

## 3. Visual system

Card grammar (the one rule everything follows): **the hero's role colour tints the card; the outcome is a 4 px stripe on the left plus a small label.** Games and heroes are the same card, differing only in content.

Tokens (dark-first; a light theme is out of scope for v1)

| Token | Value | Use |
|---|---|---|
| bg | `#121218` | window background |
| surface | `#1c1c25` | panels, card base |
| surface-line | `#2a2a36` | dividers, chip borders |
| text | `#f2f2f7` | primary text |
| text-2 | `#c9c9d6` | secondary |
| muted | `#8d8da0` | labels, meta |
| accent | `#8f73ff` | season ring, links, selected chip (brand product accent) |
| win / loss / draw | `#46d8a4` / `#ff5c7a` / `#8a8fa8` | outcome stripe + label only |
| role Tank / Damage / Support | `#5b8def` / `#ff7a59` / `#46d8a4` | card tint (gradient `role55 → surface` at 160°), role chips, role split bar |
| warn | `#f5b84a` | keyboard-grab and pending-sync warnings |

Note: Support role colour and the win colour are the same mint on purpose; they never sit in the same slot (tint vs stripe).

Typography: **Urbanist** (OFL, bundle the TTFs; weights 500/600/700/800). Scale: 11 px labels (uppercase, +0.12 em tracking), 13 px meta, 14 px body, 20/22 px card titles, 26–34 px featured card title, 32 px hero win rate, 55.6 %-style hero numbers at 26–34 px, tabular figures everywhere numbers align.

Shape and spacing: cards radius 18 px, inner padding 16–18 px, grid gap 12 px, page padding 26/36 px, stripes 4 px, bars 5–6 px. Shadows only on the companion pane.

Iconography: stroke SVG (existing set from the mocks: home, list, chart, calendar, gear, camera, eye, cloud, keyboard). No emoji.

## 4. Screens

### 4.1 Overview (main window, default)
Header: app name · **season switch** (All time / Season N segmented control) · role filter chips (Tank / Damage / Support, toggleable, filter both shelves) · live status ("Capturing · last Tab 00:27" or the warning state).

Tonight shelf: session = the daemon's session grouping (30-minute idle gap, as today). Cards for each game of the current session, newest first, **the newest card is large** and carries the stat line (E / D / A / DMG / HEAL / MIT). Others show map, hero, role, length, time. Empty state: "No games yet tonight — press Tab in-game to capture the scoreboard."

Heroes shelf: one card per hero played in the selected window, sorted by games, top 4 visible with "all heroes →". Card: role label, hero name, win rate, games, win-rate bar.

Bottom row: season panel (ring with win rate, W–L–D, games, delta vs all time, role split bar) · maps (top 4 with win rate) · tracker health (capture output + backend, OCR model, keyboard devices incl. grabbed count, sync state, companion on/off).

### 4.2 Games
Every game as a card list (compact cards, 2 columns), grouped by session with session headers ("Sat Aug 30 · 4–1"). Filters: season (inherits header), role, hero, map, outcome. Card expands to the full stat line, corrections ("edited" badge), and the actions that exist today: set outcome, edit stats, delete session, resolve hero segment. These map 1:1 onto `StoreCommand` variants (`SetOutcome`, `EditMatch`, `DeleteSession`, `ResolveSegment`).

### 4.3 Heroes and maps
Full hero grid (all heroes, cards as in 4.1) and a map list with games and win rate. Both honour the season switch and role chips.

### 4.4 Seasons
List of seasons from the server (name, window, current marker) with per-season summary (games, W–L–D, win rate) and the all-time row. Selecting one sets the header switch. Read-only in the desktop app; seasons are managed at /admin/seasons on the website. Offline: show the cached list with a "last refreshed" note.

### 4.5 Capture and settings
Same fields as today's Settings page, written to `config.toml`: capture output, player name, session window, game process names, auto-detect (enabled, poll interval, cooldown), server sync (URL, token), OCR setup (tessdata generate/install), diagnostics (`debug_ocr`), data management (vacuum, clear). Capture preview (one-shot screenshot of the selected output through the existing capture backends). Daemon control (start/stop/restart through systemctl, as today) and the update banner.

### 4.6 Companion overlay
Layer-shell surface, `Layer::Overlay`, anchored top-right, margins 28 px, width 360 px, height to content, `KeyboardInteractivity::None`, exclusive zone 0, output = the capture output from config. Contents: header (status dot, season + win rate + record), last game card (large, with stat line), tonight results strip (one bar per game), top three heroes tonight (mini cards), footer chips (last Tab, sync). Visibility: on while the game process is running (from the snapshot), hidden otherwise, plus a manual toggle from the main window and the tray. Blur: request `org_kde_kwin_blur` when available (follow-up, not v1).

## 5. Seasons in the desktop app

- Source of truth: `GET /api/public/seasons` (public, no auth). Fetched on launch and every 30 min; cached to `<data_dir>/seasons.json` so the picker works offline.
- Semantics identical to the server: a game belongs to a season when `starts_at <= played_at < ends_at` (UTC). Aggregation runs locally over the snapshot's `PersonalMatch` rows, so numbers match the website for synced games and additionally include unsynced ones.
- Selection: default = the season marked current if one exists, else all time. The choice persists in `<data_dir>/ui_state.json`.
- No seasons defined on the server → picker hidden, everything is all time (same as the web app).

## 6. Architecture

Crates
- `crates/stat-tracker` — daemon, unchanged. Its lib already exposes `storage::read_snapshot`, `storage::queue_command`, the config types and the capture backends; the new UI depends on the lib, not the binary.
- `crates/stat-tracker-ui` (new) — Iced 0.14 application. Binary name stays `stat-tracker-gui` so `install.sh`, the desktop entry and the tray/update paths keep working.
- `iced_layershell` (0.19, maintained, July 2026) for the companion; falls back to a normal always-on-top window if the compositor lacks layer-shell.

Data flow (unchanged contract)
- Daemon → UI: `live_snapshot.json` (matches + sessions), `active_game.json`, `daemon.pid`. The UI polls mtime at 1 Hz as `live_data.rs` does now (inotify later if wanted) and re-derives all aggregates in memory.
- UI → daemon: files in `<data_dir>/commands/` (`StoreCommand`), unchanged.
- UI → server: only `GET /api/public/seasons` and the existing update check.

Application model (Iced)
- `State { snapshot, seasons, ui: { season_sel, role_filter, screen, overlay_on }, config, daemon: { pid, busy } }`
- Messages per user action plus `SnapshotChanged`, `SeasonsFetched`, `DaemonStatus`, `Tick`.
- Aggregation is a pure function `aggregate(&[PersonalMatch], window: Option<(DateTime, DateTime)>, role: Option<Role>) -> Aggregates`; this is where per-season numbers come from and it gets unit tests against the same fixtures the server test uses (games on both sides of a window).
- Views: one module per screen; shared `card` widgets (game card, hero card, stat box, ring, bar).
- Tray: keep `tray-icon` for v1 (it works today); evaluate `ksni` (StatusNotifierItem without GTK) in P4.

Fonts and assets: Urbanist TTFs embedded with `include_bytes!` and loaded via `iced::font::load`; SVG icons via `iced::widget::svg` from embedded bytes.

## 7. Migration plan

| Phase | Deliverable | Acceptance |
|---|---|---|
| P0 spike | Overview screen, read-only, real `live_snapshot.json`, season switch wired to the cached server list | USER judges the native look against the mock; runs on the USER's Wayland/niri setup with wgpu |
| P1 | Games, Heroes & maps screens; role chips; corrections via `StoreCommand` | Parity with today's Matches/Stats tabs; commands round-trip through the daemon |
| P2 | Seasons screen; persistence; offline cache; aggregate unit tests | Season numbers equal the website's for a synced account |
| P3 | Companion overlay via layer-shell; show/hide on game process; tray toggle | Overlay visible above fullscreen Overwatch on niri; game keeps input |
| P4 | Settings, capture preview, daemon control, update banner, tray parity | Everything the Dioxus GUI does; `install.sh` unchanged apart from the binary |
| P5 | Remove the `gui` feature and `src/gui/` from `stat-tracker`; release | CI green; release notes; USER reinstall |

Each phase is one PR, cross-reviewed per the fleet protocol.

## 8. Risks and open questions

- wgpu on the USER's GPU/compositor: the P0 spike is the test. Fallback is Iced's `tiny-skia` software renderer.
- `iced_layershell` tracks Iced releases with some lag; pin versions together.
- Blur needs the KDE blur protocol on the layer surface; not in v1.
- Binary size grows (wgpu + fonts); acceptable for a desktop tool, note it in the release.
- Light theme: not in v1; tokens are structured so one can be added.
- Open: should the overlay also show the live OCR capture state (last Tab accepted/rejected)? Cheap to add from `active_game.json`; proposed for P3 if USER wants it.

## 9. Review asks (grok)

1. Card grammar and token table — anything that will not survive Iced's styling model?
2. Aggregation as a pure function shared by screens and tests — agree it lives in the UI crate, not the daemon?
3. Phase order — would you put the companion (P3) before games/heroes (P1)?
