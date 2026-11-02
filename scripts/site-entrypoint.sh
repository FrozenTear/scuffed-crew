#!/bin/sh
# Prepare the private bug-report and hero-icon directories, then run the
# server as scuffed. A named volume is often root-owned. If a directory cannot
# be created or handed to scuffed, the server disables that feature (report
# uploads or hero icon uploads) and keeps serving.
set -eu
reports="${REPORTS_DIR:-/app/data/reports}"
if mkdir -p "$reports" && chown -R scuffed:scuffed "$reports" && chmod 0700 "$reports"; then
    :
else
    echo "stat reports: could not prepare ${reports}; the server will disable report uploads" >&2
fi
hero_icons="${HERO_ICONS_DIR:-/app/data/hero-icons}"
if mkdir -p "$hero_icons" && chown -R scuffed:scuffed "$hero_icons" && chmod 0700 "$hero_icons"; then
    :
else
    echo "hero icons: could not prepare ${hero_icons}; the server will disable hero icon uploads" >&2
fi
exec runuser -u scuffed -- /app/scuffed-server
