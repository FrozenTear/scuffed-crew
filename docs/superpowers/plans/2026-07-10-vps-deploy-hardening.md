# VPS Deploy Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a novice-friendly VPS install (auto-generated DB secrets via install script + first-boot admin in the browser) with Podman Compose, uploads persistence, docs, and backup fixes—without requiring Discord OAuth.

**Architecture:** Install script writes `data/secrets.env` (Surreal password, encryption key, optional URL). Compose loads it and runs Surreal + site-server. When no admin member exists, the SPA shows a setup form; `POST /api/auth/setup` creates a `local` user (Argon2id password hash), admin member, and session. Later logins use `POST /api/auth/local/login`. OAuth remains optional.

**Tech Stack:** Rust (Axum, SurrealDB v3, Dioxus 0.7), Argon2id (`argon2` crate), Podman Compose, bash install script, Caddy (host).

**Spec:** `docs/superpowers/specs/2026-07-10-vps-deploy-hardening-design.md`

---

## File map

| Path | Responsibility |
|------|----------------|
| `crates/auth/src/types.rs` | Add `AuthProvider::Local` |
| `crates/auth/src/password.rs` (new) | Argon2id hash/verify |
| `crates/auth/src/lib.rs` | Export password module (server/crypto feature) |
| `crates/auth/Cargo.toml` | Add `argon2` (+ `password-hash`) under server feature |
| `crates/types/src/auth.rs` | Setup/login API DTOs; optional Local on client enum if needed |
| `crates/db/src/migrations.rs` | `provider` includes `local`; `password_hash` field on `user` |
| `crates/db/src/queries/users.rs` | Create/find local user; password hash; `count_admins` / `has_admin` |
| `crates/db/src/queries/members.rs` | Reuse `create_member` for admin on setup |
| `crates/site-server/src/routes/auth.rs` | setup-status, setup, local login, providers |
| `crates/site-server/src/lib.rs` | Register + rate-limit new routes |
| `crates/site-server/src/routes/mod.rs` | If needed for module exports |
| `crates/server/src/main.rs` | Optional emergency admin password reset on boot |
| `crates/site-server/tests/api_integration.rs` | Integration tests for setup + local login |
| `crates/app/src/routes.rs` | `/setup` route (optional if overlay in root) |
| `crates/app/src/pages/setup.rs` (new) | First-boot create-admin page |
| `crates/app/src/pages/login.rs` (new) | Local login + optional OAuth links |
| `crates/app/src/pages/mod.rs` | Export pages |
| `crates/app/src/main.rs` / layouts | Gate: redirect to `/setup` when `needs_setup` |
| `crates/app/src/layouts/admin.rs` | Local login link when Discord off |
| `crates/app/src/pages/apply.rs` | Same |
| `scripts/install.sh` (new) | Generate secrets + compose up |
| `data/secrets.env` | Generated; never commit |
| `.gitignore` | `data/secrets.env` |
| `.env.example` | Full reference for power users |
| `compose.yml` | env_file, uploads volume, relay profile |
| `relay/Containerfile` | Fix workspace copy for build |
| `scripts/backup.sh` | Compose volume name prefix + uploads |
| `scripts/reset-local-admin.sh` (new) | Emergency admin password reset helper |
| `docs/deploy.md` (new) | Deploy runbook |
| `CLAUDE.md` | Point to deploy path |

---

### Task 1: AuthProvider::Local + Argon2 password helpers

**Files:**
- Modify: `crates/auth/Cargo.toml`
- Modify: `crates/auth/src/types.rs`
- Create: `crates/auth/src/password.rs`
- Modify: `crates/auth/src/lib.rs`

- [ ] **Step 1: Add argon2 dependency under server feature**

In `crates/auth/Cargo.toml`, add to `[features] server`:
```toml
server = [
    # ... existing ...
    "dep:argon2",
    "dep:password-hash",
]
```

And dependencies:
```toml
argon2 = { version = "0.5", optional = true }
password-hash = { version = "0.5", optional = true }
```

