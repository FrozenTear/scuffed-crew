#!/usr/bin/env bash
# scripts/update.sh must record a rollback target before it removes the
# running site-server, start the previous image again when the new one is not
# healthy or the update fails anywhere else after that removal, and with
# --backup stop on an empty export with the site left alone. No real podman, no network: stub podman / curl / ss / sleep on PATH.

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
# Flags: up_created            compose up leaves site-server Created
#        start_fails_<id>      podman start <id> fails
#        start_noop_<id>       podman start <id> exits 0, container stays down
#        start_running_only_<id>  State.Running turns true, ps status does not
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
                    st="Up 1 second"
                    [[ -f "${STUB_STATE}/up_created" ]] && st="Created"
                    printf 's1\tscuffed-crew-site-server-1\t%s\t%s\t%s\n' \
                        "${st}" "${img}" "$(image_id "${img}" || echo none)" >>"${C}"
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
                *State.Running*)
                    if [[ "${status}" == Up* || -f "${STUB_STATE}/running_${cid}" ]]; then echo true; else echo false; fi ;;
                *State.Status*)
                    case "${status}" in
                        Up*) echo running ;;
                        Created*) echo created ;;
                        *) echo exited ;;
                    esac ;;
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
    start)
        # podman start <cid|name>
        cid="$(awk -F'\t' -v k="$1" '$1 == k || $2 == k { print $1; exit }' "${C}")"
        [[ -n "${cid}" ]] || exit 125
        [[ -f "${STUB_STATE}/start_fails_${cid}" ]] && exit 125
        [[ -f "${STUB_STATE}/start_noop_${cid}" ]] && exit 0
        if [[ -f "${STUB_STATE}/start_running_only_${cid}" ]]; then
            touch "${STUB_STATE}/running_${cid}"
            exit 0
        fi
        awk -F'\t' -v OFS='\t' -v k="${cid}" '$1 == k { $3 = "Up 1 second" } { print }' "${C}" >"${C}.new"
        mv "${C}.new" "${C}"
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
# current site-server container; no such file, or a container that is not Up,
# means the site is not healthy.
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
        imgid="$(awk -F'\t' '$2 ~ /site-server/ && $3 ~ /^Up/ { print $5; exit }' "${STUB_STATE}/containers")"
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
cat >"${BIN}/ss" <<'STUB'
#!/usr/bin/env bash
# $STUB_STATE/port_busy: something else listens on HOST_PORT (3999).
if [[ -f "${STUB_STATE}/port_busy" ]]; then
    echo "LISTEN 0      128        127.0.0.1:3999       0.0.0.0:*"
fi
exit 0
STUB
cat >"${BIN}/sleep" <<'STUB'
#!/usr/bin/env bash
# $STUB_STATE/term_on_sleep: send SIGTERM to update.sh, once.
if [[ -f "${STUB_STATE}/term_on_sleep" ]]; then
    rm -f "${STUB_STATE}/term_on_sleep"
    kill -TERM "${PPID}"
fi
exit 0
STUB
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
DB_STOPPED_LINE="c1\t${P}-surrealdb-1\tExited (0) 2 hours ago\t${WANT}\tdbimg\n"
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

# The Surreal container is still there and nothing removed, stopped or
# recreated it (a podman start of the stopped container as it is may have run).
expect_db_not_recreated() {
    local label="$1"
    grep -q "^c1	${P}-surrealdb-1	" "${STUB_STATE}/containers" || fail "${label}: the Surreal container is gone"
    calls_has '^podman (rm|stop|restart) .*c1' && fail "${label}: touched the Surreal container"
    if grep -E '^podman compose .* up ' "${STUB_STATE}/calls" | grep -v -- '--no-deps site-server' | grep -q .; then
        fail "${label}: compose up without --no-deps site-server"
    fi
    return 0
}

expect_db_untouched() {
    local label="$1"
    expect_db_not_recreated "${label}"
    calls_has '^podman start .*c1' && fail "${label}: started the Surreal container"
    return 0
}

