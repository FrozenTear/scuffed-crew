# Stat Tracker changelog

User-facing notes for `stat-tracker-v*` GitHub Releases. The release workflow
prepends the section whose heading matches the tag version (for example
`## 0.4.15` for `stat-tracker-v0.4.15`).

## 0.4.22

A bare or ambiguous Watchpoint is not stored as Gibraltar. A following
word is matched against Grímsvötn and Gibraltar, including the misreads
the English OCR model produces when í and ö are missing. Turkish İ
and dotless ı fold to i, so GRİMSVÖTN is Grímsvötn. If neither
name wins, the read is dropped.

The map vote reader treats one Watchpoint prefix beside Grímsvötn as
that card. A second Watchpoint card is Gibraltar. A lone Watchpoint
is not a candidate. The same Grímsvötn card, read twice, does not add
a phantom Gibraltar.

Adlersbrunn is the Junkenstein event map. It is stored as Eichenwalde
when both teams have stat rows. The both-teams gate excludes the PvE
board, including a one-letter misread of the alias. Literal
Eichenwalde is unchanged.

Château Guillard is Deathmatch. A trusted read (the top bar, an
accolade, or a session that is already Deathmatch) is kept in the
local store and never uploaded. A fuzzy board read does not rename an
open game. The desktop totals and history skip those rows, including
a map corrected to Château Guillard.

The 20-minute timer no longer closes an unfinished game, so a long
match can still take its result. After 6 hours with no result, that
timer closes it. An unfinished game is also closed at the next
new-game boundary, on the next start once the saved session is older
than 20 minutes, and when a suspend gap is longer than 20 minutes.
A clean shutdown keeps the skeleton, so a restart inside 20 minutes
resumes the same session. A finished game still closes and uploads
on shutdown. A held fresh-match board is not written onto that
finished game. It stays with the next session.

The server cannot store an outcome-less game, so the tracker does not
send those rows. On a close it marks that session's unknown rows
synced locally and makes no request. Other unknown rows stay
unsynced. Games > card > Victory, Defeat, or Draw requeues the row
with edited set. The command tick schedules that upload within about
3 seconds, including while the session is still open.

Uploads from startup, resume, the quiet-close timer, and SetOutcome
do not block Tab, polling, or shutdown signals. A held board on a
finished game is carried onto the next session when that game closes
on the quiet timer or a new-game boundary. `game_mode` follows the
stored map, including a map correction and the mode sent on upload.

Training text includes an ASCII Grimsvotn line, plus Adlersbrunn and
Chateau Guillard on their own lines. Those three names are repeated,
and the font fine-tune runs 4000 iterations, so a line crop of each
name still reads back.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.22`.

## 0.4.20

The last game of the night now uploads by itself about 3 minutes after
the result screen. Before this, that game stayed open until the next
Tab or a clean shutdown, so it could sit unsynced for hours. A Tab
still belongs to the finished game for 75 seconds after the result.
After that, if nothing new is recorded for about 3 minutes, the daemon
closes the game and uploads it. Those 3 minutes start from the last
activity: a stored capture, a recorded outcome, an accolade map, or
the session opening. They are never shorter than the 75-second grace.
A game that never got a result, and then sits idle for 20 minutes, is
closed too, with outcome Unknown. That close uploads nothing, because
an Unknown game is not sent. 0.4.22 stops closing unfinished games on
this timer. Restarting the daemon no longer drops a stale open game
without uploading its rows.

Watchpoint: Grímsvötn is an Escort map. The tracker stores that name
exactly, including í and ö. The name grimsvotn on its own, with or
without accents and with or without the Watchpoint prefix, is that
map. A bare Watchpoint is Gibraltar, and so is the name gibraltar on
its own. 0.4.22 stops trusting a bare or ambiguous Watchpoint. When
the Grímsvötn name appears in the same text as that prefix, Grímsvötn
wins. A stored row's game mode is filled from the map name read in
the scoreboard text. 0.4.22 fills it from the map that was stored.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.20`.