- [ ] **Step 2: Add Local variant**

In `crates/auth/src/types.rs`, extend `AuthProvider`:
```rust
pub enum AuthProvider {
    Discord,
    Google,
    Matrix,
    Local,
}
```

Update `Display` to print `"local"` for `Local`.

- [ ] **Step 3: Implement password module**

Create `crates/auth/src/password.rs` (only compiled with `server` feature):

```rust
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PasswordError {
    #[error("password hashing failed")]
    Hash,
    #[error("password verification failed")]
    Verify,
    #[error("invalid password hash format")]
    InvalidHash,
}

/// Hash a password with Argon2id (PHC string includes salt).
pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| PasswordError::Hash)
}

pub fn verify_password(password: &str, password_hash: &str) -> Result<bool, PasswordError> {
    let parsed = PasswordHash::new(password_hash).map_err(|_| PasswordError::InvalidHash)?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// Minimum length for setup / local login passwords.
pub const MIN_PASSWORD_LEN: usize = 12;
```

Export from `lib.rs` under `#[cfg(feature = "server")] pub mod password;`.

- [ ] **Step 4: Unit test hash round-trip**

In `password.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_and_verify() {
        let h = hash_password("correct-horse-battery").unwrap();
        assert!(verify_password("correct-horse-battery", &h).unwrap());
        assert!(!verify_password("wrong-password-xx", &h).unwrap());
    }
}
```

Run: `cargo test -p scuffed-auth --features server password::tests -- --nocapture`  
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/auth/Cargo.toml crates/auth/src/types.rs crates/auth/src/password.rs crates/auth/src/lib.rs Cargo.lock
git commit -m "feat(auth): Local provider and Argon2id password hashing"
```

---

### Task 2: Schema + DB methods for local users and admin detection

**Files:**
- Modify: `crates/db/src/migrations.rs` (user provider assert + password_hash field)
- Modify: `crates/db/src/queries/users.rs`
- Modify: `crates/types/src/auth.rs` (API DTOs if preferred here)

- [ ] **Step 1: Migration changes**

In `migrations.rs`, change provider assert to:
```sql
DEFINE FIELD provider ON user TYPE string
    ASSERT $value IN ['discord', 'google', 'matrix', 'local'];
DEFINE FIELD password_hash ON user TYPE option<string>;
```

Add index for local usernames (optional but recommended):
```sql
DEFINE INDEX user_local_username_idx ON user COLUMNS provider, username UNIQUE;
```
Note: UNIQUE with OAuth users that share username across providers is OK if composite with provider.

- [ ] **Step 2: Extend DbUser and mapping**

In `users.rs`:
- Add `password_hash: Option<String>` to `DbUser` (not exposed on public `User` type in auth crate—keep hash only in DB layer).
- Parse `AuthProvider::Local` in `db_user_to_user`.
- For local users, `provider_id` can be the username (or a fixed synthetic id equal to username).

- [ ] **Step 3: Add DB APIs**

```rust
/// True if any member has org_role admin (active).
pub async fn has_admin_member(&self) -> DbResult<bool>

/// True if any user with provider=local and non-null password_hash exists.
pub async fn has_local_login(&self) -> DbResult<bool>

/// Create local user with password hash. provider_id = username (lowercase normalized).
pub async fn create_local_user(
    &self,
    username: &str,
    password_hash: &str,
) -> DbResult<User>

/// Find local user by username (case-insensitive: store lowercase).
pub async fn get_local_user_by_username(
    &self,
    username: &str,
) -> DbResult<Option<(User, String)>>  // User + password_hash

