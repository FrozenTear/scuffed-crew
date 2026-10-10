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
