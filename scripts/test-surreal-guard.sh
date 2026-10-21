#!/usr/bin/env bash
# scripts/lib/surreal-guard.sh and the scripts that use it must never let
# compose create or recreate this project's SurrealDB container. No real
# podman: a stub records every call. Runs update.sh, reset-local-admin.sh and
# install.sh end to end against the stub.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "${SCRIPT_DIR}/.." && pwd)"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

TMP="$(mktemp -d)"
trap '[[ -n "${KEEP_TMP:-}" ]] || rm -rf "${TMP}"' EXIT

# --- stub podman / curl / ss --------------------------------------------------
BIN="${TMP}/bin"
mkdir -p "${BIN}"
cat >"${BIN}/podman" <<'STUB'
#!/usr/bin/env bash
# State: $STUB_STATE/containers  lines "id<TAB>name<TAB>status<TAB>image[<TAB>image-id]"
#        $STUB_STATE/volumes     one name per line
#        $STUB_STATE/images      lines "ref<TAB>image-id" (pulled images)
echo "podman $*" >>"${STUB_STATE}/calls"
cmd="$1"; shift || true
case "${cmd}" in
    compose)
        [[ "${1:-}" == "version" ]] && { echo "podman-compose version 1.6.0"; exit 0; }
        # An up creates the site-server container, as compose would.
        if [[ " $* " == *" up "* ]] && ! grep -q 'site-server' "${STUB_STATE}/containers" 2>/dev/null; then
            printf 's1\tscuffed-crew-site-server-1\tUp 1 second\tghcr.io/frozentear/scuffed-crew:main\n' \
                >>"${STUB_STATE}/containers"
        fi
        exit 0 ;;
    pull) exit 0 ;;
    ps)
        fmt=""
        while [[ $# -gt 0 ]]; do
            case "$1" in --format) fmt="$2"; shift 2 ;; *) shift ;; esac
        done
        [[ -f "${STUB_STATE}/containers" ]] || exit 0
        while IFS=$'\t' read -r id name status image _imgid; do
            [[ -n "${id}" ]] || continue
            if [[ -f "${STUB_STATE}/started_${id}" && ! -f "${STUB_STATE}/ps_stale" ]]; then
                status="Up 1 second"
            fi
            line="${fmt//\{\{.ID\}\}/${id}}"
            line="${line//\{\{.Names\}\}/${name}}"
            line="${line//\{\{.Status\}\}/${status}}"
            line="${line//\{\{.Image\}\}/${image}}"
            printf '%b\n' "${line}"
        done <"${STUB_STATE}/containers"
        exit 0 ;;
    inspect)
        fmt="$2"; id="$3"
        while IFS=$'\t' read -r cid name status image imgid; do
            [[ "${cid}" == "${id}" || "${name}" == "${id}" ]] || continue
            case "${fmt}" in
                *ImageName*) echo "${image}" ;;
                *'.Image}}'*) echo "${imgid:-}" ;;
                *State.Running*)
                    if [[ "${status}" == Up* || -f "${STUB_STATE}/started_${cid}" ]]; then echo true; else echo false; fi ;;
                *State.Status*) echo running ;;
            esac
            exit 0
        done <"${STUB_STATE}/containers"
        exit 1 ;;
    image)
        # podman image inspect <ref> --format '{{.Id}}'
        if [[ "${1:-}" == "inspect" ]]; then
            while IFS=$'\t' read -r ref imgid; do
                [[ "${ref}" == "$2" ]] && { echo "${imgid}"; exit 0; }
            done <"${STUB_STATE}/images"
            echo "Error: $2: image not known" >&2
            exit 125
        fi
        exit 0 ;;
    volume)
        [[ "${1:-}" == "ls" ]] && { cat "${STUB_STATE}/volumes" 2>/dev/null || true; }
        exit 0 ;;
    start)
        [[ -f "${STUB_STATE}/start_fails" ]] && exit 125
        touch "${STUB_STATE}/started_$1"; echo "$1"; exit 0 ;;
    *) exit 0 ;;
esac
STUB
printf '#!/usr/bin/env bash\nexit 0\n' >"${BIN}/curl"
printf '#!/usr/bin/env bash\nexit 0\n' >"${BIN}/ss"
chmod +x "${BIN}/podman" "${BIN}/curl" "${BIN}/ss"