/// Update password_hash for a local user (emergency reset).
pub async fn set_local_password_hash(
    &self,
    user_id: &str,
    password_hash: &str,
) -> DbResult<()>
```

Username normalization: trim + lowercase for storage and lookup.

- [ ] **Step 4: Compile check**

Run: `cargo check -p scuffed-db -p scuffed-site-server`  
Expected: success (may need match arms for Local elsewhere—fix all exhaustiveness errors).

- [ ] **Step 5: Commit**

```bash
git add crates/db/src/migrations.rs crates/db/src/queries/users.rs
git commit -m "feat(db): local users, password_hash, has_admin detection"
```

---

### Task 3: Setup + local login API routes

**Files:**
- Modify: `crates/site-server/src/routes/auth.rs`
- Modify: `crates/site-server/src/lib.rs`
- Modify: `crates/types/src/api/` or `crates/types/src/auth.rs` for request/response types

- [ ] **Step 1: Add request/response types**

In `crates/types/src/auth.rs` (shared with app):

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetupStatusResponse {
    pub needs_setup: bool,
    pub local_login: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthProvidersResponse {
    pub local: bool,
    pub discord: bool,
    pub google: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LocalLoginRequest {
    pub username: String,
    pub password: String,
}
```

- [ ] **Step 2: Implement handlers in `auth.rs`**

Logic:

**`setup_status`:**  
`needs_setup = !db.has_admin_member().await?`  
`local_login = db.has_local_login().await?`

**`providers`:**  
`local = has_local_login || needs_setup` (so UI can show forms)  
`discord` / `google` from non-empty OAuth client ids

**`setup`:**  
1. If `has_admin_member` → 403 `{ error: "setup already completed" }`  
2. Validate username (non-empty, max 32, alphanumeric + `_` / `-`)  
3. Validate password length >= `MIN_PASSWORD_LEN`  
4. `hash_password`  
5. `create_local_user`  
6. `create_member(user_id, display_name=username, OrgRole::Admin)`  
7. Create session + session cookie (same as OAuth callback)  
8. Return 200 JSON `{ ok: true }` with Set-Cookie (or 204)

**`local_login`:**  
1. Lookup local user + hash  
2. verify_password  
3. Session cookie on success; 401 on failure (same message for unknown user vs bad password)

- [ ] **Step 3: Register routes with rate limit**

In `lib.rs`, merge setup/login into the same `GovernorLayer` as OAuth (or a sibling governor with same limits):

```rust
.route("/api/auth/setup-status", get(routes::auth::setup_status))
.route("/api/auth/providers", get(routes::auth::auth_providers))
.route("/api/auth/setup", post(routes::auth::setup))
.route("/api/auth/local/login", post(routes::auth::local_login))
```

Apply governor to `setup` and `local/login` (and optionally leave status/providers unlimited or lightly limited).

- [ ] **Step 4: Exhaustiveness**

Fix any `match provider` that needs `Local` (auth routes already match string paths).

- [ ] **Step 5: Commit**

```bash
git add crates/types/src/auth.rs crates/site-server/src/routes/auth.rs crates/site-server/src/lib.rs
git commit -m "feat(api): first-boot setup and local password login"
```

---

### Task 4: Integration tests for setup and local login

**Files:**
- Modify: `crates/site-server/tests/api_integration.rs`

- [ ] **Step 1: Write tests (no seed for setup path)**

Use `test_state()` **without** calling `seed_user` for setup tests:

```rust
#[tokio::test]
async fn setup_status_needs_setup_on_empty_db() {
    let state = test_state().await;
    let app = create_router(state);
    let res = app.oneshot(
        Request::builder().uri("/api/auth/setup-status").body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: Value = /* parse */;
    assert_eq!(body["needs_setup"], true);
}

#[tokio::test]
async fn setup_creates_admin_and_blocks_second_setup() { /* POST setup, expect cookie + second 403 */ }

#[tokio::test]
async fn local_login_works_after_setup() { /* setup then login */ }

#[tokio::test]
async fn setup_rejects_short_password() { /* password len < 12 → 400 */ }
```

Helper to parse JSON body (reuse existing patterns in the file).

- [ ] **Step 2: Run tests**

