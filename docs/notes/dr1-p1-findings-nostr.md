# DR-1 P1 Findings — NOSTR lane (grok)

Author: grok · Date: 2026-07-19 · Anchor: origin/main@d9eb7ec
Scope: NOSTR partition only. IDs for P2 CONFIRM/REFUTE.

Severity guide: CRIT=exploitable now / secret integrity; HIGH=real authz/crypto gap;
MED=defense-in-depth or correctness; LOW/NIT=polish.

---

## DR1-NOSTR-001 · CRIT
**file:** `crates/site-server/src/main.rs:476-489`
**claim:** Missing `NOSTR_CHALLENGE_SECRET` falls back to a **hard-coded deterministic
key** in all builds. Production only skips the warn log (`if is_dev`); it does
**not** refuse to boot.
**scenario:** Deploy omits `NOSTR_CHALLENGE_SECRET`. Attacker who knows the fixed
dev string (`scuffed-crew-dev-nostr-challenge-key`) forges valid challenge tokens
for any `member_id` and drives `/api/nostr/verify` / login challenge paths that
trust the MAC.
**fix:** Fail closed when `!is_dev && env missing`; or require secret whenever
nostr routes are mounted.

## DR1-NOSTR-002 · HIGH
**file:** `crates/site-server/src/routes/nostr.rs:190-191` (`verify_challenge_token`)
**claim:** Challenge MAC compare is **not constant-time** (`!=` on hex strings).
**scenario:** Remote timing assist on forged tokens (stacked with weak/dev key
makes worse). Same class as CSRF cookie plain-equality (AUTH lane).
**fix:** `subtle::ConstantTimeEq` on raw MAC bytes.

## DR1-NOSTR-003 · HIGH
**file:** `crates/site-server/src/routes/nostr.rs:501-574` (`nostr_import_key`)
**claim:** "Import key" accepts `ncryptsec` + **password over HTTP**, decrypts the
nsec **on the server**, derives pubkey, then calls `set_external_nostr_key` which
**clears** any server-managed secret and sets mode=external. The decrypted secret
is never re-stored as server-managed — so this is not a restore path; it is a
destructive mode flip that briefly holds user nsec in server RAM and requires
trusting TLS + server for password+backup.
**scenario:** User thinks "restore backup into server-managed" → loses
server-managed key material; or compromised site-server logs/memory captures nsec
during import.
**fix:** Client-side-only import for external mode (NIP-07); or if server restore
is intended, re-encrypt under CryptoService AAD and keep ServerManaged; never
accept password+ncryptsec unless restore semantics are explicit + audited.

## DR1-NOSTR-004 · HIGH
**file:** `crates/relay-policy/src/policy.rs:79` + `:131-137` + main (no DB group load)
**claim:** `enforce_group_membership` defaults **false**; `update_group_members` is
`#[allow(dead_code)]` and **not wired** from DB in the relay-policy binary.
Any pubkey on the org allowlist can write NIP-29 group events for **any** `h` tag.
**scenario:** Compromised or malicious member (or stolen member key) posts into
officer/private group IDs they are not in; policy accepts.
**fix:** Default enforce=true for production; load group membership from DB on
refresh; reject unknown groups.

## DR1-NOSTR-005 · MED
**file:** `crates/site-server/src/routes/nostr.rs:432-438` (`nostr_export_backup`)
**claim:** Backup password minimum is **8** chars; account passwords require 12
(`auth` password.rs). Export yields ncryptsec of the live nsec.
**scenario:** User picks short backup password; offline brute of ncryptsec is
cheaper than account password policy implies.
**fix:** Align min with `MIN_PASSWORD_LEN` (12) + zxcvbn or same Argon params
hint in UI; rate-limit export.

## DR1-NOSTR-006 · MED
**file:** `crates/site-server/src/routes/nostr.rs` challenge/verify/export/import/dm_send
**claim:** **No application-level rate limit** on challenge issuance, verify,
export-backup, import-key, or dm_send (relay-policy rate-limits relay events only).
**scenario:** Authenticated member burns CPU on NIP-49 encrypt/decrypt or floods
gift-wrap publish; challenge spam fills logs.
**fix:** per-member token bucket on secret-touching routes; especially export/import.

## DR1-NOSTR-007 · MED
**file:** `nostr.rs:1434` vs `db/types.rs` `conversation_key`
**claim:** Gift-wrap `context_id` uses **order-dependent**
`dm:{sender}:{recipient}` while DB `conversation_key` is **sorted** pair form.
**scenario:** Relay filters / client h-tag conventions disagree with stored
conversation identity; harder multi-device sync; possible split threads if any
code assumes symmetry.
**fix:** Use the same canonical `conversation_key(a,b)` for context_id/h-tag.

## DR1-NOSTR-008 · MED
**file:** `nostr.rs:357-364`, `481-488`, `564-571` audits
**claim:** Key link / export / import all audit as `AuditAction::UpdatedMember`
with free-text detail only — not distinct actions.
**scenario:** Security review / SIEM cannot filter "key exported" vs profile edit
without parsing detail strings.
**fix:** Dedicated audit actions: `NostrKeyLinked`, `NostrKeyExported`,
`NostrKeyImported`, `NostrKeyUnlinked`.

## DR1-NOSTR-009 · LOW
**file:** `nostr.rs:203-209` (`nostr_challenge` resolve_pubkey error)
**claim:** Invalid pubkey resolve returns generic **"Internal error"** + 400,
masking client mistakes (and differing from later "Invalid secp256k1 public key").
**fix:** Stable 400 with explicit invalid-pubkey message.

## DR1-NOSTR-010 · NIT / positive
**file:** `db/types.rs:76` `#[serde(skip_serializing)]` on
`Member.nostr_secret_key_encrypted`; `MEMBER_SAFE_COLS` omits secret on list paths;
`require_server_managed_dm_caller` loads full member only when needed.
**claim:** Secret-not-in-JSON posture looks correct for HTTP responses checked.
**note:** Keep P2 confirm that no `Debug`/logs print `EncryptedBlob` plaintext and
that `get_member` is never returned through a custom serializer that bypasses skip.

## DR1-NOSTR-011 · MED (correctness)
**file:** `nostr.rs:501-541` + `db/nostr_keys.rs:168-176`
**claim:** After external transition, previous server-managed ciphertext is cleared
in DB update path — good for not leaving orphan secrets — but there is **no
re-encrypt/destroy confirmation** and no session revocation tied to key mode change.
**scenario:** Sessions remain valid after import/unlink; stolen session still hits
APIs as that member (expected for session model) but key ops should force step-up.
**fix:** Optional step-up reauth for export/import/unlink; document.

---

## Deferred to DB lane / P1-wave-2
- strategies dynamic SQL, sessions hash storage, rewrap completeness
  → `DR1-DB-*` in separate file.

## P2 handoff
CRIT/HIGH for independent re-derive: 001, 002, 003, 004.
MEDs sample: 005, 006, 007.
