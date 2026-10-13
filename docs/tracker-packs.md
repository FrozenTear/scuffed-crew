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

The server serves any valid name in its list. The tracker installs these six
names and ignores every other entry. `heroes-v1.tar` must stay in the list,
and stays first: every released tracker asks for it before anything else.

## Names and the no-change rule

A pack name is lowercase letters, digits and `-`, then `.tar`:
`<family>-v<generation>[-<variant>].tar`. `words-v1.tar` is family `words`,
generation 1. `heroes-v2-1080.tar` is family `heroes-1080`, generation 2.

**A published pack never changes.** If the art changes, build it under the
next name (`words-v2.tar`) and publish that beside the old one. The reasons:

- A tracker downloads a pack again only when the listed `version` differs
  from the one it recorded. New bytes under an old name and version never
  reach a tracker that already has that pack.
- A reader is written against one pack's contents. A tracker keeps reading
  `words-v1.tar` until a tracker release names `words-v2.tar` in `PACKS`
  (`crates/stat-tracker/src/packs.rs`). Until then `words-v2.tar` is listed
  and ignored, which is harmless.
- Old trackers stay in use. They must keep finding the pack they were
  built for.

`scripts/publish-packs.sh` enforces this: the same name with other bytes is
refused. `--replace` overrides it for a pack that no member has fetched yet,
for example one that was published broken an hour ago.

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

## Publish

