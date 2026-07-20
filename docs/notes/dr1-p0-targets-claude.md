# DR-1 P0 Recon — Owned Target Lists

Repo: ~/github/scuffed-crew · Author: claude (P0 recon) · Date: 2026-07-19
Partition is binding (grok A2). Priority core = AUTH/ACCT/ADMIN (exhaustive).
FRONT/QUAL lighter. Priority tags: P1-first / P1-second / skip.

---

## LANE: AUTH  (`crates/auth` only; + extractor chain lives in site-server/extractors.rs — flagged, boundary-shared with ADMIN)

### crypto.rs (563)
| file:line | symbol | why it matters | prio |
|---|---|---|---|
| crypto.rs:23-32 | `hash_session_token` / `verify_session_token` | BLAKE3 keyless hash of session tokens; verify is constant-time. Confirm tokens are high-entropy (they are: 32B OsRng). | P1-second |
| crypto.rs:112-167 | `CryptoService::new` / `from_keyring` | AES-256-GCM keyring, rotation, version-collision guard. Core at-rest crypto. | P1-first |
| crypto.rs:179-223 | `from_env` | ENCRYPTION_KEY / _VERSION / _PREVIOUS parsing. **allow_legacy_empty_aad defaults TRUE unless CRYPTO_STRICT_AAD=1 or PRODUCTION truthy** — AAD-downgrade risk if PRODUCTION unset in a real deploy. | P1-first |
| crypto.rs:238-343 | `encrypt_bytes`/`decrypt_bytes`/`rewrap` | nonce=12B OsRng per op, AAD bound, Zeroizing plaintext, empty-AAD fallback path (313-326). Verify fallback can't be attacker-forced. | P1-first |
| crypto.rs:346-358 | `is_strict_production_crypto` | duplicate of env_flags::is_production_env logic (convention drift / QUAL dup). | P1-second |
| crypto.rs:360-380 | `parse_aes_key` / `hash_provider_id` | key len check + zeroize; provider-id deterministic BLAKE3 (unsalted → offline dictionary on provider_id if hash leaks). | P1-second |

### password.rs (50)
| password.rs:23-38 | `hash_password`/`verify_password` | Argon2id **default params** (no explicit m/t/p tuning); MIN_PASSWORD_LEN=12. No dummy-hash on missing user (enables timing-based username enumeration — see auth.rs local_login). | P1-first |

### nip49.rs (238)
| nip49.rs:44-133 | `encrypt`/`decrypt` | scrypt+XChaCha20; log_n bounded 16..=20 (DoS guard, tested). Zeroize on all paths. Clean. | P1-second |

### server/session.rs (180)  — cookie construction + CSRF
| session.rs:15-42 | `build_session_cookie` / `build_csrf_cookie` | `is_secure = SECURE_COOKIES set OR is_production_env OR !debug_assertions`; HttpOnly+Lax. Secure can be FALSE in a release build that isn't "production" and lacks SECURE_COOKIES. | P1-first |
| session.rs:45-84 | `validate_csrf_state` | plain-equality cookie-vs-param compare; None cases → 400. No HMAC/binding of state to session. Verify replay/fixation posture. | P1-first |
| session.rs:87-93 | `generate_session_token` | 32B OsRng URL-safe base64. Good. | skip |
| session.rs:99-124 | `clear_session_cookie` / `clear_csrf_cookie` | the login-breaking add+remove-same-name bug is fixed here (regression-tested 146-179). | P1-second |

### server/oauth.rs (172)
| oauth.rs:102-119 | `create_client` | **panics** on invalid redirect/token URL (expect/panic) — DoS if config malformed at runtime. | P1-second |
| oauth.rs:122-145 | `get_auth_url` / `exchange_code` | **No PKCE**; CSRF only via cookie state. Google/Discord token exchange. | P1-first |
| oauth.rs:148-172 | `get_user_info` | Bearer access_token → userinfo endpoint. **Google path does NOT verify an OIDC id_token/nonce** (uses /oauth2/v2/userinfo). Acceptable-ish; note. | P1-second |

### server/extractor.rs (100)
| extractor.rs:53-100 | `AuthUser::from_request_parts` | Bearer-first then session cookie. DB error → 500, none → 401. Generic base of the whole chain. | P1-first |

