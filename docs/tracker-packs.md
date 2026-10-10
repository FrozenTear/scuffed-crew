# Recognizer asset packs

The stat tracker downloads private asset packs from the site. Today it reads
one pack: the hero template pack, `heroes-v1.tar`. Those files are game art.

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

## The hero pack the tracker needs

The server serves any valid name. The tracker only installs the entry named
exactly `heroes-v1.tar`. If the list is empty, the tracker does nothing. If
the list has entries but none is `heroes-v1.tar`, every tracker reports "The
reader pack did not match the site" and keeps what it had. Other entries are
ignored.

The file is a plain ustar archive. At the top level it holds:

- `manifest.json`, exactly once.
- `<hero>.png` for each hero. Use the `file` names from `HERO_NAMES` in
  `crates/types/src/heroes.rs` (`ana.png`, `wrecking-ball.png`).
- `special/` with the non-hero row images. Their names start with
  `placeholder`, `skull`, or `empty` (`special/skull_01.png`).

No other folder and nothing deeper than `special/`. Only regular files and the
`special/` directory entry. A symlink, hard link, pax or GNU long-name header,
or a `./` directory entry makes the tracker reject the pack. Build it with
GNU tar and list the members, not `.`:

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

`name` must be `heroes-v1.tar`. `version` must equal the `version` of the
`heroes-v1.tar` entry in `PACKS_DIR/manifest.json`. `files` lists every
archive member except `manifest.json` itself, once each, with its lowercase
SHA-256 and byte length. A file in the archive that is not listed, or a listed
file that is missing, fails the pack.

The tracker checks the downloaded file's size and sha256 against
`PACKS_DIR/manifest.json` before it unpacks anything. It is capped at 32 MiB.

### Where it lands and when it is fetched again

The tracker unpacks into `<data_dir>/templates/heroes/` and records the
installed version in `<data_dir>/templates/pack-versions.json`. A failed
download or a rejected pack leaves the old folder in place.

It checks the list when the daemon starts and when the setup guide runs its
reader pack step. It downloads again only when the listed `version` differs
from the recorded one, or the `heroes` folder is gone. To ship new art, change
`version` in both manifests. Replacing the file under the same version does
not reach trackers that already have it.

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
