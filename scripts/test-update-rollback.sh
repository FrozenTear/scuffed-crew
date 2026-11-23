#!/usr/bin/env bash
# scripts/update.sh must record a rollback target before it removes the
# running site-server, start the previous image again when the new one is not
# healthy, and with --backup stop on an empty export with the site left
# alone. No real podman, no network: stub podman / curl / ss / sleep on PATH.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "${SCRIPT_DIR}/.." && pwd)"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

TMP="$(mktemp -d)"
trap '[[ -n "${KEEP_TMP:-}" ]] || rm -rf "${TMP}"' EXIT

# --- stub podman / curl / ss / sleep ------------------------------------------
BIN="${TMP}/bin"
mkdir -p "${BIN}"
cat >"${BIN}/podman" <<'STUB'
#!/usr/bin/env bash
# State: $STUB_STATE/containers  lines "id<TAB>name<TAB>status<TAB>image<TAB>image-id"
#        $STUB_STATE/images      lines "ref<TAB>image-id"
#        $STUB_STATE/volumes     one name per line
echo "podman $*" >>"${STUB_STATE}/calls"
C="${STUB_STATE}/containers"
image_id() {
    awk -F'\t' -v r="$1" '$1 == r { id = $2 } END { if (id != "") print id; else exit 1 }' "${STUB_STATE}/images"
}
drop_site_server() {
    grep -v 'site-server' "${C}" >"${C}.new" || true
    mv "${C}.new" "${C}"
}
cmd="$1"; shift || true
case "${cmd}" in
    compose)
        [[ "${1:-}" == "version" ]] && { echo "podman-compose version 1.6.0"; exit 0; }
        case " $* " in
            *" up "*)
                img="${SITE_SERVER_IMAGE:-unset}"
                echo "compose-up image=${img}" >>"${STUB_STATE}/calls"
                # up_fails_new: compose cannot start anything but the rollback image.
                [[ -f "${STUB_STATE}/up_fails_new" && "${img}" != *:rollback ]] && exit 125
                if ! grep -q 'site-server' "${C}"; then
                    printf 's1\tscuffed-crew-site-server-1\tUp 1 second\t%s\t%s\n' \
                        "${img}" "$(image_id "${img}" || echo none)" >>"${C}"
                fi ;;
            *" stop site-server "*|*" rm -f site-server "*) drop_site_server ;;
        esac
        exit 0 ;;
    pull) exit 0 ;;
    ps)
        fmt=""
        while [[ $# -gt 0 ]]; do
            case "$1" in --format) fmt="$2"; shift 2 ;; *) shift ;; esac
        done
        while IFS=$'\t' read -r id name status image _imgid; do
            [[ -n "${id}" ]] || continue
            line="${fmt//\{\{.ID\}\}/${id}}"
            line="${line//\{\{.Names\}\}/${name}}"
            line="${line//\{\{.Status\}\}/${status}}"
            line="${line//\{\{.Image\}\}/${image}}"
            printf '%b\n' "${line}"
        done <"${C}"
        exit 0 ;;
    inspect)
        fmt="$2"; id="$3"
        while IFS=$'\t' read -r cid name status image imgid; do
            [[ "${cid}" == "${id}" || "${name}" == "${id}" ]] || continue
            case "${fmt}" in
                *ImageName*) echo "${image}" ;;
                *'.Image}}'*) echo "${imgid:-}" ;;
                *NetworkSettings*) echo "10.89.0.5 " ;;
                *State.Running*) if [[ "${status}" == Up* ]]; then echo true; else echo false; fi ;;
                *State.Status*) echo running ;;
            esac
            exit 0
        done <"${C}"
        exit 1 ;;
    image)
        if [[ "${1:-}" == "inspect" ]]; then
            image_id "$2" && exit 0
            echo "Error: $2: image not known" >&2
            exit 125
        fi
        exit 0 ;;
    tag)
        [[ -f "${STUB_STATE}/tag_fails" ]] && exit 125
        printf '%s\t%s\n' "$2" "$1" >>"${STUB_STATE}/images"
        exit 0 ;;
    rm)
        # podman rm -f <cid>
        grep -v "^$2	" "${C}" >"${C}.new" || true
        mv "${C}.new" "${C}"
        exit 0 ;;
    volume)
        [[ "${1:-}" == "ls" ]] && { cat "${STUB_STATE}/volumes" 2>/dev/null || true; }
        exit 0 ;;
    *) exit 0 ;;