Run: `cargo test -p scuffed-site-server --test api_integration setup_ local_login -- --nocapture`  
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add crates/site-server/tests/api_integration.rs
git commit -m "test: first-boot setup and local login API"
```

---

### Task 5: SPA — setup page, login page, routing gates

**Files:**
- Create: `crates/app/src/pages/setup.rs`
- Create: `crates/app/src/pages/login.rs`
- Modify: `crates/app/src/pages/mod.rs`
- Modify: `crates/app/src/routes.rs`
- Modify: `crates/app/src/main.rs` and/or `layouts/public.rs`, `layouts/admin.rs`
- Modify: `crates/app/src/pages/apply.rs` (login CTA)
- Modify: `crates/app/src/hooks/api.rs` if shared fetch helpers needed

- [ ] **Step 1: Routes**

Add outside or inside layouts carefully—setup should work with minimal chrome:

```rust
#[route("/setup")]
Setup {},
#[route("/login")]
Login {},
```

Prefer **no** heavy public layout requirement, or use PublicLayout.

- [ ] **Step 2: Setup page**

On mount: `GET /api/auth/setup-status`. If `!needs_setup`, navigate to `/login` or `/`.  
Form: username, password, confirm. Client-side: password length ≥ 12, match confirm.  
`POST /api/auth/setup` with credentials include cookies. On success: navigate `/admin/`, refresh auth state (`use_auth_init` / re-fetch `/api/auth/me`).

- [ ] **Step 3: Login page**

`GET /api/auth/providers`.  
If `local`: username/password form → `POST /api/auth/local/login`.  
If discord/google: existing link buttons.  
If `needs_setup` from status: redirect `/setup`.

- [ ] **Step 4: Gate in app root**

In `main.rs` or a small component wrapping `Router`: after auth init, fetch setup-status once; if `needs_setup` and current route is not `/setup`, navigate to `/setup`.

- [ ] **Step 5: Replace hard-coded Discord-only CTAs**

In `admin.rs` access-denied and `apply.rs`: link to `/login` (or show local + Discord based on providers) instead of only `/api/auth/discord/login`.

- [ ] **Step 6: Build check**

Run: `cargo check -p scuffed-app --target wasm32-unknown-unknown`  
Expected: success

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/pages/setup.rs crates/app/src/pages/login.rs crates/app/src/pages/mod.rs crates/app/src/routes.rs crates/app/src/main.rs crates/app/src/layouts/admin.rs crates/app/src/pages/apply.rs
git commit -m "feat(app): first-boot setup and local login UI"
```

---

### Task 6: Emergency admin password reset

**Files:**
- Modify: `crates/server/src/main.rs` (after migrations)
- Create: `scripts/reset-local-admin.sh`

- [ ] **Step 1: Boot-time reset**

After `run_migrations`, if env `BOOTSTRAP_ADMIN_RESET=1` and `BOOTSTRAP_ADMIN_PASSWORD` set:

```rust
let username = std::env::var("BOOTSTRAP_ADMIN_USERNAME").unwrap_or_else(|_| "admin".into());
// get_local_user_by_username, set_local_password_hash(hash_password(...))
// log warning: reset applied; unset BOOTSTRAP_ADMIN_RESET
```

Only when `SURREALDB_URL` is set (production path). Do not create users here (setup owns creation).

- [ ] **Step 2: Script**

`scripts/reset-local-admin.sh`: documents setting env vars and `podman compose up -d --force-recreate site-server`, or runs compose with env.

- [ ] **Step 3: Commit**

```bash
git add crates/server/src/main.rs scripts/reset-local-admin.sh
git commit -m "feat: emergency local admin password reset"
```

---

### Task 7: install.sh + secrets + gitignore

**Files:**
- Create: `scripts/install.sh`
- Modify: `.gitignore`
- Ensure `data/` exists (uploads already under `data/uploads`)

- [ ] **Step 1: gitignore**

Add:
```
data/secrets.env
```

- [ ] **Step 2: install.sh**

