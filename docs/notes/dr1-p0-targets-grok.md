# DR-1 P0 Recon — Owned Target Lists (grok)

Repo: scuffed-crew · Author: grok (P0 recon) · Date: 2026-07-19
Anchor: origin/main@d9eb7ec · Plan: docs/plans/2026-07-19-dr1-deep-review.md
Partition binding (A2): grok = **NOSTR + DB**. Claude owns AUTH/ACCT/ADMIN/FRONT/QUAL
(see docs/notes/dr1-p0-targets-claude.md). Aim-only — no findings yet (P1 waves).

Cross-lane flags (do not double-own):
- `MEMBER_SAFE_COLS` lives in `db/queries/members.rs` (ACCT) but NOSTR must audit
  any path that projects full member/nostr secret fields into chat/DM responses.
- Session crypto / cookie flags = AUTH; DB sessions store only token hashes.
- `CryptoService` primitives = AUTH; this lane reviews *call sites* + AAD binding.

---

## LANE: NOSTR

Scope: `crates/chat`, `crates/relay-policy`, `site-server` nostr + dm routes,
`db` nostr_keys/dms, `server` chat wiring.

### crates/chat/src/nostr/encryption.rs (546)
| file:line | symbol | why it matters | prio |
|---|---|---|---|
| encryption.rs:70-76 | `EncryptionService` / `new` | wraps CryptoService for NIP-44/59 | P1-first |
| encryption.rs:84-107 | `decrypt_member_keys` | secret blob → Keys; zeroize + pubkey match | P1-first |
| encryption.rs:121-141 | `encrypt_nip44` | pairwise encrypt; wrong-recipient risk | P1-first |
| encryption.rs:143-169 | `decrypt_nip44` | fail-closed on wrong key | P1-first |
| encryption.rs:171-235 | `build_gift_wraps` | NIP-59 multi-recipient gift wrap | P1-first |
| encryption.rs:237-283 | `unwrap_gift_wrap` / `_json` | server-side unwrap; kind checks | P1-first |
| encryption.rs:285-301 | `keys_from_secret_plaintext` | raw secret → Keys | P1-second |

### crates/chat/src/nostr/auth.rs (333)
| file:line | symbol | why | prio |
|---|---|---|---|
| auth.rs:67-84 | `KeyMode` / `NostrAuthService` | server-managed vs external key | P1-first |
| auth.rs:98-116 | `generate_keypair` | encrypt secret at rest via crypto | P1-first |
| auth.rs:118-149 | `provision_auth_event` | NIP-42 AUTH event signing | P1-first |
| auth.rs:151-195 | `sign_event_for_member` | **server-side signing** with stored secret | P1-first |

### crates/chat/src/nostr/events.rs (774)
| file:line | symbol | why | prio |
|---|---|---|---|
| events.rs:34-55 | `build_auth_event` | challenge/relay tags + kind | P1-first |
| events.rs:57-79 | `build_group_message` | kind 9 group chat | P1-second |
| events.rs:81-117 | `build_add_user` / `build_remove_user` | roster admin events | P1-first |
| events.rs:119-155 | `build_group_metadata` | group metadata | P1-second |
| events.rs:157-172 | `build_delete_event` | kind 5 deletions | P1-second |
| events.rs:197-230 | `build_profile_metadata` | kind 0 | P1-second |
| events.rs:232-278 | `build_community_definition` | kind 34550 | P1-second |
| events.rs:280-349 | `build_reaction` / `build_community_post` | public social surface | P1-second |
| events.rs:351-372 | `build_custom_event` | generic signer escape hatch | P1-first |
| events.rs:374-387 | `keys_from_hex` / `generate_keys` | secret material handling | P1-first |

### crates/chat/src/nostr/relay.rs (354) + groups.rs (170)
| file:line | symbol | why | prio |
|---|---|---|---|
| relay.rs:80-132 | `RelayClient::connect` | WS + message pump | P1-second |
| relay.rs:150-188 | `publish_event` / `send_auth` / `subscribe` | auth + publish path | P1-first |
| relay.rs:216-261 | `publish_event_oneshot` | fire-and-forget publish | P1-second |
| relay.rs:263-326 | `query_events_oneshot` | fetch filter surface | P1-second |
| groups.rs:44-151 | `GroupManager` CRUD | create/add/remove/update/delete | P1-first |

