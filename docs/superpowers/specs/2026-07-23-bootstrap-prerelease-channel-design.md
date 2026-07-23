# bootstrap.sh release-channel selection (stable vs prerelease) + uninstaller

**Date:** 2026-07-23
**Scope:** `crates/stat-tracker/dist/{bootstrap.sh,install.sh,uninstall.sh}`,
`crates/stat-tracker/README.md`, `.github/workflows/stat-tracker-release.yml`

## Problem

The curl one-liner installer resolves the newest **stable** release only —
`resolve_release` skips anything flagged `prerelease`. RC builds (e.g.
`stat-tracker-v0.3.0-rc4`) are therefore unreachable except by pinning
`STAT_TRACKER_TAG`. Testers should be able to opt into prereleases without
knowing exact tag names.

## Interface

- New env var `STAT_TRACKER_CHANNEL` = `stable` (default) | `prerelease`.
  Any other value → error before any network call.
- `STAT_TRACKER_TAG` keeps top precedence: pinning a tag bypasses channel
  logic entirely (unchanged).
- Interactive prompt: fires only when **all** hold — `STAT_TRACKER_CHANNEL`
  unset, a prerelease newer than the newest stable ships the asset, and
  `/dev/tty` is readable+writable. Prompt shows both tags, reads from
  `/dev/tty`, defaults to stable on empty input or EOF. No tty → stable
  silently (CI-safe; today's behavior).

## Resolution logic

One pass over `/releases?per_page=20` (newest-first, drafts excluded by the
API… drafts are also skipped explicitly) collecting two candidates that ship
`scuffed-stat-tracker-linux-x86_64.tar.gz`:

1. the newest **stable** release, and
2. the newest **prerelease** that is newer than that stable (i.e. appears
   before it in the list).

Channel decision in bash:

| Channel | Prerelease found | Stable found | Result |
|---|---|---|---|
| `prerelease` | yes | – | prerelease |
| `prerelease` | no | yes | stable, with info note |
| `stable` / unset | – | yes | stable (or prompt, see above) |
| `stable` (explicit) | yes | no | error, hint to use channel/tag |
| unset | yes | no | prerelease, with warning |
| any | no | no | error (as today) |

The chosen tag is then extracted from the already-fetched JSON; the final
tag/url/sha extraction step is unchanged.

## Docs

Header comment in `bootstrap.sh` and the one-liner section of
`crates/stat-tracker/README.md` document `STAT_TRACKER_CHANNEL` and the
prompt.

## Testing

No shell test harness exists for this script. Manual verification against the
live GitHub API using a throwaway `PREFIX` + `SKIP_INTEGRATION=1`:

- default, no tty → resolves newest stable (v0.2.2 today)
- `STAT_TRACKER_CHANNEL=prerelease` → resolves v0.3.0-rc4
- `STAT_TRACKER_CHANNEL=stable` → resolves v0.2.2
- `STAT_TRACKER_CHANNEL=bogus` → early error
- prompt path under a pty (`script -qec …`), answering `p` and empty

## Uninstaller (second increment, same branch)

Problem: no uninstall existed, and the 33 bundled OCR libs land in the
*shared* `$PREFIX/lib` under generic sonames — unidentifiable after the
tarball is gone.

- `install.sh` records every file it installs (bins, uninstaller, libs,
  desktop entry, systemd unit — **not** tessdata, which is app data) in
  `$PREFIX/share/scuffed-stat-tracker/install-manifest.txt`. Reinstalls merge
  with the previous manifest (`sort -u`) so upgrades that drop files leave
  them removable. `$PREFIX`-relative placement keeps throwaway-PREFIX smoke
  installs from touching the real `$HOME`.
- New `dist/uninstall.sh`, shipped at tarball root and installed as
  `$PREFIX/bin/scuffed-stat-tracker-uninstall`. Behavior: `systemctl --user
  disable --now` the unit (best-effort), remove manifest entries (absolute
  paths only, files/symlinks only), refresh desktop DB / systemd, then handle
  data (`~/.local/share/scuffed-stat-tracker`) + config
  (`~/.config/scuffed-stat-tracker`): kept by default, deleted with `--purge`
  or an interactive `[y/N]` prompt (stdin tty; `--yes` suppresses).
- No manifest (pre-manifest install): falls back to known default paths and
  prints the `tar tzf … | grep '/lib/'` recipe for the libs.
- Release workflow stages + chmods `uninstall.sh` into the tarball.

Verified against the real rc4 tarball in a fake `$HOME`: install → manifest
(38 entries) → default uninstall removes all 38, keeps data; `--purge` leaves
an empty tree; manifest-less fallback removes the 5 known paths and warns
about libs; pty prompt answering `y` purges.
