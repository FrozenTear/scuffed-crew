# Hand-edit a reader pack

This guide is for a member who wants to change a reader pack on their own
PC: replace a hero icon, tweak a letter tile, or edit a rank or queue-title
`templates.json`. Official packs still come from the site. The tracker
downloads them and unpacks them under its data folder. Nothing in this
guide is about publishing a pack to the server. For that, see
[Recognizer asset packs](tracker-packs.md).

## Stop and start the tracker

On a standard install, the tracker runs as the
`scuffed-stat-tracker.service` user unit, and Settings says "Runs as a
login service." If Settings says "No login service installed", there is
no unit: the app starts the tracker itself. Either way, closing the app
window does not stop the tracker.

- From a terminal, with the login service only:
  `systemctl --user stop scuffed-stat-tracker.service`, then
  `systemctl --user start scuffed-stat-tracker.service` (or `restart` to
  do both).
- In the app, with or without the login service: **Stop** on the tracker
  status row in Settings, then **Start tracker** (or **Restart tracker**)
  on the Settings status strip. On the older Settings layout, the
  **Tracker service** card has **Stop**, **Restart** and **Start**.

"Stop the tracker" and "start the tracker" below mean one of these.

### Where the tracker log is

- With the login service, the log goes to the journal:
  `journalctl --user -u scuffed-stat-tracker.service`.
- With "No login service installed", the log is `daemon.log` in the data
  folder.

"The tracker log" below means whichever of these applies to you.

## Where the packs live

The data folder is `~/.local/share/scuffed-stat-tracker`. It is
`$XDG_DATA_HOME/scuffed-stat-tracker` only when the tracker's own
environment sets `XDG_DATA_HOME`, which the user unit normally does not,
whatever your shell has. If `config.toml` sets `data_dir` to an absolute
path, the packs are under that path instead. Below, `<data_dir>` means
that folder.

Each pack lands in a folder under `<data_dir>/templates/`:

| Pack | Folder under the data dir |
| --- | --- |
| `heroes-v1.tar` | `templates/heroes/` |
| `heroes-v2.tar` | `templates/heroes-v2/` |
| `heroes-v2-1080.tar` | `templates/heroes-v2-1080/` |
| `bans-v1.tar` | `templates/bans-v1/` |
| `bans-picks-v1.tar` | `templates/bans-picks-v1/` |
| `maps-v1.tar` | `templates/maps-v1/` |
| `words-v1.tar` | `templates/words-v1/` |
| `names-v1.tar` | `templates/names-v1/` |
| `rank-v1.tar` | `templates/rank-v1/` |
| `queue-title-v1.tar` | `templates/queue-title-v1/` |

The tracker keeps three record files in `<data_dir>/templates/`:

- `pack-versions.json`: the installed version of each pack.
- `pack-file-hashes.json`: for `rank-v1` and `queue-title-v1` only, the
  sha256 of the installed `templates.json`.
- `pack-manifests.json`: part of the `names-v1` install record.

Do not edit those record files by hand. The tracker writes them after an
install.

## What sits in each folder

Keep the same file names the reader already uses. The maps and words
readers check names and layout files in the live folder when they load
it, and can refuse the whole folder. The hero reader loads any `.png` at
the top of a hero folder, so a stray file there is used, not refused.
Details per pack follow.

**Hero packs** (`heroes-v1`, `heroes-v2`, `heroes-v2-1080`): one
`<hero-key>.png` per hero (for example `ana.png`, `wrecking-ball.png`),
and optional files in `special/` for the placeholder, skull and empty
slots. The reader treats these as PNG images. It uses the alpha channel
as the foreground mask when the file has one. The exact pixel size of
each official portrait is not fixed in the pack layout. Keep the same
size as the file you replace.

Every `.png` at the top of a hero folder is loaded as a class, named after
the file (placeholder, skull and empty icons are special classes, not
heroes). Keep no other `.png` there, not even a backup such as
`ana-old.png`: store backups outside the `templates/` folder. In
`special/`, only files whose names start with `placeholder`, `skull` or
`empty` are used; other names there are ignored.

**Ban icons** (`bans-v1`): top-level `<hero-key>.png` only. No reader
opens this folder yet.

