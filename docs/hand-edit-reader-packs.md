# Hand-edit a reader pack

This guide is for a member who wants to change a reader pack on their own
PC: replace a hero icon, tweak a letter tile, or edit a rank or queue-title
`templates.json`. Official packs still come from the site. The tracker
downloads them and unpacks them under its data folder. Nothing in this
guide is about publishing a pack to the server. For that, see
[Recognizer asset packs](tracker-packs.md).

Every step below is what the tracker on this branch does. If a detail was
not checked in the code, it is marked.

## Where the packs live

The default data folder is `~/.local/share/scuffed-stat-tracker`. If
`config.toml` sets `data_dir` to an absolute path, the packs are under
that path instead.

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

A record of the installed pack versions is
`<data_dir>/templates/pack-versions.json`. For `rank-v1` and
`queue-title-v1` only, the sha256 of the installed `templates.json` is
also in `<data_dir>/templates/pack-file-hashes.json`. Do not edit those
record files by hand. The tracker writes them after an install.

## What sits in each folder

Keep the same file names the reader already uses. A name the layout does
not allow is refused only when a pack is unpacked from the site, not when
you edit a file that is already in the live folder.

**Hero packs** (`heroes-v1`, `heroes-v2`, `heroes-v2-1080`): one
`<hero-key>.png` per hero (for example `ana.png`, `wrecking-ball.png`),
and optional files in `special/` such as `placeholder`, `skull` and
`empty`. The reader treats these as PNG images. It uses the alpha channel
as the foreground mask when the file has one. The exact pixel size of
each official portrait is not fixed in the pack layout. Keep the same
size as the file you replace.

**Ban icons** (`bans-v1`): top-level `<hero-key>.png` only. No reader
opens this folder yet.

**Bans and picks** (`bans-picks-v1`): one 100 by 96 `<hero-key>.png` for
each of the 54 heroes, and an optional `no-preference.png`. A missing
hero file, or a PNG that is not 100 by 96, refuses the whole folder.

**Map glyphs** (`maps-v1`): `atlas.json` and `g<4 hex digits>.png` (for
example `g003a.png`). A path `atlas.json` does not name, or a glyph file
that is not one of those names, refuses the pack when it is loaded.

**Result words** (`words-v1`): `<word_id>.png` and `<name>.json` files,
for example `accolade_victory.png`. A hand-installed
`templates/result/` folder is used only when `templates/words-v1/` is
missing or does not load.

**Names** (`names-v1`): `manifest.json`, `glyphs.png`, `glyphs.txt`,
`marks.png`, `marks.txt`, `compose.txt` and `whiten.f32`. This is the
only pack that keeps its inner `manifest.json` in the live folder. The
loader checks every file's size and sha256 against that manifest. A
letter tile is 16 by 28 pixels. A mark tile is 16 by 8. If you change a
file, update that file's `size` and `sha256` in `manifest.json` too, or
the whole pack is refused and names fall back to Tesseract.

**Rank** (`rank-v1`): one `templates.json` and nothing else. It must be
JSON the rank reader can load: the `recognizer` field must be `rank-v1`,
the `rank_progress` gate class must be present, and each template vector
must have the length its family uses.

**Queue title** (`queue-title-v1`): one `templates.json` and nothing
else. It must be JSON the queue title reader can load: `width` 120,
`height` 35, at least one class, and each class vector 4200 values
(120 times 35).

## Edit a template safely

1. Quit the tracker first. Some readers only load a pack when the daemon
   starts, and a restore only runs on the next pack sync (see below).
2. Copy the folder, or at least the file you will change, to a backup
   somewhere else. The tracker itself keeps a copy only for `rank-v1`
   and `queue-title-v1`, and only later, when a new official pack is
   about to replace that folder. Every other pack is replaced without a
   keep.
3. Replace or edit the file in the live folder. Keep the file name, the
   format the reader already expects, and, for an image, the same size
   as the official file you are replacing.
