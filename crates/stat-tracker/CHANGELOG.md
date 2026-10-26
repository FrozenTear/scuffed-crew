# Stat Tracker changelog

User-facing notes for `stat-tracker-v*` GitHub Releases. The release workflow
prepends the section whose heading matches the tag version (for example
`## 0.4.15` for `stat-tracker-v0.4.15`).

Each release section starts with a short summary for players: one to three
sentences in plain language, before any technical detail. That summary is
the first paragraph under the `## X.Y.Z` heading (a blank line ends it).
After the summary, a `### Highlights` list gives two to four short
player-facing bullets. Each bullet adds a concrete change the summary
does not already say. The rest of the section is technical detail.
Keep the `## X.Y.Z` headings and the `### Install` blocks. The release
workflow copies from the matching heading through the next `##` heading,
including `### Install`. The desktop app hides `### Install`, shows the
summary, then the highlights, and tucks the remaining text under Details.

## 0.5.0

0.5.0 is the first stable release since 0.4.24 and brings together everything from the 0.5.0 alphas and release candidates. The tracker now reads scoreboards with its own reader by default, with player names from the downloaded `names-v1` pack, and reads a board with classic OCR when its own reader finds nothing. Screen capture works on KDE Plasma and GNOME on Wayland, and you can report a game the tracker got wrong straight from the app.

### Highlights

- On KDE Plasma the screen is shared once with the cursor hidden, the stream pauses between grabs, and Plasma 6.8 or newer can use KDE direct with no share dialog. With two monitors on KDE you pick which one the tracker watches.
- The result comes only from the VICTORY or DEFEAT word on the end screens, a Tab is timed from the moment you press it, and a game you crash out of and rejoin stays one game.
- Sync sends a big backlog in smaller batches, a game the site refuses is set aside with a Retry button while the rest still upload, and a game you delete in the app is removed from the site right away.
- The first launch opens a setup guide. An update installed from the app switches over after the current game is saved, and AUR or other package installs are told to update with the package manager.

The tracker code in 0.5.0 is the same as in 0.5.0-rc.9; only the version number changed. The notes for each step are in the 0.5.0-alpha.1 to 0.5.0-rc.9 sections below. If you stayed on the stable channel, this is the first 0.5.0 build the app and bootstrap.sh offer you, and the new reader, the name pack and the capture changes all arrive with it.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.5.0/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v0.5.0 bash
```

Or extract the tarball and run `./install.sh`.

## 0.5.0-rc.9

A red flash on screen during a match, such as Vendetta's effects, could be taken for the DEFEAT screen, and the game was then split in two. The tracker no longer reads a win or loss from screen colour at all: it reads the result word on the end screens, and a Tab that shows the same game still going on keeps it as one game.

### Highlights

- The result comes from the VICTORY or DEFEAT word on the end title, the accolade screen or the rank screen. If you skip all of them, set Win or Loss by hand in the app, as before when no result was read.
- If a game was given a result while it was still being played, and a Tab pressed after that result showed the board still going, the next Tab on the same map whose numbers have gone on rising keeps the game open and clears that result, so the real one is taken when the game ends. A result you set by hand is never cleared this way, also not one set while that Tab is being read.
- A Tab whose second look at the scoreboard fails no longer throws away the board and result the first look had.

### Technical detail

- Colour banner removed (R9-1): `detect_banner` (red or gold covering more than 35% of the middle band, 30 to 70% of the 16:9 frame) is gone, and so is `OutcomeSource::Banner`, which confirmed a result from one frame on the poll (`resolve_word_tick`, `poll_debug_hit`) and supplied a Tab frame's outcome. On 2026-10-22 one poll frame of a Vendetta game was read as Defeat this way, the first Tab after the 75 s grace window opened a new session ("superseded by Tab-opened game") for a board still at 9/8 then 10/9 on the same map, and the real Victory came later from the end title and accolade reads. The poller now confirms a result only on two agreeing word reads inside the 60 s confirm window, from the end title, the accolade screen or the rank screen. A Tab frame still takes its outcome from one read of its own frame, as before: the accolade or rank-screen word (only when the session is old enough), the end title, or the scoreboard OCR header line under ocr-v1. `OutcomeSource` was never stored, uploaded or written into a report kit (only its debug name appears in log lines), so the variant was removed rather than kept. The Tab frame's accolade and rank-screen word reads keep their 5-minute session age gate (`MIN_BANNER_SESSION_AGE`, name kept).
- Same game past the grace window (R9-2): a Tab more than 75 s after a finished game's result used to open a new session at once. When the finished game has a stored row, a top-bar or accolade map, a result the tracker read itself (not one set in the app, and not one restored from a previous run), no held fresh-match board and no rejoin carry, and its last accepted board is at most 120 s old, the Tab's board now decides. The 120 s run from the last accepted board, not from the result, so how often you press Tab decides whether a game is covered. The board is compared when the Tab's top-bar map names the game's map, the own row is identified and not suspect, elims, deaths and damage have a number on both boards, and the last board is mature (elims at least 4, deaths at least 2 and damage at least 1500, so a short quick-play backfill is never continued). It must have no column below the last board except at most one by an OCR-sized step (one count of elims, assists or deaths, or a tenth of damage, healing or mitigation), and growth under the rate ceiling. If two or more columns moved on and Tab was pressed for the last board after the result was recorded (the press counts, not when its report came back: a result can be confirmed while a capture runs), the game was still being played: the board is stored on that game, and its result is cleared on the game and in the store (rows and session go back to `unknown`) together with the result words read for it, so `reader = "new"` does not stamp the old result on later rows. The frame's own outcome is not taken on that Tab. The next confirmed result word sets the result; a word streak from at or before the cleared result is dropped, so a repeated misread needs two new reads. Words read while the false result was held were ignored and are not replayed, so an end screen already past is not read again, and the game then ends without a result unless you set one. If Tab was pressed for the last board before the result, a rise is the last fight before a real end, so it is not taken as evidence: the new session opens and the result stays. If nothing moved on, or only one column, it is a late look at the same board: the game keeps its result, and the board is not stored and does not restart the 120 s. Anything else (another map, no map, a board back near zero such as the next game on the same map, two drops, a missed or rejected frame, a failed capture) opens the new session as before, as of the Tab press, and the closed game is uploaded at once. The new session opens when the Tab's report lands instead of at the press, and only if the finished game is still the open one or no game is open. Known limit: rows already uploaded with the false result keep it on the site, because `unknown` rows are not uploaded, until a result is set on that game again. Logs: `Tab past the post-match grace window: the board decides whether this is the same game`, `Tab past the grace window: board compared with the finished game` (with the verdict), and a WARN `the board moved on after the result was recorded: the game is still being played, result cleared`.
- Failed retry grab (R9-3): when a Tab retry's screen grab fails with an error other than the portal and stream cases already handled, the retries end and the best earlier grab is kept (for example a board the reader refused, with the VICTORY or DEFEAT on its frame), with a WARN `Tab capture retry failed (screen grab or frame analysis): keeping the earlier result`, and no second "gave up after retries" line. The same holds when the analysis of a retried frame fails. Before, that error failed the whole capture and dropped the earlier grab. A failed first grab still fails the capture.
- Tests (R9-4): the missed-capture report for a Tab turned away by a paused or unanswering screenshot portal; a refused board with no outcome outranking an unsure team size that read VICTORY (the refused board is kept, as `tab_outcome_rank` says; its doc now says the unsure frame's outcome is lost then, and an outcome-aware tie-break is left for 0.6); and in the app, the doc comment of `tracker_note` is on `tracker_note` again.

## 0.5.0-rc.8

If the screenshot portal crashes, or stops answering once screenshots have been allowed, the Older screenshot capture method no longer stops all capture until the tracker restarts. A Tab whose scoreboard the new reader refuses is now tried again while you hold Tab. Installs from the AUR or another package are told to update with the package manager, and an update started from the app keeps going if the app quits part way.

### Highlights

- With an NVIDIA card, KDE direct now needs Plasma 6.8.1 or newer, because an older KWin can crash when a screenshot meets any app's screen share, such as Discord or OBS. On Plasma 6.8.0 with NVIDIA the tracker uses the automatic method instead, and Settings says why.
- On an X11 session, Settings offers Automatic and Older screenshot. A saved Screen stream or KDE direct is shown with a warning, and the tracker uses Automatic instead. There is no screen stream to fall back to on X11, so if X11 capture fails there, the app now says that nothing can capture.
- If an Older screenshot request is waiting on a screenshot permission prompt, the app now says a prompt may be waiting, possibly behind the game.
- An in-app update now installs the libraries before the programs, so one cut off part way never leaves new programs beside old libraries. If the app quits during the update, the install still finishes, and the tracker is started again when that update had stopped it and autostart is on. From now on, if you stop the tracker yourself before an in-app update, it stays stopped.

### Technical detail

- Older screenshot (#440): a request is now a direct `org.freedesktop.portal.Screenshot` call on a session bus connection of its own, with the same options (not interactive, not modal). ashpd's request could not be closed before it answered. Every 60 s without an answer the tracker looks again, and an answer that arrives during that look is still used. If `org.freedesktop.portal.Desktop` now has no owner, or a different one from the portal that took the request (a crash or restart), the request is given up. If the permission store says `yes` to screenshots for the tracker (app id `""`), or has no screenshot entry while a request of this tracker has already got a picture, no dialog can be what it waits for, so the request is given up and ordinary polls make no new portal request for a pause: 30 s after the first stall, then 2, 5 and 10 minutes after each later one (#442). Ordinary polls are also what wake the end-of-match result check, so during a pause the result screens are not read either, except by a check that was woken before the pause while a Tab capture runs. Once the given-up request's Close has been answered, a Tab may try during a pause. While that Close is unanswered, no request is made at all, Tab included, for the whole pause and for at least 2 minutes after the give-up, so two of the tracker's requests are never open at once in that time. An answered request ends the pause and starts the steps again at 30 s, and polls turned away during a pause do not count as poll failures. A Tab turned away by a pause, or whose own request is given up, is now a missed capture logged at INFO ("Tab capture missed: the screenshot portal is not answering") instead of an ERROR, and is not retried (#443). Otherwise (the permission is `no` or `ask`, there is no entry and no picture yet, or the store cannot be read) a permission prompt may be up, so the request keeps waiting as in rc.7 and the app shows "A screenshot permission prompt may be waiting, possibly behind the game. Screenshots wait for it." That notice goes away if the request ends without an answer. If the portal's owner cannot be read, the request keeps waiting and nothing changes. A request that is given up is closed with `Request.Close` (bounded at 2 s) and its connection dropped, so a late answer reaches nothing. A file the portal writes for it later is left in place, because the KDE portal names files to the second and removing it could remove another request's picture. After a stall, the app shows "No answer from the screenshot portal. It may have stopped; restarting the tracker or the portal can help." until a grab works on any route, a request is answered, or the route is decided again. These notices end with one period in the app, not two (#443). Only the wait for the answer is bounded: reading, decoding and cropping the file still run under the lock. Before, an unanswered request held the lock until the tracker restarted, so a portal that crashed mid-request stopped all capture.
- Refused Tab boards (#440): with the new reader, a Tab frame whose board the reader refuses (for example "board was not found") is now tried again while Tab is held, like a frame with no full scoreboard. Before, it counted as accepted, so a press queued behind it was dropped and the 3 s debounce started. It ranks below a board the reader takes and above every other result, including a frame whose team size is unsure, so a VICTORY or DEFEAT seen on the first Tab frame is kept when a retry is unclear, as in rc.7 (#443). It logs `Tab frame refused by the scoreboard reader` with the reason. Without the new reader (ocr-v1), or for a board read with the OCR fallback, nothing changes. The debounce and a queued press now follow what the capture did with the board: a board that was recorded, held as a fresh-match board, or kept pending starts the 3 s debounce and drops a press queued behind it; otherwise the queued press runs. A press while a capture reads a board the reader passed is now queued instead of ignored. A refused board can cost one more full read while Tab is held on Older screenshot, and two on other routes. A tap released less than 1 s after the press whose board the reader refused now logs its own hint (#442), "the scoreboard reader refused the board and Tab was released before another grab: hold Tab for about a second", at WARN at most every 10 minutes and at INFO in between, like the hint for a frame with no scoreboard.
- KDE direct on NVIDIA (#437): with the NVIDIA kernel driver loaded (`/sys/module/nvidia` or `/sys/module/nvidia_drm`), KDE direct is unavailable on a Plasma known to be older than 6.8.1, whether or not a screen share is open, because KWin there can crash when a screenshot meets any app's screen share (KDE bug 524840). Nothing is rendered, and the tracker falls back as for any other unavailability, with one WARN and the Settings notice; the reason reads "it needs Plasma 6.8.1 or newer on NVIDIA (found 6.8.0, KDE bug 524840)". Plasma 6.0 to 6.7 already refuse KDE direct, so in practice this changes only Plasma 6.8.0 on NVIDIA. When the Plasma version cannot be read, the rc.7 rule applies (no render beside the tracker's own share), and on NVIDIA the log warns if KDE direct is found available. Without NVIDIA the rc.7 rule stands: nothing renders beside the tracker's own share unless Plasma 6.8.1 or later is known.
- X11 sessions (#437): the session is X11 only on positive evidence, `XDG_SESSION_TYPE=x11` with no `WAYLAND_DISPLAY`. Wayland is detected as before, and anything else (the type unset, `tty`, or `x11` beside a stray `WAYLAND_DISPLAY`) is unknown and keeps rc.7's behaviour. A failed X11 probe is now logged at WARN with its error. On a known X11 session, automatic no longer falls back to the desktop portal, so when X11 capture fails nothing captures and the app shows "X11 capture is not available (<error>); no capture method works on this session". A saved KDE direct or Screen stream is ignored there and not probed: automatic runs, and the app's notice names the saved method and what is used instead. A saved Older screenshot still stands on X11, and `STAT_TRACKER_CAPTURE` is still a deliberate override that fails closed.
- X11 in Settings (#439): on a session the app knows is X11 (the same rule), the capture method row shows Automatic and Older screenshot, and its help line says Automatic captures with X11 there. A saved Screen stream or KDE direct stays visible and selected, with "<method> works only on Wayland. This is an X11 session, so the tracker uses Automatic instead." Older screenshot carries the note "Older screenshot captures the whole desktop on X11 with more than one monitor, so Automatic is recommended." KDE direct notes are not shown there. Wayland and unknown sessions are unchanged.
- Poll failures (#437): a poll grab that fails while the game runs is now a WARN on the first failure of a run, then at most one every 5 minutes, with TRACE in between. Before, every failure was TRACE and said "game may not be running". A held capture lock, a share still opening (every game start on KDE and GNOME), a declined share, a share closed until the game runs, and a poll with no fresh stream frame are not failures. The 5-minute clock is not reset by a success, so grabs that fail and work in turn do not warn every tick. A run that warned logs `poll capture works again` at INFO when a grab works, and the run ends when the game stops.
- Logs (#437): `perf.log` starts only after the single-instance guard is held and after a `--vacuum` run has exited, so a refused second start no longer writes a `setup` line into the running tracker's `perf.log`, and a `--vacuum` run writes none. The `screencast frames received per second` lines divide by the window's real length, fraction included: 19 frames in 1.9 s now reads 10, not 19. The once-a-minute timing line is now `screencast timing (totals since the share opened): ...`, because its counts are totals since the share opened. The `perf.log` compositor CPU also counts cinnamon, muffin, xfwm4, marco, picom, compiz, budgie-wm, gamescope (and gamescope-wl) and cosmic-comp, and finds Xorg when it runs as `X`.
- Site requests (#437): every request through the tracker's sync client (uploads, daemon config) and its reader pack client (pack list and files) now sends `User-Agent: scuffed-stat-tracker/<version>`. The app downloads reader packs through that same client, so it sends it too. The app's other site requests (token check, device link, reports, seasons) do not yet.
- Release list (#438): `bootstrap.sh` reads the GitHub release list 100 at a time, up to 5 pages, and stops once it has a stable release with the tracker download or a page comes back short. Before, it read only the newest 20, and site releases share that list, so the stable tracker release could drop off it and a fresh install would get a prerelease, or fail with `STAT_TRACKER_CHANNEL=stable`. If a later page fails, the pages already read are used; if they hold no stable release, the default and stable channels stop with a message instead of installing a prerelease. If no list can be read at all, the error says so.
- Interrupted installs (#438): `bootstrap.sh` and `install.sh` ignore SIGPIPE and never fail on a log write, so an install keeps going when the app that started it quits (an install run by a 0.4.24 app died when that app quit). `install.sh` installs the bundled libraries before the binaries, so an install cut off part way never leaves a new tracker or app beside old libraries; old libraries it replaces are removed after the binaries. `bootstrap.sh` puts SIGPIPE back to the default before it runs a tag's own script, so older tags print no `Broken pipe` lines, and `install_path_named` now reads its whole list for the same reason.
- Restart after an update (#438): when it installs the service, `install.sh` starts it again only when all of these hold: autostart is on, the service is stopped (not failed), the new tracker passed its `--version` check, it stopped no earlier than 2 minutes before `bootstrap.sh` began (`STAT_TRACKER_BOOTSTRAP_STARTED`) and not later than now, and the graphical session is up. That covers the in-app updater's own stop when the app quit before it could start the tracker again. A plain `./install.sh` run never starts it, and neither does an update started from the app while the tracker was stopped: the app tells the installer the tracker was not running, so a tracker the player stopped stays stopped (#443). An update started from an older app does not tell it, so there the 2-minute window above still decides. The start does not wait (`--no-block`), and a failed start only warns.
- Release notes (#438): the notes for a stable tag now list the commits since the previous stable tag, not since the last rc. The `Update from the terminal` text now says to prefer the Update button in the app, and to use the terminal if it fails or does not offer the release, because an older app shows these notes inside its own Update offer.
- Package installs (#439): at start the app asks once, off the window's thread, which package owns the running app (`pacman -Qo` or `dpkg -S`, given up after 5 s). For a package install, the update banner and the Settings row say "Installed from a package: update with your package manager, for example `paru -Syu` or `yay -Syu`." (on apt, `sudo apt update && sudo apt upgrade`), with no install command, no Copy button and no Update now. Until the check answers, the banner and the Settings row show "Checking how this app was installed…" in place of the command, with Update now and Copy off. Other installs are unchanged. Before, a package install was offered the curl installer, which adds a second copy in `~/.local` whose service and desktop entry hide the package's.
- Leftover app (#439): when an app becomes the one running instance, it stops (SIGTERM) older copies of the app run by the same user that do not take part in the single-instance lock, such as a 0.4.x app left running after an in-app update, together with their overlays. Before, such an app kept running beside the new one, and saving its Settings wrote an older config over the 0.5 keys. It leaves alone itself and the app that started it, an app started less than 5 s before it (or after it), `--fixture` and `--replace` runs, an app that has the lock file open (even one deleted since) or whose open files cannot be read, an app whose start time cannot be read, an app in the middle of a 0.4.x in-app update (its update folder exists, or its installer or `systemctl` runs as its child), other users' processes and the tracker service. Each process is checked again just before the signal, so a reused pid is skipped.
- New reader label (#439): the Settings switch "New reader (alpha)" is now "New reader".
- Version pin test (#437): the README and `bootstrap.sh` must pin `TAG=stat-tracker-v<version>` as a whole word on a line, and this changelog must have an exact `## <version>` line, so a `0.5.0` pin no longer passes against a `0.5.0-rc.7` line.