esac
STUB

cat >"${BIN}/curl" <<'STUB'
#!/usr/bin/env bash
# /api/health answers with the git_sha in $STUB_STATE/health_<image-id> of the
# current site-server container; no such file means the site is not healthy.
# $STUB_STATE/health_once: an image answers once, then never again.
# /export copies $STUB_STATE/export_body to the -o file (empty when absent).
echo "curl $*" >>"${STUB_STATE}/calls"
out="" url="" stdin_cfg=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        -o) out="$2"; shift 2 ;;
        -K) [[ "$2" == "-" ]] && stdin_cfg=1; shift 2 ;;
        -H|--max-time|-u) shift 2 ;;
        http*) url="$1"; shift ;;
        *) shift ;;
    esac
done
case "${url}" in
    */api/health)
        imgid="$(awk -F'\t' '$2 ~ /site-server/ { print $5; exit }' "${STUB_STATE}/containers")"
        [[ -n "${imgid}" && -f "${STUB_STATE}/health_${imgid}" ]] || exit 7
        printf '{"status":"ok","git_sha":"%s"}\n' "$(cat "${STUB_STATE}/health_${imgid}")"
        [[ -f "${STUB_STATE}/health_once" ]] && rm -f "${STUB_STATE}/health_${imgid}"
        exit 0 ;;
    */export)
        [[ "${stdin_cfg}" == "1" ]] && cat >"${STUB_STATE}/curl_stdin"
        [[ -f "${STUB_STATE}/export_fails" ]] && exit 22
        if [[ -f "${STUB_STATE}/export_body" ]]; then
            cat "${STUB_STATE}/export_body" >"${out}"
        else
            : >"${out}"
        fi
        exit 0 ;;
esac
echo "curl stub: unexpected url ${url}" >&2
exit 1
STUB
printf '#!/usr/bin/env bash\nexit 0\n' >"${BIN}/ss"
printf '#!/usr/bin/env bash\nexit 0\n' >"${BIN}/sleep"
chmod +x "${BIN}/podman" "${BIN}/curl" "${BIN}/ss" "${BIN}/sleep"

