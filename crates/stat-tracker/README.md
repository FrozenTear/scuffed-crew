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
  - The portal is the fallback on Wayland (slower; not ideal for the poller).
    On a session that is X11 for sure (`XDG_SESSION_TYPE=x11` and no
    `WAYLAND_DISPLAY`), Automatic never falls back to it: there is no screen
    stream there, so if X11 capture fails nothing captures and the app says
    so. A session that is neither known Wayland nor known X11 may still fall
    back to the portal.
- **Keyboard access via evdev.** Tab detection (daemon) and the companion
  overlay show/hide shortcut (GUI, default Super+Shift+C) read `/dev/input`.
  The user must be in the `input` group, or have seat `uaccess` on those
  nodes. No X11 key grab.

  Add the group, then log out and back in. A new terminal is not enough, since
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
| **Daemon** | glibc ≥ 2.35 (Ubuntu 22.04+, Debian 12+, Fedora, Arch, openSUSE, RHEL 9+, Linux Mint 21) | OCR `.so` closure is **bundled** in `lib/scuffed-stat-tracker/ocr`, including Tesseract 5.5.3 (`libtesseract.so.5`, Ubuntu 22.04 leptonica 1.82). Installer copies that tree so RUNPATH `$ORIGIN/../lib/scuffed-stat-tracker/ocr` works. OpenSSL is **not** bundled. |
| **GUI** | glibc ≥ 2.35 + **GTK 3** + Vulkan (or Iced software fallback) | Iced 0.14 (`scuffed-stat-tracker-ui`), binary name `stat-tracker-gui`. Tray (Hide-to-tray) needs **Ayatana AppIndicator** when the distro ships it; without it the window still starts. |
| **Host still needed** | Wayland **or** X11 + `input` group | Capture, compositor, and keyboard access stay host-provided. The release tarball bundles `eng.traineddata`. |

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
kept), since most hosts cannot regenerate it locally (`text2image`
hangs/segfaults with pango ≥ 1.56).

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
TAG=stat-tracker-v0.6.0-alpha.1.2
curl --proto '=https' -fsSL "https://raw.githubusercontent.com/FrozenTear/scuffed-crew/${TAG}/crates/stat-tracker/dist/bootstrap.sh" \
  | STAT_TRACKER_TAG="$TAG" STAT_TRACKER_PREFIX="$HOME/.local" bash
```

On Arch-based distros the alpha builds are also on the AUR as
[`scuffed-stat-tracker-alpha-bin`](https://aur.archlinux.org/packages/scuffed-stat-tracker-alpha-bin).
It provides and conflicts with `scuffed-stat-tracker`, so it replaces
`scuffed-stat-tracker-bin` and the other way round. Saved games and settings in
your home folder stay. Both packages use the same
`scuffed-stat-tracker.service` user unit, so after switching run
`systemctl --user daemon-reload` and
`systemctl --user restart scuffed-stat-tracker.service`.

The tarball's `.sha256` is checked before extract. A missing `.sha256` asset
stops the install. With no minisign public key configured, bootstrap warns and
falls back to that sha256 from the same GitHub release. Once a public key is
configured, a missing or bad `.minisig` refuses the install. It does not fall
back to sha256.

**Manual:** download the tarball and its `.sha256` from the release page.
With no minisign public key, the check warns and falls back to that sha256.
A configured key refuses a missing or bad signature. Extract, then:

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

One thing in the saved-games folder goes on every uninstall, kept data or
not: `private/own-name/`, the small crops of your own in-game name that
`reader = "new"` keeps to find your row (see below). The uninstaller never
follows a link and never deletes outside your home folder or the install
folder, so it leaves the crops when the saved-games folder is a link, lies
outside your home folder, or when a system package owns this copy.
`scuffed-stat-tracker-uninstall` says so; the app's Uninstall does not list
them in that case. You can delete `private/own-name/` by hand at any time:
the tracker learns the crops again.

If a system package owns this copy, nothing is removed. The app shows
`sudo pacman -R <pkg>` or `sudo apt remove <pkg>` and can copy that command.
An AppImage is removed like any other copy, including the AppImage file.
A copy outside your home folder is left alone. The app lists the paths and
says to remove them manually.

## Running

```sh
# daemon (foreground; logs to stderr)
cargo run -p scuffed-stat-tracker