## 0.4.19

Boundaries are one board-order state machine. A result hint stays sealable
until a second board with progressed stats is accepted after it, or one
progressed board after a hero select has armed a boundary, or until a
reset, a gap, a Tab that names a different map, an end screen whose map
and the session map are both trusted and differ, or an unblocked map vote
or hero ban seals it. The first progressed board keeps the hint.
Progressed means a counter moved forward from the reset baseline: not
the same totals, not a decided result header, and not an implausible
jump. There is no 60-second hint timer. The 75-second grace starts
when the result is recorded, not when the word was first seen.

Wall-clock time is the 75-second grace, the 120-second stat gap, the
45-second wait before the first fresh-match board counts, the 20-minute
bound on an unfinished session, the map-vote debounce, and the rate
ceilings: elims, assists, and deaths `elapsed / 5 + 8`, and damage,
healing, and mitigation `elapsed * 80 + 2500`. The rate clock is the
reset baseline, not the last stored row. The 2x–4x all-increase band
applies within 60s of the baseline.

A map vote that is not blocked, or a hero ban, after a hint that already
has a board splits and seals the hint. A ban with no held board splits
at once. A ban after a held fresh-match board does not. A hero select
after that board only arms a pending boundary, and a select after a held
fresh-match board does not split. The next fresh-match board then splits
and seals. A hero select when no board has been stored yet seals
immediately, except on the session a start screen just opened: a swap
before that session's first Tab stays, including after the debounce.
The session a vote opens keeps that vote's candidates and its first Tab.
That is the trade-off: a vote session with no board and no result word
absorbs a following select-only game until the 20-minute idle bound, and
its old candidates veto that game's top-bar reads until an accolade fills
the map in. A hero select during a live match primes a reset and does not
split. A hero ban, or a map vote past the debounce, closes an unfinished
session without sealing a result. The same-map guard covers votes. The
same-map-plus-hero guard still suppresses a gap split of an unfinished
match.

A fresh-match reset is the same identified row, with clean elims, deaths,
and damage, at or under elims max(2, previous/4), deaths max(1,
previous/4), and damage previous/4, versus a mature board, and the first
is at least 45 seconds later. The first is held off the current session
and written onto the new one, at that capture's own time, when a second
fresh board commits. That carried board is stored on the first Tab that
writes it, and not again on later Tabs. A hero select or ban before that
board is the other signal. A select after the held board does not split
by itself. A ban after that held board does not split either.
An unidentified or implausible row is stored and leaves the held board
and the streak. A plausible continuation, or counted progress, clears
them. A hero
change by itself is not a split. A different row never counts as a reset.
A row with no id never counts. The new session's outcome is Unknown
unless the split is the 120-second gap or a different-map Tab, which
keeps that frame's header. An end screen whose map and the session map
are both trusted and differ gives the new session that screen's result,
whether or not a boundary is armed. A reset or a gap after a hint seals
the hint on the old session. An implausible jump is still stored, and it
does not become the reset baseline. One misread stat cell is still held.
A long post-match screen does not split on the gap.

A hinted session whose next Tab names a different map splits and seals
the hint. The session has to already have a map. With no board of its
own, that map is the accolade read on the hint tick, and the first
different Tab is enough. Without that accolade there is nothing to
differ from, and the limitation is unchanged. A session that already
has a board waits out the 120-second gap, and only when the stored map
came from the top bar or the accolade. A full-board text fallback is
stored and does not split a later Tab. A late Tab of the same map does
not split. A read inside the gap does not split.

An unconfirmed word replaces the hint when its map matches the session,
or when the boundary is not armed and either side has no trusted name.
A different map never replaces the hint. While a hero select has armed
a boundary, a word where either side has no trusted name does not
replace the hint either. The cost of that armed rule: a real result
read once with no map is dropped, and the earlier false hint is sealed
by the next vote. The unarmed exception is the other cost of still
taking a rank screen or an end title that prints no map: a mapless
misread can replace the hint, and a second mapless read then seals it
onto this session.