**Bans and picks** (`bans-picks-v1`): one 100 by 96 `<hero-key>.png` for
each of the 54 heroes, and an optional `no-preference.png`. A missing
hero file, or a PNG that is not 100 by 96, refuses the whole folder.

**Map glyphs** (`maps-v1`): `atlas.json` and `g<4 hex digits>.png` (for
example `g003a.png`). The hex digits must be lowercase (`0-9`, `a-f`).
`atlas.json` names the glyph files the reader loads, and its `height`
must be 32. Each glyph PNG must be 32 pixels high. A hand-installed
`templates/maps/` folder is used only when `templates/maps-v1/` is
missing or does not load.

**Result words** (`words-v1`): `<word_id>.png` and `<name>.json` files,
for example `accolade_victory.png`, and an optional `layouts.json`. A
hand-installed `templates/result/` folder is used only when
`templates/words-v1/` is missing or does not load.

**Names** (`names-v1`): `manifest.json`, `glyphs.png`, `glyphs.txt`,
`marks.png`, `marks.txt`, `compose.txt` and `whiten.f32`. This is the
only pack that keeps its inner `manifest.json` in the live folder. The
loader checks every file's size and sha256 against that manifest. A
letter tile is 16 by 28 pixels. A mark tile is 16 by 8. If you change a
file, update that file's `size` and `sha256` in `manifest.json` too, or
the whole pack is refused and names are read the Tesseract way, while
OCR is on.

**Rank** (`rank-v1`): one `templates.json` and nothing else. It must be
JSON the rank reader can load: the `recognizer` field must be `rank-v1`,
the `rank_progress` gate class must be present, and each template vector
must have the length its family uses.

**Queue title** (`queue-title-v1`): one `templates.json` and nothing
else. It must be JSON the queue title reader can load: `width` 120,
`height` 35, at least one class, and each class vector 4200 values
(120 times 35).

### Rank and queue title readers are off by default

Both readers start off. With them off, `templates.json` is never loaded,
so an edit there changes nothing.

Each one is turned on by an environment variable in the tracker's service
environment:

- `STAT_TRACKER_RANK_RECOGNIZER=on` for the rank reader.
- `STAT_TRACKER_QUEUE_TITLE_RECOGNIZER=on` for the queue title reader.

`1`, `true` and `yes` also count as on. For the user unit, set them in a
drop-in: `systemctl --user edit scuffed-stat-tracker`, then add an
`Environment=` line under `[Service]`, and restart the tracker. With "No
login service installed", the drop-in does not apply: the tracker gets
its environment from the app, so set the variables in the environment
you start the app from, then restart the tracker.

Settings also has a switch, **Read my rank after Comp games**. When the
tracker reads that switch, it turns on both the rank reader and the queue
title reader, and the two environment variables still override it. If
the switch shows "Not read by this build yet, so this changes nothing
for now.", it does nothing, and only the environment variables turn the
readers on.

## Edit a template safely

