# shellcheck shell=bash
# Decide whether relay-up.sh may recreate strfry.
# The password, /api/ready, and a real record sign-in all have to succeed
# before that switch. This file does not run compose.

trim_secret() {
    local raw="$1"
    raw="${raw#"${raw%%[![:space:]]*}"}"
    raw="${raw%"${raw##*[![:space:]]}"}"
    printf '%s' "$raw"
}

# Prints a reason on stdout and returns 1 when the switch must not happen.
# Prints "ok" and returns 0 when ready_code and signin_code are both 200
# and the password is a non-blank base64-alphabet value.
relay_preflight_decision() {
    local password ready signin
    password="$(trim_secret "$1")"
    ready="$2"
    signin="$3"
    if [[ -z "$password" ]]; then
        echo "SURREALDB_RELAY_PASSWORD is missing or blank"
        return 1
    fi
    if [[ ! "$password" =~ ^[A-Za-z0-9+/=]+$ ]]; then
        echo "SURREALDB_RELAY_PASSWORD has characters this script will not send"
        return 1
    fi
    if [[ "$ready" != "200" ]]; then
        echo "/api/ready returned ${ready}"
        return 1
    fi
    if [[ "$signin" != "200" ]]; then
        echo "relay sign-in returned ${signin}"
        return 1
    fi
    echo "ok"
    return 0
}

# HTTP status from POST /signin inside the site-server container.
# Prints only the status code. Never prints the body or the password.
relay_signin_status() {
    local password user ns db container payload code
    password="$(trim_secret "$1")"
    if [[ -z "$password" || ! "$password" =~ ^[A-Za-z0-9+/=]+$ ]]; then
        printf '000'
        return 0
    fi
    user="$(trim_secret "${SURREALDB_RELAY_USER:-scuffed_relay}")"
    ns="$(trim_secret "${SURREALDB_NS:-scuffed_crew}")"
    db="$(trim_secret "${SURREALDB_DB:-main}")"
    if [[ ! "$user" =~ ^[A-Za-z0-9_]+$ || ! "$ns" =~ ^[A-Za-z0-9_]+$ || ! "$db" =~ ^[A-Za-z0-9_]+$ ]]; then
        printf '000'
        return 0
    fi
    container="$(podman ps -a --format '{{.Names}}' 2>/dev/null \
        | grep -E "^${PROJECT_NAME}[-_]site-server([-_][0-9]+)?$" | head -n1 || true)"
    if [[ -z "$container" ]]; then
        printf '000'
        return 0
    fi
    payload="$(printf '{"ns":"%s","db":"%s","ac":"relay","user":"%s","pass":"%s"}' \
        "$ns" "$db" "$user" "$password")"
    # timeout covers a stuck podman exec. curl's own limits cover a hung sign-in.
    code="$(printf '%s' "$payload" | timeout 20 podman exec -i "$container" \
        curl -sS -o /dev/null -w '%{http_code}' \
        --connect-timeout 2 --max-time 5 \
        -H 'Accept: application/json' \
        -H 'Content-Type: application/json' \
        --data-binary @- \
        http://surrealdb:8000/signin 2>/dev/null || true)"
    if [[ ! "$code" =~ ^[0-9]{3}$ ]]; then
        printf '000'
        return 0
    fi
    printf '%s' "$code"
}
