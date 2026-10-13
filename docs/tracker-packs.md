# Recognizer asset packs

The stat tracker downloads private asset packs from the site. It installs six
packs (see [The packs the tracker installs](#the-packs-the-tracker-installs)).
Only one of them is read today: the hero template pack, `heroes-v1.tar`. The
others are downloaded so later readers can use them. Those files are game art.

Do not commit them. Do not attach them to a GitHub release. Do not put them in
fixtures or under `crates/stat-tracker/test-data/`. The server only reads files
an operator places in a private directory.

## Where the files live

Set `PACKS_DIR` to that directory. It sits outside SurrealDB and outside the
web root, the same idea as `REPORTS_DIR`.

Compose sets `PACKS_DIR=/app/data/packs` and mounts the named volume
`packs-data` there **read-only**:

```yaml
- packs-data:/app/data/packs:ro
```

The image entrypoint does not create or chown this path. A read-only mount
cannot be changed from inside the container. Put the files in the volume
before the server starts, or copy them in while the volume is writable and
then start the stack so the mount is read-only.

The directory and `manifest.json` must be readable by the container user
`scuffed`. Mode `0755` on the directory and `0644` on the files is enough. A
named volume is often `root:root` mode `0755`, which `scuffed` can read.

`scripts/backup.sh` does not snapshot `packs-data`. Keep your own copy. Do not
publish the volume.

## If packs are off

When `PACKS_DIR` is unset, blank, or the directory cannot be read, the server
still starts. Both routes return:

```json
{"error":"packs_disabled"}
```

HTTP status is 503. Fix the directory and the next request tries again. If
startup refused the path because it overlaps uploads, reports, or `dist`,
change `PACKS_DIR` and restart.

## manifest.json

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

## The packs the tracker installs

The server serves any valid name. The tracker installs these names, each
into its own folder under `<data_dir>/templates/`, and ignores every other
entry:

| Pack | Folder | What may be inside (besides `manifest.json`) | Read by |
| --- | --- | --- | --- |
| `heroes-v1.tar` | `templates/heroes/` | `<hero>.png`, `special/<name>.png` | shadow hero log; new reader's fallback |
| `heroes-v2.tar` | `templates/heroes-v2/` | `<hero>.png`, `special/<name>.png` | new reader, 1440p-scale boards (when complete) |
| `heroes-v2-1080.tar` | `templates/heroes-v2-1080/` | `<hero>.png`, `special/<name>.png` | new reader, boards with a crop at most 881 px high (when complete) |
| `bans-v1.tar` | `templates/bans-v1/` | `<hero>.png` | nothing yet |
| `maps-v1.tar` | `templates/maps-v1/` | `atlas.json`, `g<4 hex digits>.png` | new reader, map and mode from the Tab banner |
| `words-v1.tar` | `templates/words-v1/` | `<word_id>.png`, `<name>.json` | new reader, match result from result frames, and the end title on poll ticks and Tab frames when the pack has end-title templates (hand-installed `templates/result/` is the fallback) |

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

### Where they land and when they are fetched again

Each pack is unpacked into a staging folder next to its live folder and then
swapped into place, so a failed download or a rejected pack leaves the old
folder in place. Installed versions are recorded in
`<data_dir>/templates/pack-versions.json`, one `"<name>": "<version>"` key per
pack. A file written by an older tracker that names only `heroes-v1.tar` is
read as is, and its entry is kept.

It checks the list when the daemon starts and when the setup guide runs its
reader pack step. It downloads a pack again only when the listed `version`
differs from the recorded one, or that pack's folder is gone. To ship new
art, change `version` in both manifests. Replacing the file under the same
version does not reach trackers that already have it.

Packs are fetched one after another, `heroes-v1.tar` first. A first install
of all six is seven requests (the list and six files), inside the burst of 8
below.

- **Daemon start** syncs `heroes-v1.tar` only (the list and one file, as
  before) and then starts capture. The other five packs are synced in the
  background with the same list, so capture never waits on them.
- **Setup guide** ("Download reader pack") syncs all six.
- **One sync at a time.** A sync holds an exclusive lock on
  `<data_dir>/templates/.pack-sync.lock` before its first request. A second
  sync (the daemon and the app at once) sends nothing and reports "A reader
  pack sync is already running. Try again in a moment." That is not a failure.
- A 401, 403, 429, or `packs_disabled` answer on one download stops the packs
  after it for that run. Their folders are kept, a pack that is already
  current is still reported as current, and the next run tries again.
- Staging (`.<pack>-next-*`) and backup (`.<pack>-prev-*`) folders left under
  `templates/` by a killed sync are removed by the next sync once they are
  an hour old.

The setup guide shows the hero pack's line, plus "Extra reader packs ready:
N of M." when the server lists any of the other packs, plus the rate-limit
sentence when one of them was rate limited.

## Copy into the volume

From a private directory that is not this git repo (Podman):

```bash
podman volume create packs-data
podman run --rm \
  -v packs-data:/packs:Z \
  -v /path/to/private-packs:/src:ro,Z \
  docker.io/library/alpine:3 \
  sh -c 'cp -a /src/. /packs/ && chmod 0755 /packs && chmod 0644 /packs/*'
```

Then start the stack. The compose file mounts that volume read-only.

## How the tracker calls it

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