```bash
#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SECRETS="$ROOT/data/secrets.env"
cd "$ROOT"

command -v podman >/dev/null || { echo "podman required"; exit 1; }

mkdir -p "$ROOT/data"

# Return 0 if TCP port is free on the host (IPv4).
port_is_free() {
  local p="$1"
  if command -v ss >/dev/null 2>&1; then
    ! ss -tlnH "sport = :$p" 2>/dev/null | grep -q .
  else
    ! (echo >/dev/tcp/127.0.0.1/"$p") 2>/dev/null
  fi
}

pick_host_port() {
  local p candidates=(3000 8080 9090 3100 8888 18080)
  for p in "${candidates[@]}"; do
    if port_is_free "$p"; then
      echo "$p"
      return 0
    fi
  done
  # Random high port; retry a few times
  local i
  for i in $(seq 1 40); do
    p=$((9100 + RANDOM % 20000))
    if port_is_free "$p"; then
      echo "$p"
      return 0
    fi
  done
  echo "Could not find a free host port" >&2
  return 1
}

if [[ ! -f "$SECRETS" ]]; then
  echo "Generating $SECRETS ..."
  SURREALDB_PASSWORD="$(openssl rand -base64 32 | tr -d '\n')"
  ENCRYPTION_KEY="$(openssl rand -base64 32 | tr -d '\n')"
  HOST_PORT="$(pick_host_port)"
  echo "Selected free host port: $HOST_PORT (127.0.0.1 only)"
  read -rp "Public site URL [http://127.0.0.1:${HOST_PORT}]: " REDIRECT_BASE_URL || true
  REDIRECT_BASE_URL="${REDIRECT_BASE_URL:-http://127.0.0.1:${HOST_PORT}}"
  umask 077
  cat > "$SECRETS" <<EOF
# Generated by scripts/install.sh — do not commit
SURREALDB_USER=root
SURREALDB_PASSWORD=${SURREALDB_PASSWORD}
SURREALDB_NS=scuffed_crew
SURREALDB_DB=main
ENCRYPTION_KEY=${ENCRYPTION_KEY}
ENCRYPTION_KEY_VERSION=1
HOST_PORT=${HOST_PORT}
REDIRECT_BASE_URL=${REDIRECT_BASE_URL}
ALLOWED_ORIGINS=${REDIRECT_BASE_URL}
RUST_LOG=info
EOF
  chmod 600 "$SECRETS"
  echo "Wrote secrets (mode 600). DB password is not shown again; file: data/secrets.env"
else
  echo "Using existing $SECRETS"
  # shellcheck disable=SC1090
  set -a; source "$SECRETS"; set +a
  HOST_PORT="${HOST_PORT:-3000}"
fi

# Prefer podman compose; fall back to podman-compose
if podman compose version >/dev/null 2>&1; then
  COMPOSE=(podman compose)
elif command -v podman-compose >/dev/null; then
  COMPOSE=(podman-compose)
else
  echo "Need podman compose or podman-compose"; exit 1
fi

"${COMPOSE[@]}" --env-file "$SECRETS" up --build -d

echo
echo "Stack starting on 127.0.0.1:${HOST_PORT:-?} (see HOST_PORT in secrets)."
echo "Next:"
echo "  1. Point Caddy at 127.0.0.1:${HOST_PORT:-PORT} if public (see docs/deploy.md)"
echo "  2. Open ${REDIRECT_BASE_URL:-your URL} and create the admin account"
echo "  3. Infra secrets: $SECRETS"
```

Make executable: `chmod +x scripts/install.sh`

Compose publish line (Task 8):
```yaml
ports:
  - "127.0.0.1:${HOST_PORT:-3000}:3000"
```

- [ ] **Step 3: Smoke syntax**

Run: `bash -n scripts/install.sh`  
Expected: no output (ok)

- [ ] **Step 4: Commit**

```bash
git add scripts/install.sh .gitignore
git commit -m "feat: install.sh auto-generates secrets for Podman Compose"
```

