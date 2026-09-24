#!/usr/bin/env bash
# Fill TRUSTED_PROXIES in data/secrets.env when it is missing or empty.
#
# Host Caddy proxies to 127.0.0.1:HOST_PORT. Podman forwards that into the
# site-server container, and the peer the app sees is the compose-network
# gateway (not the browser, and usually not 127.0.0.1). The rate limiter
# trusts loopback only unless TRUSTED_PROXIES lists that gateway.
#
# Safe to re-run: a non-empty TRUSTED_PROXIES is left untouched.
# Prints a line containing "(wrote)" when it changes the file.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SECRETS="${SECRETS:-$ROOT/data/secrets.env}"
PROJECT_NAME="${COMPOSE_PROJECT_NAME:-$(basename "$ROOT")}"

if [[ ! -f "$SECRETS" ]]; then
    echo "error: missing $SECRETS — run scripts/install.sh first" >&2
    exit 1
fi

current="$(grep -E '^TRUSTED_PROXIES=' "$SECRETS" | head -n1 | cut -d= -f2- | tr -d '[:space:]' || true)"
if [[ -n "$current" ]]; then
    echo "TRUSTED_PROXIES already set (${current})"
    exit 0
fi

if ! command -v podman >/dev/null 2>&1; then
    echo "warning: podman not available; cannot discover the compose gateway." >&2
    echo "Set TRUSTED_PROXIES in $SECRETS to the site-server network gateway (see docs/deploy.md)." >&2
    exit 0
fi

collect_json() {
    local cid name net
    # Container inspect works while site-server still exists (before update.sh
    # removes it). Gateways here are the addresses the app's peer can be.
    while read -r cid name; do
        [[ -n "${cid}" ]] || continue
        case "${name}" in
            "${PROJECT_NAME}"-site-server|"${PROJECT_NAME}"-site-server-*|\
            "${PROJECT_NAME}"_site-server|"${PROJECT_NAME}"_site-server_*)
                podman inspect "${cid}" --format '{{json .NetworkSettings.Networks}}' 2>/dev/null || true
                echo
                ;;
        esac
    done < <(podman ps -a --format '{{.ID}} {{.Names}}' 2>/dev/null || true)

    # Networks outlive the container. Names follow the compose project.
    while read -r net; do
        [[ -n "${net}" ]] || continue
        case "${net}" in
            "${PROJECT_NAME}_internal"|"${PROJECT_NAME}_external"|\
            "${PROJECT_NAME}-internal"|"${PROJECT_NAME}-external")
                podman network inspect "${net}" 2>/dev/null || true
                echo
                ;;
        esac
    done < <(podman network ls --format '{{.Name}}' 2>/dev/null || true)
}

extract_gateways() {
    python3 -c '
import json, sys
raw = sys.stdin.read()
gws = []

def add(gw):
    if not isinstance(gw, str):
        return
    gw = gw.strip()
    if not gw or gw in ("<nil>", "None", "null"):
        return
    if gw not in gws:
        gws.append(gw)

def walk(obj):
    if isinstance(obj, dict):
        if "gateway" in obj:
            add(obj.get("gateway"))
        if "Gateway" in obj:
            add(obj.get("Gateway"))
        for v in obj.values():
            walk(v)
    elif isinstance(obj, list):
        for v in obj:
            walk(v)

dec = json.JSONDecoder()
idx = 0
n = len(raw)
while idx < n:
    while idx < n and raw[idx].isspace():
        idx += 1
    if idx >= n:
        break
    try:
        obj, end = dec.raw_decode(raw, idx)
    except json.JSONDecodeError:
        break
    walk(obj)
    idx = end
print(",".join(gws))
'
}

if ! command -v python3 >/dev/null 2>&1; then
    echo "warning: python3 is required to parse podman network JSON." >&2
    echo "Set TRUSTED_PROXIES in $SECRETS manually (see docs/deploy.md)." >&2
    exit 0
fi

gws="$(collect_json | extract_gateways || true)"
if [[ -z "$gws" ]]; then
    echo "warning: no compose gateway found for project ${PROJECT_NAME}." >&2
    echo "After the stack is up, set TRUSTED_PROXIES in $SECRETS to the gateway" >&2
    echo "from: podman network inspect ${PROJECT_NAME}_internal" >&2
    echo "See docs/deploy.md. Until then, forwarded client IPs are trusted from loopback only." >&2
    exit 0
fi

tmp="$(mktemp)"
if grep -q '^TRUSTED_PROXIES=' "$SECRETS"; then
    awk -v v="$gws" '
        BEGIN { done = 0 }
        /^TRUSTED_PROXIES=/ {
            if (!done) { print "TRUSTED_PROXIES=" v; done = 1 }
            next
        }
        { print }
        END { if (!done) print "TRUSTED_PROXIES=" v }
    ' "$SECRETS" > "$tmp"
else
    cat "$SECRETS" > "$tmp"
    printf '\n# Podman network gateway(s) allowed to supply X-Forwarded-For. See docs/deploy.md.\nTRUSTED_PROXIES=%s\n' "$gws" >> "$tmp"
fi
mv "$tmp" "$SECRETS"
chmod 600 "$SECRETS"
echo "TRUSTED_PROXIES=${gws} (wrote)"