## 0.5.0-rc.7

On KDE Plasma 6.1 or newer and on niri, with PipeWire 1.4 or newer, the screen stream now pauses between grabs, so the desktop stops drawing frames for the tracker while it is not reading one. Settings has a new capture method, KDE direct, for Plasma 6.8 or newer: it asks KDE for the screen directly, with no share dialog, no screenshot file and no cursor, and hands the tracker a picture of the same kind and size as a stream frame. Older screenshot does less work, and reports now show where capture time goes.

### Highlights

- Pausing is off on GNOME, on Plasma 5 and 6.0, on other desktops, when the KWin version cannot be read, and with PipeWire older than 1.4. If a paused stream does not come back when the tracker needs a frame, pausing turns itself off for that stream. To turn it off yourself, put `screencast_pause_between_grabs = false` near the top of `config.toml`, above any `[section]` line.
- KDE direct is off unless you choose it in Settings. If KDE does not allow it, the tracker uses the automatic method instead (the screen stream on Plasma Wayland) and Settings says why.
- Older screenshot takes one screenshot at a time, retries a missed Tab once instead of twice, and looks about every 3 seconds outside a match. The end-of-match result check keeps working while you hold Tab. On GNOME, the app and the log warn that GNOME flashes the screen and plays a sound on every capture.

### Technical detail