# desktop GUI (Iced, crate scuffed-stat-tracker-ui, binary stat-tracker-gui)
cargo run -p scuffed-stat-tracker-ui
```

Replacing an old Dioxus `stat-tracker-gui`: reinstall (prebuilt `./install.sh`,
or `crates/stat-tracker/install.sh` from a source checkout). The binary name
does not change; only the implementation does.

First-run sync setup: `scuffed-stat-tracker --token <daemon-token> --server
https://…` writes `~/.config/scuffed-stat-tracker/config.toml` (chmod 600:
it holds the bearer token). Tokens are minted in the site under
My Stats → Daemon Tokens.

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
| `capture_route` | `auto` (default), `screencast`, `screenshot`, or `kwin` (KDE direct). See below. A non-empty `STAT_TRACKER_CAPTURE` wins over this |
| `debug_portal_frames` | Off by default. Saves up to four Tab pairs under `{data_dir}/debug/portal-compare/` (`0001-stream.png` and `0001-screenshot.png`). Also `STAT_TRACKER_DEBUG_PORTAL_FRAMES=1`, which is not written to this file. The pictures stay on this machine and are not uploaded. Leave it off while a voice app is sharing the screen: the screenshot half can hit the KWin cursor crash |
| `data_dir` | Store/log/debug location (default `~/.local/share/scuffed-stat-tracker`). Must be absolute. Required in the file: a `config.toml` without it does not load, and the default applies only when there is no file. `~` is not expanded, so write the full path. The systemd unit can write this path, the config dir, and the session runtime dir; a custom directory gets a drop-in at install (see below) |
| `auto_detect.*` | Poll-based match start/end detection (interval, cooldown). `poll_interval_secs` is whole seconds; 4 means automatic (1.5 s with the new reader, 4 s otherwise). `poll_interval_ms`, when set, overrides that (500 to 10000). It is not written unless you set it |
| `screencast_max_fps` | Screen-stream cap, 1 to 5 frames per second. Missing means 4 (2 before 0.5.0-rc.6; set 2 to go back). Sets the PipeWire max-framerate and the tracker's own throttle. Not written unless you set it |
| `screencast_pause_between_grabs` | Missing or `true`: the screen stream is paused between grabs on KWin 6.1 or newer and on niri, so the compositor does not draw frames the tracker would throw away (see Screen capture route). Other desktops keep it running. `false` keeps it running everywhere, as before 0.5.0-rc.7. Put it near the top of the file, above any `[section]` line. Env `STAT_TRACKER_SCREENCAST_PAUSE=1` forces pausing on any desktop (for testing) and `=0` turns it off; the env var wins over this key and is not written to this file. Forced on, damage-driven portals (xdg-desktop-portal-wlr, -hyprland, possibly COSMIC) send no frame for a still screen and may trip the fail-safe, and Plasma 5 / KWin 6.0 (and GNOME) get their per-resume signal leak back. Not written unless you set it |
| `game_process_names` | Only capture, and poll for match start and end, while one of these processes runs, so a Tab press on the desktop or in another app does not record garbage frames (empty disables the gate) |
| `debug_ocr` | Dump Tab OCR intermediates under `{data_dir}/debug/` and poll Victory/Defeat evidence frames under `{data_dir}/debug/poll/` (confirm + first streak, not every tick) (also env `STAT_TRACKER_DEBUG_OCR=1`). It also turns on `debug/accepted/` (the last 60 accepted scoreboard crops) and `debug/rejected/` (the last 30 rejected frames). Those images are not blanked and show other players' names, so they are off by default. Settings toggle: Save debug images from each capture. With it off, an existing folder is left as it is and the daemon logs once at start that it is no longer written. `debug/mapcrops/` and `debug/mapmiss/` hold only the map-name box and stay on |
| `debug_save_pregame_screens` | Off by default. Set it in the file or with **Save pre-game screens** under Settings, Advanced, Troubleshooting in the desktop app (tagged After restart). Restart the tracker after a change. Saves full-size PNGs of the pre-game screens under `{data_dir}/debug/pregame/<YYYYMMDD_HHMMSS>/` (UTC), one folder per game: the first and last frame of the map vote, the hero ban and the hero select, one ban-results frame, and the loading screen within 3 minutes of them. The ban-results frame is taken from the polls after the ban vote ends (from 1.5 s later, or 4 s with ocr-v1) whose title region looks like the ban-results title. The ban icons and scores fill in under the title while the screen stays up, so each later poll that still shows the title replaces the held frame and confirms the one before it, and the latest confirmed one is saved (one frame is held in memory; the confirmed one before it is staged on disk as a hidden `.ban_results.png.staged` until the results end, then renamed or deleted). With no examples this is a built-in shape rule (bright, centred title text on a dark ground), so it works out of the box. Example screenshots of the ban-results screen in `{data_dir}/templates/pregame/ban_results/` replace the shape rule: with any example loaded, the title must match one of them and the shape rule is not used. Examples are read when the tracker starts (restart after adding them), regular `.png` files only (a `.jpg`, which Steam saves by default, is ignored, and so are symlinks), and of those the first 16 by name; none are shipped. The first frame is a candidate only: it is kept when it is confirmed by one more non-phase frame (not a map vote, hero ban or hero select; the loading screen counts), and only within 15 seconds plus one poll interval of the first candidate frame. With `reader = "new"` a frame showing the hero select title also ends the wait (the title mask is loaded only with that reader), and on the very next poll it confirms the held frame, first or later (that title is read with no lag), so a single results frame right before the select title is saved. A later frame that replaced it is confirmed only by the next frame with the results title, or by that select title frame. A plain frame after it means it is not confirmed, since it may be the select sliding in: the confirmed frame before it is then saved when the hero select reads, the game closes or capture stops, inside the window (the window running out drops both). A lone results frame right before a hero select read, or before the game closes, is not saved: nothing tells it from the select sliding in. Staged and temporary files left by a stopped run are deleted when the tracker starts and when the next game folder is made, with any game folder they leave empty; a folder holding only such files never counts toward the 20 games. The loading screen is the ENTERING GAME strip with its text centred and about as wide as those words. Each frame that passes that check is staged on disk (a hidden `.loading.png.staged`) and only the last one is saved, and only when the game starts: a hero ban or hero select read within 12 seconds of it. A loading frame with no game start after it is dropped: no hero ban or hero select read within 12 seconds (the menu, a lobby), the game closing, capture stopping, the tracker stopping, or a requeue's map vote; a later ENTERING GAME frame in the same game is still saved. So an ENTERING GAME frame after a game update band (APPLYING UPDATE) replaces it, but an update band that passes the check and is not followed by an ENTERING GAME frame before the game starts can still be saved as the loading screen. The check's bounds have no measurements from real frames yet; the opt-in `SCUFFED_LOADING_FRAMES_DIR` test in `tests/integration/loading_frames.rs` is how they are supplied. A game starts at its map vote or hero ban; the hero select between rounds is not saved. The images are not blanked and show other players' names (the hero select shows your team's), so they are off by default. About 5 to 8 files per game, 3 to 5 MB each at 1440p: about 0.5 to 0.8 GB for 20 games. The newest 20 game folders are kept and older ones are deleted (a small `.order` file there remembers which are newest, even with a wrong clock). The files stay on this PC: nothing uploads, syncs or reads them. Off means no extra work and no new files; turning it off leaves `debug/pregame/` as it is, and the daemon logs once at start that it is no longer written |
| `finished_game_close_secs` | Quiet time after the last activity before a finished game is closed and uploaded. Activity is a stored capture, a recorded outcome, an accolade map, or the session open. Never shorter than the 75-second grace. Default 180. Config-file only (no Settings control) |
| `ocr_threads` | Parallel OCR workers (1–8). Each keeps a ~23 MB Tesseract model in RAM. Omit for auto (`(cores/2)` clamped 2–4). Also env `STAT_TRACKER_OCR_THREADS` or CLI `--ocr-threads N`. Use `1` to minimize RAM; higher speeds Tab OCR. |
| `shadow_recognizer` | Experimental, off by default. Runs a template digit matcher on each accepted scoreboard in a background thread and logs where it disagrees with OCR to `{data_dir}/shadow/digits.jsonl` (values and confidences only, capped at about 4 MB). With hero icon templates installed in `{data_dir}/templates/heroes/`, the same thread also reads each row's hero and logs it to `{data_dir}/shadow/heroes.jsonl`. It is a log only: with `reader = "ocr-v1"`, stored stats, the capture gate and uploads still come from OCR. When the tracker runs the new reader (`reader = "new"`, or `ocr = "off"` granted), this log runs whatever this key says. Settings toggle: Extra number reader (test). Also env `SCUFFED_SHADOW_RECOGNIZER=1` for one run, which is not written to this file. The tracker reads the file when it starts, so restart it after saving. |
| `reader` | Which reader supplies saved and uploaded stats: `"new"` (the default since alpha.12, also when there is no `reader` line) or `"ocr-v1"` (Tesseract). Settings writes `"ocr-v1"` explicitly, so a switch back stays after a reload. With `"ocr-v1"`, Tesseract reads the scoreboards, the map in the top bar, the result words, the FINAL SCORE line and the game-start screens, and the name column of every scoreboard is read to look for your `player_name`. With `"new"`: while OCR is on, a scoreboard the new reader finds nothing on (not found, team size unknown, no digit values) is read with ocr-v1 for that Tab and stored as read by `ocr-v1`. On every other Tab the six stats come from the new reader only: a scoreboard it does not vouch for is not stored, and a stat it is unsure of keeps the game's last accepted value and is marked, with no OCR number in its place. A game is stored only once each of the six stats has had a sure read on some Tab: a stat never read is not stored or uploaded as 0 (see `debug/unrecorded_games.jsonl` below). Your row is found as described under `private/own-name` below. The map in the top bar falls back to OCR when the banner reader is not sure (unless OCR is off). The result words, the FINAL SCORE line and the game-start screens are read from the words pack, with OCR only where a template or mask is not loaded; the accolade and rank words and the accolade map are still read with OCR. When `ocr = "off"` is granted, the new reader runs whatever this key says, and the key keeps its value; when the tracker keeps OCR on, this key applies as usual |
| `name_reader` | On unless the file says `false`, with either `reader`. Your own row's name is read with letter templates from the `names-v1` reader pack instead of Tesseract: a row is used when its confidence meets the pack's own `min_confidence`, and a blank row, or one under that threshold, is read with Tesseract. With no `names-v1` pack installed, names are read with Tesseract. A default install downloads the pack with the other reader packs. With `ocr = "off"` there is no name read, so this key has no effect. Not written unless set to `false` |
| `ocr` | `"on"` (default) or `"off"`, in any case. Off: the tracker reads every screen with its own reader and does not start Tesseract for reading (no Tesseract instance, no OCR workers, no tessdata lookup). Install or Rebuild reading model in Settings still runs the Tesseract tools when you press it. Off implies the new reader, so `reader` is not consulted and keeps its value. The tracker turns OCR off only when nothing would go unread. That needs a tracker version that reads everything without Tesseract, and three reader packs: a hero pack (heroes-v2 or heroes-v1; heroes-v2-1080 alone is not enough), maps-v1, and a words-v1 with the result words for the end screens, the match-start titles and the end-of-match titles. Otherwise it keeps OCR on for that run and logs why, and Settings refuses the switch with the same reason. With OCR off the map name on the scoreboard has no second reader, so a game whose map is never read is saved without a map until you pick one. For the same reason a co-op event scoreboard (Junkenstein on Adlersbrunn, stats on one team only) is dropped only when its map name is read sure; when it is not, that scoreboard can be saved as a game (not seen on a real frame so far). With OCR off, a Tab is not stored when you have never had the highlight, your hero icon is dead or covered, and no seed is trusted yet (a seed is a learned crop of your name, see `private/own-name` below; with OCR on, the name read finds your row on such a Tab). Two more costs come with the new reader that off implies: a game that ends with a stat never read is not stored (see `debug/unrecorded_games.jsonl` below), and a result screen takes longer to read (about 65 ms against 25 ms per check). Turning OCR back on in the middle of a game keeps what the game has: a stat with no sure read yet stays open until a scoreboard reads it, and the game has no row before that (see `debug/unrecorded_games.jsonl` below). If a pack goes away while the tracker runs with OCR off, what that pack reads is held and the app says which pack it is; the tracker does not go back to OCR by itself. Settings toggle: Use classic OCR (Tesseract). Also env `STAT_TRACKER_OCR=on` or `off` for one run, which is not written to this file. A config from before this key has no `ocr` line and behaves as before (on); nothing needs migrating. The tracker reads the file when it starts, so restart it after saving. What the running tracker decided, and the OCR calls it stopped, are in `{data_dir}/ocr_switch.json` |
| `setup_completed` | Set when the first-run guide is finished or skipped. Missing or false shows the guide on launch. |
| `reader_pack_url` | Not used. Reader packs come from the signed-in site, and the setup guide always shows its reader pack step. Older files that still set it load fine. |
| `player_names` | More in-game names that are all yours (alts, a renamed account). A `#1234` tag is dropped and names under 3 characters are ignored. Empty by default and not written while empty. Not read by this build yet: your row is still found with `player_name` only |
| `live_upload` | Upload each game to the site after it closes. On by default and not written while on; `false` is meant to keep games on this computer, queued. Not read by this build yet: games still upload as before |
| `rank_reader` | Read your rank after Comp games. Off by default and not written while off. Not read by this build yet |
| `share_stat_pictures` | Upload the grayscale stat area after each game to help train the reader (no names, kept 30 days on the server). Off by default and not written while off. Not read by this build yet: nothing is uploaded |
| `share_report_pictures` | Whether the stat-area pictures on a misread report start ticked. Off by default and not written while off. Not read by this build yet: reports send pictures as before |
| `capture_only_on_screen` | Only capture while the game window is visible on the captured screen. On (Auto) by default and not written while on; when the tracker cannot tell (most Wayland desktops), Auto captures. `false` captures whenever the game runs. Not read by this build yet: capture works as in 0.5 |

