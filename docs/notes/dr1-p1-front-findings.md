# DR-1 P1 FRONT lane findings

Lane: FRONT (`crates/app` admin + account UI, `crates/api-client`). Reviewer: claude (Opus).
Scope reminder: client-side correctness/consistency only. Server authz is separately reviewed; here
the question is whether the UI MISLEADS about permissions, leaks admin-only data into the rendered
page/state, swallows errors, or leaves stale UI after mutations.

Severity counts: CRIT 0 · HIGH 0 · MED 3 · LOW 2 · NIT 1

---

## DR1-FRONT-001 | MED | crates/app/src/pages/admin/{audit_log,moderation,tournaments,articles,teams,applications,schedule,announcements,relay,matches,games}.rs (list render blocks) + hooks/api.rs:25-44,86-105 | CONFIRMED

Claim: `ApiResource` exposes an `error` signal that `use_api`/`use_api_list` set on every fetch
failure (api.rs:33, 94, 122, 152), but only `admin/members.rs` reads it. All 11 other admin list
pages match `None => rsx!{ p{ "Loading..." } }` and never inspect `.error`, so a failed initial
fetch is indistinguishable from an in-flight one.

Failure scenario: an officer opens `/admin/audit-log` (server returns 403, AdminUser-gated), or any
admin list hits a transient 500 / dropped connection → the page renders "Loading..." forever. No
toast, no error text, no Retry button. The user has no signal that anything failed.

Fix direction: standardize the list-render on the members.rs pattern (read `.error`, render the
message + a Retry button that bumps `.refresh`). Best done once as a shared helper/component so all
12 admin pages inherit it.

---

## DR1-FRONT-002 | MED | crates/app/src/routes.rs:76-107, crates/app/src/layouts/admin.rs:193,217-238,299-304 | CONFIRMED (no live data leak; authz-model mismatch + no defense-in-depth)

Claim: `AdminLayout` guards the ENTIRE admin panel with `is_officer_or_above()` only (admin.rs:217).
The four admin-only destinations (Moderation, Relay, Audit Log, Settings) are gated on the client
solely by `if is_admin { Link ... }` around the sidebar entries (admin.rs:299-304). The page
components themselves (`AdminSettings`, `AdminModeration`, `AdminRelay`, `AdminAuditLog`) contain NO
self-guard — an officer who types the URL directly renders the full component.

Two distinct consistency problems, both confirmed against the server extractor map:
- Under-exposure / mismatch: `list_moderation` + `create_moderation_action` are `OfficerUser`-gated
  server-side (moderation.rs:40,243), i.e. officers ARE authorized to moderate — yet the sidebar
  hides Moderation from officers behind `is_admin`. Client authz model contradicts the server.
- Link-hiding as the ONLY client gate: Audit Log (`AdminUser`, audit_log.rs:31) and Settings-save
  (`AdminUser`) are admin-only server-side. An officer reaching `/admin/audit-log` or `/admin/settings`
  by URL renders the page and either gets a silent perpetual "Loading..." (see FRONT-001, audit-log
  403) or a pre-filled org-settings form (get_settings is public) whose Save 403s.

No live admin-only DATA leak was found — every admin-only GET is server-gated. The issue is (a) the
UI misleads about who can do what and (b) there is zero client-side defense-in-depth: if any future
admin-only GET is added without an extractor, this pattern turns it straight into a leak.

Failure scenario: officer visits `/admin/settings`, sees the full settings form and every admin
control, edits, clicks Save, gets an opaque 403 toast; separately, officers never discover they can
use Moderation because the link is hidden from them despite server permission.

Fix direction: make the client authz model match the server — gate the sidebar Moderation link on
`is_officer_or_above()` (it is officer-permitted), keep Relay/AuditLog/Settings on `is_admin`, and add
an `is_admin` self-guard (Access Denied block, mirroring admin.rs:217-238) to the `AdminSettings`,
`AdminRelay`, and `AdminAuditLog` page components so direct-URL access renders a clean denial instead
of a broken/misleading page.

---

