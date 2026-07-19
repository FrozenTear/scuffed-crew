# DR1 P2 Adversarial Verdicts — NOSTR-002 & NOSTR-003 (verifier: claude)

Anchor: working tree @ main. Read-only. Author of code = grok.

---

## DR1-NOSTR-002 — VERDICT: CONFIRMED-LOWER (LOW, defense-in-depth)

**Claim:** challenge MAC compared with non-constant-time `!=` on hex strings → remote timing side-channel. Grok = HIGH.

### (a) Is the compare non-constant-time? YES.
`crates/site-server/src/routes/nostr.rs:190`
```rust
if expected_hex.as_str() != provided_hmac {
    return Err("Invalid token signature");
}
```
`!=` on `&str` is byte-wise and short-circuits on the first differing byte. Not constant-time. Claim (a) is factually correct.

### (b) Real exploitability? NOT realistic remotely.
The MAC is `blake3::keyed_hash(key, "{challenge}:{member_id}:{expires_ts}")`, rendered to 64 hex chars. An attacker *can* in principle mount the classic byte-at-a-time timing attack (fix challenge/member_id/expires, vary `provided_hmac`, measure response latency to learn how many leading hex chars matched). But:
- The short-circuit delta is a **single string-compare iteration (~1 ns)** buried under a full blake3 hash + base64 decode + `u64::parse` + `SystemTime::now`. Signal is ~nanoseconds; network jitter is **milliseconds** — 6 orders of magnitude larger. Extracting a per-byte oracle for a 64-char hex string this way over HTTP is not practically feasible.
- Grok argues it stacks with the weak/dev key (NOSTR-001). It does not: if the challenge key is known, the attacker just **computes the MAC directly** — the timing channel adds nothing. When the key is unknown (correct deploy), the timing channel is the only avenue and it's impractical. So it never rises to HIGH.

### Codebase consistency (grok's "same class as CSRF" claim) — CONFIRMED.
`crates/auth/src/server/session.rs:55`: `(Some(stored), Some(received)) if stored == *received` — OAuth CSRF state is also plain `==`. So non-constant-time token comparison is a **codebase-wide convention**, not a nostr-specific gap. (CSRF is even less relevant: attacker would already need the cookie value.)

### Severity call: **LOW (defense-in-depth).** Real correctness nit, not a reachable forge.
**Fix scope (1-liner):** `subtle::ConstantTimeEq::ct_eq` on the raw 32-byte `blake3::Hash` (`expected.as_bytes()`) vs a hex-decoded `provided_hmac`; length-check first. Batch the CSRF `==` in the same pass for consistency.
**When:** MORNING / backlog. Not overnight-blocking.

---

## DR1-NOSTR-003 — VERDICT: CONFIRMED-LOWER (MED, UX/mode-flip footgun; NOT key-loss/secret-exposure)

**Claim:** import-key decrypts nsec server-side, then `set_external_nostr_key` clears the server-managed secret and flips mode=external — destructive, not a restore; holds nsec in RAM. Grok = HIGH.

### (a) Does it really clear the server-managed secret? YES — verified end-to-end.
`crates/db/src/queries/nostr_keys.rs:168-177` `set_external_nostr_key` calls `update_member_nostr_keys(id, Some(pubkey), Some("external"), None)`.
`crates/db/src/queries/members.rs:488-501`:
```sql
UPDATE $rid SET nostr_pubkey=$pubkey, nostr_key_mode=$key_mode,
                nostr_secret_key_encrypted=$enc RETURN AFTER
```
`$enc` is bound from `None` → NULL. This is an **unconditional SET**, not a COALESCE — so the server-managed encrypted secret **is destroyed** in the DB. Premise (a) confirmed.

### (b) Is the decrypted nsec persisted / logged? NO — transient only.
`nostr.rs:506` decrypt into `Zeroizing<String>`; `:518` derive pubkey via `SecretKey::from_hex`; `:526` `secret_hex.zeroize()`. Only `pubkey_hex` is persisted. The nsec is never written to DB, never logged (no `tracing` of it). Crucially, holding the server-managed nsec in RAM transiently is **exactly what already happens on every server-managed post/react/DM** (`get_nostr_secret_key` decrypts server-side each call). Import introduces **no new secret-exposure surface**. Grok's "secret-exposure" framing is REFUTED.

### (c) Real footgun? Partly — a UX/mode-flip trap, recoverable.
- The Import section (`identity.rs:458-489`) is rendered **unconditionally** (outside the `if is_server_managed` guard) and is labeled *"Import Key (NIP-49) — Restore a key from an ncryptsec backup."* That copy is **misleading**: for a server-managed user it does not restore server-managed; it wipes the server-managed secret and converts the account to **external** mode.
- Consequence: user loses server-side DM/post/react (all gated on `ServerManaged`) and the DB copy of their key, with **no confirmation dialog**.
- But it is **recoverable and not key-loss**: the user still holds the ncryptsec backup they just imported, and the **pubkey/identity is preserved** (same key). They can operate in external mode via NIP-07, or unlink + re-provision (fresh key). Nothing is cryptographically destroyed that the user can't reproduce from their backup.

### Severity call: **MED.** Real destructive-without-warning mode flip + misleading label; NOT HIGH (no key-loss, no new secret exposure, TLS trust model identical to login/export password).
**Fix scope:** (1) fix UI copy — say "converts your account to external (NIP-07) mode and removes the server-managed key"; (2) add a confirm step; and/or (3) only show Import when `!is_server_managed`, or offer a true re-encrypt-under-CryptoService path that keeps ServerManaged. Optionally distinct audit action (ties to NOSTR-008).
**When:** MORNING. UX-layer fix, not overnight-blocking.

---

## One-line summary
- DR1-NOSTR-002: CONFIRMED-LOWER → **LOW**. Non-CT compare is real but blake3-hex-MAC timing forge is impractical over the network; same non-CT class as CSRF `==` (codebase-wide). Trivial `subtle` fix, morning.
- DR1-NOSTR-003: CONFIRMED-LOWER → **MED**. Mode flip really clears the server-managed secret (unconditional SET NULL) and the "Restore" label misleads, but nsec is transient/zeroized/not logged and identity+backup survive — a recoverable UX footgun, not HIGH key-loss/secret-exposure. Morning.
