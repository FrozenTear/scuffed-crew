#!/usr/bin/env bash
# Initialize the restic backup repository.
# Run once before the first backup.
#
# Environment (required):
#   RESTIC_REPOSITORY   Restic repo path/URL
#   RESTIC_PASSWORD     Restic repo password

set -euo pipefail

: "${RESTIC_REPOSITORY:?Set RESTIC_REPOSITORY}"
: "${RESTIC_PASSWORD:?Set RESTIC_PASSWORD}"

echo "Initializing restic repository at ${RESTIC_REPOSITORY}..."
restic init

echo "Repository initialized. You can now run backup.sh."
