# scuffed-stat-tracker

Overwatch 2 personal stat tracker for Linux. A background daemon
watches for Tab (scoreboard) presses, OCRs the scoreboard, tracks game
sessions/outcomes, stores everything locally, and optionally syncs per-game
results to the Scuffed Crew site. The desktop GUI is the Iced crate
`scuffed-stat-tracker-ui` (binary still named `stat-tracker-gui` so
install paths and the `.desktop` entry stay the same).

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
| **Daemon** | glibc ≥ 2.35 (Ubuntu 22.04+, Debian 12+, Fedora, Arch, openSUSE, RHEL 9+) | OCR `.so` closure is **bundled** in `lib/scuffed-stat-tracker/ocr` (soname splits across distros). Installer copies that tree so RUNPATH `$ORIGIN/../lib/scuffed-stat-tracker/ocr` works. OpenSSL is **not** bundled. |
| **GUI** | glibc ≥ 2.35 + **GTK 3** + Vulkan (or Iced software fallback) | Iced 0.14 (`scuffed-stat-tracker-ui`), binary name `stat-tracker-gui`. |
| **Host still needed** | Wayland **or** X11 + `input` group + `eng.traineddata` | Capture/compositor and keyboard access stay host-provided. |

## Install (prebuilt Linux x86_64)

No Rust toolchain required. GitHub Releases publish
`scuffed-stat-tracker-linux-x86_64.tar.gz` (`bin/`, optional `lib/`, assets,
`install.sh`) on tags `stat-tracker-v*`. Release notes:
`CHANGELOG.md`. Tag runbook (human gate):
`docs/notes/stat-tracker-v0.4.1-tag.md`.

Since **v0.3.0** the tarball also bundles `tessdata/eng.traineddata` (the
runtime OCR model); `install.sh` drops it into
`~/.local/share/scuffed-stat-tracker/tessdata/` (never overwriting a model you
already have), so no distro tessdata package is required. The tarball also
carries a CI-trained `koverwatch.traineddata` (game-font model) installed the
same way; the bundled copy is canonical and replaces an older one (a `.bak` is
kept), since most hosts cannot regenerate it locally — `text2image`
hangs/segfaults with pango ≥ 1.56.

**One-liner** (downloads latest matching release and installs into
`~/.local`):

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Defaults to the newest **stable** release. If a newer prerelease (RC) exists
and you're at an interactive terminal, the script asks which one you want
(stable is the default answer). Skip the question with
`STAT_TRACKER_CHANNEL=prerelease` (or `=stable`); non-interactive runs always
get stable.

Pin a tag or change the install prefix:

```sh
STAT_TRACKER_TAG=stat-tracker-v0.4.1 \
STAT_TRACKER_PREFIX=$HOME/.local \
  bash -c 'curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash'
```

**Manual:** download the tarball (+ optional `.sha256`) from the release page,
extract, then:

```sh
cd scuffed-stat-tracker-linux-x86_64
./install.sh          # bins → $PREFIX/bin, OCR/gui libs → $PREFIX/lib/scuffed-stat-tracker/{ocr,gui}
```

The in-tarball installer lives at `dist/install.sh` in this crate (copied to
the tarball root by the release workflow). Source checkouts still use
`crates/stat-tracker/install.sh`, which **builds with cargo**.

**Uninstall:**

```sh
scuffed-stat-tracker-uninstall            # keeps match log + config for reinstalls
scuffed-stat-tracker-uninstall --purge    # also deletes app data and config
```

`install.sh` records everything it installs in
`$PREFIX/share/scuffed-stat-tracker/install-manifest.txt`; the uninstaller
removes exactly those files (plus stopping/disabling the systemd unit). For
installs made before the manifest existed it falls back to the known default
paths and prints how to identify the bundled libs from the tarball.

## Running

```sh
# daemon (foreground; logs to stderr)
cargo run -p scuffed-stat-tracker

# desktop GUI (Iced — crate scuffed-stat-tracker-ui, binary stat-tracker-gui)
cargo run -p scuffed-stat-tracker-ui
```

Replacing an old Dioxus `stat-tracker-gui`: reinstall (prebuilt `./install.sh`,
or `crates/stat-tracker/install.sh` from a source checkout). The binary name
does not change; only the implementation does.

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

## Troubleshooting

**`stat-tracker-gui` fails with `OPENSSL_3.2.0 not found` (v0.4.0).**
v0.4.0 bundled Ubuntu 22.04 `libcrypto.so.3` / `libssl.so.3` into
`~/.local/lib`. Both binaries used RUNPATH `$ORIGIN/../lib`, so that copy
won over `/usr/lib` and broke hosts whose `libcryptsetup` needs OpenSSL
3.2 (Aerynos). Install **v0.4.1** (the installer removes those leftovers)
or delete the tracker-owned files:

```sh
rm -f ~/.local/lib/libcrypto.so.3 ~/.local/lib/libssl.so.3
```

**Games play but nothing is recorded, and `debug/accepted/` stays empty.**
The daemon reads Tab presses straight from `/dev/input`. A global-hotkey daemon
that grabs keyboards exclusively (e.g. GPU Screen Recorder's `gsr-global-hotkeys
--all`) silently starves it: the kernel delivers events only to the grabber. The
daemon logs `keyboard is exclusively grabbed by another process` at WARN and
picks up the grabber's virtual pass-through keyboard automatically when it
appears (hotplug). If the WARN says *every* keyboard is grabbed, switch the
hotkey tool to its no-grab / virtual-devices mode or restart the tracker after
it. Games the poller saw but never got a Tab for are listed in
`debug/unrecorded_games.jsonl`.

## Dev tools

`examples/` contains the diagnosis workflow — each file documents its usage:
`extract` (full pipeline against a still image), `polltick` (poll-tick CPU
cost), `probe_outcome`, `accolade`, `profile`, `dumpdb`. Fixture replay tests
(`tests/`, `#[ignore]`d) validate outcome detection against real frames in
`tests/fixtures/outcomes/`; scoreboard replays expect (uncommitted) screenshots
in `tests/fixtures/replays/`.
