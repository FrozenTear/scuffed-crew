#!/bin/sh
# strfry on 7777 (Compose DNS ws://strfry:7777). Read gate on 7779, which
# Compose publishes as host 127.0.0.1:7777 for Caddy /relay.
set -eu
cd /app
# strfry.conf stores the DB at ./strfry-db. Compose mounts a volume here.
# A container started without that mount still needs the directory.
mkdir -p /app/strfry-db
# The base image keeps the binary at /app/strfry and does not put it on PATH.
/app/strfry relay &
strfry_pid=$!
relay-read-gate &
gate_pid=$!
trap 'kill "$strfry_pid" "$gate_pid" 2>/dev/null || true; wait || true' EXIT INT TERM
while kill -0 "$strfry_pid" 2>/dev/null && kill -0 "$gate_pid" 2>/dev/null; do
    sleep 1
done
exit 1
