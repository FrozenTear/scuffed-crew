#!/bin/sh
# Prepare the private bug-report directory, then run the server as scuffed.
# A named volume is often root-owned. If this directory cannot be created or
# handed to scuffed, the server disables report uploads and keeps serving.
set -eu
reports="${REPORTS_DIR:-/app/data/reports}"
if mkdir -p "$reports" && chown -R scuffed:scuffed "$reports" && chmod 0700 "$reports"; then
    :
else
    echo "stat reports: could not prepare ${reports}; the server will disable report uploads" >&2
fi
exec runuser -u scuffed -- /app/scuffed-server
