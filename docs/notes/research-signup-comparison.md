# Signup paths — comparison (claude synthesis, 2026-07-17)

Constraint (user decision, standing): privacy-first — no third-party OAuth may ever be
the *only* signup path. All paths create a **bare user**; membership still goes through
the application review. Spam exposure is therefore identical and low for every option.

| | Local self-reg | Nostr (NIP-07) | Discord OAuth | Matrix |
|---|---|---|---|---|
| Privacy | ✦✦✦ no email, no 3rd party | ✦✦✦ self-sovereign key, no 3rd party | ✦ links Discord identity | ✦✦ homeserver sees auth (self-hostable) |
| Onboarding friction | lowest (username+password) | needs a Nostr key + NIP-07 extension | lowest for Discord users | needs a Matrix account + flow TBD |
| Implementation effort | ~1 day (designed, see research-account-creation-claude.md) | ~1–1.5 days (see below) | ~15 min config (already implemented) | TBD — grok researching (OIDC/MSC3861 vs password login) |
| Recovery | admin-set temp password | user owns key (lost key = lost account; relink via admin) | Discord handles it | homeserver handles it |
| Fit with stack | Governor + password utils exist | **strong** — org already runs Nostr chat, members hold nostr keys, unique pubkey index exists | implemented, just unconfigured | Matrix used for notifications only today |

## Nostr signup — feasibility notes (claude lane)

Existing primitives make this cheap:
- `user` table already models external identity as `provider` + `provider_id(_hash)`
  (discord/google) — nostr becomes a third provider with `provider_id = pubkey`,
  reusing `upsert_user_from_oauth`-style creation.
- Challenge-response machinery already exists in `routes/nostr.rs` (HMAC-signed
  challenge tokens, `state.nostr_challenge_key`) for *linking* a pubkey to a member;
  a login variant is the same dance without a session: issue challenge → NIP-07
  extension signs event → server verifies sig → known pubkey = login, new = signup.
- The SPA already talks to `window.nostr` (identity settings link flow).

Flow: Login page "Sign in with Nostr" → `GET /api/auth/nostr/challenge` →
extension signs → `POST /api/auth/nostr/verify` → session cookie. Member-linked
pubkeys (existing `member_nostr_pubkey_idx`) map to their user; unknown pubkeys
create a bare user. Same Governor rate limits.

Caveat: gamers without a Nostr extension won't use this — it complements
local self-reg, doesn't replace it.

## Matrix — grok findings (docs/notes/research-signup-matrix-grok.md)

Verdict: **NOT MVP.** The `matrix` provider exists only as a stub (placeholder OIDC
URLs, routes never wired); runtime Matrix is outbound bot notifications only. A real
implementation needs a self-hosted homeserver + Matrix Authentication Service running
first (ops-heavy, days–weeks), for an audience local+Nostr already covers. Per
`brief.md`, self-hosted Matrix is post-launch coordination chat, not launch identity.
Revisit only after the org actually runs a homeserver.

## FINAL recommendation (both lanes in, 2026-07-17)

Ship in this order:
1. **Local self-registration** — the privacy-first baseline everyone can use (~1 day, fully designed).
2. **Nostr login/signup** — near-free given existing NIP-07/challenge primitives; flagship privacy feature (~1–1.5 days).
3. **Discord OAuth** — optional convenience, config-only, enable after 1–2 exist so it is never the gate.
4. **Matrix** — deferred until the org self-hosts a homeserver+MAS; stub already anticipates it.
