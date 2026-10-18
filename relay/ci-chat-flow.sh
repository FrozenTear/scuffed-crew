#!/usr/bin/env bash
# Build the relay image, seed one team channel, and run the chat flow check.
#
# The check authenticates to the read gate, loads the public channel from
# the team_channel rows, subscribes to kind 9 on #h and kind 1059 on #p,
# publishes one of each straight to strfry, and reads both back.
# It also requires a cross-team h tag, the allowlist, an unrelated kind,
# and the rate limit to be refused. Group enforcement defaults on.
#
# RELAY_SKIP_BUILD=1 uses $RELAY_IMAGE as-is (for a before/after image).
# RELAY_POLICY_RATE_LIMIT inside the container is 1 so the second member
# event is the rate-limit case.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

IMAGE="${RELAY_IMAGE:-relay-ci:local}"
NET="${RELAY_CI_NET:-relay-ci-net}"
SURREAL_NAME="${RELAY_CI_SURREAL:-relay-ci-surreal}"
STRFRY_NAME="${RELAY_CI_STRFRY:-relay-ci-strfry}"
SURREAL_PORT="${RELAY_CI_SURREAL_PORT:-18000}"
GATE_PORT="${RELAY_CI_GATE_PORT:-17779}"
STRFRY_PORT="${RELAY_CI_STRFRY_PORT:-17778}"
STATE="${RELAY_CHAT_STATE:-/tmp/relay-chat-flow.json}"

if docker info >/dev/null 2>&1; then
    DOCKER=(docker)
else
    DOCKER=(sudo docker)
fi

cleanup() {
    "${DOCKER[@]}" rm -f "$STRFRY_NAME" "$SURREAL_NAME" >/dev/null 2>&1 || true
    "${DOCKER[@]}" network rm "$NET" >/dev/null 2>&1 || true
}
trap cleanup EXIT

cleanup
"${DOCKER[@]}" network create "$NET" >/dev/null

if [[ "${RELAY_SKIP_BUILD:-}" != "1" ]]; then
    echo "building $IMAGE"
    "${DOCKER[@]}" build -f relay/Containerfile -t "$IMAGE" "$ROOT"
fi

echo "starting surreal"
"${DOCKER[@]}" run -d --name "$SURREAL_NAME" --network "$NET" \
    -p "127.0.0.1:${SURREAL_PORT}:8000" \
    -u 0:0 \
    -e SURREAL_USER=root \
    -e SURREAL_PASS=root \
    -e SURREAL_BIND=0.0.0.0:8000 \
    docker.io/surrealdb/surrealdb:v3.0 \
    start rocksdb:/data/scuffed.db

export SURREALDB_URL="ws://127.0.0.1:${SURREAL_PORT}"
export SURREALDB_USER=root
export SURREALDB_PASSWORD=root
export SURREALDB_NS=scuffed_crew
export SURREALDB_DB=main
export RELAY_CHAT_STATE="$STATE"
export RELAY_GATE_URL="ws://127.0.0.1:${GATE_PORT}"
export RELAY_STRFRY_URL="ws://127.0.0.1:${STRFRY_PORT}"

echo "seeding team channel"
cargo run -p relay-policy --features chat-flow --bin relay-chat-flow -- seed

echo "starting relay"
# strfry calls setrlimit(NOFILE, 1000000) and exits when the hard limit is lower.
# mkdir covers images whose entrypoint does not create strfry-db.
"${DOCKER[@]}" run -d --name "$STRFRY_NAME" --network "$NET" \
    --entrypoint /bin/sh \
    --ulimit nofile=1048576:1048576 \
    -p "127.0.0.1:${GATE_PORT}:7779" \
    -p "127.0.0.1:${STRFRY_PORT}:7777" \
    -e SURREALDB_URL="ws://${SURREAL_NAME}:8000" \
    -e SURREALDB_USER=root \
    -e SURREALDB_PASSWORD=root \
    -e SURREALDB_NS=scuffed_crew \
    -e SURREALDB_DB=main \
    -e RELAY_POLICY_RATE_LIMIT=1 \
    -e RELAY_POLICY_RATE_WINDOW=60 \
    -e RELAY_GATE_LISTEN=0.0.0.0:7779 \
    -e RELAY_GATE_UPSTREAM=ws://127.0.0.1:7777 \
    -e RUST_LOG=relay_policy=info,relay_read_gate=info \
    "$IMAGE" \
    -c 'mkdir -p /app/strfry-db && exec /bin/sh /usr/local/bin/relay-entrypoint.sh'

wait_log() {
    local needle="$1"
    local i
    for i in $(seq 1 45); do
        if "${DOCKER[@]}" logs "$STRFRY_NAME" 2>&1 | grep -q "$needle"; then
            echo "relay log has: $needle"
            return 0
        fi
        if ! "${DOCKER[@]}" inspect -f '{{.State.Running}}' "$STRFRY_NAME" 2>/dev/null | grep -qx true; then
            echo "relay container exited before: $needle"
            return 1
        fi
        sleep 1
    done
    echo "timed out waiting for relay log: $needle"
    "${DOCKER[@]}" logs "$STRFRY_NAME" 2>&1 | tail -80 || true
    return 1
}

# relay-policy is spawned by strfry on the first write, so the allowlist
# line is not in the log until the check publishes.
wait_log "read gate listening" || true

set +e
cargo run -p relay-policy --features chat-flow --bin relay-chat-flow -- check
code=$?
set -e
if [[ "$code" -ne 0 ]]; then
    echo "---- relay logs ----"
    "${DOCKER[@]}" logs "$STRFRY_NAME" 2>&1 | tail -160 || true
    exit "$code"
fi