Example low-RAM (a config file must set `data_dir`; the default path is
only used when there is no file at all, so copy your own path in):

```toml
data_dir = "/home/<you>/.local/share/scuffed-stat-tracker"
ocr_threads = 1
```

The daemon reads config once at startup. Restart it after changes.
Changing `capture_route` or `capture_output` in Settings restarts a running
tracker for you.

### Screen capture route

On KDE and GNOME Wayland the tracker opens one screen-share (PipeWire) and
hides the cursor, instead of taking a new screenshot for every frame. You
are asked to allow that share once. The permission is remembered in
`screencast-restore-token` inside the data directory, so it is not asked
again after a reboot. Settings → Stored data → Delete all local match data
removes that file too (and its `.tmp`), so the next share asks again.
The compositor is offered a variable framerate with a cap of
`screencast_max_fps` frames per second (4 when that key is unset).
A frame is copied only when the tracker needs one. On KWin 6.1 or newer
and on niri the stream is also paused between grabs, so the compositor
stops drawing frames for it; a poll resumes it for its grab, and a Tab
resumes it from the key press (50 ms after the press at 4 fps, so the
compositor's next frame after the first lands just past the 300 ms overlay
delay) and keeps it running until its retries are over. An open share
stays paused while Overwatch is not running. GNOME, Plasma 5, KWin 6.0,
wlroots portals and desktops the tracker does not recognise keep the
stream running: Mutter and older KWin leak a signal connection on every
resume. The tracker reads the desktop name, `KDE_SESSION_VERSION`, the
running compositor processes and, on KDE, KWin's version (over D-Bus,
once more after 3 seconds on Plasma 6, then from `kwin_wayland --version`)
once at start, and logs one
line saying whether it pauses and why. Pausing also needs PipeWire 1.4 or
newer; older PipeWire keeps the stream running.