4. Start the tracker again. See [When the reader uses your edit](#when-the-reader-uses-your-edit).

### What a bad file does

It depends on the pack.

- **Hero packs.** A file that is not a readable PNG is skipped and
  logged at debug. The rest of the folder still loads. A missing or
  skipped hero is not matched from that pack.
- **Bans and picks.** A missing hero PNG, or one that is not 100 by 96,
  refuses the whole folder. The reader does not run until the folder
  loads.
- **Names.** A missing, unreadable or hash-mismatched file refuses the
  whole pack. Names are then read the Tesseract way, while OCR is on.
- **Rank and queue title.** A `templates.json` that is missing, not JSON,
  or that fails the checks above does not load. That reader stays off
  for the run. Pack sync does not replace a live folder just because the
  file is bad: if the recorded version still matches the site list and
  the file is present, the folder is left as it is. Remove the folder
  (or `templates.json`) to get the official file back, as in
  [Get back to the official pack](#get-back-to-the-official-pack).
- **A new official pack the site sends.** Before it is swapped in, a
  `rank-v1` or `queue-title-v1` pack is loaded with that reader's
  template loader. If it does not load, it is not saved. The message is
  "The reader pack did not load, so it was not saved." The live folder
  stays as it was. Other packs are not load-checked this way before the
  swap; a bad official download is still refused if its size, sha256 or
  layout is wrong.

## How the tracker treats a hand edit

Pack sync runs when the daemon starts, and when the setup guide runs its
reader pack step. A pack is downloaded again only when its live folder is
not in place, or when the version on the site list differs from the one
in `pack-versions.json`. "In place" means the folder is there. For
`names-v1` it also needs `manifest.json`. For `rank-v1` and
`queue-title-v1` it also needs `templates.json`. Changing a file inside
a folder that is still in place does not, by itself, make sync download
the pack again.

### Rank and queue title: the official file is not overwritten

These two packs are the only ones the tracker keeps.

After each successful install, the tracker records the sha256 of the
live `templates.json` in `templates/pack-file-hashes.json`. The next
time a new official version is about to replace that folder, the tracker
looks at the live folder. It keeps the folder unless it is exactly one
regular file named `templates.json` whose sha256 is the recorded one.

So a folder you placed by hand, one you edited, one with an extra file,
or one installed before this hash was recorded, is kept. "Kept" means
the whole live folder is moved to

`<data_dir>/templates-kept/<pack folder>-<UTC YYYYMMDDTHHMMSSZ>-<pid>/`

for example `templates-kept/rank-v1-20261114T065156Z-4242/`. That sits
beside `templates/`, not inside it. When that name is already taken,
`-2`, `-3` and so on are added. The path is written in the daemon log as
soon as the folder is moved.

The new official pack then becomes the live folder, and its
`templates.json` hash is recorded. Your edited copy is the one in
`templates-kept/`. It is not used until you copy it back yourself.

If the kernel allows an atomic directory exchange (`RENAME_EXCHANGE`),
the new pack is swapped into the live path first, and the old folder is
moved out after, so the live path is never missing. If that move to
`templates-kept/` fails, the old folder is tried beside the live folder
under `templates/` with the same kept name (no leading dot). If that
fails too, the two folders are exchanged back: your folder is live
again, the official pack is not saved, and the message is "The reader
pack was not saved: the old pack folder could not be moved to
templates-kept, so it was left in place." If even the exchange back
fails, the official pack stays live and the old folder is left at its
hidden staging path (`.rank-v1-next-...` or `.queue-title-v1-next-...`
under `templates/`). The message then says the old folder was left at
the staging path, and that a later sync moves it to `templates-kept/`.
It is not removed. Once that staging folder is an hour old, a later
sync moves it to `templates-kept/` (or beside the live folder). If
`templates-kept/` cannot be created at all, the official pack is refused
with the "not saved ... templates-kept" message, and your folder stays
as it was.

Without the exchange, the old folder is moved to `templates-kept/`
first, then the new one is put in place, and moved back if that fails.

An unchanged official folder (exactly one `templates.json` with the
recorded hash) is replaced, and nothing is kept.

### Other packs: a new official version replaces the folder

Hero, map, word, name, ban and bans-picks packs do not use
`pack-file-hashes.json` and are not moved to `templates-kept/`.

If the site still lists the same version you have recorded, sync leaves
the folder alone, so your edited files stay. If the site publishes a new
version, the folder is swapped out and the old tree is removed (or left
briefly as a hidden `.<pack>-prev-...` folder under `templates/`, which
a later sync deletes once it is an hour old). Your edit is not kept.
Copy the folder yourself before you edit one of these packs if you want
a backup.

## When the reader uses your edit

- **Hero, map and result-word packs.** The new reader checks the
  folder's stamp (device, inode and mtime) on a later board and reloads
  in the background when the stamp changes. A folder that changes while
  it is loading is not used; the load is tried again later. Replacing a
  file by renaming it into the folder updates the folder mtime. Whether
  overwriting a file in place also updates that stamp was not verified
  here, so a restart is the reliable way to pick up an in-place edit.
- **Names.** The pack is loaded when the tracker starts. After a pack
  sync install, it is reloaded in the background once the folder matches
  the install records. A folder you change by hand, with no matching
  install record, is left until the next tracker start. At that start
  the folder is loaded from disk, including your edits, if
  `manifest.json` still matches the files.
- **Rank and queue title.** These packs are loaded once when the daemon
  starts. An edit, or a pack that arrived while the tracker was running,
  is used from the next start.
- **Bans and picks.** The pack is loaded on the first game read of that
  run, then kept. An edit after that first load is used from the next
  start.

## Get back to the official pack

Do this when your edited files are still in the live folder and you want
the site's pack again.

1. Quit the tracker. The background pack job ends once every pack is
   current, and it does not look again until the next daemon start or
   the setup guide's reader pack step. Deleting a folder while the
   tracker is already idle will not fetch the official pack until one of
   those runs.
2. Remove the live pack folder. For `rank-v1` and `queue-title-v1`,
   removing `templates.json` is enough (the folder is then not "in
   place"). You do not need to edit `pack-versions.json`. That file can
   still name the version after the folder is gone; the next sync sees
   the missing folder and downloads the pack again.
3. Start the tracker, or run the setup guide's "Download reader pack"
   step, while the install is linked to a site account (pack sync needs
   the daemon token).
4. For `rank-v1` and `queue-title-v1`, that next daemon start is also
   when the official `templates.json` is loaded.

If a newer official pack already replaced your edit, the live folder is
already the official one. Your copy is only under `templates-kept/` (or,
in the rare cases above, beside the live folder or at a staging path).
The tracker does not copy a kept folder back into the live path by
itself.

## Clean up `templates-kept/`

Nothing removes `templates-kept/`: not pack sync, not the app, and not
`uninstall.sh --purge` (that script deletes only the names the tracker
writes, and neither `templates` nor `templates-kept` is one of them).
At each daemon start, when `templates-kept/` holds folders, or a kept
folder sits beside a live pack folder under `templates/`, the tracker
logs the `templates-kept/` path once, with both counts.

Delete `templates-kept/` by hand when you no longer need what is in it.
A kept folder is only a backup. Removing it does not change the live
pack.

## Privacy

The pack folders under `templates/` and the copies under
`templates-kept/` stay on this PC. Pack sync only downloads packs from
the site. No pack-sync path uploads those folders, and
`uninstall.sh --purge` is written to leave them in place so a hand-edited
copy is not deleted by mistake.

## See also

The operator guide, [Recognizer asset packs](tracker-packs.md), covers
building, publishing, rollback, and the same keep rules in more detail.
