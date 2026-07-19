# Research: Matrix as a signup path (grok, 2026-07-17)

**Lane:** complement Claude’s `research-signup-comparison.md` and `research-account-creation-claude.md`.  
**Constraint:** privacy-first — third-party OAuth must never be the *only* path (user decision).

## What exists today

| Surface | Status |
|---------|--------|
| DB `AuthProvider` / migration assert | **`matrix` is a legal provider value** (`crates/db` migrations; `AuthProvider::Matrix`) |
| `crates/auth/src/server/matrix.rs` | **Stub only** — `MatrixProvider` implements `OAuthProvider` with placeholder OIDC URLs (`example.com`); comments say “when MAS/OIDC is ready” |
| Site-server routes | **Not wired** — `login`/`callback` match only `discord` / `google`; `auth_providers` JSON has no `matrix` flag |
| Runtime Matrix | **Bot notifications only** — `MatrixNotifier` uses `MATRIX_HOMESERVER_URL` + bot access token + room IDs (Client-Server API send). No user login |
| Product intent (`brief.md`) | Self-hosted Matrix (Continuwuity + Cinny) is **post-launch coordination chat**, not launch identity |

Net: schema and a stub anticipates Matrix OIDC someday; **nothing shippable for signup**; ops surface today is notify-out, not SSO-in.

## Signup options (technical)

### A. MAS / OIDC (MSC3861-era Matrix Authentication Service)

- Modern Matrix stacks can put **MAS** in front of the homeserver as an OIDC IdP.
- Our stub assumes: `issuer_url` + client id/secret + standard authorize/token/userinfo (same shape as Google/Discord `OAuthProvider`).
- **Work to make real:**
  1. Deploy/configure homeserver **and** MAS (or equivalent OIDC issuer).
  2. Register an OIDC client for scuffed-crew; redirect  
     `{REDIRECT_BASE_URL}/api/auth/matrix/callback`.
  3. OIDC discovery → fill `ProviderConfig` (replace stub URLs).
  4. Wire `matrix` arm in site-server login/callback + `auth_providers`.
  5. Map `sub` → `provider_id` (already in stub); upsert via existing OAuth user path.
- **Effort:** **ops-heavy (days–weeks if homeserver/MAS not live)**; app code ~0.5–1 day once issuer works.
- **Privacy:** good *if* homeserver is **self-hosted by the org** (no Discord). Still a second identity system to run/secure.

### B. Homeserver password / login token (CS API)

- `POST /_matrix/client/v3/login` with `m.login.password` (or token) → verify → create local session.
- **Not** our current OAuth trait path; custom route, secrets handling, rate limits.
- Couples site accounts to Matrix password policy; recovery lives on HS; poor fit for “gamer without Matrix”.
- **Effort:** ~1–2 days code + still need a homeserver and accounts.
- **Recommendation:** reject for public signup (ops + UX cost, little upside over local register).

### C. “Already have Matrix → deep link only”

- Keep Matrix **out of signup**; after local/Nostr account, optional “link Matrix” for rooms later.
- Matches brief: Matrix as coordination upgrade, not onboarding gate.

## Fit vs other paths (for the comparison table)

| Dimension | Finding |
|-----------|---------|
| Implementation effort | **High if counting infrastructure**; app stub is ~30% of OAuth wire-up. Honest estimate: **3–10+ days** first time (MAS + HS + app), not “config only” |
| Onboarding friction | High for recruits without Matrix; low only for members already on org HS |
| Privacy | Strong only on **org-run** HS/MAS; worse if relying on matrix.org or random public HS |
| Stack fit | Weak for **signup** today (notifications only). Strong later for **team chat** once Continuwuity is deployed |
| Users not covered by local+Nostr | Almost none at launch — privacy users get local/Nostr; Discord users get optional OAuth later |

## Recommendation (Grok)

1. **Do not implement Matrix signup in the first privacy-first ship.**  
2. Order stays: **local self-reg → Nostr (NIP-07) → optional Discord config → Matrix later**.  
3. **When to reopen Matrix identity:** after Continuwuity (or chosen HS) is **production for members** *and* MAS (or OIDC) is intentional — then flesh `MatrixProvider` + route arms (low app delta, real ops already paid).  
4. Until then, leave `AuthProvider::Matrix` / stub as forward-compat; no env surface in `auth_providers`.

## Sync for unified plan (Claude)

- Mobile audit: `docs/notes/research-admin-mobile-grok.md` (P0 shell + DataTable scroll).  
- Matrix: **optional phase-3 identity**, not a blocker for register design.  
- Unified plan for user should treat Matrix signup as **out of MVP** with a one-liner rationale above.
