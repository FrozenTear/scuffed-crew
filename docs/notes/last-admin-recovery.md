# Last-admin lockout recovery (manual DB)

The last-admin hardening (DR1 ACCT-002/003/004) refuses any demote, deactivate,
suspend, or ban that would leave the org with **zero actionable admins**. That
protects a live org, but an org can still lock itself out by other means — the
last admin's account is lost, its credentials are compromised, or a bug flips
its `is_active`/`org_role`. When that happens the app has no in-band way back in
(every admin action needs an admin), so recovery is a **root-level DB edit**.

This is a break-glass procedure. Prefer restoring from a backup
(`scripts/backup.sh` / `scripts/restore.sh`) if you have a good one. Only edit
the DB by hand when you cannot.

## What "actionable admin" means

An **actionable admin** is a member that is simultaneously:

- `is_active = true`, and
- `org_role = 'admin'`, and
- **not** currently under an active suspension or ban.

This is the exact rule the server enforces
(`crates/db/src/queries/members.rs::count_actionable_admins`, lines 664–688):

```surql
SELECT count() FROM member
WHERE is_active = true AND org_role = 'admin'
AND meta::id(id) NOT IN (
    SELECT VALUE member_id FROM moderation_action
    WHERE is_active = true
    AND action_type IN ['suspension', 'ban']
    AND (expires_at IS NONE OR expires_at > time::now())
)
GROUP ALL
```

Note the two id shapes: `member.id` is a RecordId (`member:<key>`) while
`moderation_action.member_id` is the **bare string key** — hence `meta::id(id)`
on the member side. You will need the same distinction below.

The policy invariants live in
`crates/site-server/src/membership_policy.rs` (`can_change_role` line 146,
`can_set_is_active` line 178, `can_suspend_or_ban_admin` line 233); all guard
`actionable_admin_count <= 1`.

## Symptoms of lockout

- `/api/auth/setup-status` returns `{"needs_setup": false, ...}` (setup is done)
  but nobody can reach `/admin/`.
- Every role/activation change returns **409** with
  `"Would leave org without an actionable admin"`.
- The one admin account is deactivated, demoted, suspended/banned, or its
  credentials are lost.

## Recovery

All queries below are **plain SurrealQL run as root** — you type literal values,
so bind-parameter rules do not apply. We use `WHERE user_id = '…'` /
`display_name = '…'` filters (the `member_user_idx` on `user_id` is UNIQUE) so
the queries are copy-paste safe.

### 0. Back up first

```bash
./scripts/backup.sh   # snapshot before any manual write
```

### 1. Open a root SQL shell against the running DB

The Compose stack runs SurrealDB v3 as service `surrealdb` (image
`surrealdb/surrealdb:v3.0`, binary `/surreal`, bound `0.0.0.0:8000`, namespace
`scuffed_crew`, database `main` — see `compose.yml`). Root credentials are
`SURREALDB_USER` / `SURREALDB_PASSWORD` in `data/secrets.env`. From the deploy
directory:

```bash
set -a; . ./data/secrets.env; set +a
podman compose --env-file data/secrets.env exec surrealdb \
  /surreal sql \
    --endpoint ws://localhost:8000 \
    --username "$SURREALDB_USER" --password "$SURREALDB_PASSWORD" \
    --namespace scuffed_crew --database main --pretty
```

You **must** be root here, not the scoped `scuffed_app` EDITOR user — recovery
touches admin authority and (indirectly) member rows the app guards.

### 2. See the members and pick who to restore

```surql
SELECT meta::id(id) AS member_key, user_id, display_name, org_role, is_active
FROM member ORDER BY org_role, display_name;
```

Choose an **existing, trusted** member to become the recovered admin (ideally
the person who should hold it). Note their `user_id` and `member_key`.

### 3. Find any active suspension/ban blocking that member

```surql
SELECT meta::id(id) AS action_key, member_id, action_type, is_active, expires_at
FROM moderation_action
WHERE is_active = true
AND action_type IN ['suspension', 'ban']
AND (expires_at IS NONE OR expires_at > time::now());
```

`member_id` here is the **bare key** (matches `member_key` from step 2, no
`member:` prefix).

### 4. Re-activate and promote the chosen member

`org_role` must be one of `'admin' | 'officer' | 'member' | 'recruit'`
(`migrations.rs` line 53); `is_active` is a bool (line 65).

```surql
UPDATE member
SET is_active = true, org_role = 'admin'
WHERE user_id = '<user_id-from-step-2>';
```

### 5. Clear any suspension/ban that keeps them non-actionable

Only if step 3 returned rows for this member. Match on the **bare key**:

```surql
UPDATE moderation_action
SET is_active = false
WHERE member_id = '<member_key-from-step-2>'
AND is_active = true
AND action_type IN ['suspension', 'ban'];
```

(`moderation_action` has no append-only guard, so this UPDATE is allowed.)

### 6. Verify an actionable admin now exists

Re-run the canonical count from the top of this doc; it must return `>= 1`:

```surql
SELECT count() FROM member
WHERE is_active = true AND org_role = 'admin'
AND meta::id(id) NOT IN (
    SELECT VALUE member_id FROM moderation_action
    WHERE is_active = true
    AND action_type IN ['suspension', 'ban']
    AND (expires_at IS NONE OR expires_at > time::now())
)
GROUP ALL;
```

Then log in through the app and confirm `/admin/` loads.

## If the credentials (not the role) are the problem

If the admin's **member row is fine** but you only lost the password of a
**local** account, you usually do not need this procedure — use
`scripts/reset-local-admin.sh` (see `docs/deploy.md` → "Forgot admin password").
Use the DB path above when the account was demoted, deactivated, suspended,
banned, or otherwise stripped of admin authority.

## Safety notes

- **Promote a real, trusted, existing member.** Do not invent a user; a member
  row without a matching `user` login cannot sign in.
- **Manual DB writes bypass the audit log.** `audit_log` is written at the app
  layer, so a break-glass edit leaves no audit row. Note the change out-of-band
  (ticket/changelog) and, once back in the app, re-apply any *legitimate*
  moderation through the admin UI so it is recorded properly.
- If the member **row itself was deleted** (not just deactivated/demoted),
  restoring from a backup is safer than hand-crafting `member` + `user` rows.
- Keep the root SQL session short and close it when done; the scoped app user
  should be the only long-lived DB connection.
