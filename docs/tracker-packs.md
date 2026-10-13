# Recognizer asset packs

A pack is one tar file of small images and JSON that the stat tracker needs
to read the game screen: hero portraits, ban icons, map-name glyphs, result
words. The tracker does not ship them. It downloads them from the site with
its sync token and unpacks them under its data folder.

Packs are game art. Do not commit them. Do not attach them to a GitHub
release, a PR, or a comment. Do not put them in fixtures or under
`crates/stat-tracker/test-data/`. They live in two places only: the build
folder on the machine that made them, and `PACKS_DIR` on the server.

## The six packs

| Pack | What it is for | Size on 2026-10-13 |
| --- | --- | --- |
| `heroes-v1.tar` | hero portraits, the first pack | on the server; not built by packgen |
| `heroes-v2.tar` | hero portraits for 1440p-scale boards | 860 KiB |
| `heroes-v2-1080.tar` | hero portraits for boards with a crop at most 881 px high | 590 KiB |
| `bans-v1.tar` | ban icons | 600 KiB |
| `maps-v1.tar` | map-name glyphs | 40 KiB |
| `words-v1.tar` | result words | 90 KiB |

Which folder each one lands in, what may be inside, and which reader uses it
are in [Reference: inside a pack](#reference-inside-a-pack).

The server serves any valid name in its list. A tracker asks for the names
that are built into it and ignores every other entry.

## Names, versions, and which tracker asks for what

**A file name is a format generation.** `<family>-v<generation>[-<variant>].tar`,
lowercase letters, digits and `-`: `words-v1.tar` is the first layout of the
result-words pack. The names are compiled into the tracker
(`PACKS` in `crates/stat-tracker/src/packs.rs`), and a released tracker asks
for its packs by exactly those names.

**The `version` is the content.** New art for the same layout (a game patch
changes the portraits in `heroes-v2.tar`, a phrase mask is added to
`words-v1.tar`) is published under the **same name with a new `version`**.
A tracker downloads a pack again when the listed `version` differs from the
one it recorded, so every tracker that asks for that pack picks the new
content up on its next sync.
This is the normal way to change a pack. See
[Update a pack in place](#update-a-pack-in-place).

**A new name is only for a layout that released trackers cannot read.** If
`words-v1.tar` could not hold what the reader needs, the new layout would be
`words-v2.tar`, published beside `words-v1.tar`, and a tracker release would
name it. Until then it is listed and ignored, which is harmless.

Same name, other bytes and the **same** `version` is always a mistake: the
bytes changed and no tracker that has the pack would ever fetch them. The
publish script refuses that, with every option.

Which names each tracker asks for:

| Tracker | Asks for |
| --- | --- |
| 0.5.0-alpha.2 to 0.5.0-alpha.6 (released) | `heroes-v1.tar` only |
| main, to be released as 0.5.0-alpha.7 | all six: `heroes-v1.tar` first, then `heroes-v2.tar`, `heroes-v2-1080.tar`, `bans-v1.tar`, `maps-v1.tar`, `words-v1.tar` |

So today `heroes-v1.tar` must be in the list: without it every released
tracker reports "The reader pack did not match the site" for the hero pack.
Once alpha.7 is out, all six must stay. A name may leave the list only when
no tracker release that members still run asks for it. The publish script
warns when the list has no `heroes-v1.tar`.

## Build

Packs are built on the operator's machine with packgen
(`crates/stat-tracker/scripts/packgen.py`; on the `claude/packgen` branch
until it is merged). It reads game-art extracts and labelled captures from
paths given on the command line, refuses to write into a git work tree, and
writes one folder: `<out>/packs/` with one `<name>.tar` per pack and a
`manifest.json` that lists each tar's `name`, `version`, `sha256` and `size`.
That `packs/` folder is the `<outdir>` for the publish script. The inputs and
the output stay on the build machine. The subcommands and their inputs are in
the docstring at the top of the script.

`heroes-v1.tar` is older than packgen. It was copied to the server by hand
and is not in the packgen output. Publishing leaves it alone.

An `<outdir>` does not have to hold every pack. To change one pack, a folder
with that one tar and a `manifest.json` that lists it is enough.

## Publish

The examples are bash. Run them from a checkout of this repo, in `bash`.

```bash
# 1. Check the build folder. Reads no server.
scripts/publish-packs.sh --check ~/.local/share/scuffed-stat-tracker-packbuild/packs

# 2. See what would happen. Reads the server, changes nothing.
scripts/publish-packs.sh --dry-run --host root@your-vps ~/.local/share/scuffed-stat-tracker-packbuild/packs

# 3. Do it.
scripts/publish-packs.sh --host root@your-vps ~/.local/share/scuffed-stat-tracker-packbuild/packs
```

The server does not need a restart. It reads `manifest.json` on every
request, and the container sees the volume's files as soon as they land.

`--host` is the ssh login that owns the podman volumes (root on the VPS).
The script finds the one volume named `packs-data` or `<project>_packs-data`
(on the VPS: `scuffed-crew_packs-data`) and writes into its mountpoint, the
same way `scripts/backup.sh` reads the other volumes. Nothing is installed on
the server: the destination half of the script (`scripts/lib/packs-dest.sh`)
is sent over the ssh connection, one connection to read and one to publish.

The server needs bash 4.4 or newer, GNU coreutils 8.24 or newer, `flock`
(part of util-linux) and podman. The destination half checks the first three
itself and stops with a clear message, also on a dry run. It flushes the
files it wrote with `sync <file>`. It never calls a bare `sync`, which would
stall a busy host. One publish runs at a time: the destination half holds a
lock on the folder, and a second publish is refused while it is held.

Other destinations:

| Want | Run |
| --- | --- |
| the podman volume on this machine | `scripts/publish-packs.sh --local-volume <outdir>` |
| a plain folder on this machine | `PACKS_DEST=/some/dir scripts/publish-packs.sh <outdir>` |
| a plain folder on the server | `PACKS_DEST=/some/dir scripts/publish-packs.sh --host root@your-vps <outdir>` |
| a volume with another name | `PACKS_VOLUME=<full name>` with `--host` or `--local-volume` |

`PACKS_VOLUME` is the full volume name. A prefix is refused.

### What it refuses

Before it touches the destination, the script refuses:

- an `<outdir>` inside a git work tree;
- a manifest entry whose tar is missing, and a tar the manifest does not list;
- a tar whose sha256 or size differs from its manifest entry;
- a pack name that is not `[a-z0-9-]+.tar`, so no path separators;
- a file that is not a plain tar (a gzip file named `.tar`, for example);
- a tar that holds an absolute path, `..`, a symlink, a hard link, or a
  device node. Tars are listed with `tar -t`. They are never unpacked;
- a pack whose inner `manifest.json` has another `name` or `version` than its
  list entry (the tracker would refuse that pack);
- a pack over 32 MiB, and a merged list over 32 entries or 64 KiB (the
  tracker's and the server's limits).

After it has read the destination, it also refuses:

- a published name with other bytes and the same `version`, always;
- a published name with a new `version`, unless `--update` is given;
- a destination without `manifest.json`, unless `--first-publish` is given,
  and with it a folder that holds anything but packs of this publish. Six
  packs are live, so on the VPS a run that reports `0 pack(s) published` is
  looking at the wrong folder;
- a published list whose own files are missing or damaged. Publish that pack
  again, or roll back, first;
- a backups folder inside the destination, also through a symlink, or
  inside a git work tree;
- a second publish while one is running.

### What it does

1. **Merge.** Packs that are published and not in `<outdir>` stay listed, in
   their old order. New packs go last. A pack with the same name, version and
   sha256 is skipped.
2. **Upload.** Each tar goes to a temp name in the destination folder
   (`.<name>.publish-<stamp>.tmp`). Its sha256 and size are checked there.
3. **Check.** The new manifest is written beside the old one and flushed.
   Every pack it lists must be at the destination with that sha256 and size.
   If not, the run stops here.
4. **Save.** The old manifest is copied to the backups folder. For an update
   the old tar is copied there too, and the copy's sha256 is checked.
5. **Switch.** New names are renamed into place. Then, back to back: the tar
   of each updated pack, and last the new manifest over `manifest.json`.
6. **Verify.** The manifest is read back, and every listed pack is hashed
   again at the destination.

A failure or a lost connection before the last rename of step 5 leaves the
old manifest and every tar it lists as they were. A new tar may already be in
the folder. Nothing lists it, so the server does not serve it. Temp files
from a killed run are removed by the next run; only the script's own temp
names are, never another file.

A file that is already in the folder under the name of a pack being
published as `new` or `repaired` (an unlisted tar of an earlier run, or a
damaged one) is not simply overwritten: a copy goes to
`packs-set-aside-<stamp>/` in the backups folder first, and the run prints
`Set aside: ...`. A failed run can leave its copies in the backups folder.

It prints one line per pack:

```text
PACK                   VERSION          SIZE  SHA256        STATUS
heroes-v1.tar          1              123456  0123456789ab  kept
maps-v1.tar            1               40960  0123456789ab  skipped
words-v1.tar           2               93184  0123456789ab  updated (1 -> 2)
words-v2.tar           1               93184  0123456789ab  new
```

`kept` is published and not in `<outdir>`. `skipped` is in both, identical.
`new` was uploaded under a name that was not listed. `updated (1 -> 2)` got
new bytes and a new version under its old name. `repaired` means the pack was
listed but its file at the destination was missing or damaged, and was
uploaded again. A run that only repairs does not change the list: it prints
`Backup: not needed` and no rollback line.

The script cannot tell a newer `version` from an older one; it only sees
that they differ. Read every `updated` row before a real run.

### When a run does not end cleanly

The script never says what is live from memory. If the publish half fails,
the connection drops, or a signal arrives, it waits for a publish half that
is still running, reads the destination again, and prints one of:

- **the OLD list is still live, every pack in place.** Nothing was published.
- **the NEW list is live, every pack in place.** The publish did happen. The
  `Backup:` and `Rollback:` lines are printed.
- **the OLD list is live, but `<pack>` does not match it.** A publish was
  killed between the two renames of an update. Run the same command again:
  it puts the old file back first, then publishes. The by-hand line that is
  printed does the same repair alone.
- **what is live is NOT known.** The destination could not be read again.
  Run the same command with `--dry-run` to look.

It exits non-zero in all four. A SIGTERM, SIGINT or SIGHUP sent to the
script alone while the publish half runs does not stop that half: the script
waits for it, prints the `Backup:` and `Rollback:` lines, and exits with the
signal's status. Ctrl-C in a terminal is different: it also reaches the
publish half (a local destination) or ends the ssh connection (`--host`).
The publish half then stops before the switch or finishes it, and the
script reads back and reports which, as above.

If a half-switched pack cannot be repaired, the script says why and changes
nothing: the saved copy of the old tar is gone (remove
`<destination>/.publish-journal` by hand, then publish that pack again), or
the list names bytes that are neither the old file nor the one in the
folder (compare by hand, then remove that file).

### Update a pack in place

```bash
scripts/publish-packs.sh --dry-run --update --host root@your-vps <outdir>
scripts/publish-packs.sh --update --host root@your-vps <outdir>
```

`--update` allows a published name to get new bytes under a new `version`.
Without it that is refused, so an update never happens by accident. Bump the
version when you build the pack (the tar's inner `manifest.json` and the
list entry must agree). The table shows `updated (1 -> 2)`, and the old tar
is saved in the backups folder (`packs-tars-pre-<stamp>/`).

An update is two renames, the tar and then `manifest.json`, a few
milliseconds apart, and for that moment the old list is live over the new
bytes. The server answers 404 `pack_not_found` for that pack when its size
changed, a tracker refuses bytes whose sha256 is not the listed one and keeps
its folder, and a tracker that already has the old version asks for nothing:
no tracker installs a wrong pack, and its next sync gets the new one.

SIGTERM, SIGINT and SIGHUP are ignored for those two renames: one that
arrives then is dropped and the update goes through. `kill -9` or a power
cut between them leaves the old list over the new tar. The destination half
wrote down what it was doing before the first rename
(`.publish-journal`), and the next run reads that note against the live
`manifest.json`: a pack whose old bytes the live list still names gets them
back from the saved copy before anything else is done, and a pack whose file
already matches the live list is never touched.

A member's tracker notices the update on its next sync: a restart, or the
setup guide's "Download reader pack".

### Taking a pack off the list

The script never removes a pack from the list. There is no `--keep` and no
`--replace`. The list holds one entry per format generation and has room for
32, and a missing name breaks every tracker release that asks for it (see the
table in [Names, versions, and which tracker asks for what](#names-versions-and-which-tracker-asks-for-what)).
If a name really has to go, that is a deliberate edit of `manifest.json` on
the server, by hand, after checking that table and with no publish running:
write the new file beside the old one and rename it over.

## Verify

The script verifies the destination itself (step 6) and prints
`Verified: ...`. That proves the files. It does not prove that the site
serves them.

To check through the site, give the script a tracker token in the
environment:

```bash
read -rs PACKS_TRACKER_TOKEN && export PACKS_TRACKER_TOKEN
PACKS_SITE_URL=https://your-site scripts/publish-packs.sh --host root@your-vps <outdir>
unset PACKS_TRACKER_TOKEN
```

Make a token for this check: sign in to the site, open `/stats/tokens`
("Daemon Tokens"), press "+ New Token", label it (`packs-check`), and copy it
when it is shown. Paste it at the `read` prompt, which does not echo.
Revoke it on the same page afterwards. Do not take the token out of a
tracker's settings.

The script then calls `GET /api/tracker/packs`, compares the list with the
manifest at the destination, downloads every pack, and compares status,
`ETag`, `Content-Length` and the sha256 of the body. The token is read from
`PACKS_TRACKER_TOKEN` only. There is no command-line option for it. It is
never printed, it is in no command line (curl reads it from a mode-600 file
in the script's temp folder, removed when the script ends), no child process
gets it in its environment (it is taken out on the script's first line),
and a `bash -x` trace does not show it. The site URL and the token's shape
are checked before anything is published.

With nothing to publish the same command only runs the check, so it can be
repeated. Without a token the script says `Authenticated check: SKIPPED`.

The pack routes allow 8 requests in a burst per client IP, then one every
10 seconds, and answer 429 with the seconds to wait. One check is seven
requests for six packs. A second check within a minute is not a failure: the
script waits as long as each 429 asks (up to six times per request) and goes
on, so it is slower, about ten seconds per pack.

## Roll back

Every publish that replaces a manifest copies the old one to the backups
folder on the destination machine first, and prints the path and one command:

```text
Backup: /root/scuffed-crew-backups/packs-manifest-pre-20261013T120000Z.json
Rollback: ssh root@your-vps "flock -w 60 <volume> sh -c 'cp -p /root/scuffed-crew-backups/packs-manifest-pre-20261013T120000Z.json <volume>/.manifest.json.rollback-20261013T120000Z.tmp && mv -f <volume>/.manifest.json.rollback-20261013T120000Z.tmp <volume>/manifest.json'"
```

Run that line as printed. It puts the old list back with one rename. The
tars of the rolled-back publish stay in the folder unlisted, so the server
does not serve them. The line is one `sh -c` command: it can be pasted into
bash or nushell, and it does not depend on root's login shell on the server.
It runs under the publish lock (`flock`), waiting up to a minute for a
publish that is running, so the two never work on the folder at once.

After an update the line is longer. It first puts each saved tar back, then
the manifest:

```text
Old tar: /root/scuffed-crew-backups/packs-tars-pre-20261013T120000Z/words-v1.tar
Backup: /root/scuffed-crew-backups/packs-manifest-pre-20261013T120000Z.json
Rollback: ssh root@your-vps "flock -w 60 <volume> sh -c 'cp -p <old tar> <volume>/.words-v1.tar.rollback-<stamp>.tmp && mv -f <volume>/.words-v1.tar.rollback-<stamp>.tmp <volume>/words-v1.tar && cp -p <backup> <volume>/.manifest.json.rollback-<stamp>.tmp && mv -f <volume>/.manifest.json.rollback-<stamp>.tmp <volume>/manifest.json'"
```

Every file is copied to a temp name in the volume and renamed into place. It
never writes over a live tar, so a download that is running keeps the bytes
it started with, and it does not touch the folder's mode. The line is built
from files the script hashed after the publish: a saved file that is not the
old one is not offered. A rolled-back update has the same short window as
the update itself. Trackers that already fetched the new version see the old
`version` again and fetch the old pack on their next sync.

The backups folder is `~/scuffed-crew-backups` of the login that ran the
destination half (`/root/scuffed-crew-backups` on the VPS).
`PACKS_BACKUP_DIR` changes it. It must be outside the destination and
outside any git work tree (old packs are saved there). A folder the script
creates is mode 0700; an existing one is left as it is. The first
publish into an empty folder has nothing to back up. Its rollback line
removes `manifest.json`, which turns the pack routes off again.

`scripts/backup.sh` does not snapshot the packs volume. The copy of record
is the build folder on the operator's machine.

## Example: a content update, start to end

`words-v1.tar` goes from version 1 to version 2. `<outdir>` holds the new
build. Run this in `bash`, from a checkout of this repo. `<vps>` is the
server and `<site>` the public host name.

```bash
unset PACKS_DEST PACKS_VOLUME PACKS_BACKUP_DIR

# 1. Local check. Reads no server.
scripts/publish-packs.sh --check <outdir>

# 2. A read-only look at the server: versions, the lock tool, the one packs
#    volume, and the six tars plus manifest.json in it.
ssh root@<vps> 'bash --version | head -n1; sync --version | head -n1; command -v flock; podman volume ls --format "{{.Name}}" | grep packs-data; ls -la "$(podman volume inspect scuffed-crew_packs-data --format "{{.Mountpoint}}")"'

# 3. Dry run. Changes nothing. Want: "Destination: .../scuffed-crew_packs-data/_data
#    (6 pack(s) published)", one row "words-v1.tar 2 ... updated (1 -> 2)",
#    every other row kept or skipped.
scripts/publish-packs.sh --dry-run --update --host root@<vps> <outdir>

# 4. The update. Copy the "Old tar:", "Backup:" and "Rollback:" lines before
#    you close the terminal. Want: "Verified: ... 6 pack(s) match".
scripts/publish-packs.sh --update --host root@<vps> <outdir>

# 5. Verify through the site. Nothing is published again; only the check runs.
read -rs PACKS_TRACKER_TOKEN && export PACKS_TRACKER_TOKEN
PACKS_SITE_URL=https://<site> scripts/publish-packs.sh --host root@<vps> <outdir>
unset PACKS_TRACKER_TOKEN

# 6. Roll back, if needed: run the "Rollback:" line from step 4 as printed.
```

Stop, and change nothing, if step 3 prints `0 pack(s) published`, another
path than the packs volume, `under the same version` for any pack, or
`updated` for a pack you did not mean to change. If step 4 does not end with
`Verified`, read what it printed about which list is live
([When a run does not end cleanly](#when-a-run-does-not-end-cleanly)).

## What a member's tracker does on its next start

Nothing is pushed. A tracker reads the list when its daemon starts and when
the setup guide runs its reader pack step. Which packs it then asks for
depends on the tracker release.

**Released trackers, 0.5.0-alpha.2 to alpha.6**, know one pack name:

- They read the list and look for `heroes-v1.tar`. Every other entry is
  ignored. The five newer packs do nothing for these members until they
  install a tracker that names them.
- `heroes-v1.tar` is downloaded when its folder is missing, or when the
  listed `version` differs from the one recorded in
  `<data_dir>/templates/pack-versions.json`.
- A list with entries but without `heroes-v1.tar` is reported as "The reader
  pack did not match the site". The hero folder they have is kept.

**The tracker on main, to be released as 0.5.0-alpha.7**, knows all six:

- **Daemon start** syncs `heroes-v1.tar` only (the list and one file) and
  then starts capture. The other five packs are synced in the background
  with the same list, so capture never waits on them.
- **Setup guide** ("Download reader pack") syncs all six.
- A pack is downloaded when its folder is missing, or when the listed
  `version` differs from the one recorded in
  `<data_dir>/templates/pack-versions.json`. Otherwise it is left alone.
- A name the tracker does not know is ignored. A name it knows that the
  list does not have is reported as not listed, and its folder is kept.
- The new reader notices a new hero or map folder on a later board, without
  another restart.

In every release, each download is checked against the list's size and
sha256 before anything is unpacked, then unpacked into a staging folder and
swapped in. A failed download or a refused pack leaves the old folder in
place.

So after a publish: a member who restarts the tracker gets what their
release asks for. A member who does not restart keeps running on what they
have.

More detail on a sync, for the tracker on main:

- **One sync at a time.** A sync holds an exclusive lock on
  `<data_dir>/templates/.pack-sync.lock` before its first request. A second
  sync (the daemon and the app at once) sends nothing and reports "A reader
  pack sync is already running. Try again in a moment." That is not a failure.
- A 401, 403, 429, or `packs_disabled` answer on one download stops the packs
  after it for that run. Their folders are kept, a pack that is already
  current is still reported as current, and the next run tries again.
- A first install of all six is seven requests (the list and six files),
  inside the burst of 8.
- Staging (`.<pack>-next-*`) and backup (`.<pack>-prev-*`) folders left under
  `templates/` by a killed sync are removed by the next sync once they are
  an hour old.

The setup guide shows the hero pack's line, plus "Extra reader packs ready:
N of M." when the server lists any of the other packs, plus the rate-limit
sentence when one of them was rate limited.

## What "packs_disabled" means

The server could not load a pack list, so pack downloads are off. Both
routes answer HTTP 503 with:

```json
{"error":"packs_disabled"}
```

The rest of the site is not affected, and the server did start. The causes:

- `PACKS_DIR` is unset or blank;
- the folder is missing or the container user cannot read it;
- there is no `manifest.json` in it (nothing was published yet, or a first
  publish was rolled back);
- `manifest.json` breaks a rule in the next section;
- the path overlaps uploads, reports, or `dist`. That one is decided at
  startup: change `PACKS_DIR` and restart.

For every cause but the last, fix the folder and the next request works. No
restart. The server logs `recognizer packs unavailable` with the reason.

A tracker that gets this answer keeps the packs it has and tries again on
its next start. Its setup guide shows "Reader packs aren't set up on the
server yet. The tracker still works. Try again later or ask an admin."

## Where the files live

`PACKS_DIR` is a folder outside SurrealDB and outside the web root, the same
idea as `REPORTS_DIR`. Compose sets `PACKS_DIR=/app/data/packs` and mounts
the named volume `packs-data` there **read-only**:

```yaml
- packs-data:/app/data/packs:ro
```

Podman Compose prefixes the project name, so on the VPS the volume is
`scuffed-crew_packs-data`. The server cannot write to it. The publish script
writes through the volume's mountpoint on the host.

The folder and its files must be readable by the container user `scuffed`.
The script writes files as mode `0644`. A named volume's folder is
`root:root` mode `0755`, which `scuffed` can read.

## Reference: manifest.json

`PACKS_DIR/manifest.json` is a JSON array. Each object has `name`, `version`,
`sha256`, and `size`:

```json
[
  {
    "name": "heroes-v1.tar",
    "version": "1",
    "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    "size": 1234
  }
]
```

`name` is one file in that directory, not a path. Letters, digits, `.`, `_`,
and `-` only. It cannot be `..`, it cannot contain `..`, and it cannot be
`manifest.json`. `version` is a short label (up to 32 of those same
characters). `sha256` is 64 lowercase hex characters, the SHA-256 of the file.
`size` is the byte length. At most 32 entries. The manifest itself must be
64 KiB or smaller.

A name that is not in this file is not served, including a path that tries to
leave the directory. The server canonicalizes the file and requires it to stay
inside `PACKS_DIR`.

`scripts/publish-packs.sh` writes this file with one entry per line.

## Reference: inside a pack

| Pack | Folder | What may be inside (besides `manifest.json`) | Read by |
| --- | --- | --- | --- |
| `heroes-v1.tar` | `templates/heroes/` | `<hero>.png`, `special/<name>.png` | shadow hero log; new reader's fallback |
| `heroes-v2.tar` | `templates/heroes-v2/` | `<hero>.png`, `special/<name>.png` | new reader, 1440p-scale boards (when complete) |
| `heroes-v2-1080.tar` | `templates/heroes-v2-1080/` | `<hero>.png`, `special/<name>.png` | new reader, boards with a crop at most 881 px high (when complete) |
| `bans-v1.tar` | `templates/bans-v1/` | `<hero>.png` | nothing yet |
| `maps-v1.tar` | `templates/maps-v1/` | `atlas.json`, `g<4 hex digits>.png` | new reader, map and mode from the Tab banner |
| `words-v1.tar` | `templates/words-v1/` | `<word_id>.png`, `<name>.json` | new reader, match result from result frames, the result word on the accolade and rank screens, and the end title on poll ticks and Tab frames when the pack has end-title templates (hand-installed `templates/result/` is the fallback) |

`<hero>` is a file stem from `HERO_NAMES` in `crates/types/src/heroes.rs`
(`ana`, `wrecking-ball`). For the v2 hero packs and `bans-v1.tar` a stem is
1 to 64 of `a-z`, `0-9`, `_`, `-`. For `words-v1.tar` it is 1 to 64 of `a-z`,
`0-9`, `_` (`accolade_victory.png`, `words.json`). Glyph names are a `g` and
four lowercase hex digits (`g003a.png`). `heroes-v1.tar` keeps its older rule:
any one top-level file, or one file in `special/`.

Each pack is checked on its own. A file outside its pack's layout, a bad
sha256, or a failed download refuses that pack only. The other packs still
install, and the refused pack's folder is left as it was.

If the list is empty, the tracker does nothing. If the list has entries but
none is `heroes-v1.tar`, every tracker reports "The reader pack did not match
the site" for the hero pack and keeps its hero folder. The tracker on main
still installs the other packs it finds. A pack the list does not name is reported as not
listed, and its installed folder is kept.

The new reader (`reader = "new"`) uses a v2 hero pack only when it has every
hero and special class `heroes-v1.tar` has. The daemon installs the v2 packs
after the reader has started; the reader notices the new folder on a later
board and re-loads its hero set in the background.

The new reader reads the map from the Tab board's top banner with
`maps-v1.tar`. A map pack copied in by hand to `templates/maps/` is used only
when `templates/maps-v1/` is missing or does not load. Like the v2 hero
packs, maps-v1 is picked up on a later Tab once it is swapped in, without a
restart; a folder that changes while it loads is not used. A sure banner read
is the top-bar map: the session map, a vote-candidate veto, and the rule that
keeps Deathmatch and other untracked maps local apply to it as they do to the
top-bar OCR. A flagged read, no banner, a name the tracker's map table does
not hold, or a panic in the banner reader falls back to the top-bar OCR. A
sure banner read also replaces a session map that came only from the
full-board text, while that game has shown no result or boundary signal; a
session map from the top bar or an accolade is never replaced. With
`reader = "ocr-v1"` the banner is not read.

The new reader reads the end title (VICTORY!, DEFEAT or DRAW over the world
at the end of a match) from the end-title templates of `words-v1.tar`:
`end_title_<word>.png`, and variants named `end_title_<name>_<word>.png`. It
needs a VICTORY and a DEFEAT one (a variant counts). Without that pair, or
with no words pack at all, the new reader reads the end title and the
end-of-match title card with OCR, as `reader = "ocr-v1"` does, and logs one
warning. With the pair loaded no OCR reads them. On a poll tick the first
sure read only wakes the poller; the same sure word on the next poll tick
counts as one result-word read, and a result still needs a second agreeing
word read. A Tab frame's sure read is that frame's result. With
`reader = "ocr-v1"` the templates are not read.

The new reader also reads the result word on the post-match accolade screen
and on the competitive rank screen from `words-v1.tar`: `accolade_<word>.png`
and `rank_screen_<word>.png`, and variants named `<screen>_<name>_<word>.png`.
Each screen needs a VICTORY and a DEFEAT template. Without that pair for a
screen, or with no words pack at all, that screen's word is read with OCR,
as `reader = "ocr-v1"` does, with one warning per screen. With the pair
loaded no OCR reads it. The read is made on the poll ticks the OCR was made
on: the screen's top-left crop must be lit and must have held still since
the previous tick. A sure read counts as one result-word read, and a result
still needs a second agreeing word read; a Tab frame's sure read is that
frame's result. The pack has no DRAW template for these two screens, so a
draw is not read from them. With `reader = "ocr-v1"` the templates are not
read.

Every glyph file that `atlas.json` names must be a `g<4 lowercase hex>.png`
file in the pack folder, the same names pack sync allows. Any other path
refuses the whole pack, and the reader falls back to the next folder.

Each file is a plain ustar archive. At the top level it holds:

- `manifest.json`, exactly once.
- The files from the table above. Only the hero packs may carry a
  `special/` folder, and nothing goes deeper than `special/`.

Only regular files and, for the hero packs, the `special/` directory entry.
A symlink, hard link, pax or GNU long-name header, or a `./` directory entry
makes the tracker reject the pack. Build it with GNU tar and list the
members, not `.`:

```bash
cd /path/to/private-packs/heroes
tar --format=ustar -cf ../heroes-v1.tar manifest.json *.png special
```

The inner `manifest.json` names the pack and lists every other file in the
archive:

```json
{
  "name": "heroes-v1.tar",
  "version": "1",
  "files": [
    {
      "path": "ana.png",
      "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      "size": 1234
    },
    {
      "path": "special/skull_01.png",
      "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      "size": 567
    }
  ]
}
```

`name` must be the pack's own name (`heroes-v1.tar` above). `version` must
equal the `version` of that entry in `PACKS_DIR/manifest.json`. `files` lists
every regular file in the archive except `manifest.json` itself, once each,
with its lowercase SHA-256 and byte length. Other keys (such as `source`) are
ignored. Do not list the `special/` directory entry: the tracker skips
directory entries, so listing one fails the pack. A file in the archive that
is not listed, or a listed file that is missing, fails the pack.

The tracker checks each downloaded file's size and sha256 against
`PACKS_DIR/manifest.json` before it unpacks anything. Each is capped at
32 MiB.

The publish script checks the member paths and types, and that the inner
`name` and `version` match the list. The rest of this section is checked by
the tracker when it installs the pack.

## Reference: how the tracker calls it

Both routes use the same daemon token as `POST /api/stats/upload`. The tracker
has a token, not a browser session.

```text
Authorization: Bearer <daemon token>
```

A missing, unknown, revoked, or expired token gets the same body:

```json
{"error":"Unauthorized"}
```

`GET /api/tracker/packs` returns the manifest list (`name`, `version`,
`sha256`, `size`).

`GET /api/tracker/packs/{name}` streams that file when `name` matches one
entry exactly. Anything else is 404:

```json
{"error":"pack_not_found"}
```

The download response sends:

- `Cache-Control: private, no-store`
- `Content-Disposition: attachment` (with the pack file name)
- `ETag` set to the manifest sha256, quoted (`"<sha256>"`)
- `Content-Type: application/octet-stream`

`X-Content-Type-Options: nosniff` is set by `scuffed-server` on every
response. Do not add that header in Caddy. A second line is sent beside the
one from the app. See `docs/deploy.md`.

These routes do not write to the database and do not log the token. They are
rate limited per client IP (burst 8, then one request every 10 seconds), in
their own bucket. A limited call returns 429:

```json
{"error":"rate_limited","retry_after":1}
```

`retry_after` is a whole number of seconds, at least 1. `Retry-After` carries
the same count.