The second agreeing read opens a new session when both the session map
and the word's map are trusted (top bar or accolade) and they differ,
whether or not a boundary is armed. The old hint stays on this session.
The new session takes the new word and that map. When this tick has no
map of its own, it uses the map from the previous read of the same
outcome. That map is stored again on every read, so reads under 60
seconds apart can pass it along, and a read exactly 60 seconds later
still carries it. A carried map older than that window is dropped. The
late read still counts as a new unconfirmed word. This is harmless for
the confirm itself, because the second read already agrees; the carried
name is what the split uses when this tick has none. If either side has
no trusted name, the confirming read seals onto the open session. A
banner has no map, so a banner-only confirmation still seals onto the
open session.

Two mapless reads confirm each other and seal onto the open session.
The same happens when a Tab capture is in flight: the cheap outcome
poll skips the accolade crop, so those ticks have no map and cannot
open the next session. The cost is the next game's result landing on
this one. It happens when the accolade screen is not read, when a
carried map is older than the confirm window, or when the only
confirming ticks fall during that Tab.

A trusted top bar and a trusted accolade that disagree inside one
match open a new session and seal this session's hint. The cost is
this match's real result leaving with that split when the two reads
simply disagree, before the player has queued again. That close is
logged as an end-screen map, counted apart from a stat regression.
The log also records whether a boundary was armed, the sealed hint, and
the accolade map. Those fields separate an armed split, or a split that
sealed a hint, from the other end-screen closes. An unarmed close that
sealed nothing logs the same shape for a disagreement and for a requeue.
A gap and a hinted Tab stay a stat regression.

A full-board text fallback is not a map for a different-map split. An
accolade replaces that name only while no boundary signal has been seen
yet: no arm, no start screen, no primed or held reset, the outcome not
yet recorded, and no hint of a different result. After any of those
signals, the accolade must not rewrite the map or its snapshots. The
tracker cannot tell that the accolade is this session's own end screen
beyond those signals. A hint that matches the next result, or a session
with no hint, can still take the next game's accolade. Replacing a
hint of a different result sets a lock that closes the window, so the
confirming read cannot relabel the map. A misread end title that sets
that lock, and an arm, keep the text name. The tick that reads this
session's own accolade records the result then, so the hint is not
cleared and the text name stays. The text name changes only when
progressed boards clear the hint before that end screen. Those boards
also clear the lock. A restart that cannot restore the hint's timestamp
drops the hint and keeps the lock. A carry older than the confirm
window is dropped, so it cannot relabel the session. A later top bar
that names the same map still upgrades the source. A skeleton written
before 0.4.19 has no source; a map on that file is untrusted text.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.19`.

## 0.4.18

Season 5 ([patch notes](https://overwatch.blizzard.com/en-us/news/patch-notes/)):
new captures recognize the Support hero Doctrine, and read Sombra as
Support. Games already stored keep the role they were captured with.
Roadhog's rework does not change his role.

Doctrine ships with stand-in wiki art (Blizzard Entertainment artwork,
sourced via the Overwatch wiki). The tracker replaces that file
automatically with a real in-game crop the first time you play Doctrine.

`--collect-portraits` only fills missing portraits and the Doctrine
stand-in; it never overwrites an existing reference.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.18`.

## 0.4.17

The user systemd unit now refuses new privileges and mounts the
filesystem read-only (`ProtectSystem=strict`) except the default data
dir, the config dir, and the session runtime dir. `/dev/input` stays
available for hotkeys. A custom `data_dir` outside those three paths
gets a drop-in (`scuffed-stat-tracker.service.d/data-dir.conf`) at
install time. If that directory is still not writable, the daemon
stops and prints the drop-in to add. Reinstall after changing
`data_dir`.

Match data, the command queue, and config are owner-only (directories
0700, files 0600). The daemon only treats a pid as its own when the
process image is `scuffed-stat-tracker`. A rejected sync token (HTTP
401 or 403) pauses sync until the URL or token changes. The Overview
title-row dot follows capture state.

