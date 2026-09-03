# Stat Tracker changelog

User-facing notes for `stat-tracker-v*` GitHub Releases. The release workflow
prepends the section whose heading matches the tag version (for example
`## 0.4.1` for `stat-tracker-v0.4.1`).

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