### crates/chat/src/community.rs + provisioning.rs
| file:line | symbol | why | prio |
|---|---|---|---|
| community.rs:77-150 | `build_lfg_event` / match / announcement | public signed community events | P1-second |
| provisioning.rs:41-106 | `provision_team_channels` | auto channel create on team | P1-second |
| provisioning.rs:108+ | `sync_team_roster` | group membership sync | P1-first |

### crates/relay-policy (policy.rs + main.rs)
| file:line | symbol | why | prio |
|---|---|---|---|
| policy.rs:13-104 | `PolicyConfig` defaults | allowed kinds, rate window, group gate | P1-first |
| policy.rs:106-125 | `PolicyEngine::new` | engine state | P1-first |
| policy.rs:127-138 | `update_allowlist` / `update_group_members` | dynamic ACL | P1-first |
| policy.rs:145-191 | `evaluate` | **NIP-42/29 policy core** accept/reject | P1-first |
| policy.rs:192-201 | `prune_rate_buckets` | rate-limit memory | P1-second |
| main.rs:95-116 | `load_pubkey_allowlist` | DB → allowlist refresh | P1-first |
| main.rs:117+ | `main` relay loop | wire policy to real events | P1-first |

### crates/db nostr_keys + dms
| file:line | symbol | why | prio |
|---|---|---|---|
| nostr_keys.rs:23-52 | `generate_encrypted_keypair` | raw 32B secret + AAD pubkey | P1-first |
| nostr_keys.rs:54-89 | `decrypt_nostr_secret_key` | decrypt + encoding + derive check | P1-first |
| nostr_keys.rs:91-120 | `verify_secret_derives_pubkey` | mismatch fail-closed | P1-first |
| nostr_keys.rs:122-166 | `provision_nostr_keypair` / `get_nostr_secret_key` | member store path | P1-first |
| nostr_keys.rs:168-185 | `set_external_nostr_key` | external mode (no secret stored?) | P1-first |
| dms.rs:43-68 | `seal_dm_content` | AES-GCM at rest + AAD | P1-first |
| dms.rs:70-97 | `open_dm_content` | **PRODUCTION rejects plaintext** | P1-first |
| dms.rs:121-170 | `insert_dm_message` | idempotent insert | P1-first |
| dms.rs:172-345 | list/inbox/conversations/markers | authz must be at HTTP layer | P1-first |

### crates/site-server/src/routes/nostr.rs (1839)
| file:line | symbol | why | prio |
|---|---|---|---|
| nostr.rs:51-119 | `nostr_json` | NIP-05 /.well-known | P1-second |
| nostr.rs:198-247 | `nostr_challenge` | login challenge issuance | P1-first |
| nostr.rs:249-369 | `nostr_verify` | **sig verify + session mint** (rate-limit none?) | P1-first |
| nostr.rs:371-425 | `nostr_unlink` | unlink key | P1-second |
| nostr.rs:427-499 | `nostr_export_backup` | **secret export** — authz + audit | P1-first |
| nostr.rs:501-595 | `nostr_import_key` | **secret import** — validation | P1-first |
| nostr.rs:597-722 | `nostr_community` | community admin surface | P1-second |
| nostr.rs:734-862 | `nostr_react` | reactions | P1-second |
| nostr.rs:864-993 | `nostr_feed` | feed query filters | P1-second |
| nostr.rs:995-1125 | `nostr_post` | server-signed post | P1-first |
| nostr.rs:1127-1190 | `nostr_health` | relay health | skip |
| nostr.rs:1302-1362 | `require_server_managed_dm_caller` | DM authz gate | P1-first |
| nostr.rs:1364-1376 | `require_encryption_service` | fail without crypto | P1-first |
| nostr.rs:1394-1518 | `dm_send` | send path + seal | P1-first |
| nostr.rs:1520-1650 | `dm_sync` | sync / high-water | P1-first |
| nostr.rs:1652-1808 | `dm_inbox` / `conversations` / `thread` / `stream` | read paths | P1-first |
| nostr.rs:1810+ | `dm_mark_read` | markers | P1-second |

