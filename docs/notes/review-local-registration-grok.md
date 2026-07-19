# Review: feat/local-registration@1b48a29

**Reviewer:** grok  
**Verdict:** CHANGES REQUESTED (2 blockers)  
**Date:** 2026-07-17

## Summary

Privacy-first local self-registration + admin password reset. Design matches the research decision (local primary, no email, recruitment_open kill switch, bare user, application gate). Rate-limited with existing auth governor. Integration tests cover happy path, validation/dup/closed, admin reset authz + OAuth 400. New tests green (3/3).

## Blockers

### 1. SECURITY — password reset leaves sessions alive
`admin_reset_local_password` updates the hash and audits, but does **not** call `delete_sessions_for_user`. Deactivate/ban/moderation already revoke sessions. Admin recovery after compromise is incomplete if the attacker keeps a cookie.

**Fix:** after successful `set_local_password_hash`, call `delete_sessions_for_user(&member.user_id)` (log error, don't fail the reset if revoke fails — same pattern as deactivate). Optional test: register → login session works → admin reset → same cookie `/api/auth/me` is unauthorized.

### 2. Accidental `.memdb` commit
Branch adds `.memdb/daemon-state.json` and `.memdb/daemon.pid` (local agent paths/PIDs). Not product code.

**Fix:** remove from the commit; add `.memdb/` (or at least those files) to `.gitignore`.

## Non-blocking notes

- **PW button** on every admin member row (OAuth gets 400). Fine for v1; later hide when Member exposes provider.
- **Register entry** is nested under `show_local`. Edge case OAuth-only org with `register=true` and no local users can't open the form. Prod path (setup admin → local exists) is fine.
- Client register form does not pre-check min password length (server does). Minor UX.
- Username uniqueness is check-then-insert (no DB UNIQUE); same as setup. Rate limit reduces double-click races; not new risk class.

## What looks good

- `recruitment_open` kill switch + providers `register`/`min_age` flags with serde defaults
- Bare user only (no member on register); funnel to `/apply`
- `MIN_PASSWORD_LEN` + `validate_local_username` reuse
- Dup → 409 via `already taken` string (matches `create_local_user`)
- Admin-only extractor + local-provider gate + audit line
- CSS uses design tokens (`--accent`, `--text-*`, etc.)

## CI note

`fix/ci-green@df1c47d` (rustfmt + design-token) was already **APPROVED and MERGED** by Grok earlier; main CI is green at `d4f8ada`. This review is for the READY local-registration feature, not a new CI-only fix.
