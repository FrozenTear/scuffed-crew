#!/usr/bin/env bash
# Fail when the server reads an environment variable that compose.yml does
# not pass to the site-server container.
#
# compose.yml has no env_file. data/secrets.env is only used to fill in
# ${...} values, so a variable that is not listed under site-server's
# `environment:` never reaches the process, even when it is set in
# secrets.env. That silently disabled CSP_ENFORCE, ENCRYPTION_KEY_PREVIOUS,
# and the Discord/Matrix notifiers.
#
# Names are collected from the crates linked into scuffed-server:
#   env::var("NAME") / env::var_os("NAME")
#   any helper whose name contains "env" called with a literal: env_usize("NAME", ..)
# Each must appear as a key under services.site-server.environment, or in
# ALLOWED_OMISSIONS below with a reason.
#
# Usage: scripts/check-compose-env.sh [repo-root] [compose-file]
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
COMPOSE="${2:-${ROOT}/compose.yml}"
CRATES=(server site-server db auth chat types)

# Read by the code but deliberately not forwarded. Keep the reason.
declare -A ALLOWED_OMISSIONS=(
    # The image listens on 3000 and compose maps HOST_PORT to it.
    [PORT]="container port is fixed at 3000; HOST_PORT picks the host side"
    # One-shot init job. Forwarding it from secrets.env would make the
    # long-running service migrate and exit on every restart. Run it as
    # `compose run --rm -e SURREALDB_MIGRATE_ONLY=1 site-server` instead.
    [SURREALDB_MIGRATE_ONLY]="one-shot job flag; pass with compose run -e"
    # Read only inside #[cfg(test)] in routes/seo.rs.
    [SCUFFED_EXTRA_INDEX]="test-only"
)

if ! command -v perl >/dev/null 2>&1; then
    echo "error: perl is required" >&2
    exit 1
fi

# Names read by Rust sources on stdin (whole files, so calls split across
# lines by rustfmt are still found).
extract_names() {
    perl -0777 -ne '
        while (/\b(?:env::var(?:_os)?|[A-Za-z_]*env[A-Za-z_]*)\s*\(\s*"([A-Z][A-Z0-9_]*)"/g) {
            print "$1\n";
        }
    '
}

# Keys directly under services.site-server.environment.
compose_site_env_keys() {
    awk '
        /^[^[:space:]#]/ { in_services = ($0 ~ /^services:/) ; svc = ""; next }
        in_services && /^  [A-Za-z0-9_.-]+:/ {
            svc = $1; sub(/:$/, "", svc); in_env = 0; next
        }
        svc == "site-server" && /^    environment:/ { in_env = 1; next }
        svc == "site-server" && /^    [A-Za-z_]/ { in_env = 0; next }
        in_env && /^      [A-Za-z_][A-Za-z0-9_]*:/ {
            k = $1; sub(/:$/, "", k); print k
        }
    ' "$1"
}

# --- self-tests: a detector that cannot fail guards nothing -----------------
fixture_names="$(printf '%s\n' \
    'let a = std::env::var("ALPHA_ONE").ok();' \
    'let b = std::env::var(' '    "BRAVO_TWO",' ').ok();' \
    'let c = env_usize("CHARLIE", 3);' \
    'let d = non_empty_env("DELTA_X");' \
    'let e = std::env::var_os("ECHO");' \
    'let f = "NOT_AN_ENV";' | extract_names | sort | tr '\n' ' ')"
if [[ "${fixture_names}" != "ALPHA_ONE BRAVO_TWO CHARLIE DELTA_X ECHO " ]]; then
    echo "error: env-name extractor self-test failed: got '${fixture_names}'" >&2
    exit 1
fi
fixture_compose="$(mktemp)"
trap 'rm -f "${fixture_compose}"' EXIT
cat >"${fixture_compose}" <<'YAML'
services:
  surrealdb:
    environment:
      ONLY_DB: x
  site-server:
    image: x
    environment:
      SITE_A: ${SITE_A:-}
      # comment
      SITE_B: b
    volumes:
      - NOT_A_KEY:/x
networks:
  internal:
YAML
fixture_keys="$(compose_site_env_keys "${fixture_compose}" | sort | tr '\n' ' ')"
if [[ "${fixture_keys}" != "SITE_A SITE_B " ]]; then
    echo "error: compose environment parser self-test failed: got '${fixture_keys}'" >&2
    exit 1
fi

# --- real check -------------------------------------------------------------
if [[ ! -f "${COMPOSE}" ]]; then
    echo "error: ${COMPOSE} not found" >&2
    exit 1
fi

src_dirs=()
for c in "${CRATES[@]}"; do
    [[ -d "${ROOT}/crates/${c}/src" ]] && src_dirs+=("${ROOT}/crates/${c}/src")
done
if [[ "${#src_dirs[@]}" -eq 0 ]]; then
    echo "error: no crate sources found under ${ROOT}/crates" >&2
    exit 1
fi

mapfile -t code_names < <(
    find "${src_dirs[@]}" -type f -name '*.rs' -print0 \
        | xargs -0 cat \
        | extract_names | sort -u
)
mapfile -t compose_keys < <(compose_site_env_keys "${COMPOSE}" | sort -u)

if [[ "${#code_names[@]}" -eq 0 ]]; then
    echo "error: found no env var reads in ${CRATES[*]}; the extractor is broken" >&2
    exit 1
fi
if [[ "${#compose_keys[@]}" -eq 0 ]]; then
    echo "error: found no site-server environment keys in ${COMPOSE}" >&2
    exit 1
fi

declare -A in_compose=()
for k in "${compose_keys[@]}"; do in_compose["$k"]=1; done
declare -A in_code=()
for n in "${code_names[@]}"; do in_code["$n"]=1; done

status=0
for n in "${code_names[@]}"; do
    if [[ -n "${in_compose[$n]:-}" ]]; then
        continue
    fi
    if [[ -n "${ALLOWED_OMISSIONS[$n]:-}" ]]; then
        continue
    fi
    echo "ERROR: ${n} is read by the server but not passed to site-server in ${COMPOSE#"${ROOT}"/}" >&2
    status=1
done

# A stale allow-list entry hides nothing today but would hide a real gap later.
for n in "${!ALLOWED_OMISSIONS[@]}"; do
    if [[ -z "${in_code[$n]:-}" ]]; then
        echo "ERROR: allow-listed ${n} is no longer read by the server; remove it from ALLOWED_OMISSIONS" >&2
        status=1
    elif [[ -n "${in_compose[$n]:-}" ]]; then
        echo "ERROR: allow-listed ${n} is also in compose.yml; drop one of them" >&2
        status=1
    fi
done

if [[ "${status}" -ne 0 ]]; then
    echo "Add the variable under site-server environment: with a default that keeps the code's unset behaviour" >&2
    echo "(\${VAR:-} only when the code treats an empty value like unset), or allow-list it here with a reason." >&2
    exit 1
fi

echo "ok: ${#code_names[@]} server env vars; all passed to site-server or allow-listed (${!ALLOWED_OMISSIONS[*]})"
