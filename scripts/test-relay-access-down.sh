#!/usr/bin/env bash
# The rollback script removes the record login and does not put a password
# on the surreal command line.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=relay-access-down.sh
source "${SCRIPT_DIR}/relay-access-down.sh"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

sql="$(relay_access_down_sql)"
[[ "$sql" == *"REMOVE ACCESS IF EXISTS relay ON DATABASE;"* ]] || fail "missing REMOVE ACCESS"
[[ "$sql" == *"REMOVE TABLE IF EXISTS relay_db_user;"* ]] || fail "missing REMOVE TABLE"
for statement in \
    "REMOVE EVENT IF EXISTS relay_member_write ON TABLE member;" \
    "REMOVE EVENT IF EXISTS relay_member_delete ON TABLE member;" \
    "REMOVE EVENT IF EXISTS relay_roster_write ON TABLE plays_on;" \
    "REMOVE EVENT IF EXISTS relay_roster_delete ON TABLE plays_on;" \
    "REMOVE EVENT IF EXISTS relay_moderation_write ON TABLE moderation_action;" \
    "REMOVE EVENT IF EXISTS relay_moderation_delete ON TABLE moderation_action;" \
    "REMOVE EVENT IF EXISTS relay_team_channel_write ON TABLE team_channel;" \
    "REMOVE EVENT IF EXISTS relay_team_channel_delete ON TABLE team_channel;" \
    "REMOVE TABLE IF EXISTS relay_member;" \
    "REMOVE TABLE IF EXISTS relay_roster;" \
    "REMOVE TABLE IF EXISTS relay_moderation;" \
    "REMOVE TABLE IF EXISTS relay_team_channel;"
do
    [[ "$sql" == *"$statement"* ]] || fail "missing $statement"
done
events_at=$(printf '%s' "$sql" | grep -n 'REMOVE EVENT IF EXISTS relay_member_write' | head -1 | cut -d: -f1)
tables_at=$(printf '%s' "$sql" | grep -n 'REMOVE TABLE IF EXISTS relay_member;' | head -1 | cut -d: -f1)
access_at=$(printf '%s' "$sql" | grep -n 'REMOVE ACCESS IF EXISTS relay' | head -1 | cut -d: -f1)
[[ "$events_at" -lt "$tables_at" && "$tables_at" -lt "$access_at" ]] || fail "rollback order is events, tables, then access"

argv="$(relay_access_down_argv)"
[[ "$argv" == *"/surreal"* ]] || fail "argv does not call /surreal"
[[ "$argv" != *"--pass"* && "$argv" != *"--password"* ]] || fail "argv carries a password flag"
[[ "$argv" != *"SURREALDB_PASSWORD"* && "$argv" != *"SURREAL_PASS"* ]] || fail "argv names the password"

script="${SCRIPT_DIR}/relay-access-down.sh"
grep -q -- '--yes' "$script" || fail "script does not require confirmation"
if grep -n -E 'awk[[:space:]]+-v|echo .*(SURREALDB_PASSWORD|SURREAL_PASS)|--pass|--password' "$script"; then
    fail "script can pass or print a password"
fi
grep -q 'relay-access-down.sh' "${SCRIPT_DIR}/../docs/deploy.md" || fail "docs/deploy.md does not name the script"
grep -q 'Stop the relay' "${SCRIPT_DIR}/../docs/deploy.md" || fail "docs do not say to stop the relay first"

echo "relay access rollback checks passed"
