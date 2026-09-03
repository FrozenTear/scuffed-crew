# Stat Tracker changelog

User-facing notes for `stat-tracker-v*` GitHub Releases. The release workflow
prepends the section whose heading matches the tag version (for example
`## 0.4.0` for `stat-tracker-v0.4.0`).

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
