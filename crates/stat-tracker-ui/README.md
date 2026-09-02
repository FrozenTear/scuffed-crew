# scuffed-stat-tracker-ui

P0 spike: native **Iced 0.14** (wgpu) Overview for the Scuffed Crew stat tracker.

- Package: `scuffed-stat-tracker-ui`
- Binary: `stat-tracker-gui` (same name the Dioxus GUI uses, so install paths stay compatible later)
- Reads the daemon snapshot via `stat_tracker::storage::read_snapshot` (`live_snapshot.json`)
- Does **not** change the daemon, OCR, capture, sync, or store schema
- Does **not** write `StoreCommand`s

Design: PR #55 rev 2 (`docs/notes/stat-tracker-gui-redesign-2026-09-02.md`).

## Run

```sh
# Live daemon data (default XDG data dir / config.toml)
cargo run -p scuffed-stat-tracker-ui

# Empty Overview
cargo run -p scuffed-stat-tracker-ui -- --fixture empty

# Sample matches + seasons (season switch defaults to the current season)
cargo run -p scuffed-stat-tracker-ui -- --fixture sample
```

Headless software-rendered PNGs (no window / wgpu):

```sh
cargo run -p scuffed-stat-tracker-ui -- --fixture empty --preview crates/stat-tracker-ui/artifacts/overview-empty.png
cargo run -p scuffed-stat-tracker-ui -- --fixture sample --preview crates/stat-tracker-ui/artifacts/overview-sample.png
```

Checked-in previews: `artifacts/overview-empty.png` and `artifacts/overview-sample.png`.

### Flags

| Flag | Meaning |
|---|---|
| `--data-dir PATH` | Daemon data dir (default: config / XDG; temp dir when `--fixture` is set) |
| `--fixture empty\|sample` | Write a demo `live_snapshot.json` and read it back through `storage::read_snapshot` |
| `--seasons-url URL` | `GET /api/public/seasons`; cached to `<data_dir>/seasons.json` |
| `--preview PATH` | Software-render Overview to PNG and exit |

Seasons URL falls back to `SCUFFED_SERVER` or `config.toml` sync URL. Offline: cache only. No seasons → picker hidden, all time.

## Out of scope (later phases)

Games / Heroes full screens, companion overlay, settings, removing the Dioxus `gui` feature.

Urbanist (OFL) is bundled as the labelled tracker product face — see `fonts/OFL.txt`.
