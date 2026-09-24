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
| `SURREALDB_MIGRATE_ONLY=1` | Server runs root bootstrap (`Database::bootstrap_from_env`) then **exits** — for init/migrate jobs |
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

### Split migrator vs app (optional multi-container)

Single-container install keeps default bootstrap on every start (root session is short-lived, then EDITOR). For a stricter split:

1. **Init job:** `SURREALDB_MIGRATE_ONLY=1` (needs root + app passwords) — migrate, ensure app user, exit.
2. **App:** `SURREALDB_BOOTSTRAP=0` + app credentials only — never signs in as root.

Kubernetes is out of scope. **Quadlet** (systemd-native containers) is an optional later migration if you want boot integration without Compose — no Quadlet units ship yet.

## Prebuilt images (recommended)

GitHub Actions builds the `site-server` image on every push to `main` and publishes to GHCR:

| Tag | Image |
|-----|--------|
| `main` / `latest` | `ghcr.io/frozentear/scuffed-crew:main` |
| commit | `ghcr.io/frozentear/scuffed-crew:sha-<short>` |

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

On Contabo-style day-2 hosts, if a project Surreal is already Up, `update.sh` recreates site-server with `up -d --no-deps` so `/bin/podman-compose` does not spawn an underscore-named DB twin on the same volume (see Troubleshooting: duplicate Surreal `_` vs `-`).

Override image pin in `data/secrets.env` if needed:

```bash
SITE_SERVER_IMAGE=ghcr.io/frozentear/scuffed-crew:main
# or a specific sha:  .../scuffed-crew:sha-abc1234
```

### Build from source (fallback only)

```bash
BUILD_FROM_SOURCE=1 ./scripts/install.sh
# or: podman compose --env-file data/secrets.env up --build -d
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

### SurrealDB: `is unhealthy` but logs show “Started web server”

DB is fine; an old healthcheck probe was wrong. Current `compose.yml` does **not** healthcheck Surreal and uses `depends_on: service_started`. Pull latest and `up -d` again.

### SurrealDB: duplicate container (`_` vs `-`) and site-server stuck Created / public 502

On some hosts (notably Contabo with `/bin/podman-compose`), Compose-v2 names the long-lived DB `scuffed-crew-surrealdb-1` (volume `scuffed-crew_surrealdb-data`). A later `compose up -d site-server` that honors `depends_on` can create an underscore twin `scuffed-crew_surrealdb_1` on the **same** RocksDB volume. The twin exits (`Exit 1`); site-server can sit in `Created` and the public site returns 502 until the app is started with `--no-deps` and the duplicate is stopped.

This is **name-skew**, not a `SURREALDB_URL` typo. The compose service remains `surrealdb` (`ws://surrealdb:8000`). Do **not** rename the service or volume key, and do **not** pin `container_name` without a planned cutover.

**Never `podman volume rm` the Surreal data volume for this symptom.** The twins share `scuffed-crew_surrealdb-data`; removing it destroys production data.

`scripts/update.sh` detects a running project Surreal (`${PROJECT_NAME}-surrealdb*`, `${PROJECT_NAME}_surrealdb*`) and starts site-server with `up -d --no-deps` so the existing DB is left alone. Prefer the hyphen name (`…-surrealdb-1`) when both exist. After the app is healthy, the script stops extras and tries `rm -f`; if compose blocks the rm, the duplicate is left `Exited`.

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
2. Runs `podman compose --env-file data/secrets.env up --build -d`
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
	@hashed path *.wasm *.js *.css
	header @hashed Cache-Control "public, max-age=31536000, immutable"
	header X-Content-Type-Options "nosniff"
	header X-Frame-Options "DENY"
	header Referrer-Policy "strict-origin-when-cross-origin"
	reverse_proxy 127.0.0.1:HOST_PORT   # from data/secrets.env on the VPS
}
```

```bash
# on VPS
grep '^HOST_PORT=' /root/github/scuffed-crew/data/secrets.env
# put that number in reverse_proxy, then:
caddy validate --config /etc/caddy/Caddyfile
systemctl reload caddy
# or: caddy reload --config /etc/caddy/Caddyfile
```

Template also lives in repo: `deploy/Caddyfile`.

The app sets `Content-Security-Policy-Report-Only` itself (same-origin scripts,
Google Fonts, Discord/Google avatar hosts, and `NOSTR_RELAY_URL` for chat
sockets). Leave CSP off the Caddy block so the two policies do not intersect.
Set `CSP_ENFORCE=1` in `data/secrets.env` and recreate the app container to
send enforcing `Content-Security-Policy` instead. `CSP_EXTRA_CONNECT_SRC` and
`CSP_IMG_SRC` add relay or image origins without a code change.

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

**3. App public URL** (required for cookies / redirects):

```bash
cd /root/github/scuffed-crew   # your clone path
# edit data/secrets.env:
#   REDIRECT_BASE_URL=https://ow.scuffedcrew.no
#   ALLOWED_ORIGINS=https://ow.scuffedcrew.no
#   # Blank ALLOWED_ORIGINS is treated as unset and falls back to REDIRECT_BASE_URL
#   # (F-API-004). Do not leave it empty thinking it means "allow all".
#   NIP05_DOMAIN=ow.scuffedcrew.no      # see "NIP-05 domain" below

podman compose --env-file data/secrets.env up -d --force-recreate site-server
```

**4. From home PC:** open `https://ow.scuffedcrew.no`  
First visit → create admin if `setup-status` still needs setup.

**HTTPS note:** release builds set **Secure** cookies. Use the `https://` subdomain above; plain `http://IP:port` often won’t keep login.

## Power-user path

Copy `.env.example` → `data/secrets.env` (or `.env`), set values yourself, then:

```bash
podman compose --env-file data/secrets.env up --build -d
```

## Optional Nostr relay

```bash
# also set NOSTR_RELAY_URL=ws://strfry:7777 in secrets
podman compose --env-file data/secrets.env --profile relay up --build -d
```

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

## Backups

`scripts/backup.sh` stores the SurrealDB export, data volumes, and
`data/secrets.env` (including `ENCRYPTION_KEY`) in the **same** restic
repository. The repository is encrypted with `RESTIC_PASSWORD`. That password
is not inside the snapshot — keep it off the host (a password manager). One
password then opens both the database and the key that decrypts Nostr keys,
OAuth ids, and DMs. That is deliberate: a second store is easy to forget, and
forgetting it is how a host loss becomes unreadable data.

The backup refuses to run if `ENCRYPTION_KEY` is missing or blank. Each snapshot
also carries `encryption-key.fingerprint` (a sha256 of the key material, not the
key) so restore can fail when the host key does not match.

```bash
# once
export RESTIC_REPOSITORY=... RESTIC_PASSWORD=...
./scripts/backup-init.sh

# daily (sources data/secrets.env; refuses to run if ENCRYPTION_KEY is blank)
./scripts/backup.sh
```

Snapshots taken before this change do **not** contain `secrets.env`. After
deploying these scripts, run `./scripts/backup.sh` once so a snapshot actually
holds the key. Until that snapshot exists, keep an offline copy of
`data/secrets.env`.

Systemd units under `deploy/` can load:

```
EnvironmentFile=-/opt/scuffed-crew/data/secrets.env
```

## Restore

Stop the app first. Restore secrets **before** starting it. A fresh
`install.sh` on a rebuilt host generates a new `ENCRYPTION_KEY`, and that key
cannot decrypt the restored database.

```bash
export RESTIC_REPOSITORY=... RESTIC_PASSWORD=...
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