- Stream pause (#431): the stream is set inactive (`pw_stream_set_active(false)`) whenever no grab holds it. A poll that resumes it waits up to the stream's reuse wait plus 400 ms (750 ms at 4 fps); a buffer ends the wait sooner. A Tab press takes a hold at its key event, resumes the stream at the press plus 300 ms minus one frame period (50 ms at 4 fps), and keeps it running until its retries end. A Tab that preempts a poll takes the stream over before the poll's grab is dropped, so it does not pause in between. While Overwatch is not running nothing grabs, so an open share stays paused. Paused, quiet and never-streaming times count only while the tracker wants the stream running.
- Where it is on (#431): decided once at startup and logged as `screencast stream pause between grabs: <why>`. On for KWin 6.1 or newer and niri. Off for Plasma 5 (`KDE_SESSION_VERSION=5`), KWin 6.0 and older (KWin connects `outputChange` on every Streaming transition), a KWin version that cannot be read, GNOME (Mutter connects a `monitors-changed-internal` handler on every resume and never disconnects it), wlroots portals and unrecognised desktops. The KWin version comes from D-Bus `supportInformation`, read once more after 3 s on a Plasma 6 session, then from `kwin_wayland --version`. `import-session-env.sh` now carries `KDE_SESSION_VERSION`. Pausing needs the exported `pw_loop_invoke`, so PipeWire 1.4 or newer; an older libpipewire keeps the stream running.
- Pause fail-safe (#431, #434): a resume is answered by a frame. On niri, which draws a cast only on damage, reaching Streaming again also answers it. Pausing stops for one stream, which then runs as in rc.6 with one WARN, when resumes stay unanswered for 1.5 s of active time twice with no answer in between, a Tab misses while its resume is unanswered, a pause that began as the tracker's is still Paused 1.5 s of active time after the resume, the stream fails while a resume is unanswered or within 2 s of a Paused that answered the tracker's own pause, or the stream is closed while a resume is unanswered. Unanswered active time adds up across resumes: it starts at the first resume that goes unanswered and keeps counting through later short grabs, it is checked when a grab ends or the stream is paused, and after each count of 1.5 s the next one starts at the next resume. A lost PipeWire connection (Unconnected with the core's error message, caught in `on_state`) never counts for any of these, and neither does a closed portal session the tracker has already seen close when it checks; a stream that fails just before the tracker sees its portal session close can still count. A grab on a lost connection ends at once (#434). The WARN for a stream that ended just after a pause says it can also be an output or monitor change, or the compositor ending the share. A new share or reopen may try again; after two tripped streams pausing stays off until restart.
- Pause switch (#431): `screencast_pause_between_grabs = false` turns pausing off everywhere. `STAT_TRACKER_SCREENCAST_PAUSE=0` turns it off and `=1` forces it on on any desktop, for testing only (on GNOME, Plasma 5 and KWin 6.0 it brings back the leaks above, and on damage-driven portals such as xdg-desktop-portal-wlr a still screen may trip the fail-safe). The env var wins over the key and is not written back. With pausing off the tracker makes no `pw_stream_set_active` call and starts no timer, as in rc.6. The once-a-minute `screencast timing:` line gains `resumes=`, `avg_resume_ms=`, `max_resume_ms=` and `resumes_without_buffer=`. The pause decision, which on KDE can wait a few seconds for the KWin version, runs alongside the capture-route probe, and no stream opens before both finish (#434).
- KDE direct (#432): a new route, `CaptureBackend::Kwin`, chosen with `capture_route = "kwin"`, `STAT_TRACKER_CAPTURE=kwin` or `kde-direct`, or the Settings chip KDE direct (shown on KDE, or when it is the saved route). Each grab calls `org.kde.KWin.ScreenShot2` `CaptureScreen` for the chosen output (`CaptureActiveScreen` when the output layout cannot be read), with the cursor off and native resolution on. The pixels come over a pipe widened to 1 MiB, and the read must get exactly stride × height bytes and then EOF, and the call and read together must finish within 2 s. QImage formats 4 to 6 and 16 to 18 are converted to RGB8. The picture is never resized: a size more than 1 px off the cached output re-reads the geometry once (at most once a minute while a size keeps differing, #433), and is otherwise used as KWin rendered it. The screen stream is never opened on this route. A Tab waits up to 2 s for a running call and keeps two retries. The report manifest still names this route `portal`.
- KDE direct probe (#432): at startup the tracker renders a 1×1 `CaptureArea` at the top-left pixel of the output Auto would capture (`CaptureActiveScreen` when the layout is unknown). `Error.NoAuthorized`, or the empty reply Plasma 6.0 to 6.7 send, means unavailable and is kept for 60 s. A timeout, no bus, or KWin not yet on the bus at login is tried again on KDE after 1, 2, 4 and 8 s; each retry looks at the tracker's share again before it renders, and a D-Bus connect that does not finish within 2 s counts as a timeout (#433). If KDE direct is still unavailable, the tracker falls back to the automatic route with one WARN and a notice that Settings shows; it never switches route while running, and a restart tries again. Anything that renders beside the tracker's own screen share needs Plasma 6.8.1 or later, known for sure (KDE bug 524840): this gates the app's check and the `debug_portal_frames` compare half. The app renders its check only when KDE direct is the saved route or the chip is chosen; otherwise it reads the version only. That is one rule, which also follows the default-on constant (#433). `KWIN_DIRECT_DEFAULT_ON_KDE` stays `false`, so automatic still picks the stream on KDE.
- Older screenshot (#432, #433): every portal screenshot in the process runs under one lock. The KDE portal names its file to the second, so two requests in one second could share a file and one could delete the other's. A request the portal has not answered keeps the lock until it answers, with a warning at 20 s and every 60 s after. A poll that finds a request running is skipped, except the end-reel outcome-only poll, which waits up to 1 s for the lock while a Tab holds it (#433), so a held Tab no longer costs the ticks over the result banner. A poll turned away by a Tab does not restart the slow-cadence count, so the next tick polls again, and a poll's zero wait takes a free lock with `try_lock` instead of yielding first (#433). A Tab waits up to 4 s for a running request to end and then takes its own screenshot. If the request is still running after 4 s, the Tab does not take one beside it: it counts as busy, is retried while Tab is held and is ranked below any frame already grabbed. The PNG is read and decoded in `spawn_blocking`. Tab retries are capped at one on this route. Out of a match (no open or a finished game, no end-reel wake, no pending boundary), Older screenshot and KDE direct poll about every 3 s; mid-match slow mode is unchanged. The app's Capture now runs in its own process, so it and a daemon poll in the same second can still share one portal file, and one of them can fail; the README says to press Capture now again (#433).
- Output geometry (#432): the output layout is cached for 30 s (5 s when the read came back empty) and used by the screenshot crop and by the stream's `fit_stream`, including the automatic KDE stream. The cache is dropped and read again when it cannot explain a captured image. The monitor picker always reads fresh.
- GNOME (#432): forcing Older screenshot logs a startup WARN, and Settings shows "Older screenshot: GNOME flashes the screen and plays a sound on every capture."
- Compare frames (#432, #433): with `debug_portal_frames` on KDE at Plasma 6.8.1 or later, each of up to four Tab pairs also saves `NNNN-kwin.png` next to `NNNN-stream.png`. That half has to pass the same Plasma 6.8.1 gate as a fresh render beside a share, even when a probe made with no share open found KDE direct available (#433).
- Capture cost telemetry (#430), measurement only: the per-minute `sample` line in `perf.log` now fills `capture_cpu_ms` on every capture route (before, X11 only), measured as thread CPU around synchronous sections. On the screenshot portal it adds `portal_shots`, the round trip, PNG size, decode (including the file read), output-geometry and crop times (p50 and max). On the screen stream it adds `stream_delivered`, `stream_fps`, `stream_copied`, `stream_copy_ratio`, the negotiated or configured cap, and the last buffer's type, size, format and dimensions. Every line ends with the CPU % over the same window of the busiest compositor, Xwayland, the desktop portal, PipeWire, Xorg (X11 sessions only, whoever owns it) and the tracker's own GUI and overlay. Apart from Xorg, only processes owned by the user's real uid count. Each line covers only its own window. `perf.log` rotates at 512 KiB (was 256 KiB), about 20 hours across `perf.log` and `perf.log.1`. At shutdown the daemon waits up to 500 ms, or less when the shutdown deadline is nearer, for queued lines to reach `perf.log` (#433).
- Host line (#430): every report's `log.txt` starts with a `host` line, read without running any command: desktop and session type from the environment, and which DRM card drives a connected output, `nvidia_drm` modeset, and each card's driver and connectors from sysfs. It is scrubbed and capped at 2048 bytes with the card list last. The report manifest is unchanged, so deployed sites accept it.

## 0.5.0-rc.6

A Tab is now timed from the moment you press it and no longer waits behind the tracker's background check, and the screen stream runs at 4 frames a second, so the scoreboard is caught sooner. A result the tracker has already decided is no longer turned back into unknown by a late scoreboard read, a scoreboard read that finishes while the tracker is stopping is no longer lost, and the tracker tells the site about a game you delete in the app right away, instead of at the next game or restart. An in-app update that hangs no longer leaves the tracker stopped.

### Highlights

- A second Tab after a missed scoreboard is read instead of dropped, and the tracker stops retrying once you let go of Tab.
- The screen stream runs at 4 frames a second by default. To go back, put `screencast_max_fps = 2` near the top of `config.toml`, above any `[section]` line.
- A screen stream that stalls again later in the same game is reopened again, with a longer wait each time (1 minute, rising to 5). On niri and other wlroots desktops a stalled stream takes a direct screenshot instead, and no screenshot file is kept.
- A game that was uploaded and then turns out to be on a mode or map the tracker does not count is removed from the site too. A game that did not upload is retried on its own: about once a minute at first, slowing to every 5 minutes while the site stays down.
- If an in-app update hangs, it is stopped after an hour, the previous version is put back and the tracker is started again. Quitting from the tray during an update waits for it to finish. The app checks for updates again every 6 hours.
- On an AUR install, Install reading model puts back the model that came with the tracker instead of training one on your computer. Installs from the curl installer or an in-app update keep no copy of that model, so there it shows the command to reinstall. The Rebuild button is gone. The site sign-in step has a Copy code button, and Copy no longer freezes the window.

### Technical detail

- Tab timing (#422): the keyboard task sends each Tab press and release with the kernel's evdev time. evdev uses CLOCK_REALTIME, so the event's age is subtracted from `Instant::now()`. An age up to 1.5 s is used as is. An older or future event time means the wall clock stepped, and that press counts as now. Auto-repeat is not sent. The 400 ms overlay wait on a direct capture counts from the key event too. A Tab press preempts a running poll grab and its analysis. A release does not. A dropped poll leaves no state behind: the stability history is written back and the stream frame marked as seen only when a read completes. The next tick polls again.
- Tab retries (#422): a press while a capture without an accepted board runs cancels that capture's retries, ends a retry wait that is running, and is queued. It runs when that capture reports. A queued press whose grab starts more than 0.3 s after its key takes the next buffer from when the grab starts, as a retry does, so a press queued past its 1.2 s deadline does not miss at once. If the capture accepted a board, the queued press is dropped. A capture task that dies without a report no longer holds later presses. The 3 s debounce starts only at an accepted board. A retry takes the first buffer that arrives after the retry starts (press = now minus 300 ms, hard stop now plus 900 ms), instead of waiting another 300 ms plus a frame. Retries stay capped at two and start only while Tab is held. A release ends a running retry wait. A release from before the capture's own press is ignored. When Tab was released less than 1 s after the press and the kept frame showed no scoreboard, the log says `Tab was released before the scoreboard could be captured: hold Tab for about a second`, at WARN at most every 10 minutes and at INFO in between. It is not shown in the app. The retry log line now says "the frame shows no full scoreboard".
- Stream rate (#422): `screencast_max_fps` defaults to 4. The 1 to 5 clamp is unchanged, and 2 goes back to the old rate. Frames are still copied only while a grab waits. The poll's reuse wait follows the rate (350 ms at 4 fps).
- Stream reopen (#420): the once-per-game reopen cap is gone. Reopens are spaced 1 minute after the previous reopen, then 2, then 4, capped at 5 minutes, so a stream that keeps dying is reopened for as long as the tracker runs, never in a loop. A stall inside the gap reopens on the first poll after it. A new game resets the gap to 1 minute and keeps the clock. Reopens still run only on a poll, retire the old share before the new open, close the portal session of a failed open, and are skipped without a saved restore token.
- Stall fallback (#420): a stall is a dead stream, one waiting for its reopen, or a quiet one (no PipeWire buffer for 8 seconds). On niri and wlroots desktops a Tab or poll grab on a dead stream, or on one waiting for its reopen, takes a screenshot, and a poll on a quiet stream does so at most once every 10 seconds. GNOME with a forced stream does the same except for quiet polls. KDE, and GNOME on the automatic stream, never take one. On niri, sway, hyprland, wayfire, river and labwc with `WAYLAND_DISPLAY` set, every fallback screenshot, including the Tab timeout, tries wayshot first (one output, no file) and uses Older screenshot only if wayshot fails. The log line names which. `portal::capture` runs the portal request on its own task, so a request whose caller gives up still removes its PNG. A runtime shutdown in the middle of a request can still cut it short. The stall warning is logged once per 10 minutes, then at debug.
- Decided outcomes (#419): a Tab capture that was running when the result was decided, by the end-reel poll or an edit in the app, no longer writes `unknown` over it. A row with an unknown outcome takes the session's decided outcome, in the store and in `matches.jsonl`. A stored capture whose outcome differs from the finished game's re-seals the game, which re-queues its rows for upload once. A capture that lands after its game was deleted in the same run does not bring the game back.
- Shutdown (#419): the report of the capture that shutdown drained is applied before the game is saved. The stages run in a fixed order: capture drain, session save, screencast close (its own 1 s limit), final upload, snapshot export. The drain is bounded to the 5 second deadline minus 1.5 s. A stuck capture is detached and logged, and the game is saved without its report. A deadline overrun log names the stage that was running and the stages not reached.
- Game split (#419): the fresh-reset check measures its age from the baseline board (or the gate when there is no baseline), not from the last stored row of any kind. An unidentified row between two fresh own-row boards no longer merges two games into one. A fresh-shaped board on the baseline's row never replaces a mature baseline, its streak or the held board.
- Deletes (#423): a delete from the app schedules an upload on the same 3 second command tick, so its tombstone is sent then instead of at the next 5th-Tab sync, game close or restart. `delete_session` runs its three statements in one checked transaction, so a failed tombstone write leaves the local rows in place. A row written while its game was being deleted is removed again.
- Untracked games (#423): each session is decided by its newest unsynced row. If that row is local-only (an untracked map or mode), every unsynced row of the session stays local, and when any snapshot of it was a tracked read, raw or edited, a server delete is queued before the rows are marked synced. A plain Deathmatch game makes no request. A session whose newest snapshot is untracked no longer uploads its older tracked snapshot. A pending tombstone for a session that is uploaded again in the same pass is cleared locally and not sent.
- Idle sync (#423): every 60 seconds the command tick schedules a sync when none is in flight, the sync backoff allows it, and unsynced rows or tombstones exist. An edit in the app is sent by this tick within a minute.
- In-app update (#425): the installer runs under a 60-minute limit, in its own process group that is killed on timeout, and the existing rollback then puts the previous binaries back and starts the tracker. `curl_https` in `bootstrap.sh` passes `--connect-timeout 15 --speed-limit 1024 --speed-time 60`, so a dead connect or a transfer below 1 KiB/s for a minute gives up. The installer downloads into the app's own work dir (`STAT_TRACKER_DIR`), which the app removes. Its output goes to `{data_dir}/update-install.log`, overwritten each update, which both uninstallers remove. A failed Start after an install or a rollback is named in the final message, and "The tracker was started again" is only said when the Start worked.
- Quit during an update (#425): tray Quit, or closing the last window with no tray, while an update runs hides the window and exits when the update ends. Show cancels a waiting quit. A quit while the update still waits for the open game cancels the update before it stops, installs or starts anything.
- Update re-check (#425): every 6 hours, never before the start check, during an update, or in fixture mode. A re-check replaces the offer only for a different version, so a failed check keeps the banner, and a new version clears an old error.
- Reading model (#425): the Rebuild reading model button is removed. Install reading model restores `koverwatch.traineddata` from `PREFIX/share/scuffed-stat-tracker/tessdata`, then `/usr/share/scuffed-stat-tracker/tessdata` (the AUR package's copy), and keeps a differing existing model as `.bak`. With no bundled copy it shows the pinned reinstall command. A curl or in-app-update install keeps no bundled copy under PREFIX yet (`install.sh` copies the model straight into the user tessdata dir), so it gets the command. Training writes into a temp dir, and the installed model is replaced only after training succeeds. The missing-model status no longer says the model is needed to read the scoreboard.
- Copy (#407): the setup guide's site sign-in step has a Copy code button. It copies only the short code, never the device code, and the note under it never shows the code. Copy closes the tool's stdin after writing, sends its stderr to a temp file, and waits at most 3 seconds. After that the copy is reported unconfirmed, the app clipboard is written too, and the tool keeps running so a slow clipboard owner keeps the selection. This applies to all three Copy buttons.
- Release workflow (#418): the release and stat-tracker workflows run read-only, and only the job that creates the release can write. Actions are pinned to commit SHAs, checkouts do not keep credentials, and tracker builds use `--locked`. Release notes drop `HOLD-MERGE` markers from PR titles.

## 0.5.0-rc.5

On KDE Plasma Wayland the tracker now shares the screen once, with the cursor hidden, instead of taking a new screenshot every time it looks. That stops KWin crashing when a voice app is also sharing your screen. If you install an update while a game is open, the tracker switches to the new version on its own after that game is saved. Start, Stop, Restart, and Start on login work for an AUR or deb install as well as the curl installer, and they stop a leftover tracker instead of leaving two running.

### Highlights

- KDE capture hides the cursor and keeps one screen share open. The allow prompt is once, including after a reboot. The share closes when Overwatch quits, once that permission has been saved, and the next game opens it again without asking.
- If you cancel or close the screen-share prompt, the tracker stops asking. On KDE it takes no captures until you start Overwatch again, and the app says why.
- A Tab on the screen stream waits for a frame from after the key press, so a 2 fps share does not read the scoreboard before it is drawn.
- Stop and Restart find the tracker that is actually running, including one left over from an update, and leave a single copy.
- After an update, a running tracker restarts into the new version once the current game is saved.
- Settings has Automatic, Screen stream, and Older screenshot. An existing `STAT_TRACKER_CAPTURE=portal` keeps the new stream.

### Technical detail

- Service detection asks `systemctl --user show` for `LoadState` and `FragmentPath`, so a unit in `~/.config/systemd/user`, `/usr/lib/systemd/user`, or `/etc/systemd/user` counts. If systemctl cannot answer, those directories are scanned. Start on login, Stop, Restart, and the updater use that answer, not a remembered flag. `reset-failed` runs before start and before enable, which clears a start-limit-hit unit.
- Stop and Restart stop the unit when it exists, then signal every identity-checked process holding the pid file or the store lock. A process whose executable is not `scuffed-stat-tracker` is not signalled. If one is still alive, the error names its pid and another copy is not started. When the unit exists, Start only starts the unit. The unit sets `STAT_TRACKER_REPLACE_STRAY=1`, so that start replaces a same-binary stray before it takes the pid file. A start without that variable only replaces a different binary (the leftover from an update). `--vacuum` and a second start of the same binary do not signal the live daemon. A replace waits for `active_game.json` to go stale (up to an hour) so an open game is saved first. An explicit Stop does not wait.
- About once a minute the daemon compares `/proc/self/exe` with the binary on disk. If it was replaced and no game is open, a service exits 75. `SuccessExitStatus=75` treats that as success, and `RestartForceExitStatus=75` starts the new `ExecStart`. A process that is not a service re-execs the on-disk path. One log line says which. The restart remembers the stamp of the binary it exec'd, so a later update is still picked up, and a failed exec keeps the old process running. `STAT_TRACKER_SYSTEMD=1` in the unit is what counts as a service. `=0` on the stray-replace flag leaves it off. Restart now in the app waits until the current game is saved, and gives up that wait after an hour.
- Startup logs the resolved capture route, whether environment, Settings, or automatic won, and the desktop. A forced method that does not match the desktop is a warning (wayshot on KDE or GNOME, x11 on Wayland KDE or GNOME, a stream forced off Wayland). `portal-screenshot` on KDE is not a mismatch. `portal` and `screencast` are the new stream. `portal-screenshot` and Settings "Older screenshot" are the previous portal. A non-empty `STAT_TRACKER_CAPTURE` wins over `capture_route`. For the service, set the variable in `~/.config/environment.d/*.conf`. Settings names that file when it is overriding the choice.
- The stream is one xdg-desktop-portal ScreenCast session with cursor mode Hidden. Cursor metadata is never requested. The restore token is created mode 0600 at `{data_dir}/screencast-restore-token` and is not logged. A backend that rejects persistence is asked again without a token, still with a persistent grant. Cancelling or closing the share prompt stops asking until Overwatch is started again, and the restore token stays unless the portal rejected it. On KDE there are no captures in that time, and no screenshot fallback. The log and the app status say `Screen share was declined: no captures until Overwatch is restarted` once per decline, not on every Tab. A desktop where a screenshot fallback is already safe keeps that fallback. A dialog that times out is a No for this launch only when no restore token was sent: no captures where a screenshot is not already safe, one warn line, and the reason in Tracker health. The next Overwatch launch asks again. A timeout after a token was sent is not a No. It waits 30 seconds and tries again, with no decline notice. A compositor that reports cursor modes and omits Hidden is logged as not offering a hidden cursor. If the portal cannot be reached, or `AvailableCursorModes` cannot be read or comes back empty, the log says the portal is unavailable on the first failure, again only when the failure kind changes (the portal proxy, the cursor-mode read, or empty cursor modes) or after 10 minutes, and at debug in between. A successful open clears that warning, so a later outage inside 10 minutes still logs. That failure keeps the restore token and waits 30 seconds before the next open, so a saved KDE grant comes back without a prompt. On KDE that backoff does not open the screenshot portal. That skip is logged at info once per backoff window, then at debug. The portal-unavailable warning, including the cursor-mode read, follows that same rule. The poll remembers the last frame sequence together with the stream that produced it, so a reopened share whose frame numbers start again is still read. PipeWire older than 1.4 does not export `pw_loop_iterate`, so those versions run `pw_main_loop_run`. One log line names which path is used. A failed or timed-out open closes its portal session before the next try, so KWin does not keep a second share. PipeWire is loaded with dlopen, not linked. Stream properties are created from an empty `spa_dict`. `pw_properties_new_dict` is not called with a null dict. The offer is framerate 0/1 plus max-framerate N/1. `screencast_max_fps` sets N (default 2, clamped 1 to 5) and the tracker's own throttle. The frames-per-second debug line prints one decimal, so a rate under one frame a second is not shown as 0. Only the newest buffer is kept, and it is copied only when a grab is waiting. A poll waits one frame period plus 100 ms for a new buffer before it reuses the previous frame (600 ms at 2 fps, 350 ms at 4 fps). A grab with no new buffer can hand back the previous frame, marked as already seen. The poll ignores that frame. That skip is logged at debug once per 30 seconds, then at trace. A wait that ends with no new buffer is not a read, so one reused picture cannot confirm Victory or Defeat. The image stays at the stream's own pixel size, and is cropped only when the stream covers more than one monitor. The configured monitor is matched by position and size. Pixels are copied in the negotiated order (BGRx, BGRA, RGBx, RGBA, xBGR, and the other raw formats) using the buffer stride and offset, so a red pixel stays red. The stream is created on the portal's PipeWire connection and targets that node id, with no fallback to another camera. The share closes when the game process gate goes from running to not running and a restore token is saved, and on shutdown. A Tab that is still retrying cannot open it again until the next game is seen. Without a token it stays open so the next game does not ask again. The daemon writes whether the share is open into `ocr_switch.json`. On KDE, Capture now and the setup guide read that and do not take a portal screenshot while the share is open. A status file whose pid is not the live daemon counts as closed. The log names the portal backend from the `.portal` files and `portals.conf`, and the running `xdg-desktop-portal*` executable, not `comm`.
- A Tab on the stream does not sleep 400 ms. It takes the first buffer whose arrival is at least 300 ms after the key press, and stops waiting 1200 ms after the press. A buffer from the first 300 ms does not end the request and is not copied. The first Tab of a game waits up to 5 seconds for a fresh frame from a share that is still opening, instead of dropping the key while the dialog is still opening. A share that takes about 4 seconds still keeps that Tab. If the open uses up the 1.2 second frame budget, the grab runs again from the moment the share opens. A later Tab retry starts a new 1200 ms frame deadline from its own press. It does not repeat the 5 second open wait. While the share is still opening, that retry is not logged as a missed frame. On KDE a timeout, a backoff, or a dead stream does not open the screenshot portal: the grab is skipped and logged as a stream miss, not as a rejected scoreboard, and no placeholder image is saved. On niri, wlroots, and GNOME when the stream was forced, that timeout takes one screenshot. Each Tab grab logs the delay from the press to the frame. Poll grabs use a new stream frame only. A share that never starts streaming is closed after a few seconds, then the 30 second backoff applies. Three grabs in a row with no usable buffer do the same. A quiet stream that stays in Streaming is not reopened, and polls do not count toward a reopen. The stream is reopened when PipeWire reports an error, when it stays unconnected after it had been streaming, when a pause lasts 2 seconds, when the portal session closes, or when the connection faults, and when two Tab waits in a row get no PipeWire buffer. That reopen happens on the next poll, so the Tab that noticed it keeps its result. It happens once per game, and at least 5 minutes after the previous reopen. A new game clears the miss streak and the once-per-game cap, and leaves the 5 minute clock. Without a saved restore token the stream stays up and one warning is logged. Where a screenshot fallback is used, the log names it Older screenshot. The permission dialog times out and a shutdown cancels it. `auto_detect.poll_interval_ms`, when set, overrides `poll_interval_secs` and the automatic 1.5 s or 4 s interval (clamped 500 to 10000). SIGTERM and SIGINT share one 5 second deadline from the moment the signal arrives. A self-restart after an update has no signal, and that shutdown gets the same 5 seconds. The loop polls that signal first, and also wakes when the background listener has already seen Ctrl+C. A poll capture already running is raced against that signal and is dropped when it wins. The OCR task is detached when the signal wins: it may finish in the background and does not write a match row. A poll write that has started, including the sealed outcome in `retire_active_game`, runs to completion inside whatever of that deadline is left, and is not dropped mid-write. The runtime then stops with whatever of that deadline is left, even if a capture is still running.
- A scoreboard row whose every cell is suspect, or whose own-row identity is suspect, does not save those numbers and does not raise the running totals. One later clean read replaces a low-trust total even when the clean number is lower. A total a clean read already confirmed still does not go down. When the scoreboard header strip is missing, the error names how many rows were wide enough and how long the longest run was.
- `debug_portal_frames` or `STAT_TRACKER_DEBUG_PORTAL_FRAMES=1` saves up to four Tab pairs under `{data_dir}/debug/portal-compare/`. They stay on this machine. The screenshot half can hit the KWin cursor crash while a voice app is sharing the screen, so leave it off for normal play.

## 0.5.0-rc.4

Screen capture now works on KDE Plasma on Wayland without any workaround. In rc.3 the tracker still tried a capture method that KDE does not support, so "Capture now" and game recording failed with "Cannot find required wayland protocol". It now tests that method with a real screenshot and moves on to your desktop's screenshot prompt when it fails. With two monitors on KDE you can pick which one the tracker watches, instead of getting both screens in one image.

### Highlights

- Fixed screen capture on KDE Plasma Wayland. The tracker uses your desktop's screenshot prompt, so allow it once.
- With two or more monitors on KDE or GNOME Wayland, the monitor picker now lists your screens, and the tracker only reads the one you chose (Auto uses the first one).
- If you added a `STAT_TRACKER_CAPTURE=portal` override for rc.3, you can keep it or remove it. It picks the same method.
- On Auto, the tracker now prefers the external monitor. You can still pick one in Capture.
- Picking another screen in Capture and saving now restarts the tracker for you, so the new screen is used right away.
- When a capture fails because the desktop lacks wlroots screenshots, the message now names the missing protocol and the portal override.

### Technical detail

- wayshot probe: KWin exposes wl_outputs but neither wlr-screencopy nor ext-image-copy-capture, so the old probe (connect and require one output) passed and every real capture failed with libwayshot `ProtocolNotFound`. The probe now takes one real screenshot of the first output on a short-lived connection and discards it, so auto-detect falls through to the portal. The portal is probed at most once, and the chosen method is logged as "capture method chosen". The daemon and the app share `detect_backend`, so both get the fix.
- Portal crop: the XDG screenshot portal returns the whole desktop. Output names and logical rectangles come from xdg-output (via libwayshot, which KWin serves without screencopy). The chosen output's rectangle is mapped into image pixels by the ratio of image size to the logical desktop's bounding box, which covers outputs at different heights and offsets and fractional scales, and is clamped to the image. One output, or no readable geometry, keeps the full image. `list_outputs` now returns those names for the portal backend. When the portal draws the desktop above an output's real mode (a 1920x1080 monitor at scale 1 next to a laptop at 1.25 comes out 2400x1350), the cut-out is resized back to the mode size with Lanczos3; within 1px it is left alone.
- Auto output (portal and wayshot): with several outputs, external monitors win over built-in panels (eDP, LVDS, DSI), then the largest real pixel area, ties in list order. A single output is unchanged. X11 Auto is unchanged.
- The tracker reads `capture_output` only at start, so the app now restarts a running tracker (through the same path as the Restart button) when a save changes it.

## 0.5.0-rc.3

Screen capture now works on KDE Plasma and GNOME on Wayland. The tracker used to pick a capture method that cannot see the screen there, so setup said "Could not capture the screen". It now asks your desktop for a screenshot instead, and KDE shows an Allow prompt the first time (#411). If capture still fails, setup shows the reason so you can send it to us.

### Highlights

- Fixed screen capture on KDE Plasma and GNOME Wayland desktops. Allow the screenshot prompt once (#411).
- niri, Hyprland and Sway on Wayland, and X11 desktops like Linux Mint, keep the capture method they used before (#411).
- When the capture test in setup fails, the message now ends with the real reason (#411).
- If you added a `STAT_TRACKER_CAPTURE=portal` override as a workaround, you can keep it or remove it. It picks the same method (#411).

### Technical detail

- Backend order (#411): on a Wayland session (`XDG_SESSION_TYPE=wayland`, or `WAYLAND_DISPLAY` when the type is missing) the automatic order is wayshot, portal, then X11 as a last resort. KWin and Mutter have no wlr-screencopy, and the X11 probe passed through XWayland, so X11 was picked and could not see the screen. X11 sessions keep wayshot, X11, portal. `STAT_TRACKER_CAPTURE` still forces a backend. The daemon and the app share `detect_backend`, so both get the fix.

## 0.5.0-rc.2

The name reader now really switches on after you update: once the `names-v1` pack downloads, from the tracker itself or from the app's setup guide, the tracker starts reading names with it right away, with no restart. Starting or restarting the tracker takes a few seconds again instead of about a minute (#401). When two of your accounts are in the same lobby, games show fewer unsure stats (#404).

### Highlights

- When two of your accounts are in the same lobby, games show fewer unsure stats: a clear highlight on your row that matches your hero now counts as sure (#404).
- If you updated to rc.1, you do not need to do anything. The tracker sees that the name pack was saved without one of its files and downloads it again by itself (#401).
- A pack that does not check out is never switched in. The tracker keeps reading with what it has (#401).
- Fixed the slow start on Wayland and X11 desktops: the step that finds your desktop session runs in under a second, and its log line now says which session it found and how long that took (#401).

### Technical detail

- names-v1 manifest (#401): rc.1 unpacked `names-v1.tar` without its inner `manifest.json`, which the name reader needs (canvas, glyph and mark counts, params, per-file sha256). It logged `names-v1 pack refused: missing manifest.json` and stayed on `ocr-v1`. Pack sync now keeps the verified manifest in `templates/names-v1/`. The other packs' folders are unchanged.
- rc.1 folders (#401): a names-v1 folder without `manifest.json` counts as not installed, so the next sync downloads the pack again, even though `pack-versions.json` already names it.
- Reload after install (#401): the name reader loaded once at startup, about a second before the pack keeper installed names-v1. The keeper now reports each install, and the daemon reloads the name reader on a background thread. Before the swap it checks that the manifest sha256 on disk is the one sync verified (before and after the load), every file against that manifest, and the version against the server list. The cached pack is then replaced in one step, so a scoreboard read in flight keeps the pack it started with. A pack that fails a check leaves the current reader running. The next game after the pack lands is stored with `name_recognizer = names-v1`.
- App installs (#401): when the app's setup guide installs names-v1 while the tracker runs, the tracker reloads it with the same checked path once the install is complete: the live folder's manifest version matches `pack-versions.json` and its sha256 matches the one pack sync now records in `templates/pack-manifests.json`. Staging and backup folders are ignored, nothing reloads between the swap and the record writes, and a version-only record still has every file checked against the folder's manifest. A failed check keeps the current pack. A folder changed by hand with no matching record is used from the next start.
- Session environment (#401): `import-session-env.sh`, the oneshot before every daemon start, walked all of `/proc` once per compositor name with two commands per process (about 60 s on a busy niri desktop). It now reads each process name once with the shell's own `read`, skips unreadable or exited processes without a message, stops after 5 s and falls back to the process environment as before.
- Own row with several accounts (#404): when two or more rows match saved names (several of the member's accounts in one lobby), no single row leads by 0.10, so before this every board fell back to the highlight alone and marked all 7 own-row fields. Now a strong highlight (lead at least 1.30) with the career hero read on that row is a sure own row (`highlight_and_hero`). A weak highlight, the hero on another row, or no career panel stays unsure. A seed that leads by the margin (0.97 vs 0.84) is still a normal seed match. Learning a new name crop from the career hero alone needs 4 boards, 3 games and 2 heroes, and only on boards where no row matches a saved name. Games already stored keep their flags.
- Session lookup (#401): `plasmashell`, `kwin_x11` and `cinnamon` are added, so Mint Cinnamon on X11 takes `DISPLAY` and `XDG_CURRENT_DESKTOP` from the Cinnamon session. One log line names the session source, the processes scanned and the time taken.

## 0.5.0-rc.1

This is the first release candidate for 0.5.0. The name reader is on by default and is used once the `names-v1` pack has downloaded from the site; until then the tracker keeps using the current reader. Syncing sends a big backlog in smaller batches. If the site refuses one game, only that game is set aside, marked "Upload rejected by server" on its card with a Retry button, and the rest still upload (#386, #370, #378).

### Highlights

- Classic OCR in the downloaded Linux release uses Tesseract 5.5.3, and Linux Mint 21 and other systems with glibc 2.35 still run it. An update or uninstall removes a leftover libtesseract.so.4 from an older install (#360).
- Uninstall in the app no longer removes anything while the tracker is still running, and says so (#395).
- Better reads: header labels are found on smaller and dimmed scoreboards (cv-v7), an unsure stat now saves what the board showed, flagged, instead of an older number (cv-v8), and the last hero portrait on 5v5 boards is no longer cut off (#363, #391, #382).

Technical detail. Name reader (#371, #386): `name_reader` is on unless `config.toml` sets `name_reader = false`. Once a `names-v1` pack has downloaded (an optional pack sync entry, installed to `templates/names-v1/` and checked against the sha256 in the server's pack list), it supplies the configured player's row when that row meets the pack's own `min_confidence`. A blank or low-confidence row falls back to Tesseract for that row only. With no pack loaded, the match keeps the Tesseract rows already in hand and is tagged `ocr-v1`. The match stores `name_recognizer` (`names-v1` or `ocr-v1`) and sends it on every upload and resend; the server stores it from #373. Sync caps (#370, #396): each upload carries at most 200 matches and 500 deleted sessions, now taken from the shared `scuffed_types` caps added in #372. HTTP 413 `batch_too_large` shrinks the caps and resends in the same pass, a 429 stops the rest of the chunks and keeps `Retry-After`, and 401 `token_expired` pauses sync and asks the app to relink. Learned caps reset when the server URL changes. Quarantine (#378): a 400 or 422 whose body names a row bisects that chunk until the bad rows are sent alone. A row that fails alone is quarantined only when another row in that chunk was accepted; if the chunk accepts nothing, every row stays queued and sync backs off. Editing the game, the per-row Retry button, or **Retry rejected uploads** on the Games page clears the flag and queues the game again. A future 409 `stats_delete_pending` does no splits, pauses upload, and keeps every row queued locally; automatic sync stays off until a new token is saved.

Release build (#360). The release is still built on Ubuntu 22.04. Tesseract 5.5.3 is compiled on that runner with OpenMP on, without -march=native, and without libcurl or libarchive. Leptonica stays Ubuntu 22.04's 1.82. The daemon loads the bundled libtesseract.so.5. The koverwatch model is still trained with Ubuntu's Tesseract 4.1.1 tools, and the same size check still applies. How this library reads that model has not been measured yet.

Also since alpha.12: the saved own-row review crop now includes the hero portrait (#379). Dimmed headers mark own-row suspects on the ocr-v1 fallback as well as the cv reader, rank results are logged on the first value and again only when it changes, and a report uses the stored defeat when the kit result is unknown (#383). Tests pin the icon reader on a cut 5v5 last row and the ocr-v1 own-row name match on a synthetic board (#384, #364). Uninstall retries transient stop errors and treats a missing unit or an already-stopped process as stopped (#395). Report bundles use zip 9.0.5, and rustls and ammonia are patched for current advisories (#374, #365).

## 0.5.0-alpha.12

The new reader is now the default, and when it finds nothing on a scoreboard the tracker reads that board with the old reader (ocr-v1) instead of skipping it (#333). Your last game now uploads right away even when a late Tab closes it, and capture on X11 is much lighter on bigger screens (#320, #330).

### Highlights

- If you already chose `reader = "ocr-v1"`, nothing changes for you. To go back to the old reader, pick it in Settings or put `reader = "ocr-v1"` in `config.toml`. If you had chosen the old reader, check Settings after updating: an older bug could drop that choice when several settings were saved at once.
- Alpha installs are now offered newer alphas in the app even if you never ticked "Get alpha updates", and every release page has a pinned "Update from the terminal" line (#321, #322).
- Report a problem picks one kind at a time, Performance and Crash reports never send pictures, and the note box asks what you noticed (#323, #329).
- Smaller fixes: Proton VPN is no longer mistaken for the game's Proton, the setup line names your CPU, up to 60 debug crops are kept, and the app says when hero names need site sync (#327, #332).

Technical detail. `ReaderSetting` defaults to `New`; a config without a `reader` key loads as `new`, and `reader = "ocr-v1"` is still written out explicitly. Per-Tab fallback (`reader_apply::ocr_fallback_reason`): a board with status not found or team size unknown, or with no digit value in any stat cell, is read with ocr-v1's cell pass for that Tab when classic OCR is on, and stored with recognizer `ocr-v1`. A board the reader found but doubts (team size disagreement, unsure own row) is refused or held as before, and with `ocr = "off"` nothing falls back.

Also since alpha.11: a game closed at a Tab boundary schedules its own upload, and an empty Tab-opened game closes after 20 minutes instead of 6 hours (#320). X11 `image_to_rgba8` converts the common 32-bit ZPixmap layout a row at a time, about 48 ms to 3 ms per 2560x1440 frame on the build box (#330). Update checks treat an alpha install with no `update_channel` as the alpha channel (#321). `dist/release-notes.sh` adds a tag-pinned terminal update block to every release body (#322).

## 0.5.0-alpha.11

The tracker now keeps a small always-on performance log while it runs. Choosing Performance under Send report sends only that log, with no board pictures or crops (#318).

### Highlights

- The performance log notes your setup once (including Proton when it is running), then about once a minute records CPU, memory, screen grab time and read time, and how CPU is split between grabbing and reading.
- On X11 each grab also logs how long it took, its size, and whether it took the whole screen, so a slow full-screen grab shows up clearly.
- Performance reports stay small: only the scrubbed performance log goes up, as a no-game report the site already accepts.

Technical detail. New `perf_log` module writes `perf.log` in the data dir (rotates to `perf.log.1` at 256 KiB, about 512 KiB kept). Setup header covers app version, session type, kernel, CPU count, RAM and Proton version. Sample lines include daemon CPU% and RSS, capture p50/max, `capture_cpu_ms` / `ocr_cpu_ms`, poll analysis timing, and OCR run count with p50/max. X11 GetImage grabs record time, size and scope (`full` vs `output`) plus `x11_shm=no`. Wayland and portal grabs use the shared capture timing only. Report a problem, Performance: `log.txt` is only the scrubbed perf log (latest setup header first, newest lines); no daemon journal lines, pictures or crops. Manifest stays the no-game shape (`session_id` none, `game` null, empty suspect reads, one `log` file).

## 0.5.0-alpha.10

The daily report limit is now 10 (#284). A restart continues the game you were in only when it is clearly that same game, and a clipped 5v5 scoreboard no longer stores a hero the tracker is not sure of. A game report waits until you pick what was wrong. A game report's log now covers only that game. If you crash mid-match and rejoin, it stays one game with your stats added up (#293).

### Highlights

- Fixed a Wayland screen capture leak that made the compositor hold more and more memory while the tracker ran, causing frame drops and possible crashes (#306).
- If an in-app update fails, the tracker puts the previous version back and starts again, instead of staying stopped (#298).
- The tracker now has a memory limit, so it can no longer help push your PC into running out of memory (#298).
- Hero portraits now come from the downloaded hero pack instead of being built into the app (#299).
- Names read on a scoreboard stay on this PC and are used to blank those names in a report. They are never included in the report file (#290, #292).

Technical detail.

- The cap counts successful stores for one member since midnight UTC. A rejected upload does not count. Over the cap, the site responds 429 (#284).
- A 5v5 read whose header still starts at the left edge flags every row unknown and suspect. The hero log id is `hero-v1.2`. The own row's hero is stored only when the hero authority names that same hero. No authority known keeps the previous behaviour. A contradicting or abstaining authority keeps the earlier hero and adds `hero` to `suspect_fields`. Uploads still send `cv-v6` (#288).
- After a restart, an unfinished game with a stored board resumes only with a known map and a last capture at most 5 minutes old. Otherwise that game is retired and the next Tab starts a new game. A clock that moved back also starts a new game. A game that already saw a result is restored so the result and the post-match scoreboard stay on it, and the first Tab more than 75 seconds after that result starts a new game. A session with no stored board still uses the 20 minute bound (#289).
- After a crash, a scoreboard on the same map with reset stats within 5 minutes of the last heartbeat continues the restored game once. The stats from before the crash are carried and added to the new reads. The carry is saved in `active_game.json`, so a second restart neither double counts nor opens a new window. Outside the window a stat reset still splits (#293).
- A game report opens with no category selected, so no zip is built and Send and Save stay off until one is picked. The prompt after Edit stats still opens on Wrong stats. Each accepted capture appends the names read on its rows to `report-kit/<session>/names.json` (mode 0600, at most 64 names) only when that kit folder already exists. The file is pruned with the kit and passed to the scrubber. With the new reader no names are read, so the list stays empty (#290).
- A game report reads the journal from 120 seconds before the first capture through the end of that game's window, still capped at the newest 20,000 lines. When the game has no end time the read stops at now. A log-only report stays newest-lines only. No report zip contains `names.json` (#292).

Rejoin after a crash: within 5 minutes of a restart that resumed an unfinished game, a board on the same map (read on that Tab) from your own trusted row, where at least two of elims, deaths and damage fell, continues the resumed game. The stats from before the crash are added to every later board of that game, so stored stats never go down. It applies once per restart. A different map, a game that already had a result, or a drop more than 5 minutes after the restart splits as before. The restart resume window (5 minutes) now counts from when the crashed run was last seen alive (its heartbeat, at most a minute before the crash) instead of from your last Tab. The 20 minute limit since the last Tab still applies.

The tracker no longer ships hero portraits in the binary. The portrait matcher reads the installed hero pack (`heroes-v1`, `heroes-v2`, `heroes-v2-1080`). With no pack installed it names no hero. The portrait matcher has no recognizer id. With a pack installed it compares those pack pngs instead of the old bundled 32x32 art, so a portrait-only hero confirmation can differ. `hero-v1.2` is not bumped: that id is the template recognizer, which already read the same pack files, and its scores do not move. The digit recognizer stays `cv-v6`.

Updates and install: a failed in-app update restores the saved `scuffed-stat-tracker` and `stat-tracker-gui` binaries and starts the service again. The updater now also requires `python3` and `sha256sum`, and the install script refuses to install when `sha256sum` is missing instead of skipping the check. The prerelease channel only offers alpha and rc builds, so a test build such as `-test.1` is never offered or installed without a pinned tag. The systemd user unit sets `MemoryHigh=1200M`, `MemoryMax=1500M` and `MemorySwapMax=256M`; the daemon peaks at about 1 GB today, and over the limit only the tracker restarts.

## 0.5.0-alpha.9

Install this update between games, not during one. A Tab press that caught the scoreboard while it was still drawing no longer loses the game: the tracker now looks again a moment later.

### Highlights

- When a Tab scoreboard is turned down because the game had not finished drawing it, the tracker captures it again about 300 ms and about 700 ms later before giving up (#286).
- Report pictures now work for 5v5 games, with player names blanked (#285).

Technical detail. A Tab capture that the scoreboard preflight rejects as not a scoreboard is retried twice, at about 300 ms and 700 ms after the first rejection, and each retry is logged. Only that rejection is retried, and the best outcome of the attempts is kept, so a later rejected grab never replaces a useful one. The report blanker measures 5v5 boards with their own row geometry; the blank-table version is now 12 (#285).

## 0.5.0-alpha.8

Install this update between games, not during one. Report this game works again for every game in your history, and the report now says which game it is about.

### Highlights

- With the new reader, the tracker looks for Victory and Defeat screens every 1.5 seconds instead of every 4, so short end screens are no longer missed.
- A saved check interval of 4 seconds now means auto (1.5 s with the new reader). For slower checks on a low-power laptop, enter 5 or more in Settings.
- With classic OCR off, a game where one stat was never read clearly is saved with its best guess instead of being dropped. That stat is marked unsure, and clicking it opens Edit stats.
- An update waits until the game you are in is saved before it restarts the tracker.
- A game whose scoreboard picture could not be kept safely can still be reported. The report then carries the saved values and no picture.
- The report log now has the tracker's own lines from that game, taken from the system journal and scrubbed of player names and home folders.
- Game cards show the date as well as the time, the hero's role sits next to the hero name, and Delete session no longer gets cut off on a narrow window.

Technical detail. Report pictures stay in `report-kit/<session_id>/` in the data folder, outside `debug/`, written whether or not debug OCR is on, and pruned to the newest kits. When the board blanks to nothing or to plain fill (for example when its header strip is not found), no picture is saved and `reads.json` is still saved with an empty `scoreboard_sha256`, so the report links its `session_id`. A game with no kit at all is reported from its stored row. The manifest resolution is always landscape. After a report is sent, its heading reads Sent and the delete date is shown in local time with its UTC offset. The New reader (alpha) text now says what happens with classic OCR on and with it off. `auto_detect.poll_interval_secs` is now optional: missing, or 4 (the value older Settings always wrote), means automatic (1.5 s with the new reader, 4 s with classic OCR); any other value overrides both. The best guess is stored when the game closes, with the column in `suspect_fields`; mid-game no row is written while a column is unread. When the site's daily report limit is reached, the app shows the site's message and keeps the game's report pictures so it can be sent the next day. A report holds at most 30 files besides `manifest.json`, no picture over 7680 px on a side, and at most 10 MiB in total.

## 0.5.0-alpha.7

This alpha adds a switch in Settings that turns classic OCR (Tesseract) off, so the tracker reads the game with its own reader. The switch is opt-in: if you leave it alone, OCR stays on and the tracker reads as before.

### Highlights

- The switch is Settings → Scoreboard reading → Use classic OCR (Tesseract). Turn it off, save, and restart the tracker. It needs three reader packs from the site (heroes, maps and words). While one is missing, the tracker keeps OCR on and tells you which.
- Classic OCR off has limits. A game with a stat that was never read is not stored, and reading a result screen takes more time. The full list is further down.
- With classic OCR on, reading uses less CPU, with identical reads (#259). With the Tesseract 4.1.1 the release ships, a scoreboard read uses about 41 to 47% of the CPU it did. The cost: at 1080p one classic OCR scoreboard read takes about twice as long to finish, about 8 s instead of 4 s. At 1440p a typical read is a little faster, but the slowest ones take about twice as long.
- When an update changed the app while its window is open, the window asks: The app was updated. Restart now? (#255). Settings now uses the width of a wide window (#258).

The OCR switch (#261). In config.toml the setting is `ocr = "on"` or `ocr = "off"`. A config without that line means on, so nothing changes until you change it. Off means the new reader reads every screen and Tesseract is not started for reading. The tracker reads the setting when it starts, so restart it after you save. The tracker turns OCR off only when nothing would go unread: it needs a hero pack, the map pack and a current word pack. Settings shows Reader packs ready: N of 3. If a pack is missing, the tracker keeps OCR on for that run and says why, and Settings refuses the click with the same reason. If a pack goes away while the tracker runs with OCR off, what that pack reads is put on hold and the app says which pack it is. The tracker does not go back to OCR by itself. Turning OCR back on in the middle of a game keeps what the game has so far: a stat with no sure read yet stays open until a scoreboard reads it. While the tracker runs with OCR off it always uses the new reader. Settings then says so in place of the New reader (alpha) switch, and hides Install reading model and Rebuild reading model.

What classic OCR off costs. The counts are from the final test run on this version's code, with the OCR model an install uses:

- Stats. A stat the reader is not sure of is never stored as a guess: the game keeps the last sure number and marks the stat. A game shows up only once each of its six stats has been read. A game that ends with a stat never read is not stored (#266). On 30 test games, 22 got a stored row with the new reader against 23 with OCR, and 132 final stats had a number against 138. One game never had its healing read. A follow-up is planned before off can become the default: keep such a game on this PC and upload it when the stat is read or typed in.
- Time on a result screen. One check of a result screen takes about 61 ms with the new reader, against about 24 ms with classic OCR on Tesseract 5. With the Tesseract 4.1.1 the release ships, one OCR read of a result screen takes about 46 ms; the whole check was not measured. A scoreboard read is far faster with the new reader: about 26 to 34 ms, against about 0.6 s at 1440p and 7.3 s at 1080p for classic OCR as shipped.
- The map. With OCR off the map name on the scoreboard has no second reader. When no scoreboard and no end screen of a game gives the map, the game is saved without a map and stays on this PC until you pick one. The new reader read the map name on 18 of 19 of our own scoreboards at 1080p (OCR: 5), on 43 of 50 at 1440p with 1 more wrong and marked (OCR: 39), and on 80 of 87 scoreboards from public videos at 1080p (OCR: 18).
- Your hero. The hero icon on your own row was read on fewer of our scoreboards (6 fewer at 1440p, 4 fewer on 5v5; none read wrong). The tracker also reads the hero panel, and that full read was not measured again.
- Your row. A scoreboard is not stored when all of these are true: your row has never been highlighted, your hero icon is dead or covered, and the tracker has not learned your name yet (#273). With OCR on, the name read can find your row on such a scoreboard. In the test run, OCR off lost none of the 150 test scoreboards that OCR on reads.
- Co-op event. A co-op event scoreboard (Junkenstein on Adlersbrunn, stats on one team only) is left out only when its map name is read sure. When it is not, that scoreboard can be saved as a game (#261, #271).
- A draw. On the post-match and rank screens the new reader reads VICTORY and DEFEAT only. A draw is read from the end title (#270).
- Not tested on real pictures, because we had none: the ESPERANÇA line on the end screen (#265), ADLERSBRUNN on the scoreboard and on the end screen (#271, #265), and a Junkenstein scoreboard (#271). The first two were tested on lines drawn with the map pack's own letters. SELECTING MAP was read on 13 pictures at 1080p, and at 1440p only on the one picture its template was cut from (#269).

What the new reader reads without OCR in this alpha. It is used when classic OCR is off, or when New reader (alpha) is on:

- The six stats on the scoreboard (#266).
- Your row: from the highlight, from a few small crops of your own name that the tracker learns, and from the hero on your hero panel when the highlight is not found (#262, #273). The crops stay on this PC, are never sent, and are deleted on uninstall.
- Your hero, from the hero icons on your row and on the hero panel (#249, #256). A 5v5 scoreboard that is wider than the usual cut is cut again, so the icons line up (#249).
- The map and mode, from the map name on the scoreboard (#251, #252) and from the end screen (#265). Adlersbrunn, the Halloween name for Eichenwalde, is in the map list and is saved as Eichenwalde (#271).
- The result: VICTORY or DEFEAT on the post-match and rank screens (#253, #270), and the end title (#263). The end title counts when two checks in a row read the same word.
- The match start: VOTE FOR A MAP, BAN HEROES and SELECT A PREFERRED HERO (#260), and SELECTING MAP, which needs version 2 of the word pack (#269).
- A flat area of one colour can no longer score as a word (#272).

Other changes:

- Classic OCR no longer starts extra threads inside each read (`OMP_THREAD_LIMIT=1`). An update sets it by itself, so you do not need to change anything (#259). It was measured on this version's code with the Tesseract 4.1.1 and the game-font model the release ships. Reads were identical with and without the limit. OCR's CPU time for a whole test run went down by 2.1 to 2.5 times. One 1440p scoreboard read: CPU from about 6.8 s to 1.3 s, and a typical read finishes in about 0.5 s where it took 0.6 s, but the slowest one in twenty takes 6.8 s where it took 3.1 s. One 1080p scoreboard read: CPU from about 18 s to 9 s, but a typical read takes about 8.1 s to finish where it took 4.2 s.
- The tracker now downloads all six reader packs from the site, not only the first hero pack (#248, #250). If the site was busy, offline or had downloads turned off when the tracker started, it tries again by itself, so you no longer need to restart the tracker to get them (#268).
- The tracker no longer runs an OCR pass on the top of the Tab screen looking for a VICTORY or DEFEAT header, because the current game shows no result word there. The result is still read from the end title, the post-match screens and the scoreboard's own text. This changes the default reader too. Not checked: end-of-match Tabs in Push, Flashpoint or Clash, quick play, arcade, custom games, Stadium, and a draw (#257).
- The old CHOOSE YOUR HERO title is no longer looked for at match start, with either reader. The game does not show it any more (#260).
- With the new reader, Practice Range Tabs are recognised and kept local (#251, #252).
- The restart question appears only when the app itself changed. After an in-app update that did not change the app, the button says No restart needed (#255).
- Settings on a wide window: three columns at 1920 and 2560 wide, four at 3440. Before, it stayed at two narrow columns. A portrait monitor keeps one column (#258).
- A problem report now says when the tracker ran with classic OCR off, and the site accepts that (#267, #261).
- Report pictures still have hero portraits filled. The plan from alpha.6 to bring them back in this alpha has not happened yet.

The default is unchanged. With the switch untouched, the tracker reads as in alpha.6, apart from the changes listed above. Off becomes the default only when it is better than OCR on every point, and it is not yet. Behind: games that are not stored (22 against 23, measured with Tesseract 5 only), the hero icon on your own row (6 fewer at 1440p, 4 fewer on 5v5), and the time to check a result screen (about 61 ms against 24 ms with Tesseract 5; not measured with the shipped 4.1.1). Ahead against classic OCR as shipped: the end title (33 of 33 at full brightness against 2, and VICTORY on 6 of 19 fading pictures against 4) and the time to read a scoreboard (about 30 ms against 0.6 to 7.3 s).

These notes were corrected after the release. The first version quoted classic OCR as measured with Tesseract 5, while the release ships Tesseract 4.1.1, which reads end titles less often and takes about twice as long.

Stable installs are not offered this alpha. The desktop app skips it until Settings → Get alpha updates is on (or config.toml has `update_channel = "prerelease"`). bootstrap.sh skips it until `STAT_TRACKER_CHANNEL=prerelease`. Pinning this tag still installs it.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.5.0-alpha.7/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v0.5.0-alpha.7 bash
```

Or extract the tarball and run `./install.sh`.

## 0.5.0-alpha.6

This alpha lets you report a game the tracker got wrong, with a picture of the scoreboard in which every player name is blanked before the picture is saved.

### Highlights

- Report this game is on each game card that has a saved scoreboard. Pick what was wrong (stats, hero, map, mode or result), look at the pictures and the list of files that would be sent, then Send it or Save the zip. A game the tracker never recorded can be reported as Missed game from Report a problem, with no pictures. After you fix a game's result, stats or hero, the message offers Send this fix to help training, which opens the report with training ticked; you can untick it before you send.
- An update now restarts the tracker service if it is running, so the new version is the one that runs. A game in progress at that moment is not kept.
- Only one copy of the app runs: opening it again brings up the window you already have (#241). Coming from alpha.5: when the update has finished, quit the app (from its tray icon, if it has one) and open it again, because the alpha.5 app cannot restart itself into the new version.
- On 5v5, your own row is now also found by your name during the match, with or without perk icons, not only by the highlight (#239).

Game reports (#235, #236, #238, #246). While a game is captured, the tracker keeps a report kit for it on this PC: the newest accepted scoreboard of the game, blanked when it is captured, the result word from the competitive rank screen, and the values the tracker read from that scoreboard. The kits of the last 20 games are kept, in folders only your user can open. On the scoreboard only the digits are copied; names, titles, badges, icons, chat and everything around the table are filled with a flat colour. Hero portraits are filled too in this alpha; alpha.7 plans to bring them back, pasted from the game's own portrait art instead of copied from the screen. Only 6v6 boards at 1440p and 1080p are kept; a 5v5 game or another resolution gets no scoreboard and can be reported only as Missed game. A kit that does not match its blanking checks is never put in a report. Training is off unless you tick the box or open the report from Send this fix to help training. Your fixes to the result, stats or hero go into the report as corrections next to the original reads, which stay unchanged. A map fix is not carried yet.

Other changes:

- The debug folders `debug/accepted` and `debug/rejected` are written only while Settings → Save debug images from each capture is on (or `debug_ocr` in config.toml). They hold full scoreboards with other players' names and stay on this PC. Existing folders are left alone; if one is there, the log says once at start that it is no longer written (#244).
- Restart now starts the app the update installed. An app opened while an update is installing never closes the running app in the middle of the install (#241, #245).
- The tracker turns off core dumps when it starts, so a crash never writes a memory image that could hold a scoreboard (#245).
- What's new shows its text without `**` and backtick marks (#241).
- Hero portraits learned for heroes with accents or punctuation in their names (Soldier: 76, Torbjörn, D.Va, Lúcio) are saved under plain names. Files alpha.5 saved under the old names are moved aside, never deleted (#243).
- Two OCR calls that ran on every end-of-match nameplate and whose result was thrown away are gone (#243).
- The release build stops instead of hanging when a package mirror does not answer (#241).

The readers are unchanged: the tracker still uses classic OCR for saved stats unless `reader = "new"` is set. A switch to read without OCR is planned for a later alpha, and its default will be decided by measurement.

Stable installs are not offered this alpha. The desktop app skips it until Settings → Get alpha updates is on (or config.toml has `update_channel = "prerelease"`). bootstrap.sh skips it until `STAT_TRACKER_CHANNEL=prerelease`. Pinning this tag still installs it.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.5.0-alpha.6/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v0.5.0-alpha.6 bash
```

Or extract the tarball and run `./install.sh`.

## 0.5.0-alpha.5

This alpha adds a Report a problem button that sends the tracker log to the site, and keeps your scoreboard row when the hero portrait cannot be matched.

### Highlights

- **Report a problem** is in the sidebar and in Settings. Pick Crash, Performance, or Other and add a note. Before anything leaves your computer, you see the exact log and files that would be sent. Send uses the tracker's existing site link. You can also save the zip to your Downloads folder instead.
- If the tracker stopped by itself, the app asks on the next start whether you want to report it.
- Your own row on the scoreboard is found again when the hero portrait does not match, for example while the ult ring covers it. Before, every later Tab read failed with "player row not found" and the game kept its early stats.
- Settings has a **Get alpha updates** toggle, so you no longer need to edit config.toml to get alphas.

The report log covers the last 30 minutes. After a crash the app noticed, it also covers the 30 minutes before the stop and 2 minutes after. Your sync token, player name, account name, computer name, home folder path, and OCR text are taken out of the log before it is shown. Officers see who sent a report. A site allows 5 reports per member per day. Nothing is retried on its own (#230, #232).

Stable installs are not offered this alpha. The desktop app skips it until Settings → Get alpha updates is on (or config.toml has `update_channel = "prerelease"`). bootstrap.sh skips it until `STAT_TRACKER_CHANNEL=prerelease`. Pinning this tag still installs it.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.5.0-alpha.5/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v0.5.0-alpha.5 bash
```

Or extract the tarball and run `./install.sh`.

## 0.5.0-alpha.4

This alpha adds a timing line to the tracker log, so a frame-rate drop while the tracker runs can be traced to the screen grab or to the reading.

### Highlights

- Once a minute while the game is open, `daemon.log` gets one `capture timing` line. It shows how long screen grabs, poll checks, and scoreboard reads took.
- Nothing else changes. Captures, reads, and uploads work as in alpha.3.

The line lists the capture backend (`x11`, `wayshot`, or `portal`) and, for each stage, the count, the typical time, the longest time, and the total for that minute (#224). Nothing is logged while nothing is captured, for example with the game closed. If you see a frame-rate drop, send that line with your report.

Stable installs are not offered this alpha. The desktop app skips it until config.toml has `update_channel = "prerelease"`. bootstrap.sh skips it until `STAT_TRACKER_CHANNEL=prerelease`. Pinning this tag still installs it.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.5.0-alpha.4/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v0.5.0-alpha.4 bash
```

Or extract the tarball and run `./install.sh`.

## 0.5.0-alpha.3

This alpha keeps a bad scoreboard read from wiping a stat you already had. A faded map name gets one brighter try, and a match that is still going shows on the Games tab with a note that the numbers are not final.

### Highlights

- Heroes land on the right name and role. Wrecking Ball is a Tank, including a one-time fix of games you already saved.
- Settings fits a tall portrait screen, and the form scrolls inside the window.
- Install stops when the checksum file is missing. `stat-tracker-gui --version` prints the tracker version and exits.
- The hero reader pack is chosen by name and swapped in safely. Sign-in and server errors say what happened, and the pack you already have stays put.

The new reader falls back one field at a time to the old reader (#214). A stat never drops because of a single bad read. If eliminations were 11 and one frame reads them as 1, the saved value stays 11.

Dim or faded map names, such as Lijiang Tower, get one brightened retry on scoreboard frames (#218).

On the Games tab, an open match shows In progress with the live stats and a note that they are not final (#217). When the match closes, the card goes back to the usual outcome. Settings uses the available height on a tall portrait display. `bootstrap.sh` refuses the install if the `.sha256` file is missing. The desktop app accepts `--version` and `-V`. The hero pack file is selected by name. The live heroes directory is swapped into place, and the previous pack is left alone if the download fails. A 401 says the sync token is not valid and to sign in again from Settings. A 403 says the site refused the reader pack. Check you're signed in with a member account. The installed pack was left in place. A 503 that says packs are not set up yet says the tracker still works and to try again later. Each game saves two local map-name images under `debug/mapcrops` so later checks can compare them with what was read. Those images stay on this machine. They are never uploaded and they are not included in reports.

Stable installs are not offered this alpha. The desktop app skips it until config.toml has `update_channel = "prerelease"`. bootstrap.sh skips it until `STAT_TRACKER_CHANNEL=prerelease`. Pinning this tag still installs it.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.5.0-alpha.3/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v0.5.0-alpha.3 bash
```

Or extract the tarball and run `./install.sh`.

## 0.5.0-alpha.2

This build is the next alpha. Signed-in members get the hero icon pack from the Scuffed Crew server.

### Highlights

- The tracker downloads that pack for signed-in members and checks it before using it, so hero names use the new reader for everyone.
- In the setup guide, Enter and Escape no longer act while you type in a box.
- The sync paused message has a small wording fix.

The New reader (alpha) switch still stays off until you turn it on. With this pack, hero names from that reader no longer need a file you copied into the data folder yourself.

A stable update is not offered this alpha. The desktop app skips it until config.toml has `update_channel = "prerelease"`. bootstrap.sh skips it until `STAT_TRACKER_CHANNEL=prerelease`. Pinning this tag still installs it.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.5.0-alpha.2/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v0.5.0-alpha.2 bash
```

Or extract the tarball and run `./install.sh`.

## 0.5.0-alpha.1

This build is an alpha. You can turn on a new scoreboard reader in Settings, and it stays off until you do.

### Highlights

- Settings has a New reader (alpha) switch (`reader` in config.toml). It stays off until you turn it on. When it is on, it reads numbers, the map, Victory or Defeat, and heroes. A field it is not sure about keeps the old reader's value, and the Games tab marks that field unsure.
- Hero names: the new reader needs the hero icon pack. In this alpha the pack isn't downloaded for you yet, so hero names come from the old reader unless the pack is already in your tracker's data folder. A members-only download comes in alpha.2.
- The first launch opens a setup guide: a screen capture check, Overwatch settings, sync sign-in by a code or a pasted token, and the reader pack.
- Settings can remove the tracker's own files and services. If a package manager installed this copy, Settings shows that package's remove command instead. Saved games are removed only if you tick that option.

The tracker reads `reader` when it starts, so restart it after you save. `ocr-v1` is the default when the key is missing. `new` stores the new reader's value for each field it read confidently on your own row, and keeps the old value for the rest. The map and mode already chosen for the game stay, including a Deathmatch game, which stays on this machine. If the new reader names a different map, Games marks the map unsure. If it cannot find the board, cannot tell the team size, finds the columns but reads no numbers, is not sure which row is yours, or does not replace any field, the game stays tagged with the old reader. A field it never tried is not marked unsure. Editing an unsure field clears the mark. Game start, game end, and the checks that hold a bad number do not change. Uploads send the reader name stored on the game, not whatever the switch says now.

Hero icon templates live in the data folder under `templates/heroes/`. Without them the new reader does not name heroes, and those names stay with the old reader.

The Extra number reader (test) switch is still there. It only writes a private log and does not change saved games or uploads on its own.

A stable update is not offered this alpha. The desktop app skips it until config.toml has `update_channel = "prerelease"`. bootstrap.sh skips it until `STAT_TRACKER_CHANNEL=prerelease`. Pinning this tag still installs it.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.5.0-alpha.1/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v0.5.0-alpha.1 bash
```

Or extract the tarball and run `./install.sh`.

## 0.4.24

Release notes now show inside the desktop app. The extra number reader,
still off unless you turn it on, reads a lone thin 4 correctly and marks
a right 3 or 8 as unsure much less often. It never changes saved games or
uploads.

### Highlights

- The notes sit in a collapsed update banner, open once after each update, and stay available from About.
- A cell shorter than the rest of its row takes that row's text band before the digit is split.
- Confidence is divided by that digit's usual margin, and every shadow log line includes "recognizer":"cv-v2".

The desktop app compiles this changelog into the binary, so the notes
work with no network. An update check leaves the banner collapsed: one
line with the new version, the first sentence of the newest summary, and
the same update actions as before. A What's new button expands the cards
for the offered version and any versions in between. The collapsed state
is the default whenever that banner is shown.

The notes also open on launch when the running version is not the one
last closed. That covers a tarball install, the AUR package, a .deb, and
an AppImage, because the check uses the version the GUI resolves at
runtime. Closing the dialog, or pressing Escape, stores the version in
the GUI state file so it does not open again until the version changes.
About in the sidebar opens the same dialog for every bundled release.
Each release is its own card: the version, the GitHub date when an
update check has one, and a badge when one applies (New, Installed, or
Update available). Only the latest three cards show until Show older
releases. The rest of a section, after the summary and the short list
above, stays behind Details and starts closed. Install steps stay on
the GitHub page. Links in the notes open in the browser for http and
https only.

The extra reader is still opt-in and log-only. It runs only with
`shadow_recognizer = true` in config.toml, or with
`SCUFFED_SHADOW_RECOGNIZER=1` for a single run. Settings does not save
that environment override into the file. A background thread reads each
accepted scoreboard and appends one line to
`<data dir>/shadow/digits.jsonl`. Stored stats, the capture gate, and
uploads are the same as with the reader off.

A lone thin 4 was measured too short. The stem is about 2 px wide and
the crossbar about 10 px, so the stem rows fall under the 25% ink cut
and are dropped. The glyph was then about 9 or 10 px tall instead of
13, wide enough to be split as touching digits. Live boards came back
as 16 and 311, and one history cell came back as 141. Cells in a row
share one font size and baseline. After each cell measures itself, the
row's lower median height, and the median top of the cells that agree
with it, is the reference. A cell whose own height is 2 px or more off
that reference takes the row's text band before the split and the canvas
scale. A cell that is taller than the row is corrected the same way.
Fewer than 4 segmented cells leaves each cell on its own measurement.
Templates, thresholds, and grammar are unchanged. On the labelled
sets the history Teams board goes from 287 of 288 cells to 288 of 288,
and the pasted thin 4s go from 1120 of 1152 to 1152 of 1152. No new
flags and no new errors. A wrong read is still not left unflagged.

The old confidence was the raw gap between the best template and the
runner-up, and that gap is not the same size for every digit. A correct
3, whose runner-up is always an 8, typically clears by about 0.12 and
can fall to 0.05, while a lone 1 sits near 0.43. One global cut at 0.06
kept flagging those right 3s and 8s. Each glyph's gap is now divided by
the typical gap for that class (digits 0 through 9, and the comma): the
median best-minus-second-best of correct reads, measured with these
templates on the labelled 1440p and 1080p Tab sets, 20 boards each. A
per-match hold-out of those boards moves no flag decision. The cell
score is the lowest glyph. It is also limited by how far the best
reading beats the best different reading, using `2 * gap / 0.2`, so
that limit sits on the same scale. A score of 1.0 means as clear as a
typical correct read of that digit. The suspect line is a calibrated
score below 0.35. Width, grammar, digit count, rival group, and low
score are unchanged.

A right 3 or 8 whose raw gap is 0.049 still clears 0.35 after that
division, on both template sizes. A 1 or a 7 at the same raw gap stays
flagged. The live misread 16, whose raw gap on the 6 was 0.024, stays
flagged too. On the labelled and stressed sets, the worst wrong read
the old cut caught scores 0.29 after calibration, so the 0.35 line
keeps every one of those errors flagged.

Logged scores are clamped to the range 0 to 1. A NaN becomes 0, which
is suspect, so it is not written out as a non-number.

Every line in `shadow/digits.jsonl` now includes
`"recognizer":"cv-v2"`. `cv-v1` is the 0.4.23 matcher (raw gaps, cut at
0.06). A line with no recognizer field is read as `cv-v1`, so a log
from before this release is never pooled with a log from after. Values
and scores from two ids are not compared as if they were one scale.

A snapshot test pins the id, the calibration constants (the 0.35 line,
the 0.2 divisor, and both typical-gap tables), and each fixture cell's
value and suspect flag. Scores are left out of that hash. Eight sample
cells must each stay within 0.02, each of the six fixture boards must
keep its mean within 0.005, and every fixture cell must sit at least
0.05 away from the 0.35 line. The fixtures are synthetic glyphs on a
flat background, resized and noised in a fixed way, at both template
sizes. There are no captured boards. The `cv-v1` history entry is the
0.4.23 matcher run on those same fixtures.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.4.24/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v0.4.24 bash
```

Or extract the tarball and run `./install.sh`.

## 0.4.23

You can turn on an extra number reader that only writes a private log on
this computer. It does not change your saved games or what gets uploaded,
and it stays off unless you enable it.

### Highlights

- Turn it on with shadow_recognizer = true in config.toml.
- For one run only, set SCUFFED_SHADOW_RECOGNIZER=1. That is not saved into the config file.
- The log is shadow/digits.jsonl in your data folder, and it rolls over around 4 MB.
- The extra read stays on a side thread and stops if one board takes longer than 300 ms, so a capture is not held up.

Adds an optional shadow digit reader, off by default. When it is on
(`shadow_recognizer = true` in config.toml, or
`SCUFFED_SHADOW_RECOGNIZER=1` for one run), a background thread reads
each accepted scoreboard with a template digit matcher and writes the
values and confidences to a local log at
`<data dir>/shadow/digits.jsonl` (rotated, about 4 MB at most). It
never changes stored stats, the capture gate, or uploads, and nothing
from it leaves the machine. Settings does not save the environment
override into config.toml.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.23`.

## 0.4.22

Map names are less likely to be mixed up, including the two Watchpoint
maps and a couple of event maps. Long games are no longer closed too
early, and uploading a result no longer freezes the tracker.

### Highlights

- An unclear Watchpoint name is dropped. Grímsvötn still wins when that name is readable, even if the accents are missing.
- Adlersbrunn is saved as Eichenwalde when both teams have stats. Château Guillard games stay on this computer and are skipped in totals and history.
- A match with no result can run longer than 20 minutes. It closes after 6 hours without a result, or when the next game starts.
- Choosing Victory, Defeat, or Draw queues an upload in about 3 seconds, and Tab still works while that send is in progress.

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
name still reads back. `LIJIANG TOWER` is on its own line four times,
plus one title-case line, so the IJ pair is not dropped. `LIANG TOWER`
and `LULANG TOWER` still canonicalize to Lijiang Tower.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.22`.

## 0.4.21

A faint 0 on the scoreboard was sometimes missed, which shoved the other
numbers into the wrong columns. Those zeros are read now, and a timer on
the same line is no longer treated as a stat.

### Highlights

- Those zeros count in every stat column, not only one of them.
- A clock like 00:02 on the player line is skipped, so it cannot push assists into the damage figure.
- If the gap between rows cannot decide 5v5 or 6v6, that capture is thrown out instead of moving every player one slot.

A zero on the scoreboard is drawn fainter than the other digits. The
cell reader was dropping those cells, and one empty cell threw away the
whole row. Those Tabs were often saved anyway, through the text
fallback, with the columns shifted. Those zeros now read as 0 in any
of the six stat columns. A row that is on screen but whose cells still
cannot be read is logged separately from a frame where the player row
was not found.

That check is two tests, not one grey level. The stroke has to be
neutral grey, because purple and yellow row fills are strongly
saturated and the soft edge of a glyph picks that colour up. It also
has to be clearly darker than white text. On the Dorado cells the
stroke cores are about 171-186 and white digits reach 250-255. When
the bright mask keeps the stroke but Tesseract returns nothing, the
same check still reads the ring as 0.

The text fallback no longer takes the last six numbers on the player's
line. The name has to match as a whole word. That line can also contain
the hero panel's objective timer, and a trailing `00:02` shifted the
columns (assists and deaths became the damage and healing figures). The
fallback now ignores a clock, keeps a line only when the stats are one
unbroken run of exactly six numbers with no word or percent after that
run, and uses the same elims, assists, and deaths ceilings as the cell
reader. A chat line and a join line are skipped so a later stat line
can match. Numbers from that fallback are low-trust: the first capture
is checked too, and one clean cell read replaces an unconfirmed column
when that read is not a clip below a value the fallback moved off, and
not a wide column with its last digit cut off. A jump past the rate
cap, or a trailing-digit inject, is still held. A confirmed value,
including one a fallback moved away from, still has to agree three
times before it moves down.

A recovered 0 now carries a confidence from how cleanly the ring
matched (hole size and how centred it is), from 55 for a ring that only
just passes up to 95, instead of a flat 60. A clean ring scores 75 or
more. A ring touching the cell edge loses 15 and is still marked
suspect.

The bottom player row is cut short at the edge of the scoreboard crop,
so its cells get a slightly smaller upscale. Tesseract sometimes reads
nothing at that size where it read the digit before, so an empty read
there is tried once more at the old size. Column calibration scores
each candidate layout on bright digits only, so recovered zeros cannot
push it to a layout the old build did not pick.

5v5 or 6v6 no longer trusts one row-pitch measurement that fits
neither layout. On a post-game table the stronger measurement said
5v5 while the rows were 6v6, and every row was read one slot off. A
pitch that fits neither layout is ignored, and when the two
measurements point at different sizes the capture is rejected (saved
to `debug/rejected`) instead of read with shifted rows.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.22`.

## 0.4.20

The last game of a session uploads on its own a few minutes after the
result, instead of waiting until the next game or a shutdown. Watchpoint:
Grímsvötn is stored as its own map, not as Watchpoint: Gibraltar.

### Highlights

- The quiet wait is about 3 minutes from the last activity, and never shorter than 75 seconds after the result.
- A Tab during those 75 seconds still counts toward the game that just ended.
- Grímsvötn is recognized with or without accents, and with or without the Watchpoint prefix.
- A game with no result that sits idle for 20 minutes is closed as Unknown and not uploaded (0.4.22 removed this timeout).

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

The tracker is better at telling one match from the next, so a result
and the stats after it stay on the right game. It no longer uses a short
timer to decide that a new match has started.

### Highlights

- The 75 second grace starts when the result is saved, not when the result word first shows up.
- Changing heroes in the middle of a match does not open a new game by itself.
- A later Tab with a different map name can end the current game, once that game already has a map.
- The first board of a new game has to be at least 45 seconds after the old one, and it stays off the old game until a second board agrees.

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
Still on prior polish / packaging from 0.4.1-0.4.12.

Daemon OCR / capture / sync / store schema are unchanged. In-app
Update now (0.4.8), Settings Maps-level polish (0.4.7), Maps-grammar
Seasons grid (0.4.6), Settings/Maps/Games polish (0.4.5), companion
overlay hotkey (0.4.4), and packaging hotfixes 0.4.1-0.4.3 are
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
prior polish / packaging from 0.4.1-0.4.11.

Daemon OCR / capture / sync / store schema are unchanged. In-app
Update now (0.4.8), Settings Maps-level polish (0.4.7), Maps-grammar
Seasons grid (0.4.6), Settings/Maps/Games polish (0.4.5), companion
overlay hotkey (0.4.4), and packaging hotfixes 0.4.1-0.4.3 are
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
overlay hotkey (0.4.4), and packaging hotfixes 0.4.1-0.4.3 are
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
Phrase OCR is optional and log-only, the cheap color gate wakes.
Calibrated on a real 2560×1440 card.

0.4.9 already shipped on-hit poll debug PNGs (#80) and POTG / end-reel
letterbox wake (#81). Confirm rules are unchanged.

Daemon OCR / capture / sync / store schema are unchanged. In-app
Update now (0.4.8), Settings Maps-level polish (0.4.7), Maps-grammar
Seasons grid (0.4.6), Settings/Maps/Games polish (0.4.5), companion
overlay hotkey (0.4.4), and packaging hotfixes 0.4.1-0.4.3 are
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
overlay hotkey (0.4.4), and packaging hotfixes 0.4.1-0.4.3 are
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
and packaging hotfixes 0.4.1-0.4.3 are unchanged.

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
hotfixes 0.4.1-0.4.3, companion overlay hotkey (0.4.4), Settings/Maps/Games
polish (0.4.5), and Maps-grammar Seasons grid (0.4.6) are unchanged.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```

Or extract the tarball and run `./install.sh`. Pin with
`STAT_TRACKER_TAG=stat-tracker-v0.4.7`.

## 0.4.6

Seasons Maps-grammar grid (#74): season cards use the same 2-4 column
layout as Maps, with a big WR, win/loss stripe, and FillPortion bar.

Settings denser cards (#74): surface cards with a 1-2 column field grid.
Companion hotkey setting is unchanged.

Daemon OCR / capture / sync / store schema are unchanged. Packaging
hotfixes 0.4.1-0.4.3, companion overlay hotkey (0.4.4), and prior
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

Maps visual polish (#72): compact 2-4 column map cards with a WR bar and
win/loss stripe.

Daemon OCR / capture / sync / store schema are unchanged. Packaging hotfixes
0.4.1-0.4.3 and the companion overlay hotkey (0.4.4) are unchanged.

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
daemon Tab capture, not X11 `XGrabKey`). Needs the `input` group or seat
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
- AerynOS may not ship it, the main window still works.

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
- **Companion overlay**: layer-shell panel (`stat-tracker-gui --companion`)
  that sits above fullscreen Overwatch while the game process is running.
- **Dioxus GUI removed** (P5 / #61). The daemon crate is daemon-only; Iced is
  the sole desktop UI.
- **Reinstall keeps the `stat-tracker-gui` binary name.** Desktop entry, PATH,
  and tarball layout are unchanged. Run `./install.sh` (or the bootstrap
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