---

### Task 8: compose.yml — env_file, uploads, relay profile

**Files:**
- Modify: `compose.yml`

- [ ] **Step 1: Update compose**

Key changes:

```yaml
services:
  surrealdb:
    # keep image/command; password from env_file
    env_file:
      - path: data/secrets.env
        required: false
    # still use ${SURREALDB_PASSWORD:?...} from env_file

  site-server:
    env_file:
      - path: data/secrets.env
        required: false
    environment:
      SURREALDB_URL: ws://surrealdb:8000
      SURREALDB_USER: ${SURREALDB_USER:-root}
      SURREALDB_PASSWORD: ${SURREALDB_PASSWORD:?Set SURREALDB_PASSWORD (run scripts/install.sh)}
      # ... ns, db, REDIRECT_BASE_URL, ENCRYPTION_KEY, etc.
      UPLOAD_DIR: /app/data/uploads
      NOSTR_RELAY_URL: ${NOSTR_RELAY_URL:-}
    volumes:
      - uploads-data:/app/data/uploads
    # remove hard dependency on strfry URL default if relay not running

  strfry:
    profiles: ["relay"]
    # only starts with --profile relay
    # fix depends_on as needed

volumes:
  surrealdb-data:
  uploads-data:
  strfry-data:
```

Default `NOSTR_RELAY_URL` empty when relay profile off (was `ws://strfry:7777`). Document setting it when enabling profile.

- [ ] **Step 2: Commit**

```bash
git add compose.yml
git commit -m "chore(compose): secrets env_file, uploads volume, optional relay profile"
```

---

### Task 9: Fix relay Containerfile

**Files:**
- Modify: `relay/Containerfile`

- [ ] **Step 1: Copy full workspace members needed for cargo**

Cargo requires all workspace member paths to exist. Options:

**Preferred:** copy entire `crates/` and root `Cargo.toml`/`Cargo.lock`:

```dockerfile
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo build --release -p relay-policy \
    && cp target/release/relay-policy /usr/local/bin/relay-policy
```

Use a rust version consistent with the monorepo (align with root `Containerfile` rust version if possible).

- [ ] **Step 2: Commit**

```bash
git add relay/Containerfile
git commit -m "fix(relay): copy full crates workspace for relay-policy build"
```

---

### Task 10: Backup script + secrets sourcing

**Files:**
- Modify: `scripts/backup.sh`
- Modify: `deploy/scuffed-backup.service` if needed for EnvironmentFile path

- [ ] **Step 1: Volume discovery**

Replace single name with loop:

```bash
VOLUME_PATH=""
for name in surrealdb-data scuffed-crew_surrealdb-data; do
  VOLUME_PATH="$(podman volume inspect "$name" --format '{{.Mountpoint}}' 2>/dev/null || true)"
  [[ -n "$VOLUME_PATH" ]] && break
done
# Also try: podman volume ls --format '{{.Name}}' | grep surrealdb-data
```

Include uploads:

```bash
for name in uploads-data scuffed-crew_uploads-data; do
  UP="$(podman volume inspect "$name" --format '{{.Mountpoint}}' 2>/dev/null || true)"
  [[ -n "$UP" && -d "$UP" ]] && BACKUP_PATHS+=("$UP")
done
```

Optional strfry volume same pattern.

- [ ] **Step 2: Document EnvironmentFile**

In `deploy/scuffed-backup.service`, prefer:
```
EnvironmentFile=-/opt/scuffed-crew/data/secrets.env
EnvironmentFile=-/opt/scuffed-crew/.env
```

Map `SURREALDB_ROOT_PASSWORD` from `SURREALDB_PASSWORD` in backup.sh:

```bash
ROOT_PASS="${SURREALDB_ROOT_PASSWORD:-${SURREALDB_PASSWORD:?Set SURREALDB_PASSWORD or SURREALDB_ROOT_PASSWORD}}"
ROOT_USER="${SURREALDB_ROOT_USER:-${SURREALDB_USER:-root}}"
```