### crates/server/src/routes/chat.rs
| file:line | symbol | why | prio |
|---|---|---|---|
| chat.rs:21-169 | `provision_auth_token` | desktop/chat auth token | P1-first |
| chat.rs:194-390 | `send_encrypted` | encrypt + publish path | P1-first |
| chat.rs:392-460 | `decrypt_message` | server decrypt API | P1-first |
| chat.rs:462+ | `load_member_with_secret` | loads encrypted secret for sign | P1-first |

### NOSTR review focus
1. Signature verification on every inbound event (especially `nostr_verify`, relay-policy `evaluate`, gift-wrap unwrap).
2. Server-side signing paths never leak raw secret (export/import/backup/logs).
3. Key-at-rest: AAD bound to pubkey; external vs server-managed mode transitions.
4. DM seal/open production plaintext policy + caller authz on every list endpoint.
5. NIP-42 AUTH + NIP-29 group membership enforcement defaults.
6. Secret-in-logs sweep across chat/dm/nostr routes (usernames/pubkeys OK; nsec/hex secrets not).
7. MEMBER_SAFE cross-check: no DM/chat response returns `nostr_secret` / full member row.

---

## LANE: DB

Scope: remainder of `crates/db` after ACCT carve-out.
**EXCLUDED (ACCT owns):** `queries/members.rs`, `queries/applications.rs`,
`membership_policy.rs` (if present), member role/ban CAS paths.
**EXCLUDED from deep P1 (NOSTR owns content):** `queries/nostr_keys.rs`,
`queries/dms.rs` — listed above under NOSTR; DB lane only notes shared
client/rewrap interactions.

### crates/db/src/client.rs (436)
| file:line | symbol | why | prio |
|---|---|---|---|
| client.rs:60-76 | `check_credentials` | empty user/pass guard | P1-second |
| client.rs:78-85 | `assert_safe_sql_ident` | **injection guard for dynamic idents** | P1-first |
| client.rs:87-90 | `escape_surreal_string` | string escape helper | P1-first |
| client.rs:92-113 | `assert_remote_production_policy` | remote DB prod policy | P1-first |
| client.rs:200-222 | `load_crypto` | ENCRYPTION_KEY optional/required | P1-first |
| client.rs:224-285 | `connect` / `connect_scoped` | auth modes | P1-first |
| client.rs:287-322 | `ensure_database_app_user` | bootstrap app role | P1-second |
| client.rs:324-400 | `bootstrap_from_env` / `connect_from_env` | env wiring | P1-first |
| client.rs:403+ | `connect_memory` | tests | skip |

### crates/db/src/rewrap.rs (434)
| file:line | symbol | why | prio |
|---|---|---|---|
| rewrap.rs:50-73 | `rewrap_all_encrypted_fields` | key rotation entry | P1-first |
| rewrap.rs:75-150 | `rewrap_users` | OAuth token blobs | P1-first |
| rewrap.rs:152-238 | `rewrap_members` | member encrypted fields | P1-first |
| rewrap.rs:240+ | `rewrap_dm_messages` | DM ciphertext migration | P1-first |

### crates/db/src/migrations.rs (711) + types.rs (1213)
| area | why | prio |
|---|---|---|
| migrations.rs full | SCHEMAFULL / indexes / destructive steps | P1-first |
| types.rs RecordId/Datetime maps | datetime type correctness; Thing id strings | P1-first |
| types.rs encrypted blob types | serde of secrets at rest | P1-first |
| types.rs ~1209 `conversation_key` format | DM conversation id stability | P1-second |

### crates/db/src/queries/sessions.rs (211)
| file:line | symbol | why | prio |
|---|---|---|---|
| sessions.rs:33-92 | `create_session` | hash store; expiry | P1-first |
| sessions.rs:94-141 | `get_session` / `get_session_user` | lookup by raw token hash | P1-first |
| sessions.rs:143-184 | `delete_session` / `delete_sessions_for_user` | revocation bulk | P1-first |
| sessions.rs:186+ | `cleanup_expired_sessions` | GC | P1-second |

