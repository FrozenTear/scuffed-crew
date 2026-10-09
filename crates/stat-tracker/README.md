# scuffed-stat-tracker

Overwatch 2 personal stat tracker for Linux. A background daemon
watches for Tab (scoreboard) presses, OCRs the scoreboard, tracks game
sessions/outcomes, stores everything locally, and optionally syncs per-game
results to the Scuffed Crew site. The desktop GUI is the Iced crate
`scuffed-stat-tracker-ui` (binary still named `stat-tracker-gui` so
install paths and the `.desktop` entry stay the same).

## Platform requirements

- **The tracker runs on Linux with both Wayland and X11 capture.** X11 is still being tested more widely.
  - Wayland: libwayshot on wlr-screencopy compositors (Sway, Hyprland, …),
    with XDG Desktop Portal fallback.
  - X11: native capture when a usable X server is detected and
    Wayland capture is unavailable.
  - Portal remains last-resort on either stack (slower; not ideal for the poller).
- **Keyboard access via evdev.** Tab detection (daemon) and the companion
  overlay show/hide shortcut (GUI, default Super+Shift+C) read `/dev/input`.
  The user must be in the `input` group, or have seat `uaccess` on those
  nodes. No X11 key grab.

  Add the group, then log out and back in. A new terminal is not enough —
  existing sessions keep the old group list until the next login:

  ```sh
  sudo usermod -aG input "$USER"
  # log out of the desktop session and log back in
  groups | grep -qw input && echo "input group is active"
  ```

  See `crates/stat-tracker-ui/README.md` (Companion shortcut).
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
| **GUI** | glibc ≥ 2.35 + **GTK 3** + Vulkan (or Iced software fallback) | Iced 0.14 (`scuffed-stat-tracker-ui`), binary name `stat-tracker-gui`. Tray (Hide-to-tray) needs **Ayatana AppIndicator** when the distro ships it; without it the window still starts. |
| **Host still needed** | Wayland **or** X11 + `input` group + `eng.traineddata` | Capture/compositor and keyboard access stay host-provided. |

## Install (prebuilt Linux x86_64)

No Rust toolchain required. GitHub Releases publish
`scuffed-stat-tracker-linux-x86_64.tar.gz` (`bin/`, optional `lib/`, assets,
`install.sh`) on tags `stat-tracker-v*`. Release notes:
`CHANGELOG.md`. Tag runbook (human gate):
`docs/notes/stat-tracker-v0.4.4-tag.md`.

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
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

That `main` URL is only the stable entrypoint, including for GUIs that already
have it saved. The script resolves the release tag (newest **stable** by
default) and runs `bootstrap.sh` from that tag. `install.sh` is the copy
inside the release tarball, not whatever is currently on `main`.

The script installs the newest stable release. It does not offer an alpha or
rc unless `STAT_TRACKER_CHANNEL=prerelease`. The desktop app does the same
until `update_channel = "prerelease"` is set in config.toml. Pin a tag to
install one release, including an alpha.

Pin a tag by fetching that tag's bootstrap (the assignment has to be on
`bash`, because `VAR=x curl … | bash` does not pass `VAR` to `bash`):

```sh
TAG=stat-tracker-v0.5.0-alpha.2
curl --proto '=https' -fsSL "https://raw.githubusercontent.com/FrozenTear/scuffed-crew/${TAG}/crates/stat-tracker/dist/bootstrap.sh" \
  | STAT_TRACKER_TAG="$TAG" STAT_TRACKER_PREFIX="$HOME/.local" bash
```

The tarball's `.sha256` is checked before extract when that asset is published.
That digest comes from the same release. With no minisign public key configured,
the script logs that and stops there. Once a public key is configured, a missing
or bad `.minisig` refuses the install. It does not fall back to sha256.

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

The desktop app has Uninstall in the sidebar, next to About. It removes the
tracker from this computer and asks before removing anything.

Saved games, debug images, logs, and settings (including the sync token) stay
unless that box is checked. The box uses the folder this copy is actually
using, and shows that path. A folder outside your home folder is left in place.

From a terminal:

```sh
bash bootstrap.sh --uninstall
bash bootstrap.sh --uninstall --purge
scuffed-stat-tracker-uninstall
```

The first command keeps saved games and settings. `--purge` deletes them too.
`scuffed-stat-tracker-uninstall` does the same as the first command. It lists
the files and asks before removing them. Add `--yes` to skip the question.

The installer records each file it wrote. Uninstall removes those files and
nothing else. A recorded path that leaves your home folder, or that contains
`..`, is left alone. If that record is missing, uninstall shows the original
files that are still there and asks first. Settings stay unless you also
delete saved games.

If a system package owns this copy, nothing is removed. The app shows
`sudo pacman -R <pkg>` or `sudo apt remove <pkg>` and can copy that command.
An AppImage is removed like any other copy, including the AppImage file.
A copy outside your home folder is left alone. The app lists the paths and
says to remove them manually.

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

