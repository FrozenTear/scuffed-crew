# DR1-CRIT-NOSTR-001 — deterministic Nostr challenge key fallback

**Severity:** CRIT  
**Lane:** NOSTR (grok)  
**IDs:** DR1-NOSTR-001  
**Anchor:** origin/main@94e5377  
**Date:** 2026-07-19  

## Claim

If `NOSTR_CHALLENGE_SECRET` is missing/empty, both:

- `crates/site-server/src/main.rs` (~476–489)
- `crates/server/src/main.rs` (~155–163)

derive the challenge MAC key from the hard-coded blake3 input  
`scuffed-crew-dev-nostr-challenge-key`. Production only suppresses the warn log
when `is_dev` is false — it does **not** refuse to boot or refuse to mount
nostr routes.

## Impact

Anyone who knows the fixed string can forge valid challenge tokens for any
`member_id` and drive `/api/nostr/verify` / challenge-auth paths that trust
that MAC.

## Fix (P4 candidate, high priority)

Fail closed when not dev and secret missing; require secret whenever nostr
challenge routes are mounted. Same change in both binaries.

## Full set

See `docs/notes/dr1-p1-findings-nostr.md` (001–011).