# After a failure between the removal and the health check: the new image was
# never started, the previous one is back, and the output says so.
expect_rolled_back() {
    local label="$1"
    calls_has "^podman tag imgOld ${IMG}:rollback\$" || fail "${label}: imgOld was not tagged"
    calls_has "^compose-up image=${IMG}:main\$" && fail "${label}: the new image was started"
    calls_has "^compose-up image=${IMG}:rollback\$" || fail "${label}: the previous image was not started. $(tail_out)"
    [[ "$(site_image)" == "${IMG}:rollback|imgOld" ]] || fail "${label}: site-server is not on the previous image: $(site_image)"
    out_has "update FAILED. Rolled back" || fail "${label}: message does not mention the rollback. $(tail_out)"
    out_has "Previous git_sha: ${OLD_SHA}" || fail "${label}: message does not name the previous git_sha. $(tail_out)"
    out_has "Updated & healthy" && fail "${label}: a failed update claims success"
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
out_has "data/secrets.env still names the failed image: SITE_SERVER_IMAGE=${IMG}:main" \
    || fail "rollback: message does not say secrets.env still names the failed pin. $(tail_out)"
out_has "    SITE_SERVER_IMAGE=${IMG}:rollback" || fail "rollback: message does not say which tag to set back. $(tail_out)"
out_has "git_sha ${OLD_SHA}, image imgOld" || fail "rollback: pin note does not name the previous git_sha. $(tail_out)"
out_has "${IMG}:sha-${OLD_SHA:0:7} " || fail "rollback: pin note does not name the pullable sha tag. $(tail_out)"
out_has "not undone" || fail "rollback: the migration note is gone. $(tail_out)"
expect_db_untouched "rollback"

# compose up of the new image fails outright: same rollback.
new_state up-fails-once "${DB_LINE}${SITE_LINE}"
rm "${STUB_STATE}/health_imgNew"
touch "${STUB_STATE}/up_fails_new"
if run_update; then fail "update.sh exited 0 after compose up failed"; fi
[[ "$(site_image)" == "${IMG}:rollback|imgOld" ]] || fail "compose up failure: previous image not restored: $(site_image). $(tail_out)"
out_has "Rolled back" || fail "compose up failure: message does not mention the rollback. $(tail_out)"
out_has "    SITE_SERVER_IMAGE=${IMG}:rollback" || fail "compose up failure: message does not say which tag to set back. $(tail_out)"

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

# --- c) site-server left Created after --no-deps ---------------------------------
new_state created-healthy "${DB_LINE}${SITE_LINE}"
touch "${STUB_STATE}/up_created"
run_update || fail "Created site-server: exit $?. $(tail_out)"
out_has "is Created after --no-deps, starting it" || fail "Created site-server is not reported. $(tail_out)"
calls_has "^podman start ${P}-site-server-1\$" || fail "Created site-server was not started with podman start"
up_line="$(line_of "^compose-up image=${IMG}:main\$")"
start_line="$(line_of "^podman start ${P}-site-server-1\$")"
[[ "${up_line}" -lt "${start_line}" ]] || fail "podman start ran before compose up"
[[ "$(site_image)" == "${IMG}:main|imgNew" ]] || fail "Created site-server: not on the new image: $(site_image)"
out_has "Live git_sha: ${NEW_SHA}" || fail "Created site-server: update did not finish. $(tail_out)"
expect_db_untouched "Created site-server"

# Started from Created, then not healthy: rollback (whose container is also
# left Created and started the same way).
new_state created-unhealthy "${DB_LINE}${SITE_LINE}"
touch "${STUB_STATE}/up_created"
rm "${STUB_STATE}/health_imgNew"
if run_update; then fail "update.sh exited 0 after a Created site-server failed health"; fi
[[ "$(grep -cE "^podman start ${P}-site-server-1\$" "${STUB_STATE}/calls")" == "2" ]] \
    || fail "Created + unhealthy: want podman start for the new and for the previous container"
[[ "$(site_image)" == "${IMG}:rollback|imgOld" ]] || fail "Created + unhealthy: previous image not restored: $(site_image). $(tail_out)"
out_has "update FAILED. Rolled back" || fail "Created + unhealthy: message does not mention the rollback. $(tail_out)"
out_has "Previous git_sha: ${OLD_SHA}" || fail "Created + unhealthy: message does not name the previous git_sha"
expect_db_untouched "Created + unhealthy"

# --- d) failures between the removal and the health check -----------------------
# Port still busy after the site-server is gone.
new_state port-busy "${DB_LINE}${SITE_LINE}"
touch "${STUB_STATE}/port_busy"
if run_update; then fail "update.sh exited 0 with HOST_PORT still busy"; fi
out_has "still in use after removing" || fail "port busy: the port diagnostic is gone. $(tail_out)"
out_has "HOST_PORT=<free>" || fail "port busy: the HOST_PORT hint is gone"
expect_rolled_back "port busy"
expect_db_untouched "port busy"

# Fresh install with the port busy: nothing to roll back to, no rollback claimed.
new_state fresh-port-busy "" "${IMG}:main\timgNew\n"
: >"${STUB_STATE}/volumes"
touch "${STUB_STATE}/port_busy"
if run_update; then fail "fresh install with HOST_PORT busy exited 0"; fi
out_has "nothing to roll back to" || fail "fresh install, port busy: does not say there is no rollback target. $(tail_out)"
out_has "Rolling back" && fail "fresh install, port busy: claims a rollback"
calls_has '^compose-up ' && fail "fresh install, port busy: compose up still ran"

# surreal_guard_apply fails: podman start of the stopped DB fails.
new_state apply-start-fails "${DB_STOPPED_LINE}${SITE_LINE}"
touch "${STUB_STATE}/start_fails_c1"
if run_update; then fail "update.sh exited 0 after podman start of the DB failed"; fi
rm_line="$(line_of '^podman (compose .* )?(stop|rm) ')"
db_start="$(line_of '^podman start c1$')"
[[ -n "${db_start}" && "${rm_line}" -lt "${db_start}" ]] || fail "apply failure: the DB start did not come after the site-server removal"
out_has "podman start ${P}-surrealdb-1 failed" || fail "apply failure: the guard error is gone. $(tail_out)"
expect_rolled_back "apply failure (start)"
expect_db_not_recreated "apply failure (start)"

# surreal_guard_apply fails: the DB starts but does not stay running.
new_state apply-not-running "${DB_STOPPED_LINE}${SITE_LINE}"
touch "${STUB_STATE}/start_noop_c1"
if run_update; then fail "update.sh exited 0 after the DB did not stay running"; fi
out_has "did not stay running" || fail "apply failure: the guard error is gone. $(tail_out)"
expect_rolled_back "apply failure (not running)"
expect_db_not_recreated "apply failure (not running)"

# apply reports the DB running, but no project Surreal is Up afterwards.
new_state no-surreal-up "${DB_STOPPED_LINE}${SITE_LINE}"
touch "${STUB_STATE}/start_running_only_c1"
if run_update; then fail "update.sh exited 0 with no Surreal Up after the start"; fi
out_has "no project Surreal container is Up after starting ${P}-surrealdb-1" \
    || fail "no Surreal Up: the error is gone. $(tail_out)"
out_has "Not starting site-server with dependencies" || fail "no Surreal Up: the with-dependencies refusal is gone"
expect_rolled_back "no Surreal Up"
expect_db_not_recreated "no Surreal Up"

# Interrupted (SIGTERM) while waiting for the new site-server: same rollback.
new_state interrupted "${DB_LINE}${SITE_LINE}"
rm "${STUB_STATE}/health_imgNew"
touch "${STUB_STATE}/term_on_sleep"
if run_update; then fail "update.sh exited 0 after SIGTERM during the health wait"; fi
[[ ! -e "${STUB_STATE}/term_on_sleep" ]] || fail "interrupt: the signal was never sent"
out_has "is not healthy after" && fail "interrupt: the health wait ran to its end"
[[ "$(site_image)" == "${IMG}:rollback|imgOld" ]] || fail "interrupt: previous image not restored: $(site_image). $(tail_out)"
out_has "update FAILED. Rolled back" || fail "interrupt: message does not mention the rollback. $(tail_out)"
expect_db_untouched "interrupt"

# --- e) --backup ----------------------------------------------------------------
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

echo "ok: update.sh rollback and --backup (23 scenarios)"