A fail-safe watches each resume. A resume is answered when a frame
arrives. On niri, which draws a shared screen only when it changes, the
stream reaching Streaming again also answers it, so a still screen does
not count. KWin always sends a frame on resume, so there a resume that
reaches Streaming with no frame is a broken one. The tracker stops pausing
a stream, keeps it running and logs one warning when resumes stay
unanswered for 1.5 s of active time twice with no answer in between (the
time adds up across resumes), when a Tab misses while its
resume is not answered, when the stream stays paused 1.5 s of active time
after a resume, at once when the stream fails while a resume is not
answered, at once when the stream fails within 2 seconds of a pause the
tracker asked for, or when the stream is closed while a resume is not
answered. During a game the stream pauses about every 1.5 seconds, so that
2-second window is nearly always open; it only leaves out ends on an idle
share. What keeps outside causes from counting: a lost PipeWire connection
(a PipeWire restart) is never counted, and neither is a portal session
the tracker has already seen close. A stream that fails just before the
tracker sees its portal session close can still count.
On KDE a share stopped from the tray or an output change ends the KWin
stream, and xdg-desktop-portal-kde then closes the session. The warning for
a stream that ended just after a pause says it can also be an output or
monitor change or the compositor ending the share. The next share or
reopen tries pausing again once;
after a second such stream it stays off until the tracker restarts. To
turn pausing off yourself, set `screencast_pause_between_grabs = false`,
or put `STAT_TRACKER_SCREENCAST_PAUSE=0` in `~/.config/environment.d/` for
the systemd service (as for `STAT_TRACKER_CAPTURE` below). The log line
`screencast timing (totals since the share opened):` (once a minute while
grabs succeed) shows `resumes`, `avg_resume_ms`, `max_resume_ms` and
`resumes_without_buffer`, counted since the share opened. With
pausing on, the `screencast frames received per second` line (every 3
minutes) reads far below the cap: frames arrive only while a grab holds
the stream. A later poll that sees
no new buffer does not count as a read, so a reused picture cannot confirm
Victory or Defeat by itself. The share closes when Overwatch quits, once
the permission has been saved, and the next game opens it again with that
permission. If the permission was not saved, the share stays up so quitting
the game does not ask again. If you cancel or close the share prompt, the
tracker does not ask again until Overwatch is started again. On KDE, and
GNOME on the automatic stream, it takes no captures and does not fall back
to screenshots. Where a screenshot is already safe, including GNOME when
the stream was forced, screenshots continue. One log line and the app
status say why. The saved permission is kept. Other desktops (niri, Sway,
Hyprland, X11) keep their direct capture unless you force the stream.
When the stream is the whole desktop, it is cut to your monitor with the
monitor layout the tracker keeps for 30 seconds (5 when it could not be
read); the layout is read again sooner when it cannot explain the frame.

