# shellcheck shell=bash
# Fallbacks for compose.yml. podman-compose 1.5 cannot expand a nested
# default: ${A:-${B}} becomes A's value plus a stray "}" when A is set, and
# literal text when A is unset. Call export_compose_fallbacks after sourcing
# secrets.env and before every compose command.
#
# ALLOWED_ORIGINS blanks become REDIRECT_BASE_URL (else the loopback URL).
# SURREALDB_APP_PASSWORD blanks become SURREALDB_PASSWORD. A missing app
# password key is still generated distinct from root by install.sh/update.sh
# before this runs; production refuses to boot if the two passwords match.

export_compose_fallbacks() {
    local host_port redirect
    host_port="${HOST_PORT:-3000}"
    redirect="${REDIRECT_BASE_URL:-http://127.0.0.1:${host_port}}"
    REDIRECT_BASE_URL="$redirect"
    if [[ -z "${ALLOWED_ORIGINS:-}" ]]; then
        ALLOWED_ORIGINS="$redirect"
    fi
    if [[ -z "${SURREALDB_APP_PASSWORD:-}" ]]; then
        SURREALDB_APP_PASSWORD="${SURREALDB_PASSWORD:-}"
    fi
    export REDIRECT_BASE_URL ALLOWED_ORIGINS SURREALDB_APP_PASSWORD
}