Surreal HTTP URL for export: document host publishing or `podman exec` export alternative if DB not on localhost. For v1, add optional:

```bash
# If Surreal not published, export via exec:
# podman exec <surreal-container> ...
```

Minimum: if `SURREALDB_URL` is `ws://...`, convert to `http://` for CLI or document running backup inside the network. Simplest reliable approach for compose:

```bash
# Prefer exec into running surreal container when SURREAL_EXPORT_VIA_PODMAN=1
```

Implement: try `podman ps --filter name=surrealdb --format '{{.ID}}'` and `podman exec` with `surreal export` if binary exists in image; else existing HTTP URL path.

- [ ] **Step 3: Commit**

```bash
git add scripts/backup.sh deploy/scuffed-backup.service
git commit -m "fix(backup): compose volume names, uploads, secrets env"
```

---

### Task 11: Documentation

**Files:**
- Create: `docs/deploy.md`
- Modify: `.env.example`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Write `docs/deploy.md`**

Sections:
1. Prerequisites (Podman, compose plugin, openssl; Caddy for public HTTPS)
2. Happy path: `./scripts/install.sh`
3. First-boot create admin (browser)
4. What `data/secrets.env` contains (do not commit; DB password not day-to-day)
5. Caddyfile + DNS + `REDIRECT_BASE_URL`
6. Secure cookies / HTTPS note
7. Optional `--profile relay` + set `NOSTR_RELAY_URL=ws://strfry:7777`
8. Backups (restic init, timer, EnvironmentFile)
9. Admin password recovery (`reset-local-admin.sh` / env)
10. Power-user manual env from `.env.example`
11. **Optional later: Quadlet** — migrate Compose services to systemd Quadlet for boot-native management; not required for first install; no units shipped yet

- [ ] **Step 2: Expand `.env.example`**

Document all vars including install-generated ones, with comments that install.sh is preferred for novices. No required admin password vars for happy path.

- [ ] **Step 3: CLAUDE.md**

Add short "Production / VPS" blurb pointing to `docs/deploy.md` and `scripts/install.sh`.

- [ ] **Step 4: Commit**

```bash
git add docs/deploy.md .env.example CLAUDE.md
git commit -m "docs: VPS deploy runbook and env reference"
```

---

### Task 12: End-to-end verification

- [ ] **Step 1: Unit/integration**

```bash
cargo test -p scuffed-auth --features server
cargo test -p scuffed-site-server --test api_integration
```

Expected: PASS (or pre-existing failures unrelated—only require new tests green).

- [ ] **Step 2: Install dry-run (if Podman available)**

```bash
# Use a throwaway secrets path if needed
./scripts/install.sh
curl -sS http://127.0.0.1:3000/api/health
curl -sS http://127.0.0.1:3000/api/auth/setup-status
# POST setup with long password; verify needs_setup false; POST local login
```

If Podman unavailable in CI agent environment, document manual verification for the operator.

- [ ] **Step 3: Final commit only if fixes needed**

---

## Spec coverage checklist

| Spec requirement | Task |
|------------------|------|
| First-boot admin UI + API | 3, 5 |
| Local login after setup | 3, 5 |
| Argon2 hash, no plaintext | 1, 2 |
| No admin password in env (normal path) | 3, 7, 11 |
| install.sh auto-generates DB password, encryption key, free HOST_PORT | 7, 8 |
| Optional URL prompt only | 7 |
| Compose env_file + uploads volume | 8 |
| Relay optional profile | 8, 9 |
| Relay Containerfile fix | 9 |
| Backup volume names + uploads | 10 |
| docs/deploy.md + Quadlet later note | 11 |
| Emergency admin reset | 6 |
| Success criteria smoke | 12 |

---

## Out of scope (do not implement)

- Kubernetes
- Quadlet unit files
- Browser DB password UI
- Discord/Google OAuth app registration guides
- Matrix homeserver