```bash
# 1. See what would happen. Reads the server, changes nothing.
scripts/publish-packs.sh --dry-run --host root@your-vps ~/.local/share/scuffed-stat-tracker-packbuild/packs

# 2. Do it.
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
The server needs bash, coreutils and podman.

Other destinations:

| Want | Run |
| --- | --- |
| the podman volume on this machine | `scripts/publish-packs.sh --local-volume <outdir>` |
| a plain folder on this machine | `PACKS_DEST=/some/dir scripts/publish-packs.sh <outdir>` |
| a plain folder on the server | `PACKS_DEST=/some/dir scripts/publish-packs.sh --host root@your-vps <outdir>` |
| a volume with another name | `PACKS_VOLUME=<exact name>` with `--host` or `--local-volume` |

### What it refuses

Before it touches the destination, the script refuses:

- an `<outdir>` inside a git work tree;
- a manifest entry whose tar is missing, and a tar the manifest does not list;
- a tar whose sha256 or size differs from its manifest entry;
- a pack name that is not `[a-z0-9-]+.tar`, so no path separators;
- a tar that holds an absolute path, `..`, a symlink, a hard link, or a
  device node. Tars are listed with `tar -t`. They are never unpacked;
- a pack whose inner `manifest.json` has another `name` or `version` than its
  list entry (the tracker would refuse that pack);
- a pack over 32 MiB, and a merged list over 32 entries or 64 KiB (the
  tracker's and the server's limits).

After it has read the destination, it also refuses:

- the same name with other bytes or another version, without `--replace`;
- a published list whose own files are missing or damaged. Publish that pack
  again, or roll back, first.

### What it does

1. **Merge.** Packs that are published and not in `<outdir>` stay listed, in
   their old order. New packs go last. A pack with the same name, version and
   sha256 is skipped.
2. **Upload.** Each new tar goes to a temp name in the destination folder
   (`.<name>.publish-<stamp>.tmp`). Its sha256 and size are checked there.
3. **Check.** `manifest.json.new` is written and synced. Every pack it lists
   must be at the destination with that sha256 and size. If not, the run
   stops here.
4. **Switch.** New tars are renamed into place. The old manifest is copied to
   the backups folder. `manifest.json.new` is renamed over `manifest.json`
   last.
5. **Verify.** The manifest is read back, and every listed pack is hashed
   again at the destination.

A failure or a lost connection before step 4's last rename leaves the old
manifest and every tar it lists as they were. A new tar may already be in
the folder. Nothing lists it, so the server does not serve it, and the next
publish overwrites it. Temp files from a killed run are removed by the next
run.

It prints one line per pack:

```text
PACK                             SIZE  SHA256        STATUS
heroes-v1.tar                  123456  0123456789ab  kept
words-v1.tar                    92160  0123456789ab  skipped
words-v2.tar                    93184  0123456789ab  new
```

`kept` is published and not in `<outdir>`. `skipped` is in both, identical.
`new` was uploaded. `replaced` and `dropped` come from the two options below.
`repaired` means the pack was listed but its file at the destination was
missing or damaged, and was uploaded again.

### --replace

`--replace` allows the same name with other bytes. That pack cannot switch
in one step, because its file and the manifest are two renames. Between them,
for a moment, the server answers 404 for that pack or a tracker rejects it by
sha256 and keeps its old folder. The old tar is saved in the backups folder
first, and a failed run puts it back. If the `version` is unchanged the
script warns: trackers that already have the pack will not fetch it again.

### --keep N

By default every pack ever published stays listed. `--keep N` keeps the `N`
newest generations of each family and drops the rest from the list. A dropped
tar is moved to the backups folder after the switch, never before. A pack in
`<outdir>` is never dropped. Neither is `heroes-v1.tar`, or any name in
`PACKS_PIN` (space-separated; setting it replaces the default).

## Verify

The script verifies the destination itself (step 5) and prints
`Verified: ...`. That proves the files. It does not prove that the site
serves them.

To check through the site, give the script a tracker token in the
environment:

```bash
read -rs PACKS_TRACKER_TOKEN && export PACKS_TRACKER_TOKEN
PACKS_SITE_URL=https://your-site scripts/publish-packs.sh --host root@your-vps <outdir>
```

It then calls `GET /api/tracker/packs`, compares the list with the manifest
it wrote, downloads every pack, and compares status, `ETag`,
`Content-Length` and the sha256 of the body. The token is read from
`PACKS_TRACKER_TOKEN` only. It is never printed, and there is no command-line
option for it. Use a tracker token made for this check and revoke it
afterwards. Running the same command again later is safe: with nothing to
publish it only runs the check.

Without a token the script says `Authenticated check: SKIPPED`.

The pack routes allow 8 requests in a burst per client IP, then one every
10 seconds. With more than seven packs the check waits between requests.

## Roll back

Every publish that replaces a manifest copies the old one to the backups
folder on the destination machine first, and prints the path and one command:

```text
Backup: /root/scuffed-crew-backups/packs-manifest-pre-20261013T120000Z.json
Rollback: ssh root@your-vps 'cp -p /root/scuffed-crew-backups/packs-manifest-pre-20261013T120000Z.json <volume>/manifest.json.new && mv -f <volume>/manifest.json.new <volume>/manifest.json'
```

Run that line as printed. It puts the old list back with one rename. The
tars of the rolled-back publish stay in the folder unlisted, so the server
does not serve them. After `--replace` or `--keep` the line also copies the
saved tars back from `packs-tars-pre-<stamp>/`.

The backups folder is `~/scuffed-crew-backups` of the login that ran the
destination half (`/root/scuffed-crew-backups` on the VPS).
`PACKS_BACKUP_DIR` changes it. The first publish into an empty folder has
nothing to back up. Its rollback line removes `manifest.json`, which turns
the pack routes off again.

`scripts/backup.sh` does not snapshot the packs volume. The copy of record
is the build folder on the operator's machine.

## What a member's tracker does on its next start

Nothing is pushed. A tracker reads the list when its daemon starts and when
the setup guide runs its reader pack step.

- **Daemon start** syncs `heroes-v1.tar` only (the list and one file) and
  then starts capture. The other five packs are synced in the background
  with the same list, so capture never waits on them.
- **Setup guide** ("Download reader pack") syncs all six.
- A pack is downloaded when its folder is missing, or when the listed
  `version` differs from the one recorded in
  `<data_dir>/templates/pack-versions.json`. Otherwise it is left alone.
- Each download is checked against the list's size and sha256 before
  anything is unpacked, then unpacked into a staging folder and swapped in.
  A failed download or a refused pack leaves the old folder in place.
- A name the tracker does not know is ignored. A name it knows that the
  list does not have is reported as not listed, and its folder is kept.
- The new reader notices a new hero or map folder on a later board, without
  another restart.

So after a publish: a member who restarts the tracker gets the new packs in
the background while capture runs. A member who does not restart keeps
running on what they have.

More detail on a sync:

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
| `words-v1.tar` | `templates/words-v1/` | `<word_id>.png`, `<name>.json` | new reader, match result from result frames (hand-installed `templates/result/` is the fallback) |

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
the site" for the hero pack, keeps its hero folder, and still installs the
other packs it finds. A pack the list does not name is reported as not
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
