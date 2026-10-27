# Deploying Scuffed Crew (VPS / Podman)

This is the supported path for a **single VPS** with Podman Compose. You do **not** need Discord OAuth for first install.

## Prerequisites

- Podman with `podman compose` (or `podman-compose`)
- `openssl` (for secret generation)
- Optional for public HTTPS: Caddy (or nginx) on the host
- DNS only if you use a public hostname

## Database security (production)

| Env | Purpose |
|-----|---------|
| `SURREALDB_PASSWORD` | Strong **root** password for bootstrap only (install generates) |
| `SURREALDB_APP_USER` / `SURREALDB_APP_PASSWORD` | Runtime DB user (default `scuffed_app`) — **EDITOR**, not root. **Must differ from root password** when `PRODUCTION=1` |
| `SURREALDB_AUTH_MODE` | `scoped` (default in prod) — root bootstrap (migrate + ensure user), then app reconnects as EDITOR |
| `SURREALDB_BOOTSTRAP` | Default: bootstrap on start (single-container). Set `SURREALDB_BOOTSTRAP=0` for **app-only** containers that must never use root |
| `SURREALDB_MIGRATE_ONLY=1` | Server runs root bootstrap (`Database::bootstrap_from_env`) then **exits** — for init/migrate jobs. `compose.yml` does not forward it (the service would exit and restart in a loop); run it as `podman compose --env-file data/secrets.env run --rm --no-deps -e SURREALDB_MIGRATE_ONLY=1 site-server` (without `--no-deps`, compose starts and may recreate `surrealdb` first) |
| `ENCRYPTION_KEY` | **Required** for remote DB — OAuth IDs, Nostr keys, DM at rest (AES-256-GCM + AAD) |
| `ENCRYPTION_KEY_VERSION` | Current key version (default `1`) |
| `ENCRYPTION_KEY_PREVIOUS` | Optional `ver:base64,ver:base64` for rotation reads |
| `CRYPTO_STRICT_AAD=1` | Disable empty-AAD legacy decrypt (on by default when `PRODUCTION=1`) |
| `NOSTR_CHALLENGE_SECRET` | **Required** outside dev — MAC key for Nostr login challenge tokens; boot **refuses** without it (no public dev-key fallback) |
| `PRODUCTION=1` | **Required** for remote SurrealDB; secure cookies; no plaintext DMs |

`scripts/install.sh` writes `PRODUCTION=1`, `SURREALDB_AUTH_MODE=scoped`, a random `NOSTR_CHALLENGE_SECRET`, and **distinct** root + app passwords.  
Remote boot **refuses** if `PRODUCTION` or `ENCRYPTION_KEY` is missing; the server also **refuses to boot** outside dev if `NOSTR_CHALLENGE_SECRET` is missing/empty.  
When `PRODUCTION` is set, an unset or blank `SURREALDB_URL` is a hard error (exit 1, clear message). The process does not start an in-memory database, does not seed a dev admin, and does not serve `/api/dev/login`. Local dev leaves both `PRODUCTION` and `SURREALDB_URL` unset.  
In production scoped mode, missing or root-equal `SURREALDB_APP_PASSWORD` is a hard error (no silent fallback).

Never ship with `root`/`root`. Migrations run as root during bootstrap only; the long-lived app uses a database-scoped **EDITOR** user.

## Stats upload limits

`POST /api/stats/upload` has two always-on rate limits. Per client IP: 40 requests, then one more every 2 seconds. That budget is this route's own. It is looser than the per-token limit, so a few devices on one home IP hit the token limit first. Avatar and image uploads stay at 8 requests, then one every 10 seconds. The key is the trusted-proxy client IP. Over the IP limit the response is `429` with `Retry-After` (seconds), `Cache-Control: no-store`, and `{"code":"rate_limited","error":"rate_limited","retry_after":N}`. Per daemon token, after the token is validated: 10 requests, then one more every 6 seconds. The bucket is the token id. Unknown bearers do not create buckets, and idle buckets are pruned every 60 seconds. Over that limit the response is `429` with `Retry-After`, `Cache-Control: no-store`, and `{"code":"rate_limited"}`. Both 429 bodies include `code`, so a client can branch on that field alone. The batch-cap `413` also sends `Cache-Control: no-store`.

New daemon tokens expire 180 days after they are created. A token already stored with `expires_at` NONE keeps working. An expired token on upload or daemon-config is `401` `{"code":"token_expired"}`. Token-check and pack downloads still use one shared unauthorized body. These two limits are always on. They do not follow `STATS_UPLOAD_BATCH_CAP`.

`STATS_UPLOAD_BATCH_CAP` is off when unset, empty, or `0` / `false` / `no` / `off` (any case). Any other value rejects one upload with more than 200 matches or more than 500 `deleted_sessions` before any write: `413` `{"code":"batch_too_large","max_matches":200,"max_deleted_sessions":500}`. While the flag is off, oversized batches are stored as they are today.

Leave the flag off until a tracker release that sends at most 200 matches and 500 deletions, and that handles `413`, `429`, and `token_expired`, is out and most members are on it. Today's tracker puts every unsynced game in one request. Compose passes the variable through as `STATS_UPLOAD_BATCH_CAP` (blank by default). The process reads it on each upload.

### Split migrator vs app (optional multi-container)

Single-container install keeps default bootstrap on every start (root session is short-lived, then EDITOR). For a stricter split:

1. **Init job:** `SURREALDB_MIGRATE_ONLY=1` (needs root + app passwords) — migrate, ensure app user, exit. With this compose file: `podman compose --env-file data/secrets.env run --rm --no-deps -e SURREALDB_MIGRATE_ONLY=1 site-server`.
2. **App:** `SURREALDB_BOOTSTRAP=0` + app credentials only — never signs in as root.

Kubernetes is out of scope. **Quadlet** (systemd-native containers) is an optional later migration if you want boot integration without Compose — no Quadlet units ship yet.

## Prebuilt images (recommended)

GitHub Actions publishes the `site-server` image to GHCR only after CI passes on `main`. The **Publish image** workflow runs when a CI run for a push to `main` completes, and builds the newest `main` commit whose CI succeeded. It skips a run when `:main` already carries that commit or a newer one, or when nothing the image is built from (`Cargo.*`, `Containerfile`, `Dioxus.toml`, `crates/`, `scripts/build.sh`, `scripts/site-entrypoint.sh`, the workflow) changed since the commit `:main` was built from. So not every commit gets an image:

| Tag | Image |
|-----|--------|
| `main` / `latest` | `ghcr.io/frozentear/scuffed-crew:main`, the newest published commit |
| commit | `ghcr.io/frozentear/scuffed-crew:sha-<short>`, only for commits that were published. A skipped commit (docs-only, red CI, or overtaken by a newer green commit) has no `sha-` tag |

Check what is live: `curl -s https://ow.scuffedcrew.no/api/health` returns `{"status":"ok","git_sha":"<commit>"}`, and the response carries the same commit in the `x-scuffed-git-sha` header. `unknown` means an image built without the commit stamp (a local `--build`, or an image published before this was added).

**Do not compile on the VPS** unless you must. First CI run can take a while; later runs use Buildx cache.

### First-time package visibility

1. After the workflow **Publish image** succeeds once, open  
   `https://github.com/users/FrozenTear/packages` (or the package linked from the Actions run).
2. Package settings → **Change visibility** → **Public** (simplest for a single VPS),  
   **or** keep private and on the VPS: `podman login ghcr.io` (PAT with `read:packages`).

### Day-to-day update (minutes, not an hour)

```bash
cd /path/to/scuffed-crew
./scripts/update.sh
# = git pull --ff-only + harden secrets.env (if needed) + podman pull + recreate site-server
```

`update.sh` **appends** missing production keys to an existing `data/secrets.env` (same as re-running `install.sh`): `PRODUCTION`, `SURREALDB_AUTH_MODE`, `SURREALDB_APP_USER`, `SURREALDB_APP_PASSWORD`. It never overwrites existing values and never regenerates `ENCRYPTION_KEY`. If the update script itself changes on pull, it re-execs once so the new logic runs immediately.

Before compose runs, `scripts/lib/compose-env.sh` fills a blank `ALLOWED_ORIGINS` from `REDIRECT_BASE_URL` and a blank `SURREALDB_APP_PASSWORD` from `SURREALDB_PASSWORD`. `compose.yml` uses one substitution level for those. podman-compose 1.5 turns a nested default into the outer value plus a stray `}`, or into literal text when the outer name is unset. A missing app-password key is still generated distinct from the root password. Production still refuses to boot when the two passwords match.

On Contabo-style day-2 hosts, if a project Surreal is already Up, `update.sh` recreates site-server with `up -d --no-deps` so `/bin/podman-compose` does not spawn an underscore-named DB twin on the same volume (see Troubleshooting: duplicate Surreal `_` vs `-`).

