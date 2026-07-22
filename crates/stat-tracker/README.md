# scuffed-stat-tracker

Overwatch 2 personal stat tracker for Linux. A background daemon
watches for Tab (scoreboard) presses, OCRs the scoreboard, tracks game
sessions/outcomes, stores everything locally, and optionally syncs per-game
results to the Scuffed Crew site. An optional Dioxus desktop GUI
(`stat-tracker-gui`, behind the `gui` feature) shows live status, history,
stats, and daemon controls.

## Platform requirements

- **Linux + Wayland; experimental X11 capture.**
  - Wayland: libwayshot on wlr-screencopy compositors (Sway, Hyprland, …),
    with XDG Desktop Portal fallback.
  - X11 (experimental): native capture when a usable X server is detected and
    Wayland capture is unavailable.
  - Portal remains last-resort on either stack (slower; not ideal for the poller).
- **Keyboard access via evdev.** Tab detection reads `/dev/input` — the user
  must be in the `input` group (`sudo usermod -aG input $USER`, re-login).
- **Tessdata (`eng.traineddata`).** Looked up in (first hit wins):
  user `~/.local/share/scuffed-stat-tracker/tessdata/`, `TESSDATA_PREFIX`,
  `/usr/share/tessdata`, `/usr/share/tesseract-ocr/*/tessdata` (Debian/Ubuntu),
  `/usr/share/tesseract/tessdata` (Fedora), `/usr/local/share/tessdata`.
  A game-font-tuned model improves accuracy:
  `scuffed-stat-tracker --generate-tessdata` writes
  `koverwatch.traineddata` under the user tessdata dir (picked up on next start).

### Distro matrix (prebuilt release)

| Component | Minimum | Notes |
|-----------|---------|--------|
| **Daemon** | glibc ≥ 2.35 (Ubuntu 22.04+, Debian 12+, Fedora, Arch, openSUSE, RHEL 9+) | OCR `.so` closure is **bundled** in `lib/` (soname splits across distros). Installer copies `lib/` → `$PREFIX/lib` so RPATH `$ORIGIN/../lib` works. |
| **GUI** | modern distro with **webkit2gtk-4.1** + glibc ≥ 2.39 (Ubuntu 24.04+, Debian 13, Arch, recent Fedora) | Not portable to older LTS; use daemon-only on older boxes if needed. |
| **Host still needed** | Wayland **or** X11 + `input` group + `eng.traineddata` | Capture/compositor and keyboard access stay host-provided. |

HOLD `stat-tracker-v0.1.0` until portable CI + this runtime lane land.

## Install (prebuilt Linux x86_64)

No Rust toolchain required. GitHub Releases publish
`scuffed-stat-tracker-linux-x86_64.tar.gz` (`bin/`, optional `lib/`, assets,
`install.sh`) on tags `stat-tracker-v*`.

Since **v0.3.0** the tarball also bundles `tessdata/eng.traineddata` (the
runtime OCR model); `install.sh` drops it into
`~/.local/share/scuffed-stat-tracker/tessdata/` (never overwriting a model you
already have), so no distro tessdata package is required.

**One-liner** (downloads latest matching release and installs into
`~/.local`):

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Pin a tag or change the install prefix:

```sh
STAT_TRACKER_TAG=stat-tracker-v0.1.0 \
STAT_TRACKER_PREFIX=$HOME/.local \
  bash -c 'curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash'
```

**Manual:** download the tarball (+ optional `.sha256`) from the release page,
extract, then:

```sh
cd scuffed-stat-tracker-linux-x86_64
./install.sh          # bins → $PREFIX/bin, bundled lib/ → $PREFIX/lib, desktop + systemd unit
```

The in-tarball installer lives at `dist/install.sh` in this crate (copied to
the tarball root by the release workflow). Source checkouts still use
`crates/stat-tracker/install.sh`, which **builds with cargo**.

## Running

```sh
# daemon (foreground; logs to stderr)
cargo run -p scuffed-stat-tracker

# GUI
cargo run -p scuffed-stat-tracker --features gui --bin stat-tracker-gui
```

First-run sync setup: `scuffed-stat-tracker --token <daemon-token> --server
https://…` writes `~/.config/scuffed-stat-tracker/config.toml` (chmod 600 —
it holds the bearer token). Tokens are minted in the site under
My Stats → Settings.

Useful flags: `--list-outputs`, `--collect-portraits` (build hero-portrait
references from your own captures), `--dump-poll-frames` (ring buffer of
poll-tick frames for diagnosis), `--generate-tessdata`.

A user systemd unit named `scuffed-stat-tracker.service` is recognized by the
GUI's daemon card (start/stop/autostart route through systemd when installed).

## Config (`~/.config/scuffed-stat-tracker/config.toml`)

| Key | Meaning |
|---|---|
| `player_name` | Scoreboard name used to find your row (fetched from the server if unset) |
| `capture_output` | Display/output name to capture (`--list-outputs`) |
| `data_dir` | Store/log/debug location (default `~/.local/share/scuffed-stat-tracker`) |
| `auto_detect.*` | Poll-based match start/end detection (interval, cooldown) |
| `game_process_names` | Only capture while one of these processes runs (empty disables the gate) |
| `debug_ocr` | Dump OCR intermediate PNGs under `{data_dir}/debug/` (also env `STAT_TRACKER_DEBUG_OCR=1`) |
| `ocr_threads` | Parallel OCR workers (1–8). Each keeps a ~23 MB Tesseract model in RAM. Omit for auto (`(cores/2)` clamped 2–4). Also env `STAT_TRACKER_OCR_THREADS` or CLI `--ocr-threads N`. Use `1` to minimize RAM; higher speeds Tab OCR. |

Example low-RAM:

```toml
ocr_threads = 1
```

The daemon reads config once at startup — restart it after changes.

## Data & IPC

Single-process SurrealKV store at `{data_dir}/stats.surrealkv`. Because only
one process can hold it, the daemon exports `live_snapshot.json` after
mutations (debounced) and appends to `matches.jsonl`; the GUI reads those when
the daemon holds the lock and sends manual edits through a file command queue
(`{data_dir}/commands/`).

## Dev tools

`examples/` contains the diagnosis workflow — each file documents its usage:
`extract` (full pipeline against a still image), `polltick` (poll-tick CPU
cost), `probe_outcome`, `accolade`, `profile`, `dumpdb`. Fixture replay tests
(`tests/`, `#[ignore]`d) validate outcome detection against real frames in
`tests/fixtures/outcomes/`; scoreboard replays expect (uncommitted) screenshots
in `tests/fixtures/replays/`.
