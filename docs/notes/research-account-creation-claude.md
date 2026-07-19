# Research: why nobody can make an account (claude, 2026-07-17)

## Root cause — confirmed on prod

`GET https://ow.scuffedcrew.no/api/auth/providers` → `{"local":true,"discord":false,"google":false}`

Account creation paths in the codebase, exhaustively:

| Path | Status on prod |
|---|---|
| One-time `/api/auth/setup` (first admin) | consumed (`needs_setup:false`) |
| OAuth callback `upsert_user_from_oauth` (Discord/Google) | **dead — both providers unconfigured** (`DISCORD_CLIENT_ID`/`GOOGLE_CLIENT_ID` empty in `data/secrets.env`; compose defaults them to empty) |
| Local username/password | **login-only** — `create_local_user` is only reachable from setup; no register endpoint exists |
| `POST /api/applications` (join the org) | requires `AuthUser` — **can't even apply without an account** |

Net: the login page shows a username/password form (for the one admin) and no
signup affordance of any kind. Prospective members hit a wall.

## Options

**A. Config-only — Discord OAuth (recommended first step, ~15 min, no code)**
1. Discord Developer Portal → create application → OAuth2.
2. Redirect URL: `https://ow.scuffedcrew.no/api/auth/discord/callback`.
3. VPS `data/secrets.env`: set `DISCORD_CLIENT_ID`, `DISCORD_CLIENT_SECRET`,
   and `REDIRECT_BASE_URL=https://ow.scuffedcrew.no` (currently defaulting to
   `http://127.0.0.1:3000` — OAuth would bounce to localhost without this).
4. Recreate site-server. Login page auto-shows "Sign in with Discord";
   first sign-in creates the user; they can then submit an application.
   Gaming org → Discord-first is the natural fit; zero new attack surface.

**B. Local self-registration (feature work, needs product decisions)**
New `POST /api/auth/local/register` + signup UI. Requires: rate limiting
(Governor already wraps auth routes), password rules, 16+ ToS checkbox,
probably invite codes or approval gating to keep drive-by spam out.
~1-2 days including review. Only worth it if Discord-less members matter.

**C. A now, B later** — unblocks members today, keeps options open.

## USER DECISION (2026-07-17)
Privacy-first: OAuth must never be the only signup path. **Option B is required**;
Discord OAuth (A) may be offered as optional convenience later, or not at all.

## Design: local self-registration (privacy-first)

**Endpoint** — `POST /api/auth/local/register`, inside the existing rate-limited
`auth_routes` (Governor: 5-burst, 1/2s per IP — already in place).

- Fields: `username` (unique, same ident rules as setup), `password`
  (reuse `MIN_PASSWORD_LEN` + `hash_password`), `confirm_16_plus: bool` (brand
  requirement; 400 unless true). **No email — data minimization.**
- Creates a bare **user** only (no member row) — identical trust level to the
  OAuth path today: the only thing a bare user can do is submit/withdraw an
  application; org content stays behind the `OrgMember` extractor. The existing
  application review remains the real membership gate, so open registration
  adds no meaningful attack surface.
- Duplicate username → 409. Auto-login on success (session cookie, same as setup).
- **Kill switch:** `registration_open` flag in site settings (admin-togglable,
  default on) so drive-by spam waves can be shut off without a deploy.

**Login page** — add a "Create account" section when `registration_open`
(providers endpoint gains the flag).

**Recovery (no email on file)** — MVP: admin/officer sets a temporary password
from the admin members page (AdminUser-gated endpoint, audit-logged, forced
distinct from old hash). Self-serve recovery without email is out of scope.

**Effort:** ~1 day incl. tests + cross-review. Server: register endpoint +
settings flag + admin reset. App: signup form on login page + admin reset button.