`update.sh` never creates or recreates the `surrealdb` container while one exists or its volume holds data. It starts `site-server` with `--no-deps` (starting a stopped database container as it is), and it stops with an error when the database container runs another image than `compose.yml` names. A new SurrealDB tag in `compose.yml` is applied only by [Upgrading the SurrealDB server](#upgrading-the-surrealdb-server). `SCUFFED_ALLOW_DB_IMAGE_MISMATCH=1 ./scripts/update.sh` updates the app and leaves the database on its current image.

### Which variables reach the server

`compose.yml` has no `env_file:`. `data/secrets.env` only fills in the `${...}` values, so a variable reaches the server only when it is listed under `site-server` → `environment:`. Setting an unlisted name in `secrets.env` does nothing. `scripts/check-compose-env.sh` (CI) fails when the server reads a name that compose does not pass, apart from a short allow-list (`PORT`, `SURREALDB_MIGRATE_ONLY`, and the test-only `SCUFFED_EXTRA_INDEX`).

Optional names passed through as blank when unset. For each one the server treats blank the same as unset:

| Name | Effect when set |
|---|---|
| `ENCRYPTION_KEY_PREVIOUS` | Older keys (`ver:base64,ver:base64`) for reading rows sealed before a key rotation. A malformed entry or a version equal to `ENCRYPTION_KEY_VERSION` stops the server at boot |
| `CRYPTO_STRICT_AAD` | `1` disables the empty-AAD legacy decrypt (already off when `PRODUCTION` is truthy) |
| `ALLOW_PLAINTEXT_PROVIDER_IDS` | `1` allows cleartext OAuth ids on a remote DB, non-production only. No effect with `PRODUCTION=1` |
| `SURREALDB_BOOTSTRAP` | `0`/`false`/`no`/`off` skips root bootstrap, so migrations do **not** run on start. Use only with a separate migrate job |
| `FORCE_APP_PASSWORD_SYNC` | `1` re-defines the app DB user's password on every boot |
| `CSP_ENFORCE`, `CSP_EXTRA_CONNECT_SRC`, `CSP_IMG_SRC` | See [Public settings in the HTML shell](#public-settings-in-the-html-shell) |
| `LEADERBOARD_CACHE_TTL_SECS`, `LEADERBOARD_CACHE_MAX_SCANS` | See [Public leaderboard cache](#public-leaderboard-cache) |
| `DISCORD_WEBHOOK_OFFICERS`, `DISCORD_WEBHOOK_GENERAL`, `DISCORD_WEBHOOKS_ENABLED` | Discord notifications. A blank URL skips that channel. `DISCORD_WEBHOOKS_ENABLED=0` turns both off |
| `MATRIX_HOMESERVER_URL`, `MATRIX_BOT_ACCESS_TOKEN`, `MATRIX_OFFICER_ROOM_ID`, `MATRIX_GENERAL_ROOM_ID` | Matrix notifications. Off unless all four are non-empty |

Before these names were forwarded (up to rc.5), they were ignored even when present in `secrets.env`. If one of them is already in your `secrets.env`, it takes effect on the next `update.sh`. Check them first: [Phase A](#phase-a-app-update-database-unchanged).

Override image pin in `data/secrets.env` if needed:

```bash
SITE_SERVER_IMAGE=ghcr.io/frozentear/scuffed-crew:main
# or a specific sha:  .../scuffed-crew:sha-abc1234
```

### Build from source (fallback only)

```bash
BUILD_FROM_SOURCE=1 ./scripts/install.sh
# or, with a database container already running:
#   podman compose --env-file data/secrets.env up --build -d --no-deps site-server
```

## Troubleshooting

### SurrealDB: `Permission denied` creating RocksDB directory

The official image runs as a non-root user; Podman volumes are often root-owned. `compose.yml` sets `user: "0:0"` on the Surreal service for single-tenant VPS installs.

If you still see the error on an old volume:

```bash
podman compose --env-file data/secrets.env down
# optional: remove only if you can lose empty/broken DB data
# podman volume rm scuffed-crew_surrealdb-data
podman compose --env-file data/secrets.env up -d
```

This `up -d` creates the database container on the image `compose.yml` names. If an older SurrealDB wrote that volume, that is an upgrade with no backup and no rollback: follow [Upgrading the SurrealDB server](#upgrading-the-surrealdb-server) instead.

### SurrealDB: `is unhealthy` but logs show “Started web server”

The database is fine; an old probe had a 5 s timeout, shorter than the time the `surreal` CLI waits before it exits. Current `compose.yml` has a Surreal healthcheck: `/surreal is-ready --endpoint http://127.0.0.1:8000` every 30 s, with a 20 s timeout, 3 retries and a 20 s start period. `site-server` still only waits for the database container to start, not for it to be healthy. The probe belongs to the database container, so a fix only takes effect when that container is recreated, and `./scripts/update.sh` never does that. If `compose.yml` names the image the container already runs (`podman inspect <surreal container> --format '{{.ImageName}}'`), stop site-server, then `podman compose --env-file data/secrets.env up -d --no-deps surrealdb` recreates it on the same image and volume, then `./scripts/update.sh`. If the images differ, that is a database upgrade: follow [Upgrading the SurrealDB server](#upgrading-the-surrealdb-server).

### SurrealDB: duplicate container (`_` vs `-`) and site-server stuck Created / public 502

On some hosts (notably Contabo with `/bin/podman-compose`), Compose-v2 names the long-lived DB `scuffed-crew-surrealdb-1` (volume `scuffed-crew_surrealdb-data`). A later `compose up -d site-server` that honors `depends_on` can create an underscore twin `scuffed-crew_surrealdb_1` on the **same** RocksDB volume. The twin exits (`Exit 1`); site-server can sit in `Created` and the public site returns 502 until the app is started with `--no-deps` and the duplicate is stopped.

This is **name-skew**, not a `SURREALDB_URL` typo. The compose service remains `surrealdb` (`ws://surrealdb:8000`). Do **not** rename the service or volume key, and do **not** pin `container_name` without a planned cutover.

**Never `podman volume rm` the Surreal data volume for this symptom.** The twins share `scuffed-crew_surrealdb-data`; removing it destroys production data.

`scripts/update.sh` detects a running project Surreal (`${PROJECT_NAME}-surrealdb*`, `${PROJECT_NAME}_surrealdb*`) and starts site-server with `up -d --no-deps` so the existing DB is left alone. Prefer the hyphen name (`…-surrealdb-1`) when both exist. After the app is healthy, the script stops extras and tries `rm -f`; if compose blocks the rm, the duplicate is left `Exited`.

The same twin starts from a relay bring-up. `podman compose --profile relay up --build -d strfry` follows `depends_on` and podman-compose creates `scuffed-crew_surrealdb_1` on the volume already held by `scuffed-crew-surrealdb-1`. RocksDB's `LOCK` file is what stopped both processes from writing. That is not a guarantee: if the live process is down, the twin can open the database. `strfry` has no `depends_on`, and `scripts/relay-up.sh` always passes `--no-deps`. It refuses to run unless a project Surreal container is already Up, and it does not create one.

Do not pin `container_name` or the compose project `name` to force one container name. A hyphen `container_name` makes `podman create` fail when `scuffed-crew-surrealdb-1` already exists, so that host does not get a second writer. On a host whose only database is `scuffed-crew_surrealdb_1`, the same pin starts a new hyphen container on the same volume. `x-podman.name_separator_compat` makes podman-compose use hyphens for containers and also for networks (`scuffed-crew-internal` instead of the existing `scuffed-crew_internal`), so the relay would not see the running database. Volume names are already `${project}_surrealdb-data` in both tools. That shared name is why a second container is dangerous. `scripts/update.sh` keeps using `--no-deps` for site-server when a project Surreal is Up.

Manual recovery if an older `update.sh` already created the twin:

```bash
podman ps -a --format '{{.Names}} {{.Status}}' | grep -E 'surrealdb|site-server'

# Stop the underscore twin; keep scuffed-crew-surrealdb-1
podman stop scuffed-crew_surrealdb_1
# rm if compose will allow it; otherwise leave Exited
podman rm -f scuffed-crew_surrealdb_1 || true

# Recreate only the app — do not recreate Surreal
COMPOSE_PROJECT_NAME=scuffed-crew podman-compose --env-file data/secrets.env up -d --no-deps site-server
# or: podman compose --env-file data/secrets.env up -d --no-deps site-server
```

### Locked out of admin (no actionable admin left)

Break-glass root-level DB recovery when the last admin is deactivated, demoted, suspended/banned, or lost: `docs/notes/last-admin-recovery.md`.

### Pruning `audit_log` (GDPR / right-to-erasure)

`audit_log` is append-only; a targeted prune needs a root-level drop/re-add of the guard event — procedure in `docs/notes/audit-log-ops.md`.

## Happy path (novice)

```bash
git clone <repo> scuffed-crew && cd scuffed-crew
./scripts/install.sh
```

What install does:

1. Creates **`data/secrets.env`** (mode `600`) if missing:
   - Random **SurrealDB** password (you never type this day-to-day)
   - Random **encryption key**
   - Free **`HOST_PORT`** (tries 3000, 8080, … then a high random port)
   - Optional prompt for public site URL → `REDIRECT_BASE_URL`
2. Runs `podman compose --env-file data/secrets.env up --build -d` on a new host. On a host that already has a database container it starts only `site-server`, with `--no-deps`, and stops if that container runs another image than `compose.yml` names
3. Prints the bound address: `127.0.0.1:$HOST_PORT`

Then:

1. Open the URL (or set up Caddy — see below).
2. **First visit:** create the **admin account** (username + password ≥ 12 chars). That password is only stored as an Argon2 hash in the DB.
3. Later: sign in at `/login` with that username/password.

### What you should remember

| Secret | Who sets it | Where it lives |
|--------|-------------|----------------|
| Admin password | You, in the browser at first boot | Password manager (hash in DB only) |
| Database password | Install script | `data/secrets.env` (for backups / recovery) |
| Host port | Install script | `HOST_PORT` in `data/secrets.env` — **stable across updates** |

Re-running install **does not** regenerate secrets or re-roll the port if `data/secrets.env` already exists. Pulling new images / rebuilds keeps the same port.

## Public URL while apex domain is busy

The stack binds **`127.0.0.1:HOST_PORT` only** — it does not take over port 80/443.

Options:

- **SSH tunnel:** `ssh -L ${HOST_PORT}:127.0.0.1:${HOST_PORT} user@vps` then open `http://127.0.0.1:${HOST_PORT}`
- **Subdomain:** e.g. `app.scuffedcrew.no` → Caddy reverse_proxy to `localhost:HOST_PORT`
- **Different host port:** edit `HOST_PORT` in `data/secrets.env` and recreate `site-server`

### Public hostname: `ow.scuffedcrew.no` (same idea as `news.scuffedcrew.no`)

Compose stays on **127.0.0.1:HOST_PORT**. Host **Caddy** terminates TLS and proxies, like your other subdomains.

**1. DNS** (wherever `scuffedcrew.no` is managed — same place as `news`):

| Type | Name | Value |
|------|------|--------|
| A | `ow` | your VPS public IPv4 |
| AAAA | `ow` | IPv6 if you use it for news |

Wait until `dig +short ow.scuffedcrew.no` returns the VPS.

**2. Caddy** — add a site block next to `news.scuffedcrew.no` (path is often `/etc/caddy/Caddyfile`):

```caddy
ow.scuffedcrew.no {
	encode zstd gzip
	# Same-origin Nostr relay. See "Optional Nostr relay" below and deploy/Caddyfile.
	handle /relay* {
		header X-Frame-Options "DENY"
		header Referrer-Policy "strict-origin-when-cross-origin"
		header X-Content-Type-Options "nosniff"
		header Permissions-Policy "camera=(), microphone=(), geolocation=()"
		header Strict-Transport-Security "max-age=63072000; includeSubDomains; preload"
		uri strip_prefix /relay
		reverse_proxy 127.0.0.1:7777
	}
	handle {
		reverse_proxy 127.0.0.1:HOST_PORT   # from data/secrets.env on the VPS
	}
}
```

`X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY` and `Referrer-Policy: strict-origin-when-cross-origin` are set by scuffed-server on every response it serves. Do not add them for the site in Caddy. A second line is sent beside the origin header. Only the `/relay*` block sets these headers in Caddy, because strfry answers that path: the three above plus `Permissions-Policy` and the production `Strict-Transport-Security` value, all copied from `crates/server/src/security.rs`. If the live host file still has site-wide `header X-Content-Type-Options`, `header X-Frame-Options` or `header Referrer-Policy` lines, move or delete them as above and reload Caddy.

Do not add a Caddy `Cache-Control` header for `*.wasm` / `*.js` / `*.css`. The origin is the only cache policy (`cache_control_value` in `crates/site-server`): Dioxus `dxh`-hashed assets are `public, max-age=31536000, immutable`, unhashed `.js` / `.css` / `.wasm` are `no-cache`, other static files are one day, and the HTML shell is `no-cache`. A matcher on every `.js` / `.css` / `.wasm` path also covers unhashed files and the HTML fallback, and Caddy emits a second `Cache-Control` beside the origin's.

`deploy/Caddyfile` is the repo template. Changing it does not edit the live host file (often `/etc/caddy/Caddyfile`). Copy this block there, drop any leftover `X-Content-Type-Options` line, and reload Caddy yourself.

```bash
# on VPS
grep '^HOST_PORT=' /root/github/scuffed-crew/data/secrets.env
# put that number in reverse_proxy, then:
caddy validate --config /etc/caddy/Caddyfile
systemctl reload caddy
# or: caddy reload --config /etc/caddy/Caddyfile
```

Template also lives in repo: `deploy/Caddyfile`.

### Public settings in the HTML shell

`GET /`, `GET /index.html`, and every client route that serves the SPA shell (`/strategies/foo`, `/admin/settings`, …) include the anonymous settings JSON immediately before `</head>`:

```html
<script id="sc-settings" type="application/json">{"id":"…","org_name":"…"}</script>
```

That object is the body of anonymous `GET /api/settings` (`load_anonymous_settings` mapped with `to_api_settings`, then `serde_json` — the same serializer as the JSON response). `<`, `>`, `&`, U+2028, and U+2029 are escaped as `\u003c`, `\u003e`, `\u0026`, `\u2028`, and `\u2029`, so a settings string cannot close the script element. The script block is cached with the blob. The rewritten head prefix is computed once for that blob and reused. The shell response stays `Cache-Control: no-cache`. `type="application/json"` is not executed (`script_type_is_executable` skips it), so it does not change the CSP script hashes.

The same rewrite fills `<title>`, `og:title`, and `og:site_name` from `org_name`, and the description / `og:description` meta tags from `site_description`, when those tags are already in `index.html`. Inserted values are trimmed, then HTML-escaped. A blank `org_name` or `site_description` (empty after trimming) leaves that field's template text in place; the other field is still rewritten. If settings are unavailable the template text is left as built.

`index.html` is read when the process starts and kept in memory. After rebuilding `dist/`, restart site-server so the new shell is what gets served. A running process keeps the previous file until then.

The blob lives in memory on each server process for 10 seconds (`PUBLIC_SETTINGS_TTL` in `crates/site-server`). A `PUT /api/settings` or first-boot setup write drops it immediately and drops it again when the write returns, including when the write fails. A shell read that overlaps the save may include the row it just read in that response, but the row is not kept: the store is skipped when the generation has already moved, and the second drop clears a store that landed before the handler returned. A later request cannot serve that row as the stale fallback. A read that is still running after the cap can still fill the cache when it finishes, if the generation it started with is still current. That store's TTL starts when the read started, not when it finished. A timeout records its backoff against that starting generation, so a save during the wait does not suppress the next one. A direct database edit that bypasses those routes is not invalidated; the shell can keep the previous settings for at most about 10 seconds plus the 300 millisecond cap, because the TTL is measured from when the read started. A timed-out or failed re-read can still serve the last blob from the current generation. The embed read waits at most 300 milliseconds (`EMBED_SETTINGS_TIMEOUT`). That cap includes waiting behind an in-flight read that has not yet passed it. Once a refresh is already in flight past the cap, or refreshes are backing off, a miss returns immediately with the stale blob or with no embed. On timeout or error the page is still served, with that blob if one exists and without the `sc-settings` block otherwise. The cache is per process: a restart clears it, and two app instances do not share it.

A missing file under `/assets/`, or a missing top-level file whose extension is a real static type (`js`, `mjs`, `css`, `wasm`, `map`, `svg`, `png`, `jpg`, `jpeg`, `webp`, `gif`, `avif`, `ico`, `woff`, `woff2`, `ttf`, `json` — for example `/favicon.ico` or `/foo.wasm`), is `404` with `Cache-Control: no-store` and a plain-text body. A multi-segment path outside `/assets/` is a client route even when the last segment looks like a file (`/wiki/config.json`, `/blog/foo.png`, `/articles/v1.2.png`, `/wiki/foo.bar`).

Unmatched `/api` and `/api/*` requests return `404` with `Content-Type: application/json`, body `{"error":"Not found"}`, and `Cache-Control: no-store` (HEAD gets the same status and headers with no body; OPTIONS is answered by the CORS layer). A wrong method other than OPTIONS on a known API path stays `405`. OPTIONS on a route registered inside `create_router` is answered by the CORS layer with `200`. `scuffed-server` adds the strategy routes (`/api/strategy/strategies`, `/api/strategy/strategies/mine`, `/api/strategy/strategies/{id}`, `/api/strategy/heroes`, `/api/strategy/meta`, `/api/strategy/patch-notes`, `/api/strategy/patch-notes/{version}`), the chat routes (`/api/chat/auth-token`, `/api/chat/send-encrypted`, `/api/chat/send-public`, `/api/chat/decrypt`), and the websocket route (`/api/strategy/ws`) after `create_router`. OPTIONS on those still returns `405`. `/apiary` and `/api-docs` are client routes.

The app sets enforcing `Content-Security-Policy` itself (same-origin scripts,
Google Fonts, Discord/Google avatar hosts, and `NOSTR_RELAY_URL` for chat
sockets). `script-src` does not allow `'unsafe-eval'`. Leave CSP off the Caddy
block so a second policy does not intersect. `CSP_EXTRA_CONNECT_SRC` and
`CSP_IMG_SRC` add relay or image origins without a code change.

If something breaks under the enforcing policy (an admin-set external image,
a second chat relay), set `CSP_ENFORCE=0` in `data/secrets.env` and recreate
the app container (`./scripts/update.sh`). The same policy is then sent as
`Content-Security-Policy-Report-Only`, so the browser console names the
blocked origin. Add that origin to `CSP_IMG_SRC` or `CSP_EXTRA_CONNECT_SRC`,
then remove `CSP_ENFORCE=0` to enforce again.

> **Optional: cache the ICS feeds at the edge.** `/api/calendar/all.ics` and
> `/api/calendar/team/{id}` run a full event list plus a settings read per hit,
> and they already send `Cache-Control: public, max-age=3600` — which only does
> anything if something upstream honors it. The server-side per-IP governor
> (NS2-6) is the floor that holds regardless, but if calendar clients ever
> generate real load, caching the response in Caddy is the cheaper lever:
>
> ```caddy
> # inside the site block, before reverse_proxy — requires the cache-handler plugin
> @ics path /api/calendar/*.ics /api/calendar/team/*
> route @ics {
> 	cache
> }
> ```
>
> Without the plugin, the governor alone is sufficient for this org's scale.

> **Rate limiting & `X-Forwarded-For`.** Auth, upload, and public rate
> limiters key off the client IP. Forwarded headers are trusted only when the
> TCP peer is loopback, or is listed in `TRUSTED_PROXIES` (comma-separated IPs
> or CIDRs in `data/secrets.env`). **The default is loopback only.** Private
> ranges are not trusted, so a LAN client or another container cannot rotate
> `X-Forwarded-For` into a fresh bucket.
>
> **This deploy's hop.** Host Caddy (`deploy/Caddyfile`) reverse-proxies to
> `127.0.0.1:HOST_PORT`. Compose publishes that port into `site-server`.
> Rootful Podman presents the connection *inside* the container as the
> compose-network **gateway** (the bridge address, commonly `10.89.x.1` — one
> per attached network), not as the browser and not as `127.0.0.1`. Caddy's
> `reverse_proxy` sets `X-Forwarded-For` to the real client. `TRUSTED_PROXIES`
> must be those gateway addresses or every visitor shares one bucket.
>
> `scripts/install.sh` and `scripts/update.sh` run
> `scripts/ensure-trusted-proxies.sh`, which writes the live gateway into
> `data/secrets.env` when the key is missing or empty. A value you already set
> is left alone. Loopback stays trusted in addition to whatever you list, so a
> forwarder that shows up as `127.0.0.1` still works.
>
> Keep the publish bound to `127.0.0.1:HOST_PORT`. Do **not** set
> `TRUSTED_PROXIES` to `10.0.0.0/8` or any whole private range — that trusts
> every private peer again. An extra public CDN in front of Caddy must be
> listed by its egress IP as well, or its traffic shares one bucket.
>
> **Contabo after this change.** `./scripts/update.sh` discovers the gateway
> and recreates `site-server` (volumes stay). Then confirm:
>
> ```bash
> cd /root/github/scuffed-crew
> grep '^TRUSTED_PROXIES=' data/secrets.env
> # startup log line: rate-limit trusted proxies
> podman logs --tail 80 "$(podman ps -q --filter name=site-server | head -n1)" | grep 'trusted proxies'
> ```
>
> Manual path, if you are not using the update script — read the gateway, do
> not guess it:
>
> ```bash
> cd /root/github/scuffed-crew
> cid=$(podman ps -aq --filter name=site-server | head -n1)
> podman inspect "$cid" --format '{{range .NetworkSettings.Networks}}{{.Gateway}} {{end}}'
> # data/secrets.env:
> #   TRUSTED_PROXIES=<those IPs, comma-separated>
> podman compose --env-file data/secrets.env up -d --no-deps site-server
> ```
>
> The peer has to match. During
> `curl -sS -o /dev/null "http://127.0.0.1:${HOST_PORT}/api/health"`,
> `podman exec <site-server> ss -tn '( sport = :3000 )'` shows the source
> address. It must be `127.0.0.1` or one of the `TRUSTED_PROXIES` values.

> **WebSocket caps.** `/api/strategy/ws` has one admission limit. The client
> IP is resolved with the same trusted-proxy rules as the HTTP limiters
> (`TRUSTED_PROXIES` / loopback). Do not key this cap on the raw container
> peer or every visitor behind Caddy shares one bucket.
>
> | Env | Default | Meaning |
> |---|---|---|
> | `WS_MAX_CONNECTIONS` | `512` | Strategy sockets open at once, held until the socket closes |
> | `WS_MAX_PER_IP` | `32` | Concurrent strategy sockets for one client IP (several players, multiple tabs) |
>
> Over-cap upgrades are rejected with `Retry-After: 1` (503 when the global
> cap is full, 429 when the per-IP cap is full). A strategy socket that has
> not sent JoinRoom within 10 seconds is closed.
>
> Set the two variables in `data/secrets.env`. Compose passes them through.
> Leave them unset to keep the defaults.

**3. App public URL** (required for cookies / redirects).

`REDIRECT_BASE_URL` is the public origin: scheme, host, and port. A path or query on it is ignored. Chat AUTH then allows this site's relay at `wss://<host>/relay` when the base is `https`, or `ws://<host>/relay` when the base is `http`. `http` and `https` relay tags are not signed. The relay path is always `/relay`.

```bash
cd /root/github/scuffed-crew   # your clone path
# edit data/secrets.env:
#   REDIRECT_BASE_URL=https://ow.scuffedcrew.no
#   ALLOWED_ORIGINS=https://ow.scuffedcrew.no
#   # Blank ALLOWED_ORIGINS is treated as unset and falls back to REDIRECT_BASE_URL
#   # (F-API-004). Do not leave it empty thinking it means "allow all".
#   NIP05_DOMAIN=ow.scuffedcrew.no      # see "NIP-05 domain" below

podman compose --env-file data/secrets.env up -d --force-recreate --no-deps site-server
```

**4. From home PC:** open `https://ow.scuffedcrew.no`  
First visit → create admin if `setup-status` still needs setup.

**HTTPS note:** release builds set **Secure** cookies. Use the `https://` subdomain above; plain `http://IP:port` often won’t keep login.

## Public leaderboard cache

`GET /api/public/leaderboards` keeps an in-process cache so the full `personal_match` scan does not run on every request. The default lifetime is 30 seconds (`LEADERBOARD_CACHE_TTL_SECS` in `data/secrets.env`). Values below 5 or above 300 are clamped. A blank or non-numeric value uses 30. The cache is per process: a restart clears it, and two app instances do not share it.

Each stored entry stays fresh for `TTL` times a factor chosen once in the inclusive range 0.8 to 1.2. Keys filled together after a restart therefore do not all expire on the same tick. Past that window the handler keeps serving the previous board, with its original `cached_at`, while exactly one background refresh runs for that key. A request waits for a scan only when the key is cold, or when the stored board is older than 10 TTLs. A failed refresh is not stored, so the previous board stays until a later refresh succeeds.

At most `LEADERBOARD_CACHE_MAX_SCANS` scans run at once (default 2, clamped to 1..=8). Background refreshes may use at most one fewer slot than that cap, and always at least one. They try once and do not wait. If no refresh slot is free, the key joins a queue ordered by board age, bounded by the key cap. When a scan finishes, that slot is handed to the oldest key still waiting, so refreshes run back to back while a slot is free. A request does not sit on the 10 TTL cap while a refresh slot is idle and a stale key is queued. If nothing hands the key a slot, the next request-driven try waits a jittered backoff between 0.25 and 1 times the TTL, so skipped keys do not all retry on the same millisecond. Cold keys and boards older than 10 TTLs may use any free slot, including the one a refresh cannot take, so they are not queued behind background refreshes. A blocking read whose refresh is already running joins that one scan.

One scan covers a whole season. The query groups `personal_match` by member and hero once. Every hero, every metric (winrate, kd, games), and every limit is a sort and a truncate of that result. The public page's hero list is 54 names, so those filters share one cache entry per season instead of 54 scans. With the default cap of 2, filling 54 separate hero keys at about 1.4 seconds per pair took about 40 seconds, and the last cold reader waited about 19 seconds. After this change that wait is one grouped scan of the same table. A single-hero scan in that harness was about 1.4 seconds. The grouped query does the same pass with a wider `GROUP BY`. Even at a few times that cost the wait stays under 19 seconds, and the aim is one scan (about 5 seconds or less).

Steady state for one hot season is about one TTL plus that scan. A quiet rerun of the previous commit measured a 135 second median and a 234 second p99 (4 to 10 times a 30 second TTL) because every skipped key retried on the same half-TTL tick, one won, and the rest waited another 15 seconds. Only 11 scans ran in 90 seconds while Surreal sat at 30% CPU. The handoff removes that idle gap. If many seasons are hot at once, age grows with how long the refresh slots take to walk the queue. `LEADERBOARD_CACHE_MAX_SCANS` and `LEADERBOARD_CACHE_TTL_SECS` are still the knobs. The 10 TTL cap still forces a wait, so a board is not served older than that.

`limit`, `metric`, and `hero` are applied in memory from the season snapshot. The JSON `rows` array is cut to the limit the caller asked for (clamped to 1..=100). Two limits, two heroes, or two metrics on the same season share one `cached_at`. An unusual limit does not add another database scan.

Freshness is that TTL only. A stats upload, an edit, or a deleted session does not clear the cache. Clearing on every upload dropped boards that were still loading once uploads arrived faster than the scan, so a busy process never stored a result. A member's new game shows up after the fresh window, on the refresh that follows it. A failed database read is not stored. If a previous board for the same query is still in memory, the handler may serve it instead of a 500.

The JSON body is an object. `rows` has the same member fields as the old array (`member_id`, `display_name`, `games`, `winrate`, `kd`). `cached_at` is RFC 3339 UTC, the time this process started the query that produced `rows`. The site uses it to show how long ago the board was computed, and refreshes that label while the page is open. A cache hit, and a stale board served during refresh, both repeat that timestamp. A response with no `cached_at` still renders the rows; the site hides the label.

The cache key for this route is the season id and a public audience tag. Metric, limit, and hero are projections of the snapshot stored under that key. This route does not take a role or game-mode filter. Anonymous and logged-in callers see the same rows, and inactive members stay off the board. The audience tag is there so a crew-only board cannot be stored in a slot an anonymous caller reads.

## Power-user path

Copy `.env.example` → `data/secrets.env` (or `.env`), set values yourself, then (first install only; later, use `update.sh` or `up -d --no-deps site-server`):

```bash
podman compose --env-file data/secrets.env up --build -d
```

## Optional Nostr relay

The relay is off until you opt in. Browsers use same-origin `/relay`. That path
is a read gate: no NIP-42 AUTH means no events, a member only receives kind
1059 gift wraps `p`-tagged to their own key, team chat only if they are on that
team, and officer chat only if they are an officer on that team. The write
policy plugin is `/usr/local/bin/relay-policy` only. It checks the member
allowlist, the kind whitelist, and the per-pubkey rate limit. It does not
filter `REQ` results, so the read gate is a separate process in the same
container.

`relay-policy` and `relay-read-gate` are compiled from this repository as
static musl binaries. The runtime base is
`docker.io/dockurr/strfry` pinned by digest
`sha256:599ab3500dbfbe6cb78c668e1892cd9802c192d066df4660e8e3175034a8344d`
(Alpine 3.18, musl). A short name (`dockurr/strfry@sha256:...`) makes
`podman build` wait for a registry prompt, so the Containerfiles and
`compose.yml` use a registry host on every image (`docker.io/library/...`
for official images, `docker.io/surrealdb/surrealdb:v3.2.3` for SurrealDB,
`ghcr.io/...` for the site image). A glibc build of those plugins does not
load there. CI builds this image, seeds team channel rows the same way
`ensure_team_channel_rows` does, and walks the site chat path: NIP-42 AUTH
on the gate, `get_team_channels` (the query behind
`GET /api/teams/:id/channels`), a kind 9 `#h` subscription, a kind 9 publish
the way `POST /api/chat/send-public` does, then a kind 1059 gift wrap with a
`p` tag read back on a `#p` subscription. The kind 9 must come back on the
`#h` subscription.

Team chat does not need relay-signed group metadata (kinds 39000, 39001,
and 39002). The site learns the group id from `GET /api/teams/:id/channels`.
`ensure_team_channel_rows` writes those rows. The browser subscribes to kind
9 with an `h` tag for that id. Officer chat and DMs are kind 1059 gift wraps
filtered by `#p`. `subscribe_group_metadata` has no caller. Kind 9 delivery
uses the SurrealDB roster and `team_channel` rows.

`RELAY_POLICY_ENFORCE_GROUPS` defaults to true, in the image and in
Compose. An event with an `h` tag is stored only when that id is an active
`team_channel` and the author may write it. A public channel accepts any
active roster member of that team (the same rows as
`GET /api/teams/{id}/channels` and `send_public`). An officer channel
(`{team}-officers`) accepts only roster members whose org role is officer
or admin, the same `can_access_officer_channel` check the read gate uses.
Kind 1059 gift wraps have no `h` tag, so this check does not apply to them.
Kind 0, kind 7, and kind 34550 are signed with the member key and also have
no `h` tag. Set the variable to `false` to allow any allowlisted pubkey to
write any `h` group.

The write plugin and the read gate reload when `plays_on`, `team_channel`,
or `member` changes, using a SurrealDB LIVE query. Those statements are
`LIVE SELECT id FROM <table>`. The notice is only a reload trigger.
`member` holds `nostr_secret_key_encrypted`, and that field must not enter
the relay process. `plays_on` and `team_channel` have no secret columns.
They use the same `id` projection so the body is not shipped either.
Officer permission and the Nostr pubkey live on `member`, so a promotion
reloads too. A rejected write for an unknown author, an unknown group, or a
pubkey outside that group's writer set schedules the same reload, debounced
to at least 2 seconds, as a backup. `RELAY_POLICY_REFRESH_SECS` (default 60)
is the slow backup if LIVE is down.

Kind 1059 is not one rate bucket per recipient. The plugin keys
`gift:{sender}:{recipient}`. The sender is strfry's authenticated pubkey
when that connection completed NIP-42. site-server publishes straight to
`ws://strfry:7777` with no AUTH. `publish_event_oneshot` is defined in
`crates/chat/src/nostr/relay.rs`. Kind 1059 DMs call it from
`crates/site-server/src/routes/nostr.rs` (`POST /api/nostr/dm/send`).
Officer gift wraps use `RelayClient::publish_event` in
`crates/server/src/routes/chat.rs` (`send_encrypted`), also with no AUTH.
The wrap author is ephemeral, so each of those wraps has its own bucket and
does not consume a member's kind 9 budget. The gate keeps a second bucket,
`gift:{authed member}:{recipient}`, and refuses the EVENT before forward
when that member is over `RELAY_POLICY_RATE_LIMIT`. One signed-in member
cannot use up another member's inbox budget. The gate drops expired
`gift:` buckets on a 60 second timer. The write plugin does the same for
its rate buckets, including `gift:` keys.

Gift wraps that the site publishes on `ws://strfry:7777` do not pass
through the gate. The plugin bucket does not cap them, because each wrap
author is a new ephemeral key. The gate's
`gift:{authed member}:{recipient}` limit is the only cap on gift wraps a
browser can send. That holds only while container port 7777 stays on the
Compose network and is never published on a host port. `compose.yml` maps
host `127.0.0.1:7777` to the gate on container port 7779
(`127.0.0.1:7777:7779`). Publishing `7777:7777` would let a client reach
strfry directly and skip that cap.

The relay image has to be rebuilt before this plugin is what a running
container executes. `./scripts/relay-up.sh` does that (`up --build` recreates
strfry). Restarting a container built from an older image keeps the old
write plugin.

Host `127.0.0.1:7777` is the gate (container port 7779). Container port 7777
is strfry itself and stays on the Compose network, so site-server still uses
`ws://strfry:7777` for publish and its own subscriptions. Do not publish
`7777:7777`. A published strfry port would skip the gate, and gift wraps on
that path have no other cap.

The gate does not pipe HTTP through to strfry. A NIP-11 request
(`Accept: application/nostr+json`) is answered by the gate, then the connection
is closed. Any other non-upgrade request is `400` with `Connection: close`.
Only a WebSocket upgrade (`Upgrade` and `Connection` tokens, optional
whitespace) is forwarded, and it still requires NIP-42 AUTH before a `REQ`
or an `EVENT`. The AUTH pubkey must be an active member. A publish with no
member AUTH gets `OK false` and `auth-required:`, and that EVENT is not
forwarded to strfry.

Kind 4 plain DMs are rejected on write (the relay-policy plugin and the gate)
as well as on read. DMs are NIP-59 kind 1059 gift wraps. NIP-29 group metadata
(kinds 39000 through 39003) uses a `d` tag. Only an authenticated member of
the group named by that tag can read it. A signed-in member of another group
does not. Officer group metadata stays limited to officers or admins on that
team. That is the shape `GroupManager` publishes and `NostrFilter::group_metadata`
lists.

strfry only sees the gate's address (`127.0.0.1`), so a per-IP limit in
strfry does not apply to browsers. The gate limits concurrent connections
(default 32), new connections (default 60 per 60 seconds), and client messages
(default 240 per 60 seconds) per client IP. It trusts `X-Forwarded-For` only
when the TCP peer is loopback or listed in `RELAY_GATE_TRUSTED_PROXIES`.

Compose sets that variable with one substitution, `${TRUSTED_PROXIES:-}`.
`podman-compose` before 1.6.0 (the `/bin/podman-compose` builds on some VPS
images) does not expand a nested default such as
`${RELAY_GATE_TRUSTED_PROXIES:-${TRUSTED_PROXIES:-}}`. With the outer name
unset, those builds pass the literal `${TRUSTED_PROXIES:-}` into the
container. The gate logs `ignoring unparseable entry` for that value and
ignores it, so host Caddy is not a trusted proxy and every browser shares
the port-forwarder's bucket. Docker Compose and podman-compose both expand
the single-level form, so both pass the gateway list through.
`scripts/ensure-trusted-proxies.sh` fills `TRUSTED_PROXIES` with the
compose-network gateway (the address the container sees for host Caddy). A
direct client cannot pick its own IP by sending the header. If
`TRUSTED_PROXIES` is empty, every browser shares the port-forwarder's
bucket. Set `TRUSTED_PROXIES`. A `RELAY_GATE_TRUSTED_PROXIES` line in
`data/secrets.env` is not read.

### Enable

1. In `data/secrets.env` set:

   ```bash
   NOSTR_RELAY_URL=ws://strfry:7777
   ```

   `PRODUCTION=1` must already be set (install.sh writes it). The relay refuses
   to start when that flag is set and the SurrealDB password is missing,
   the credentials are root/root, or `SURREALDB_URL` would fall back to localhost.

2. Rebuild and start strfry (the image also starts the read gate). The main
   stack must already be up, so Surreal is the existing container and not a
   new one. Pull this change first. `relay-up.sh` rebuilds the image and
   recreates the container. A restart of the previous container does not
   load the new write plugin:

   ```bash
   ./scripts/relay-up.sh
   ```

   That runs `podman compose --env-file data/secrets.env --profile relay up --build -d --no-deps strfry`
   (or `podman-compose` when that is the provider) with `COMPOSE_PROJECT_NAME` set.
   If that up fails, the script prints the project name and exits 1.
   `--no-deps` is required.
   Without it, podman-compose also starts Surreal and can create
   `scuffed-crew_surrealdb_1` on the same volume as `scuffed-crew-surrealdb-1`.
   `strfry` has no `depends_on` for the same reason. Do not run
   `up --profile relay` with no service name on a day-2 host: that starts
   the default services, including Surreal, and can create the twin. See
   Troubleshooting: duplicate Surreal `_` vs `-`.

3. Recreate site-server so it picks up `NOSTR_RELAY_URL`:

   ```bash
   ./scripts/update.sh
   ```

4. Caddy: copy the `handle /relay*` block from `deploy/Caddyfile` into the live
   site block, before the catch-all. It is `uri strip_prefix /relay` and
   `reverse_proxy 127.0.0.1:7777`. Put the site `reverse_proxy` in its own
   `handle` so `/relay` is not also sent to site-server. Reload Caddy
   (`caddy validate` then `systemctl reload caddy`).

5. Check the relay from the server:

   ```bash
   curl -sS http://127.0.0.1:HOST_PORT/api/nostr/health
   ```

   `configured` must be true and `reachable` must be true. `configured` follows
   `NOSTR_RELAY_URL`. `reachable` means site-server got a NIP-11 response from
   `ws://strfry:7777`.

### Roll back

1. Set `NOSTR_RELAY_URL=` (empty) in `data/secrets.env`.
2. Stop strfry. Leave the volume in place:

   ```bash
   podman compose --env-file data/secrets.env --profile relay stop strfry
   ```

   Do not run `podman volume rm` on `strfry-data`.
3. Recreate site-server so it drops the relay URL: `./scripts/update.sh`.

Removing the Caddy `handle /relay*` block is optional. With strfry stopped,
that path stops answering; the rest of the site is unchanged.

## NIP-05 domain

`NIP05_DOMAIN` is the domain that serves `/.well-known/nostr.json`, and it
becomes the right-hand side of every member's NIP-05 identifier
(`name@ow.scuffedcrew.no`). For this deploy:

```bash
NIP05_DOMAIN=ow.scuffedcrew.no
```

If it is unset, the server falls back to `REDIRECT_BASE_URL` **only when that
resolves to a valid public domain**. Loopback hosts, bare IPs, ports, and
`.local`/`.internal`-style names are all rejected, and in that case kind-0
profile events publish **without** a `nip05` field.

That omission is deliberate, not a degradation: kind-0 events are immutable
once they reach a relay, so publishing an identifier against a domain you do
not control breaks NIP-05 verification for that member permanently — and lets
whoever registers the domain impersonate them. No identity is better than a
wrong one.

Verify a live deploy:

```bash
curl -s https://ow.scuffedcrew.no/.well-known/nostr.json?name=yourname | jq
```

The `names` entry must map your NIP-05 local name to your pubkey. If members
already have kind-0 events carrying an older, wrong domain, changing this
setting does **not** rewrite them — republishing is a separate, deliberate
operation, documented next.

### Repairing already-published identities

Kind-0 events are immutable once a relay has them. Members provisioned while
the server published the wrong domain still advertise it, and only a *new*
event fixes that. `POST /api/admin/nostr/republish-profiles` does it, behind
three independent locks so nothing reaches a relay by accident:

1. **Admin session** — the route uses the admin extractor.
2. **`NIP05_REPUBLISH_ENABLED=1`** — off by default; only the exact string `1`
   arms it. Restart the server with it set, and unset it afterwards.
3. **`{"confirm": true}`** — any other body, or none, is a **dry run**.

It also refuses outright if `NIP05_DOMAIN` is invalid or `NOSTR_RELAY_URL` is
unset, rather than reporting a success that published nothing.

Always dry-run first — it lists every member that would be touched and the
exact identifier each would receive:

```bash
# 1. dry run (no body = dry run)
curl -s -X POST https://ow.scuffedcrew.no/api/admin/nostr/republish-profiles \
  -H "Cookie: <admin session>" | jq

# 2. read candidate_count and the candidates[].nip05 values, then commit
curl -s -X POST https://ow.scuffedcrew.no/api/admin/nostr/republish-profiles \
  -H "Cookie: <admin session>" -H 'Content-Type: application/json' \
  -d '{"confirm": true}' | jq
```

Only **server-managed** keys are republished. Members holding their own key
(`external`) sign their own events; the server will not overwrite an identity
it does not control. Those members must republish from their own client.

The confirmed run is audit-logged. Unset `NIP05_REPUBLISH_ENABLED` and restart
once you are done — leaving it armed serves no purpose.

## Tracker bug reports

Member bug-report zips stay on disk. They are not rows in SurrealDB and they
are not under the web root. Compose sets `REPORTS_DIR` to `/app/data/reports`
and mounts the named volume `reports-data` there.

The image entrypoint (`scripts/site-entrypoint.sh`) creates that directory,
gives it to the container user `scuffed`, and sets mode 0700 before the server
process starts. A fresh named volume is often owned by root, which is why the
entrypoint does this instead of starting the binary directly.

If the directory is missing or not writable when the server starts, stat
reports turn themselves off. The log line is `stat reports disabled: ...` and
the rest of the site keeps serving. Every report route then returns 503
with body `{"error":"reports_disabled"}` until the directory can be written
and the server is restarted.

`scripts/backup.sh` includes the `reports-data` volume in the restic snapshot,
the same way it includes uploads.

## Recognizer asset packs

Hero icons and other recognizer packs are game art. Do not commit them, do not
attach them to a release, and do not put them in fixtures. Compose mounts the
named volume `packs-data` (on the VPS: `scuffed-crew_packs-data`) read-only at
`/app/data/packs` and sets `PACKS_DIR` to that path. The directory is outside
SurrealDB and outside the web root.

Fill the volume with `scripts/publish-packs.sh`, from the machine that built
the packs. Do not copy files into the volume by hand:

```bash
scripts/publish-packs.sh --check /path/to/packbuild/packs
scripts/publish-packs.sh --dry-run --host root@your-vps /path/to/packbuild/packs
scripts/publish-packs.sh --host root@your-vps /path/to/packbuild/packs
```

It keeps the packs that are already published, uploads the new ones, and
switches `manifest.json` last, so a failed run leaves the old list in place.
It never takes a pack off the list. New content for a pack that is already
published goes out under the same name with a new version, with `--update`.
The site does not need a restart. Each run copies the old manifest (and, for
an update, the old tar) to `/root/scuffed-crew-backups/` on the VPS and
prints the one-line rollback command. After a run that did not end cleanly
it reads the volume again and says which list is live.

The VPS needs bash 4.4 or newer, GNU coreutils 8.24 or newer and `flock`
(util-linux); the script checks and says so, also on a dry run. Naming and
versions, which tracker release asks for which pack, updating, verifying
through the site, and rollback: [tracker-packs.md](tracker-packs.md).

If `PACKS_DIR` is unset, the directory cannot be read, or it has no valid
`manifest.json`, the site still starts. `GET /api/tracker/packs` and
`GET /api/tracker/packs/{name}` return 503 `{"error":"packs_disabled"}`.
`packs-data` is not in `scripts/backup.sh`. The copy of record is the build
folder on the operator's machine.

## 0.6 first boot: roster dedupe (DB7)

The first start of a 0.6 build collapses duplicate `plays_on` roster edges
(one member on one team more than once), then installs the UNIQUE index
`plays_on_member_team_idx` on (in, out). The deletes are permanent and every
deleted edge is logged at info. Later boots see the index and skip the pass.

1. **Back up first.** `scripts/update.sh` has no backup step. Run
   `scripts/backup.sh` (or snapshot the volume) before rollout.
2. **Run the read-only duplicate count first** and note it:
   ```surql
   SELECT * FROM (SELECT in, out, count() AS n FROM plays_on GROUP BY in, out) WHERE n > 1;
   ```
   The dedupe runs inside startup migrations, before `/api/health` answers.
   It handles about 74 duplicate groups per second (1783 groups took about
   23 s in QA), so a large count means a longer wait before health is green.
   Raise the health or start timeout if needed rather than restarting.
3. **Stop the old site-server first.** During a rolling deploy an old
   container that still adds roster entries can create a duplicate between
   the dedupe and the index, which blocks the new container from starting
   until the old one stops. Bring the old one down, then start the new one.
4. **The migrating container must run the new build.** Containers with
   `SURREALDB_BOOTSTRAP=0` never run migrations, so they never run DB7. The
   container that bootstraps (or the `SURREALDB_MIGRATE_ONLY=1` job) has to
   be on the new image.
5. **After boot:** expect the info log `plays_on is UNIQUE on (in, out)`.
   Its `duplicate_pairs` equals the number of rows the step 2 query returned
   (one row per duplicate group). Its `deleted` is the number of extra edges,
   the sum of `n - 1` over those rows. Re-run the step 2 query: 0 rows.

Two servers that boot against one database at the same moment can hit
`Transaction conflict: Resource busy ... can be retried` in a startup
migration. Each fatal migration step retries that error up to 4 times with
a short backoff (about 2 s in total) before it gives up. A failed step names
itself in the startup panic, for example
`SurrealDB startup migration failed: DB7 roster dedupe/unique index failed after 1 attempt(s): ...`.
A panic that starts with `Failed to connect to SurrealDB` is a connection or
login problem, not a migration.

**Rollback to a pre-0.6 build:** that build relates a new edge on every add,
which the index rejects (500 on re-add or double click). Run
`REMOVE INDEX plays_on_member_team_idx ON plays_on` as root, or accept that.
Deleted duplicates come back only from the backup.

## Upgrading the SurrealDB server

`compose.yml` pins the server image to an exact tag equal to the `surrealdb`
crate in `Cargo.lock` (CI runs `scripts/check-surrealdb-version.sh`). Changing
that tag in `compose.yml` does not upgrade the database by itself, and must
not do so by accident:

- Any `compose up` of `site-server` **without `--no-deps`**, a plain `up -d`,
  or `compose run` without `--no-deps` follows `depends_on`. When the running
  database container's image differs from `compose.yml`, or no database
  container is Up, compose creates the database container on the new tag
  over the live volume. That is an upgrade with no backup.
- `update.sh`, `install.sh` and `reset-local-admin.sh` refuse to do that
  (`scripts/lib/surreal-guard.sh`, tested by `scripts/test-surreal-guard.sh`).
  They start `site-server` with `--no-deps` whenever a project database
  container exists, start a stopped one as it is, and stop with an error
  when that container runs another image than `compose.yml` names, or when
  no container exists but the data volume does.
- `SCUFFED_ALLOW_DB_IMAGE_MISMATCH=1` lets them go on and leave the
  database on its current image. `SCUFFED_ALLOW_DB_CREATE=1` lets compose
  create the database container over an existing volume. Use the second
  only after a backup, or when the `compose.yml` image wrote that data.

**There is no tag-only rollback.** Measured on a scratch datastore: 3.0.5
data opens in place on 3.2.3, but once 3.2.3 has opened it, 3.0.5 no longer
starts on it (its log repeats ``A deserialization error occured: Invalid
revision `2` for type `Node` `` and `is-ready` never succeeds). Rolling back
means restoring the cold copy of the volume taken in step B5, and loses
every write made after it.

SurrealDB's notes say 3.0.x → 3.1.0 and 3.1.x → 3.2.x upgrade in place with
unchanged catalog and on-disk layouts. 3.2.5 and later write data an earlier
3.2 binary cannot read, so do not jump past the crate version.

**Crate bumps.** Dependabot security updates are on for this repository. A
Dependabot PR that moves the `surrealdb` crate fails
`scripts/check-surrealdb-version.sh` on purpose. Merge it only together with
the matching `compose.yml` tag, and treat that merge as the start of a Phase B
window: until it is done, `update.sh` refuses (or needs
`SCUFFED_ALLOW_DB_IMAGE_MISMATCH=1`). Read the release notes of the new
version for one-way format changes (3.2.5 and later) before you merge.

The upgrade from rc.5 has two phases. Do Phase A first and let it run. Do
Phase B as its own maintenance window.

### Phase A: app update, database unchanged

rc.6 also starts passing 17 variables from `data/secrets.env` to the server
(see [Which variables reach the server](#which-variables-reach-the-server)).
Until now they were ignored. Any of them already in your `secrets.env` takes
effect in this phase.

1. Pull and list which of them you have (names only, values stay hidden):

   ```bash
   cd /path/to/scuffed-crew
   git pull --ff-only
   grep -nE '^(ENCRYPTION_KEY_PREVIOUS|CRYPTO_STRICT_AAD|ALLOW_PLAINTEXT_PROVIDER_IDS|SURREALDB_BOOTSTRAP|FORCE_APP_PASSWORD_SYNC|CSP_|LEADERBOARD_CACHE_|DISCORD_WEBHOOK|MATRIX_)' data/secrets.env | cut -d= -f1
   ```

2. For each name printed, decide whether you want it now. Delete or comment
   the line if not:
   - `SURREALDB_BOOTSTRAP=0`: migrations stop running on start. Remove it
     unless you run a separate migrate job.
   - `CSP_ENFORCE=0`: the CSP becomes report-only. Remove it unless intended.
   - `DISCORD_WEBHOOK_*`, `MATRIX_*`: notifications start posting. Check the
     webhook and room are still the right ones.
   - `ENCRYPTION_KEY_PREVIOUS`: must be `ver:base64[,ver:base64]` with no
     version equal to `ENCRYPTION_KEY_VERSION`, or the server refuses to boot.
   - `FORCE_APP_PASSWORD_SYNC=1`: re-defines the app DB password on every boot.
   - The others (`CRYPTO_STRICT_AAD`, `ALLOW_PLAINTEXT_PROVIDER_IDS`) change
     nothing while `PRODUCTION=1`. `LEADERBOARD_CACHE_*` only tune the cache.

3. Update the app and leave the database where it is:

   ```bash
   SCUFFED_ALLOW_DB_IMAGE_MISMATCH=1 ./scripts/update.sh
   ```

   Without the variable, `update.sh` stops before touching any container,
   because the database runs `surrealdb:v3.0` and `compose.yml` names the new
   tag. That refusal is expected until Phase B is done; it is the reminder.

4. Check: `curl -fsS http://127.0.0.1:$HOST_PORT/api/health`, no errors in
   `podman logs <site-server>`, admin login, `/stats`, and the CSP header
   (`curl -sI https://<site>/ | grep -i content-security-policy` shows
   `Content-Security-Policy:` unless you kept `CSP_ENFORCE=0`).

### Phase B: database upgrade (own maintenance window)

The site is down for a few minutes. The names below are examples. Find yours
first:

```bash
cd /path/to/scuffed-crew
podman ps -a --format '{{.Names}}\t{{.Image}}\t{{.Status}}' | grep -E 'surrealdb|site-server|strfry'
podman volume ls | grep surrealdb-data
DB=scuffed-crew-surrealdb-1          # the one Up container (see Troubleshooting if there are two)
APP=scuffed-crew-site-server-1
VOL=scuffed-crew_surrealdb-data
RELAY=                               # strfry container name, if the relay is deployed
set -a; . data/secrets.env; set +a
```

#### B1. Record what runs now

```bash
git pull --ff-only
podman exec "$DB" /surreal version    # e.g. 3.0.5 for linux on x86_64
podman inspect "$DB" --format '{{.ImageName}}' | tee /root/scuffed-surrealdb-old-image.txt
podman inspect "$DB" --format '{{.Image}}'          # image ID
```

That image is the rollback target. Do not run `podman image prune` until the
upgrade has held for a week.

#### B2. Pause the backup timer and check its path

```bash
systemctl stop scuffed-backup.timer
systemctl cat scuffed-backup.service | grep ExecStart
```

The shipped unit runs `/opt/scuffed-crew/scripts/backup.sh`. If this checkout
lives elsewhere (for example `/root/github/scuffed-crew`), the timer has been
running another copy, or nothing. Run the backup below from this checkout
either way, and fix the unit's `ExecStart` after the upgrade.

#### B3. Back up

```bash
set -a; . /etc/scuffed-crew/backup.env; set +a
./scripts/backup.sh
restic snapshots --tag scuffed-crew | tail -3   # a snapshot from just now
```

Also keep a logical export on the host, outside restic:

```bash
podman exec -e SURREAL_PASS="$SURREALDB_PASSWORD" "$DB" /surreal export \
  --endpoint http://127.0.0.1:8000 --username "${SURREALDB_USER:-root}" \
  --namespace "${SURREALDB_NS:-scuffed_crew}" --database "${SURREALDB_DB:-main}" \
  /tmp/pre-upgrade.surql
podman cp "$DB":/tmp/pre-upgrade.surql /root/scuffed-pre-upgrade-$(date +%F).surql
ls -lh /root/scuffed-pre-upgrade-*.surql
```

#### B4. Pull the new image before the downtime

```bash
NEW_IMAGE="$(awk '/^  surrealdb:/{s=1} s && /^    image:/{print $2; exit}' compose.yml)"
echo "$NEW_IMAGE"
podman pull "$NEW_IMAGE"
```

#### B5. Stop, copy the volume cold, remove the containers

```bash
podman stop "$APP"; podman rm "$APP"  # first: podman-compose may have made it depend on $DB
podman stop "$DB"
podman volume export "$VOL" -o /root/scuffed-surrealdb-pre-upgrade-$(date +%F).tar
tar tf /root/scuffed-surrealdb-pre-upgrade-*.tar | head    # scuffed.db/...
podman rm "$DB"                       # the container only; never `podman volume rm`
podman ps -a | grep surrealdb         # must print nothing
```

The copy is taken with the database stopped, so it is consistent. The restic
snapshot's volume copy is taken while it runs; the logical export is the
consistent part of that snapshot.

#### B6. Start the new server

```bash
podman compose --env-file data/secrets.env up -d --no-deps surrealdb
podman ps --format '{{.Names}}\t{{.Image}}\t{{.Status}}' | grep surrealdb   # exactly one, the new tag
DB=<the name printed above>
podman exec "$DB" /surreal version
podman exec "$DB" /surreal is-ready --endpoint http://127.0.0.1:8000 && echo ready
podman logs "$DB" 2>&1 | grep -iE 'error|panic' | head
```

If there are two surreal containers, stop the new one and see Troubleshooting
(duplicate `_` vs `-`) before going on.

#### B7. Start the app and the relay

```bash
./scripts/update.sh                   # images match now; no flag needed
[ -n "$RELAY" ] && podman restart "$RELAY"
```

The relay policy and read gate inside the strfry container connect to
SurrealDB once at startup, so they need the restart after the database was
replaced.

#### B8. Check, then resume backups

```bash
curl -fsS "http://127.0.0.1:${HOST_PORT:-3000}/api/health"; echo
podman logs "$APP" 2>&1 | grep -iE 'error|panic|migrat' | head
```

In a browser: log in as an admin (checks the app DB user and sessions), open
`/stats` and one member's stats page, the public leaderboard, and the admin
audit log. Send one chat message if the relay is on. Wait for the surrealdb
container to show `healthy` in `podman ps`. Then:

```bash
systemctl start scuffed-backup.timer
./scripts/backup.sh                   # first snapshot on the new version
```

Keep the `.tar`, the `.surql` export, `/root/scuffed-surrealdb-old-image.txt`
and the old image for at least a week.

### Optional later: rebuild compound unique indexes

3.1.0 changed how unique indexes store rows whose indexed columns are `NONE`
or `NULL`, and says existing compound unique indexes need `REBUILD INDEX`.
Measured on scratch data: a 3.0.5-written index `COLUMNS m, e, d UNIQUE`
does not find a row with `d = NONE` through `WHERE m = .. AND e = ..`, on
3.0.5 and on 3.2.3 alike (today's behaviour). After `REBUILD INDEX` on 3.2.3
it does. Duplicate `NONE` tuples are allowed before and after on both
versions. A rebuild therefore changes query results (rows that were missed
start showing up), so it is a separate decision, not part of this upgrade.
Do not run `REBUILD INDEX` across a version change.

### Roll back

Do this if B6 or B8 fails and the cause is not obvious in a few minutes.
**Everything written to the database after B5 is lost**: new games, chat,
sign-ups, admin changes. Decide quickly, or fix forward.

```bash
podman stop "$APP"; podman rm "$APP"
podman stop "$DB"; podman rm "$DB"
# Keep the failed state for later diagnosis:
podman volume export "$VOL" -o /root/scuffed-surrealdb-failed-$(date +%F).tar
# Replace the datastore with the cold copy from B5:
rm -rf "$(podman volume inspect "$VOL" --format '{{.Mountpoint}}')/scuffed.db"
podman volume import "$VOL" /root/scuffed-surrealdb-pre-upgrade-<date>.tar
# Point compose at the image recorded in B1 (a local edit; undo it before the next upgrade):
OLD_IMAGE="$(cat /root/scuffed-surrealdb-old-image.txt)"
sed -i -E "s#^(    image: )docker\.io/surrealdb/surrealdb:[^[:space:]]+#\1${OLD_IMAGE}#" compose.yml
grep -n 'image:' compose.yml | head -3   # the surrealdb line shows the old image
podman compose --env-file data/secrets.env up -d --no-deps surrealdb
podman exec <new surreal name> /surreal version   # the old version
./scripts/update.sh
[ -n "$RELAY" ] && podman restart "$RELAY"
systemctl start scuffed-backup.timer
```

`update.sh` runs `git pull --ff-only`, which keeps the local `compose.yml`
edit unless `main` changed that file. If the `.tar` is unusable, restore the
restic snapshot (`./scripts/restore.sh`, see [Restore](#restore)) and load the
logical export into an empty volume on the old version with
`podman exec ... /surreal import --endpoint http://127.0.0.1:8000 --username root --namespace scuffed_crew --database main /tmp/pre-upgrade.surql`
(after `podman cp` of the `.surql` into the container). **Do not** start the
old version on a volume the new one has opened.

## Backups

`scripts/backup.sh` stores the SurrealDB export, data volumes, and
`data/secrets.env` (including `ENCRYPTION_KEY`) in the **same** restic
repository. One repository password opens both the database and the key that
decrypts Nostr keys, OAuth ids, and DMs. That is deliberate: a second store for
`ENCRYPTION_KEY` is easy to forget, and forgetting it is how a host loss becomes
unreadable data.

Two different secrets, stored in two different places:

| Secret | Where it lives | Why |
|---|---|---|
| Restic **repository** | **Off this host** (`sftp:`, `s3:`, `rest:`, or any other restic backend) | A disk on the same machine dies with the host |
| `RESTIC_PASSWORD` | **On this host**, for the daily timer, in a root-owned mode-600 file **outside** the repo and outside every path restic snapshots. A copy also goes in a password manager | An unattended timer cannot prompt |
| `ENCRYPTION_KEY` | `data/secrets.env` on the host (mode 600), **and inside the snapshot**, **and** a copy in the same password manager | The snapshot cannot decrypt Nostr keys or DMs without it. The password-manager copy covers a backup that predates this, or a snapshot you cannot open yet |

`backup.sh` refuses a local `RESTIC_REPOSITORY` (`/var/backups/...`, `local:...`, or a relative path) unless `BACKUP_ALLOW_LOCAL_REPO=1`. Do not set that on the production timer. It also refuses to run if `ENCRYPTION_KEY` is missing or blank, or if `RESTIC_PASSWORD` is assigned inside `data/secrets.env` (that file is copied into the snapshot). Each snapshot carries `encryption-key.fingerprint` (a sha256 of the key material, not the key) so restore fails when the host key does not match.

The timer reads `/etc/scuffed-crew/backup.env` (not `data/secrets.env`):

```
RESTIC_REPOSITORY=sftp:USER@BACKUP-HOST:/srv/restic/scuffed-crew
RESTIC_PASSWORD_FILE=/etc/scuffed-crew/restic-password
```

`RESTIC_PASSWORD_FILE` is one line, the password, nothing else. Interactive restores can export `RESTIC_PASSWORD` instead of the file. Set only one of them.

### Enable the daily backup

The repo ships `deploy/scuffed-backup.service` and `deploy/scuffed-backup.timer` (daily, 03:00, with a short random delay). They do nothing until they are installed and restic is pointed at an off-host repository. Pick any restic backend; the URL below is a placeholder.

1. Install restic on the host (`restic version`).
2. Create a repository on another machine (sftp), an S3-compatible bucket, or a [rest-server](https://github.com/restic/rest-server). You need the URL restic will use.
3. Put a copy of the repository password **and** of `ENCRYPTION_KEY` (from `data/secrets.env`) in a password manager. A lost host is recoverable from the off-host backup plus those two values.
4. On the host, as root:

```bash
install -d -m 700 /etc/scuffed-crew
install -m 600 /dev/null /etc/scuffed-crew/restic-password
# write the password into that file (one line), then:
chmod 600 /etc/scuffed-crew/restic-password
chown root:root /etc/scuffed-crew/restic-password

cat > /etc/scuffed-crew/backup.env <<'EOF'
RESTIC_REPOSITORY=sftp:USER@BACKUP-HOST:/srv/restic/scuffed-crew
RESTIC_PASSWORD_FILE=/etc/scuffed-crew/restic-password
EOF
chmod 600 /etc/scuffed-crew/backup.env
chown root:root /etc/scuffed-crew/backup.env
```

Replace the `RESTIC_REPOSITORY` line with the real URL (`sftp:user@host:/path`, `s3:https://s3.example.com/bucket/path`, `rest:https://backup.example:8000/scuffed-crew`, and so on).

5. Initialize the repository once:

```bash
set -a
. /etc/scuffed-crew/backup.env
set +a
./scripts/backup-init.sh
```

6. Install and start the timer (paths assume the repo is at `/opt/scuffed-crew`):

```bash
cp deploy/scuffed-backup.service deploy/scuffed-backup.timer /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now scuffed-backup.timer
systemctl start scuffed-backup.service
systemctl status scuffed-backup.service
```

7. `systemctl start scuffed-backup.service` must succeed, and `restic snapshots --tag scuffed-crew` on the off-host repo must show a snapshot. Snapshots taken before `secrets.env` was included do **not** contain `ENCRYPTION_KEY`. Keep the password-manager copy until a new snapshot exists.

`BACKUP_ALLOW_LOCAL_REPO=1` is only for a deliberate same-host drill. Leave it unset on the timer.

## Restore

Stop the app first. Restore secrets **before** starting it. A fresh
`install.sh` on a rebuilt host generates a new `ENCRYPTION_KEY`, and that key
cannot decrypt the restored database.

On a new host, export `RESTIC_PASSWORD` from the password manager (or recreate
the mode-600 `RESTIC_PASSWORD_FILE`) and set `RESTIC_REPOSITORY` to the same
off-host URL:

```bash
export RESTIC_REPOSITORY='sftp:USER@BACKUP-HOST:/srv/restic/scuffed-crew'
export RESTIC_PASSWORD='from-the-password-manager'
./scripts/restore.sh latest
```

`restore.sh` copies the snapshot's `secrets.env` into `data/secrets.env` when
the host key is missing or different, then runs `scripts/check-restore-key.sh`.
That check exits non-zero if `ENCRYPTION_KEY` is missing or its fingerprint
does not match the snapshot. Do not start the app until it prints
`encryption key matches`.

Non-interactive install of the backup's secrets file:

```bash
SCUFFED_RESTORE_INSTALL_SECRETS=1 ./scripts/restore.sh latest
```

Check an already restored tree yourself:

```bash
./scripts/check-restore-key.sh \
  --secrets data/secrets.env \
  --fingerprint /path/to/restored/encryption-key.fingerprint
```

## Forgot admin password

```bash
BOOTSTRAP_ADMIN_USERNAME=admin \
BOOTSTRAP_ADMIN_PASSWORD='your-new-long-password' \
./scripts/reset-local-admin.sh
```

Then remove `BOOTSTRAP_ADMIN_RESET` from the environment and recreate `site-server` without it.

## Verify

```bash
curl -sS "http://127.0.0.1:${HOST_PORT}/api/health"
curl -sS "http://127.0.0.1:${HOST_PORT}/api/auth/setup-status"
# {"needs_setup":true,"local_login":false} before first admin
```

After setup, `needs_setup` is false; use `/login`.

## Optional later: Quadlet

When the Compose stack is stable, you may migrate services to **systemd Quadlet** (`.container` units) for boot-native management. That is optional and not required for a working install.