### server/{discord,google,matrix}.rs
| discord.rs / google.rs | provider mappings | provider_id from `id`; username fallbacks. | P1-second |
| matrix.rs:37-45 | `MatrixProvider::config` | **STUB with example.com URLs / TODO** — if ever registered it silently points at example.com. Confirm not wired. | P1-second |

### env_flags.rs / lib.rs / types.rs
| env_flags.rs:8-22 | `is_production_env` | central prod gate; truthy classification. | P1-first |
| types.rs:89-111 | `SessionConfig` (default 168h, csrf 10min) | session lifetime = 1 week; no idle timeout / rotation. | P1-second |
| types.rs:44-58 | `User::new` | uuid v4 id; created_at chrono. | skip |

### Extractor chain (site-server/src/extractors.rs — shared boundary, review under AUTH per lane scope)
| extractors.rs:35-88 | `OrgMember` | member lookup + is_active + suspended check, fail-closed on DB err. Single get_member_auth_by_user. | P1-first |
| extractors.rs:104-152 | `OfficerUser` / `AdminUser` | role gate via is_at_least / `!= Admin`. | P1-first |
| extractors.rs:90-102 | `OptionalOrgMember` | swallows ALL errors → None (a suspended/inactive user is indistinguishable from anon for optional routes). | P1-second |
| extractors.rs:159-255 | `DaemonUser` | Bearer daemon-token → member; active+not-suspended check. | P1-second |

**AUTH review focus:**
- AAD legacy-empty fallback default-on + Secure-cookie default both hinge on PRODUCTION/build flags — enumerate every prod-gate and confirm a real VPS deploy trips them (crypto.rs:213-215, session.rs:16-19).
- Brute-force/rate-limit posture: **NONE found** on local_login / nostr_verify / setup — grok checklist item; confirm absence.
- Username-enumeration timing oracle: verify_password only runs when user row exists (no dummy hash).
- OAuth: no PKCE, no id_token verification, CSRF = plain cookie compare; assess against provider threat model.
- Secret-in-logs sweep: none found (tracing logs usernames/pubkeys only, secrets Zeroized) — confirm across chat/dm too (NOSTR lane owns that).

---

## LANE: ACCT  (db membership_policy-adjacent + applications/members queries; site-server applications/members-role/ban routes; membership_policy.rs)

### membership_policy.rs (413) — pure policy, unit-tested
| line | symbol | why | prio |
|---|---|---|---|
| 40-55 | `is_valid_application_transition` | state machine edges (Pending/Trial → Accepted/Rejected/Withdrawn). No re-open. | P1-first |
| 71-93 | `role_on_application_accept` / ensures/deactivates helpers | **direct Pending→Accepted yields Recruit, Trial→Accepted yields Member** — asymmetry, confirm intended. | P1-first |
| 103-127 | `can_moderate` | self-mod block; officer may only touch member/recruit. | P1-first |
| 146-174 | `can_change_role` | last-actionable-admin demote guard; ignores actor/target id (self-demote covered by count). | P1-first |
| 178-221 | `can_set_is_active` | self-flip block + last-admin deactivate guard. | P1-first |
| 233-253 | `can_suspend_or_ban_admin` | last-admin ban/suspend guard; already-suspended admin excluded from count. | P1-first |
| 130-135,224-226 | `moderation_revokes_sessions` / `deactivation_revokes_sessions` | drive session revocation side effects. | P1-second |

### routes/applications.rs (459)
| 53-115 | `submit_application` | active-member block + open-app block; **concurrent double-submit** handled by count_open_applications>1 rollback+409 (94-105). Race window is post-insert compensate, not CAS. | P1-first |
| 149-193 | `withdraw_my_application` | self-withdraw gate; actor_id = member id else user id. | P1-second |
| 202-239 | `update_application` | officer transition; validity check then apply. | P1-first |
| 242-331 | `apply_application_transition` | **side-effects-before-CAS ordering** (ensure_member for trial/accept; deactivate recruit + revoke sessions for reject/withdraw). Central correctness path. | P1-first |
| 334-435 | `ensure_member_for_application` | create/reactivate/promote member; won't demote officer+; welcome notify. | P1-first |

