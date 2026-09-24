#!/usr/bin/env bash
# Initialize the restic backup repository.
# Run once before the first backup.
#
# Environment (required):
#   RESTIC_REPOSITORY       Off-host restic repo (local paths need BACKUP_ALLOW_LOCAL_REPO=1)
#   RESTIC_PASSWORD         or RESTIC_PASSWORD_FILE (mode 600, outside the repo)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=lib/restic-access.sh
source "${SCRIPT_DIR}/lib/restic-access.sh"
load_restic_access

echo "Initializing restic repository at ${RESTIC_REPOSITORY}..."
restic init

echo "Repository initialized. You can now run backup.sh."