If the share cannot start, the tracker logs a warning. On niri, wlroots,
and GNOME when the stream was forced, that grab takes a screenshot, and so
does a Tab or poll grab while the stream is dead or waiting for its
reopen. A poll on a stream that has sent no new frame for 8 seconds does
the same at most once every 10 seconds, except on GNOME, where the portal
screenshot flashes the screen. On niri and wlroots the screenshot is
wayshot (one monitor, no file), and Older screenshot only when wayshot
fails. On KDE, and GNOME on the automatic stream, it skips the screenshot,
including while backing off or after the stream dies. A failed start is not retried
for 30 seconds. If the portal is down, or its cursor-mode property cannot
be read, the log says the portal is unavailable and the saved permission
is kept. The next try after that backoff opens the share again. A share
that never starts streaming is closed and then waits out that same
backoff. Silence while the stream is still running does not reopen it,
and a poll that sees no new buffer does not count. The stream is reopened
when PipeWire reports an error, when it stays unconnected after it had
been streaming, when a pause lasts 2 seconds, when the portal session
closes, or when the connection faults. Two Tab waits that get no PipeWire
buffer do the same. The reopen runs on the next poll, at least 1 minute
after the last reopen. Each further reopen in the same game doubles that
gap, up to 5 minutes, so a stream that stalls again is reopened again
rather than left dead. A new game clears the miss streak and drops the gap
back to 1 minute. It does not clear the clock.
Without a saved permission the stream stays up and one warning is logged.
A permission dialog that times out is a No until the next Overwatch
launch only when no restore token was sent. A timeout after a token was
sent waits 30 seconds and asks again. Stopping the tracker cancels the
dialog. On KDE, and GNOME on the automatic stream, a Tab that sees no
frame after the key press does not fall back to Older screenshot. A Tab
on a direct capture or the screenshot portal still waits 400 ms for the
overlay. On the stream it waits for a buffer at least 300 ms after the
key, and gives up 1200 ms after the key. A buffer from those first 300 ms
does not finish the wait. A poll waits one frame period plus 100 ms before
it reuses the previous frame (350 ms at the default 4 fps, 600 ms at 2 fps),
and 400 ms more when it had to resume the paused stream (750 ms at 4 fps).
The tracker's own pause does not count toward the 2-second pause, the
8-second quiet stream, or the 6-second never-streaming limit: those clocks
count active time, which runs only while the tracker wants the stream
running.