### routes/members.rs (858)
| 73-79,131-145 | `double_option` / `normalize_optional_handle` | omit-vs-null semantics; social-handle validation (URL reject, charset, ≤32). | P1-second |
| 108-129 | `normalize_social_handle` | input validation. | P1-second |
| 148-372 | `update_member` | self-or-officer; **nostr_pubkey set rejected here** (must go via challenge/verify, 192-199); is_active last-admin guard + compensate (304-340); deactivate → revoke sessions + audit. Large god-ish handler (228 lines). | P1-first |
| 376-449 | `publish_profile_metadata` | fire-and-forget; decrypts+Zeroizes server nostr secret. | P1-second |
| 457-575 | `change_role` | admin-only; count_actionable_admins + suspend check → can_change_role → **change_member_role has NO CAS**; compensate-after via assert_has_actionable_admin (543-562). TOCTOU window between count and write. This is the "role-change 400" family. | P1-first |
| 615-693 / 696-730 | `upsert_game_account` / `delete_game_account` | self-or-officer; validation. | P1-second |
| 741-858 | `admin_reset_local_password` | admin-only local-account recovery; MIN_PASSWORD_LEN; **revokes all sessions** after reset (837-845); Local-provider-only. | P1-first |

### routes/moderation.rs (303)
| 38-226 | `create_moderation_action` | officer+; can_moderate + can_suspend_or_ban_admin; **ban deactivates member first + compensate**; suspension re-checks actionable admin after row exists (186-199); revoke sessions on ban/suspend (201-210). Complex compensation chain. | P1-first |
| 278-303 | `lift_moderation_action` | admin-only; lift does NOT reactivate (per policy doc). | P1-second |

### db/queries/applications.rs (283)
| 178-265 | `update_application_status` | **atomic CAS** `UPDATE … WHERE status=$expected RETURN AFTER`; distinguishes NotFound vs Conflict. Trial sets 14d trial window. Bind-params only. | P1-first |
| 63-114 | `submit_application` / `count_open_applications` | insert + open-count (GROUP ALL). | P1-second |

### db/queries/members.rs (675)
| 14 | `MEMBER_SAFE_COLS` | projection omitting nostr_secret_key_encrypted; used by list/get_safe/auth paths. | P1-first |
| 240-293 | `get_member_safe` / `get_member_auth_by_user` | safe projections feeding extractors. | P1-first |
| 532-548 | `change_member_role` | **plain UPDATE SET org_role, no CAS/expected guard** — pairs with route-level compensate. | P1-first |
| 551-623 | `count_active_admins` / `count_actionable_admins` / `assert_has_actionable_admin` | actionable = active admin minus suspended/ban (two queries, HashSet subtract). Non-atomic vs role writes. | P1-first |
| 331-455 | `update_member` | 12-arg field update (too_many_arguments allow); is_active toggle. | P1-second |

### db/queries/sessions.rs (211)
| 33-91 | `create_session` | hashes token; drops expired; **MAX_SESSIONS_PER_USER=10** eviction loop (oldest-first). | P1-first |
| 118-140 | `get_session_user` | single-timeout session+user resolve. | P1-first |
| 157-183 | `delete_sessions_for_user` | bulk revoke (ban/deactivate/reset). | P1-first |

### db/queries/users.rs (381)
| 108-132 | `require_oauth_encryption` | **ENCRYPTION_KEY mandatory for OAuth when PRODUCTION or remote SurrealDB**; ALLOW_PLAINTEXT_PROVIDER_IDS escape hatch (non-prod). | P1-first |
| 134-215 | `create_user` / `update_user` | encrypted vs plaintext provider_id branches; provider_id_hash for lookup; never clobbers password_hash. | P1-first |
| 245-335 | `normalize_local_username` / `create_local_user` / `get_local_user_by_username` / `set_local_password_hash` | local-account CRUD; uniqueness re-check. | P1-first |