### crates/db/src/queries/users.rs (381) — non-ACCT bits
(Local-reg/password policy detail is AUTH+ACCT; DB focuses store integrity.)
| file:line | symbol | why | prio |
|---|---|---|---|
| users.rs:29-65 | `upsert_user_from_oauth` | encrypt provider tokens | P1-first |
| users.rs:108-132 | `require_oauth_encryption` | prod crypto gate | P1-first |
| users.rs:134-218 | `create_user` / `update_user` | encrypted field write | P1-first |
| users.rs:250-334 | `create_local_user` / get/set password hash | hash storage only | P1-second |
| users.rs:336+ | `db_user_to_user` | decrypt projection | P1-first |

### crates/db/src/queries/daemon_tokens.rs (118)
| file:line | symbol | why | prio |
|---|---|---|---|
| daemon_tokens.rs:39-63 | `create_daemon_token` | raw token → store hash? | P1-first |
| daemon_tokens.rs:65-85 | `validate_daemon_token` | bearer validate | P1-first |
| daemon_tokens.rs:100+ | `revoke_daemon_token` | revoke scoped | P1-second |

### High-churn / large query modules
| file (LOC) | symbols / focus | prio |
|---|---|---|
| tournaments.rs (1637) | create/update match/round; `thing_to_id`; score set paths — CAS? | P1-first |
| forum.rs (821) | `ensure_forum_hierarchy` seed; thread/reply CRUD | P1-second |
| strategies.rs (576) | **dynamic `where_clause` SQL via `format!`** (:300-324) — injection | P1-first |
| personal_stats.rs (663) | match upsert; member join; tracker write path | P1-second |
| matches.rs (405) | match result store | P1-second |
| channels.rs (229) | team channel rows (pairs with provisioning) | P1-second |
| settings.rs (290) | site settings dual-write shell skin | P1-second |
| polls.rs (243) | vote integrity (double-vote) | P1-first |
| wiki.rs (261) | topic id in errors only; update CAS | P1-second |
| audit_log.rs (98) | append-only? tamper surface | P1-first |
| teams/events/scrims/rsvps/attendance/articles/announcements/seasons/game_accounts/games/moderation | standard CRUD + bind params | P1-second |

### DB review focus
1. **Bind-params-only rule:** every user-influenced value is `$bind`, never
   string-interpolated into SurrealQL. Exception audit: `strategies.rs`
   where_clause builder, any `format!("UPDATE … {sets}")`, member safe cols
   (ACCT but pattern).
2. `assert_safe_sql_ident` coverage — only idents, not values.
3. Datetime: `DateTime<Utc>` vs Surreal Datetime round-trip; no stringly times
   in comparisons that skew.
4. Projections: never `SELECT *` on tables with secrets (users/members/dms).
5. CAS/atomicity: tournament score, poll votes, settings updates — race windows.
6. Migrations: forward-only; no drop of encrypted columns without rewrap.
7. Rewrap completeness vs all EncryptedBlob fields.

---

## P0 inventory notes (memtrace / scale)

| area | approx LOC | note |
|---|---|---|
| site-server nostr.rs | 1839 | largest single NOSTR surface |
| db tournaments.rs | 1637 | largest DB query module |
| db types.rs | 1213 | shared types / serde |
| chat events.rs | 774 | event builders |
| db migrations.rs | 711 | schema |
| db members.rs | 675 | ACCT (cross-check MEMBER_SAFE only) |

## Next

- P1 wave-1 (priority core): grok NOSTR in parallel with claude AUTH/ACCT/ADMIN.
- P1 wave-2: grok DB with claude FRONT/QUAL.
- Findings IDs: `DR1-NOSTR-NNN`, `DR1-DB-NNN` on `fleet::dr1-nostr` /
  `fleet::dr1-db` (and mirror to git if ydoc initiative threads empty).
- Durable copy of this file is SoT if ydoc wipes again.