# --- fixture repo (no .git, so update.sh skips git pull) -----------------------
FIX="${TMP}/scuffed-crew"
mkdir -p "${FIX}/scripts/lib" "${FIX}/data"
cp "${REPO}/compose.yml" "${FIX}/"
cp "${REPO}/scripts/update.sh" "${FIX}/scripts/"
cp "${REPO}"/scripts/lib/*.sh "${FIX}/scripts/lib/"
printf '#!/usr/bin/env bash\nexit 0\n' >"${FIX}/scripts/ensure-trusted-proxies.sh"
cat >"${FIX}/data/secrets.env" <<'ENV'
SURREALDB_PASSWORD=rootpw-s3cret
SURREALDB_APP_PASSWORD=apppw
ENCRYPTION_KEY=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
NOSTR_CHALLENGE_SECRET=x
HOST_PORT=3999
SITE_SERVER_IMAGE=ghcr.io/frozentear/scuffed-crew:main
ALLOWED_ORIGINS=http://127.0.0.1:3999
PRODUCTION=1
SURREALDB_AUTH_MODE=scoped
SURREALDB_APP_USER=scuffed_app
ENV

WANT="$(awk '/^  surrealdb:/{s=1} s && /^    image:/{print $2; exit}' "${REPO}/compose.yml")"
[[ -n "${WANT}" ]] || fail "could not read the surrealdb image from compose.yml"
P=scuffed-crew
IMG=ghcr.io/frozentear/scuffed-crew
OLD_SHA=aaaaaaa1
NEW_SHA=bbbbbbb2
DB_LINE="c1\t${P}-surrealdb-1\tUp 3 days\t${WANT}\tdbimg\n"
SITE_LINE="s0\t${P}-site-server-1\tUp 3 days\t${IMG}:main\timgOld\n"

# new_state <name> <containers-lines> [<images-lines>]
# Default images: :main was just pulled and is now imgNew. The old site
# answers health with OLD_SHA, the new image with NEW_SHA.
new_state() {
    STUB_STATE="${TMP}/state-$1"
    rm -rf "${STUB_STATE}" "${FIX}/backups-manual"
    mkdir -p "${STUB_STATE}"
    printf '%b' "$2" >"${STUB_STATE}/containers"
    printf '%b' "${3:-${IMG}:main\timgNew\n}" >"${STUB_STATE}/images"
    printf '%s\n' "${P}_surrealdb-data" >"${STUB_STATE}/volumes"
    printf '%s' "${OLD_SHA}" >"${STUB_STATE}/health_imgOld"
    printf '%s' "${NEW_SHA}" >"${STUB_STATE}/health_imgNew"
    : >"${STUB_STATE}/calls"
    export STUB_STATE
}

run_update() {
    env -i PATH="${BIN}:/usr/bin:/bin" HOME="${TMP}" STUB_STATE="${STUB_STATE}" \
        SURREAL_GUARD_WAIT_SECS=0 bash "${FIX}/scripts/update.sh" "$@" \
        >"${STUB_STATE}/out" 2>&1 </dev/null
}

out_has() { grep -qF -- "$1" "${STUB_STATE}/out"; }
calls_has() { grep -qE -- "$1" "${STUB_STATE}/calls"; }
tail_out() { tail -8 "${STUB_STATE}/out"; }
line_of() { grep -nE -m1 -- "$1" "${STUB_STATE}/calls" | cut -d: -f1; }
site_image() { awk -F'\t' '$2 ~ /site-server/ { print $4 "|" $5 }' "${STUB_STATE}/containers"; }

expect_db_untouched() {
    local label="$1"
    grep -q "^c1	${P}-surrealdb-1	" "${STUB_STATE}/containers" || fail "${label}: the Surreal container is gone"
    calls_has '^podman (rm|stop|start|restart) .*c1' && fail "${label}: touched the Surreal container"
    if grep -E '^podman compose .* up ' "${STUB_STATE}/calls" | grep -v -- '--no-deps site-server' | grep -q .; then
        fail "${label}: compose up without --no-deps site-server"
    fi
    return 0
}

expect_site_untouched() {
    local label="$1"
    calls_has '^podman (compose .* )?(rm|stop|start|tag|up) ' \
        && fail "${label}: touched a container: $(grep -E '^podman (compose .* )?(rm|stop|start|tag|up) ' "${STUB_STATE}/calls" | head -3)"
    [[ "$(site_image)" == "${IMG}:main|imgOld" ]] || fail "${label}: running site-server changed: $(site_image)"
}

# --- a) healthy update ---------------------------------------------------------
new_state healthy "${DB_LINE}${SITE_LINE}"
run_update || fail "healthy update: exit $?. $(tail_out)"
calls_has "^podman tag imgOld ${IMG}:rollback\$" || fail "healthy update did not tag imgOld as ${IMG}:rollback"
tag_line="$(line_of '^podman tag ')"
rm_line="$(line_of '^podman (compose .* )?(stop|rm) ')"
[[ -n "${rm_line}" && "${tag_line}" -lt "${rm_line}" ]] || fail "rollback tag was not recorded before the site-server was removed"
out_has "Live git_sha before update: ${OLD_SHA}" || fail "healthy update did not print the previous git_sha. $(tail_out)"
out_has "Rollback image: ${IMG}:rollback" || fail "healthy update did not print the rollback tag"
out_has "Updated & healthy" || fail "healthy update lost the Updated & healthy line"
out_has "Live git_sha: ${NEW_SHA}" || fail "healthy update did not print the new live git_sha. $(tail_out)"
[[ "$(site_image)" == "${IMG}:main|imgNew" ]] || fail "healthy update: site-server is not on the new image: $(site_image)"
[[ ! -e "${FIX}/backups-manual" ]] || fail "update without --backup wrote backups-manual/"
expect_db_untouched "healthy update"

# A leftover --no-deps argument is a no-op.
new_state no-deps-arg "${DB_LINE}${SITE_LINE}"
run_update --no-deps || fail "update.sh --no-deps: exit $?. $(tail_out)"
out_has "Live git_sha: ${NEW_SHA}" || fail "update.sh --no-deps did not finish the update"

# --- b) failed health, rollback -------------------------------------------------
new_state rollback "${DB_LINE}${SITE_LINE}"
rm "${STUB_STATE}/health_imgNew"
if run_update; then fail "update.sh exited 0 after the new container failed health"; fi
calls_has "^podman tag imgOld ${IMG}:rollback\$" || fail "rollback: imgOld was not tagged"
calls_has "^compose-up image=${IMG}:main\$" || fail "rollback: the new image was never started"
calls_has "^compose-up image=${IMG}:rollback\$" || fail "rollback: the previous image was not started. $(tail_out)"
new_up="$(line_of "^compose-up image=${IMG}:main\$")"
old_up="$(line_of "^compose-up image=${IMG}:rollback\$")"
stop_after="$(awk -v n="${new_up}" 'NR > n && /^podman (compose .* )?(stop|rm) / { print NR; exit }' "${STUB_STATE}/calls")"
[[ -n "${stop_after}" && "${stop_after}" -lt "${old_up}" ]] || fail "rollback: the unhealthy container was not stopped before the previous image started"
[[ "$(site_image)" == "${IMG}:rollback|imgOld" ]] || fail "rollback: site-server is not on the previous image: $(site_image)"
out_has "update FAILED" || fail "rollback: no clear failure message. $(tail_out)"
out_has "Rolled back" || fail "rollback: message does not mention the rollback. $(tail_out)"
out_has "Previous git_sha: ${OLD_SHA}" || fail "rollback: message does not name the previous git_sha. $(tail_out)"
out_has "Updated & healthy" && fail "rollback: a failed update claims success"
expect_db_untouched "rollback"

# compose up of the new image fails outright: same rollback.
new_state up-fails-once "${DB_LINE}${SITE_LINE}"
rm "${STUB_STATE}/health_imgNew"
touch "${STUB_STATE}/up_fails_new"
if run_update; then fail "update.sh exited 0 after compose up failed"; fi
[[ "$(site_image)" == "${IMG}:rollback|imgOld" ]] || fail "compose up failure: previous image not restored: $(site_image). $(tail_out)"
out_has "Rolled back" || fail "compose up failure: message does not mention the rollback. $(tail_out)"

# Rollback itself fails: previous image never becomes healthy.
new_state rollback-unhealthy "${DB_LINE}${SITE_LINE}"
rm "${STUB_STATE}/health_imgNew"
# The old site answers once (the sha read), then never again.
touch "${STUB_STATE}/health_once"
if run_update; then fail "update.sh exited 0 after a failed rollback"; fi
out_has "rollback FAILED" || fail "failed rollback is not reported. $(tail_out)"
out_has "Rolled back:" && fail "failed rollback claims the previous image is healthy"
out_has "git_sha ${OLD_SHA})." || fail "failed rollback does not name the previous git_sha"
expect_db_untouched "failed rollback"

# Rollback tag cannot be created: stop with the site still running.
new_state tag-fails "${DB_LINE}${SITE_LINE}"
touch "${STUB_STATE}/tag_fails"
if run_update; then fail "update.sh went on without a rollback tag"; fi
calls_has '^podman (compose .* )?(rm|stop|up) ' && fail "tag failure: the site-server was still touched"
[[ "$(site_image)" == "${IMG}:main|imgOld" ]] || fail "tag failure: running site-server changed"

# Site down (earlier failed update left a dead container) and a rollback tag
# exists: the dead container's image must not replace it.
new_state keep-tag "${DB_LINE}s0\t${P}-site-server-1\tExited (1) 1 hour ago\t${IMG}:main\timgBad\n" \
    "${IMG}:rollback\timgOld\n${IMG}:main\timgNew\n"
run_update || fail "update over a dead site-server: exit $?. $(tail_out)"
calls_has '^podman tag ' && fail "a dead site-server replaced the existing rollback tag"
out_has "Keeping the existing rollback image" || fail "kept rollback tag is not reported"
out_has "Live git_sha: ${NEW_SHA}" || fail "update over a dead site-server did not finish"

# A healthy build without a sha reports "unknown". It is still the rollback
# target: an older :rollback tag must be replaced.
new_state unknown-sha "${DB_LINE}${SITE_LINE}" "${IMG}:rollback\timgAncient\n${IMG}:main\timgNew\n"
printf 'unknown' >"${STUB_STATE}/health_imgOld"
rm "${STUB_STATE}/health_imgNew"
if run_update; then fail "update.sh exited 0 after the new container failed health (unknown sha)"; fi
calls_has "^podman tag imgOld ${IMG}:rollback\$" || fail "a healthy site with an unknown sha was not tagged as the rollback image"
[[ "$(site_image)" == "${IMG}:rollback|imgOld" ]] || fail "unknown sha: rolled back to the wrong image: $(site_image)"

# Fresh install: nothing to tag, a health failure fails as before.
new_state fresh-unhealthy "" "${IMG}:main\timgNew\n"
: >"${STUB_STATE}/volumes"
rm "${STUB_STATE}/health_imgNew"
if run_update; then fail "fresh install with a failing health check exited 0"; fi
calls_has '^podman tag ' && fail "fresh install tagged a rollback image"
calls_has "^compose-up image=.*:rollback\$" && fail "fresh install tried a rollback"
out_has "nothing to roll back to" || fail "fresh install failure does not say there is no rollback target. $(tail_out)"

new_state fresh-healthy "" "${IMG}:main\timgNew\n"
: >"${STUB_STATE}/volumes"
run_update || fail "fresh install: exit $?. $(tail_out)"
out_has "Live git_sha: ${NEW_SHA}" || fail "fresh install did not print the live git_sha"

# --- c) --backup ----------------------------------------------------------------
new_state backup-empty "${DB_LINE}${SITE_LINE}"
if run_update --backup; then fail "update.sh --backup went on after an empty export"; fi
calls_has '^curl .*http://10\.89\.0\.5:8000/export$' || fail "--backup did not request /export on the Surreal container IP. $(tail_out)"
out_has "export is missing or empty" || fail "empty export is not reported. $(tail_out)"
expect_site_untouched "--backup, empty export"
[[ -z "$(ls -A "${FIX}/backups-manual" 2>/dev/null)" ]] || fail "empty export file was left behind"

new_state backup-http-error "${DB_LINE}${SITE_LINE}"
touch "${STUB_STATE}/export_fails"
if run_update --backup; then fail "update.sh --backup went on after a failed export request"; fi
expect_site_untouched "--backup, failed request"

new_state backup-no-db "${SITE_LINE}"
if run_update --backup; then fail "update.sh --backup went on with no Surreal Up"; fi
expect_site_untouched "--backup, no Surreal"

new_state backup-ok "${DB_LINE}${SITE_LINE}"
printf -- '-- export\nDEFINE TABLE member;\n' >"${STUB_STATE}/export_body"
run_update --backup || fail "update.sh --backup: exit $?. $(tail_out)"
shopt -s nullglob
files=("${FIX}"/backups-manual/surreal-export-*Z.surql)
shopt -u nullglob
[[ "${#files[@]}" == "1" ]] || fail "--backup wrote ${#files[@]} export files, want 1"
cmp -s "${files[0]}" "${STUB_STATE}/export_body" || fail "--backup export content differs"
[[ "$(stat -c %a "${files[0]}")" == "600" ]] || fail "export file mode is $(stat -c %a "${files[0]}"), want 600"
[[ "$(stat -c %a "${FIX}/backups-manual")" == "700" ]] || fail "backups-manual mode is $(stat -c %a "${FIX}/backups-manual"), want 700"
export_call="$(grep -E '^curl .*/export$' "${STUB_STATE}/calls")"
[[ "${export_call}" == *"surreal-ns: scuffed_crew"* && "${export_call}" == *"surreal-db: main"* ]] \
    || fail "--backup did not send the ns/db headers: ${export_call}"
[[ "${export_call}" == *"Accept: application/octet-stream"* ]] || fail "--backup did not send the Accept header"
grep -qx 'user = "root:rootpw-s3cret"' "${STUB_STATE}/curl_stdin" || fail "--backup did not authenticate as root"
grep -q 'rootpw-s3cret' "${STUB_STATE}/out" "${STUB_STATE}/calls" && fail "--backup leaked the database password (output or argv)"
backup_line="$(line_of '^curl .*/export$')"
rm_line="$(line_of '^podman (compose .* )?(stop|rm) ')"
[[ "${backup_line}" -lt "${rm_line}" ]] || fail "--backup ran after the site-server was removed"
out_has "Live git_sha: ${NEW_SHA}" || fail "--backup update did not finish"
expect_db_untouched "--backup"

# --- argv -----------------------------------------------------------------------
new_state bad-arg "${DB_LINE}${SITE_LINE}"
if run_update --bakcup; then fail "update.sh accepted an unknown argument"; fi
[[ ! -s "${STUB_STATE}/calls" ]] || fail "unknown argument still ran podman or curl"

echo "ok: update.sh rollback and --backup (15 scenarios)"