**ACCT review focus:**
- Last-admin invariant is enforced by **count-then-write-then-assert-then-compensate**, never a single atomic guard — hunt the TOCTOU windows (change_member_role, is_active, ban, suspend) under concurrent admin actions.
- Pending→Accepted = Recruit vs Trial→Accepted = Member asymmetry: confirm intended, not a role-grant bug.
- Session-revocation coverage: reset-password ✓, deactivate ✓, ban/suspend ✓, reject/withdraw recruit ✓ — verify role-change/demote does NOT revoke (should it, for privilege drop?).
- Application submit double-submit is compensate-after-insert, not CAS — check the race can't leave 2 open apps or an orphan member.
- Bind-params-only rule holds in every query read (no interpolation).

---

## LANE: ADMIN  (all OTHER site-server admin REST; per-route authz + audit coverage)

Route→handler→extractor→audit map (from lib.rs registration + handler signatures). audit✓ = fires `audit()`.

| route file | handler | extractor | mutation | audit |
|---|---|---|---|---|
| settings.rs:118 | `update_settings` | AdminUser | org/brand/homepage settings | ✓(1) |
| settings.rs:91 | `get_settings` | (public-ish) | read | – |
| games.rs:64/102 | `create_game`/`update_game` | AdminUser | catalog | ✓(2) |
| teams.rs:77 | `create_team` | **AdminUser** | team | ✓ |
| teams.rs:123 | `update_team` | **OfficerUser** | team | ✓ — **authz asymmetry create=admin vs update=officer** |
| tournaments.rs (737) | create/update/transition/generate_bracket/participants/report_match/next_round | OfficerUser | many | ✓(9) |
| articles.rs (352) | create/update/publish/unpublish | OfficerUser; `delete_article`=AdminUser | CMS | ✓(5) |
| announcements.rs | create/update/delete | OfficerUser | – | ✓(3) |
| events.rs | create/update/delete | OfficerUser | – | ✓(3) |
| roster.rs | add/update/remove | OfficerUser | team roster | ✓(3) |
| matches.rs | record/update | OfficerUser; get=OrgMember | – | ✓(2) |
| scrims.rs | create/update_status | OrgMember(!) | member-created scrims | ✓(2) |
| polls.rs | create/deactivate=Officer; vote/unvote=member | mixed | – | ✓(2) |
| wiki.rs | create/update=member; delete=Officer | mixed | wiki | ✓(3) |
| forum.rs (811) | category/board=Officer; thread/reply=member; pin/lock=Officer | mixed | forum | ✓(2) — **only 2 audits for ~9 mutations** |
| stats.rs (441) | upload=DaemonUser; daemon-token create/list/revoke=member; member_settings=member | mixed | tokens/settings | ✓(3) |
| leaderboards.rs | admin_create_season/admin_list_seasons | AdminUser | seasons | – **create_season NOT audited** |
| integrations.rs:19 | `test_discord_webhook` | AdminUser | side-effecting test | – |
| audit_log.rs:29 | `list_audit_log` | AdminUser | read | – |
| **uploads.rs:23/83** | `upload_avatar`(OrgMember)/`upload_image`(Officer) | mixed | **file write to disk** | **✗ NOT audited** |
| dev.rs:19 | `dev_login` | none | sets session cookie | – (dev only) |

Detailed targets:
| file:line | symbol | why | prio |
|---|---|---|---|
| uploads.rs (site-server/src/uploads.rs:49-101) | `sniff_image_ext`/`save_upload` | **magic-byte sniff (good)**, but NO image re-encode / dimension cap → decompression-bomb / pixel-flood risk (GIF/WebP/PNG); uuid filename (good, no path traversal). | P1-first |
| routes/uploads.rs:23-140 | `upload_avatar`/`upload_image` | no audit; single-field multipart; size caps 2MB/5MB. Any OrgMember can write avatars. | P1-second |
| routes/dev.rs:19-65 | `dev_login` | **cookie built without secure/same_site/max_age** (line 56-61); gated only on `SURREALDB_URL` unset AND route only registered when dev_mode (lib.rs:79-89). Confirm double-gate holds. | P1-first |
| routes/settings.rs:118 | `update_settings` | 16-arg passthrough to db.update_settings; validate brand color / homepage JSON injection surface. | P1-second |
| routes/leaderboards.rs:189-263 | `admin_create_season` | admin mutation, **unaudited**. | P1-second |
| routes/teams.rs:77 vs 123 | create vs update extractor mismatch | privilege inconsistency. | P1-first |
| routes/integrations.rs:19 | `test_discord_webhook` | admin-triggered outbound request (SSRF? fixed URL from settings). | P1-second |
| routes/audit_log.rs:60 | `audit()` helper | fire-and-forget, logs error but never fails request (per convention). Confirm no audit-loss on the mutations that matter. | P1-second |
| main.rs:45-60,459-482 | dev-mode branch / seed | in-memory DB + seed only when SURREALDB_URL unset; PRODUCTION refuses root (per CLAUDE.md). | P1-second |

