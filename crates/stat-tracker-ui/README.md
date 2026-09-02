# scuffed-stat-tracker-ui

Native **Iced 0.14** (wgpu) tracker GUI for the Scuffed Crew stat tracker.

- Package: `scuffed-stat-tracker-ui`
- Binary: `stat-tracker-gui` (same name the Dioxus GUI uses, so install paths stay compatible)
- Reads the daemon snapshot via `stat_tracker::storage::read_snapshot` (`live_snapshot.json`)
- Writes `StoreCommand` files under `<data_dir>/commands/` — same contract as the Dioxus GUI (`SetOutcome`, `EditMatch`, `DeleteSession`, `ResolveSegment`)
- Does **not** change the daemon, OCR, capture, sync, or store schema
- Does **not** open SurrealKV; the running daemon applies queued commands and refreshes the snapshot

Design: `docs/notes/stat-tracker-gui-redesign-2026-09-02.md` rev 2 (branch `docs/tracker-gui-redesign`).

P0 Overview is on `main` (PR #56). This crate is P1: Games / Heroes / Maps, role chips, max content width, command writes.

There is **no** software `--preview` path and no `preview.rs`. Robert will capture real niri window shots (empty + sample + live) for Design.

## Run fixtures

```sh
# Empty Overview / Games / Heroes / Maps (no season picker)
cargo run -p scuffed-stat-tracker-ui -- --fixture empty

# Sample matches + seasons (picker stays; defaults to the current season)
cargo run -p scuffed-stat-tracker-ui -- --fixture sample
```

`--fixture` never fetches `GET /api/public/seasons`. It writes a demo `live_snapshot.json` (and sample `seasons.json`) under a temp dir, then reads it back through `storage::read_snapshot`.

Override the dir if you want to inspect command files afterwards:

```sh
cargo run -p scuffed-stat-tracker-ui -- --fixture sample --data-dir /tmp/sst-ui-sample
ls /tmp/sst-ui-sample/commands/   # after using Games → expand a card → Victory / Edit / Delete
```

## Live (daemon running)

```sh
cargo run -p scuffed-stat-tracker-ui
# or pin the same data dir the daemon uses:
cargo run -p scuffed-stat-tracker-ui -- --data-dir "$HOME/.local/share/scuffed-stat-tracker"
```

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
|---|---|
| `--data-dir PATH` | Daemon data dir (default: config / XDG; temp dir when `--fixture` is set) |
| `--fixture empty\|sample` | Write a demo `live_snapshot.json` and read it back through `storage::read_snapshot` |
| `--seasons-url URL` | `GET /api/public/seasons`; cached to `<data_dir>/seasons.json`. Ignored with `--fixture`. |

Seasons URL falls back to `SCUFFED_SERVER` or `config.toml` sync URL (live mode only). Offline: cache only. No seasons → picker hidden, all time.

## Out of scope (later phases)

P2 Seasons screen, P3 companion overlay, P4 settings/tray, P5 remove the Dioxus `gui` feature.

Urbanist (OFL) is bundled as the labelled tracker product face — see `fonts/OFL.txt`.
