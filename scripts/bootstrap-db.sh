#!/usr/bin/env bash
# Bootstrap SurrealDB namespace, database, and application user.
# Run once after starting SurrealDB for the first time.
#
# Prerequisites:
#   - SurrealDB running and reachable
#   - Root credentials available
#
# Usage:
#   ./scripts/bootstrap-db.sh [surreal-url]
#
# Environment:
#   SURREALDB_ROOT_USER     Root username (default: root)
#   SURREALDB_ROOT_PASSWORD Root password (required)

set -euo pipefail

SURREAL_URL="${1:-http://localhost:8000}"
ROOT_USER="${SURREALDB_ROOT_USER:-root}"
ROOT_PASS="${SURREALDB_ROOT_PASSWORD:?Set SURREALDB_ROOT_PASSWORD}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Bootstrapping SurrealDB at ${SURREAL_URL}..."
surreal import \
    --conn "${SURREAL_URL}" \
    --user "${ROOT_USER}" \
    --pass "${ROOT_PASS}" \
    "${SCRIPT_DIR}/bootstrap-db.surql"

echo "Done. Remember to change the app user password in bootstrap-db.surql for production."