Useful flags: `--list-outputs`, `--collect-portraits` (only fills missing
portraits and the Doctrine stand-in; it never overwrites an existing
reference), `--dump-poll-frames` (ring buffer of poll-tick frames for
diagnosis), `--generate-tessdata`.

A user systemd unit named `scuffed-stat-tracker.service` is recognized by the
GUI's daemon card (start/stop/autostart route through systemd when installed).
`install.sh` rewrites `ExecStart` to the absolute `$PREFIX/bin` daemon and
installs a oneshot that fills the display variables the unit does not
inherit. See Troubleshooting if capture stays on `CaptureBackend::None`.

## Config (`~/.config/scuffed-stat-tracker/config.toml`)

| Key | Meaning |
|---|---|
| `player_name` | Scoreboard name used to find your row (fetched from the server if unset) |
| `capture_output` | Display/output name to capture (`--list-outputs`) |
| `data_dir` | Store/log/debug location (default `~/.local/share/scuffed-stat-tracker`). Must be absolute. The systemd unit can write this path, the config dir, and the session runtime dir; a custom directory gets a drop-in at install (see below) |
| `auto_detect.*` | Poll-based match start/end detection (interval, cooldown) |
| `game_process_names` | Only capture while one of these processes runs (empty disables the gate) |
| `debug_ocr` | Dump Tab OCR intermediates and poll Victory/Defeat evidence frames (confirm + first streak, not every tick) under `{data_dir}/debug/` (also env `STAT_TRACKER_DEBUG_OCR=1`) |
| `finished_game_close_secs` | Quiet time after the last activity before a finished game is closed and uploaded. Activity is a stored capture, a recorded outcome, an accolade map, or the session open. Never shorter than the 75-second grace. Default 180. Config-file only (no Settings control) |
| `ocr_threads` | Parallel OCR workers (1–8). Each keeps a ~23 MB Tesseract model in RAM. Omit for auto (`(cores/2)` clamped 2–4). Also env `STAT_TRACKER_OCR_THREADS` or CLI `--ocr-threads N`. Use `1` to minimize RAM; higher speeds Tab OCR. |
| `shadow_recognizer` | Experimental, off by default. Runs a template digit matcher on each accepted scoreboard in a background thread and logs where it disagrees with OCR to `{data_dir}/shadow/digits.jsonl` (values and confidences only, capped at about 4 MB). Stored stats and uploads are unchanged. Settings toggle: Extra number reader (test). Also env `SCUFFED_SHADOW_RECOGNIZER=1` for one run, which is not written to this file. The tracker reads the file when it starts, so restart it after saving. |
| `setup_completed` | Set when the first-run guide is finished or skipped. Missing or false shows the guide on launch. |
| `reader_pack_url` | Optional https address of a reader template pack. The setup guide offers the download when this is set, and skips that step when it is empty. |

Example low-RAM:

```toml
ocr_threads = 1
```

The daemon reads config once at startup — restart it after changes.

### Where the daemon writes

The user unit sets `ProtectSystem=strict`. When that sandbox is applied,
only these paths are writable:

| Path | What lands there |
|---|---|
| `~/.local/share/scuffed-stat-tracker` | Default data dir. Tessdata always lives at `tessdata/` here, even when `data_dir` is custom |
| `~/.config/scuffed-stat-tracker` | `config.toml` (mode 0600) and `session.env` |
| `$XDG_RUNTIME_DIR` (`%t`) | Wayland, X11, D-Bus, and PipeWire sockets |

`{data_dir}` holds `stats.surrealkv`, `commands/`, `debug/` PNGs, portraits,
`daemon.pid`, `sync_auth.json`, `live_snapshot.json`, `active_game.json`,
`daemon.log`, and vacuum backups. The default directory is already in the
table above. An absolute `data_dir` anywhere else — including another
folder under `$HOME` — is read-only under the sandbox. `install.sh` writes
`~/.config/systemd/user/scuffed-stat-tracker.service.d/data-dir.conf` for
that path. Reinstall after you change `data_dir`. If the directory is
still not writable, the daemon exits and prints the same drop-in. A
relative `data_dir` is not put in the unit.

`%h` in the unit is `$HOME`. It does not follow `XDG_DATA_HOME` or
`XDG_CONFIG_HOME`. On a user manager that cannot apply the sandbox, the
drop-in is unused and the startup check still requires a writable
`data_dir`.

These writes are not the long-running unit, so they are not in
`ReadWritePaths`:

- `--generate-tessdata` (CLI or the GUI button) writes
  `~/.local/share/fonts` and a temp dir, then exits.
- The GUI updater stages the tarball under `/tmp/sst-gui-update-*`.
  `PrivateTmp` stays off the daemon so X11's `/tmp/.X11-unix` remains
  visible. `PrivateDevices` and `DeviceAllow` stay unset so `/dev/input`
  hotkeys keep working.