**ADMIN review focus:**
- Audit-log coverage gaps: **uploads (avatar/image) and admin_create_season are unaudited**; forum has 2 audits for ~9 officer/member mutations — enumerate every mutating route vs audit() and list the holes.
- Authz consistency: teams create=Admin/update=Officer mismatch; scrims mutate at OrgMember; confirm each extractor matches the policy doc's intended tier.
- Upload hardening: no re-encode/dimension limits → decompression bomb; content sniffed but not transcoded.
- dev_login must be un-reachable in prod (route-registration gate + SURREALDB_URL gate + cookie flags).
- 403/400/409 semantics: spot-check a sample of routes return policy-correct codes (PolicyDenial.status()).

---

## LANE: FRONT  (crates/app admin+account UI, crates/api-client) — lighter

### api-client (525)
| lib.rs:57-122 | `ApiClient` fetch/post/put/patch/delete + get_me/logout | central HTTP surface; error mapping in ClientError. | P1-second |
| native_impl.rs (147) / web_impl.rs (146) | platform send impls | cookie vs bearer handling; confirm web relies on same-site cookie. | P1-second |

### Account/auth pages
| pages/login.rs (576) | `Login` | local + OAuth + nostr login UI; 321-line component. | P1-second |
| pages/setup.rs (285) | first-boot admin | password rules mirrored client-side. | P1-second |
| pages/apply.rs (294) | application submit | – | P1-second |
| pages/identity.rs (604) | `IdentitySettings` | nostr key mgmt UI (challenge/verify/import/export). | P1-second |
| pages/profile_settings.rs (444) | profile edit | double-option / clear semantics client side. | P1-second |
| pages/member_profile.rs (456) / members.rs | member views | – | skip |

### Admin pages (crates/app/src/pages/admin/)
| admin/members.rs (957) | member admin | role change / activate / moderation UI; refresh-counter pattern. | P1-second |
| admin/settings.rs (927) | `AdminSettings` (748-line component) | biggest admin component; brand/homepage. | P1-second |
| admin/tournaments.rs (1032) | tournament admin | largest admin page. | P1-second |
| admin/moderation.rs (284) | moderation UI | – | P1-second |
| admin/{articles,forum,games,teams,schedule,announcements,matches,relay,audit_log,dashboard}.rs | CRUD pages | error-handling + toast + refresh-counter consistency. | P1-second |
| layouts/admin.rs (316) | admin shell / auth guard | client-side authz assumption (server is source of truth). | P1-first |

**FRONT review focus:**
- Client-side authz is cosmetic — confirm no page assumes hidden = protected (server extractors are the gate); audit layouts/admin.rs guard.
- Error-handling/toast/refresh-counter consistency across admin CRUD (176 refresh/toast hits in admin/) — convention drift is the main QUAL angle here.
- Nostr key material in identity.rs: ensure secrets aren't logged/persisted in browser state.
- api-client error mapping: does a 401/403 surface a re-login vs a silent failure.

---

## LANE: QUAL  (metrics only — whole workspace, stat-tracker/map-* INCLUDED here)

