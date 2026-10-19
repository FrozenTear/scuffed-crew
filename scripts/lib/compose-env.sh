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
#
# The relay container reads SURREALDB_RELAY_USER / SURREALDB_RELAY_PASSWORD.
# SURREALDB_RELAY_FALLBACK_USER and SURREALDB_RELAY_FALLBACK_PASSWORD are
# always empty. This script does not copy SURREALDB_PASSWORD into the relay
# container. A missing relay password logs a warning and does not print it.
#
# SITE_SERVER_IMAGE defaults to ghcr.io/frozentear/scuffed-crew:main.
# SITE_SERVER_IMAGE_DIGEST, when set, replaces the tag with @sha256:<hex>.

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
    SURREALDB_RELAY_FALLBACK_USER=""
    SURREALDB_RELAY_FALLBACK_PASSWORD=""
    local relay_pw
    relay_pw="${SURREALDB_RELAY_PASSWORD:-}"
    relay_pw="${relay_pw#"${relay_pw%%[![:space:]]*}"}"
    relay_pw="${relay_pw%"${relay_pw##*[![:space:]]}"}"
    if [[ -z "$relay_pw" ]]; then
        echo "warning: SURREALDB_RELAY_PASSWORD is unset or blank. The relay container does not receive SURREALDB_PASSWORD. PRODUCTION=1 refuses to start the relay until the relay password is set." >&2
    fi
    resolve_site_server_image
    export REDIRECT_BASE_URL ALLOWED_ORIGINS SURREALDB_APP_PASSWORD \
        SURREALDB_RELAY_FALLBACK_USER SURREALDB_RELAY_FALLBACK_PASSWORD \
        SITE_SERVER_IMAGE
}

# Build the image reference compose and podman pull will use.
#
# Unset SITE_SERVER_IMAGE stays on :main (dev host). A tag pin is the full
# SITE_SERVER_IMAGE value, including ghcr.io/frozentear/scuffed-crew:sha-<short>
# or an image that already ends in @sha256:<hex>. SITE_SERVER_IMAGE_DIGEST
# wins over the tag: the repository is kept and the tag is replaced.
resolve_site_server_image() {
    local image digest hex repo name prefix
    image="${SITE_SERVER_IMAGE:-ghcr.io/frozentear/scuffed-crew:main}"
    digest="${SITE_SERVER_IMAGE_DIGEST:-}"
    if [[ "$image" == *' '* || "$image" == *$'\n'* || "$image" == *$'\t'* || "$image" == *'$'* || "$image" == *'`'* ]]; then
        echo "error: SITE_SERVER_IMAGE contains a character this script will not pass to podman" >&2
        return 1
    fi
    if [[ -z "$digest" ]]; then
        SITE_SERVER_IMAGE="$image"
        return 0
    fi
    hex="${digest#sha256:}"
    hex="$(printf '%s' "$hex" | tr '[:upper:]' '[:lower:]')"
    if [[ ! "$hex" =~ ^[0-9a-f]{64}$ ]]; then
        echo "error: SITE_SERVER_IMAGE_DIGEST must be 64 hex characters, with or without a sha256: prefix" >&2
        return 1
    fi
    repo="${image%@sha256:*}"
    if [[ "$repo" == */* ]]; then
        name="${repo##*/}"
        prefix="${repo%/*}"
        name="${name%%:*}"
        repo="${prefix}/${name}"
    else
        repo="${repo%%:*}"
    fi
    SITE_SERVER_IMAGE="${repo}@sha256:${hex}"
}