## Data & IPC

Single-process SurrealKV store at `{data_dir}/stats.surrealkv`. Because only
one process can hold it, the daemon exports `live_snapshot.json` after
mutations (debounced) and appends to `matches.jsonl`; the GUI reads those when
the daemon holds the lock and sends manual edits through a file command queue
(`{data_dir}/commands/`).

## Troubleshooting

**App launcher does not start `stat-tracker-gui` (works from a terminal).**
Graphical sessions (GNOME, Cosmic, **AerynOS**) often have a PATH that
does not include `~/.local/bin`. Through **v0.4.2** the `.desktop` `Exec=`
was the bare binary name, so the launcher could not find it. **v0.4.3**
writes absolute `Exec=` / `TryExec=` to `$PREFIX/bin/stat-tracker-gui`.
Reinstall, then if the menu entry is stale:

```sh
update-desktop-database ~/.local/share/applications
```

`gtk-update-icon-cache` is not required (`Icon=applications-games` is a
theme name, not a file we install). Log out/in if the launcher still
caches the old entry.

**`stat-tracker-gui` panics with `Failed to load ayatana-appindicator3` (v0.4.0 / v0.4.1).**
`tray-icon` loads `libayatana-appindicator3.so.1` or `libappindicator3.so.1`
at runtime. Distros that do not ship Ayatana AppIndicator (including
**AerynOS**) used to abort the GUI. **v0.4.2** starts the Iced window
without a tray and logs a warning. Hide-to-tray will not work until you
install the optional package (if your distro has it):

```sh
# Debian / Ubuntu
sudo apt install libayatana-appindicator3-1
# Fedora
sudo dnf install libayatana-appindicator-gtk3
# Arch
sudo pacman -S libayatana-appindicator
```

AerynOS may not package this library. Closing the main window quits when
there is no tray.

**`stat-tracker-gui` fails with `OPENSSL_3.2.0 not found` (v0.4.0).**
v0.4.0 bundled Ubuntu 22.04 `libcrypto.so.3` / `libssl.so.3` into
`~/.local/lib`. Both binaries used RUNPATH `$ORIGIN/../lib`, so that copy
won over `/usr/lib` and broke hosts whose `libcryptsetup` needs OpenSSL
3.2 (Aerynos). Install **v0.4.1** (the installer removes those leftovers)
or delete the tracker-owned files:

```sh
rm -f ~/.local/lib/libcrypto.so.3 ~/.local/lib/libssl.so.3
```

**Daemon runs under systemd but never captures (`CaptureBackend::None`).**
Wayshot needs `WAYLAND_DISPLAY`, X11 needs `DISPLAY`, and the portal probe
looks at `XDG_CURRENT_DESKTOP`. A process the GUI spawns itself inherits
the session. The user unit does not. GNOME and KDE import those variables
into `systemd --user`; Sway and Hyprland do not, and
`graphical-session.target` does not either. An import done once at install
also dies on logout, and the socket name can change the next time the
compositor starts.

Reinstall so the unit on disk is rewritten, then restart the daemon from
the GUI (or `systemctl --user restart scuffed-stat-tracker.service`).
Install does three things:

1. `ExecStart` becomes the absolute `$PREFIX/bin/scuffed-stat-tracker`.
   A custom prefix used to keep launching `~/.local/bin`.
2. It imports `WAYLAND_DISPLAY`, `DISPLAY`, `XDG_CURRENT_DESKTOP`, and
   `XDG_SESSION_TYPE` into the user manager when `systemctl --user` works.
3. It installs `scuffed-stat-tracker-session.service`, a oneshot that runs
   before every daemon start. The oneshot reads those variables from the
   compositor's `/proc/<pid>/environ` (sway and Hyprland first, then other
   known compositors, then Xorg/Xwayland) and writes
   `~/.config/scuffed-stat-tracker/session.env`. The daemon unit loads that
   file, so a later login still has a display socket even though the
   manager import from step 2 is gone.

The compositor value wins over whatever the user manager already had, so a
stale `wayland-1` does not hide a new socket. A shell that is not a
compositor is ignored.

Caveats:

- SSH, or install before the compositor is up, with none of those variables
  in the environment: the manager import is skipped and `session.env` is
  empty until a later start while a recognized compositor is running.
  Start the daemon again after you log in; the oneshot retries.
- No user bus (`systemctl --user` fails): the unit and `session.env` are
  still written. The manager import is retried, with a 5 second timeout,
  each time the daemon starts.
- A compositor not in the helper's list is invisible. From a terminal in
  that session run `systemctl --user import-environment WAYLAND_DISPLAY
  DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE` and restart the daemon.
- Do not install with `sudo`. The helper only reads processes of the
  installing user. `session.env` is always `~/.config/scuffed-stat-tracker/`
  (the unit's `%h`), not `$XDG_CONFIG_HOME`.

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
