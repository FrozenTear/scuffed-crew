# DR1-NOSTR-001 — Replay-path adversarial verification

## VERDICT: CONFIRMED-HIGH

The replay attack is **real and unblocked** — nothing in the verify path prevents
re-submitting an old victim-signed kind-22242 event under a freshly-minted token
when the MAC key is known. It is **not CRIT** because the login events are **never
broadcast to a relay**; they travel only as a direct HTTPS POST to the server, so
capturing one requires MITM / TLS interception or a privileged network/log
position — not passive relay observation.

---

## Q1: Does the replay work? YES.

Line-by-line trace of `nostr_login_verify` (auth.rs:756-782):

1. `verify_challenge_token(key, body.token)` (auth.rs:764) → `nostr.rs:155-195`.
   This is **fully stateless**: base64-decode, split on `|`, check `now >
   expires_ts` (nostr.rs:181), recompute `blake3::keyed_hash` and compare
   (nostr.rs:187-191). **No DB lookup, no consumed-challenge set, no nonce
   store.** The challenge string lives *inside* the token; the server keeps no
   record of which challenges it issued or consumed.
2. `subject != NOSTR_LOGIN_SUBJECT` (auth.rs:770) → only checks the literal
   `"@login"` marker, which the attacker sets when minting the token.
3. `kind != Custom(22242)` (auth.rs:773) → captured event already is 22242.
4. `content != challenge` (auth.rs:776) → attacker mints the token with
   `challenge` = the captured event's exact content string, so this passes.
5. `signed_event.verify()` (auth.rs:779) → nostr 0.44.2, `Event::verify` =
   `verify_with_ctx` (event/mod.rs:160-183): checks **only** `verify_id()`
   (id == hash of pubkey/created_at/kind/tags/content) and
   `verify_signature_with_ctx()` (Schnorr). **No `created_at` recency/freshness
   check anywhere** — not in the crate, not in the handler. An event signed
   months ago verifies identically to one signed now.
6. `pubkey_hex = signed_event.pubkey` (auth.rs:782) → the **victim's** pubkey →
   member/user lookup → session minted as the victim (auth.rs:785-868).

**Deciding fact:** the token's MAC+TTL is the *only* replay guard, and it is
stateless + attacker-mintable under a known key. Combined with `Event::verify()`
validating only id+signature (never `created_at`), there is **no one-time-use,
no request-binding, and no event-freshness** anywhere in the path. A single
captured victim login event is a permanently-replayable credential.

This is the guard claude hypothesized and grok did not analyze — it is genuinely
absent. Neither a nonce store nor a created_at check exists.

## Q2: Are the events capturable? HARD (MITM-class).

Client login flow `nostr_login_flow` (login.rs:504-576):
- Fetches challenge (login.rs:509), calls the browser NIP-07 extension
  `window.nostr.signEvent` (login.rs:543-556), then `post_json` the signed event
  **directly to `/api/auth/nostr/verify`** (login.rs:560-568).
- **No `publish_event` / no relay write** in the login path (confirmed: the only
  outbound calls are the challenge `fetch` and the verify `post_json`).

Kind-22242 is NIP-42 AUTH — by spec not stored/rebroadcast by relays — and here
it isn't even sent to a relay at all. So the "ephemeral events are relayed and
observable" premise in the escalation does **not** hold for this flow. Capture
therefore requires one of:
- MITM / TLS interception of the victim's POST body (hard),
- a malicious NIP-07 extension (which already holds the victim's key → moot),
- server-side request-body logging / host compromise (post-auth).

None of these is passive/remote observation. That drops the exploit from
"unauthenticated remote takeover" to "takeover given a privileged network/log
position" → **HIGH, not CRIT.**

## Notes
- Each challenge is fresh 32-byte OsRng (auth.rs:719-722), so the attacker cannot
  pre-sign a predicted challenge — they must capture a real event. But because
  replay of an *old* captured event is unbounded (no freshness/nonce), one
  capture suffices and never expires.
- The fix (fail-closed on unset `NOSTR_CHALLENGE_SECRET` + provision in
  install.sh) is correct and cheap at either severity. Independently, adding an
  event `created_at` freshness window and/or a one-time-challenge store would
  close the replay path even if the key ever leaked — worth doing regardless.