## DR1-FRONT-003 | MED | crates/app/src/pages/admin/members.rs:118-130, 253-261, 275-283 | CONFIRMED

Claim: three sub-modal loaders swallow fetch errors with `if let Ok(list) = ...fetch().await`:
the game-accounts `use_resource` loader (118-130), `open_mod_history` (253-261), and `open_stats`
(275-283). On error they leave the data `Vec::new()`/`None` and set loading=false, so the modal
renders the same empty state as a genuinely-empty result ("No moderation history.",
"No attendance data.", "No game accounts linked.").

Failure scenario: a transient 500 on `GET /api/members/{id}/moderation` shows the admin a clean
moderation record for a member who actually has history — a wrong moderation/role decision can follow
from data that silently failed to load.

Fix direction: capture the `Err` arm and surface it (toast or an inline "Failed to load — retry"
row), consistent with the toast-on-error convention used elsewhere in this same file.

---

## DR1-FRONT-004 | LOW | crates/app/src/hooks/api.rs:54-83 | CONFIRMED

Claim: `use_api_list` auto-follows at most `LIST_MAX_PAGES = 10` × `LIST_PAGE_LIMIT = 100` = 1000
rows, then stops with no indication that more exist. Used by the members table, applications list,
and the moderation member-picker.

Failure scenario: once the org exceeds 1000 members, the admin members table and the moderation
"select member" dropdown silently omit everyone past row 1000; those members become unmanageable and
un-moderatable from the UI with no visible cause.

Fix direction: surface a "showing first 1000" indicator when the cap is hit, or switch these lists to
explicit pagination like the audit log.

---

## DR1-FRONT-005 | LOW | crates/app/src/pages/admin/members.rs:399-491 | CONFIRMED

Claim: the avatar upload bypasses `ApiClient` and hand-rolls a `web_sys` fetch. On a non-ok response
it shows only `"Upload failed: HTTP {status}"` (line 481-484), discarding the server's
`{"error":"..."}` body that `ClientError`/`format_http_error` would otherwise surface. It also uses
`resp_val.unchecked_into::<Response>()` (line 444) rather than a checked cast.

Failure scenario: upload rejected for a specific reason (e.g. bad magic bytes, size) → the admin sees
a bare status code instead of the actionable server message that every other mutation in the app
surfaces; inconsistent error UX.

Fix direction: route multipart uploads through a shared api-client helper, or at minimum parse the
error body and reuse `format_http_error` so the message matches the rest of the app.

---

## DR1-FRONT-006 | NIT | crates/app/src/pages/identity.rs:266-287 | CONFIRMED

Claim: `on_import_key` clears `import_ncryptsec`/`import_password` after use (310-311), but
`on_export_backup` never clears `backup_password` from its signal after a successful export (the
ncryptsec result is intentionally shown). Minor secret-lifetime hygiene inconsistency in the same
file. (`native_impl.rs:5-10` `client()` uses `.unwrap_or_else(|_| Client::new())` — no panic, noted
as clean.)

Fix direction: clear `backup_password` after a successful export for parity with the import path.

---

## Notes / things checked that are clean (no finding)

- Mutation → refresh-counter consistency: role change, toggle-active, add/delete game-account, avatar,
  moderation create, moderation lift, application accept/reject all bump `.refresh` on success. No
  missing-refresh (stale-UI-after-mutation) bugs found in the admin pages reviewed.
- No optimistic-local-mutation race: mutations close the modal then refetch the list (no local list
  mutation that could desync); `on_toggle_confirm` etc. are safe.
- api-client web path uses `RequestMode/Credentials::SameOrigin` (web_impl.rs:10-11) → relies on the
  same-site session cookie, correct; native path uses bearer + 30s timeout. Error mapping is uniform
  (`ClientError` + `format_http_error` surfaces the server `error` field). No panics/unwraps on the
  network paths.
- identity.rs does not log or persist nostr secret material to browser state; ncryptsec is only held
  in a signal for display as designed.
- profile_settings.rs double-Option intent (plain `Option` → null = clear) is documented and correct.