`STAT_TRACKER_CAPTURE`, when set and not empty, wins over `capture_route`.
For the systemd service, put it in `~/.config/environment.d/` (for example
`scuffed-capture.conf`), then log out and back in. Do not put it in
`session.env`: that file is rewritten every time the service starts.

| Value | Route |
|---|---|
| `portal` or `screencast` | Screen stream, cursor hidden, with the automatic fallback above |
| `portal-screenshot` | Older screenshot portal |
| `kwin` or `kde-direct` | KDE direct (below) |
| `wayshot` or `wayland` | Direct Wayland capture |
| `x11` | X11 |

Settings has the same choice: Automatic, Screen stream, Older screenshot,
and on KDE, KDE direct.

**KDE direct** (`capture_route = "kwin"`) asks KWin directly; Plasma shows
no prompt or indicator; images stay in memory and are never saved unless a
debug option is on. It calls KWin's own screenshot interface
(`org.kde.KWin.ScreenShot2`) for the game's monitor, with the cursor left
out, and reads the pixels from a pipe. There is no dialog, no PNG and no
file in Pictures. It needs Plasma 6.8 or later: older Plasma refuses apps
it does not list. With the NVIDIA driver loaded it needs Plasma 6.8.1 or
later: KWin before that can crash when a screenshot meets any app's screen
share (KDE bug 524840), so on NVIDIA a known older Plasma counts as
unavailable and no test picture is taken. At startup the tracker reads the interface version and
takes a 1x1 test picture on one of your monitors; on KDE a failure that can
pass (KWin not up yet at login) is tried again for up to 15 seconds. If it
still fails, the tracker uses the route Automatic would have chosen (the
screen stream on KDE), logs `KDE direct capture is not available (...);
using ... instead`, and Settings shows the reason; restart the tracker to
try again. Unlike the other forced values, which fail closed, `kwin` falls
back this way from the environment variable and from Settings alike. The
app's Settings runs that test picture only when KDE direct is the saved
choice or you pick it, and never while the tracker shares the screen on a
Plasma before 6.8.1; otherwise it reads the version only and says KDE
direct was not checked.

KDE direct never runs beside the tracker's own screen stream. The one
exception is the debug compare pair below, which needs Plasma 6.8.1 or
later (the full fix for KDE bug 524840). A grab waits 400 ms after Tab, as
direct capture does, and retries as usual. The picture is the size KWin
renders the monitor at, its real mode size; it is never resized. If it
differs from the size the tracker last read for that monitor, the tracker
reads the monitor layout again, at most once a minute while it keeps
differing. Automatic does not choose KDE direct yet: route selection is
unchanged.

Older screenshot on KDE takes about half a second of KWin and portal work
per grab. The tracker runs one request at a time (a Tab waits up to 4
seconds for a poll's request to finish, then takes its own; if the request
is still running, that grab counts as busy and is retried while Tab is
held; a poll that finds one running is skipped; in the end-reel window the outcome poll waits up to a second, so
holding Tab over VICTORY does not cost it), a rejected Tab is retried once
instead of twice, and out of a match the poll grabs about every 3 seconds
instead of every tick. KDE direct polls at that rate out of a match too.
The app's Capture now is a separate process and is not in that one-at-a-
time line. On KDE direct that is harmless (KWin handles such calls one
after the other). On Older screenshot, a Capture now and a tracker poll
in the same second can share one portal file, and one of the two can
fail (the one whose file the other already removed; a file that was only
replaced still gives a picture); press Capture now again. On GNOME,
Older screenshot flashes the screen and plays a sound on every capture;
the startup log warns when it is forced there, and Settings says so.
A request the portal does not answer is looked at every 60 seconds. If
the portal went away (a crash or restart), it is given up. If screenshot
permission is already granted (or there is no permission entry and
screenshots have worked before), it is closed and given up, ordinary polls
make no request for at least 30 seconds (then 2, 5 and 10 minutes after
later stalls, and at least 2 minutes while the given-up request's Close is
unanswered), and Settings says the portal stopped answering. Otherwise a
permission prompt may be waiting, possibly behind the game: the request
keeps waiting and Settings says so.
When the environment variable is set, Settings says so and names the file
to edit. The startup log says which source won (`environment`, `settings`,
or `automatic`), the desktop, and the portal backend that answered.