# --- fixture repo (no .git, so update.sh skips git pull) -----------------------
FIX="${TMP}/scuffed-crew"
mkdir -p "${FIX}/scripts/lib" "${FIX}/data"
cp "${REPO}/compose.yml" "${FIX}/"
cp "${REPO}/scripts/update.sh" "${REPO}/scripts/install.sh" \
    "${REPO}/scripts/reset-local-admin.sh" "${FIX}/scripts/"
cp "${REPO}"/scripts/lib/*.sh "${FIX}/scripts/lib/"
printf '#!/usr/bin/env bash\nexit 0\n' >"${FIX}/scripts/ensure-trusted-proxies.sh"
cat >"${FIX}/data/secrets.env" <<'ENV'
SURREALDB_PASSWORD=rootpw
SURREALDB_APP_PASSWORD=apppw
ENCRYPTION_KEY=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
NOSTR_CHALLENGE_SECRET=x
HOST_PORT=3999
SITE_SERVER_IMAGE=ghcr.io/frozentear/scuffed-crew:main
ALLOWED_ORIGINS=http://127.0.0.1:3999
ENV

WANT="$(awk '/^  surrealdb:/{s=1} s && /^    image:/{print $2; exit}' "${REPO}/compose.yml")"
# Built from parts so scripts/check-surrealdb-version.sh does not read the
# old tag here as an image pin.
IMG_REPO="docker.io/surrealdb/surrealdb"
[[ "${WANT}" == "${IMG_REPO}":v* ]] || fail "could not read the surrealdb image from compose.yml (${WANT})"
OLD="${IMG_REPO}:v3.0"
P=scuffed-crew

# new_state <name> <containers-lines> <volumes-lines> [<images-lines>]
new_state() {
    STUB_STATE="${TMP}/state-$1"
    rm -rf "${STUB_STATE}"
    mkdir -p "${STUB_STATE}"
    printf '%b' "$2" >"${STUB_STATE}/containers"
    printf '%b' "$3" >"${STUB_STATE}/volumes"
    printf '%b' "${4:-}" >"${STUB_STATE}/images"
    : >"${STUB_STATE}/calls"
    export STUB_STATE
}

run_script() {
    # $1 = script, rest = env assignments
    local script="$1"
    shift
    env -i PATH="${BIN}:/usr/bin:/bin" HOME="${TMP}" STUB_STATE="${STUB_STATE}" \
        SURREAL_GUARD_WAIT_SECS=0 "$@" bash "${FIX}/scripts/${script}" \
        >"${STUB_STATE}/out" 2>&1 </dev/null
}

up_calls() { grep -E '^podman compose .* up ' "${STUB_STATE}/calls" || true; }

expect_refused() {
    local label="$1"
    grep -qE ' (rm|stop|start) ' "${STUB_STATE}/calls" \
        && fail "${label}: refused run still touched a container: $(grep -E ' (rm|stop|start) ' "${STUB_STATE}/calls" | head -3)"
    [[ -z "$(up_calls)" ]] || fail "${label}: refused run still ran compose up: $(up_calls)"
}

expect_no_deps_only() {
    local label="$1" calls
    calls="$(up_calls)"
    [[ -n "${calls}" ]] || fail "${label}: no compose up ran. Output: $(tail -5 "${STUB_STATE}/out")"
    if grep -v -- '--no-deps site-server' <<<"${calls}" | grep -q .; then
        fail "${label}: compose up without --no-deps site-server: ${calls}"
    fi
}

# --- update.sh ----------------------------------------------------------------
new_state up-match "c1\t${P}-surrealdb-1\tUp 3 days\t${WANT}\n" "${P}_surrealdb-data\n"
run_script update.sh || fail "update.sh, DB Up on compose image: exit $?. $(tail -5 "${STUB_STATE}/out")"
expect_no_deps_only "update.sh, DB Up on compose image"

new_state up-mismatch "c1\t${P}-surrealdb-1\tUp 3 days\t${OLD}\n" "${P}_surrealdb-data\n"
if run_script update.sh; then fail "update.sh accepted a DB on another image than compose.yml"; fi
grep -q "Upgrading the SurrealDB server" "${STUB_STATE}/out" || fail "update.sh refusal does not point at the runbook"
expect_refused "update.sh, DB image mismatch"

new_state up-mismatch-ok "c1\t${P}-surrealdb-1\tUp 3 days\t${OLD}\n" "${P}_surrealdb-data\n"
run_script update.sh SCUFFED_ALLOW_DB_IMAGE_MISMATCH=1 \
    || fail "update.sh with SCUFFED_ALLOW_DB_IMAGE_MISMATCH=1: exit $?. $(tail -5 "${STUB_STATE}/out")"
expect_no_deps_only "update.sh, mismatch allowed"
grep -qE 'podman (start|rm|stop) c1' "${STUB_STATE}/calls" && fail "update.sh touched the DB container with the mismatch flag"

new_state stopped-match "c1\t${P}-surrealdb-1\tExited (0) 1 hour ago\t${WANT}\n" "${P}_surrealdb-data\n"
run_script update.sh || fail "update.sh, stopped DB: exit $?. $(tail -5 "${STUB_STATE}/out")"
grep -q '^podman start c1$' "${STUB_STATE}/calls" || fail "update.sh did not podman start the stopped DB"
expect_no_deps_only "update.sh, stopped DB on compose image"

new_state stopped-wont-start "c1\t${P}-surrealdb-1\tExited (1) 1 hour ago\t${WANT}\n" "${P}_surrealdb-data\n"
touch "${STUB_STATE}/start_fails"
if run_script update.sh; then fail "update.sh went on after podman start of the DB failed"; fi
[[ -z "$(up_calls)" ]] || fail "update.sh ran compose up after the DB failed to start: $(up_calls)"

# podman start returns but ps never shows the DB Up: must not fall back to
# starting site-server with dependencies.
new_state stopped-not-up "c1\t${P}-surrealdb-1\tExited (1) 1 hour ago\t${WANT}\n" "${P}_surrealdb-data\n"
touch "${STUB_STATE}/ps_stale"
if run_script update.sh; then fail "update.sh went on with no DB Up after podman start"; fi
[[ -z "$(up_calls)" ]] || fail "update.sh ran compose up with no DB Up: $(up_calls)"

new_state stopped-mismatch "c1\t${P}_surrealdb_1\tExited (137) 1 hour ago\t${OLD}\n" "${P}_surrealdb-data\n"
if run_script update.sh; then fail "update.sh accepted a stopped DB on another image"; fi
expect_refused "update.sh, stopped DB image mismatch"

new_state no-container-volume "" "${P}_surrealdb-data\n"
if run_script update.sh; then fail "update.sh would create the DB over an existing volume"; fi
grep -q SCUFFED_ALLOW_DB_CREATE "${STUB_STATE}/out" || fail "volume refusal does not name SCUFFED_ALLOW_DB_CREATE"
expect_refused "update.sh, volume without container"

new_state no-container-volume-ok "" "${P}_surrealdb-data\n"
run_script update.sh SCUFFED_ALLOW_DB_CREATE=1 \
    || fail "update.sh with SCUFFED_ALLOW_DB_CREATE=1: exit $?. $(tail -5 "${STUB_STATE}/out")"
up_calls | grep -q -- '--no-deps' && fail "SCUFFED_ALLOW_DB_CREATE=1 should start with dependencies"
up_calls | grep -q ' up -d site-server' || fail "SCUFFED_ALLOW_DB_CREATE=1 did not run up -d site-server"
grep -q 'over the existing' "${STUB_STATE}/out" || fail "SCUFFED_ALLOW_DB_CREATE=1 run does not say it uses the existing volume"
grep -q 'No project Surreal container or volume' "${STUB_STATE}/out" && fail "SCUFFED_ALLOW_DB_CREATE=1 run claims there is no volume"

new_state fresh "" "other_volume\n"
run_script update.sh || fail "update.sh fresh host: exit $?. $(tail -5 "${STUB_STATE}/out")"
up_calls | grep -q ' up -d site-server' || fail "fresh host did not start site-server with dependencies"

# Image identity. podman reports a container created from repo:tag@sha256:d
# as repo@sha256:d, so a tag+digest pin in compose.yml must still match.
set_compose_image() {
    sed -i -E "s#^(    image: )docker\.io/surrealdb/surrealdb[:@][^[:space:]]+#\1$1#" "${FIX}/compose.yml"
}
DIGEST="sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
set_compose_image "${WANT}@${DIGEST}"
new_state digest-pin-not-pulled "c1\t${P}-surrealdb-1\tUp 3 days\t${IMG_REPO}@${DIGEST}\timgA\n" "${P}_surrealdb-data\n"
run_script update.sh || fail "update.sh refused a tag+digest pin (not pulled): $(tail -5 "${STUB_STATE}/out")"
expect_no_deps_only "update.sh, tag+digest pin, image not pulled"

new_state digest-pin-pulled "c1\t${P}-surrealdb-1\tUp 3 days\t${IMG_REPO}@${DIGEST}\timgA\n" "${P}_surrealdb-data\n" \
    "${WANT}@${DIGEST}\timgA\n"
run_script update.sh || fail "update.sh refused a tag+digest pin (pulled): $(tail -5 "${STUB_STATE}/out")"
expect_no_deps_only "update.sh, tag+digest pin, image pulled"

new_state digest-pin-other-digest "c1\t${P}-surrealdb-1\tUp 3 days\t${IMG_REPO}@sha256:ffff\timgB\n" "${P}_surrealdb-data\n"
if run_script update.sh; then fail "update.sh accepted a container on another digest"; fi
expect_refused "update.sh, other digest"
set_compose_image "${WANT}"
grep -q "image: ${WANT}\$" "${FIX}/compose.yml" || fail "fixture compose.yml was not restored"

# Same name, different image ID (a re-pushed tag): compose would recreate.
new_state same-name-other-id "c1\t${P}-surrealdb-1\tUp 3 days\t${WANT}\timgOld\n" "${P}_surrealdb-data\n" \
    "${WANT}\timgNew\n"
if run_script update.sh; then fail "update.sh accepted a container whose image ID differs from the pulled compose image"; fi
grep -q 'compared by image ID' "${STUB_STATE}/out" || fail "ID refusal does not say it compared image IDs"
expect_refused "update.sh, same name, other image ID"

# Different name, same image ID: the same image.
new_state other-name-same-id "c1\t${P}-surrealdb-1\tUp 3 days\t${IMG_REPO}@${DIGEST}\timgA\n" "${P}_surrealdb-data\n" \
    "${WANT}\timgA\n"
run_script update.sh || fail "update.sh refused a container on the same image ID: $(tail -5 "${STUB_STATE}/out")"
expect_no_deps_only "update.sh, same image ID"

new_state other-project "c9\tother-surrealdb-1\tUp 1 day\t${OLD}\n" "other_surrealdb-data\n"
run_script update.sh || fail "update.sh with only another project's DB: exit $?"
grep -q 'podman inspect .* c9' "${STUB_STATE}/calls" && fail "guard inspected another project's container"

# --- reset-local-admin.sh -----------------------------------------------------
new_state reset-match "c1\t${P}-surrealdb-1\tUp 3 days\t${WANT}\n" "${P}_surrealdb-data\n"
run_script reset-local-admin.sh BOOTSTRAP_ADMIN_PASSWORD=pw \
    || fail "reset-local-admin.sh: exit $?. $(tail -5 "${STUB_STATE}/out")"
up_calls | grep -q -- 'up -d --force-recreate --no-deps site-server' \
    || fail "reset-local-admin.sh did not use --no-deps: $(up_calls)"
expect_no_deps_only "reset-local-admin.sh"

new_state reset-mismatch "c1\t${P}-surrealdb-1\tUp 3 days\t${OLD}\n" "${P}_surrealdb-data\n"
if run_script reset-local-admin.sh BOOTSTRAP_ADMIN_PASSWORD=pw; then
    fail "reset-local-admin.sh accepted a DB image mismatch"
fi
expect_refused "reset-local-admin.sh, mismatch"

# --- install.sh re-run on an existing host --------------------------------------
new_state install-rerun "c1\t${P}-surrealdb-1\tUp 3 days\t${WANT}\n" "${P}_surrealdb-data\n"
run_script install.sh || fail "install.sh re-run: exit $?. $(tail -5 "${STUB_STATE}/out")"
expect_no_deps_only "install.sh re-run"

new_state install-mismatch "c1\t${P}-surrealdb-1\tUp 3 days\t${OLD}\n" "${P}_surrealdb-data\n"
if run_script install.sh; then fail "install.sh re-run accepted a DB image mismatch"; fi
expect_refused "install.sh re-run, mismatch"

new_state install-fresh "" ""
run_script install.sh || fail "install.sh fresh: exit $?. $(tail -5 "${STUB_STATE}/out")"
up_calls | grep -qE ' up -d$' || fail "install.sh fresh did not run a plain up -d: $(up_calls)"

echo "ok: surreal guard (update.sh, reset-local-admin.sh, install.sh; 21 scenarios)"
