# audit_log operations — GDPR / right-to-erasure prune

`audit_log` is **append-only** in production (DR1-DB-007). Two layers enforce it
(`crates/db/src/migrations.rs`, lines 227–240):

```surql
DEFINE TABLE OVERWRITE audit_log SCHEMAFULL
    PERMISSIONS
        FOR select, create FULL
        FOR update, delete NONE;

DEFINE EVENT OVERWRITE audit_log_append_only ON TABLE audit_log
    WHEN $event = "UPDATE" OR $event = "DELETE"
    THEN { THROW "audit_log is append-only (DR1-DB-007)"; };
```

The **event** is the layer that actually stops deletes: it runs inside the write
transaction for **every** writer regardless of role — including the scoped
EDITOR app user **and root** — so a `DELETE` is rolled back by the `THROW` even
when table PERMISSIONS would otherwise allow it. (Table PERMISSIONS `delete
NONE` blocks the scoped app user; root bypasses PERMISSIONS but not the event.)

Because of this, a legitimate GDPR / right-to-erasure prune cannot be done with
a plain `DELETE`. An operator must temporarily **remove the event**, delete the
targeted rows, then **re-create the event exactly**. The window between the two
is the only time the table is mutable, so keep it as short as possible.

## Prune procedure

### 0. Back up first, and be root

```bash
./scripts/backup.sh
```

`REMOVE EVENT` / `DEFINE EVENT` are schema operations — you must be **root**, not
the scoped `scuffed_app` EDITOR user (which cannot redefine schema and is blocked
by both layers anyway). Open a root SQL shell against the running DB (SurrealDB
v3, service `surrealdb`, ns `scuffed_crew`, db `main` — see `compose.yml`; root
creds are `SURREALDB_USER`/`SURREALDB_PASSWORD` in `data/secrets.env`):

```bash
set -a; . ./data/secrets.env; set +a
podman compose --env-file data/secrets.env exec surrealdb \
  /surreal sql \
    --endpoint ws://localhost:8000 \
    --username "$SURREALDB_USER" --password "$SURREALDB_PASSWORD" \
    --namespace scuffed_crew --database main --pretty
```

### 1. (Optional) Preview exactly what you will delete

```surql
SELECT meta::id(id) AS id, actor_id, action, target_type, target_id, created_at
FROM audit_log
WHERE actor_id = '<user_id>' OR target_id = '<user_id>';
```

### 2. Drop the append-only event (starts the mutable window)

```surql
REMOVE EVENT audit_log_append_only ON TABLE audit_log;
```

### 3. Targeted delete

Erase only the rows you must. For right-to-erasure of one user (they may appear
as either the actor or the target):

```surql
DELETE audit_log WHERE actor_id = '<user_id>' OR target_id = '<user_id>';
```

(For a retention prune instead, filter by `created_at < <cutoff>` — e.g.
`DELETE audit_log WHERE created_at < time::now() - 2y;`.)

### 4. Re-create the event EXACTLY as defined in migrations

Copy this verbatim from `migrations.rs` lines 238–240 — do not paraphrase, or a
future migration re-`OVERWRITE` may diverge from your hand-typed version:

```surql
DEFINE EVENT OVERWRITE audit_log_append_only ON TABLE audit_log
    WHEN $event = "UPDATE" OR $event = "DELETE"
    THEN { THROW "audit_log is append-only (DR1-DB-007)"; };
```

The mutable window is now closed.

### 5. Verify immutability is restored

```surql
-- Event is present again:
INFO FOR TABLE audit_log;

-- A delete must now be refused with the THROW message:
DELETE audit_log LIMIT 1;
-- expect: an error "audit_log is append-only (DR1-DB-007)" and NO row removed
```

## Warnings

- **The window between step 2 and step 4 has no immutability.** Any writer can
  mutate `audit_log` then. Do the whole procedure in one uninterrupted root
  session, during maintenance, right after a backup — never leave the event
  removed.
- The `PERMISSIONS ... FOR update, delete NONE` clause stays intact across this
  procedure; you are only toggling the event. Do not weaken the table
  permissions.
- Re-running the app's migrations (`SURREALDB_MIGRATE_ONLY=1` /
  `bootstrap_from_env`) also re-`OVERWRITE`s the event and table, so a normal
  deploy will restore the guard if you forget step 4 — but do not rely on that;
  verify per step 5.
- Manual deletes leave no audit trail of themselves (you are editing the audit
  trail). Record the prune out-of-band (ticket / erasure log) with scope, the
  filter used, and who authorized it.