Hotkeys need the `input` group. Log out of the desktop and back in
after `usermod`; a new terminal is not enough.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.17`.

## 0.4.16

Game times in the desktop app now show in your local time zone
(#138). Before, the header "Last game", game cards, the Games list and
the companion overlay showed UTC, so a game that ended at 00:15 in
Oslo read as 22:15. Stored data is unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.16`.

## 0.4.15

Overview header (#122): filter chips vs companion switch vs plain
status (L25).

Updater and Settings install command are pinned to the release tag
(#125). Optional minisign verification. A failed install leaves the
daemon stopped. Binary replace is atomic. Tar path and symlink
hardening (M19).

Sync token is refused over non-loopback HTTP (#126). Sync backs off.
429 and 503 honor Retry-After (M20).

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.15`.

## 0.4.14

The systemd user unit now gets a display session (#109). Install
refreshes Wayland or X11 variables into
`~/.config/scuffed-stat-tracker/session.env` on each start, so Sway
and Hyprland autostart can capture. Reinstall so the unit points at
the installed binary.

Startup with no keyboard (#111) listens for stop while it retries.
`systemctl --user stop` finishes cleanly. If no keyboard shows up,
the daemon exits with an error so systemd can try again after you
join the `input` group or log in again.

GUI stop (#113) waits until the daemon has actually exited before
another one starts, and it does not delete a newer daemon's pid
file. A second process cannot open the local stats store at the
same time.

Sync (#114) does not mark a match uploaded if a newer local write
landed while the upload was in flight. Shutdown waits for that
upload to finish before the next one, so a late mark cannot
overwrite a newer result. Older local rows still load.

0.4.13 shipped mode buckets for Neon Junction, Paraiso, and
Esperanca (#91). Still on prior polish / packaging from
0.4.1–0.4.13.

Maps buckets, in-app Update now (0.4.8), Settings Maps-level polish
(0.4.7), Maps-grammar Seasons grid (0.4.6), Settings/Maps/Games
polish (0.4.5), companion overlay hotkey (0.4.4), and packaging
hotfixes 0.4.1–0.4.3 are unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.14`.

## 0.4.13

Bucket Neon Junction, Paraiso, and Esperanca by game mode so Maps
leave OTHER (#91). Neon Junction classifies as Hybrid. Paraiso and
Esperanca accept both plain and accented live names (Paraíso /
Esperança) and classify as Hybrid / Push.

0.4.12 shipped cheap outcome-only poll during end_reel_wake (#87).
Still on prior polish / packaging from 0.4.1–0.4.12.

Daemon OCR / capture / sync / store schema are unchanged. In-app
Update now (0.4.8), Settings Maps-level polish (0.4.7), Maps-grammar
Seasons grid (0.4.6), Settings/Maps/Games polish (0.4.5), companion
overlay hotkey (0.4.4), and packaging hotfixes 0.4.1–0.4.3 are
unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.13`.

## 0.4.12

Cheap outcome-only poll during `end_reel_wake` even when Tab
scoreboard OCR is in flight (#87). Mid-match Tab-busy poll skip is
unchanged. Confirm and streak rules are unchanged. Ban Heroes still
does not wake end-reel. Tab-reject Victory adopt is out of scope
(follow-up).

0.4.11 shipped end-reel / POTG false-wake tighten (#85). Still on
prior polish / packaging from 0.4.1–0.4.11.

Daemon OCR / capture / sync / store schema are unchanged. In-app
Update now (0.4.8), Settings Maps-level polish (0.4.7), Maps-grammar
Seasons grid (0.4.6), Settings/Maps/Games polish (0.4.5), companion
overlay hotkey (0.4.4), and packaging hotfixes 0.4.1–0.4.3 are
unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.12`.

## 0.4.11

Tighten end-reel / POTG wake false positives (#85): reject
`ENTERING GAME` loading letterbox and Tab scoreboard nameplate
false wakes. Ban Heroes is a distinct `detect_ban_screen` signal
(not an end-reel wake; future: register bans). Real nameplate POTG
and cinematic reel still wake.

0.4.10 shipped nameplate POTG wake (#83). Confirm rules are
unchanged.

Daemon OCR / capture / sync / store schema are unchanged. In-app
Update now (0.4.8), Settings Maps-level polish (0.4.7), Maps-grammar
Seasons grid (0.4.6), Settings/Maps/Games polish (0.4.5), companion
overlay hotkey (0.4.4), and packaging hotfixes 0.4.1–0.4.3 are
unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.11`.

## 0.4.10

POTG nameplate title-card wake (#83): a Play of the Game **nameplate**
title card (gold battletag + white title, color gate) holds full ~4s
poll cadence for 45s. Letterbox / highlight end-reel path is unchanged.
Phrase OCR is optional and log-only — the cheap color gate wakes.
Calibrated on a real 2560×1440 card.

0.4.9 already shipped on-hit poll debug PNGs (#80) and POTG / end-reel
letterbox wake (#81). Confirm rules are unchanged.

Daemon OCR / capture / sync / store schema are unchanged. In-app
Update now (0.4.8), Settings Maps-level polish (0.4.7), Maps-grammar
Seasons grid (0.4.6), Settings/Maps/Games polish (0.4.5), companion
overlay hotkey (0.4.4), and packaging hotfixes 0.4.1–0.4.3 are
unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.10`.

## 0.4.9

On-hit poll debug PNGs (#80): when debug OCR is on (Settings / config
or `STAT_TRACKER_DEBUG_OCR=1`), the normal poll path saves
Victory/Defeat/Draw frames under `debug/poll/` on confirm and on the
first word-OCR streak. Mid-match ticks stay silent.

POTG / end-reel slow-mode wake (#81): a Play of the Game or highlight
end-reel sighting holds full ~4s poll cadence for 45s so the short
Victory/Defeat window is sampled. Confirm rules are unchanged.

Daemon OCR / capture / sync / store schema are unchanged. In-app
Update now (0.4.8), Settings Maps-level polish (0.4.7), Maps-grammar
Seasons grid (0.4.6), Settings/Maps/Games polish (0.4.5), companion
overlay hotkey (0.4.4), and packaging hotfixes 0.4.1–0.4.3 are
unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.9`.

## 0.4.8

In-app Update now (#78): preflight then bootstrap with pinned
`STAT_TRACKER_TAG`; clear blocked reasons.

Copy command for the install curl (#78): Wayland prefers `wl-copy`;
toast if wl-clipboard is missing.

Daemon OCR / capture / sync / store schema are unchanged. Settings
Maps-level polish (0.4.7), Maps-grammar Seasons grid (0.4.6),
Settings/Maps/Games polish (0.4.5), companion overlay hotkey (0.4.4),
and packaging hotfixes 0.4.1–0.4.3 are unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.8`.

## 0.4.7

Settings Maps-level polish (#76): two-column masonry, Stored data spanning
pane, full-width Save footer, denser surface cards. Companion hotkey
setting is unchanged.

Daemon OCR / capture / sync / store schema are unchanged. Packaging
hotfixes 0.4.1–0.4.3, companion overlay hotkey (0.4.4), Settings/Maps/Games
polish (0.4.5), and Maps-grammar Seasons grid (0.4.6) are unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.7`.

## 0.4.6

Seasons Maps-grammar grid (#74): season cards use the same 2–4 column
layout as Maps, with a big WR, win/loss stripe, and FillPortion bar.

Settings denser cards (#74): surface cards with a 1–2 column field grid.
Companion hotkey setting is unchanged.

Daemon OCR / capture / sync / store schema are unchanged. Packaging
hotfixes 0.4.1–0.4.3, companion overlay hotkey (0.4.4), and prior
Settings/Maps/Games polish (0.4.5) are unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.6`.

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