`STAT_TRACKER_DEBUG_PORTAL_FRAMES=1` or `debug_portal_frames = true` saves
up to four Tab pairs in `{data_dir}/debug/portal-compare/` so the two
portal images can be compared. On KDE with Plasma 6.8.1 or later, each
pair also gets the same Tab moment from KDE direct (`NNNN-kwin.png`), to
compare with the stream frame before switching routes. Nothing there is
uploaded. Turn it off before playing while a voice app is sharing your
screen.

### OCR CPU use (`OMP_THREAD_LIMIT`)

The daemon runs Tesseract with `OMP_THREAD_LIMIT=1`. Tesseract is built
with OpenMP, and with no limit each OCR call also spins up its own thread
team on top of the `ocr_threads` workers. The fields are still read in
parallel by the `ocr_threads` workers.

Measured with Tesseract 5 on the development machine: the reads are
identical on 10,784 + 816 samples, text and confidence, for about a third
of the CPU time. Release builds bundle Tesseract 5.5.3 (also built with
OpenMP). The game-font model is still trained with Tesseract 4.1.1 tools.
That bundled 5.5.3 library has not been measured on the game-font model yet.

- The systemd unit sets it (`Environment=OMP_THREAD_LIMIT=1`). An update
  rewrites the unit and restarts a running tracker. The app's Start button
  starts the unit whenever the unit is installed.
- A start outside systemd (a shell, or the app's Start button when no unit
  is installed) has no such line. OpenMP reads the variable when the binary
  loads, so the daemon starts itself again once, same pid and arguments,
  with the variable set. `SCUFFED_OMP_REEXEC` marks that second start and
  stops a loop. This happens before any flag is read, so `--version` and
  `--help` also start themselves again once (same pid, same output), and a
  `--version` process started by an installer or the app has
  `SCUFFED_OMP_REEXEC=1` in its environment.
- A value you set yourself is kept: `OMP_THREAD_LIMIT=4 scuffed-stat-tracker`,
  or for the unit a drop-in (`systemctl --user edit scuffed-stat-tracker`).
  That command writes
  `~/.config/systemd/user/scuffed-stat-tracker.service.d/override.conf`.
  The uninstallers do not remove that file, and it applies again after a
  reinstall: delete it yourself when you no longer want it.
- To run with no limit, leave `OMP_THREAD_LIMIT` unset and set
  `SCUFFED_OMP_NO_REEXEC=1`. In a drop-in for the unit that is
  `UnsetEnvironment=OMP_THREAD_LIMIT` and
  `Environment=SCUFFED_OMP_NO_REEXEC=1`. An empty `OMP_THREAD_LIMIT=`
  counts as unset, so on its own it ends up as 1.
- `--generate-tessdata` does not restart: it runs Tesseract's training tools,
  which would inherit the limit.
- Only the daemon does this. The examples, the replay benchmark and the
  fixture tests run with no limit unless you set the variable for them.

To check what a process got:

- `scuffed-stat-tracker --print-omp-limit` starts a new process and prints
  the value that process ended up with and what OpenMP itself reports. It
  says nothing about a tracker that is already running.
- A running tracker logs one line at start (`OpenMP thread limit for OCR`,
  with `omp_thread_limit` and `libgomp_thread_limit`), its thread count
  after the first scoreboard OCR, and `omp_thread_limit` and `os_threads`
  on each `capture timing` line. Under the unit these lines are in the
  journal: `journalctl --user -u scuffed-stat-tracker`.
  `systemctl --user show -p Environment scuffed-stat-tracker` shows what
  the unit sets.
- These lines are not in a problem report from a standard install. A
  report's log is cut from `daemon.log`, which only a tracker the app
  started itself (no unit installed) writes.

### Where the daemon writes

The user unit sets `ProtectSystem=strict`. When that sandbox is applied,
only these paths are writable:

| Path | What lands there |
|---|---|
| `~/.local/share/scuffed-stat-tracker` | Default data dir. Tessdata always lives at `tessdata/` here, even when `data_dir` is custom |
| `~/.config/scuffed-stat-tracker` | `config.toml` (mode 0600) and `session.env` |
| `$XDG_RUNTIME_DIR` (`%t`) | Wayland, X11, D-Bus, and PipeWire sockets |

