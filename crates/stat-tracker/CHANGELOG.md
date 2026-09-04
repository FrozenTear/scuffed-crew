# Stat Tracker changelog

User-facing notes for `stat-tracker-v*` GitHub Releases. The release workflow
prepends the section whose heading matches the tag version (for example
`## 0.4.5` for `stat-tracker-v0.4.5`).

## 0.4.5

Compact Settings layout (#70): section cards, capped field widths, and a
two-column row for short numbers. Companion hotkey setting is unchanged.

Split-game dedupe (#71): reuse an unfinished same-map/hero session within
~20 min instead of opening a second empty Games card. Tab debounce and the
1800s session grouping window are unchanged.

Maps visual polish (#72): compact 2–4 column map cards with a WR bar and
win/loss stripe.

Daemon OCR / capture / sync / store schema are unchanged. Packaging hotfixes
0.4.1–0.4.3 and the companion overlay hotkey (0.4.4) are unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.5`.

## 0.4.4

Companion overlay show/hide hotkey (#68). **Settings → Companion**: enable
(default on) and a bind field, default **Super+Shift+C**. The overlay stays
click-through (`KeyboardInteractivity::None`); Esc does not apply.

The **main GUI process** reads `/dev/input` with **evdev** (same path as
daemon Tab capture — not X11 `XGrabKey`). Needs the `input` group or seat
`uaccess`, same as Tab. OverlayHold is the same as the tray / header
**Hide / show overlay**: hide sticks until the game ends; the shortcut
shows the overlay again mid-session if you press it while hidden.

Daemon OCR / capture / sync / store schema are unchanged. Desktop-launcher
absolute Exec (0.4.3), optional-tray (0.4.2), and OpenSSL packaging (0.4.1)
are unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.4`.

## 0.4.3

Packaging hotfix for laptop installs after v0.4.2: `stat-tracker-gui` ran
from a terminal but **not from the app launcher**. The installed `.desktop`
had a bare `Exec=stat-tracker-gui`. Graphical sessions (Cosmic / GNOME /
**AerynOS**) often omit `~/.local/bin` from launcher PATH even when a
login shell has it.

- Installer writes absolute `Exec=` and `TryExec=` to
  `$PREFIX/bin/stat-tracker-gui` (default `~/.local/bin/stat-tracker-gui`).
- `Icon=applications-games` is a Freedesktop **theme name**, not a file
  path. A missing theme icon only drops the pictogram; it does not block
  launch. `gtk-update-icon-cache` is not required.
- After install, `update-desktop-database ~/.local/share/applications`
  (already run when `desktop-file-utils` is present; printed as a hint
  when it is not).

Daemon OCR / capture / sync / store schema are unchanged. OpenSSL
packaging (0.4.1) and optional-tray (0.4.2) are unchanged. Reinstall to
refresh the `.desktop` file.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.3`.

## 0.4.2

Hotfix for laptop installs after v0.4.1: `stat-tracker-gui` panicked on start
when `libayatana-appindicator3` / `libappindicator3` was missing:

```
Failed to load ayatana-appindicator3 or appindicator3 dynamic library
```

`tray-icon` → `libappindicator-sys` `dlopen`s those sonames and used to
`panic!` if neither loaded. The Iced window now starts without a tray
(warning + toast). Hide-to-tray needs the system lib; closing the window
quits when there is no tray.

- Optional Ayatana AppIndicator package on distros that ship it
  (Debian/Ubuntu: `libayatana-appindicator3-1`, Fedora:
  `libayatana-appindicator-gtk3`, Arch: `libayatana-appindicator`).
- AerynOS may not ship it — the main window still works.

Daemon OCR / capture / sync / store schema are unchanged. OpenSSL packaging
from 0.4.1 is unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.2`.

## 0.4.1

Hotfix for laptop installs of v0.4.0: `stat-tracker-gui` failed on hosts
with a newer system OpenSSL (`OPENSSL_3.2.0 not found`, required by
`libcryptsetup`) because the release bundled Ubuntu 22.04 `libcrypto.so.3`
into `~/.local/lib` and both binaries' RUNPATH (`$ORIGIN/../lib`) searched
that copy first.

- Do **not** bundle `libcrypto` / `libssl` (host OpenSSL wins).
- Isolate OCR `.so`s under `$PREFIX/lib/scuffed-stat-tracker/ocr` (daemon
  RUNPATH) and libxdo under `…/gui` so they are not on the GUI RUNPATH.
- Reinstall removes leftover v0.4.0 sonames from `$PREFIX/lib`.

Daemon OCR / capture / sync / store schema are unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.1`. If you already installed v0.4.0,
run the 0.4.1 installer (it deletes the leftover `libcrypto.so.3` /
`libssl.so.3` from `~/.local/lib`).

## 0.4.0

First Iced-only desktop release. Latest published before this cut was
`stat-tracker-v0.3.4`. Daemon OCR / capture / sync / store schema are unchanged.

### Highlights

- **Iced 0.14 redesign** (`scuffed-stat-tracker-ui`): Overview, Games, Heroes,
  Maps, Seasons, and Settings. Same snapshot + `StoreCommand` contract as before.
- **Companion overlay** — layer-shell panel (`stat-tracker-gui --companion`)
  that sits above fullscreen Overwatch while the game process is running.
- **Dioxus GUI removed** (P5 / #61). The daemon crate is daemon-only; Iced is
  the sole desktop UI.
- **Reinstall keeps the `stat-tracker-gui` binary name.** Desktop entry, PATH,
  and tarball layout are unchanged — run `./install.sh` (or the bootstrap
  one-liner) to replace a Dioxus binary in place.

### Requirements

- **Daemon:** glibc ≥ 2.35; OCR libraries are bundled.
- **GUI:** GTK 3 + a Vulkan-capable GPU/compositor (or Iced software fallback)
  + glibc ≥ 2.35.
- **Host:** Linux + Wayland (or experimental X11) and membership in the
  `input` group. `eng` + `koverwatch` tessdata are bundled.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract `scuffed-stat-tracker-linux-x86_64.tar.gz` and run `./install.sh`.
Pin a tag with `STAT_TRACKER_TAG=stat-tracker-v0.4.0`.