### Files > 600 lines (source, excl. tests)
```
2181  stat-tracker/src/main.rs
1839  site-server/src/routes/nostr.rs          (NOSTR lane owns content)
1637  db/src/queries/tournaments.rs
1565  app/src/pages/strategy/editor.rs
1452  app/src/components/strategy/map_canvas.rs
1307  stat-tracker/src/storage/mod.rs
1213  db/src/types.rs
1080  app/src/pages/stats.rs
1032  app/src/pages/admin/tournaments.rs
1018  app/src/pages/home/css.rs
 957  app/src/pages/admin/members.rs
 934  types/src/strategy.rs
 927  app/src/pages/admin/settings.rs
 886  site-server/src/routes/auth.rs
 858  site-server/src/routes/members.rs
 857  stat-tracker/src/parse.rs
 835  stat-tracker/src/ocr/preprocess.rs
 821  db/src/queries/forum.rs
 811  site-server/src/routes/forum.rs
 803  stat-tracker/src/gui/style.rs
 786  stat-tracker/src/ocr/mod.rs
 774  chat/src/nostr/events.rs
 737  site-server/src/routes/tournaments.rs
 711  db/src/migrations.rs
 694  types/src/org/homepage.rs
 691  app/src/state/editor.rs
 684  app/src/pages/scrims.rs
 682  app/src/pages/strategy/patch_notes.rs
 675  db/src/queries/members.rs · app/src/layouts/public.rs
 663  db/src/queries/personal_stats.rs
 640  app/src/state/nostr.rs
 625  site-server/src/routes/public.rs
 612  stat-tracker/src/detect/hero_portrait.rs
 604  app/src/pages/identity.rs
```

### Highest cognitive-complexity functions (memtrace, AST)
| cc | file | fn | lines |
|---|---|---|---|
| 53 (crit) | stat-tracker/src/main.rs:1247 | `handle_capture` | 459 |
| 39 | stat-tracker/src/stats.rs:129 | `compute_stats` | 200 |
| 32 | stat-tracker/src/gui/stats.rs:111 | `compute_stats` | 172 |
| 31 | stat-tracker/src/parse.rs:276 | `find_hero` | 67 |
| 26/cx31 | stat-tracker/src/main.rs:99 | `main` | 226 |
| 24 | stat-tracker/src/detect/match_start.rs:131 | `detect_hero_select` | 88 |

### Longest functions / god-components (line-gap heuristic; Dioxus rsx! inflates)
| ~lines | file:line | symbol |
|---|---|---|
| 905 | app/src/pages/strategy/editor.rs:420 | `EditorLayout` |
| 748 | app/src/pages/admin/settings.rs:13 | `AdminSettings` |
| 746 | app/src/components/strategy/map_canvas.rs:186 | `MapCanvas` |
| 561 | stat-tracker/src/main.rs:686 | `run_loop` |
| 465 | stat-tracker/src/main.rs:1247 | `handle_capture` |
| 321 | app/src/pages/login.rs:163 | `Login` |
| 228 | site-server/src/routes/members.rs:148 | `update_member` |
| 230 | db/src/queries/tournaments.rs:1065 | `generate_double_elim_bracket` |
| 198 | site-server/src/routes/moderation.rs:38 | `create_moderation_action` |
| 184 | site-server/src/routes/auth.rs:454 | `setup` |

### Near-duplicate implementations (candidates)
- **`compute_stats`** exists in `stat-tracker/src/stats.rs:117` AND `stat-tracker/src/gui/stats.rs:111` (cc 39 vs 32, ~200/172 lines) — confirmed divergent duplication.
- **`find_hero` / `fuzzy_match_hero` / `fuzzy_match_map`** appear as many near-identical AST nodes in `stat-tracker/src/parse.rs` (memtrace lists 4-6 copies each ~34-47 lines) — likely test-fixture duplication or cfg variants; verify.
- **prod-gate logic** duplicated: `crypto.rs:346 is_strict_production_crypto` vs `env_flags.rs:8 is_production_env` (same match arms).
- **`double_option` deserializer + omit/null handling** repeated across members.rs / game-account / profile routes.
- **internal_err/bad_request/conflict** error-helper trio re-declared per route file (applications.rs, moderation.rs, others) — convention, but copy-paste.

**QUAL review focus (P4, not P1):**
- stat-tracker `main.rs` is the complexity sink (2181 lines, handle_capture cc53, run_loop 561, main 226) — top refactor target but EXCLUDED from P1 security waves (grok A3).
- Frontend god-components (EditorLayout 905, AdminSettings 748, MapCanvas 746) — behavior-preserving split candidates for morning (A4 blast-radius rule).
- compute_stats duplication is a real dedup win with tiny blast radius.
- Convention drift: per-file error-helper trios + duplicated prod-gate + double_option boilerplate.
