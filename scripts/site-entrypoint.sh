#!/bin/sh
# Prepare the upload, private bug-report and hero-icon directories, then run
# the server as scuffed. A named volume is often root-owned. If the report or
# hero-icon directory cannot be created or handed to scuffed, the server
# disables that feature (report uploads or hero icon uploads) and keeps
# serving. Image uploads have no off switch, so a failure there is only logged.
set -eu
uploads="${UPLOAD_DIR:-/app/data/uploads}"
if mkdir -p "$uploads" && chown -R scuffed:scuffed "$uploads"; then
    :
else
    echo "uploads: could not prepare ${uploads}; image uploads will fail until it is writable by scuffed" >&2
fi
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