1. Stop the tracker first (see [Stop and start the tracker](#stop-and-start-the-tracker)).
   Most readers load a pack only when the tracker starts. A download of
   an official pack only runs on the next pack sync: a tracker start, or
   the setup guide's **Download reader pack** step. Open the setup guide
   from Settings, **Open setup guide**. That step is the last one, and you
   can skip the capture steps before it.
2. Copy the folder, or at least the file you will change, to a backup
   outside `<data_dir>/templates/`. A backup `.png` left inside a hero
   folder is loaded as a hero. The tracker itself keeps a copy only for
   `rank-v1` and `queue-title-v1`, and only when an install is about to
   replace that folder. Every other pack is replaced without a keep.
3. Replace or edit the file in the live folder. Keep the file name, the
   format the reader already expects, and, for an image, the same size
   as the official file you are replacing.
4. Start the tracker again. See [When the reader uses your edit](#when-the-reader-uses-your-edit).

With `ocr = "off"` in `config.toml`, the tracker turns classic OCR off
only when all of these load:

- a main hero set: `heroes-v1`, or a complete `heroes-v2`
  (`heroes-v2-1080` alone does not count);
- `maps-v1` itself (a hand-installed `templates/maps/` does not count);
- `words-v1`, with VICTORY and DEFEAT for both the accolade and rank
  screens, every match-start title, and the end-title templates. An edit
  that breaks one of those title templates keeps OCR on, even if the rest
  of the folder still loads.

If one of those is missing at start, the install is linked, and startup
got the site's pack list, the tracker waits up to 45 seconds for the
download, then keeps OCR on for that run if it is still missing. In any
other case there is no wait: OCR stays on for that run. If a pack goes
away while the tracker runs, what that pack reads is held until it is
back.

### What a bad file does

It depends on the pack.

- **Hero packs.** A file that is not a readable PNG is skipped and
  logged at debug. In `heroes-v1`, the rest of the folder still loads,
  and a missing or skipped hero is not matched. `heroes-v2` and
  `heroes-v2-1080` are used only when they have every hero and special
  class that `heroes-v1` has. One missing or unreadable hero or special
  class in a v2 pack means the whole v2 pack is not used, with the WARN
  line "hero pack incomplete, not used". The 1440p reader then uses
  `heroes-v1`, and the 1080p reader uses the 1440p set. An extra
  `.png` in `templates/heroes/` (for example `ana-old.png`) is a hero
  the v2 packs do not have, so it turns off both v2 packs. An extra
  `.png` in a v2 folder is a made-up hero the reader can return.
- **Map glyphs.** The whole `maps-v1` folder is refused when
  `atlas.json` is missing or is not JSON, when it lacks one of `height`,
  `gap`, `space` or `glyphs`, when its `height` is not 32, when it names
  a file that is not `g`, 4 lowercase hex digits and `.png`, or when none
  of the glyphs it names load. A refused `maps-v1` falls back to
  `templates/maps/` if you have one. Files `atlas.json` does not name
  are ignored. A named glyph that
  is missing, unreadable or not 32 pixels high is skipped without a
  message, and maps that need that glyph get no template.
- **Result words.** A `layouts.json` that cannot be read or parsed, that
  has an unsafe id, or that has a region outside the game area refuses
  the folder. A missing `layouts.json` is fine: the built-in layouts are
  used. The folder is used only if at least one victory and one defeat
  template load. Otherwise the reader falls back to `templates/result/`.
- **Bans and picks.** A missing hero PNG, or one that is not 100 by 96,
  refuses the whole folder. The reader does not run until the folder
  loads.
- **Names.** A missing, unreadable or hash-mismatched file refuses the
  whole pack. Names are then read the Tesseract way, while OCR is on.
- **Rank and queue title.** With the reader on, a `templates.json` that
  is missing, not JSON, or that fails the checks above does not load.
  That reader stays off for the run. Pack sync does not replace a live
  folder just because the file is bad: if the recorded version still
  matches the site list and the file is present, the folder is left as
  it is. Remove the folder to get the official file back, as in
  [Get back to the official pack](#get-back-to-the-official-pack).
- **A new official pack the site sends.** Before it is swapped in, a
  `rank-v1` or `queue-title-v1` pack is loaded with that reader's
  template loader. If it does not load, it is not saved. The message is
  "The reader pack did not load, so it was not saved." The live folder
  stays as it was. Other packs are not load-checked this way before the
  swap; a bad official download is still refused if its size, sha256 or
  layout is wrong.

## How the tracker treats a hand edit

Pack sync runs when the tracker starts, and when the setup guide runs its
reader pack step. A pack is downloaded again only when its live folder is
not in place, or when the version on the site list differs from the one
in `pack-versions.json`. "In place" means the folder is there. For
`names-v1` it also needs `manifest.json`. For `rank-v1` and
`queue-title-v1` it also needs `templates.json`. Changing a file inside
a folder that is still in place does not, by itself, make sync download
the pack again.

### Rank and queue title: your folder is kept

These two packs are the only ones the tracker keeps.

After each successful install, the tracker records the sha256 of the
live `templates.json` in `templates/pack-file-hashes.json`. Each time an
install is about to replace an existing folder (a new official version,
or the same version again after the folder stopped being in place), the
tracker looks at the live folder. It keeps the folder unless it is
exactly one regular file named `templates.json` whose sha256 is the
recorded one.

So a folder you placed by hand, one you edited, one with an extra file,
an empty one, or one installed before this hash was recorded, is kept.
"Kept" means the whole live folder is moved to

`<data_dir>/templates-kept/<pack folder>-<UTC YYYYMMDDTHHMMSSZ>-<pid>/`

for example `templates-kept/rank-v1-20261114T065156Z-4242/`. That sits
beside `templates/`, not inside it. When that name is already taken,
`-2`, `-3` and so on are added. When the tracker ran the sync, the move
is logged in the tracker log with the WARN line "reader pack folder that
was not the one sync installed was moved aside", and the kept path. When
the setup guide ran it, that line goes to the app's own output instead,
and the setup guide shows only the hero pack result and "Extra reader
packs ready: N of M." Either way, the next tracker start logs where kept
folders are (see [Clean up `templates-kept/`](#clean-up-templates-kept)).

The new official pack then becomes the live folder, and its
`templates.json` hash is recorded. Your edited copy is the one in
`templates-kept/`. It is not used until you copy it back yourself.

If the kernel allows an atomic directory exchange (`RENAME_EXCHANGE`),
the new pack is swapped into the live path first, and the old folder is
moved out after, so the live path is never missing:

1. The old folder is moved to `templates-kept/`.
2. If that fails, it is moved beside the live folder under `templates/`,
   with the same kept name (no leading dot).
3. If that fails too, the two folders are exchanged back: your folder is
   live again, and the official pack is not saved. The reason given is
   "The reader pack was not saved: the old pack folder could not be
   moved to templates-kept, so it was left in place."
4. If the exchange back fails, the official pack stays live, and the old
   folder is again tried beside the live folder under a free kept name.
5. Only if that fails as well, the old folder is left at its hidden
   staging path (`.rank-v1-next-...` or `.queue-title-v1-next-...` under
   `templates/`). The reason given then says it was left at the staging
   path, and that a later sync moves it to `templates-kept/`. It is not
   removed.

A later sync moves such a staging folder to `templates-kept/` (or beside
the live folder) once the folder's own modified time is an hour old. The
old folder keeps its old modified time, so that is usually the very next
sync. The check runs only at the start of a sync run.

If `templates-kept/` cannot be created at all, the official pack is
refused with the "not saved ... templates-kept" reason, and your folder
stays as it was.

Without the exchange, the old folder is moved to `templates-kept/`
first, then the new one is put in place, and moved back if that fails.

An unchanged official folder (exactly one `templates.json` with the
recorded hash) is replaced, and nothing is kept.

In the tracker log, these reasons are the `reason` field of the pack
job's warning. The setup guide does not show them.

### Other packs: a new official version replaces the folder

Hero, map, word, name, ban and bans-picks packs do not use
`pack-file-hashes.json` and are not moved to `templates-kept/`.

If the site still lists the same version you have recorded, sync leaves
the folder alone, so your edited files stay. If the site publishes a new
version, the folder is swapped out and the old tree is removed. If that
cleanup fails, the old tree is left as a hidden folder under
`templates/`, named after the live folder: `.<folder>-prev-...` or
`.<folder>-next-...`, for example `.heroes-prev-...` for `heroes-v1`. A
later sync deletes it once its modified time is an hour old. Your edit is
not kept. Copy the folder yourself before you edit one of these packs if
you want a backup.

## When the reader uses your edit

Restart the tracker after any edit. That is the reliable rule for every
pack.

- **Hero, map and result-word packs.** The new reader checks each pack
  folder's own stamp (device, inode and modified time) on a later board,
  and reloads in the background when the stamp changes. A folder's
  modified time changes only when a file at its top level is created,
  deleted or renamed. So:
  - Adding, deleting or renaming a file at the top of the folder (for
    example writing `ana.png.new` and renaming it over `ana.png`) is
    picked up by the new reader on a later board.
  - Overwriting an existing file in place (for example `cp new.png
    ana.png`, or an editor that saves in place) does not change the
    stamp. It is used from the next start.
  - Any change inside `special/` does not change the hero folder's
    stamp. It is used from the next start.
  - The classic hero portrait matcher and the shadow hero log load the
    hero packs once per start. They use any hero edit only from the next
    start.

  A folder that changes while it is loading is not used; the load is
  tried again later.
- **Names.** The pack is loaded when the tracker starts. After a pack
  sync install, it is reloaded in the background once the folder matches
  the install records. A folder you change by hand, with no matching
  install record, is left until the next tracker start. At that start
  the folder is loaded from disk, including your edits, if
  `manifest.json` still matches the files.
- **Rank and queue title.** Only when that reader is on (see
  [Rank and queue title readers are off by default](#rank-and-queue-title-readers-are-off-by-default)).
  The pack is loaded once when the tracker starts. An edit, or a pack
  that arrived while the tracker was running, is used from the next
  start.
- **Bans and picks.** The pack is loaded on the first game read of that
  run, then kept. An edit after that first load is used from the next
  start.

## Get back to the official pack

Do this when your edited files are still in the live folder and you want
the site's pack again. The pack must still be on the site's list, or
nothing is installed.

1. Stop the tracker. The background pack job ends once every pack is
   current, and it does not look again until the next tracker start or
   the setup guide's reader pack step. Deleting a folder while the
   tracker is already idle will not fetch the official pack until one of
   those runs.
2. Remove the whole live pack folder, for example
   `<data_dir>/templates/rank-v1/`. Move it outside `templates/` first if
   you want to keep your copy. For `rank-v1` and `queue-title-v1`, do not
   delete only `templates.json`: the folder left behind is not the one
   sync installed, so it is moved to `templates-kept/` with a WARN line.
   That is harmless but confusing. You do not need to edit
   `pack-versions.json` or `pack-file-hashes.json`. They can still name
   the old install after the folder is gone; the next sync sees the
   missing folder and downloads the pack again.
3. Get the pack back while the install is linked to a site account (pack
   sync needs the daemon token). Pick one:
   - With the tracker still stopped, open the setup guide (Settings,
     **Open setup guide**) and run its last step, **Download reader
     pack**. You can skip the capture steps before it. Then start the
     tracker. The pack is in place before the tracker starts, so every
     reader loads it at that start.
   - Or start the tracker, wait for the `reader pack installed` line for
     that pack in the tracker log, then restart the tracker once more. At
     start the tracker downloads only `heroes-v1` before the readers load.
     The other packs come from a background job, and the rank and queue
     title readers load before that job can finish, so they use the
     official `templates.json` only from the restart.

With `ocr = "off"`, removing a pack the tracker needs to turn OCR off
(see [Edit a template safely](#edit-a-template-safely)) means the next
start keeps OCR on for that run if the pack is still missing, after a
wait of up to 45 seconds when the install is linked and startup got the
pack list. Removing `heroes-v2` while `heroes-v1` loads does not count:
the main hero set is still there. The first way above avoids all of
that.

If a newer official pack already replaced your edit, the live folder is
already the official one. Your copy is under `templates-kept/` (or, in
the rare cases above, beside the live folder or at a staging path).
`templates-kept/` can also hold an official folder that a killed sync
left behind, so pick the folder by the timestamp in its name and check
its contents before you copy it back. The tracker does not copy a kept
folder back into the live path by itself.

## Clean up `templates-kept/`

Nothing removes `templates-kept/`: not pack sync, not the app, and not a
purge. `scuffed-stat-tracker-uninstall --purge`, and the app's uninstall
with "Also delete saved games and settings", delete only the names the
tracker writes, and neither `templates` nor `templates-kept` is one of
them.

At each tracker start that runs pack sync (the install is linked to a
site account and its sync token was not rejected and has not expired),
when `templates-kept/` holds folders, or a kept folder sits beside a live
pack folder under `templates/`, the tracker logs the `templates-kept/`
path once in the tracker log, with both counts.

Delete `templates-kept/` by hand when you no longer need what is in it.
A kept folder is only a backup. Removing it does not change the live
pack.

## Privacy

The pack folders under `templates/` and the copies under
`templates-kept/` stay on this PC. Pack sync only downloads packs from
the site. No pack-sync path uploads those folders, and the uninstall
purge leaves them in place so a hand-edited copy is not deleted by
mistake.

## See also

The operator guide, [Recognizer asset packs](tracker-packs.md), covers
building, publishing, rollback, and the same keep rules in more detail.