`{data_dir}` holds `stats.surrealkv`, `commands/`, `debug/` PNGs, portraits,
`daemon.pid`, `sync_auth.json`, `live_snapshot.json`, `tracker.sock` (the app's query socket while the tracker runs), `active_game.json`,
`daemon.log`, and vacuum and hero-timeline backups (`stats.surrealkv.pre-*`). The default directory is already in the
table above. An absolute `data_dir` anywhere else, including another
folder under `$HOME`, is read-only under the sandbox. `install.sh` writes
`~/.config/systemd/user/scuffed-stat-tracker.service.d/data-dir.conf` for
that path. Reinstall after you change `data_dir`. If the directory is
still not writable, the daemon exits and prints the same drop-in. A
relative `data_dir` is not put in the unit.

With `reader = "new"` in config.toml, `{data_dir}` also holds
`private/own-name/seeds.json` (folder 0700, file 0600). The new reader does
not read player names to find your row. It keeps a few crops of your own
name as it is drawn on the scoreboard, and compares each row with them. A
crop is taken only from a board where your row is clearly highlighted, the
hero panel shows the same hero as that row, and no chat covers it. It is
trusted only after three boards from two games gave matching crops in one
run of the tracker.

When no crop matches and no row is highlighted, your row is the one row of
your team that shows the hero on the hero panel. A row found that way is
marked as uncertain until a crop confirms it. Its crop is learned too, but
it has to match on four boards from three games, on at least two different
heroes, in one run of the tracker. If you play one hero only and your row
is never highlighted, your row is still found this way on every scoreboard
where your hero icon can be read, and it stays marked. On a scoreboard
where it cannot be read (you are dead, or the icon is covered) and no crop
is trusted yet, the Tab is not stored.

Crops that keep pointing at another row than the one the hero panel's hero
is on are dropped and learned again.

Names are still read in one place while Tesseract is on. When no crop,
no highlight and no hero panel finds your row, the tracker reads the name
column of that one scoreboard and looks for your configured `player_name`,
as `reader = "ocr-v1"` does on every scoreboard. With `ocr = "off"` there
is no such read, and that Tab is not stored.

With the new reader (the default since alpha.12), a scoreboard it finds
nothing on is read with ocr-v1 instead, for that one Tab, while Tesseract
is on. Those stats are saved and uploaded marked as read by `ocr-v1`. To
use the old reader for everything, choose it in Settings or set
`reader = "ocr-v1"` in `config.toml`.

Only trusted crops are written to the file: crops confirmed as your own
name line. A crop that is still waiting for confirmation could be another
player's name, so it is kept in memory only and is gone when the tracker
stops. If you restart the tracker between games the count starts again,
and until your name is trusted your row is found by its highlight or by
the hero panel alone.
The file holds pixels and a hash of the configured `player_name`, never
the name. It is cleared when another name is configured, which the tracker
notices when it next starts with the new reader (`reader = "new"`, the
default when there is no `reader` line); a start where the name
is not known (not set here and not fetched) keeps it. It is removed on
uninstall, and never put in a report, an upload or the log.
`reader = "ocr-v1"` does not create it and does not read it;
after a switch back to `ocr-v1` an existing file stays, unused, until
uninstall or until you delete it.

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
one process can hold it, the daemon writes a small `live_snapshot.json` (the
newest tracked row of each game, every session, pending deletes) while a GUI
is running, after each finished game, at clean shutdown and at startup. The
GUI reads that file and sends manual edits through a file command queue
(`{data_dir}/commands/`). `matches.jsonl` is a record of captures, appended
per capture and rewritten only for rare corrections; the app does not read
it.

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
2. It imports `WAYLAND_DISPLAY`, `DISPLAY`, `XDG_CURRENT_DESKTOP`,
   `XDG_SESSION_TYPE`, and `KDE_SESSION_VERSION` (Plasma only) into the
   user manager when `systemctl --user` works.
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
  DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE KDE_SESSION_VERSION` and
  restart the daemon.
- Do not install with `sudo`. The helper only reads processes of the
  installing user. `session.env` is always `~/.config/scuffed-stat-tracker/`
  (the unit's `%h`), not `$XDG_CONFIG_HOME`.

**Games play but nothing is recorded (no `parsed scoreboard` lines in `daemon.log`).**
The daemon reads Tab presses straight from `/dev/input`. A global-hotkey daemon
that grabs keyboards exclusively (e.g. GPU Screen Recorder's `gsr-global-hotkeys
--all`) silently starves it: the kernel delivers events only to the grabber. The
daemon logs `keyboard is exclusively grabbed by another process` at WARN and
picks up the grabber's virtual pass-through keyboard automatically when it
appears (hotplug). If the WARN says *every* keyboard is grabbed, switch the
hotkey tool to its no-grab / virtual-devices mode or restart the tracker after
it. Games that were closed without a stored row are listed in
`debug/unrecorded_games.jsonl`, with the reason: no Tab was captured for the
game, or (with `reader = "new"`) its Tabs never gave a number for every stat.
In the second case the line names the stats: `never_read` had no sure read on
any Tab, `read_not_taken` had a read that was not taken because other stats
had dropped on that Tab. Such a game is stored nowhere and is not uploaded;
today it shows only in this file and in the journal, not in the app.

## Dev tools

`examples/` contains the diagnosis workflow, each file documents its usage:
`extract` (full pipeline against a still image), `polltick` (poll-tick CPU
cost), `probe_outcome`, `accolade`, `profile`, `dumpdb`. Fixture replay tests
(`tests/`, `#[ignore]`d) validate outcome detection against real frames in
`tests/fixtures/outcomes/`; scoreboard replays expect (uncommitted) screenshots
in `tests/fixtures/replays/`.
