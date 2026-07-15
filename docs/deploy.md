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
| `SURREALDB_PASSWORD` | **Required** strong password (install generates one) |
| `ENCRYPTION_KEY` | **Required** when `PRODUCTION=1` — OAuth provider IDs, Nostr server keys, DM content at rest |
| `SURREALDB_AUTH_MODE` | `root` (default, single-tenant) or `scoped` (least-privilege DB user) |
| `PRODUCTION=1` | Refuses default `root`/`root` credentials; requires `ENCRYPTION_KEY` |

Never ship with `SURREALDB_USER=root` / `SURREALDB_PASSWORD=root` outside local dev.

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
# = git pull --ff-only + podman pull + recreate site-server
```

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

**3. App public URL** (required for cookies / redirects):

```bash
cd /root/github/scuffed-crew   # your clone path
# edit data/secrets.env:
#   REDIRECT_BASE_URL=https://ow.scuffedcrew.no
#   ALLOWED_ORIGINS=https://ow.scuffedcrew.no

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

## Backups

```bash
# once
export RESTIC_REPOSITORY=... RESTIC_PASSWORD=...
./scripts/backup-init.sh

# daily (sources data/secrets.env when present)
./scripts/backup.sh
```

Systemd units under `deploy/` can load:

```
EnvironmentFile=-/opt/scuffed-crew/data/secrets.env
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
